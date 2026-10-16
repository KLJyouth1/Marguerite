//! 浏览器 RPC 直通（`browser.*`）：把内置 Chromium/CDP 桥**真实**暴露给网关 WS 客户端。
//!
//! 此前「内置浏览器」能力只在 Agent 工具链路（`tool.browser`）与 CLI（`marguerite browser`）
//! 中可用，网关无独立 RPC → 前端无法直接驱动，设置页能力项只能显示固定占位。
//! 本模块补齐 `browser.status/open/evaluate/screenshot/a11y/click/close`，前端可直接
//! 打开真实页面、求值、截图、读无障碍树、点击，无需经 chat 工具链路。
//!
//! 会话注册表进程内常驻（空闲 10 分钟回收，上限 8 个），`browser.open` 返回 `sessionId`，
//! 后续动作复用同一页面会话（跨调用持续操控）。

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use tokio::sync::Mutex;

use crate::browser::{BrowserBridge, PageSession};
use crate::gateway::protocol::RpcError;

/// 页面会话条目
struct BrowserEntry {
    page: PageSession,
    touched: Instant,
}

const SESSION_TTL: Duration = Duration::from_secs(600);
const SESSION_MAX: usize = 8;
const LOAD_SETTLE_MS: u64 = 1200;

fn sessions() -> &'static Mutex<HashMap<String, BrowserEntry>> {
    static S: OnceLock<Mutex<HashMap<String, BrowserEntry>>> = OnceLock::new();
    S.get_or_init(|| Mutex::new(HashMap::new()))
}

/// 回收空闲超时会话并裁剪到上限（保留最近活跃者）
fn purge_stale(map: &mut HashMap<String, BrowserEntry>) {
    let now = Instant::now();
    map.retain(|_, e| now.duration_since(e.touched) < SESSION_TTL);
    while map.len() > SESSION_MAX {
        let oldest = map
            .iter()
            .min_by_key(|(_, e)| e.touched)
            .map(|(k, _)| k.clone());
        if let Some(oldest) = oldest {
            map.remove(&oldest);
        } else {
            break;
        }
    }
}

/// 生成进程内唯一会话 id（`br-<ts>-<seq>`）
fn new_session_id() -> String {
    static SEQ: AtomicU64 = AtomicU64::new(1);
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    format!("br-{ts}-{:06}", SEQ.fetch_add(1, Ordering::Relaxed))
}

/// `browser.status`：探测**真实**可用性（Chromium 可执行文件检测）。
pub fn status() -> Value {
    let bridge = BrowserBridge::new();
    let available = bridge.is_available();
    json!({
        "available": available,
        "backend": "cdp",
        "bridge": "browser_bridge",
        "code": if available { "ready" } else { "not_setup" },
        "reason": if available {
            "检测到 Chromium/Chrome，内置浏览器桥可用"
        } else {
            "未检测到 Chromium/Chrome 可执行文件；browser_bridge 不可用"
        },
        "capabilities": if available {
            serde_json::json!(["browser", "navigate", "evaluate", "screenshot", "a11y", "click"])
        } else {
            serde_json::json!([])
        },
    })
}

/// `browser.open {url?}`：打开页面会话，返回 sessionId + 标题 + 当前 URL。
pub async fn open(params: &Value, bridge: &BrowserBridge) -> Result<Value, RpcError> {
    let url = params
        .get("url")
        .and_then(Value::as_str)
        .unwrap_or("about:blank");
    // P2-9：SSRF 预检（不依赖 Chrome 内建拦截兜底）。about:blank 等非 http(s)
    // 由 SsrfGuard 拒绝，但浏览器打开空白页属合法操作，此处显式放行。
    if url != "about:blank" {
        crate::security::ssrf_guard::SsrfGuard::new()
            .check(url)
            .map_err(|e| RpcError::new("SSRF_BLOCKED", format!("目标地址被安全策略拒绝: {e}")))?;
    }
    let mut page = bridge
        .open(url)
        .await
        .map_err(|e| RpcError::new("BROWSER_LAUNCH_FAILED", format!("启动浏览器失败: {e}")))?;
    tokio::time::sleep(Duration::from_millis(LOAD_SETTLE_MS)).await;

    let title = page
        .session
        .evaluate("document.title", false)
        .await
        .unwrap_or_default();
    let href = page
        .session
        .evaluate("location.href", false)
        .await
        .unwrap_or_default();

    let id = new_session_id();
    let mut map = sessions().lock().await;
    map.insert(
        id.clone(),
        BrowserEntry {
            page,
            touched: Instant::now(),
        },
    );
    purge_stale(&mut map);

    Ok(json!({
        "sessionId": id,
        "title": title,
        "url": if href.is_empty() { url.to_string() } else { href },
    }))
}

/// 取会话引用（不存在返回 NOT_FOUND），并刷新活跃时间。
async fn take_session<'a>(
    map: &'a mut HashMap<String, BrowserEntry>,
    session_id: &str,
) -> Result<&'a mut BrowserEntry, RpcError> {
    let entry = map
        .get_mut(session_id)
        .ok_or_else(|| RpcError::new("BROWSER_SESSION_NOT_FOUND", "浏览器会话不存在或已回收"))?;
    entry.touched = Instant::now();
    Ok(entry)
}

