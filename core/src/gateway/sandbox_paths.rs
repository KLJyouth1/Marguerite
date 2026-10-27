//! 沙箱路径方法族（sandbox.path.*）
//!
//! 提供目录选择器（pick）、目录浏览（list）、新建目录（create-directory）与
//! 有界文件名搜索（search）。纯 Rust 本地实现，不依赖 Python 侧车。

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use crate::proc::SilentCommand;

/// 解析 `sandbox.path.*` 的目标根目录。
///
/// 2026-10-24：此前这里自己写了一份「USERPROFILE → HOME → cwd」的回落，与
/// `workspace::Workspace::resolve`（显式绑定 → 数据目录 → cwd）**是两条根**：
/// 右侧「工作区目录」实际列的是 `C:\Users\<user>`（AppData/Cookies/Desktop…），
/// 而智能体把文件写到了数据目录里。统一委托给唯一解析器，界面与工具行为一致。
pub fn sandbox_path_root() -> PathBuf {
    crate::workspace::Workspace::resolve(None).root
}

/// `sandbox.path.pick`：桌面端原生目录选择器（Rust 本地实现）。
///
/// 历史问题：该 RPC 未在 Rust 实现 → dispatch 回退代理 Python 侧车；侧车在
/// Windows 上 spawn 一个 tkinter 子进程弹「选择目录」对话框，且以
/// CREATE_NO_WINDOW 方式创建，在无窗口会话下永久阻塞 → WS 代理超时
/// （「Python 侧车响应超时（sandbox.path.pick）」），目录选择器无法打开。
///
/// 现改为 Rust 直接以 PowerShell + System.Windows.Forms.FolderBrowserDialog
/// 弹原生目录对话框（STA 线程，与 system_control 同源的纯系统 API）：
/// - 不依赖 Python 侧车（离线也可用）；
/// - 异步 await 子进程，不阻塞网关事件循环；
/// - 用户取消 / 无桌面会话 → 返回 `path: null`，WebUI 回退到路径浏览。
pub async fn sandbox_path_pick(
    params: &Value,
) -> Result<Value, crate::gateway::protocol::RpcError> {
    use crate::gateway::protocol::RpcError;
    if cfg!(not(target_os = "windows")) {
        return Ok(json!({ "path": null, "kind": "workspace" }));
    }
    let initial = params
        .get("initialPath")
        .and_then(Value::as_str)
        .unwrap_or("");
    let script = r#"
$ErrorActionPreference = 'SilentlyContinue'
Add-Type -AssemblyName System.Windows.Forms | Out-Null
$d = New-Object System.Windows.Forms.FolderBrowserDialog
$d.Description = 'Marguerite · 选择项目目录'
$d.ShowNewFolderButton = $true
$initial = $env:MARG_PICK_INITIAL
if ($initial) { try { $d.SelectedPath = $initial } catch {} }
if ($d.ShowDialog() -eq [System.Windows.Forms.DialogResult]::OK) {
    # 以 UTF-8 裸字节写出选中路径，规避 PowerShell 控制台默认 GBK(codepage 936)
    # 编码导致的中文路径乱码：Rust 侧用 from_utf8_lossy 按 UTF-8 读取 stdout。
    $bytes = [System.Text.Encoding]::UTF8.GetBytes($d.SelectedPath + [char]10)
    $stdout = [System.Console]::OpenStandardOutput()
    $stdout.Write($bytes, 0, $bytes.Length)
    $stdout.Flush()
}
"#;
    let mut cmd = tokio::process::Command::new("powershell");
    cmd.silent().args([
        "-NoProfile",
        "-NonInteractive",
        "-STA",
        "-ExecutionPolicy",
        "Bypass",
        "-Command",
        script,
    ]);
    cmd.env("MARG_PICK_INITIAL", initial);
    // 目录对话框可能长时间打开（用户人工选择），上限 15 分钟
    let out = tokio::time::timeout(std::time::Duration::from_secs(900), cmd.output())
        .await
        .map_err(|_| RpcError::new("PICK_TIMEOUT", "目录选择超时（15 分钟内未完成选择）"))?
        .map_err(|e| RpcError::new("PICK_SPAWN", format!("无法启动目录选择器: {e}")))?;
    if !out.status.success() {
        return Ok(json!({ "path": null, "kind": "workspace" }));
    }
    let selected = String::from_utf8_lossy(&out.stdout).trim().to_string();
    Ok(json!({
        "path": if selected.is_empty() { Value::Null } else { Value::String(selected) },
        "kind": "workspace",
    }))
}

