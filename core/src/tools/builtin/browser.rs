//! Auto-extracted from builtin.rs — see parent module for shared infrastructure.
//!
//! Browser automation

use super::*;

pub(crate) fn browser_sessions() -> &'static tokio::sync::Mutex<HashMap<String, BrowserEntry>> {
    use std::sync::OnceLock;
    static SESSIONS: OnceLock<tokio::sync::Mutex<HashMap<String, BrowserEntry>>> = OnceLock::new();
    SESSIONS.get_or_init(|| tokio::sync::Mutex::new(HashMap::new()))
}

/// 回收空闲超时会话并裁剪到上限（保留最近活跃者）
pub(crate) fn purge_stale(sessions: &mut HashMap<String, BrowserEntry>) {
    let now = std::time::Instant::now();
    sessions.retain(|_, e| now.duration_since(e.touched) < BROWSER_SESSION_TTL);
    while sessions.len() > BROWSER_SESSION_MAX {
        let oldest = sessions
            .iter()
            .min_by_key(|(_, e)| e.touched)
            .map(|(k, _)| k.clone());
        if let Some(k) = oldest {
            sessions.remove(&k);
        } else {
            break;
        }
    }
}

/// 打开一个新页面会话（导航到 url 并启用 Page/Runtime 域）
pub(crate) async fn open_page(bridge: &BrowserBridge, url: &str) -> Result<PageSession> {
    let mut page = bridge.open(url).await?;
    tokio::time::sleep(Duration::from_millis(1200)).await;
    let _ = &mut page;
    Ok(page)
}

/// 打开页面会话：relay 连接用户真实 Chrome（复用登录态），其余走 headless 启动
pub(crate) async fn open_page_for(
    bridge: &BrowserBridge,
    action: &str,
    port: u16,
    url: &str,
) -> Result<PageSession> {
    if action == "relay" {
        let target = if url.trim().is_empty() {
            "about:blank"
        } else {
            url
        };
        let page = bridge.relay(port, target).await?;
        tokio::time::sleep(Duration::from_millis(1200)).await;
        Ok(page)
    } else {
        open_page(bridge, url).await
    }
}

/// 把字符串安全嵌入 JS（JSON 字符串字面量）
pub(crate) fn js_string(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_else(|_| "\"\"".to_string())
}

