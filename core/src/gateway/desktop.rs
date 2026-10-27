//! 桌面自动化（Epic C）RPC：列出 / 启动 / 附着 / 动作 / 控件树 / 窗口 / 快照 / 急停。
//!
//! 封装 `desktop_automation::AppRegistry`，为前端「桌面控制台」面板与模型工具
//! 提供启动 / 附着 / 动作 / 审计快照能力。
//!
//! # 契约（RPC 信封沿用本模块既有 `{code,message,data}`）
//!
//! - `desktop.windows {}` → `data = {count, windows:[{hwnd,title,process_name,pid,foreground,iconic,bounds}]}`
//! - `desktop.a11y {window?, maxDepth?}` →
//!   `data = {window, count, nodes:[{ref_id,role,name,automation_id,bounds,enabled,focused,patterns}]}`
//! - `desktop.action {kind, params, handle?, window?, approved?, session_key?}` →
//!   `data = ActionReceipt`（`handle` 可省略：缺省时按 `window` 附着，再退化到前台窗口）。
//!   同时接受旧形态 `{handle, action:{kind,params}}`。
//! - `desktop.attach {window?}` → `data = {handle, kind, launched_at}`
//! - `desktop.list_apps` / `desktop.launch` / `desktop.snapshots` / `desktop.stop`：保持不变。
//!
//! # 分派线程模型
//!
//! `desktop.a11y` / `desktop.action` / `desktop.windows` 是**阻塞**的 COM/UIA 调用
//! （单次最长 `UIA_CALL_TIMEOUT`=10s），由 `gateway/methods.rs` 的异步分派用
//! `tokio::task::spawn_blocking` 执行，并登记在 `ASYNC_ONLY_METHODS` —— 绝不在
//! tokio worker 线程上直接跑（否则一次读树会卡住同一运行时的其它 WS RPC）。
//!
//! # 错误码
//!
//! | code | 标签 | 含义 |
//! |---|---|---|
//! | 2401 | `APP_UNKNOWN` | 应用种类未注册 |
//! | 2402 | `UNSUPPORTED_ACTION` | 未知/不支持的动作或平台 |
//! | 2403 | `ACTION_DENIED` | 安全闸门拒绝（需 approved=true） |
//! | 2404 | `WINDOW_NOT_FOUND` | 空窗口 / 按标题找不到窗口 / 无前台窗口 |
//! | 2405 | `ACTION_TIMEOUT` | wait 轮询超时 |
//! | 2406 | `ELEMENT_NOT_FOUND` | 按 ref_id/name 找不到控件 |
//! | 2407 | `DESKTOP_NOT_RUNNING` | 句柄未绑定（未附着 / 已 quit） |
//! | 2505 | `DESKTOP_COM_ERROR` | COM / UIA 通道错误 |

use serde_json::{json, Value};

use crate::desktop_automation::a11y_tree::UiSnapshot;
use crate::desktop_automation::driver::{is_known_action_kind, DesktopError, UiAction};
use crate::desktop_automation::uia_driver;
use crate::desktop_automation::windows;
use crate::desktop_automation::{AppKind, AppRegistry, Policy, Recorder};
use crate::events::Envelope;
use crate::gateway::_helpers::{rpc_err, rpc_ok};
use crate::gateway::methods::GatewayContext;

/// 业务错误码 + 稳定标签（标签同时写进 message 前缀，客户端可正则/日志检索）。
fn desktop_error_code(e: &DesktopError) -> (u32, &'static str) {
    match e {
        DesktopError::AppUnknown(_) => (2401, "APP_UNKNOWN"),
        DesktopError::UnsupportedAction(_) => (2402, "UNSUPPORTED_ACTION"),
        DesktopError::ActionDenied(_) => (2403, "ACTION_DENIED"),
        DesktopError::WindowNotFound(_) => (2404, "WINDOW_NOT_FOUND"),
        DesktopError::Timeout(_) => (2405, "ACTION_TIMEOUT"),
        DesktopError::ElementNotFound(_) => (2406, "ELEMENT_NOT_FOUND"),
        DesktopError::NotRunning(_) => (2407, "DESKTOP_NOT_RUNNING"),
        DesktopError::ComError(_) => (2505, "DESKTOP_COM_ERROR"),
    }
}

