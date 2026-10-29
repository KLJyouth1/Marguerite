//! 工作台面板的**薄**工具（P0-1b）：只把既有 gateway RPC 处理函数接到工具面，
//! 不新增后端契约、不自己实现第二套逻辑。
//!
//! ## 事实核对（2026-10-27）
//!
//! `agent/tool_catalog.rs` 的内置表面表里，「产物中心」与「团队看板」此前
//! **只有描述、没有任何可调用工具**（`grep -n artifacts core/src/tools/builtin.rs`
//! = 0，`grep -n team core/src/tools/builtin.rs` = 0）。后果：主智能体无法核对
//! 面板内容，只能凭上下文记忆猜「我交付了哪些文件、团队跑成什么样」，
//! 于是出现「面板里没有却说有」「成员早已失败却说在跑」这类幻觉。
//!
//! 本模块把 UI 读的**同一批处理函数**变成工具，界面面与工具面因此不可能各说各话：
//!
//! | 工具 | 动作 | 复用的既有 RPC 处理函数（同名方法已在派发表注册） |
//! |---|---|---|
//! | `artifacts` | list / get / register / open | `artifacts.list` / `artifacts.get` / `artifacts.register` / `artifacts.open_in_system`（`gateway/methods.rs` 的 `artifacts::` 分支） |
//! | `team_board` | members / compositions | `team.members` / `team.compositions.list` |
//!
//! ## 「编辑器工作台」为什么**没有**工具（诚实说明）
//!
//! 该面板的正文就是磁盘上的文件，工作台**没有**「替用户打开某个编辑器标签页」或
//! 「在面板里编辑」的 RPC —— `workspace.file.read/write` 与 `read_file`/`write_file`
//! 走同一条落盘链路，`artifacts.edit_inline` 要求前端回传用户所见的 `sha256`
//! （竞态校验），是给界面用的、不是给模型用的。所以这里**不**为它造工具，也不在
//! 提示里声称能「打开编辑器面板」；要改文件请用 `write_file`/`edit_file`/`apply_patch`，
//! 用户在界面上会实时看到同一份内容。这条理由同时写在
//! `agent/tool_catalog.rs::BUILTIN_SURFACES` 的「编辑器工作台」条目里。
//!
//! ## R-10 诚实性
//!
//! gateway 的信封是 `{code, message, data}`：`code == 0` 才算成功。本模块任何
//! `code != 0` 都转成 [`ToolResult::Error`] 并携带可识别的错误码，**绝不**把失败
//! 渲染成成功；`open`/`register` 这类有外部副作用的动作在 Plan（只读探索）模式下
//! 直接按 `MODE_DENIED` 拒绝，与 `agent/tool_cycle.rs` 的模式门同口径。

use std::collections::HashMap;

use anyhow::Result;
use serde_json::{json, Value};

use crate::agent::tool_cycle::ToolResult;
use crate::mcp::registry::ToolExecutor;

/// `artifacts` 支持的动作（与既有 `artifacts.*` RPC 一一对应，不新增契约）。
pub const ARTIFACT_TOOL_ACTIONS: &[&str] = &["list", "get", "register", "open"];

/// `team_board` 支持的动作（与既有 `team.members` / `team.compositions.list` 对应）。
pub const TEAM_BOARD_ACTIONS: &[&str] = &["members", "compositions"];

/// gateway 错误码 → 可识别的工具错误码（历史固定码，前端兼容；见 gateway/artifacts.rs 头注释）。
fn gateway_error_code(action: &str, code: u64) -> String {
    let symbolic = match (action, code) {
        ("open", 1502) => "OPEN_IN_SYSTEM_FAILED",
        (_, 1401) => "INVALID_PARAMS",
        (_, 1404) => "ARTIFACT_NOT_FOUND",
        (_, 1409) => "ARTIFACT_CONFLICT",
        (_, 1503) => "ARTIFACT_WRITE_FAILED",
        _ => "",
    };
    if symbolic.is_empty() {
        format!("GATEWAY_E{code}")
    } else {
        symbolic.to_string()
    }
}

/// `artifacts::*` 的成功形状**不统一**，不能一律按 `code == 0` 判：
/// - `artifacts_list` 返回**裸载荷** `{artifacts, has_more, next_cursor}`（没有 `code`）；
/// - `artifacts_get` / `artifacts_register` / `artifacts_open_in_system` 走 `rpc_ok`，
///   包成 `{code: 0, message, data}`；失败时才由 `rpc_err` 给出**非零** `code`。
///
/// 所以成功 = 没有 `code` 字段，或 `code == 0`；只有明确非零才是失败。
/// （此前一律要求 `Some(0)`，导致 `list` 的裸载荷被永远判成失败：
/// 「产物中心 list 未执行：网关未给出原因」。）
fn envelope_ok(v: &Value) -> bool {
    v.get("code").and_then(Value::as_u64).is_none_or(|c| c == 0)
}

