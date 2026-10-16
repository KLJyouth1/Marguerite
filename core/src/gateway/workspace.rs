//! 工作区文件读写 / 文件变更台账 方法族
//!
//! 提供前端「文件更改卡片」数据源和内置多文件编辑器的读写能力。

use std::path::PathBuf;

use serde_json::{json, Value};

use crate::gateway::_helpers::param_str;
use crate::gateway::sandbox_paths;
use crate::mcp::registry::{ToolInfo, ToolRegistry};
use crate::tools::builtin::register_builtin_tools;

/// `workspace.file.read`：读取工作区文本文件。
pub fn workspace_file_read(params: &Value) -> Value {
    let raw = param_str(params, "path", "path", "").trim();
    if raw.is_empty() {
        return json!({ "ok": false, "error": "缺少 path" });
    }
    let root = sandbox_paths::sandbox_path_root();
    let p = PathBuf::from(raw);
    let abs = if p.is_absolute() { p } else { root.join(&p) };
    let rootc = root.canonicalize().unwrap_or_else(|_| root.clone());
    let absc = abs.canonicalize().unwrap_or_else(|_| abs.clone());
    if !absc.starts_with(&rootc) {
        return json!({ "ok": false, "error": "路径超出工作区" });
    }
    match std::fs::read_to_string(&absc) {
        Ok(content) => json!({
            "ok": true,
            "path": absc.to_string_lossy(),
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
    let root = sandbox_paths::sandbox_path_root();
    let p = PathBuf::from(raw);
    let abs = if p.is_absolute() { p } else { root.join(&p) };
    let rootc = root.canonicalize().unwrap_or_else(|_| root.clone());
    let parent_ok = abs
        .parent()
        .and_then(|d| d.canonicalize().ok())
        .map(|d| d.starts_with(&rootc))
        .unwrap_or(false);
    if !parent_ok && !abs.starts_with(&rootc) {
        return json!({ "ok": false, "error": "路径超出工作区" });
    }
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