/// `sandbox.path.list`：Rust 本地目录浏览（项目工作区选择器「平台目录」加载）。
pub async fn sandbox_path_list(
    params: &Value,
) -> Result<Value, crate::gateway::protocol::RpcError> {
    use crate::gateway::protocol::RpcError;
    let raw_path = params
        .get("path")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    let raw_base = params
        .get("basePath")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    let target: PathBuf = if !raw_path.is_empty() && std::path::Path::new(raw_path).is_absolute() {
        PathBuf::from(raw_path)
    } else if !raw_path.is_empty() && !raw_base.is_empty() {
        std::path::Path::new(raw_base).join(raw_path)
    } else if !raw_base.is_empty() {
        PathBuf::from(raw_base)
    } else if !raw_path.is_empty() {
        sandbox_path_root().join(raw_path)
    } else {
        sandbox_path_root()
    };

    if !target.exists() {
        return Err(RpcError::new(
            "PATH_NOT_FOUND",
            format!("目录不存在：{}", target.to_string_lossy()),
        ));
    }
    if !target.is_dir() {
        return Err(RpcError::new(
            "NOT_A_DIRECTORY",
            format!("路径不是目录：{}", target.to_string_lossy()),
        ));
    }

    let mut entries: Vec<Value> = Vec::new();
    match std::fs::read_dir(&target) {
        Ok(rd) => {
            for entry in rd.filter_map(Result::ok) {
                let fname = entry.file_name().to_string_lossy().to_string();
                let fpath = entry.path();
                let is_dir = fpath.is_dir();
                entries.push(json!({
                    "name": fname,
                    "path": fpath.to_string_lossy(),
                    "kind": if is_dir { "directory" } else { "file" },
                    "selectable": is_dir,
                    "hidden": fname.starts_with('.'),
                }));
            }
        }
        Err(e) => {
            return Err(RpcError::new(
                "READ_DIR_FAILED",
                format!("无法读取目录 {}：{e}", target.to_string_lossy()),
            ));
        }
    }
    // 目录优先、再按名称排序
    entries.sort_by(|a, b| {
        let da = a.get("kind").and_then(Value::as_str) == Some("directory");
        let db = b.get("kind").and_then(Value::as_str) == Some("directory");
        match (da, db) {
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            _ => a
                .get("name")
                .and_then(Value::as_str)
                .cmp(&b.get("name").and_then(Value::as_str)),
        }
    });

    let current = target.to_string_lossy().to_string();
    let parent = target.parent().map(|p| p.to_string_lossy().to_string());
    Ok(json!({
        "currentPath": current,
        "path": current,
        "parentPath": parent,
        "systemPickerAvailable": cfg!(target_os = "windows"),
        "entries": entries,
    }))
}

/// `sandbox.path.create-directory`：Rust 本地新建子目录（工作区选择器「新建文件夹」）。
pub async fn sandbox_path_create_directory(
    params: &Value,
) -> Result<Value, crate::gateway::protocol::RpcError> {
    use crate::gateway::protocol::RpcError;
    let name = params
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    if name.is_empty() {
        return Err(RpcError::new("INVALID_REQUEST", "name 不可为空"));
    }
    let raw_parent = params
        .get("parentPath")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    let base: PathBuf = if raw_parent.is_empty() {
        sandbox_path_root()
    } else {
        PathBuf::from(raw_parent)
    };
    let created = base.join(name);
    // 有 `basePath` 时必须是「工作区内建目录」：项目树的「新建文件夹」会带上它，
    // 防止一次越界参数（`parentPath=C:/Windows`）在根外落目录。未提供时保持
    // 旧行为（目录选择器需要能选任意父目录）。
    if let Some(root) = params.get("basePath").and_then(Value::as_str) {
        let root = root.trim();
        if !root.is_empty() {
            let root = PathBuf::from(root);
            if crate::tools::fs_path::safe_resolve(&root, &created.to_string_lossy()).is_err() {
                return Err(RpcError::new(
                    "PATH_ESCAPES_WORKSPACE",
                    format!("目标目录越出工作区，已拒绝：{}", created.to_string_lossy()),
                ));
            }
        }
    }
    std::fs::create_dir_all(&created).map_err(|e| {
        RpcError::new(
            "MKDIR_FAILED",
            format!("创建目录失败 {}：{e}", created.to_string_lossy()),
        )
    })?;
    Ok(json!({ "path": created.to_string_lossy().to_string() }))
}

