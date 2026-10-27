//! 工作区文件读写 / 文件变更台账 方法族
//!
//! 提供前端「文件更改卡片」数据源和内置多文件编辑器的读写能力。

use std::path::PathBuf;

use serde_json::{json, Value};

use crate::gateway::_helpers::param_str;
use crate::mcp::registry::{ToolInfo, ToolRegistry};
use crate::tools::builtin::register_builtin_tools;

/// `workspace.file.*` 的工作区根：与文件工具面**同一个**权威解析器。
///
/// 调用方可显式传 `projectPath` / `project_path` 与会话绑定对齐；缺省回落
/// [`crate::workspace::Workspace::resolve`] 的默认根（应用数据目录 → 进程 cwd）。
///
/// # 为什么不能用 HOME
///
/// 这里曾经直接用 `sandbox_paths::sandbox_path_root()`（`USERPROFILE`/`HOME`）当根，
/// 于是「内置工作区编辑器」实际能打开 `~/.ssh/id_rsa` 与 `~/.marguerite/gateway-token`
/// —— 后者正是本网关自己的鉴权凭据，等于把「控制台读写文件」放大成「整机凭据读取」。
/// 同仓的 `workspace.rs` 头注释早已把这种「各写一半的根」列为 P0-1b 要收敛的问题。
fn workspace_file_root(params: &Value) -> PathBuf {
    let explicit = param_str(params, "projectPath", "project_path", "")
        .trim()
        .to_string();
    let explicit = (!explicit.is_empty()).then_some(explicit.as_str());
    crate::workspace::resolve_for_session(explicit).root
}

/// 把入参路径解析到工作区内（fail-closed）。
///
/// 统一委托 [`crate::tools::fs_path::safe_resolve`]：它同时处理 `..` 归一化、
/// 绝对路径前缀校验、以及「根内软链指向根外」的存在性感知 canonicalize ——
/// 手写判定容易漏掉其中一项（旧写入实现正是用 `&&` 串了两个半判定）。
fn resolve_workspace_file(root: &std::path::Path, raw: &str) -> Result<PathBuf, String> {
    crate::tools::fs_path::safe_resolve(root, raw).map_err(|e| {
        format!("路径超出工作区（根 {}）：{e}", root.display())
    })
}

/// `workspace.file.read`：读取工作区文本文件。
pub fn workspace_file_read(params: &Value) -> Value {
    let raw = param_str(params, "path", "path", "").trim();
    if raw.is_empty() {
        return json!({ "ok": false, "error": "缺少 path" });
    }
    let root = workspace_file_root(params);
    let abs = match resolve_workspace_file(&root, raw) {
        Ok(p) => p,
        Err(e) => return json!({ "ok": false, "error": e }),
    };
    match std::fs::read_to_string(&abs) {
        Ok(content) => json!({
            "ok": true,
            "path": abs.to_string_lossy(),
            "content": content,
            "bytes": content.len(),
        }),
        Err(e) => json!({ "ok": false, "error": e.to_string() }),
    }
}

/// `workspace.file.write`：写回工作区文本文件。
pub fn workspace_file_write(params: &Value) -> Value {
    let raw = param_str(params, "path", "path", "").trim();
    if raw.is_empty() {
        return json!({ "ok": false, "error": "缺少 path" });
    }
    let content = params.get("content").and_then(Value::as_str).unwrap_or("");
    let root = workspace_file_root(params);
    // 越界即拒绝，且**必须先于**任何 `create_dir_all`：旧实现用
    // `!parent_ok && !abs.starts_with(root)` 串判定，`root/../escape.txt` 这类
    // 「词法前缀命中、真实落点在根外」的路径两个条件恰好一真一假 → 放行 → 越界写盘。
    let abs = match resolve_workspace_file(&root, raw) {
        Ok(p) => p,
        Err(e) => return json!({ "ok": false, "error": e }),
    };
    if let Some(parent) = abs.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            return json!({ "ok": false, "error": format!("创建父目录失败: {e}") });
        }
    }
    match std::fs::write(&abs, content.as_bytes()) {
        Ok(()) => json!({ "ok": true, "path": abs.to_string_lossy(), "bytes": content.len() }),
        Err(e) => json!({ "ok": false, "error": e.to_string() }),
    }
}

