//! 可恢复删除：自有回收站（`.marguerite/trash`）。
//!
//! 语义：Marguerite 自有文件删除工具的默认行为是「删除转回收站、可恢复」；
//! 永久删除必须走显式高危动词（`delete_file_permanent`），不提供静默回退。
//!
//! 设计要点：
//! - **自有 trash store**：`.marguerite/trash/<单调递增 ID>/`，不依赖 OS 回收站 API，
//!   跨平台行为一致、可测试。条目含 `entry.json`（原路径 / 删除时间 / 大小 / 来源工具）
//!   + `payload/`（被删文件或目录本体）。
//! - **JSON 索引**：`.marguerite/trash/index.json` 记录 `next_id`（单调递增）与全部条目。
//! - **恢复校验**：恢复目标必须位于回收站所属工作区内（路径穿越拒绝），
//!   且条目必须属于当前工作区（拒绝跨工作区写穿）。
//! - **冲突安全命名**：恢复目标已存在时自动追加 ` (restored-N)` 后缀，绝不覆盖。
//! - **健壮的移动**：先 `rename`，跨设备/失败时回退 copy+remove；Windows 只读文件
//!   先清除只读位再删；符号链接按链接本体处理，不跟随、不遍历目标。

use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex};

use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};

/// 回收站条目（索引与 `entry.json` 共用同一结构）
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TrashEntry {
    /// 单调递增 ID（同时是回收站内目录名）
    pub id: u64,
    /// 被删前的绝对路径
    pub original_path: String,
    /// 所属工作区（恢复时用于路径穿越校验）
    pub workspace: String,
    /// 删除时间（unix 秒）
    pub deleted_at: i64,
    /// 删除时大小（文件=字节数；目录=递归合计）
    pub size_bytes: u64,
    /// 类型：file / dir / symlink
    pub kind: String,
    /// 来源工具名
    pub source_tool: String,
}

/// 恢复结果
#[derive(Debug, Clone, PartialEq)]
pub struct RestoreOutcome {
    /// 实际恢复落点（冲突时可能是安全命名后的路径）
    pub restored_to: PathBuf,
    /// 是否因冲突而改名
    pub renamed: bool,
}

/// `index.json` 内容
#[derive(Debug, Default, Serialize, Deserialize)]
struct TrashIndex {
    next_id: u64,
    entries: Vec<TrashEntry>,
}

/// 进程级按 trash root 分桶的互斥锁。
///
/// 同一工作区可能被多个工具实例（各自持有 `TrashStore`）并发操作，
/// `index.json` 是共享可变状态，必须按回收站根串行化，否则并发删除会出现
/// ID 抢占 / 索引覆盖丢失。
static TRASH_LOCKS: LazyLock<Mutex<HashMap<PathBuf, Arc<Mutex<()>>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn trash_lock(root: &Path) -> Arc<Mutex<()>> {
    let mut map = TRASH_LOCKS.lock().unwrap_or_else(|e| e.into_inner());
    map.entry(root.to_path_buf()).or_default().clone()
}

/// 工作区内回收站目录：`<workspace>/.marguerite/trash`
pub fn trash_dir(workspace: &Path) -> PathBuf {
    workspace.join(".marguerite").join("trash")
}

/// 自有回收站（绑定一个工作区）。
pub struct TrashStore {
    workspace: PathBuf,
    root: PathBuf,
}

impl TrashStore {
    pub fn new(workspace: &Path) -> Self {
        // 工作区根统一 canonical 化：`delete` 写入条目、`restore` 比对条目时
        // 必须是同一形态，否则 Windows 8.3 短名/符号链接差异会让合法条目被误拒。
        let ws = std::fs::canonicalize(workspace).unwrap_or_else(|_| workspace.to_path_buf());
        let root = trash_dir(&ws);
        Self { workspace: ws, root }
    }

    /// 当前条目清单（按 ID 升序）。
    pub fn list(&self) -> Result<Vec<TrashEntry>> {
        let _guard = trash_lock(&self.root);
        Ok(self.load_index()?.entries)
    }

