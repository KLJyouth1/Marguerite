//! Chrome DevTools Protocol（CDP）消息层（纯类型与构造器，离线可测）
//!
//! 把 CDP 的 JSON-RPC 风格命令/响应建模为 Rust 类型，并提供常用命令构造器与
//! 响应解析助手。这些构造器/解析器不依赖网络，可单元测试。

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// 一条 CDP 命令（JSON-RPC 风格，带自增 id 用于匹配响应）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CdpCommand {
    pub id: u64,
    pub method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
}

/// CDP 错误对象
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CdpError {
    pub code: i64,
    pub message: String,
}

/// CDP 响应（也用于事件：事件带 `method` 且 `id` 为 null）
#[derive(Debug, Clone, Deserialize)]
pub struct CdpResponse {
    pub id: Option<u64>,
    pub result: Option<Value>,
    pub error: Option<CdpError>,
    #[serde(default)]
    pub method: Option<String>,
    #[serde(default)]
    pub params: Option<Value>,
}

impl CdpResponse {
    /// 把响应转为 Result：有 error 则 Err，否则返回 result（缺省 Null）
    pub fn into_result(self) -> std::result::Result<Value, CdpError> {
        if let Some(e) = self.error {
            Err(e)
        } else {
            Ok(self.result.unwrap_or(Value::Null))
        }
    }

    /// 是否为事件（非本会话请求响应）
    pub fn is_event(&self) -> bool {
        self.id.is_none() && self.method.is_some()
    }
}

// ── 常用命令构造器（id 由调用方分配，保证会话内唯一）──

pub fn page_enable(id: u64) -> CdpCommand {
    CdpCommand {
        id,
        method: "Page.enable".into(),
        params: None,
    }
}

pub fn runtime_enable(id: u64) -> CdpCommand {
    CdpCommand {
        id,
        method: "Runtime.enable".into(),
        params: None,
    }
}

pub fn page_navigate(id: u64, url: &str) -> CdpCommand {
    CdpCommand {
        id,
        method: "Page.navigate".into(),
        params: Some(json!({ "url": url })),
    }
}

pub fn page_capture_screenshot(id: u64, format: &str) -> CdpCommand {
    CdpCommand {
        id,
        method: "Page.captureScreenshot".into(),
        params: Some(json!({ "format": format, "captureBeyondViewport": false })),
    }
}

/// `Runtime.evaluate`：在页面上下文执行 JS。`await_promise` 让 await 的 Promise 被等待。
pub fn runtime_evaluate(id: u64, expression: &str, await_promise: bool) -> CdpCommand {
    CdpCommand {
        id,
        method: "Runtime.evaluate".into(),
        params: Some(json!({
            "expression": expression,
            "awaitPromise": await_promise,
            "returnByValue": true,
            "timeout": 30000
        })),
    }
}

/// `Target.createTarget`：新建标签页，返回 targetId
pub fn target_create_target(id: u64, url: &str) -> CdpCommand {
    CdpCommand {
        id,
        method: "Target.createTarget".into(),
        params: Some(json!({ "url": url })),
    }
}

/// `Target.attachToTarget`：拿到 target 的会话，返回 sessionId
pub fn target_attach_to_target(id: u64, target_id: &str, flatten: bool) -> CdpCommand {
    CdpCommand {
        id,
        method: "Target.attachToTarget".into(),
        params: Some(json!({ "targetId": target_id, "flatten": flatten })),
    }
}

// ── 无障碍（a11y）与输入（Bloom 非视觉/视觉双通道）──

/// `Accessibility.enable`：启用无障碍域（getFullAXTree 前必调，幂等）。
pub fn accessibility_enable(id: u64) -> CdpCommand {
    CdpCommand {
        id,
        method: "Accessibility.enable".into(),
        params: None,
    }
}

/// `Accessibility.getFullAXTree`：获取完整无障碍树（role/name/backendDOMNodeId/boundingBox）。
pub fn accessibility_get_full_ax_tree(id: u64) -> CdpCommand {
    CdpCommand {
        id,
        method: "Accessibility.getFullAXTree".into(),
        params: None,
    }
}