/// `workspace.file.rename`：重命名/移动工作区内文件或目录。
///
/// 只接受「同目录改名」而不接受任意新相对路径：树上的重命名语义就是改名，
/// 允许挪目录会让「右键重命名」变成隐蔽的移动操作（用户无法撤销）。
pub fn workspace_file_rename(params: &Value) -> Value {
    let raw = param_str(params, "path", "path", "").trim();
    let new_name = param_str(params, "newName", "new_name", "").trim();
    if raw.is_empty() || new_name.is_empty() {
        return json!({ "ok": false, "error": "缺少 path 或 newName" });
    }
    // 名字只能是单段：带分隔符 = 移动，带 `..` = 越界尝试。
    if new_name.contains('/') || new_name.contains('\\') || new_name.contains("..") {
        return json!({ "ok": false, "error": "newName 只能是文件名，不能包含路径分隔符" });
    }
    let root = workspace_file_root(params);
    let abs = match resolve_workspace_file(&root, raw) {
        Ok(p) => p,
        Err(e) => return json!({ "ok": false, "error": e }),
    };
    if !abs.exists() {
        return json!({ "ok": false, "error": "目标不存在" });
    }
    let Some(parent) = abs.parent() else {
        return json!({ "ok": false, "error": "无法解析父目录" });
    };
    let target = parent.join(new_name);
    if target.exists() {
        return json!({ "ok": false, "error": "同名文件或目录已存在" });
    }
    // 目标路径仍须落在工作区内（父目录已由 safe_resolve 校验，这里再兜一次）
    if let Err(e) = resolve_workspace_file(&root, &target.to_string_lossy()) {
        return json!({ "ok": false, "error": e });
    }
    match std::fs::rename(&abs, &target) {
        Ok(()) => json!({
            "ok": true,
            "path": target.to_string_lossy(),
            "renamedFrom": abs.to_string_lossy(),
        }),
        Err(e) => json!({ "ok": false, "error": e.to_string() }),
    }
}

/// `workspace.file.delete`：删除工作区内文件或目录。
///
/// 目录必须显式 `recursive=true` 才递归删除 —— 一次误点的代价不可逆，
/// 非空目录默认拒绝并把原因说清楚。
pub fn workspace_file_delete(params: &Value) -> Value {
    let raw = param_str(params, "path", "path", "").trim();
    if raw.is_empty() {
        return json!({ "ok": false, "error": "缺少 path" });
    }
    let recursive = params
        .get("recursive")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let root = workspace_file_root(params);
    let abs = match resolve_workspace_file(&root, raw) {
        Ok(p) => p,
        Err(e) => return json!({ "ok": false, "error": e }),
    };
    let meta = match std::fs::symlink_metadata(&abs) {
        Ok(m) => m,
        Err(e) => return json!({ "ok": false, "error": e.to_string() }),
    };
    let outcome = if meta.is_dir() {
        if recursive {
            std::fs::remove_dir_all(&abs)
        } else {
            std::fs::remove_dir(&abs).map_err(|e| {
                if e.kind() == std::io::ErrorKind::DirectoryNotEmpty {
                    std::io::Error::new(
                        std::io::ErrorKind::DirectoryNotEmpty,
                        "目录非空：如需递归删除请传 recursive=true",
                    )
                } else {
                    e
                }
            })
        }
    } else {
        std::fs::remove_file(&abs)
    };
    match outcome {
        Ok(()) => json!({ "ok": true, "path": abs.to_string_lossy(), "recursive": recursive }),
        Err(e) => json!({ "ok": false, "error": e.to_string() }),
    }
}