    /// 删除 → 回收站。返回新条目（含分配的单调 ID）。
    ///
    /// 失败时错误透传，**不静默回退永久删除**——避免「以为进回收站了其实永久删了」。
    pub fn delete(&self, target: &Path, source_tool: &str) -> Result<TrashEntry> {
        let meta = std::fs::symlink_metadata(target)
            .with_context(|| format!("trash target not found: {}", target.display()))?;
        let file_type = meta.file_type();
        let kind = if file_type.is_symlink() {
            "symlink"
        } else if meta.is_dir() {
            "dir"
        } else {
            "file"
        };

        // 拒绝删除回收站自身 / 其任何祖先（含工作区根）——否则「回收站删回收站」
        // 会级联销毁全部可恢复数据。比较用最近存在祖先的 canonical 形态，
        // 避免 8.3 短名 / 符号链接让前缀校验失效。
        let target_can = canonicalize_existing_ancestor(target);
        if is_inside_or_equal(&target_can, &self.root) {
            bail!("refuse to delete the recycle bin itself: {}", target.display());
        }
        if is_inside_or_equal(&self.workspace, &target_can) {
            bail!(
                "refuse to delete a path containing the workspace root: {}",
                target.display()
            );
        }

        let size_bytes = if kind == "dir" {
            dir_size(target)
        } else {
            meta.len()
        };

        let _guard = trash_lock(&self.root);
        let mut index = self.load_index()?;
        let id = index.next_id.max(1);
        index.next_id = id + 1;
        let entry_dir = self.root.join(id.to_string());
        std::fs::create_dir_all(&entry_dir)
            .with_context(|| format!("create trash entry dir {}", entry_dir.display()))?;

        let payload = entry_dir.join("payload");
        if let Err(e) = self.move_into_payload(target, &payload, kind) {
            // 移动失败：清理预建的条目目录，不留孤儿（empty 只清索引内条目）。
            let _ = std::fs::remove_dir_all(&entry_dir);
            return Err(e);
        }

        let entry = TrashEntry {
            id,
            original_path: target.display().to_string(),
            workspace: self.workspace.display().to_string(),
            deleted_at: chrono::Utc::now().timestamp(),
            size_bytes,
            kind: kind.to_string(),
            source_tool: source_tool.to_string(),
        };
        write_json_atomic(&entry_dir.join("entry.json"), &entry)?;
        index.entries.push(entry.clone());
        self.save_index(&index)?;
        Ok(entry)
    }