/// `browser.navigate {sessionId, url}`：导航到新 URL。
pub async fn navigate(params: &Value) -> Result<Value, RpcError> {
    let session_id = params
        .get("sessionId")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let url = params
        .get("url")
        .and_then(Value::as_str)
        // P2-9：SSRF 预检（与 open 同策略；about:blank 显式放行）
        .map(str::to_string)
        .unwrap_or_default();
    if !url.is_empty() && url != "about:blank" {
        crate::security::ssrf_guard::SsrfGuard::new()
            .check(&url)
            .map_err(|e| RpcError::new("SSRF_BLOCKED", format!("目标地址被安全策略拒绝: {e}")))?;
    }
    let mut map = sessions().lock().await;
    let entry = take_session(&mut map, session_id).await?;
    entry
        .page
        .session
        .navigate(&url)
        .await
        .map_err(|e| RpcError::new("BROWSER_NAVIGATE_FAILED", format!("导航失败: {e}")))?;
    tokio::time::sleep(Duration::from_millis(LOAD_SETTLE_MS)).await;
    let title = entry
        .page
        .session
        .evaluate("document.title", false)
        .await
        .unwrap_or_default();
    let href = entry
        .page
        .session
        .evaluate("location.href", false)
        .await
        .unwrap_or_default();
    Ok(json!({ "sessionId": session_id, "title": title, "url": href }))
}

/// `browser.evaluate {sessionId, expression, awaitPromise?}`：执行 JS 并返回字符串结果。
pub async fn evaluate(params: &Value) -> Result<Value, RpcError> {
    let session_id = params
        .get("sessionId")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let expression = params
        .get("expression")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let await_promise = params
        .get("awaitPromise")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let mut map = sessions().lock().await;
    let entry = take_session(&mut map, session_id).await?;
    let result = entry
        .page
        .session
        .evaluate(expression, await_promise)
        .await
        .unwrap_or_default();
    Ok(json!({ "sessionId": session_id, "result": result }))
}

/// `browser.screenshot {sessionId}`：返回 PNG base64 data URL。
pub async fn screenshot(params: &Value) -> Result<Value, RpcError> {
    let session_id = params
        .get("sessionId")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let mut map = sessions().lock().await;
    let entry = take_session(&mut map, session_id).await?;
    let png = entry
        .page
        .session
        .screenshot_png()
        .await
        .map_err(|e| RpcError::new("BROWSER_SCREENSHOT_FAILED", format!("截图失败: {e}")))?;
    use base64::Engine;
    let b64 = base64::engine::general_purpose::STANDARD.encode(&png);
    Ok(json!({
        "sessionId": session_id,
        "mime": "image/png",
        "width": png.len(),
        "dataUrl": format!("data:image/png;base64,{b64}"),
    }))
}

/// `browser.a11y {sessionId, max?}`：返回页面可交互无障碍节点简述（非视觉语义读取）。
pub async fn a11y(params: &Value) -> Result<Value, RpcError> {
    let session_id = params
        .get("sessionId")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let max = params.get("max").and_then(Value::as_u64).unwrap_or(50) as usize;
    let mut map = sessions().lock().await;
    let entry = take_session(&mut map, session_id).await?;
    let nodes = entry.page.session.a11y_nodes().await.unwrap_or_default();
    let briefs: Vec<Value> = nodes
        .into_iter()
        .take(max)
        .map(|n| {
            json!({
                "id": n.backend_node_id,
                "role": n.role,
                "name": n.name,
            })
        })
        .collect();
    Ok(json!({ "sessionId": session_id, "nodes": briefs, "count": briefs.len() }))
}

/// `browser.click {sessionId, x, y}`：在视口 CSS 像素坐标执行一次真实鼠标左键点击。
pub async fn click(params: &Value) -> Result<Value, RpcError> {
    let session_id = params
        .get("sessionId")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let x = params
        .get("x")
        .and_then(Value::as_f64)
        .ok_or_else(|| RpcError::new("INVALID_REQUEST", "缺少数值参数 x"))?;
    let y = params
        .get("y")
        .and_then(Value::as_f64)
        .ok_or_else(|| RpcError::new("INVALID_REQUEST", "缺少数值参数 y"))?;
    let mut map = sessions().lock().await;
    let entry = take_session(&mut map, session_id).await?;
    entry
        .page
        .session
        .click_at(x, y)
        .await
        .map_err(|e| RpcError::new("BROWSER_CLICK_FAILED", format!("点击失败: {e}")))?;
    Ok(json!({ "sessionId": session_id, "ok": true }))
}

/// `browser.close {sessionId}`：关闭会话并终止其 Chromium 子进程。
pub async fn close(params: &Value) -> Result<Value, RpcError> {
    let session_id = params
        .get("sessionId")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let mut map = sessions().lock().await;
    if map.remove(session_id).is_some() {
        Ok(json!({ "ok": true, "closed": session_id }))
    } else {
        Err(RpcError::new(
            "BROWSER_SESSION_NOT_FOUND",
            "浏览器会话不存在或已回收",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_status_probe_honest_availability() {
        // 探测是诚实的：Chromium 检测结果决定 available/code，而非固定占位。
        let s = status();
        assert_eq!(s["backend"], "cdp");
        assert_eq!(s["bridge"], "browser_bridge");
        // available 与 code 必须一致（ready ⇔ available=true）
        let available = s["available"].as_bool().unwrap();
        let code = s["code"].as_str().unwrap();
        assert_eq!(available, code == "ready");
        if available {
            assert!(s["capabilities"]
                .as_array()
                .unwrap()
                .contains(&json!("navigate")));
        } else {
            assert_eq!(s["capabilities"].as_array().unwrap().len(), 0);
        }
    }

    #[test]
    fn test_new_session_id_unique_and_prefixed() {
        let a = new_session_id();
        let b = new_session_id();
        assert_ne!(a, b);
        assert!(a.starts_with("br-"));
        assert!(b.starts_with("br-"));
    }
}