/// `workspace.file.reveal`：在系统文件管理器中定位文件/目录。
///
/// 不走 shell 拼接（路径里的空格/引号会被解释）：Windows 用
/// `explorer /select,<path>` 参数数组；其它平台打开父目录。
pub fn workspace_file_reveal(params: &Value) -> Value {
    let raw = param_str(params, "path", "path", "").trim();
    if raw.is_empty() {
        return json!({ "ok": false, "error": "缺少 path" });
    }
    let root = workspace_file_root(params);
    let abs = match resolve_workspace_file(&root, raw) {
        Ok(p) => p,
        Err(e) => return json!({ "ok": false, "error": e }),
    };
    if !abs.exists() {
        return json!({ "ok": false, "error": "目标不存在" });
    }
    #[cfg(windows)]
    let spawned = {
        use std::process::Command;
        Command::new("explorer")
            .arg(format!("/select,{}", abs.display()))
            .spawn()
    };
    #[cfg(not(windows))]
    let spawned = {
        use std::process::Command;
        let dir = abs.parent().map(|p| p.to_path_buf()).unwrap_or(abs.clone());
        Command::new("xdg-open").arg(dir).spawn()
    };
    match spawned {
        Ok(_) => json!({ "ok": true, "path": abs.to_string_lossy() }),
        Err(e) => json!({ "ok": false, "error": format!("无法打开文件管理器: {e}") }),
    }
}

/// `workspace.file_changes.list`：返回 Agent 最近对工作区文件的真实改动台账。
pub fn file_changes_list(params: &Value) -> Value {
    let limit: usize = params
        .get("limit")
        .and_then(Value::as_u64)
        .unwrap_or(50)
        .min(200) as usize;
    let items = crate::tools::file_change::list_file_changes(limit);
    json!({
        "items": items.iter().map(|c| json!({
            "path": c.path,
            "kind": c.kind,
            "before": c.before,
            "after": c.after,
            "tool": c.tool,
            "ts": c.ts,
        })).collect::<Vec<_>>(),
    })
}

/// `workspace.file_changes.revert`：回滚一条文件变更。
pub fn file_changes_revert(params: &Value) -> Value {
    let path = match params.get("path").and_then(Value::as_str) {
        Some(p) if !p.trim().is_empty() => p,
        _ => return json!({ "error": "missing 'path'", "code": "invalid_params" }),
    };
    let kind = params
        .get("kind")
        .and_then(Value::as_str)
        .unwrap_or("modify");
    let before = params.get("before").and_then(Value::as_str).unwrap_or("");

    let ws = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let abs = match crate::tools::fs_path::safe_resolve(&ws, path) {
        Ok(p) => p,
        Err(e) => {
            return json!({ "ok": false, "reverted": false, "error": e.to_string(), "code": "path_escape" })
        }
    };

    let outcome = match kind {
        "create" => match std::fs::remove_file(&abs) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e),
        },
        _ => {
            if let Some(parent) = abs.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            std::fs::write(&abs, before)
        }
    };

    match outcome {
        Ok(()) => json!({ "ok": true, "reverted": true, "path": path, "kind": kind }),
        Err(e) => {
            json!({ "ok": false, "reverted": false, "error": e.to_string(), "code": "revert_failed" })
        }
    }
}

/// 默认工作区（目录快照用；`tools.catalog` 在网关进程里跑）。
///
/// 实现收敛到 [`crate::workspace::Workspace::resolve`]（唯一解析器，P0-1b）。
/// 此前这里是第三份「各写一半」的实现（只认 cwd），与编排器的
/// `default_workspace()`（data_dir 优先）不一致——同一进程两条根。
fn default_workspace() -> PathBuf {
    crate::workspace::Workspace::resolve(None).root
}

/// `workspace.status`：工作区根 / 绑定状态（P0-1b 的用户侧可见面）。
///
/// 为什么需要它：未绑定时前端以前只看到一个泛泛的「失败」，用户既不知道文件
/// 将落到哪个目录、也不知道怎么改。这里把权威解析结果如实回传，其中 `guidance`
/// 就是**模型实际看到的那段**系统提示（同一份文案，用户与模型不出现信息差）。
///
/// 参数（均可选）：
/// - `path` / `projectPath`：调用方已知的会话绑定目录；缺省 = 进程默认根。
///
/// 只读、无副作用。
pub fn workspace_status(params: &Value) -> Value {
    let raw = param_str(params, "path", "projectPath", "").trim().to_string();
    let explicit = (!raw.is_empty()).then_some(raw.as_str());
    crate::workspace::resolve_for_session(explicit).status_json()
}

