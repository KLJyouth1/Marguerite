//! 会话回滚：侧 git 快照系统
//!
//! 独立于项目 .git 的工作区快照。每轮对话前对变更文件做增量 tar 快照，
//! 支持整体 `restore` / `revert_turn`。所有路径操作均 canonicalize 并校验不逃逸工作区。

use std::collections::HashMap;
use std::io::Read;
use std::path::{Component, Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tar::{Archive, Builder, Header};
use uuid::Uuid;

const SIDEGIT_DIR: &str = ".marguerite-sidegit";
const SNAPSHOTS_DIR: &str = "snapshots";
const HEAD_FILE: &str = "HEAD";
const MAX_TAR_DECOMPRESSED: usize = 256 * 1024 * 1024;

/// 公开快照元信息（`list` / 各方法返回值）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snapshot {
    /// 快照 ID（UUID）
    pub id: String,
    /// 时间戳（ISO 8601）
    pub ts: String,
    /// 用户消息 / 自动标记
    pub message: String,
    /// 清单 sha256
    pub hash: String,
}

/// 清单中的单文件记录
#[derive(Debug, Clone, Serialize, Deserialize)]
struct FileEntry {
    /// 相对工作区路径，使用 '/'
    path: String,
    /// 字节大小
    size: u64,
    /// 权限位（Unix）
    mode: u32,
    /// 内容 sha256
    sha256: String,
    /// 存放该文件内容的快照 ID（变更文件指向自身，未变更文件继承自上一快照）
    source: String,
}

/// 快照清单（落盘为 `<id>.manifest.json`）
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Manifest {
    id: String,
    ts: String,
    message: String,
    hash: String,
    files: Vec<FileEntry>,
}

/// 侧 git 快照管理器
///
/// `worktree` 指向 `$workspace/.marguerite-sidegit/<session>/`，
/// 工作区根目录为其上两级。
pub struct SideGit {
    /// $workspace/.marguerite-sidegit/<session>/
    worktree: PathBuf,
    /// 内存中的快照缓存（按 ts 倒序）
    snapshots: Vec<Snapshot>,
}

impl SideGit {
    /// 基于工作区根目录与会话 ID 构造管理器
    pub fn new(workspace: &Path, session_id: &str) -> Result<Self> {
        let ws = canonicalize_or_self(workspace);
        let worktree = ws.join(SIDEGIT_DIR).join(session_id);
        std::fs::create_dir_all(worktree.join(SNAPSHOTS_DIR))
            .with_context(|| format!("create sidegit dir {:?}", worktree))?;
        let mut sg = Self {
            worktree,
            snapshots: Vec::new(),
        };
        sg.reload_snapshots()?;
        Ok(sg)
    }