/// 搜索跳过的目录名（任意层级命中即整棵子树跳过）：依赖 / 构建 / 缓存。
const SEARCH_IGNORED_DIRS: &[&str] = &[
    ".git",
    "node_modules",
    "target",
    "dist",
    "build",
    ".next",
    ".cache",
    "__pycache__",
    ".venv",
    "venv",
];

/// 相对路径最大层级（含文件名；`basePath` 自身为第 0 层）
const SEARCH_MAX_DEPTH: usize = 12;
/// 一次搜索最多扫描的目录条目数（含目录与文件）
const SEARCH_MAX_SCANNED: usize = 20_000;
/// `limit` 缺省值
const SEARCH_DEFAULT_LIMIT: usize = 20;
/// `limit` 上限（下限固定为 1）
const SEARCH_MAX_LIMIT: usize = 100;

/// 单条命中（排序需要保留匹配特征，序列化前再转 JSON）。
struct SearchHit {
    name: String,
    path: String,
    relative: String,
    /// 文件名是否以查询串开头（排序第一优先级）
    name_prefix: bool,
    /// 小写相对路径（排序与稳定比较用）
    relative_lower: String,
}

/// `sandbox.path.search`：有界、安全的文件名搜索（Ctrl+P 快速打开的数据源）。
///
/// 契约：`{ basePath, query, limit? }` →
/// `{ items: [{ name, path, relative, kind: "file" }], truncated }`。
/// - `path` 是**绝对路径**（与 `basePath` 同一写法、原生分隔符）；
/// - `relative` 统一用 `/` 分隔（跨平台 UI 直接按 `/` 切分面包屑）；
/// - 大小写不敏感的**子串**匹配：文件名命中或相对路径命中都算；
/// - 只返回普通文件（目录、链接、管道等一律不返回）。
///
/// 安全与边界（全部是硬上限，宁少给不越界）：
/// - **不跟随符号链接**（含 Windows junction）：`DirEntry::file_type` 不解析链接，
///   链接条目直接跳过 —— 结果必然落在 `basePath` 之内；
/// - 跳过 [`SEARCH_IGNORED_DIRS`]（依赖 / 构建 / 缓存目录），相对路径最多 12 层；
/// - 最多扫描 [`SEARCH_MAX_SCANNED`] 个条目；`limit` 默认 20、夹紧到 `1..=100`；
///   命中数达到 `limit * 4` 即停止遍历（先多收再排序截断）。
///
/// `truncated` 只在**结果可能不全**时为 true：命中数超过 `limit`、或扫描 / 收集
/// 达到上限提前停止。被跳过的目录与超深子树属于既定策略，不计入 truncated。
pub async fn sandbox_path_search(
    params: &Value,
) -> Result<Value, crate::gateway::protocol::RpcError> {
    use crate::gateway::protocol::RpcError;
    let raw_base = params
        .get("basePath")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    if raw_base.is_empty() {
        return Err(RpcError::new(
            "PATH_NOT_FOUND",
            "目录不存在：（basePath 不可为空）",
        ));
    }
    let base = PathBuf::from(raw_base);
    if !base.is_absolute() {
        return Err(RpcError::new(
            "PATH_NOT_FOUND",
            format!("目录不存在：{raw_base}（basePath 必须是绝对路径）"),
        ));
    }
    if !base.exists() {
        return Err(RpcError::new(
            "PATH_NOT_FOUND",
            format!("目录不存在：{}", base.to_string_lossy()),
        ));
    }
    if !base.is_dir() {
        return Err(RpcError::new(
            "NOT_A_DIRECTORY",
            format!("路径不是目录：{}", base.to_string_lossy()),
        ));
    }
    // 根目录不可读（权限）必须如实报错：静默回空列表会被误读成「没有匹配」。
    if let Err(e) = std::fs::read_dir(&base) {
        return Err(RpcError::new(
            "READ_DIR_FAILED",
            format!("无法读取目录 {}：{e}", base.to_string_lossy()),
        ));
    }

    let query = params
        .get("query")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_lowercase();
    if query.is_empty() {
        return Ok(json!({ "items": [], "truncated": false }));
    }
    let limit = params
        .get("limit")
        .and_then(Value::as_u64)
        .unwrap_or(SEARCH_DEFAULT_LIMIT as u64)
        .clamp(1, SEARCH_MAX_LIMIT as u64) as usize;

    // 有界但仍可能扫到上万条目（网络盘/冷缓存可达秒级）：放到阻塞线程池，
    // 不阻塞网关事件循环（chat 流式与 WS 心跳都跑在同一条 loop 上）。
    let (items, truncated) = tokio::task::spawn_blocking(move || search_walk(&base, &query, limit))
        .await
        .map_err(|e| RpcError::internal(&format!("搜索任务失败：{e}")))?;
    Ok(json!({ "items": items, "truncated": truncated }))
}