/// 目录快照的**可注入**实现：补丁与工作区都从参数来。
///
/// 可注入是刻意的：补丁路径若靠环境变量在测试里重定向，就会和并行执行的其它用例
/// （`Orchestrator::new` 在构造时也读补丁）互相串扰——本轮已实测到一次这样的误伤。
/// 因此测试直接注入补丁对象，进程环境保持干净。
fn tools_catalog_with(
    patch: &crate::tools::patch::ToolPatch,
    workspace: &std::path::Path,
) -> Value {
    let mut reg = ToolRegistry::new();
    let catalog: Vec<ToolInfo> =
        match register_builtin_tools(&mut reg, workspace, None, None, None) {
            Ok(()) => reg.catalog(),
            Err(_) => Vec::new(),
        };
    json!({
        "items": catalog.iter().map(|t| json!({
            "name": t.name,
            "description": t.description,
            "source": t.source,
            "server": t.server,
            "enabled": !patch.is_disabled(&t.name),
        })).collect::<Vec<_>>(),
        "total": catalog.len(),
    })
}

/// `tools.catalog`：工具目录（从**真实** builtin 注册表生成）。
///
/// **列出全部已知工具**并逐项标注 `enabled`（是否在当前工具面），而不是把被禁用的
/// 工具直接过滤掉——否则前端无从展示它们，用户也就永远无法把工具重新启用。
/// `enabled` 取自工具面 patch 层（`tools/patch.rs`）。
pub fn tools_catalog() -> Value {
    tools_catalog_with(&crate::tools::patch::current(), &default_workspace())
}

/// 补丁中**当前匹配不到任何已知工具**的名字。
///
/// 如实回传是必要的：否则一条打错字的补丁会永远躺在文件里，UI 看起来"已禁用"，
/// 实际什么也没发生。
fn inactive_patch_names_in(
    patch: &crate::tools::patch::ToolPatch,
    workspace: &std::path::Path,
) -> Vec<String> {
    let mut reg = ToolRegistry::new();
    let known: std::collections::HashSet<String> =
        match register_builtin_tools(&mut reg, workspace, None, None, None) {
            Ok(()) => reg.list_tools().into_iter().collect(),
            Err(_) => std::collections::HashSet::new(),
        };
    patch
        .disabled
        .iter()
        .filter(|n| !known.contains(*n))
        .cloned()
        .collect()
}

/// 补丁状态视图（可注入：补丁对象 + 文件路径 + 工作区）。
fn patch_view(
    patch: &crate::tools::patch::ToolPatch,
    err: Option<String>,
    path: &std::path::Path,
    workspace: &std::path::Path,
) -> Value {
    let inactive = inactive_patch_names_in(patch, workspace);
    json!({
        // 文件损坏时为 false：仍然给出最后一次成功读取的补丁，但绝不谎称读取成功。
        "ok": err.is_none(),
        "disabled": patch.disabled_names(),
        "inactive": inactive,
        "updatedAt": patch.updated_at,
        "appliesTo": "next_turn",
        "path": path.display().to_string(),
        "error": err,
    })
}

/// `tools.patch.get`：读取工具面补丁（热启停状态）。
///
/// `appliesTo: "next_turn"` 是契约的一部分：补丁只影响之后新建的编排器，
/// **不**会中途剥夺正在执行的轮次的工具。前端据此如实提示，而不是假装即时生效。
pub fn tools_patch_get() -> Value {
    let (patch, err) = crate::tools::patch::current_with_error();
    patch_view(
        &patch,
        err,
        &crate::tools::patch::patch_path(),
        &default_workspace(),
    )
}

/// `tools.patch.set`：整体覆盖工具面补丁（参数 `disabled: string[]`）。
///
/// 采用「整体覆盖」而非增删单条：补丁文件与 UI 状态一一对应，避免两次点击之间
/// 出现半更新态。名字照实写入（不因目录里暂时没有就丢弃），但把**当前匹配不到**
/// 的名字回传在 `inactive` 里，让「没生效」这件事可见。
pub fn tools_patch_set(params: &Value) -> Value {
    let disabled = match parse_disabled_param(params) {
        Ok(v) => v,
        Err(bad) => return bad,
    };
    match crate::tools::patch::set_disabled(disabled) {
        Ok(patch) => patch_view(
            &patch,
            None,
            &crate::tools::patch::patch_path(),
            &default_workspace(),
        ),
        Err(e) => json!({ "ok": false, "code": "PATCH_WRITE_FAILED", "error": e }),
    }
}