    /// 按 ID 或原路径恢复。
    ///
    /// - `id_or_path` 为数字 → 按 ID 精确匹配；否则按原路径匹配（多条同路径取最新）。
    /// - 恢复目标必须位于**当前工作区**内（条目记录的 workspace 必须一致，
    ///   original_path 归一化后必须以工作区根为前缀）——路径穿越拒绝。
    /// - 目标已存在 → 安全命名，绝不覆盖。
    pub fn restore(&self, id_or_path: &str, workspace_root: &Path) -> Result<RestoreOutcome> {
        let ws_can = std::fs::canonicalize(workspace_root)
            .unwrap_or_else(|_| workspace_root.to_path_buf());
        let _guard = trash_lock(&self.root);
        let mut index = self.load_index()?;
        let entry = if let Ok(id) = id_or_path.trim().parse::<u64>() {
            index.entries.iter().find(|e| e.id == id).cloned()
        } else {
            // 按原路径匹配（多条同路径取最新）。比较用「最近存在祖先 canonical 化」
            // 后的形态：safe_resolve 产出的 original_path 在 Windows 上带 `\\?\`
            // 扩展长度前缀，与用户传入的普通形态字符串直接相等比较会永远匹配不上。
            let q = canonicalize_existing_ancestor(Path::new(id_or_path.trim()));
            index
                .entries
                .iter()
                .filter(|e| {
                    canonicalize_existing_ancestor(Path::new(&e.original_path)) == q
                })
                .max_by_key(|e| e.id)
                .cloned()
        }
        .ok_or_else(|| anyhow!("trash entry not found: {}", id_or_path))?;

        // 路径穿越 / 跨工作区保护：条目必须属于当前工作区，目标必须落在工作区内。
        if entry.workspace != ws_can.display().to_string() {
            bail!(
                "trash entry {} belongs to workspace {}, not {}",
                entry.id,
                entry.workspace,
                ws_can.display()
            );
        }
        let original = Path::new(&entry.original_path);
        // 词法归一化 + 最近存在祖先 canonical 化，再校验工作区前缀：
        // 「原路径在工作区外」的伪造条目即使写进索引也必须被拒绝。
        let normalized = normalize_lexically(original);
        let target_can = canonicalize_existing_ancestor(&normalized);
        if !target_can.starts_with(&ws_can) {
            bail!(
                "restore target escapes workspace (root: {}): {}",
                ws_can.display(),
                entry.original_path
            );
        }

        let payload = self.root.join(entry.id.to_string()).join("payload");
        if !payload.exists() {
            bail!("trash payload missing for entry {}", entry.id);
        }

        let (restored_to, renamed) = match normalized.exists() {
            false => (normalized.clone(), false),
            true => (safe_restore_target(&normalized), true),
        };
        if let Some(parent) = restored_to.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::rename(&payload, &restored_to).with_context(|| {
            format!(
                "restore move {} -> {}",
                payload.display(),
                restored_to.display()
            )
        })?;

        index.entries.retain(|e| e.id != entry.id);
        self.save_index(&index)?;
        let _ = std::fs::remove_dir(self.root.join(entry.id.to_string()));
        Ok(RestoreOutcome {
            restored_to,
            renamed,
        })
    }

    /// 清空回收站，返回清除的条目数。`next_id` 保持单调不回绕。
    pub fn empty(&self) -> Result<usize> {
        let _guard = trash_lock(&self.root);
        let mut index = self.load_index()?;
        let n = index.entries.len();
        for e in &index.entries {
            let dir = self.root.join(e.id.to_string());
            let _ = clear_readonly_recursively(&dir);
            let _ = std::fs::remove_dir_all(&dir);
        }
        index.entries.clear();
        self.save_index(&index)?;
        Ok(n)
    }

    // ── 内部 ──

