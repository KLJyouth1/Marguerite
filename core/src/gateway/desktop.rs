//! 桌面自动化（Epic C）RPC：列出 / 启动 / 动作 / 快照 / 急停。
//!
//! 封装 `desktop_automation::AppRegistry`，为前端「桌面控制台」面板
//! 提供启动 / 动作 / 审计快照能力。

use serde_json::{json, Value};

use crate::desktop_automation::{AppKind, AppRegistry, DesktopError, Policy, Recorder, UiAction};
use crate::events::Envelope;
use crate::gateway::_helpers::{rpc_err, rpc_ok};
use crate::gateway::methods::GatewayContext;

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
        None => return rpc_err(2401, &format!("未知桌面应用: {app_kind}")),
    };
    match AppRegistry::global().launch(kind, params) {
        Ok(h) => rpc_ok(json!({
            "handle": h.id,
            "kind": h.kind.as_str(),
            "launched_at": h.launched_at,
        })),
        Err(e) => {
            let code = match e {
                DesktopError::AppUnknown(_) => 2401,
                DesktopError::ComError(_) => 2505,
                _ => 2505,
            };
            rpc_err(code, &e.to_string())
        }
    }
}

/// `desktop.action`：对句柄执行 UI 动作（敏感动作先查 Policy 二次确认闸门）。
pub fn desktop_action(ctx: &GatewayContext, params: &Value) -> Value {
    let handle = params.get("handle").and_then(Value::as_str).unwrap_or("");
    if handle.is_empty() {
        return rpc_err(1401, "缺少 handle");
    }
    let action: UiAction =
        match serde_json::from_value(params.get("action").cloned().unwrap_or(Value::Null)) {
            Ok(a) => a,
            Err(e) => return rpc_err(2403, &format!("动作参数解析失败: {e}")),
        };
    // 安全闸门：敏感动作需二次确认（前端携带 approved=true 才放行）
    if Policy::require_confirmation(&action)
        && !params
            .get("approved")
            .and_then(Value::as_bool)
            .unwrap_or(false)
    {
        return rpc_err(2403, "该动作需要二次确认（前端请携带 approved=true）");
    }
    // 注入扫描：命中潜在注入特征直接拒绝（后端兜底）
    if let Some(reason) = Policy::scan_injection(&action) {
        return rpc_err(2403, &reason);
    }
    let session = params
        .get("session_key")
        .and_then(Value::as_str)
        .unwrap_or("");
    match AppRegistry::global().run_action(handle, &action, session) {
        Ok(receipt) => {
            let receipt_val = serde_json::to_value(&receipt).unwrap_or(Value::Null);
            // 广播 desktop.action_done（前端审计回执）
            if let Some(bus) = crate::events::global_bus() {
                bus.publish(Envelope::desktop_action_done(
                    session,
                    &receipt.handle,
                    &receipt.action,
                    &receipt_val,
                    receipt.ts,
                ));
            } else {
                // 全局 bus 未初始化时回退到网关上下文 bus（保证事件不丢）
                ctx.bus.publish(Envelope::desktop_action_done(
                    session,
                    &receipt.handle,
                    &receipt.action,
                    &receipt_val,
                    receipt.ts,
                ));
            }
            rpc_ok(receipt_val)
        }
        Err(e) => {
            let code = match e {
                DesktopError::AppUnknown(_) => 2401,
                DesktopError::ActionDenied(_) => 2403,
                _ => 2505,
            };
            rpc_err(code, &e.to_string())
        }
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