/// 把 [`DesktopError`] 变成稳定信封（`message` 形如 `WINDOW_NOT_FOUND: 窗口未找到: …`）。
fn desktop_error(e: &DesktopError) -> Value {
    let (code, tag) = desktop_error_code(e);
    rpc_err(code, &format!("{tag}: {e}"))
}

/// `desktop.list_apps`：已注册 + 运行中应用列表。
pub fn desktop_list_apps() -> Value {
    let apps = AppRegistry::global().list_apps();
    rpc_ok(json!({ "apps": apps }))
}

/// `desktop.launch`：按 app_kind 启动应用，返回句柄。
pub fn desktop_launch(params: &Value) -> Value {
    let app_kind = params.get("app_kind").and_then(Value::as_str).unwrap_or("");
    let kind = match AppKind::from_str(app_kind) {
        Some(k) => k,
        None => return rpc_err(2401, &format!("APP_UNKNOWN: 未知桌面应用 {app_kind}")),
    };
    match AppRegistry::global().launch(kind, params) {
        Ok(h) => rpc_ok(json!({
            "handle": h.id,
            "kind": h.kind.as_str(),
            "launched_at": h.launched_at,
        })),
        Err(e) => desktop_error(&e),
    }
}

/// 归一 `window` 参数：去空白，空串视为「未提供」（模型/前端很容易传 `""`）。
fn normalized_window(params: &Value) -> Option<String> {
    params
        .get("window")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// `desktop.attach`：附着已打开的应用（W6.2），返回可复用句柄。
///
/// 空 `window` 视为「附着前台窗口」；按标题找不到窗口 → `WINDOW_NOT_FOUND`。
pub fn desktop_attach(params: &Value) -> Value {
    desktop_attach_with(AppRegistry::global(), params)
}

/// [`desktop_attach`] 的注入版：注册表作为参数传入，便于单测用假驱动替换真实桌面。
fn desktop_attach_with(registry: &AppRegistry, params: &Value) -> Value {
    match registry.attach(normalized_window(params).as_deref()) {
        Ok(h) => rpc_ok(json!({
            "handle": h.id,
            "kind": h.kind.as_str(),
            "launched_at": h.launched_at,
        })),
        Err(e) => desktop_error(&e),
    }
}

/// 解析动作：优先 `action:{kind,params}`，其次顶层 `{kind, params}`（W6.5 契约形态）。
fn parse_action(params: &Value) -> Result<UiAction, String> {
    let value = match params.get("action") {
        Some(v) if v.is_object() => v.clone(),
        Some(v) if v.as_str().is_some() => {
            // 模型工具把 action 当 JSON 字符串传；网关侧也接受，避免两套形状。
            serde_json::from_str::<Value>(v.as_str().unwrap_or_default())
                .map_err(|e| format!("action 字符串不是合法 JSON: {e}"))?
        }
        _ => {
            let kind = params
                .get("kind")
                .and_then(Value::as_str)
                .unwrap_or("")
                .trim()
                .to_string();
            if kind.is_empty() {
                return Err("缺少 action（或顶层 kind）".to_string());
            }
            json!({
                "kind": kind,
                "params": params.get("params").cloned().unwrap_or(Value::Null),
            })
        }
    };
    serde_json::from_value::<UiAction>(value).map_err(|e| format!("动作参数解析失败: {e}"))
}

/// `desktop.action`：执行 UI 动作（敏感动作先查 Policy 二次确认闸门）。
pub fn desktop_action(ctx: &GatewayContext, params: &Value) -> Value {
    desktop_action_with(AppRegistry::global(), ctx, params)
}

/// [`desktop_action`] 的注入版：注册表作为参数传入，便于单测用假驱动验证
/// 「附着 → 转发 → 回执」的形状，而**不依赖真实桌面会话**。
fn desktop_action_with(registry: &AppRegistry, ctx: &GatewayContext, params: &Value) -> Value {
    let action = match parse_action(params) {
        Ok(a) => a,
        Err(msg) => return rpc_err(2402, &format!("UNSUPPORTED_ACTION: {msg}")),
    };
    // 未知动作在碰 UI 之前就拒绝：错误确定，且不依赖桌面会话。
    if !is_known_action_kind(&action.kind) {
        return rpc_err(
            2402,
            &format!("UNSUPPORTED_ACTION: 未知桌面动作 {}", action.kind),
        );
    }
    // 安全闸门：敏感动作需二次确认（前端携带 approved=true 才放行）。
    // 点击目标命中危险词（删除/支付/发送…）同样走确认，而不是只看 kind。
    if Policy::require_confirmation(&action)
        && !params
            .get("approved")
            .and_then(Value::as_bool)
            .unwrap_or(false)
    {
        let reason = Policy::sensitive_target(&action)
            .map(|term| {
                format!("该动作需要二次确认：目标命中危险操作词「{term}」（前端请携带 approved=true）")
            })
            .unwrap_or_else(|| "该动作需要二次确认（前端请携带 approved=true）".into());
        return rpc_err(2403, &format!("ACTION_DENIED: {reason}"));
    }
    // 注入扫描：只扫会进入命令/终端的字段（普通文本输入不再误拦）
    if let Some(reason) = Policy::scan_command_text(&action) {
        return rpc_err(2403, &format!("ACTION_DENIED: {reason}"));
    }
    // handle 可省略（W6.2）：缺省时按 window 附着，再退化到前台窗口。
    let mut handle = params
        .get("handle")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();
    if handle.is_empty() {
        // 顶层 window 优先，其次动作参数里的 window（两条来源都接受，避免模型
        // 把 window 写进 action.params 就失效）。
        let window = normalized_window(params).or_else(|| action.param_str_nonempty("window"));
        match registry.attach(window.as_deref()) {
            Ok(h) => handle = h.id,
            Err(e) => return desktop_error(&e),
        }
    }
    let session = params
        .get("session_key")
        .and_then(Value::as_str)
        .unwrap_or("");
    match registry.run_action(&handle, &action, session) {
        Ok(receipt) => {
            let receipt_val = serde_json::to_value(&receipt).unwrap_or(Value::Null);
            // 广播 desktop.action_done（前端审计回执）
            let envelope = |bus: &crate::events::EventBus| {
                bus.publish(Envelope::desktop_action_done(
                    session,
                    &receipt.handle,
                    &receipt.action,
                    &receipt_val,
                    receipt.ts,
                ));
            };
            if let Some(bus) = crate::events::global_bus() {
                envelope(&bus);
            } else {
                // 全局 bus 未初始化时回退到网关上下文 bus（保证事件不丢）
                envelope(&ctx.bus);
            }
            rpc_ok(receipt_val)
        }
        Err(e) => desktop_error(&e),
    }
}

/// `desktop.windows`：可见顶层窗口列表（原生 `EnumWindows`，不经 PowerShell）。
pub fn desktop_windows() -> Value {
    match windows::list_windows() {
        Ok(windows) => rpc_ok(json!({ "count": windows.len(), "windows": windows })),
        Err(e) => desktop_error(&e),
    }
}

/// 归一 `desktop.a11y` 的请求参数（纯函数，便于单测）：
/// `window` 去空白（空 → `None`），`maxDepth` / `max_depth` 只接受正整数
/// （0 / 负数 / 小数 / 字符串一律视为「未提供」，由后端取默认上限）。
fn a11y_request(params: &Value) -> (Option<String>, Option<usize>) {
    let window = normalized_window(params);
    let max_depth = params
        .get("maxDepth")
        .or_else(|| params.get("max_depth"))
        .and_then(Value::as_u64)
        .filter(|v| *v > 0)
        .and_then(|v| usize::try_from(v).ok());
    (window, max_depth)
}

/// `desktop.a11y` 的响应体（纯函数，不碰任何 OS API）：
/// `{window, count, nodes:[{ref_id,role,name,automation_id,bounds,enabled,focused,patterns,…}]}`。
fn a11y_payload(window: Option<windows::WindowInfo>, snap: &UiSnapshot) -> Value {
    let nodes: Vec<Value> = snap.nodes.iter().map(uia_driver::node_json).collect();
    json!({
        "window": window,
        "count": nodes.len(),
        "nodes": nodes,
    })
}

/// `desktop.a11y`：读控件树（可选先附着窗口；`maxDepth` 受硬上限约束）。
pub fn desktop_a11y(params: &Value) -> Value {
    let (window, max_depth) = a11y_request(params);
    match uia_driver::a11y_snapshot_for_rpc(window.as_deref(), max_depth) {
        Ok((window, snap)) => rpc_ok(a11y_payload(window, &snap)),
        Err(e) => desktop_error(&e),
    }
}

/// `desktop.snapshots`：列出某会话的审计快照。
pub fn desktop_snapshots(params: &Value) -> Value {
    let session = params
        .get("session_key")
        .and_then(Value::as_str)
        .unwrap_or("");
    let snaps = Recorder::list_snapshots(session);
    rpc_ok(json!({ "snapshots": snaps }))
}

/// `desktop.stop`：急停所有运行中应用。
pub fn desktop_stop() -> Value {
    let _ = AppRegistry::global().stop_all();
    rpc_ok(json!({ "ok": true }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn ctx() -> GatewayContext {
        GatewayContext {
            config: crate::persistence::config::Config::default(),
            token: String::new(),
            started_at: std::time::Instant::now(),
            bus: Arc::new(crate::events::EventBus::default()),
        }
    }

    fn code_of(v: &Value) -> u64 {
        v["code"].as_u64().unwrap_or(u64::MAX)
    }

    #[test]
    fn unknown_kind_is_a_structured_error_without_touching_the_desktop() {
        // W6.5 契约测试：未知 kind 必须在附着/执行之前返回结构化错误。
        let v = desktop_action(&ctx(), &json!({ "kind": "no_such_kind", "params": {} }));
        assert_eq!(code_of(&v), 2402, "实际: {v}");
        let message = v["message"].as_str().unwrap_or_default();
        assert!(message.contains("UNSUPPORTED_ACTION"), "message 必须带稳定标签: {message}");
        assert!(message.contains("no_such_kind"), "message 必须指出是哪个动作: {message}");
    }

    #[test]
    fn unknown_kind_inside_the_legacy_action_object_is_also_rejected() {
        let v = desktop_action(
            &ctx(),
            &json!({ "handle": "whatever", "action": { "kind": "not_a_kind" } }),
        );
        assert_eq!(code_of(&v), 2402, "实际: {v}");
    }

    #[test]
    fn missing_action_is_rejected_as_unsupported_not_as_missing_handle() {
        let v = desktop_action(&ctx(), &json!({ "handle": "h" }));
        assert_eq!(code_of(&v), 2402, "实际: {v}");
        assert!(v["message"]
            .as_str()
            .unwrap_or_default()
            .contains("缺少 action"));
    }

    #[test]
    fn parse_action_accepts_both_shapes() {
        let nested = parse_action(&json!({
            "action": { "kind": "invoke", "params": { "ref_id": "1.2" } }
        }))
        .expect("嵌套形态");
        assert_eq!(nested.kind, "invoke");
        assert_eq!(nested.params["ref_id"], "1.2");

        let flat = parse_action(&json!({ "kind": "set_value", "params": { "value": "x" } }))
            .expect("顶层形态");
        assert_eq!(flat.kind, "set_value");
        assert_eq!(flat.params["value"], "x");

        let as_string = parse_action(&json!({
            "action": "{\"kind\":\"focus\",\"params\":{\"ref_id\":\"1\"}}"
        }))
        .expect("字符串形态");
        assert_eq!(as_string.kind, "focus");
    }

    #[test]
    fn policy_gate_still_blocks_sensitive_targets_before_any_ui_call() {
        // 点「删除」必须要求 approved=true；错误在附着之前返回 2403，与桌面会话无关。
        let v = desktop_action(
            &ctx(),
            &json!({
                "kind": "click",
                "params": { "name": "删除" },
                "window": "记事本"
            }),
        );
        assert_eq!(code_of(&v), 2403, "实际: {v}");
        assert!(v["message"]
            .as_str()
            .unwrap_or_default()
            .contains("ACTION_DENIED"));
    }

    #[test]
    fn every_desktop_error_maps_to_a_distinct_documented_code() {
        let cases = [
            (DesktopError::AppUnknown("x".into()), 2401u32, "APP_UNKNOWN"),
            (DesktopError::UnsupportedAction("x".into()), 2402, "UNSUPPORTED_ACTION"),
            (DesktopError::ActionDenied("x".into()), 2403, "ACTION_DENIED"),
            (DesktopError::WindowNotFound("x".into()), 2404, "WINDOW_NOT_FOUND"),
            (DesktopError::Timeout("x".into()), 2405, "ACTION_TIMEOUT"),
            (DesktopError::ElementNotFound("x".into()), 2406, "ELEMENT_NOT_FOUND"),
            (DesktopError::NotRunning("x".into()), 2407, "DESKTOP_NOT_RUNNING"),
            (DesktopError::ComError("x".into()), 2505, "DESKTOP_COM_ERROR"),
        ];
        for (err, code, tag) in cases {
            let v = desktop_error(&err);
            assert_eq!(code_of(&v), u64::from(code), "{tag} 码不对: {v}");
            assert!(
                v["message"].as_str().unwrap_or_default().starts_with(tag),
                "{tag} 标签未写进 message: {v}"
            );
        }
    }

    /// 非 Windows：没有 Generic 驱动，`desktop.attach` 必须诚实报 APP_UNKNOWN
    /// （不能凭空发一个「已附着」的假句柄）。Windows 上该路径依赖真实桌面，故只在
    /// 非 Windows 上断言——不是放水，而是这条用例在 Windows 上本来就有另一条真实路径。
    #[cfg(not(windows))]
    #[test]
    fn attach_on_a_platform_without_uia_is_app_unknown() {
        let v = desktop_attach(&json!({ "window": "记事本" }));
        assert_eq!(code_of(&v), 2401, "实际: {v}");
        assert!(v["message"]
            .as_str()
            .unwrap_or_default()
            .contains("APP_UNKNOWN"));
    }

    /// 窗口枚举与控件树都必须带信封：成功时是真实数组，失败时是结构化错误，
    /// 绝不允许「code=0 但空 nodes」冒充「读到了界面」。
    #[test]
    fn windows_and_a11y_are_enveloped_and_never_fake_data() {
        let w = desktop_windows();
        assert!(w["code"].as_u64().is_some(), "必须是信封: {w}");
        if code_of(&w) == 0 {
            assert!(w["data"]["windows"].is_array(), "成功时必须是真实数组: {w}");
        } else {
            assert!(w["message"].as_str().unwrap_or_default().contains("UNSUPPORTED_ACTION"));
        }

        let a = desktop_a11y(&json!({ "maxDepth": 2 }));
        if code_of(&a) == 0 {
            assert!(a["data"]["nodes"].is_array(), "{a}");
        } else {
            // 无桌面会话时允许明确错误；绝不允许「code=0 但空 nodes 冒充读到界面」.
            assert!(a["data"].is_null(), "{a}");
        }
    }

    // ── W6.2 / W6.5：假驱动测试（不依赖真实桌面会话）──────────────────────

    use crate::desktop_automation::a11y_tree::{UiNode, UiPattern, UiSnapshot};
    use crate::desktop_automation::driver::{
        ActionReceipt, AppHandle, Bounds, DesktopAppDriver, ObservationEffect,
    };
    use std::sync::Mutex;

    /// 假驱动：记录附着/动作调用，`window` 以 `missing` 开头时模拟找窗口失败。
    struct FakeDriver {
        launches: Mutex<Vec<String>>,
        actions: Mutex<Vec<(String, String, Value, String)>>,
    }

    impl FakeDriver {
        fn new() -> Arc<Self> {
            Arc::new(FakeDriver {
                launches: Mutex::new(Vec::new()),
                actions: Mutex::new(Vec::new()),
            })
        }

        fn launch_count(&self) -> usize {
            self.launches.lock().unwrap().len()
        }

        fn first_launch(&self) -> Option<String> {
            self.launches.lock().unwrap().first().cloned()
        }

        fn last_action(&self) -> Option<(String, String, Value, String)> {
            self.actions.lock().unwrap().last().cloned()
        }
    }

    impl DesktopAppDriver for Arc<FakeDriver> {
        fn kind(&self) -> AppKind {
            AppKind::Generic
        }

        fn launch(&self, args: &Value) -> Result<AppHandle, DesktopError> {
            let window = args
                .get("window")
                .and_then(Value::as_str)
                .unwrap_or("")
                .trim()
                .to_string();
            if window.starts_with("missing") {
                return Err(DesktopError::WindowNotFound(format!("窗口未找到: {window}")));
            }
            self.launches.lock().unwrap().push(window);
            Ok(AppHandle {
                id: "mock-handle".into(),
                kind: AppKind::Generic,
                launched_at: 1_700_000_000,
            })
        }

        fn activate(&self, _h: &AppHandle) -> Result<(), DesktopError> {
            Ok(())
        }

        fn run_action(
            &self,
            h: &AppHandle,
            action: &UiAction,
            session: &str,
        ) -> Result<ActionReceipt, DesktopError> {
            self.actions.lock().unwrap().push((
                h.id.clone(),
                action.kind.clone(),
                action.params.clone(),
                session.to_string(),
            ));
            Ok(
                ActionReceipt::ok(&h.id, &action.kind, json!({ "ref_id": action.params.get("ref_id") }), None)
                    .with_observation(
                        Some(json!({ "value": "old" })),
                        Some(json!({ "value": "new" })),
                        ObservationEffect::ValueChanged,
                    ),
            )
        }

        fn quit(&self, _h: &AppHandle) -> Result<(), DesktopError> {
            Ok(())
        }
    }

    fn registry_with(driver: &Arc<FakeDriver>) -> AppRegistry {
        let reg = AppRegistry::new();
        reg.register(AppKind::Generic, Box::new(Arc::clone(driver)));
        reg
    }

    #[test]
    fn attach_returns_a_registered_reusable_handle_and_trims_the_query() {
        let driver = FakeDriver::new();
        let reg = registry_with(&driver);

        let v = desktop_attach_with(&reg, &json!({ "window": "  记事本  " }));
        assert_eq!(code_of(&v), 0, "实际: {v}");
        assert_eq!(v["data"]["handle"], "mock-handle", "{v}");
        assert_eq!(v["data"]["kind"], "generic", "{v}");
        assert_eq!(
            driver.first_launch().as_deref(),
            Some("记事本"),
            "附着查询必须去空白后传给驱动"
        );

        // 句柄已登记进注册表：带 handle 的动作不得再次附着。
        let run = desktop_action_with(
            &reg,
            &ctx(),
            &json!({ "handle": "mock-handle", "kind": "invoke", "params": { "ref_id": "1.2" } }),
        );
        assert_eq!(code_of(&run), 0, "实际: {run}");
        assert_eq!(driver.launch_count(), 1, "显式 handle 不得重复附着");
    }

    #[test]
    fn attach_window_param_must_be_a_string_or_it_means_foreground() {
        assert_eq!(
            normalized_window(&json!({ "window": " 记事本 " })),
            Some("记事本".into())
        );
        assert_eq!(normalized_window(&json!({ "window": 42 })), None);
        assert_eq!(normalized_window(&json!({})), None);
    }

    #[test]
    fn empty_window_attaches_the_foreground_window() {
        let driver = FakeDriver::new();
        let reg = registry_with(&driver);
        let v = desktop_attach_with(&reg, &json!({ "window": "   " }));
        assert_eq!(code_of(&v), 0, "实际: {v}");
        assert_eq!(
            driver.first_launch().as_deref(),
            Some(""),
            "空 window 必须退化为「附着前台窗口」（驱动收到空查询串）"
        );
    }

    #[test]
    fn missing_window_without_handle_is_window_not_found_and_no_action_runs() {
        let driver = FakeDriver::new();
        let reg = registry_with(&driver);
        let v = desktop_action_with(
            &reg,
            &ctx(),
            &json!({
                "kind": "set_value",
                "params": { "ref_id": "1.2", "value": "hi" },
                "window": "missing-notepad",
            }),
        );
        assert_eq!(code_of(&v), 2404, "实际: {v}");
        assert!(
            v["message"].as_str().unwrap_or_default().starts_with("WINDOW_NOT_FOUND"),
            "message 必须带稳定标签: {v}"
        );
        assert!(driver.last_action().is_none(), "附着失败时不得执行动作");
    }

    #[test]
    fn action_without_handle_attaches_the_window_and_returns_the_receipt_shape() {
        let driver = FakeDriver::new();
        let reg = registry_with(&driver);
        let v = desktop_action_with(
            &reg,
            &ctx(),
            &json!({
                "kind": "set_value",
                "params": { "ref_id": "1.2", "value": "hi" },
                "window": "记事本",
                "session_key": "sess-1",
            }),
        );
        assert_eq!(code_of(&v), 0, "实际: {v}");
        let receipt = &v["data"];
        assert_eq!(receipt["handle"], "mock-handle", "{v}");
        assert_eq!(receipt["action"], "set_value", "{v}");
        assert_eq!(receipt["ok"], true, "{v}");
        assert_eq!(receipt["verified"], true, "{v}");
        assert_eq!(receipt["effect"], "value_changed", "{v}");
        assert_eq!(receipt["before"]["value"], "old", "{v}");

        let (handle, kind, params, session) = driver.last_action().expect("动作必须转发给驱动");
        assert_eq!(handle, "mock-handle");
        assert_eq!(kind, "set_value");
        assert_eq!(params["ref_id"], "1.2");
        assert_eq!(params["value"], "hi");
        assert_eq!(session, "sess-1");
    }

    #[test]
    fn window_inside_action_params_also_counts_as_the_attach_target() {
        let driver = FakeDriver::new();
        let reg = registry_with(&driver);
        let v = desktop_action_with(
            &reg,
            &ctx(),
            &json!({ "kind": "invoke", "params": { "window": "计算器", "ref_id": "9" } }),
        );
        assert_eq!(code_of(&v), 0, "实际: {v}");
        assert_eq!(driver.first_launch().as_deref(), Some("计算器"));
    }

    #[test]
    fn a11y_request_normalizes_window_and_depth() {
        assert_eq!(a11y_request(&json!({})), (None, None));
        assert_eq!(
            a11y_request(&json!({ "window": "  ", "maxDepth": 0 })),
            (None, None),
            "空 window / 0 深度必须按未提供处理"
        );
        assert_eq!(
            a11y_request(&json!({ "window": " 记事本 ", "maxDepth": 3 })),
            (Some("记事本".into()), Some(3))
        );
        assert_eq!(
            a11y_request(&json!({ "max_depth": 7 })),
            (None, Some(7)),
            "snake_case 别名必须同样接受"
        );
        // 负数 / 小数 / 字符串一律视为未提供：不猜测调用方意图。
        assert_eq!(a11y_request(&json!({ "maxDepth": -1 })), (None, None));
        assert_eq!(a11y_request(&json!({ "maxDepth": 2.5 })), (None, None));
        assert_eq!(a11y_request(&json!({ "maxDepth": "3" })), (None, None));
    }

    #[test]
    fn a11y_payload_exposes_the_documented_node_fields() {
        let node = UiNode::new("button", "确定", "1.2", (10, 20, 80, 24), true)
            .with_patterns(vec![UiPattern::Invoke]);
        let snap = UiSnapshot { nodes: vec![node] };
        let info = windows::WindowInfo {
            hwnd: 42,
            title: "记事本".into(),
            process_name: "notepad.exe".into(),
            pid: 7,
            foreground: true,
            iconic: false,
            bounds: Bounds::from((0, 0, 1280, 720)),
        };
        let v = a11y_payload(Some(info), &snap);
        assert_eq!(v["window"]["hwnd"], 42, "{v}");
        assert_eq!(v["count"], 1, "{v}");
        let n = &v["nodes"][0];
        for key in [
            "ref_id",
            "role",
            "name",
            "automation_id",
            "bounds",
            "enabled",
            "patterns",
        ] {
            assert!(n.get(key).is_some(), "节点缺少契约字段 {key}: {n}");
        }
        assert_eq!(n["ref_id"], "1.2");
        assert_eq!(n["role"], "button");
        assert_eq!(n["bounds"]["width"], 80);
        assert_eq!(n["patterns"][0], "invoke");

        // window 读不到时必须是 null（绝不编造一个窗口）。
        let none = a11y_payload(None, &UiSnapshot { nodes: vec![] });
        assert!(none["window"].is_null(), "{none}");
        assert_eq!(none["count"], 0, "{none}");
    }
}