/// `DOM.enable`：启用 DOM 域（getBoxModel 前必调，幂等）。
pub fn dom_enable(id: u64) -> CdpCommand {
    CdpCommand {
        id,
        method: "DOM.enable".into(),
        params: None,
    }
}

/// `DOM.getBoxModel`：按 backendNodeId 取元素盒模型（content quad → 中心坐标）。
pub fn dom_get_box_model(id: u64, backend_node_id: i64) -> CdpCommand {
    CdpCommand {
        id,
        method: "DOM.getBoxModel".into(),
        params: Some(json!({ "backendNodeId": backend_node_id })),
    }
}

/// `Input.dispatchMouseEvent`：在视口 CSS 像素坐标触发真实鼠标事件（按下/抬起）。
pub fn input_dispatch_mouse_event(
    id: u64,
    mouse_type: &str,
    x: f64,
    y: f64,
    button: &str,
) -> CdpCommand {
    CdpCommand {
        id,
        method: "Input.dispatchMouseEvent".into(),
        params: Some(json!({
            "type": mouse_type,
            "x": x,
            "y": y,
            "button": button,
            "clickCount": 1
        })),
    }
}

// ── 响应解析助手 ──

/// 从 `Runtime.evaluate` 的响应里取出返回值（仅当 returnByValue 且 value 为字符串/数字）
pub fn extract_evaluate_value(response: &Value) -> Option<String> {
    response
        .get("result")
        .and_then(|r| r.get("value"))
        .map(|v| match v {
            Value::String(s) => s.clone(),
            other => other.to_string(),
        })
}

/// 从 `Page.captureScreenshot` 的响应里取出 base64 PNG 数据
pub fn extract_screenshot_base64(response: &Value) -> Option<String> {
    response
        .get("data")
        .and_then(|d| d.as_str())
        .map(|s| s.to_string())
}

/// 从 `Target.createTarget` 的响应里取出 targetId
pub fn extract_target_id(response: &Value) -> Option<String> {
    response
        .get("targetId")
        .and_then(|t| t.as_str())
        .map(|s| s.to_string())
}

/// 从 `/json/version` 端点 JSON 取出浏览器级 webSocketDebuggerUrl
pub fn extract_browser_ws(json_version: &Value) -> Option<String> {
    json_version
        .get("webSocketDebuggerUrl")
        .and_then(|u| u.as_str())
        .map(|s| s.to_string())
}

/// 无障碍树节点精简快照（非视觉通道的定位单元）
#[derive(Debug, Clone, PartialEq)]
pub struct AxNodeBrief {
    /// 可点击/聚焦的角色（button/link/textbox/checkbox/...）
    pub role: String,
    /// 节点名（name 属性，如按钮文案）
    pub name: String,
    /// 后端 DOM 节点 id（DOM.getBoxModel 的定位键）
    pub backend_node_id: i64,
    /// 视口相对包围盒（CSS 像素）：x, y, width, height
    pub bbox: [f64; 4],
}

/// 是否为可交互角色（Bloom 非视觉通道只关心能点的节点）
pub fn is_interactive_role(role: &str) -> bool {
    matches!(
        role,
        "button"
            | "link"
            | "textbox"
            | "searchbox"
            | "checkbox"
            | "combobox"
            | "listbox"
            | "option"
            | "menuitem"
            | "tab"
            | "radio"
            | "switch"
            | "slider"
            | "treeitem"
            | "contentinfo"
            | "row"
            | "cell"
            | "gridcell"
    )
}