    fn load_index(&self) -> Result<TrashIndex> {
        let path = self.root.join("index.json");
        match std::fs::read_to_string(&path) {
            Ok(raw) => {
                serde_json::from_str(&raw).with_context(|| format!("parse {}", path.display()))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(TrashIndex::default()),
            Err(e) => Err(anyhow::Error::from(e))
                .with_context(|| format!("read {}", path.display())),
        }
    }

    fn save_index(&self, index: &TrashIndex) -> Result<()> {
        write_json_atomic(&self.root.join("index.json"), index)
    }

    /// 把 target 移入 payload：rename 优先；失败清只读位重试；跨设备/仍失败 →
    /// copy + remove 兜底。符号链接按链接本体处理（不跟随、不遍历目标内容）。
    fn move_into_payload(&self, target: &Path, payload: &Path, kind: &str) -> Result<()> {
        if let Ok(()) = std::fs::rename(target, payload) {
            return Ok(());
        }
        // 只读文件（Windows 常见）先清只读位再试一次 rename。
        let _ = clear_readonly_recursively(target);
        if std::fs::rename(target, payload).is_ok() {
            return Ok(());
        }
        if kind == "symlink" {
            let link_target = std::fs::read_link(target)
                .with_context(|| format!("read symlink {}", target.display()))?;
            #[cfg(unix)]
            std::os::unix::fs::symlink(&link_target, payload)?;
            #[cfg(windows)]
            {
                if link_target.is_dir() {
                    std::os::windows::fs::symlink_dir(&link_target, payload)?;
                } else {
                    std::os::windows::fs::symlink_file(&link_target, payload)?;
                }
            }
            std::fs::remove_file(target)?;
            return Ok(());
        }
        if kind == "dir" {
            copy_dir_all(target, payload)?;
        } else if let Some(parent) = payload.parent() {
            std::fs::create_dir_all(parent)?;
            std::fs::copy(target, payload)?;
        }
        clear_readonly_recursively(target)?;
        if kind == "dir" {
            std::fs::remove_dir_all(target)?;
        } else {
            std::fs::remove_file(target)?;
        }
        Ok(())
    }
}

/// 递归清只读位（Windows: FILE_ATTRIBUTE_READONLY；Unix: 去掉 u-w 之外的阻塞）。
#[allow(clippy::permissions_set_readonly_false)] // 清只读=可写，正是删除前置；此处语义正确
fn clear_readonly_recursively(p: &Path) -> Result<()> {
    #[cfg(windows)]
    {
        // Windows 下 `PermissionsExt` 不存在（那是 Unix 专属 trait）；`readonly()` /
        // `set_readonly()` 直接在 std::fs::Permissions 上，无需任何扩展 trait。
        if p.is_dir() {
            let mut perms = std::fs::metadata(p)?.permissions();
            perms.set_readonly(false);
            let _ = std::fs::set_permissions(p, perms);
            if let Ok(entries) = std::fs::read_dir(p) {
                for e in entries.flatten() {
                    let _ = clear_readonly_recursively(&e.path());
                }
            }
        } else if p.exists() || p.is_symlink() {
            let mut perms = std::fs::metadata(p)?.permissions();
            if perms.readonly() {
                perms.set_readonly(false);
                let _ = std::fs::set_permissions(p, perms);
            }
        }
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(meta) = std::fs::symlink_metadata(p) {
            if !meta.file_type().is_symlink() {
                let mode = meta.permissions().mode();
                let _ = std::fs::set_permissions(p, std::fs::Permissions::from_mode(mode | 0o200));
                if meta.is_dir() {
                    if let Ok(entries) = std::fs::read_dir(p) {
                        for e in entries.flatten() {
                            let _ = clear_readonly_recursively(&e.path());
                        }
                    }
                }
            }
        }
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = p;
    }
    Ok(())
}

/// 删除前清除目标（递归）只读位，供永久删除路径复用（Windows 只读文件/目录）。
pub fn clear_readonly_for_delete_pub(p: &Path) -> Result<()> {
    clear_readonly_recursively(p)
}

fn copy_dir_all(src: &Path, dst: &Path) -> Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in
        std::fs::read_dir(src).with_context(|| format!("read dir {}", src.display()))?
    {
        let entry = entry?;
        let ty = entry.file_type()?;
        let from = entry.path();
        let to = dst.join(entry.file_name());
        if ty.is_dir() {
            copy_dir_all(&from, &to)?;
        } else if ty.is_symlink() {
            let link_target = std::fs::read_link(&from)?;
            #[cfg(unix)]
            std::os::unix::fs::symlink(&link_target, &to)?;
            #[cfg(windows)]
            {
                if link_target.is_dir() {
                    std::os::windows::fs::symlink_dir(&link_target, &to)?;
                } else {
                    std::os::windows::fs::symlink_file(&link_target, &to)?;
                }
            }
        } else {
            std::fs::copy(&from, &to)?;
        }
    }
    Ok(())
}

/// 目录占用的字节数 = **递归累加其中文件的大小**。
///
/// 不要把目录自身的 `metadata.len()` 加进来：Windows 上目录恒为 0，Linux/ext4 上是
/// 4096 —— 同一个目录在两套宿主上会给出两个数（实测 `8195` vs `3`，CI 的 ubuntu
/// 作业因此判红）。「目录多大」在回收站语境下只有一个有意义的口径：**里面的内容多大**。
fn dir_size(p: &Path) -> u64 {
    let mut total = 0u64;
    if let Ok(entries) = std::fs::read_dir(p) {
        for e in entries.flatten() {
            let path = e.path();
            if path.is_dir() && !path.is_symlink() {
                total = total.saturating_add(dir_size(&path));
            } else if let Ok(m) = std::fs::symlink_metadata(&path) {
                total = total.saturating_add(m.len());
            }
        }
    }
    total
}