/// 同步目录遍历（只在 `spawn_blocking` 线程里跑），返回 `(items, truncated)`。
fn search_walk(base: &Path, query_lower: &str, limit: usize) -> (Vec<Value>, bool) {
    let collect_cap = limit.saturating_mul(4);
    let mut hits: Vec<SearchHit> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    // 显式栈：`(目录, 该目录相对 base 的层级)`；base 自身为第 0 层。
    let mut stack: Vec<(PathBuf, usize)> = vec![(base.to_path_buf(), 0)];
    let mut scanned = 0usize;
    let mut truncated = false;

    'walk: while let Some((dir, depth)) = stack.pop() {
        let rd = match std::fs::read_dir(&dir) {
            Ok(rd) => rd,
            // 单个目录不可读（权限 / 竞态删除）不应让整次搜索失败
            Err(_) => continue,
        };
        for entry in rd.filter_map(Result::ok) {
            if scanned >= SEARCH_MAX_SCANNED {
                truncated = true;
                break 'walk;
            }
            scanned += 1;
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            // 不跟随链接：Windows junction 的 file_type 也报 symlink，一并跳过
            if file_type.is_symlink() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().to_string();
            if file_type.is_dir() {
                // 子目录再进一层后文件名落在 depth + 2，故要求 depth + 1 < MAX_DEPTH
                if depth + 1 < SEARCH_MAX_DEPTH && !SEARCH_IGNORED_DIRS.contains(&name.as_str()) {
                    stack.push((entry.path(), depth + 1));
                }
                continue;
            }
            if !file_type.is_file() {
                continue;
            }
            let path = entry.path();
            let Ok(rel) = path.strip_prefix(base) else {
                continue;
            };
            let relative = rel
                .components()
                .map(|c| c.as_os_str().to_string_lossy().to_string())
                .collect::<Vec<_>>()
                .join("/");
            let name_lower = name.to_lowercase();
            let relative_lower = relative.to_lowercase();
            if !name_lower.contains(query_lower) && !relative_lower.contains(query_lower) {
                continue;
            }
            let path = path.to_string_lossy().to_string();
            if !seen.insert(path.clone()) {
                continue;
            }
            hits.push(SearchHit {
                name,
                path,
                relative,
                name_prefix: name_lower.starts_with(query_lower),
                relative_lower,
            });
            if hits.len() >= collect_cap {
                truncated = true;
                break 'walk;
            }
        }
    }

    hits.sort_by(|a, b| {
        // 文件名前缀命中优先；再按相对路径短的优先；最后字典序（大小写不敏感）
        b.name_prefix
            .cmp(&a.name_prefix)
            .then_with(|| a.relative.chars().count().cmp(&b.relative.chars().count()))
            .then_with(|| a.relative_lower.cmp(&b.relative_lower))
            .then_with(|| a.path.cmp(&b.path))
    });
    if hits.len() > limit {
        hits.truncate(limit);
        truncated = true;
    }

    let items = hits
        .into_iter()
        .map(|h| {
            json!({
                "name": h.name,
                "path": h.path,
                "relative": h.relative,
                "kind": "file",
            })
        })
        .collect();
    (items, truncated)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// 每个用例独占的临时目录（`Drop` 清理：断言失败也不会留垃圾）。
    struct TempTree(PathBuf);

    impl TempTree {
        fn new(tag: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "marguerite-sandbox-search-{tag}-{}-{}",
                std::process::id(),
                uuid::Uuid::new_v4()
            ));
            std::fs::create_dir_all(&dir).expect("创建临时目录");
            Self(dir)
        }

        fn path(&self) -> &Path {
            &self.0
        }

        fn base_param(&self) -> String {
            self.0.to_string_lossy().to_string()
        }

        fn write(&self, rel: &str) -> PathBuf {
            let p = self.0.join(rel);
            if let Some(parent) = p.parent() {
                std::fs::create_dir_all(parent).expect("创建父目录");
            }
            std::fs::write(&p, b"x").expect("写入文件");
            p
        }

        fn mkdir(&self, rel: &str) {
            std::fs::create_dir_all(self.0.join(rel)).expect("创建目录");
        }
    }

    impl Drop for TempTree {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    async fn search(tree: &TempTree, query: &str, limit: Option<u64>) -> Value {
        let mut params = json!({
            "basePath": tree.base_param(),
            "query": query,
        });
        if let Some(limit) = limit {
            params["limit"] = json!(limit);
        }
        sandbox_path_search(&params).await.expect("搜索应当成功")
    }

    fn items_of(res: &Value) -> Vec<Value> {
        res["items"].as_array().cloned().unwrap_or_default()
    }

    #[tokio::test]
    async fn search_matches_substring_case_insensitively_with_absolute_and_relative_paths() {
        let tree = TempTree::new("match");
        tree.write("src/main.rs");
        tree.write("src/lib.rs");
        tree.write("docs/README.md");
        // 相对路径命中（文件名不含查询串）
        tree.write("docs/readme-notes.md");

        let res = search(&tree, "MAIN", None).await;
        let items = items_of(&res);
        assert_eq!(items.len(), 1, "{res}");
        assert_eq!(items[0]["name"], "main.rs");
        assert_eq!(items[0]["kind"], "file");
        let abs = items[0]["path"].as_str().expect("path 必须是字符串");
        assert!(Path::new(abs).is_absolute(), "path 必须是绝对路径: {abs}");
        assert!(Path::new(abs).is_file(), "path 必须指向真实文件: {abs}");
        assert_eq!(items[0]["relative"], "src/main.rs");
        assert_eq!(res["truncated"], json!(false), "{res}");

        // 查询串只出现在相对路径里（文件名是 README.md，路径前缀 docs/ 不在文件名中）
        let res = search(&tree, "DOCS/README.MD", None).await;
        let items = items_of(&res);
        assert_eq!(items.len(), 1, "{res}");
        assert_eq!(items[0]["relative"], "docs/README.md");
    }

    #[tokio::test]
    async fn search_skips_ignored_dirs_and_never_returns_directories() {
        let tree = TempTree::new("ignore");
        for dir in SEARCH_IGNORED_DIRS {
            tree.write(&format!("{dir}/needle-{dir}.txt"));
        }
        tree.write("kept/needle-visible.txt");
        tree.mkdir("kept/needle-subdir");

        let res = search(&tree, "needle", None).await;
        let items = items_of(&res);
        assert_eq!(items.len(), 1, "只应命中 kept 下的文件: {res}");
        assert_eq!(items[0]["relative"], "kept/needle-visible.txt");
        for item in &items {
            let rel = item["relative"].as_str().unwrap_or_default();
            for dir in SEARCH_IGNORED_DIRS {
                assert!(
                    !rel.split('/').any(|seg| seg == *dir),
                    "命中不应来自被忽略目录 {dir}: {rel}"
                );
            }
        }
    }

    #[tokio::test]
    async fn search_empty_query_returns_empty_items() {
        let tree = TempTree::new("empty");
        tree.write("src/main.rs");
        for query in ["", "   ", "\t\n"] {
            let res = search(&tree, query, None).await;
            assert_eq!(res["items"], json!([]), "query={query:?} -> {res}");
            assert_eq!(res["truncated"], json!(false), "query={query:?} -> {res}");
        }
    }

    #[tokio::test]
    async fn search_invalid_base_path_uses_list_error_codes() {
        let tree = TempTree::new("errors");

        let missing = tree.path().join("does-not-exist");
        let err = sandbox_path_search(&json!({
            "basePath": missing.to_string_lossy().to_string(),
            "query": "x",
        }))
        .await
        .expect_err("不存在的目录必须报错");
        assert_eq!(err.code, "PATH_NOT_FOUND", "{err:?}");

        let file = tree.write("plain.txt");
        let err = sandbox_path_search(&json!({
            "basePath": file.to_string_lossy().to_string(),
            "query": "x",
        }))
        .await
        .expect_err("文件路径必须报错");
        assert_eq!(err.code, "NOT_A_DIRECTORY", "{err:?}");

        let err = sandbox_path_search(&json!({ "basePath": "   ", "query": "x" }))
            .await
            .expect_err("空 basePath 必须报错");
        assert_eq!(err.code, "PATH_NOT_FOUND", "{err:?}");

        let err = sandbox_path_search(&json!({ "basePath": "relative/dir", "query": "x" }))
            .await
            .expect_err("非绝对 basePath 必须报错");
        assert_eq!(err.code, "PATH_NOT_FOUND", "{err:?}");
    }

    #[tokio::test]
    async fn search_clamps_limit_and_reports_truncation_honestly() {
        let tree = TempTree::new("limit");
        for i in 0..30 {
            tree.write(&format!("hit-{i:02}.txt"));
        }

        let res = search(&tree, "hit", Some(2)).await;
        assert_eq!(items_of(&res).len(), 2, "{res}");
        assert_eq!(res["truncated"], json!(true), "命中多于 limit 必须标记截断");

        // limit=0 夹紧到 1（不是返回空）
        let res = search(&tree, "hit", Some(0)).await;
        assert_eq!(items_of(&res).len(), 1, "{res}");
        assert_eq!(res["truncated"], json!(true), "{res}");

        // limit 上限 100：30 条全给且不截断
        let res = search(&tree, "hit", Some(1000)).await;
        assert_eq!(items_of(&res).len(), 30, "{res}");
        assert_eq!(res["truncated"], json!(false), "{res}");
    }

    #[tokio::test]
    async fn search_sorts_name_prefix_then_shorter_relative_then_alphabetical() {
        let tree = TempTree::new("sort");
        tree.write("src/apple.ts");
        tree.write("app.ts");
        tree.write("docs/zapp.md");

        let res = search(&tree, "app", None).await;
        let rels: Vec<String> = items_of(&res)
            .iter()
            .map(|i| i["relative"].as_str().unwrap_or_default().to_string())
            .collect();
        assert_eq!(
            rels,
            vec!["app.ts", "src/apple.ts", "docs/zapp.md"],
            "{res}"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn search_does_not_follow_symlinked_directories() {
        use std::os::unix::fs::symlink;

        let outside = TempTree::new("outside");
        outside.write("secret-needle.txt");
        let tree = TempTree::new("symlink");
        tree.write("inside-needle.txt");
        symlink(outside.path(), tree.path().join("linked")).expect("创建符号链接");

        let res = search(&tree, "needle", None).await;
        let items = items_of(&res);
        assert_eq!(items.len(), 1, "不得穿过符号链接: {res}");
        assert_eq!(items[0]["relative"], "inside-needle.txt");
    }
}