/// 解析 `disabled` 参数：必须是字符串数组（缺省视为空数组）。
fn parse_disabled_param(params: &Value) -> Result<Vec<String>, Value> {
    match params.get("disabled") {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::Array(items)) => Ok(items
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect()),
        Some(other) => Err(json!({
            "ok": false,
            "code": "BAD_PARAM",
            "error": format!("disabled 需为字符串数组，实际收到：{other}"),
        })),
    }
}

/// 路径可注入的 `tools.patch.get`（单测用；生产走 [`tools_patch_get`]）。
///
/// 注意分工：**缓存与 last-good 回退语义归 `tools/patch.rs` 管**（那里的用例覆盖
/// 「坏文件不得放回被禁工具」），本层只负责 JSON 契约形状，故这里直接读文件。
#[cfg(test)]
fn tools_patch_get_at(path: &std::path::Path, workspace: &std::path::Path) -> Value {
    let (patch, err) = match crate::tools::patch::read_patch_at(path) {
        Ok(p) => (p, None),
        Err(e) => (crate::tools::patch::ToolPatch::empty(), Some(e)),
    };
    patch_view(&patch, err, path, workspace)
}

/// 路径可注入的 `tools.patch.set`（单测用；生产走 [`tools_patch_set`]）。
#[cfg(test)]
fn tools_patch_set_at(
    params: &Value,
    path: &std::path::Path,
    workspace: &std::path::Path,
) -> Value {
    let disabled = match parse_disabled_param(params) {
        Ok(v) => v,
        Err(bad) => return bad,
    };
    match crate::tools::patch::write_patch_at(path, &disabled.into_iter().collect()) {
        Ok(patch) => patch_view(&patch, None, path, workspace),
        Err(e) => json!({ "ok": false, "code": "PATCH_WRITE_FAILED", "error": e }),
    }
}

#[cfg(test)]
mod file_access_tests {
    use super::*;

    /// 独立工作区 fixture：根目录由参数注入（`projectPath`），不碰进程环境。
    struct WsFixture {
        base: PathBuf,
        root: PathBuf,
    }

    impl WsFixture {
        fn new(tag: &str) -> Self {
            static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let base = std::env::temp_dir().join(format!(
                "mgt_gw_wsfile_{}_{}_{}",
                std::process::id(),
                tag,
                seq
            ));
            let root = base.join("ws");
            let _ = std::fs::remove_dir_all(&base);
            std::fs::create_dir_all(&root).unwrap();
            Self { base, root }
        }

        fn params(&self, extra: Value) -> Value {
            let mut obj = extra.as_object().cloned().unwrap_or_default();
            obj.insert(
                "projectPath".to_string(),
                Value::String(self.root.to_string_lossy().to_string()),
            );
            Value::Object(obj)
        }
    }