/// `a` 是否为 `b` 自身或 `b` 的后代（纯词法，不做 canonicalize）。
fn is_inside_or_equal(a: &Path, b: &Path) -> bool {
    normalize_lexically(a).starts_with(normalize_lexically(b))
}

/// 词法归一化：折叠 `.` 与 `..`（不触盘，用于不存在的目标路径的前缀校验）。
fn normalize_lexically(p: &Path) -> PathBuf {
    let mut stack: Vec<Component> = Vec::new();
    for comp in p.components() {
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
    stack.iter().collect()
}

/// 「存在性感知」的 canonicalize：向上找最近的存在祖先，canonicalize 后拼回
/// 剩余组件。用于目标尚未创建（恢复目标）或需统一形态比较时的前缀校验。
/// 注意：对被删的软链目标调用 `std::fs::canonicalize` 会**跟随链接**，
/// 因此本函数只用于路径形态归一化，不用于决定「删链接还是删目标」。
fn canonicalize_existing_ancestor(p: &Path) -> PathBuf {
    let mut suffix: Vec<std::ffi::OsString> = Vec::new();
    let mut cur = p;
    loop {
        if let Ok(c) = std::fs::canonicalize(cur) {
            let mut out = c;
            for s in suffix.iter().rev() {
                out.push(s);
            }
            return out;
        }
        match (cur.parent(), cur.file_name()) {
            (Some(parent), Some(name)) => {
                suffix.push(name.to_os_string());
                cur = parent;
            }
            _ => return p.to_path_buf(),
        }
    }
}

/// 冲突时生成安全命名：`name.ext` → `name (restored-1).ext`，递增直到空闲。
fn safe_restore_target(original: &Path) -> PathBuf {
    let parent = original.parent().unwrap_or_else(|| Path::new(""));
    let Some(stem) = original.file_stem().and_then(|s| s.to_str()) else {
        return original.to_path_buf();
    };
    let ext = original
        .extension()
        .and_then(|s| s.to_str())
        .map(|e| format!(".{e}"))
        .unwrap_or_default();
    let mut n = 1u32;
    loop {
        let candidate = parent.join(format!("{stem} (restored-{n}){ext}"));
        if !candidate.exists() {
            return candidate;
        }
        n += 1;
    }
}

fn write_json_atomic<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let raw = serde_json::to_vec_pretty(value)?;
    let tmp = path.with_extension(format!("tmp.{}", std::process::id()));
    std::fs::write(&tmp, &raw).with_context(|| format!("write {}", tmp.display()))?;
    std::fs::rename(&tmp, path).with_context(|| format!("commit {}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_ws(tag: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!(
            "mgt_trash_{}_{}_{}",
            tag,
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn delete_moves_file_into_trash_and_records_meta() {
        let ws = temp_ws("del_file");
        std::fs::write(ws.join("a.txt"), "hello").unwrap();
        let store = TrashStore::new(&ws);
        let entry = store.delete(&ws.join("a.txt"), "delete_file").unwrap();

        assert!(!ws.join("a.txt").exists(), "原文件应已移走");
        assert_eq!(entry.id, 1);
        assert_eq!(entry.kind, "file");
        assert_eq!(entry.source_tool, "delete_file");
        assert_eq!(entry.size_bytes, 5);
        assert_eq!(
            entry.original_path,
            ws.join("a.txt").display().to_string()
        );
        // payload 本体在回收站里，内容不丢
        let payload = trash_dir(&ws).join("1").join("payload");
        assert!(payload.exists());
        assert_eq!(std::fs::read_to_string(&payload).unwrap(), "hello");
        // 索引落盘且单调
        assert_eq!(store.list().unwrap().len(), 1);
        let _ = std::fs::remove_dir_all(&ws);
    }

    #[test]
    fn ids_are_monotonic_across_deletes() {
        let ws = temp_ws("mono");
        let store = TrashStore::new(&ws);
        for i in 0..3 {
            std::fs::write(ws.join(format!("f{i}.txt")), "x").unwrap();
            let e = store.delete(&ws.join(format!("f{i}.txt")), "delete_file").unwrap();
            assert_eq!(e.id, i + 1);
        }
        assert_eq!(store.list().unwrap().len(), 3);
        // empty 后 next_id 不回绕
        assert_eq!(store.empty().unwrap(), 3);
        std::fs::write(ws.join("after.txt"), "y").unwrap();
        let e = store.delete(&ws.join("after.txt"), "delete_file").unwrap();
        assert!(e.id >= 4, "empty 后 ID 必须继续单调递增，得到 {}", e.id);
        let _ = std::fs::remove_dir_all(&ws);
    }

    #[test]
    fn restore_by_id_and_by_original_path() {
        let ws = temp_ws("restore");
        std::fs::write(ws.join("a.txt"), "hello").unwrap();
        std::fs::write(ws.join("b.txt"), "world").unwrap();
        let store = TrashStore::new(&ws);
        let ea = store.delete(&ws.join("a.txt"), "t").unwrap();
        let _eb = store.delete(&ws.join("b.txt"), "t").unwrap();

        let out = store.restore(&ea.id.to_string(), &ws).unwrap();
        assert_eq!(out.restored_to, ws.join("a.txt"));
        assert!(!out.renamed);
        assert_eq!(std::fs::read_to_string(ws.join("a.txt")).unwrap(), "hello");

        let out2 = store
            .restore(&ws.join("b.txt").display().to_string(), &ws)
            .unwrap();
        assert_eq!(out2.restored_to, ws.join("b.txt"));
        assert_eq!(std::fs::read_to_string(ws.join("b.txt")).unwrap(), "world");
        // 条目消费后出索引
        assert!(store.list().unwrap().is_empty());
        let _ = std::fs::remove_dir_all(&ws);
    }

    #[test]
    fn restore_conflict_uses_safe_naming_and_never_overwrites() {
        let ws = temp_ws("conflict");
        std::fs::write(ws.join("a.txt"), "old").unwrap();
        let store = TrashStore::new(&ws);
        let e = store.delete(&ws.join("a.txt"), "t").unwrap();
        std::fs::write(ws.join("a.txt"), "new-in-the-way").unwrap();

        let out = store.restore(&e.id.to_string(), &ws).unwrap();
        assert!(out.renamed);
        assert_ne!(out.restored_to, ws.join("a.txt"));
        assert_eq!(
            std::fs::read_to_string(ws.join("a.txt")).unwrap(),
            "new-in-the-way",
            "已在位文件绝不允许被覆盖"
        );
        assert_eq!(
            std::fs::read_to_string(&out.restored_to).unwrap(),
            "old",
            "恢复内容必须来自回收站 payload"
        );
        let _ = std::fs::remove_dir_all(&ws);
    }

    #[test]
    fn restore_by_path_accepts_lexically_different_forms() {
        // 回归：safe_resolve 产出的 original_path 在 Windows 带 `\\?\` 扩展前缀，
        // 与用户传入的普通形态必须仍能匹配（canonical 形态比较，不是字符串相等）。
        let ws = temp_ws("lexform");
        std::fs::write(ws.join("a.txt"), "hello").unwrap();
        let store = TrashStore::new(&ws);
        store.delete(&ws.join("a.txt"), "t").unwrap();
        // `./` 装饰的词法不同形态
        let decorated = format!("{}/./a.txt", ws.display());
        let out = store.restore(&decorated, &ws).unwrap();
        assert_eq!(out.restored_to, ws.join("a.txt"));
        assert_eq!(std::fs::read_to_string(ws.join("a.txt")).unwrap(), "hello");
        let _ = std::fs::remove_dir_all(&ws);
    }

    /// 路径穿越：索引里 original_path 指向工作区外 → 恢复必须拒绝。
    /// （直接伪造索引条目，验证恢复侧校验独立于删除侧防线。）
    #[test]
    fn restore_rejects_path_traversal_outside_workspace() {
        let ws = temp_ws("traversal");
        let outside = temp_ws("outside_target");
        let store = TrashStore::new(&ws);
        // 造一个「原路径在工作区外」的条目：payload 是工作区外的一个真实文件
        let secret = outside.join("secret.txt");
        std::fs::write(&secret, "outside").unwrap();
        let entry = TrashEntry {
            id: 1,
            original_path: secret.display().to_string(),
            workspace: store.workspace.display().to_string(),
            deleted_at: 0,
            size_bytes: 7,
            kind: "file".into(),
            source_tool: "forged".into(),
        };
        std::fs::create_dir_all(trash_dir(&ws).join("1")).unwrap();
        std::fs::write(trash_dir(&ws).join("1").join("payload"), "outside").unwrap();
        store
            .save_index(&TrashIndex {
                next_id: 2,
                entries: vec![entry],
            })
            .unwrap();

        let err = store.restore("1", &ws).unwrap_err().to_string();
        assert!(
            err.contains("escapes workspace"),
            "工作区外恢复目标必须被拒绝: {err}"
        );
        assert!(secret.exists(), "工作区外文件不得被改写");
        let _ = std::fs::remove_dir_all(&ws);
        let _ = std::fs::remove_dir_all(&outside);
    }

    /// 跨工作区：条目属于另一个工作区 → 恢复拒绝。
    #[test]
    fn restore_rejects_cross_workspace_entry() {
        let ws_a = temp_ws("ws_a");
        let ws_b = temp_ws("ws_b");
        let store = TrashStore::new(&ws_a);
        let entry = TrashEntry {
            id: 1,
            original_path: ws_b.join("x.txt").display().to_string(),
            workspace: ws_b.display().to_string(),
            deleted_at: 0,
            size_bytes: 1,
            kind: "file".into(),
            source_tool: "forged".into(),
        };
        std::fs::create_dir_all(trash_dir(&ws_a).join("1")).unwrap();
        std::fs::write(trash_dir(&ws_a).join("1").join("payload"), "x").unwrap();
        store
            .save_index(&TrashIndex {
                next_id: 2,
                entries: vec![entry],
            })
            .unwrap();
        let err = store.restore("1", &ws_a).unwrap_err().to_string();
        assert!(err.contains("belongs to workspace"), "跨工作区恢复必须拒绝: {err}");
        let _ = std::fs::remove_dir_all(&ws_a);
        let _ = std::fs::remove_dir_all(&ws_b);
    }

    #[test]
    fn delete_directory_with_nested_content_roundtrips() {
        let ws = temp_ws("dir");
        std::fs::create_dir_all(ws.join("d/sub")).unwrap();
        std::fs::write(ws.join("d/one.txt"), "1").unwrap();
        std::fs::write(ws.join("d/sub/two.txt"), "22").unwrap();
        let store = TrashStore::new(&ws);
        let e = store.delete(&ws.join("d"), "delete_file").unwrap();
        assert_eq!(e.kind, "dir");
        assert_eq!(e.size_bytes, 3);
        assert!(!ws.join("d").exists());

        let out = store.restore(&e.id.to_string(), &ws).unwrap();
        assert_eq!(out.restored_to, ws.join("d"));
        assert_eq!(std::fs::read_to_string(ws.join("d/sub/two.txt")).unwrap(), "22");
        let _ = std::fs::remove_dir_all(&ws);
    }

    #[test]
    fn empty_clears_payloads_and_index() {
        let ws = temp_ws("empty");
        std::fs::write(ws.join("a.txt"), "a").unwrap();
        let store = TrashStore::new(&ws);
        store.delete(&ws.join("a.txt"), "t").unwrap();
        assert!(trash_dir(&ws).join("1").join("payload").exists());
        assert_eq!(store.empty().unwrap(), 1);
        assert!(!trash_dir(&ws).join("1").exists());
        assert!(store.list().unwrap().is_empty());
        let _ = std::fs::remove_dir_all(&ws);
    }

    /// 删除回收站自身 / 工作区根：拒绝（防止级联销毁可恢复数据）。
    #[test]
    fn refuses_deleting_trash_root_or_workspace_root() {
        let ws = temp_ws("selfdel");
        let store = TrashStore::new(&ws);
        assert!(store.delete(&trash_dir(&ws), "t").is_err());
        assert!(store.delete(&ws, "t").is_err());
        let _ = std::fs::remove_dir_all(&ws);
    }

    #[test]
    fn delete_missing_target_errors() {
        let ws = temp_ws("missing");
        let store = TrashStore::new(&ws);
        assert!(store.delete(&ws.join("nope.txt"), "t").is_err());
        let _ = std::fs::remove_dir_all(&ws);
    }

    #[cfg(unix)]
    #[test]
    fn symlink_deleted_as_link_not_followed() {
        let ws = temp_ws("symlink");
        let outside = temp_ws("symlink_outside");
        std::fs::write(outside.join("target.txt"), "keep me").unwrap();
        let link = ws.join("lnk");
        std::os::unix::fs::symlink(outside.join("target.txt"), &link).unwrap();
        let store = TrashStore::new(&ws);
        let e = store.delete(&link, "t").unwrap();
        assert_eq!(e.kind, "symlink");
        assert!(!link.exists(), "链接本身应被移走");
        assert!(
            outside.join("target.txt").exists(),
            "软链目标绝不能被删除波及"
        );
        let _ = std::fs::remove_dir_all(&ws);
        let _ = std::fs::remove_dir_all(&outside);
    }

    /// 删除中途失败 fail-closed：回收站条目目录被一个普通文件占位，
    /// `create_dir_all(entry_dir)` 必然失败。此时 `delete` 必须返回 Err，
    /// 且**原文件原地保留**——绝不允许「回收站写不进去 → 静默回退硬删」。
    #[test]
    fn delete_failure_never_silently_hard_deletes_the_original() {
        let ws = temp_ws("failclosed");
        std::fs::write(ws.join("a.txt"), "must survive").unwrap();
        let store = TrashStore::new(&ws);

        // 预占 id=1 的条目位置：放一个普通文件，使随后的 create_dir_all 失败。
        let root = trash_dir(&ws);
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("1"), "blocker").unwrap();

        let res = store.delete(&ws.join("a.txt"), "delete_file");
        assert!(
            res.is_err(),
            "回收站条目目录被占时 delete 必须报错，不得假装成功"
        );
        assert!(
            ws.join("a.txt").exists(),
            "fail-closed：删除失败时原文件必须原地保留，绝不静默硬删"
        );
        // 索引不得残留半条记录
        assert!(
            store.list().unwrap().is_empty(),
            "失败的删除不得在索引里留下条目"
        );
        let _ = std::fs::remove_dir_all(&ws);
    }

    #[cfg(windows)]
    #[test]
    fn readonly_file_can_be_trashed() {
        let ws = temp_ws("readonly");
        let f = ws.join("ro.txt");
        std::fs::write(&f, "ro").unwrap();
        let mut perms = std::fs::metadata(&f).unwrap().permissions();
        perms.set_readonly(true);
        std::fs::set_permissions(&f, perms).unwrap();

        let store = TrashStore::new(&ws);
        let e = store.delete(&f, "t").unwrap();
        assert_eq!(e.kind, "file");
        assert!(!f.exists());
        // 恢复回写后内容不变
        let out = store.restore(&e.id.to_string(), &ws).unwrap();
        assert_eq!(std::fs::read_to_string(&out.restored_to).unwrap(), "ro");
        let _ = std::fs::remove_dir_all(&ws);
    }
}