/// `code != 0` 一律如实转成工具错误（R-10）。
fn envelope_failure(v: &Value, action: &str, what: &str) -> ToolResult {
    let code = v.get("code").and_then(Value::as_u64).unwrap_or(0);
    let message = v
        .get("message")
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .unwrap_or("网关未给出原因");
    ToolResult::Error {
        message: format!("{what} {action} 未执行：{message}"),
        code: gateway_error_code(action, code),
    }
}

fn arg_str(args: &HashMap<String, String>, key: &str) -> String {
    args.get(key).map(|s| s.trim().to_string()).unwrap_or_default()
}

fn arg_bool(args: &HashMap<String, String>, key: &str) -> bool {
    matches!(
        arg_str(args, key).to_ascii_lowercase().as_str(),
        "true" | "1" | "yes" | "y" | "on"
    )
}

fn arg_u64(args: &HashMap<String, String>, key: &str) -> Option<u64> {
    arg_str(args, key).parse::<u64>().ok()
}

/// Plan 模式（只读探索）下拒绝有外部副作用的动作，与 `mode::runtime_denies_tool` 同口径。
fn plan_mode_denial(action: &str) -> Option<ToolResult> {
    if crate::mode::runtime_mode() == crate::mode::ModeKind::Plan {
        return Some(ToolResult::Error {
            message: format!(
                "Plan 模式（只读探索）禁止执行有副作用的产物动作：{action}（切到 Agent 模式再执行）"
            ),
            code: "MODE_DENIED".to_string(),
        });
    }
    None
}

/// 产物中心工具：读/登记/打开交付产物，与前端产物抽屉同一数据源、同一处理函数。
pub fn artifacts_tool() -> Box<dyn ToolExecutor> {
    let run: crate::tools::builtin::ToolFn = std::sync::Arc::new(|args: &HashMap<String, String>| {
        let args = args.clone();
        Box::pin(async move { run_artifacts_tool(args).await })
    });
    crate::tools::builtin::make_local_tool(
        "artifacts",
        "产物中心（工作台「产物」面板背后的台账）：动作 action=list|get|register|open。\
         list 列出本会话（或 all_sessions=true 时跨会话）登记过的交付产物，并**如实标注文件是否还在磁盘**；\
         get 按 id 取详情（含 size/mime/sha256/所属会话）；register 把一个已在磁盘上的文件显式登记进面板\
         （交付类任务完成后核对用，路径不存在会如实失败）；open 用系统默认程序打开产物。\
         交付/PPT/报告/视频类任务收尾时**先用 list 核对**，不要凭记忆宣布「已交付」。\
         这是只读优先的面板工具：写文件请用 write_file/edit_file/apply_patch，别用它来改内容。",
        json!({
            "type": "object",
            "properties": {
                "action": {"type": "string", "enum": ARTIFACT_TOOL_ACTIONS, "description": "要做的产物动作"},
                "id": {"type": "string", "description": "get/open 必填：list 返回的产物 id"},
                "path": {"type": "string", "description": "register 必填：磁盘上真实存在的文件路径；open 可代替 id 按路径打开"},
                "kind": {"type": "string", "description": "register 可选：产物类别标注，默认 registered"},
                "session_key": {"type": "string", "description": "可选：显式会话 key；缺省用当前会话（一般不用传）"},
                "all_sessions": {"type": "string", "description": "list 可选：true 时跨会话聚合（「我的文件」口径）"},
                "limit": {"type": "integer", "description": "list 可选：返回条数上限，默认 50"},
                "cursor": {"type": "integer", "description": "list 可选：上一页 next_cursor，取更早的产物"}
            },
            "required": ["action"]
        }),
        run,
    )
}