    impl Drop for WsFixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.base);
        }
    }

    /// 工作区内相对路径读写往返正常（功能不许因加固而丢）。
    #[test]
    fn read_write_roundtrip_inside_workspace() {
        let f = WsFixture::new("roundtrip");
        let w = workspace_file_write(&f.params(json!({
            "path": "notes/a.md",
            "content": "hello",
        })));
        assert_eq!(w["ok"], json!(true), "工作区内写入应成功: {w}");
        assert!(f.root.join("notes/a.md").exists());

        let r = workspace_file_read(&f.params(json!({ "path": "notes/a.md" })));
        assert_eq!(r["ok"], json!(true));
        assert_eq!(r["content"], json!("hello"));
    }

    /// 旧实现的越界写漏洞：`<root>/../escape.txt` 词法前缀命中工作区、
    /// 真实落点却在根外，`&&` 串判定会放行。必须彻底堵死。
    #[test]
    fn write_must_not_escape_via_parent_dir() {
        let f = WsFixture::new("escape");
        for evil in ["../escape.txt", "..\\escape.txt", "a/../../escape.txt"] {
            let w = workspace_file_write(&f.params(json!({
                "path": evil,
                "content": "pwned",
            })));
            assert_eq!(w["ok"], json!(false), "{evil} 必须被拒绝: {w}");
            assert!(
                w["error"].as_str().is_some_and(|e| e.contains("路径超出工作区")),
                "拒绝理由必须可诊断: {w}"
            );
        }
        assert!(
            !f.base.join("escape.txt").exists(),
            "绝不能在技能/工作区之外落盘"
        );
    }

    /// 根外的绝对路径（含 `~/.ssh/id_rsa` 这类凭据文件）一律拒绝。
    #[test]
    fn read_write_reject_absolute_paths_outside_workspace() {
        let f = WsFixture::new("absolute");
        let secret = f.base.join("id_rsa");
        std::fs::write(&secret, "PRIVATE KEY").unwrap();

        let r = workspace_file_read(&f.params(json!({
            "path": secret.to_string_lossy(),
        })));
        assert_eq!(r["ok"], json!(false), "根外绝对路径读取必须拒绝: {r}");

        let w = workspace_file_write(&f.params(json!({
            "path": f.base.join("evil.txt").to_string_lossy(),
            "content": "x",
        })));
        assert_eq!(w["ok"], json!(false));
        assert!(!f.base.join("evil.txt").exists());

        // 根内的绝对路径仍应放行（编辑器会回传绝对路径）
        let inside = f.root.join("ok.txt");
        std::fs::write(&inside, "ok").unwrap();
        let r2 = workspace_file_read(&f.params(json!({
            "path": inside.to_string_lossy(),
        })));
        assert_eq!(r2["ok"], json!(true), "根内绝对路径必须可用: {r2}");
    }

    /// 项目树「重命名」：只改同一目录下的名字，且不得覆盖已有文件。
    #[test]
    fn rename_stays_in_place_and_refuses_overwrite() {
        let f = WsFixture::new("rename");
        std::fs::write(f.root.join("a.md"), "hello").unwrap();
        std::fs::write(f.root.join("b.md"), "exists").unwrap();

        let ok = workspace_file_rename(&f.params(json!({ "path": "a.md", "newName": "c.md" })));
        assert_eq!(ok["ok"], json!(true), "同目录改名应成功: {ok}");
        assert!(!f.root.join("a.md").exists());
        assert!(f.root.join("c.md").exists());

        let clash = workspace_file_rename(&f.params(json!({ "path": "c.md", "newName": "b.md" })));
        assert_eq!(clash["ok"], json!(false), "不得静默覆盖: {clash}");

        let escape = workspace_file_rename(&f.params(json!({ "path": "c.md", "newName": "../x.md" })));
        assert_eq!(escape["ok"], json!(false), "带路径的名字必须拒绝: {escape}");
        assert!(!f.base.join("x.md").exists());
    }

    /// 项目树「删除」：非空目录默认拒绝，显式 recursive 才递归删。
    #[test]
    fn delete_refuses_non_empty_dir_unless_recursive() {
        let f = WsFixture::new("delete");
        std::fs::create_dir_all(f.root.join("dir")).unwrap();
        std::fs::write(f.root.join("dir/inner.md"), "x").unwrap();

        let refuse = workspace_file_delete(&f.params(json!({ "path": "dir" })));
        assert_eq!(refuse["ok"], json!(false), "非空目录必须要求 recursive: {refuse}");
        assert!(f.root.join("dir/inner.md").exists());

        let forced = workspace_file_delete(&f.params(json!({ "path": "dir", "recursive": true })));
        assert_eq!(forced["ok"], json!(true), "显式递归应成功: {forced}");
        assert!(!f.root.join("dir").exists());
    }

    /// 默认根**不得**是 HOME：否则 `~/.marguerite/gateway-token` 与 `~/.ssh/*`
    /// 都会被控制台读写。这条直接钉住「根 = 工件区解析器」这个契约。
    #[test]
    fn default_root_is_not_user_home() {
        let root = workspace_file_root(&json!({}));
        for h in [
            std::env::var("USERPROFILE").ok(),
            std::env::var("HOME").ok(),
        ]
        .into_iter()
        .flatten()
        {
            let h = PathBuf::from(h);
            assert_ne!(
                root, h,
                "默认工作区根不得等于 HOME/USERPROFILE（会暴露 ~/.ssh 与网关 token）"
            );
        }
    }
}

#[cfg(test)]
mod patch_rpc_tests {
    use super::*;
    use crate::tools::patch::ToolPatch;
    use std::sync::atomic::{AtomicU64, Ordering};