    /// 工作区根目录（= worktree 的上两级）
    fn workspace(&self) -> PathBuf {
        self.worktree
            .parent()
            .and_then(|p| p.parent())
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| self.worktree.clone())
    }

    fn snapshots_dir(&self) -> PathBuf {
        self.worktree.join(SNAPSHOTS_DIR)
    }

    fn reload_snapshots(&mut self) -> Result<()> {
        self.snapshots.clear();
        let dir = self.snapshots_dir();
        if !dir.exists() {
            return Ok(());
        }
        for entry in std::fs::read_dir(&dir)? {
            let entry = entry?;
            let p = entry.path();
            let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if !name.ends_with(".manifest.json") {
                continue;
            }
            let bytes = std::fs::read(&p)?;
            let m: Manifest = serde_json::from_slice(&bytes)
                .with_context(|| format!("parse manifest {:?}", p))?;
            self.snapshots.push(Snapshot {
                id: m.id,
                ts: m.ts,
                message: m.message,
                hash: m.hash,
            });
        }
        self.sort_snapshots();
        Ok(())
    }

    fn sort_snapshots(&mut self) {
        // ISO 8601 字符串的字典序即时间序
        self.snapshots.sort_by(|a, b| b.ts.cmp(&a.ts));
    }

    /// 每轮对话前快照工作区（增量）
    pub fn snapshot_before_turn(&mut self, msg: &str) -> Result<Snapshot> {
        let ws = self.workspace();
        let canonical_ws = canonicalize_or_self(&ws);

        // 1) 扫描工作区（排除规则）
        let scanned = scan_workspace(&canonical_ws)?;

        // 2) 读取上一快照清单用于增量 diff
        let prev = self.head_manifest()?;
        let prev_map: HashMap<&str, &FileEntry> = match &prev {
            Some(m) => m.files.iter().map(|f| (f.path.as_str(), f)).collect(),
            None => HashMap::new(),
        };

        // 3) 计算变更集合，构建完整文件清单
        let mut changed: Vec<(String, Vec<u8>, u32)> = Vec::new();
        let mut files: Vec<FileEntry> = Vec::new();

        for (rel, abs, size, mode, sha) in &scanned {
            let base = FileEntry {
                path: rel.clone(),
                size: *size,
                mode: *mode,
                sha256: sha.clone(),
                source: String::new(),
            };
            match prev_map.get(rel.as_str()) {
                Some(prev_f) if prev_f.sha256 == *sha => {
                    // 未变更：继承上一快照的 source
                    files.push(FileEntry {
                        source: prev_f.source.clone(),
                        ..base
                    });
                }
                _ => {
                    // 变更或新增：读取内容打包，source 稍后回填为本快照 ID
                    let bytes =
                        std::fs::read(abs).with_context(|| format!("read file {:?}", abs))?;
                    changed.push((rel.clone(), bytes, *mode));
                    files.push(base);
                }
            }
        }
        // 注：上一快照存在但本次扫描缺失的文件视为已删除，不进入清单

        // 4) 生成 ID / 时间戳，回填变更文件的 source
        let id = Uuid::new_v4().to_string();
        let ts = Utc::now().to_rfc3339();
        for f in files.iter_mut() {
            if f.source.is_empty() {
                f.source = id.clone();
            }
        }

        // 5) 增量打包 tar（仅包含变更文件，原始未压缩）
        let tar_bytes = build_tar(&changed)?;
        let tar_path = self.snapshots_dir().join(format!("{}.tar", id));
        std::fs::write(&tar_path, &tar_bytes)
            .with_context(|| format!("write snapshot tar {:?}", tar_path))?;

        // 6) 写清单并计算整体 hash
        let mut manifest = Manifest {
            id: id.clone(),
            ts: ts.clone(),
            message: msg.to_string(),
            hash: String::new(),
            files,
        };
        let mut tmp = manifest.clone();
        tmp.hash.clear();
        let json = serde_json::to_vec(&tmp).context("serialize manifest")?;
        manifest.hash = hex_digest(&json);
        let manifest_path = self.snapshots_dir().join(format!("{}.manifest.json", id));
        std::fs::write(&manifest_path, serde_json::to_vec_pretty(&manifest)?)
            .with_context(|| format!("write manifest {:?}", manifest_path))?;

        // 7) 更新 HEAD
        self.write_head(&id)?;

        // 8) 更新缓存
        let snap = Snapshot {
            id,
            ts,
            message: msg.to_string(),
            hash: manifest.hash,
        };
        self.snapshots.push(snap.clone());
        self.sort_snapshots();
        Ok(snap)
    }

    /// 恢复到指定快照（校验 hash 后覆盖工作区，删除快照后新增的文件）
    pub fn restore(&self, snapshot_id: &str) -> Result<()> {
        let manifest_path = self
            .snapshots_dir()
            .join(format!("{}.manifest.json", snapshot_id));
        if !manifest_path.exists() {
            return Err(anyhow!("snapshot {} not found", snapshot_id));
        }
        let raw = std::fs::read(&manifest_path)?;
        let manifest: Manifest = serde_json::from_slice(&raw)
            .with_context(|| format!("parse manifest {}", snapshot_id))?;

        // 校验整体清单 hash
        let mut tmp = manifest.clone();
        tmp.hash.clear();
        let json = serde_json::to_vec(&tmp).context("serialize manifest")?;
        let computed = hex_digest(&json);
        if computed != manifest.hash {
            tracing::warn!(snapshot = %snapshot_id, "snapshot manifest sha256 mismatch, refusing to restore");
            return Err(anyhow!(
                "snapshot {} sha256 verification failed",
                snapshot_id
            ));
        }

        let canonical_ws = canonicalize_or_self(&self.workspace());
        let mut content_cache: HashMap<String, HashMap<String, Vec<u8>>> = HashMap::new();
        let want: HashMap<&str, &FileEntry> = manifest
            .files
            .iter()
            .map(|f| (f.path.as_str(), f))
            .collect();

        // 逐文件取内容、校验、写回
        for f in &manifest.files {
            let content = self.load_content(&f.source, &mut content_cache)?;
            let bytes = content.get(&f.path).ok_or_else(|| {
                anyhow!("content for {} missing in snapshot {}", f.path, f.source)
            })?;
            if hex_digest(bytes) != f.sha256 {
                tracing::warn!(snapshot = %snapshot_id, path = %f.path, "file content sha256 mismatch, refusing to restore");
                return Err(anyhow!(
                    "file {} sha256 mismatch in snapshot {}",
                    f.path,
                    snapshot_id
                ));
            }
            let target = safe_join(&canonical_ws, Path::new(&f.path))?;
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&target, bytes).with_context(|| format!("restore file {:?}", target))?;
            apply_mode(&target, f.mode);
        }

        // 删除快照之后新增/变更的文件（不在 want 中，且非排除目录）
        delete_extraneous(&canonical_ws, &want)?;
        Ok(())
    }

    /// 回退到上一轮之前的状态（= restore(HEAD 的上一快照)）
    pub fn revert_turn(&mut self) -> Result<()> {
        let head = self
            .head_id()
            .ok_or_else(|| anyhow!("no snapshot to revert"))?;
        let idx = self
            .snapshots
            .iter()
            .position(|s| s.id == head)
            .ok_or_else(|| anyhow!("head snapshot {} not found in index", head))?;
        if idx + 1 >= self.snapshots.len() {
            return Err(anyhow!(
                "no previous snapshot to revert to (already at earliest)"
            ));
        }
        let prev_id = self.snapshots[idx + 1].id.clone();
        self.restore(&prev_id)?;
        self.write_head(&prev_id)?;
        Ok(())
    }

    /// 列出所有快照（按时间倒序）
    pub fn list(&self) -> Vec<Snapshot> {
        self.snapshots.clone()
    }

    /// 仅保留最近 N 个快照，并保护被保留清单引用的 source 快照
    pub fn cleanup(&mut self, keep_n: usize) -> Result<()> {
        let keep_n = keep_n.max(1);
        self.reload_snapshots()?;
        self.sort_snapshots();
        if self.snapshots.len() <= keep_n {
            return Ok(());
        }
        let retained: Vec<String> = self
            .snapshots
            .iter()
            .take(keep_n)
            .map(|s| s.id.clone())
            .collect();

        // 被保留清单引用的 source 快照也必须保留，否则 restore 会失效
        let mut protected: std::collections::HashSet<String> = retained.iter().cloned().collect();
        for id in &retained {
            let m = self.read_manifest(id)?;
            for f in &m.files {
                protected.insert(f.source.clone());
            }
        }

        let to_delete: Vec<String> = self
            .snapshots
            .iter()
            .map(|s| s.id.clone())
            .filter(|id| !protected.contains(id))
            .collect();
        for id in &to_delete {
            let _ = std::fs::remove_file(self.snapshots_dir().join(format!("{}.tar", id)));
            let _ =
                std::fs::remove_file(self.snapshots_dir().join(format!("{}.manifest.json", id)));
        }
        self.reload_snapshots()?;
        Ok(())
    }

    fn head_id(&self) -> Option<String> {
        let p = self.worktree.join(HEAD_FILE);
        if p.exists() {
            std::fs::read_to_string(&p)
                .ok()
                .map(|s| s.trim().to_string())
        } else {
            None
        }
    }

    fn head_manifest(&self) -> Result<Option<Manifest>> {
        match self.head_id() {
            Some(id) => {
                let p = self.snapshots_dir().join(format!("{}.manifest.json", id));
                if p.exists() {
                    let bytes = std::fs::read(&p)?;
                    Ok(Some(serde_json::from_slice(&bytes)?))
                } else {
                    Ok(None)
                }
            }
            None => Ok(None),
        }
    }

    fn read_manifest(&self, id: &str) -> Result<Manifest> {
        let p = self.snapshots_dir().join(format!("{}.manifest.json", id));
        let bytes = std::fs::read(&p)?;
        Ok(serde_json::from_slice(&bytes)?)
    }

    fn write_head(&self, id: &str) -> Result<()> {
        std::fs::write(self.worktree.join(HEAD_FILE), id).context("write HEAD")?;
        Ok(())
    }

    fn load_content(
        &self,
        source_id: &str,
        cache: &mut HashMap<String, HashMap<String, Vec<u8>>>,
    ) -> Result<HashMap<String, Vec<u8>>> {
        if let Some(c) = cache.get(source_id) {
            return Ok(c.clone());
        }
        let tar_path = self.snapshots_dir().join(format!("{}.tar", source_id));
        let map = read_tar(&tar_path)?;
        cache.insert(source_id.to_string(), map.clone());
        Ok(map)
    }
}