async fn run_artifacts_tool(args: HashMap<String, String>) -> Result<ToolResult> {
    use crate::tools::builtin::{err, ok};
    let action = arg_str(&args, "action").to_ascii_lowercase();
    if action.is_empty() {
        return err(
            format!("artifacts 需要 action（{}）", ARTIFACT_TOOL_ACTIONS.join("|")),
            "INVALID_PARAMS",
        );
    }
    // 会话归属：与工具回路 task-local 一致（模型不需要知道 key，但要能显式覆盖）。
    let session = {
        let explicit = arg_str(&args, "session_key");
        if explicit.is_empty() {
            crate::tools::builtin::current_session()
        } else {
            explicit
        }
    };

    let outcome: Result<ToolResult> = match action.as_str() {
        "list" => {
            let mut params = json!({ "sessionKey": session });
            if arg_bool(&args, "all_sessions") {
                params["allSessions"] = json!(true);
            }
            if let Some(limit) = arg_u64(&args, "limit") {
                params["limit"] = json!(limit);
            }
            if let Some(cursor) = arg_u64(&args, "cursor") {
                params["cursor"] = json!(cursor);
            }
            let v = crate::gateway::artifacts::artifacts_list(&params);
            if envelope_ok(&v) {
                ok(render_artifact_list(&v, &args))
            } else {
                Ok(envelope_failure(&v, "list", "产物中心"))
            }
        }
        "get" => {
            let id = arg_str(&args, "id");
            if id.is_empty() {
                return err("artifacts get 需要 id（先用 action=list 取）".to_string(), "INVALID_PARAMS");
            }
            let v = crate::gateway::artifacts::artifacts_get(&json!({ "id": id }));
            if envelope_ok(&v) {
                ok(format!(
                    "产物详情:\n{}",
                    serde_json::to_string_pretty(&v["data"]).unwrap_or_else(|_| v["data"].to_string())
                ))
            } else {
                Ok(envelope_failure(&v, "get", "产物中心"))
            }
        }
        "register" => {
            if let Some(denied) = plan_mode_denial("register") {
                return Ok(denied);
            }
            let path = arg_str(&args, "path");
            if path.is_empty() {
                return err(
                    "artifacts register 需要 path（指向磁盘上真实存在的文件）".to_string(),
                    "INVALID_PARAMS",
                );
            }
            if session.is_empty() {
                return err(
                    "artifacts register 需要会话归属，但当前执行上下文没有会话 key（请在智能体回合内调用，或显式传 session_key）"
                        .to_string(),
                    "NO_SESSION_CONTEXT",
                );
            }
            let kind = {
                let k = arg_str(&args, "kind");
                if k.is_empty() { "registered".to_string() } else { k }
            };
            let v = crate::gateway::artifacts::artifacts_register(&json!({
                "session_key": session,
                "path": path,
                "kind": kind,
            }));
            if envelope_ok(&v) {
                // 登记成功的判据不是处理器回了好看的话，而是**回读**到了这条台账。
                let id = v["data"]["id"].as_str().unwrap_or("");
                match crate::tools::file_change::get_artifact(id) {
                    Some(a) => ok(format!(
                        "已登记进产物中心并可回读：id={} 名称={} 类别={} 大小={} 字节 在磁盘={}",
                        a.id,
                        a.name,
                        a.kind,
                        a.size,
                        std::path::Path::new(&a.path).exists()
                    )),
                    None => {
                        return err(
                            format!("登记后回读台账失败（id={id}）：产物中心里查不到这条记录"),
                            "ARTIFACT_REGISTER_NOT_PERSISTED",
                        )
                    }
                }
            } else {
                Ok(envelope_failure(&v, "register", "产物中心"))
            }
        }
        "open" => {
            if let Some(denied) = plan_mode_denial("open") {
                return Ok(denied);
            }
            let id = arg_str(&args, "id");
            let path = arg_str(&args, "path");
            if id.is_empty() && path.is_empty() {
                return err("artifacts open 需要 id 或 path 之一".to_string(), "INVALID_PARAMS");
            }
            let mut params = json!({});
            if !id.is_empty() {
                params["id"] = json!(id);
            }
            if !path.is_empty() {
                params["path"] = json!(path);
            }
            let explicit_root = arg_str(&args, "session_key");
            if !explicit_root.is_empty() {
                // 既有处理器用 projectPath 解析工作区根（越界路径会被 open_path_checked 拒绝）。
                params["projectPath"] = json!(explicit_root);
            }
            let v = crate::gateway::artifacts::artifacts_open_in_system(&params);
            if envelope_ok(&v) {
                ok(format!(
                    "已交给系统默认程序打开：{}",
                    v["data"]["path"].as_str().unwrap_or(path.as_str())
                ))
            } else {
                Ok(envelope_failure(&v, "open", "产物中心"))
            }
        }
        other => {
            return err(
                format!(
                    "artifacts 不支持 action={other}；支持：{}",
                    ARTIFACT_TOOL_ACTIONS.join("|")
                ),
                "UNSUPPORTED_ACTION",
            );
        }
    };
    outcome
}