    /// 本模块的测试**完全不动进程环境**：补丁文件路径与工作区都按参数注入。
    /// 早期版本用环境变量重定向补丁路径，结果与并行执行的 `Orchestrator::new`
    /// （构造时也读补丁）串扰，误伤了一个治理门用例——这是本轮实测到的教训。
    struct Fixture {
        path: PathBuf,
        workspace: PathBuf,
    }

    impl Fixture {
        fn new(tag: &str) -> Self {
            static SEQ: AtomicU64 = AtomicU64::new(0);
            let seq = SEQ.fetch_add(1, Ordering::SeqCst);
            let base = std::env::temp_dir().join(format!(
                "mgt_gw_tool_patch_{}_{}_{}",
                std::process::id(),
                tag,
                seq
            ));
            let workspace = base.join("ws");
            let _ = std::fs::create_dir_all(&workspace);
            Self {
                path: base.join("tool_patch.json"),
                workspace,
            }
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(self.path.parent().unwrap_or(&self.workspace));
        }
    }

    fn patch_of(names: &[&str]) -> ToolPatch {
        let mut p = ToolPatch::empty();
        for n in names {
            p.disabled.insert((*n).to_string());
        }
        p
    }

    #[test]
    fn workspace_status_defaults_to_an_existing_unbound_root() {
        let v = workspace_status(&json!({}));
        assert_eq!(v["ok"], json!(true));
        let root = v["root"].as_str().unwrap_or_default();
        assert!(!root.is_empty(), "必须给出工作区根，用户才知道文件落到哪");
        assert_eq!(v["bound"], json!(false), "未传 path 不得谎称已绑定");
        let g = v["guidance"].as_str().unwrap_or_default();
        assert!(
            g.contains("不要猜测"),
            "未绑定时必须给出可执行动作：{g}"
        );
        assert!(g.contains(root), "指引必须与回传的根一致：{g}");
    }

    #[test]
    fn workspace_status_reflects_an_explicit_existing_dir_as_bound() {
        let f = Fixture::new("status");
        let v = workspace_status(&json!({ "path": f.workspace.to_string_lossy() }));
        assert_eq!(v["ok"], json!(true));
        assert_eq!(v["bound"], json!(true));
        assert_eq!(v["source"], json!("bound"));
        assert_eq!(v["root"], json!(f.workspace.to_string_lossy()));
        let g = v["guidance"].as_str().unwrap_or_default();
        assert!(g.contains("已绑定"), "已绑定状态要如实说明：{g}");
        assert!(!g.contains("不要猜测"), "已绑定时不应要求重新绑定：{g}");
    }

    #[test]
    fn workspace_status_reports_a_missing_dir_without_lying_about_the_root() {
        let ghost = std::env::temp_dir().join("mgt_gw_ws_status_missing_dir");
        let _ = std::fs::remove_dir_all(&ghost);
        let v = workspace_status(&json!({ "projectPath": ghost.to_string_lossy() }));
        assert_eq!(v["bound"], json!(false));
        assert_eq!(v["source"], json!("missing"));
        assert_eq!(
            v["root"], json!(ghost.to_string_lossy()),
            "目录不存在也必须如实回传用户指定的根——工具正是按它跑的"
        );
    }

    #[test]
    fn patch_get_defaults_to_empty_and_healthy() {
        let f = Fixture::new("default");
        let v = tools_patch_get_at(&f.path, &f.workspace);
        assert_eq!(v["ok"], true, "无补丁文件是正常状态，不是错误");
        assert_eq!(v["disabled"].as_array().map(Vec::len), Some(0));
        assert_eq!(v["appliesTo"], "next_turn", "契约须显式声明生效时机");
        assert_eq!(v["inactive"].as_array().map(Vec::len), Some(0));
    }