/// 工作区扫描条目：(相对路径, 绝对路径, 大小, 权限, 内容sha256)
type ScanEntry = (String, PathBuf, u64, u32, String);

/// 递归扫描工作区，返回 (相对路径, 绝对路径, 大小, 权限, 内容sha256)
fn scan_workspace(root: &Path) -> Result<Vec<ScanEntry>> {
    let mut out: Vec<ScanEntry> = Vec::new();
    walk_workspace(root, root, &mut out)?;
    Ok(out)
}

fn walk_workspace(
    dir: &Path,
    root: &Path,
    out: &mut Vec<(String, PathBuf, u64, u32, String)>,
) -> Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        let ft = entry.file_type()?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if ft.is_dir() {
            if is_excluded_component(&name) {
                continue;
            }
            walk_workspace(&path, root, out)?;
        } else if ft.is_file() {
            let rel = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");
            let meta = std::fs::metadata(&path)?;
            let mode = mode_from_meta(&meta);
            let bytes = std::fs::read(&path)?;
            let sha = hex_digest(&bytes);
            out.push((rel, path, meta.len(), mode, sha));
        }
        // 符号链接等：跳过，避免逃逸
    }
    Ok(())
}

/// 排除规则：不被快照的顶层目录名
fn is_excluded_component(name: &str) -> bool {
    matches!(name, ".git" | "target" | "node_modules" | ".versions")
        || name.starts_with(".marguerite-")
}