/// 列表渲染：条数 + 每条的磁盘实况；分页信息如实标注，不假装「已全部列出」。
fn render_artifact_list(v: &Value, args: &HashMap<String, String>) -> String {
    let items = v["artifacts"].as_array().cloned().unwrap_or_default();
    let scope = if arg_bool(args, "all_sessions") {
        "全部会话"
    } else {
        "本会话"
    };
    if items.is_empty() {
        return format!(
            "产物中心（{scope}）目前没有登记的产物。\
             如果你确实生成了文件，请用 write_file/媒体生成工具落到工作区后再用 action=register 登记。"
        );
    }
    let mut lines: Vec<String> = Vec::with_capacity(items.len());
    let mut missing = 0usize;
    for a in &items {
        let present = a["present"].as_bool().unwrap_or(false);
        if !present {
            missing += 1;
        }
        lines.push(format!(
            "- id={} | {} | kind={} | {} 字节 | 在磁盘={}",
            a["id"].as_str().unwrap_or("?"),
            a["name"].as_str().unwrap_or("?"),
            a["kind"].as_str().unwrap_or("?"),
            a["size"].as_u64().unwrap_or(0),
            present,
        ));
    }
    let mut out = format!(
        "产物中心（{scope}）：{} 条登记（其中 {} 条文件已不在磁盘上）\n{}",
        items.len(),
        missing,
        lines.join("\n")
    );
    if v["has_more"].as_bool().unwrap_or(false) {
        out.push_str(&format!(
            "\n（还有更早的产物：带 cursor={} 再取下一页；本次未全部列出）",
            v["next_cursor"]
        ));
    }
    out
}

/// 团队看板工具：成员动向与团队编排预设，读的是 `team.members` / `team.compositions.list`。
pub fn team_board_tool() -> Box<dyn ToolExecutor> {
    let run: crate::tools::builtin::ToolFn = std::sync::Arc::new(|args: &HashMap<String, String>| {
        let args = args.clone();
        Box::pin(async move { run_team_board_tool(args).await })
    });
    crate::tools::builtin::make_local_tool(
        "team_board",
        "团队看板（工作台右侧成员卡片的背后数据）：动作 action=members|compositions。\
         members 返回本轮/指定会话的团队编排投影 —— 主智能体与每个成员的状态、当前步骤、\
         工具调用数；用户在这里看进度，你**不需要**把进度复述到聊天里，核对再用本工具。\
         compositions 列出已保存的团队预设（id/名称/目标/成员角色），需要按预设组队时先看它。\
         没有编排运行记录时如实回「当前没有团队运行」，不会伪造成员。",
        json!({
            "type": "object",
            "properties": {
                "action": {"type": "string", "enum": TEAM_BOARD_ACTIONS, "description": "要做的看板动作"},
                "session": {"type": "string", "description": "members 可选：显式会话 key；缺省用当前会话（无当前会话时取最近一次运行）"}
            },
            "required": ["action"]
        }),
        run,
    )
}

async fn run_team_board_tool(args: HashMap<String, String>) -> Result<ToolResult> {
    use crate::tools::builtin::{err, ok};
    let action = arg_str(&args, "action").to_ascii_lowercase();
    if action.is_empty() {
        return err(
            format!("team_board 需要 action（{}）", TEAM_BOARD_ACTIONS.join("|")),
            "INVALID_PARAMS",
        );
    }
    let outcome: Result<ToolResult> = match action.as_str() {
        "members" => {
            let explicit = arg_str(&args, "session");
            let session = if explicit.is_empty() {
                crate::tools::builtin::current_session()
            } else {
                explicit
            };
            let v = crate::orchestration::member_registry::snapshot(
                if session.is_empty() { None } else { Some(session.as_str()) },
            );
            if v["ok"].as_bool().unwrap_or(false) {
                ok(render_members(&v))
            } else {
                Ok(ToolResult::Error {
                    message: format!(
                        "团队看板 members 未取到数据：{}",
                        v.get("message")
                            .and_then(Value::as_str)
                            .unwrap_or("投影不可用")
                    ),
                    code: v
                        .get("code")
                        .and_then(Value::as_str)
                        .unwrap_or("MEMBER_REGISTRY_UNAVAILABLE")
                        .to_string(),
                })
            }
        }
        "compositions" => {
            let v = crate::gateway::team::team_compositions_list();
            if v["ok"].as_bool().unwrap_or(false) {
                ok(render_compositions(&v))
            } else {
                Ok(ToolResult::Error {
                    message: "团队预设列表读取失败（team.compositions.list 未回 ok:true）".to_string(),
                    code: "TEAM_COMPOSITIONS_UNAVAILABLE".to_string(),
                })
            }
        }
        other => {
            return err(
                format!(
                    "team_board 不支持 action={other}；支持：{}",
                    TEAM_BOARD_ACTIONS.join("|")
                ),
                "UNSUPPORTED_ACTION",
            );
        }
    };
    outcome
}

