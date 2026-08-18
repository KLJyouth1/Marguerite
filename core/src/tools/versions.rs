//! 文件级版本快照（.versions/）
//!
//! `edit_file` / `write_code` 前自动复制原文件到 `.versions/`，支持单文件历史回滚。
//! 路径操作均 canonicalize 并校验不逃逸工作区。

use std::path::{Component, Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use chrono::Utc;
use sha2::{Digest, Sha256};

const VERSIONS_DIR: &str = ".versions";
const KEEP_VERSIONS: usize = 20;

/// 单文件版本元信息
#[derive(Debug, Clone)]
pub struct VersionInfo {
    /// 时间戳（ISO 8601）
    pub ts: String,
    /// 相对工作区的文件路径
    pub path: String,
    /// 版本字节大小
    pub size_bytes: u64,
    /// 内容 sha256
    pub hash: String,
}

/// 文件级版本管理器
///
/// `versions_dir` 指向 `$workspace/.versions/`，工作区根目录为其上级目录。
pub struct VersionManager {
    /// $workspace/.versions/
    versions_dir: PathBuf,
}

impl VersionManager {
    /// 基于工作区根目录构造管理器
    pub fn new(workspace: &Path) -> Result<Self> {
        let versions_dir = workspace.join(VERSIONS_DIR);
        std::fs::create_dir_all(&versions_dir)
            .with_context(|| format!("create .versions dir {:?}", versions_dir))?;
        Ok(Self { versions_dir })
    }

    fn workspace(&self) -> PathBuf {
        self.versions_dir
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| self.versions_dir.clone())
    }

    /// 编辑文件前调用：复制当前版本到 .versions/
    pub fn snapshot_file(&self, path: &Path) -> Result<VersionInfo> {
        let ws = self.workspace();
        let ws_can = canonicalize_or_self(&ws);
        let abs = safe_join(&ws, path)?;
        if !abs.exists() {
            return Err(anyhow!("cannot snapshot non-existent file: {:?}", path));
        }
        if !abs.is_file() {
            return Err(anyhow!("not a regular file: {:?}", path));
        }
        let rel = abs
            .strip_prefix(&ws_can)
            .unwrap_or(&abs)
            .to_string_lossy()
            .replace('\\', "/");

        let now = Utc::now();
        let file_ts = now.format("%Y%m%dT%H%M%SZ").to_string();
        let enc = encode_path(&rel);

        // 同秒防冲突的唯一文件名
        let mut bak_name = format!("{}.{}.bak", enc, file_ts);
        let mut counter = 1;
        while self.versions_dir.join(&bak_name).exists() {
            bak_name = format!("{}.{}.{}.bak", enc, file_ts, counter);
            counter += 1;
        }
        let bak_path = self.versions_dir.join(&bak_name);
        std::fs::copy(&abs, &bak_path).with_context(|| "copy version file")?;

        let meta = std::fs::metadata(&bak_path)?;
        let bytes = std::fs::read(&bak_path)?;
        let hash = hex_digest(&bytes);

        // 自动清理：每个文件保留最近 KEEP_VERSIONS 个版本
        self.auto_cleanup(&rel)?;

        Ok(VersionInfo {
            ts: now.to_rfc3339(),
            path: rel,
            size_bytes: meta.len(),
            hash,
        })
    }

    /// 列出某文件的所有版本（按时间倒序）
    pub fn list_versions(&self, path: &Path) -> Result<Vec<VersionInfo>> {
        let ws = self.workspace();
        let ws_can = canonicalize_or_self(&ws);
        let abs = safe_join(&ws, path)?;
        let rel = abs
            .strip_prefix(&ws_can)
            .unwrap_or(&abs)
            .to_string_lossy()
            .replace('\\', "/");
        let enc = encode_path(&rel);

        let mut out: Vec<VersionInfo> = Vec::new();
        if !self.versions_dir.exists() {
            return Ok(out);
        }
        for entry in std::fs::read_dir(&self.versions_dir)? {
            let entry = entry?;
            let p = entry.path();
            let name = p
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("")
                .to_string();
            let ts_str = match extract_ts(&name, &enc) {
                Some(t) => t,
                None => continue,
            };
            let dt = chrono::DateTime::parse_from_str(&ts_str, "%Y%m%dT%H%M%SZ")
                .map(|d| d.to_rfc3339())
                .unwrap_or_else(|_| ts_str.to_string());
            let meta = std::fs::metadata(&p)?;
            let bytes = std::fs::read(&p)?;
            out.push(VersionInfo {
                ts: dt,
                path: rel.clone(),
                size_bytes: meta.len(),
                hash: hex_digest(&bytes),
            });
        }
        out.sort_by(|a, b| b.ts.cmp(&a.ts));
        Ok(out)
    }

    /// 回滚到指定版本（ts 为 `list_versions` 返回的 ISO 时间戳）
    pub fn rollback(&self, path: &Path, ts: &str) -> Result<()> {
        let ws = self.workspace();
        let ws_can = canonicalize_or_self(&ws);
        let abs = safe_join(&ws, path)?;
        let rel = abs
            .strip_prefix(&ws_can)
            .unwrap_or(&abs)
            .to_string_lossy()
            .replace('\\', "/");
        let enc = encode_path(&rel);

        let bak = self
            .find_bak(&enc, ts)?
            .ok_or_else(|| anyhow!("no version for {:?} at {}", path, ts))?;
        if let Some(parent) = abs.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::copy(&bak, &abs).with_context(|| "rollback copy")?;
        Ok(())
    }

    fn find_bak(&self, enc: &str, ts: &str) -> Result<Option<PathBuf>> {
        let want_compact = compact_ts(ts);
        if !self.versions_dir.exists() {
            return Ok(None);
        }
        let mut candidates: Vec<(PathBuf, std::time::SystemTime)> = Vec::new();
        for entry in std::fs::read_dir(&self.versions_dir)? {
            let entry = entry?;
            let p = entry.path();
            let name = p
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("")
                .to_string();
            let ts_str = match extract_ts(&name, enc) {
                Some(t) => t,
                None => continue,
            };
            if let Some(want) = &want_compact {
                if ts_str != want.as_str() {
                    continue;
                }
            }
            let mtime = std::fs::metadata(&p)?
                .modified()
                .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
            candidates.push((p, mtime));
        }
        candidates.sort_by_key(|(_, t)| *t);
        Ok(candidates.into_iter().last().map(|(p, _)| p))
    }

    fn auto_cleanup(&self, rel: &str) -> Result<()> {
        let enc = encode_path(rel);
        let mut files: Vec<(PathBuf, std::time::SystemTime)> = Vec::new();
        if !self.versions_dir.exists() {
            return Ok(());
        }
        for entry in std::fs::read_dir(&self.versions_dir)? {
            let entry = entry?;
            let p = entry.path();
            let name = p
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("")
                .to_string();
            if !name.starts_with(&enc) || !name.ends_with(".bak") {
                continue;
            }
            let mtime = std::fs::metadata(&p)?
                .modified()
                .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
            files.push((p, mtime));
        }
        if files.len() <= KEEP_VERSIONS {
            return Ok(());
        }
        files.sort_by_key(|(_, t)| *t);
        let excess = files.len() - KEEP_VERSIONS;
        for (p, _) in files.iter().take(excess) {
            let _ = std::fs::remove_file(p);
        }
        Ok(())
    }
}