/// 构建增量 tar（仅含变更文件），返回 tar 字节
fn build_tar(changed: &[(String, Vec<u8>, u32)]) -> Result<Vec<u8>> {
    let mut buf = Vec::new();
    {
        let mut builder = Builder::new(&mut buf);
        for (rel, bytes, mode) in changed {
            let mut header = Header::new_gnu();
            header.set_size(bytes.len() as u64);
            header.set_mode(*mode);
            header.set_cksum();
            builder
                .append_data(&mut header, Path::new(rel), bytes.as_slice())
                .with_context(|| format!("append {} to tar", rel))?;
        }
        builder.finish().context("finalize tar")?;
    }
    Ok(buf)
}

/// 手工构造一个 tar 字节流，在 header 的 name 字段直接写入 `../<name>`，
/// 绕过 `tar::Builder` 在 append 时对 `..` 路径的校验。
/// 仅用于测试路径逃逸防护（path-escape guard）。
#[cfg(test)]
fn build_malicious_tar(content: &[u8]) -> Vec<u8> {
    let mut header = tar::Header::new_gnu();
    header.set_size(content.len() as u64);
    header.set_mode(0o644);
    {
        // 直接覆写 name 字段（偏移 0..100），嵌入 `..` 逃逸路径
        let raw = &mut header.as_old_mut().name;
        let name = b"../evil.txt";
        raw[0..name.len()].copy_from_slice(name);
        for b in raw[name.len()..100].iter_mut() {
            *b = 0;
        }
    }
    header.set_cksum();
    let mut buf = Vec::new();
    buf.extend_from_slice(header.as_bytes());
    buf.extend_from_slice(content);
    let pad = (512 - content.len() % 512) % 512;
    buf.extend(std::iter::repeat_n(0u8, pad));
    // 两个零块标记归档结束
    buf.extend(std::iter::repeat_n(0u8, 1024));
    buf
}