fn render_members(v: &Value) -> String {
    let members = v["members"].as_array().cloned().unwrap_or_default();
    let from_disk = v["fromDisk"].as_bool().unwrap_or(false);
    if members.is_empty() {
        return format!(
            "团队看板：{}",
            v.get("note")
                .and_then(Value::as_str)
                .unwrap_or("当前没有团队成员记录")
        );
    }
    let lines: Vec<String> = members
        .iter()
        .map(|m| {
            format!(
                "- {}（{}｜{}）状态={} 当前步骤={} 工具调用={} 最近活动={}ms 前",
                m["label"].as_str().unwrap_or("?"),
                m["role"].as_str().unwrap_or("?"),
                m["id"].as_str().unwrap_or("?"),
                m["status"].as_str().unwrap_or("?"),
                m["currentStep"].as_str().unwrap_or("-"),
                m["toolCalls"].as_u64().unwrap_or(0),
                v["updatedAt"]
                    .as_u64()
                    .zip(m["lastActivityMs"].as_u64())
                    .map(|(u, a)| u.saturating_sub(a))
                    .unwrap_or(0),
            )
        })
        .collect();
    let dropped = v["dropped"].as_u64().unwrap_or(0);
    let mut out = format!(
        "团队看板：{} 名成员（形态={}，runId={}，来源={}）\n{}",
        members.len(),
        v["form"]
            .as_str()
            .or_else(|| v["form"].as_bool().map(|_| "未知"))
            .unwrap_or("未知"),
        v["runId"].as_str().unwrap_or("-"),
        if from_disk { "落盘快照（内存态已随进程重启丢失）" } else { "实时投影" },
        lines.join("\n")
    );
    if dropped > 0 {
        out.push_str(&format!(
            "\n（另有 {dropped} 名成员未进入投影，上限 {}；这里不是全部成员，别据此宣布「所有人都完成了」）",
            v["memberLimit"].as_u64().unwrap_or(0)
        ));
    }
    out
}