    /// 端到端：写入补丁后目录的 `enabled` 必须如实反映，
    /// 且**被禁工具仍在目录里**（否则 UI 无法把它重新启用）。
    #[test]
    fn patch_set_reflects_in_catalog_and_keeps_disabled_tool_listed() {
        let f = Fixture::new("catalog");
        let set = tools_patch_set_at(&json!({ "disabled": ["shell_exec"] }), &f.path, &f.workspace);
        assert_eq!(set["ok"], true);
        assert_eq!(set["appliesTo"], "next_turn");
        assert_eq!(set["inactive"], json!([]));

        let cat = tools_catalog_with(&patch_of(&["shell_exec"]), &f.workspace);
        let items = cat["items"].as_array().expect("目录须为数组");
        let shell = items
            .iter()
            .find(|t| t["name"] == "shell_exec")
            .expect("被禁工具仍须出现在目录里，否则无法重新启用");
        assert_eq!(shell["enabled"], false, "被禁工具须标注 enabled=false");
        let disabled_count = items.iter().filter(|t| t["enabled"] == false).count();
        assert_eq!(disabled_count, 1, "只应有一个工具被禁用");
        assert!(items.len() > 10, "真实工具面应有数十个工具");
    }

    #[test]
    fn patch_set_rejects_non_array_param() {
        let f = Fixture::new("badparam");
        let v = tools_patch_set_at(&json!({ "disabled": "shell_exec" }), &f.path, &f.workspace);
        assert_eq!(v["ok"], false);
        assert_eq!(v["code"], "BAD_PARAM");
    }

    /// 补丁里匹配不到任何工具的名字必须**如实回传** `inactive`，
    /// 否则打错字的补丁会永远躺着、UI 看起来"已禁用"其实什么也没发生。
    #[test]
    fn patch_reports_names_that_match_no_known_tool() {
        let f = Fixture::new("inactive");
        let set = tools_patch_set_at(
            &json!({ "disabled": ["shell_exec", "no_such_tool_xyz"] }),
            &f.path,
            &f.workspace,
        );
        assert_eq!(set["ok"], true);
        assert_eq!(set["inactive"], json!(["no_such_tool_xyz"]));

        let got = tools_patch_get_at(&f.path, &f.workspace);
        assert_eq!(got["disabled"], json!(["no_such_tool_xyz", "shell_exec"]));
        assert_eq!(got["inactive"], json!(["no_such_tool_xyz"]));
    }

    /// 补丁文件损坏：`ok:false` + 错误可诊断（不得谎称健康）。
    #[test]
    fn patch_get_reports_corruption_instead_of_pretending_healthy() {
        let f = Fixture::new("corrupt");
        assert_eq!(
            tools_patch_set_at(&json!({ "disabled": ["shell_exec"] }), &f.path, &f.workspace)["ok"],
            true
        );
        std::fs::write(&f.path, "{ broken").unwrap();

        let v = tools_patch_get_at(&f.path, &f.workspace);
        assert_eq!(v["ok"], false, "损坏必须如实报错");
        assert!(v["error"].as_str().is_some_and(|e| e.contains("JSON 非法")));
    }

    /// 回归守卫：目录必须来自**真实注册表**。此前 `tools.catalog` 派发到硬编码列表，
    /// 里面的 `file_read` / `terminal_exec` / `kb_search` 等名字在工具面上并不存在，
    /// 前端「工具与能力」展示的是编造的工具名。
    #[test]
    fn catalog_serves_real_registry_names_not_fabricated_ones() {
        let f = Fixture::new("catalogreal");
        let cat = tools_catalog_with(&ToolPatch::empty(), &f.workspace);
        let items = cat["items"].as_array().expect("目录须为数组");
        let names: Vec<String> = items
            .iter()
            .filter_map(|t| t["name"].as_str().map(str::to_string))
            .collect();

        for real in ["read_file", "write_file", "shell_exec", "web_search", "system_control"] {
            assert!(
                names.iter().any(|n| n == real),
                "真实工具 {real} 必须出现在目录里，实际：{names:?}"
            );
        }
        for fake in ["file_read", "file_write", "terminal_exec", "kb_search", "plans_create"] {
            assert!(
                !names.iter().any(|n| n == fake),
                "编造的工具名 {fake} 不得再出现在目录里"
            );
        }
        // `useToolSources` 靠 `source` 识别 MCP 来源徽标；缺字段会让徽标恒不显示。
        assert!(
            items.iter().all(|t| t["source"].is_string()),
            "每项都须带 source 字段"
        );
        assert!(
            items.iter().all(|t| t["description"].is_string()),
            "每项都须带 description 字段"
        );
    }
}