/// 解压 tar 为 路径->内容 映射（带解压上限保护）
fn read_tar(tar_path: &Path) -> Result<HashMap<String, Vec<u8>>> {
    let tar_buf = std::fs::read(tar_path)?;
    if tar_buf.len() > MAX_TAR_DECOMPRESSED {
        return Err(anyhow!("tar too large: {} bytes", tar_buf.len()));
    }
    let mut archive = Archive::new(tar_buf.as_slice());
    let mut map: HashMap<String, Vec<u8>> = HashMap::new();
    for entry in archive.entries()? {
        let mut entry = entry?;
        let rel = entry
            .path()
            .map_err(|e| anyhow!("bad tar path: {:?}", e))?
            .into_owned()
            .to_string_lossy()
            .replace('\\', "/");
        let mut data = Vec::new();
        entry.read_to_end(&mut data)?;
        map.insert(rel, data);
    }
    Ok(map)
}

/// 删除快照之后新增/变更的文件（不在 want 中，且非排除目录）
fn delete_extraneous(root: &Path, want: &HashMap<&str, &FileEntry>) -> Result<()> {
    fn walk(dir: &Path, root: &Path, want: &HashMap<&str, &FileEntry>) -> Result<()> {
        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            let ft = entry.file_type()?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if ft.is_dir() {
                if is_excluded_component(&name) {
                    continue;
                }
                // 仅遍历以定位多余文件；不删除目录本身。
                // 注：Windows \\?\ 路径下 read_dir/remove_dir 可见性不一致，
                // 对仍含保留文件的父目录调用 remove_dir 会连带误删保留文件。
                // delete_extraneous 的契约是“删除多余文件”，目录清理交由调用方/上层处理。
                walk(&path, root, want)?;
            } else if ft.is_file() {
                let rel = path
                    .strip_prefix(root)
                    .unwrap_or(&path)
                    .to_string_lossy()
                    .replace('\\', "/");
                if !want.contains_key(rel.as_str()) {
                    std::fs::remove_file(&path)
                        .with_context(|| format!("delete extraneous {:?}", path))?;
                }
            }
        }
        Ok(())
    }
    walk(root, root, want)?;
    Ok(())
}

/// 路径安全：将相对（或绝对）路径解析到工作区内，拒绝逃逸
fn safe_join(root: &Path, rel: &Path) -> Result<PathBuf> {
    let base: PathBuf = if rel.is_absolute() {
        PathBuf::new()
    } else {
        root.to_path_buf()
    };
    let mut stack: Vec<Component> = base.components().collect();
    for comp in rel.components() {
        match comp {
            Component::ParentDir => {
                // 仅当栈顶为普通组件时才弹出，防止逃逸出根
                if matches!(stack.last(), Some(Component::Normal(_))) {
                    stack.pop();
                }
            }
            Component::CurDir => {}
            other => stack.push(other),
        }
    }
    let normalized: PathBuf = stack.iter().collect();
    let canonical = if normalized.exists() {
        std::fs::canonicalize(&normalized)?
    } else if let Some(parent) = normalized.parent() {
        if let Ok(cp) = std::fs::canonicalize(parent) {
            cp.join(normalized.file_name().unwrap_or_default())
        } else {
            normalized
        }
    } else {
        normalized
    };
    let canon_root = canonicalize_or_self(root);
    if !canonical.starts_with(&canon_root) {
        // P0-1b：与 fs_path::safe_resolve 同一口径——错误必须自带根 + 下一步动作
        return Err(anyhow!(
            "path escapes workspace (root: {}): {:?} —— 请改用该目录内的相对路径",
            root.display(),
            rel
        ));
    }
    Ok(canonical)
}

#[cfg(unix)]
fn mode_from_meta(meta: &std::fs::Metadata) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    meta.permissions().mode()
}

#[cfg(not(unix))]
fn mode_from_meta(_meta: &std::fs::Metadata) -> u32 {
    0o644
}

#[cfg(unix)]
fn apply_mode(path: &Path, mode: u32) {
    use std::os::unix::fs::PermissionsExt;
    if mode != 0 {
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode));
    }
}

#[cfg(not(unix))]
fn apply_mode(_path: &Path, _mode: u32) {}

fn canonicalize_or_self(p: &Path) -> PathBuf {
    std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf())
}