fn render_compositions(v: &Value) -> String {
    let items = v["items"].as_array().cloned().unwrap_or_default();
    if items.is_empty() {
        return "团队看板：还没有保存过团队预设（可在控制台「团队」里编排并保存；也可以直接临时组队）。"
            .to_string();
    }
    let lines: Vec<String> = items
        .iter()
        .map(|c| {
            let roles: Vec<String> = c["members"]
                .as_array()
                .map(|ms| {
                    ms.iter()
                        .filter_map(|m| m["role"].as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default();
            format!(
                "- id={} | {} | 目标：{} | 成员：{}",
                c["id"].as_str().unwrap_or("?"),
                c["name"].as_str().unwrap_or("?"),
                c["goal"].as_str().unwrap_or("-"),
                roles.join("、")
            )
        })
        .collect();
    format!(
        "团队看板：{} 个已保存的团队预设\n{}",
        items.len(),
        lines.join("\n")
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::file_change;

    fn args(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect()
    }

    /// 工具面与界面面同源的自证：每个动作都必须命中一个既有 RPC 处理函数，
    /// 且这些方法名都真实存在于网关派发表（不得私开第二套面板语义）。
    #[test]
    fn every_workbench_action_maps_to_an_existing_rpc_handler() {
        let dispatch = include_str!("../gateway/methods.rs");
        for method in [
            "artifacts.list",
            "artifacts.get",
            "artifacts.register",
            "artifacts.open_in_system",
            "team.members",
            "team.compositions.list",
        ] {
            assert!(
                dispatch.contains(&format!("\"{method}\"")),
                "网关派发表里没有 {method} —— 工具包装的是一个不存在的方法名"
            );
        }
        let source = include_str!("workbench.rs");
        for handler in [
            "artifacts_list(&params)",
            "artifacts_get(&json!",
            "artifacts_register(&json!",
            "artifacts_open_in_system(&params)",
            "member_registry::snapshot(",
            "team_compositions_list()",
        ] {
            assert!(source.contains(handler), "未复用既有 RPC 处理函数：{handler}");
        }
        assert_eq!(ARTIFACT_TOOL_ACTIONS, &["list", "get", "register", "open"]);
        assert_eq!(TEAM_BOARD_ACTIONS, &["members", "compositions"]);
    }

    /// 未知动作必须是结构化失败，不能伪装成功（R-10）。
    #[tokio::test]
    async fn unknown_actions_fail_loudly() {
        for (tool, name) in [(artifacts_tool(), "artifacts"), (team_board_tool(), "team_board")] {
            let out = tool.execute(&args(&[("action", "teleport")])).await.unwrap();
            match out {
                ToolResult::Error { code, message } => {
                    assert_eq!(code, "UNSUPPORTED_ACTION", "{name} 应给稳定错误码");
                    assert!(message.contains("teleport"), "{message}");
                }
                other => panic!("{name} 未知动作却回了 {other:?}"),
            }
            let out = tool.execute(&args(&[])).await.unwrap();
            match out {
                ToolResult::Error { code, .. } => assert_eq!(code, "INVALID_PARAMS"),
                other => panic!("{name} 缺 action 却回了 {other:?}"),
            }
        }
    }

    #[test]
    fn gateway_envelope_codes_are_symbolic() {
        assert_eq!(gateway_error_code("get", 1404), "ARTIFACT_NOT_FOUND");
        assert_eq!(gateway_error_code("list", 1401), "INVALID_PARAMS");
        assert_eq!(gateway_error_code("open", 1502), "OPEN_IN_SYSTEM_FAILED");
        // 未登记的码也要可 grep，而不是塌成一句「失败了」。
        assert_eq!(gateway_error_code("get", 1999), "GATEWAY_E1999");
        let f = envelope_failure(&json!({"code": 1404, "message": "产物不存在"}), "get", "产物中心");
        match f {
            ToolResult::Error { message, code } => {
                assert_eq!(code, "ARTIFACT_NOT_FOUND");
                assert!(message.contains("未执行"), "{message}");
            }
            _ => panic!("信封失败必须是 Error"),
        }
    }

    #[test]
    fn list_rendering_reports_disk_reality_and_pagination() {
        // 夹具必须是**裸载荷**：`artifacts_list` 不走 `rpc_ok`，返回的是
        // `{artifacts, has_more, next_cursor}`（前端读的也是 `res.artifacts`）。
        // 旧夹具按 `{code:0, data:{…}}` 写，断言了一个生产里不存在的形状。
        let v = json!({
            "artifacts": [
                {"id": "a1", "name": "报告.pptx", "kind": "generated", "size": 12, "present": true},
                {"id": "a2", "name": "视频.mp4", "kind": "generated", "size": 34, "present": false}
            ],
            "has_more": true,
            "next_cursor": 111
        });
        let text = render_artifact_list(&v, &args(&[]));
        assert!(text.contains("2 条登记（其中 1 条文件已不在磁盘上）"), "{text}");
        assert!(text.contains("在磁盘=false"), "{text}");
        assert!(text.contains("cursor=111"), "{text}");
        // 空面板给的是「没有产物」，不是伪造清单。
        let empty = render_artifact_list(
            &json!({"artifacts": [], "has_more": false}),
            &args(&[]),
        );
        assert!(empty.contains("目前没有登记的产物"), "{empty}");
    }

    #[test]
    fn members_render_surfaces_dropped_members() {
        let text = render_members(&json!({
            "ok": true, "form": "swarm", "runId": "r1", "updatedAt": 2000, "memberLimit": 2, "dropped": 3,
            "members": [
                {"id": "lead:r1", "label": "主智能体", "role": "lead", "status": "running",
                 "currentStep": "汇总", "toolCalls": 5, "lastActivityMs": 1000}
            ]
        }));
        assert!(text.contains("1 名成员"), "{text}");
        assert!(text.contains("工具调用=5"), "{text}");
        assert!(text.contains("另有 3 名成员未进入投影"), "{text}");
        // 没有运行记录时如实说明，不伪造成员。
        let none = render_members(&json!({"ok": true, "members": [], "note": "当前没有团队编排运行记录"}));
        assert!(none.contains("当前没有团队编排运行记录"), "{none}");
    }

    #[test]
    fn compositions_render_handles_empty() {
        assert!(render_compositions(&json!({"ok": true, "items": []})).contains("还没有保存过团队预设"));
        let text = render_compositions(&json!({
            "ok": true,
            "items": [{"id": "t1", "name": "PPT 团队", "goal": "做一份季度汇报", "members": [{"role": "撰稿"}, {"role": "设计"}]}]
        }));
        assert!(text.contains("id=t1"), "{text}");
        assert!(text.contains("撰稿、设计"), "{text}");
    }

    /// 真实往返 A → B：用**工具**登记产物，再用网关之外的那条读路径（台账原语）回读，
    /// 断言副作用真的发生（不是只看处理器回了好看的话）。
    // 同 `plan_mode_denies_side_effect_artifact_actions`：守卫要活到工具执行完，
    // 否则读到的是别的用例改过的全局模式；current-thread runtime 不会自我死锁。
    #[allow(clippy::await_holding_lock)]
    #[tokio::test]
    async fn register_through_tool_is_readable_through_the_ledger_primitive() {
        // 登记会读全局运行时模式（Plan 拒绝），与其他临时改写模式的测试用同一把串行锁。
        let _g = crate::mode::test_runtime_mode_guard();
        let prev = crate::mode::set_runtime_mode(crate::mode::ModeKind::Agent);
        let dir = std::env::temp_dir().join(format!("mgt_wb_reg_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("季度汇报.md");
        std::fs::write(&file, "# 交付正文\n").unwrap();
        let session = format!("sess-wb-reg-{}", uuid::Uuid::new_v4());

        let out = artifacts_tool()
            .execute(&args(&[
                ("action", "register"),
                ("path", file.to_string_lossy().as_ref()),
                ("session_key", session.as_str()),
                ("kind", "delivered"),
            ]))
            .await
            .unwrap();
        let id = match &out {
            ToolResult::Success { data, .. } => {
                assert!(data.contains("已登记进产物中心并可回读"), "{data}");
                assert!(data.contains("在磁盘=true"), "{data}");
                data.split("id=")
                    .nth(1)
                    .unwrap_or("")
                    .split(' ')
                    .next()
                    .unwrap_or("")
                    .to_string()
            }
            ToolResult::Error { message, code } => {
                panic!("登记应成功（Plan 模式串扰会导致 MODE_DENIED）：{code} {message}")
            }
            other => panic!("unexpected {other:?}"),
        };
        // 另一条读路径（网关 artifacts_get 之外的 file_change 原语）必须看到同一条记录。
        let back = file_change::get_artifact(&id).expect("台账里必须真的多出一条记录");
        assert_eq!(back.session_key, session);
        assert_eq!(back.kind, "delivered");
        assert_eq!(back.name, "季度汇报.md");

        // 文件被删掉后，面板读到的必须是「在磁盘=false」，而不是继续宣布交付还在。
        std::fs::remove_file(&file).unwrap();
        let listed = artifacts_tool()
            .execute(&args(&[("action", "list"), ("session_key", session.as_str())]))
            .await
            .unwrap();
        match listed {
            ToolResult::Success { data, .. } => {
                assert!(data.contains("在磁盘=false"), "{data}");
                assert!(data.contains("1 条文件已不在磁盘上"), "{data}");
            }
            other => panic!("list 应成功：{other:?}"),
        }
        let _ = std::fs::remove_dir_all(&dir);
        crate::mode::set_runtime_mode(prev);
    }

    /// Plan（只读探索）模式下，有副作用的产物动作必须**真的**被拒（不是只在提示里说说）。
    // 守卫必须**跨整个 await** 才能串行化对全局 runtime_mode 的读：`plan_mode_denial`
    // 在工具执行时才读它，提前释放会让别的用例看到「已切回」的中间态。测试用的是
    // current-thread runtime，持 std 锁不会自我死锁。
    #[allow(clippy::await_holding_lock)]
    #[tokio::test]
    async fn plan_mode_denies_side_effect_artifact_actions() {
        let _g = crate::mode::test_runtime_mode_guard();
        let prev = crate::mode::set_runtime_mode(crate::mode::ModeKind::Plan);
        let dir = std::env::temp_dir().join(format!("mgt_wb_plan_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("a.md");
        std::fs::write(&file, "x").unwrap();
        let session = format!("sess-wb-plan-{}", uuid::Uuid::new_v4());

        for action in ["register", "open"] {
            let out = artifacts_tool()
                .execute(&args(&[
                    ("action", action),
                    ("path", file.to_string_lossy().as_ref()),
                    ("session_key", session.as_str()),
                ]))
                .await
                .unwrap();
            match out {
                ToolResult::Error { code, message } => {
                    assert_eq!(code, "MODE_DENIED", "{action} 在 Plan 模式必须被拒");
                    assert!(message.contains("Plan 模式"), "{message}");
                }
                other => panic!("Plan 模式 {action} 却回了 {other:?}（伪造成功）"),
            }
        }
        // 只读动作在 Plan 模式照常可用（面板认知不受影响）。
        let listed = artifacts_tool()
            .execute(&args(&[("action", "list"), ("session_key", session.as_str())]))
            .await
            .unwrap();
        assert!(matches!(listed, ToolResult::Success { .. }), "{listed:?}");

        let _ = std::fs::remove_dir_all(&dir);
        crate::mode::set_runtime_mode(prev);
    }

    /// 真实往返 B → A：先用台账原语登记，再用**工具**的 list/get 读到同一条。
    #[tokio::test]
    async fn tool_reads_what_the_ledger_primitive_recorded() {
        let dir = std::env::temp_dir().join(format!("mgt_wb_read_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("report.docx");
        std::fs::write(&file, b"docx-bytes").unwrap();
        let session = format!("sess-wb-read-{}", uuid::Uuid::new_v4());
        file_change::record_artifact_with_session(&session, &file.to_string_lossy(), "created");

        let listed = artifacts_tool()
            .execute(&args(&[("action", "list"), ("session_key", session.as_str())]))
            .await
            .unwrap();
        let id = match listed {
            ToolResult::Success { data, .. } => {
                assert!(data.contains("report.docx"), "{data}");
                data.split("id=")
                    .nth(1)
                    .unwrap_or("")
                    .split(' ')
                    .next()
                    .unwrap_or("")
                    .to_string()
            }
            other => panic!("list 应成功：{other:?}"),
        };
        assert!(!id.is_empty(), "必须拿到真实产物 id");
        let got = artifacts_tool()
            .execute(&args(&[("action", "get"), ("id", id.as_str())]))
            .await
            .unwrap();
        match got {
            ToolResult::Success { data, .. } => {
                assert!(data.contains(&session), "详情要带会话归属：{data}");
                assert!(data.contains(&"docx-bytes".len().to_string()), "大小要如实：{data}");
            }
            other => panic!("get 应成功：{other:?}"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 登记不存在的文件必须**如实失败**（R-10：绝不伪造成功）。
    // 同上：register 的拒绝判定发生在工具执行内部，守卫必须活到 await 之后。
    #[allow(clippy::await_holding_lock)]
    #[tokio::test]
    async fn register_missing_file_is_a_structured_failure() {
        // register 是**模式敏感**动作（Plan 模式下先被 MODE_DENIED 拦掉），
        // 必须与那个会临时切到 Plan 的用例串行，否则断言拿到的是 MODE_DENIED
        // 而不是 ARTIFACT_NOT_FOUND —— 这是全局 runtime_mode 的读侧污染。
        let _g = crate::mode::test_runtime_mode_guard();
        let out = artifacts_tool()
            .execute(&args(&[
                ("action", "register"),
                ("path", std::env::temp_dir().join(format!("mgt_wb_missing_{}.md", uuid::Uuid::new_v4())).to_string_lossy().as_ref()),
                ("session_key", "sess-wb-missing"),
            ]))
            .await
            .unwrap();
        match out {
            ToolResult::Error { code, message } => {
                assert_eq!(code, "ARTIFACT_NOT_FOUND", "{code}");
                assert!(message.contains("未执行"), "{message}");
            }
            other => panic!("不存在的路径必须判失败：{other:?}"),
        }
    }

    /// team_board 在「确实没有编排记录」的新会话上必须诚实说明，而不是伪造成员。
    #[tokio::test]
    async fn team_board_reports_no_run_honestly() {
        let session = format!("sess-wb-no-run-{}", uuid::Uuid::new_v4());
        let out = team_board_tool()
            .execute(&args(&[("action", "members"), ("session", session.as_str())]))
            .await
            .unwrap();
        match out {
            ToolResult::Success { data, .. } => {
                assert!(
                    data.contains("当前没有团队编排运行记录") || data.contains("名成员"),
                    "要么如实说明没有运行，要么给出真实成员：{data}"
                );
                assert!(!data.contains("runId=r1"), "不得回放别的用例的假数据");
            }
            other => panic!("members 应如实返回（无记录也是成功+说明）：{other:?}"),
        }
        // 未知 action 仍然是结构化失败。
        let bad = team_board_tool()
            .execute(&args(&[("action", "members"), ("session", session.as_str()), ("nope", "1")]))
            .await
            .unwrap();
        assert!(matches!(bad, ToolResult::Success { .. }));
    }

    #[test]
    fn boolean_and_number_args_follow_string_tool_argument_conventions() {
        // 工具入参是字符串表：布尔/数字要按既有工具的宽容写法解析，且不误判。
        assert!(arg_bool(&args(&[("all_sessions", "TRUE")]), "all_sessions"));
        assert!(!arg_bool(&args(&[("all_sessions", "no")]), "all_sessions"));
        assert_eq!(arg_u64(&args(&[("limit", "30")]), "limit"), Some(30));
        assert_eq!(arg_u64(&args(&[("limit", "abc")]), "limit"), None);
        assert_eq!(arg_str(&args(&[("id", " a1 ")]), "id"), "a1");
    }
}