/// 从 `Accessibility.getFullAXTree` 的 result 提取可交互节点快照。
pub fn extract_ax_nodes(result: &Value) -> Vec<AxNodeBrief> {
    let mut out = Vec::new();
    let nodes = result.get("nodes").and_then(|n| n.as_array());
    let Some(nodes) = nodes else {
        return out;
    };
    for node in nodes {
        let role = node
            .get("role")
            .and_then(|r| r.get("value"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        if role.is_empty() || !is_interactive_role(&role) {
            continue;
        }
        let name = node
            .get("name")
            .and_then(|n| n.get("value"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        // 忽略无操作语义的装饰节点（空名且无点击角色之外的细分）
        if name.is_empty()
            && !matches!(
                role.as_str(),
                "image" | "textbox" | "searchbox" | "combobox"
            )
        {
            continue;
        }
        let backend_node_id = node
            .get("backendDOMNodeId")
            .and_then(|v| v.as_i64())
            .unwrap_or(-1);
        if backend_node_id < 0 {
            continue;
        }
        let bb = node.get("boundingBox");
        let bbox = if let Some(bb) = bb {
            let x = bb.get(0).and_then(|v| v.as_f64()).unwrap_or(0.0);
            let y = bb.get(1).and_then(|v| v.as_f64()).unwrap_or(0.0);
            let w = bb.get(2).and_then(|v| v.as_f64()).unwrap_or(0.0);
            let h = bb.get(3).and_then(|v| v.as_f64()).unwrap_or(0.0);
            [x, y, w, h]
        } else {
            [0.0; 4]
        };
        out.push(AxNodeBrief {
            role,
            name,
            backend_node_id,
            bbox,
        });
    }
    out
}

/// 从 `DOM.getBoxModel` 的 result 提取 content 盒中心坐标（viewport CSS 像素）。
///
/// CDP 的 `content` 是扁平 8 元数组：`[x1,y1, x2,y2, x3,y3, x4,y4]`（左上→右上→右下→左下）。
pub fn extract_box_model_center(result: &Value) -> Option<(f64, f64)> {
    let content = result.get("model")?.get("content")?.as_array()?;
    if content.len() < 8 {
        return None;
    }
    let mut xs = [0.0f64; 4];
    let mut ys = [0.0f64; 4];
    for i in 0..4 {
        xs[i] = content.get(i * 2)?.as_f64()?;
        ys[i] = content.get(i * 2 + 1)?.as_f64()?;
    }
    Some((xs.iter().sum::<f64>() / 4.0, ys.iter().sum::<f64>() / 4.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_serializes_with_id_method_params() {
        let cmd = page_navigate(7, "https://example.com");
        let s = serde_json::to_string(&cmd).unwrap();
        assert!(s.contains("\"id\":7"));
        assert!(s.contains("\"method\":\"Page.navigate\""));
        assert!(s.contains("https://example.com"));
    }

    #[test]
    fn runtime_evaluate_requests_return_by_value() {
        let cmd = runtime_evaluate(3, "1+1", true);
        let v: Value = serde_json::from_str(&serde_json::to_string(&cmd).unwrap()).unwrap();
        assert_eq!(v["method"], "Runtime.evaluate");
        assert_eq!(v["params"]["returnByValue"], true);
        assert_eq!(v["params"]["awaitPromise"], true);
        assert_eq!(v["params"]["expression"], "1+1");
    }

    #[test]
    fn response_with_error_returns_err() {
        let raw = r#"{"id":1,"error":{"code":-32000,"message":"boom"}}"#;
        let r: CdpResponse = serde_json::from_str(raw).unwrap();
        assert!(!r.is_event());
        let e = r.into_result().unwrap_err();
        assert_eq!(e.code, -32000);
        assert_eq!(e.message, "boom");
    }

    #[test]
    fn response_event_detected() {
        let raw = r#"{"method":"Page.loadEventFired","params":{}}"#;
        let r: CdpResponse = serde_json::from_str(raw).unwrap();
        assert!(r.is_event());
    }

    #[test]
    fn extract_evaluate_string_value() {
        // 注意：send_command 已用 into_result() 拆出 CDP 响应的 result 字段，
        // 此处传入的是「拆包后」的 result（即生产里 evaluate() 实际拿到的 Value）。
        let raw = r#"{"result":{"type":"string","value":"hello"},"exceptionDetails":null}"#;
        let v: Value = serde_json::from_str(raw).unwrap();
        assert_eq!(extract_evaluate_value(&v).as_deref(), Some("hello"));
    }

    #[test]
    fn extract_screenshot_and_target() {
        // 同理：传入拆包后的 result（生产里 screenshot_png()/open() 实际拿到的）。
        let shot: Value = serde_json::from_str(r#"{"data":"BASE64DATA"}"#).unwrap();
        assert_eq!(
            extract_screenshot_base64(&shot).as_deref(),
            Some("BASE64DATA")
        );

        let tgt: Value = serde_json::from_str(r#"{"targetId":"ABC"}"#).unwrap();
        assert_eq!(extract_target_id(&tgt).as_deref(), Some("ABC"));

        let ver: Value = serde_json::from_str(
            r#"{"webSocketDebuggerUrl":"ws://127.0.0.1:9222/devtools/browser/x"}"#,
        )
        .unwrap();
        assert_eq!(
            extract_browser_ws(&ver).as_deref(),
            Some("ws://127.0.0.1:9222/devtools/browser/x")
        );
    }

    #[test]
    fn a11y_constructors_serialize_methods() {
        let cmd = accessibility_get_full_ax_tree(9);
        let v: Value = serde_json::from_str(&serde_json::to_string(&cmd).unwrap()).unwrap();
        assert_eq!(v["method"], "Accessibility.getFullAXTree");

        let dom = dom_get_box_model(10, 42);
        let v: Value = serde_json::from_str(&serde_json::to_string(&dom).unwrap()).unwrap();
        assert_eq!(v["method"], "DOM.getBoxModel");
        assert_eq!(v["params"]["backendNodeId"], 42);

        let input = input_dispatch_mouse_event(11, "mousePressed", 12.5, 34.5, "left");
        let v: Value = serde_json::from_str(&serde_json::to_string(&input).unwrap()).unwrap();
        assert_eq!(v["method"], "Input.dispatchMouseEvent");
        assert_eq!(v["params"]["type"], "mousePressed");
        assert_eq!(v["params"]["x"], 12.5);
        assert_eq!(v["params"]["y"], 34.5);
    }

    #[test]
    fn extract_ax_nodes_filters_interactive() {
        let result: Value = serde_json::from_str(
            r#"{"nodes":[
                {"role":{"value":"button"},"name":{"value":"搜索"},"backendDOMNodeId":11,"boundingBox":[1,2,80,32]},
                {"role":{"value":"generic"},"name":{"value":"容器"},"backendDOMNodeId":12,"boundingBox":[0,0,10,10]},
                {"role":{"value":"textbox"},"name":{"value":""},"backendDOMNodeId":13,"boundingBox":[5,6,200,30]},
                {"role":{"value":"staticText"},"name":{"value":"说明文字"},"backendDOMNodeId":14,"boundingBox":[0,0,1,1]}
            ]}"#,
        )
        .unwrap();
        let nodes = extract_ax_nodes(&result);
        assert_eq!(nodes.len(), 2, "只保留可交互节点");
        assert_eq!(nodes[0].role, "button");
        assert_eq!(nodes[0].name, "搜索");
        assert_eq!(nodes[0].backend_node_id, 11);
        assert_eq!(nodes[0].bbox, [1.0, 2.0, 80.0, 32.0]);
        assert_eq!(nodes[1].role, "textbox");
    }

    #[test]
    fn extract_box_center_averages_quad() {
        // CDP content 为扁平 8 元数组：[x1,y1, x2,y2, x3,y3, x4,y4]
        let result: Value =
            serde_json::from_str(r#"{"model":{"content":[0,0,0,100,100,100,100,0]}}"#).unwrap();
        assert_eq!(extract_box_model_center(&result), Some((50.0, 50.0)));
        let missing: Value = serde_json::from_str(r#"{"model":{}}"#).unwrap();
        assert_eq!(extract_box_model_center(&missing), None);
    }
}