/// 从 .bak 文件名中提取紧凑时间戳（匹配 `<enc>.<ts>[.i].bak`）
fn extract_ts(name: &str, enc: &str) -> Option<String> {
    if !name.starts_with(enc) {
        return None;
    }
    let rest = &name[enc.len()..];
    if !rest.starts_with('.') {
        return None;
    }
    let tail = &rest[1..];
    if !tail.ends_with(".bak") {
        return None;
    }
    let ts_part = &tail[..tail.len() - 4];
    let ts_str = ts_part.split('.').next().unwrap_or(ts_part);
    Some(ts_str.to_string())
}

fn compact_ts(iso: &str) -> Option<String> {
    let dt = chrono::DateTime::parse_from_rfc3339(iso).ok()?;
    Some(dt.format("%Y%m%dT%H%M%SZ").to_string())
}

/// 将相对路径编码为文件名安全的形式（'/' 也编码，避免跨目录）
fn encode_path(rel: &str) -> String {
    let mut s = String::new();
    for c in rel.bytes() {
        match c {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'.' | b'-' | b'_' => s.push(c as char),
            _ => s.push_str(&format!("%{:02X}", c)),
        }
    }
    s
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
        return Err(anyhow!("path escapes workspace: {:?}", rel));
    }
    Ok(canonical)
}

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
    use uuid::Uuid;

    fn setup(name: &str) -> (VersionManager, PathBuf) {
        let tmp =
            std::env::temp_dir().join(format!("marguerite_versions_{}_{}", name, Uuid::new_v4()));
        std::fs::create_dir_all(&tmp).unwrap();
        let vm = VersionManager::new(&tmp).unwrap();
        (vm, tmp)
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
    fn test_version_snapshot_and_list() {
        let (vm, tmp) = setup("list");
        write_file(&tmp, "a.txt", "v1");
        let info = vm.snapshot_file(Path::new("a.txt")).unwrap();
        assert_eq!(info.path, "a.txt");
        assert_eq!(info.size_bytes, 2);
        assert!(!info.hash.is_empty());
        let versions = vm.list_versions(Path::new("a.txt")).unwrap();
        assert_eq!(versions.len(), 1);
        cleanup_dir(&tmp);
    }

    #[test]
    fn test_version_rollback() {
        let (vm, tmp) = setup("rollback");
        write_file(&tmp, "a.txt", "original");
        let info = vm.snapshot_file(Path::new("a.txt")).unwrap();
        write_file(&tmp, "a.txt", "modified");
        assert_eq!(read_file(&tmp, "a.txt"), "modified");
        vm.rollback(Path::new("a.txt"), &info.ts).unwrap();
        assert_eq!(read_file(&tmp, "a.txt"), "original");
        cleanup_dir(&tmp);
    }

    #[test]
    fn test_version_auto_cleanup_keeps_20() {
        let (vm, tmp) = setup("cleanup");
        write_file(&tmp, "a.txt", "x");
        for i in 0..25 {
            write_file(&tmp, "a.txt", &format!("v{}", i));
            vm.snapshot_file(Path::new("a.txt")).unwrap();
        }
        let versions = vm.list_versions(Path::new("a.txt")).unwrap();
        assert!(
            versions.len() <= KEEP_VERSIONS,
            "must keep at most {} versions, got {}",
            KEEP_VERSIONS,
            versions.len()
        );
        cleanup_dir(&tmp);
    }

    #[test]
    fn test_version_sha256_and_size() {
        let (vm, tmp) = setup("hash");
        write_file(&tmp, "a.txt", "hello world");
        let info = vm.snapshot_file(Path::new("a.txt")).unwrap();
        assert_eq!(info.size_bytes, 11);
        assert_eq!(info.hash, hex_digest(b"hello world"));
        cleanup_dir(&tmp);
    }

    #[test]
    fn test_version_nonexistent_file() {
        let (vm, tmp) = setup("missing");
        let result = vm.snapshot_file(Path::new("nope.txt"));
        assert!(result.is_err(), "snapshot of missing file must error");
        cleanup_dir(&tmp);
    }

    #[test]
    fn test_version_nested_path() {
        let (vm, tmp) = setup("nested");
        write_file(&tmp, "src/deep/a.txt", "nested");
        let info = vm.snapshot_file(Path::new("src/deep/a.txt")).unwrap();
        assert_eq!(info.path, "src/deep/a.txt");
        let versions = vm.list_versions(Path::new("src/deep/a.txt")).unwrap();
        assert_eq!(versions.len(), 1);
        write_file(&tmp, "src/deep/a.txt", "changed");
        vm.rollback(Path::new("src/deep/a.txt"), &info.ts).unwrap();
        assert_eq!(read_file(&tmp, "src/deep/a.txt"), "nested");
        cleanup_dir(&tmp);
    }

    #[test]
    fn test_version_path_safety() {
        // 回滚目标路径逃逸工作区必须被拒绝
        let (vm, tmp) = setup("escape");
        write_file(&tmp, "a.txt", "data");
        let info = vm.snapshot_file(Path::new("a.txt")).unwrap();
        // 尝试回滚到工作区外的绝对路径
        let outside = tmp.parent().unwrap().join("escaped.txt");
        write_file(&tmp, "a.txt", "new");
        let result = vm.rollback(&outside, &info.ts);
        assert!(result.is_err(), "rollback to outside path must be rejected");
        assert!(!outside.exists(), "must not write outside workspace");
        cleanup_dir(&tmp);
    }
}