pub(crate) fn browser_tool(
    workspace: impl Into<RootSet>,
    vision_cap: Option<Arc<VisionCapability>>,
) -> Box<dyn ToolExecutor> {
    let workspace: RootSet = workspace.into();
    boxed(
        "browser",
        "浏览器自动化（真实 Chromium，CDP）：Agent 可控有状态会话。双通道操控——视觉= screenshot（截图观察）/ vision（截图→视觉模型描述）/ vision_click（截图→VLM 定位坐标→真实点击）；非视觉= open/text/links/html（DOM 读取）+ a11y（无障碍语义快照：role/name/ref）+ click_ref（按 ref 语义点击）/ evaluate/click/type/scroll（DOM 操控）。relay=连接用户真实 Chrome（--remote-debugging-port，复用登录态/cookies）。参数: action(必填), url, js, selector, text, ref(a11y ref), hint(视觉定位目标描述), browser_id(复用会话), out(截图路径), max_chars, max(链接数), wait_ms, px, port(relay 调试端口,默认9222)。不传 browser_id 时每次新建会话。",
        json!({"type":"object","properties":{
            "action":{"type":"string","enum":["open","text","links","html","evaluate","click","type","screenshot","a11y","click_ref","vision","vision_click","wait","scroll","close","relay"]},
            "url":{"type":"string"},
            "js":{"type":"string"},
            "selector":{"type":"string"},
            "text":{"type":"string"},
            "ref":{"type":"number","description":"a11y 快照中的节点 ref（backendDOMNodeId），click_ref 必填"},
            "hint":{"type":"string","description":"vision_click 的目标元素描述（如「右上角搜索按钮」）"},
            "browser_id":{"type":"string","description":"复用已有会话；不传则每次新建"},
            "out":{"type":"string","description":"截图输出相对路径（screenshot 必填）"},
            "max_chars":{"type":"number"},
            "max":{"type":"number","description":"links 最多收集条数"},
            "wait_ms":{"type":"number","description":"wait 动作等待毫秒数"},
            "px":{"type":"number","description":"scroll 动作滚动像素"},
            "port":{"type":"number","description":"relay 动作连接真实 Chrome 的远程调试端口，默认 9222"}
        },"required":["action"]}),
        Arc::new(move |args| {
            let workspace = workspace.clone();
            let vision_cap = vision_cap.clone();
            Box::pin(async move {
                let action = args.get("action").cloned().unwrap_or_default();
                if action.trim().is_empty() {
                    return err("browser 需要非空 action".into(), "INVALID_PARAMS");
                }
                let browser_id = args
                    .get("browser_id")
                    .cloned()
                    .filter(|s| !s.trim().is_empty());
                let url = args.get("url").cloned().unwrap_or_default();
                let port: u16 = args.get("port").and_then(|p| p.parse().ok()).unwrap_or(9222);

                // wait / close 不依赖 Chromium 会话
                if action == "wait" {
                    let ms: u64 = args.get("wait_ms").and_then(|v| v.parse().ok()).unwrap_or(1000);
                    tokio::time::sleep(Duration::from_millis(ms)).await;
                    return ok(format!("已等待 {ms}ms"));
                }
                if action == "close" {
                    if let Some(bid) = &browser_id {
                        browser_sessions().lock().await.remove(bid);
                    }
                    return ok("浏览器会话已关闭".into());
                }

                // 用户接管闸门（browser.control.set owner=user）：
                // 「接管」必须是互斥的，否则用户点到一半就被智能体下一步带走。
                // close / wait 不受限（收尾与等待不改变页面状态）。
                if let Some(reason) = crate::browser::control::agent_block_reason() {
                    return err(reason, "USER_IN_CONTROL");
                }

                // 需要页面的动作：一次性会话必须有 url（复用会话可不传）
                let needs_page = matches!(
                    action.as_str(),
                    "open" | "text" | "links" | "html" | "evaluate" | "click" | "type"
                        | "screenshot" | "a11y" | "click_ref" | "vision" | "vision_click"
                        | "scroll"
                );
                if needs_page && url.trim().is_empty() && browser_id.is_none() {
                    return err("browser 需要非空 url（复用会话时可只传 browser_id）".into(), "INVALID_PARAMS");
                }

                let bridge = BrowserBridge::new();
                if !bridge.is_available() {
                    return err(
                        "未检测到 Chromium/Chrome 可执行文件（browser_bridge 不可用）".into(),
                        "NOT_AVAILABLE",
                    );
                }

                let mut guard = browser_sessions().lock().await;
                purge_stale(&mut guard);

                // 复用既有会话；否则新建（有 browser_id 则登记，无则一次性）
                if let Some(bid) = &browser_id {
                    if let Some(entry) = guard.get_mut(bid) {
                        entry.touched = std::time::Instant::now();
                        return run_browser_action(
                            &mut entry.page,
                            args,
                            &action,
                            &url,
                            false,
                            &workspace,
                            &vision_cap,
                        )
                        .await;
                    }
                    // 新建会话并登记到注册表（Agent 后续可跨调用持续操控）
                    let mut page = match open_page_for(&bridge, &action, port, &url).await {
                        Ok(p) => p,
                        Err(e) => return err(format!("打开页面失败: {e}"), "BROWSER_ERROR"),
                    };
                    let result = run_browser_action(&mut page, args, &action, &url, true, &workspace, &vision_cap).await;
                    guard.insert(bid.clone(), BrowserEntry {
                        page,
                        touched: std::time::Instant::now(),
                    });
                    result
                } else {
                    // 一次性会话：用完即弃（保持旧语义）
                    let mut page = match open_page_for(&bridge, &action, port, &url).await {
                        Ok(p) => p,
                        Err(e) => return err(format!("打开页面失败: {e}"), "BROWSER_ERROR"),
                    };
                    run_browser_action(&mut page, args, &action, &url, true, &workspace, &vision_cap).await
                }
            })
        }),
    )
}

// ── 系统级鼠标键盘控制（第三方软硬件/系统级输入，Windows user32）──
//
// Marguerite 融合路由中 Bloom 模式的系统控制通道：真实移动系统光标、点击/滚轮、
// 发送键盘按键与文本，供 Agent 操控任意桌面应用（不限于内置浏览器）。
// 实现走 PowerShell + user32 P/Invoke（SendInput 语义），零新增 Rust 依赖。
// 该能力在模式门控中属 Desktop 类（Bloom 默认人工确认，skip_permissions 全开自动）。