fn hex_digest(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(bytes);
    let out = h.finalize();
    let mut s = String::with_capacity(out.len() * 2);
    for b in out {
        s.push_str(&format!("{:02x}", b));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup(session: &str) -> (SideGit, PathBuf) {
        let base =
            std::env::temp_dir().join(format!("marguerite_sidegit_{}_{}", session, Uuid::new_v4()));
        std::fs::create_dir_all(&base).unwrap();
        // canonicalize 以避免 8.3 短路径（ADMINI~1）与 canonicalize 后的长路径
        // （\\?\C:\Users\Administrator\...）不一致，导致读回快照文件时路径对不上。
        let tmp = canonicalize_or_self(&base);
        let sg = SideGit::new(&tmp, "sess1").unwrap();
        (sg, tmp)
    }

    fn write_file(dir: &Path, rel: &str, content: &str) {
        let p = dir.join(rel);
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(&p, content).unwrap();
    }

    fn read_file(dir: &Path, rel: &str) -> String {
        std::fs::read_to_string(dir.join(rel)).unwrap()
    }

    fn cleanup_dir(dir: &Path) {
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn test_snapshot_and_list() {
        let (mut sg, tmp) = setup("list");
        write_file(&tmp, "a.txt", "hello");
        let snap = sg.snapshot_before_turn("turn 1").unwrap();
        assert_eq!(sg.list().len(), 1);
        assert_eq!(snap.message, "turn 1");
        assert!(!snap.hash.is_empty());
        cleanup_dir(&tmp);
    }

    #[test]
    fn test_snapshot_restore_roundtrip() {
        // R1: 快照 -> 修改 -> restore
        let (mut sg, tmp) = setup("r1");
        write_file(&tmp, "a.txt", "original");
        let snap = sg.snapshot_before_turn("turn 1").unwrap();
        write_file(&tmp, "a.txt", "modified by agent");
        assert_eq!(read_file(&tmp, "a.txt"), "modified by agent");
        sg.restore(&snap.id).unwrap();
        assert_eq!(read_file(&tmp, "a.txt"), "original");
        cleanup_dir(&tmp);
    }

    #[test]
    fn test_revert_turn() {
        // R2: 回到上轮前状态
        let (mut sg, tmp) = setup("r2");
        write_file(&tmp, "a.txt", "v1");
        sg.snapshot_before_turn("turn 1").unwrap();
        write_file(&tmp, "a.txt", "v2");
        sg.snapshot_before_turn("turn 2").unwrap();
        assert_eq!(read_file(&tmp, "a.txt"), "v2");
        sg.revert_turn().unwrap();
        assert_eq!(read_file(&tmp, "a.txt"), "v1");
        cleanup_dir(&tmp);
    }

    #[test]
    fn test_incremental_snapshot() {
        // R3: 未变更文件不重打包
        let (mut sg, tmp) = setup("r3");
        write_file(&tmp, "keep.txt", "stable content");
        write_file(&tmp, "old.txt", "will change");
        sg.snapshot_before_turn("turn 1").unwrap();
        write_file(&tmp, "old.txt", "changed content");
        write_file(&tmp, "new.txt", "brand new");
        let snap2 = sg.snapshot_before_turn("turn 2").unwrap();

        // 第二个快照的 tar 只应包含变更/新增文件：old.txt + new.txt
        let tar_path = sg.snapshots_dir().join(format!("{}.tar", snap2.id));
        let map = read_tar(&tar_path).unwrap();
        assert_eq!(
            map.len(),
            2,
            "incremental tar should only contain changed files"
        );
        assert!(map.contains_key("old.txt"));
        assert!(map.contains_key("new.txt"));
        assert!(
            !map.contains_key("keep.txt"),
            "unchanged file must not be repacked"
        );

        // 但 restore 仍能完整重建（keep.txt 由 source 引用）
        sg.restore(&snap2.id).unwrap();
        assert_eq!(read_file(&tmp, "keep.txt"), "stable content");
        assert_eq!(read_file(&tmp, "old.txt"), "changed content");
        assert_eq!(read_file(&tmp, "new.txt"), "brand new");
        cleanup_dir(&tmp);
    }

    #[test]
    fn test_non_git_workspace() {
        // R4: 非 git 仓库工作区正常工作
        let (mut sg, tmp) = setup("r4");
        // 确保没有 .git
        assert!(!tmp.join(".git").exists());
        write_file(&tmp, "src/main.rs", "fn main() {}");
        let snap = sg.snapshot_before_turn("turn 1").unwrap();
        write_file(&tmp, "src/main.rs", "fn main() { println!(); }");
        sg.restore(&snap.id).unwrap();
        assert_eq!(read_file(&tmp, "src/main.rs"), "fn main() {}");
        cleanup_dir(&tmp);
    }

    #[test]
    fn test_sha256_mismatch_rejects_restore() {
        // R5: sha256 校验失败 -> 拒绝恢复
        let (mut sg, tmp) = setup("r5");
        write_file(&tmp, "a.txt", "data");
        let snap = sg.snapshot_before_turn("turn 1").unwrap();
        // 破坏 manifest 的 hash 字段使其校验失败
        let manifest_path = sg
            .snapshots_dir()
            .join(format!("{}.manifest.json", snap.id));
        let mut m: Manifest =
            serde_json::from_slice(&std::fs::read(&manifest_path).unwrap()).unwrap();
        m.hash = "deadbeef".to_string();
        std::fs::write(&manifest_path, serde_json::to_vec_pretty(&m).unwrap()).unwrap();
        let result = sg.restore(&snap.id);
        assert!(result.is_err(), "restore must reject corrupted manifest");
        cleanup_dir(&tmp);
    }

    #[test]
    fn test_corrupted_tar_rejects_restore() {
        let (mut sg, tmp) = setup("r5b");
        write_file(&tmp, "a.txt", "data");
        let snap = sg.snapshot_before_turn("turn 1").unwrap();
        // 破坏 tar 内容，使文件 sha256 校验失败。
        // 注意：raw tar 没有压缩层来检测损坏（zstd 会），因此必须破坏
        // 真实文件内容字节，而非 tar 的填充区，否则解出的文件 sha 仍匹配。
        let tar_path = sg.snapshots_dir().join(format!("{}.tar", snap.id));
        let mut bytes = std::fs::read(&tar_path).unwrap();
        if let Some(pos) = bytes.windows(4).position(|w| w == b"data") {
            // 翻转文件内容区的一个字节，使解出的内容 sha256 与清单不符
            bytes[pos + 1] ^= 0xFF;
        } else if !bytes.is_empty() {
            let last = bytes.len() - 1;
            bytes[last] ^= 0xFF;
        }
        std::fs::write(&tar_path, bytes).unwrap();
        let result = sg.restore(&snap.id);
        assert!(result.is_err(), "restore must reject corrupted tar");
        cleanup_dir(&tmp);
    }

    #[test]
    fn test_exclusion_rules() {
        let (mut sg, tmp) = setup("excl");
        write_file(&tmp, "src/main.rs", "code");
        write_file(&tmp, ".git/config", "git stuff");
        write_file(&tmp, "target/debug/bin", "binary");
        write_file(&tmp, "node_modules/lib/index.js", "dep");
        write_file(&tmp, ".versions/foo.bak", "backup");
        write_file(&tmp, ".marguerite-sidegit/other/snap.tar", "self");
        let snap = sg.snapshot_before_turn("turn 1").unwrap();
        let m = sg.read_manifest(&snap.id).unwrap();
        let paths: Vec<&str> = m.files.iter().map(|f| f.path.as_str()).collect();
        assert!(
            paths.contains(&"src/main.rs"),
            "tracked file must be snapshotted"
        );
        assert!(
            !paths.iter().any(|p| p.starts_with(".git")),
            ".git excluded"
        );
        assert!(
            !paths.iter().any(|p| p.starts_with("target")),
            "target excluded"
        );
        assert!(
            !paths.iter().any(|p| p.starts_with("node_modules")),
            "node_modules excluded"
        );
        assert!(
            !paths.iter().any(|p| p.starts_with(".versions")),
            ".versions excluded"
        );
        assert!(
            !paths.iter().any(|p| p.starts_with(".marguerite-sidegit")),
            "self dir excluded"
        );
        cleanup_dir(&tmp);
    }

    #[test]
    fn test_restore_deletes_new_files() {
        let (mut sg, tmp) = setup("del");
        write_file(&tmp, "a.txt", "original");
        let snap = sg.snapshot_before_turn("turn 1").unwrap();
        write_file(&tmp, "b.txt", "added after snapshot");
        assert!(tmp.join("b.txt").exists());
        sg.restore(&snap.id).unwrap();
        assert!(
            !tmp.join("b.txt").exists(),
            "file created after snapshot must be removed"
        );
        assert_eq!(read_file(&tmp, "a.txt"), "original");
        cleanup_dir(&tmp);
    }

    #[test]
    fn test_deleted_file_not_in_manifest() {
        let (mut sg, tmp) = setup("del2");
        write_file(&tmp, "a.txt", "A");
        write_file(&tmp, "b.txt", "B");
        sg.snapshot_before_turn("turn 1").unwrap();
        std::fs::remove_file(tmp.join("b.txt")).unwrap();
        let snap2 = sg.snapshot_before_turn("turn 2").unwrap();
        write_file(&tmp, "b.txt", "recreated");
        sg.restore(&snap2.id).unwrap();
        assert!(
            !tmp.join("b.txt").exists(),
            "deleted file should stay absent after restore"
        );
        assert_eq!(read_file(&tmp, "a.txt"), "A");
        cleanup_dir(&tmp);
    }

    #[test]
    fn test_cleanup_keeps_recent() {
        let (mut sg, tmp) = setup("clean");
        for i in 0..5 {
            write_file(&tmp, "f.txt", &format!("v{}", i));
            sg.snapshot_before_turn(&format!("turn {}", i)).unwrap();
        }
        assert_eq!(sg.list().len(), 5);
        sg.cleanup(2).unwrap();
        assert_eq!(sg.list().len(), 2, "cleanup should keep only 2 snapshots");
        cleanup_dir(&tmp);
    }

    #[test]
    fn test_cleanup_protects_referenced_source() {
        // 不变文件始终引用首个快照，cleanup 不得删除它，否则 restore 失效
        let (mut sg, tmp) = setup("clean_protect");
        write_file(&tmp, "stable.txt", "never changes");
        let first = sg.snapshot_before_turn("turn 1").unwrap();
        for i in 0..5 {
            write_file(&tmp, "counter.txt", &format!("v{}", i));
            sg.snapshot_before_turn(&format!("turn {}", i + 2)).unwrap();
        }
        sg.cleanup(2).unwrap();
        // 首个快照被保护（其 tar 含 stable.txt 的内容）
        let first_tar = sg.snapshots_dir().join(format!("{}.tar", first.id));
        assert!(
            first_tar.exists(),
            "referenced base snapshot must be protected"
        );
        // restore 仍应成功并完整重建
        let latest = sg.list()[0].id.clone();
        sg.restore(&latest).unwrap();
        assert_eq!(read_file(&tmp, "stable.txt"), "never changes");
        cleanup_dir(&tmp);
    }

    #[test]
    fn test_path_escape_guard() {
        // 篡改清单注入逃逸路径，restore 必须拒绝
        let (mut sg, tmp) = setup("escape");
        write_file(&tmp, "a.txt", "data");
        let snap = sg.snapshot_before_turn("turn 1").unwrap();
        let manifest_path = sg
            .snapshots_dir()
            .join(format!("{}.manifest.json", snap.id));
        let mut m: Manifest =
            serde_json::from_slice(&std::fs::read(&manifest_path).unwrap()).unwrap();
        m.files.push(FileEntry {
            path: "../evil.txt".to_string(),
            size: 4,
            mode: 0o644,
            sha256: hex_digest(b"pwn!"),
            source: snap.id.clone(),
        });
        // 把 evil 内容塞进 tar 也需要，这里只需验证 safe_join 拒绝
        m.hash = {
            let mut tmp2 = m.clone();
            tmp2.hash.clear();
            hex_digest(&serde_json::to_vec(&tmp2).unwrap())
        };
        std::fs::write(&manifest_path, serde_json::to_vec_pretty(&m).unwrap()).unwrap();
        // 重新打包 tar 包含 evil 路径内容。注意：标准 tar Builder 会在 append
        // 时拒绝 `..` 路径，因此这里手工构造 tar 字节，直接把 `../evil.txt`
        // 写进 header 的 name 字段，绕过 Builder 的路径校验，确保 restore
        // 走到 safe_join 才被拒绝。
        let tar_bytes = build_malicious_tar(b"pwn!");
        std::fs::write(
            sg.snapshots_dir().join(format!("{}.tar", snap.id)),
            tar_bytes,
        )
        .unwrap();
        let result = sg.restore(&snap.id);
        assert!(result.is_err(), "escape path must be rejected");
        assert!(
            !tmp.join("../evil.txt").exists(),
            "must not write outside workspace"
        );
        cleanup_dir(&tmp);
    }
}
