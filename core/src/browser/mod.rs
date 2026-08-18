//! 浏览器桥（browser_bridge）：Chrome DevTools Protocol 重型原生桥
//!
//! 通过本地 `ws://` 连接 Chrome/Chromium 的 DevTools 端点，实现页面导航、JS 求值、
//! 截图等能力——对标 `tool.browser` 的「浏览器自动化」。这是与 标准 AI 助手 的差异化
//! 重型原生桥之一（不依赖任何 WebView/浏览器渲染层，而是直接驱动真实 Chromium）。
//!
//! 设计：
//! - 启动 Chromium（`--remote-debugging-port=0 --headless`），从 stderr 解析 DevTools WS 地址；
//! - 连上浏览器级会话 → 建标签页（Target.createTarget）→ 附加（flatten）→ 在目标会话里
//!   发 `Page`/`Runtime` 命令；
//! - 命令/响应建模见 [`cdp`] 模块（离线可测）。
//!
//! 实时集成测试为 `#[ignore]`，需本机安装 Chromium 且联网（手动 `cargo test -- --ignored`）。

pub mod cdp;
pub mod gateway;

use std::path::PathBuf;
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use futures_util::{SinkExt, StreamExt};
use serde_json::Value;
use tokio::io::AsyncBufReadExt;
use tokio::net::TcpStream;
use tokio::process::{Child, Command as TokioCommand};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};
use tracing::warn;

pub use cdp::{
    accessibility_enable, accessibility_get_full_ax_tree, dom_enable, dom_get_box_model,
    extract_ax_nodes, extract_box_model_center, extract_browser_ws, extract_evaluate_value,
    extract_screenshot_base64, extract_target_id, input_dispatch_mouse_event,
    page_capture_screenshot, page_enable, page_navigate, runtime_enable, runtime_evaluate,
    target_attach_to_target, target_create_target, AxNodeBrief, CdpCommand, CdpError, CdpResponse,
};

/// 浏览器调试端点（含子进程句柄，drop 时杀掉 Chromium）
pub struct DevToolsEndpoint {
    pub web_socket_url: String,
    child: Child,
}

impl Drop for DevToolsEndpoint {
    fn drop(&mut self) {
        let _ = self.child.start_kill();
    }
}

/// 一个 CDP 会话（连接到浏览器或某个 target）。
pub struct CdpSession {
    write: futures_util::stream::SplitSink<WebSocketStream<MaybeTlsStream<TcpStream>>, Message>,
    read: futures_util::stream::SplitStream<WebSocketStream<MaybeTlsStream<TcpStream>>>,
    next_id: AtomicU64,
    /// flatten 目标会话的 sessionId（浏览器级会话为 None）
    session_id: Option<String>,
}

impl CdpSession {
    /// 连接到一个 WS 端点，返回会话。
    pub async fn connect(web_socket_url: &str) -> Result<Self> {
        let (ws, _resp) = tokio_tungstenite::connect_async(web_socket_url)
            .await
            .with_context(|| format!("连接 CDP 端点失败: {}", web_socket_url))?;
        let (write, read) = ws.split();
        Ok(Self {
            write,
            read,
            next_id: AtomicU64::new(1),
            session_id: None,
        })
    }

    fn alloc_id(&self) -> u64 {
        self.next_id.fetch_add(1, Ordering::SeqCst)
    }

    /// 发送命令并等待匹配 id 的响应（跳过事件帧）。
    pub async fn send_command(&mut self, mut cmd: CdpCommand) -> Result<Value> {
        if cmd.id == 0 {
            cmd.id = self.alloc_id();
        }
        // flatten 会话需要带上 sessionId
        let mut wire = serde_json::to_value(&cmd)?;
        if let Some(sid) = &self.session_id {
            wire["sessionId"] = Value::String(sid.clone());
        }
        let text = serde_json::to_string(&wire)?;
        self.write
            .send(Message::Text(text))
            .await
            .context("发送 CDP 命令失败")?;

        let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
        loop {
            if tokio::time::Instant::now() >= deadline {
                return Err(anyhow!("等待 CDP 响应超时（id={}）", cmd.id));
            }
            let frame = match tokio::time::timeout(Duration::from_secs(30), self.read.next()).await
            {
                Ok(Some(Ok(msg))) => msg,
                Ok(Some(Err(e))) => return Err(anyhow!("接收 CDP 帧失败: {}", e)),
                Ok(None) => return Err(anyhow!("CDP 连接已关闭")),
                Err(_) => return Err(anyhow!("接收 CDP 帧超时")),
            };
            match frame {
                Message::Text(t) => {
                    let resp: CdpResponse =
                        serde_json::from_str(&t).with_context(|| "解析 CDP 响应失败")?;
                    if resp.is_event() {
                        continue; // 事件帧，跳过
                    }
                    if resp.id == Some(cmd.id) {
                        return resp
                            .into_result()
                            .map_err(|e| anyhow!("CDP 错误（{}）: {}", e.code, e.message));
                    }
                    // 其它请求的响应：继续等
                }
                Message::Ping(_) | Message::Pong(_) => continue,
                Message::Close(_) => return Err(anyhow!("CDP 连接被对端关闭")),
                _ => continue,
            }
        }
    }

    /// 启用 Page 与 Runtime 域（导航/求值前必调）。
    pub async fn enable_domains(&mut self) -> Result<()> {
        self.send_command(page_enable(self.alloc_id())).await?;
        self.send_command(runtime_enable(self.alloc_id())).await?;
        Ok(())
    }

    /// 导航到 URL（返回由 CDP 返回的 frameId，可能为空）。
    pub async fn navigate(&mut self, url: &str) -> Result<()> {
        self.send_command(page_navigate(self.alloc_id(), url))
            .await?;
        Ok(())
    }

    /// 在页面上下文执行 JS 并返回字符串结果。
    pub async fn evaluate(&mut self, expression: &str, await_promise: bool) -> Result<String> {
        let resp = self
            .send_command(runtime_evaluate(self.alloc_id(), expression, await_promise))
            .await?;
        extract_evaluate_value(&resp).ok_or_else(|| anyhow!("无法从 Runtime.evaluate 提取返回值"))
    }

    /// 单次截图并解码为 PNG 字节；内部任何失败一律返回 `None`（交由上层判定是否重试）
    async fn capture_once(&mut self) -> Option<Vec<u8>> {
        let resp = self
            .send_command(page_capture_screenshot(self.alloc_id(), "png"))
            .await
            .ok()?;
        let b64 = extract_screenshot_base64(&resp)?;
        use base64::Engine;
        base64::engine::general_purpose::STANDARD
            .decode(b64.trim())
            .ok()
    }

    /// 截图，返回 PNG 字节（base64 解码）。
    /// 若首帧可疑（空白/未 ready），等待 300ms 后重试一次；仍可疑则返回该结果并告警，不报错。
    pub async fn screenshot_png(&mut self) -> Result<Vec<u8>> {
        if let Some(bytes) = self.capture_once().await {
            if !is_suspicious_screenshot(&bytes) {
                return Ok(bytes);
            }
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
        match self.capture_once().await {
            Some(bytes) => {
                if is_suspicious_screenshot(&bytes) {
                    warn!(
                        bytes = bytes.len(),
                        "截图疑似空白帧，二次重试后仍可疑（返回当前结果）"
                    );
                }
                Ok(bytes)
            }
            None => Err(anyhow!("无法从 Page.captureScreenshot 提取数据")),
        }
    }

    /// 启用 Accessibility 与 DOM 域（a11y 快照与坐标定位前必调，幂等）。
    pub async fn enable_a11y_dom(&mut self) -> Result<()> {
        self.send_command(accessibility_enable(self.alloc_id()))
            .await?;
        self.send_command(dom_enable(self.alloc_id())).await?;
        Ok(())
    }

    /// 获取页面可交互无障碍树快照（非视觉通道的核心语义读取）。
    pub async fn a11y_nodes(&mut self) -> Result<Vec<AxNodeBrief>> {
        self.enable_a11y_dom().await?;
        let resp = self
            .send_command(accessibility_get_full_ax_tree(self.alloc_id()))
            .await?;
        let nodes = extract_ax_nodes(&resp);
        if nodes.is_empty() {
            return Err(anyhow!("页面无可交互无障碍节点（页面可能未加载完成）"));
        }
        Ok(nodes)
    }

    /// 按 backendDOMNodeId 取元素盒模型中心坐标（viewport CSS 像素）。
    pub async fn box_center(&mut self, backend_node_id: i64) -> Result<(f64, f64)> {
        self.enable_a11y_dom().await?;
        let resp = self
            .send_command(dom_get_box_model(self.alloc_id(), backend_node_id))
            .await?;
        extract_box_model_center(&resp)
            .ok_or_else(|| anyhow!("DOM.getBoxModel 未返回有效盒模型（元素可能隐藏或已移除）"))
    }

    /// 在视口 CSS 像素坐标执行一次真实鼠标左键点击（按下+抬起）。
    pub async fn click_at(&mut self, x: f64, y: f64) -> Result<()> {
        self.send_command(input_dispatch_mouse_event(
            self.alloc_id(),
            "mousePressed",
            x,
            y,
            "left",
        ))
        .await?;
        self.send_command(input_dispatch_mouse_event(
            self.alloc_id(),
            "mouseReleased",
            x,
            y,
            "left",
        ))
        .await?;
        Ok(())
    }

    /// 当前视口 CSS 尺寸（innerWidth × innerHeight，用于截图像素 → CSS 坐标映射）。
    pub async fn viewport_size(&mut self) -> Result<(f64, f64)> {
        let w = self
            .evaluate("window.innerWidth", false)
            .await
            .unwrap_or_default()
            .parse::<f64>()
            .unwrap_or(0.0);
        let h = self
            .evaluate("window.innerHeight", false)
            .await
            .unwrap_or_default()
            .parse::<f64>()
            .unwrap_or(0.0);
        if w <= 0.0 || h <= 0.0 {
            return Err(anyhow!(
                "无法读取视口尺寸（innerWidth={w}, innerHeight={h}）"
            ));
        }
        Ok((w, h))
    }
}

/// 截图解码后判为可疑（空白/损坏帧）的最小字节数。
/// 真实页面截图通常为数 KB；低于此阈值视为首帧空白或页面未 ready。
const MIN_SCREENSHOT_BYTES: usize = 1024;

/// 判定截图解码结果是否可疑：空切片或字节数过小。
/// 用于在捕获到空白首帧时触发一次重试，避免把空白 PNG 交给模型做视觉判断。
fn is_suspicious_screenshot(decoded: &[u8]) -> bool {
    decoded.len() < MIN_SCREENSHOT_BYTES
}

/// 浏览器桥：负责发现/启动 Chromium 并打开页面会话。
pub struct BrowserBridge {
    chrome: Option<PathBuf>,
}

impl Default for BrowserBridge {
    fn default() -> Self {
        Self::new()
    }
}

impl BrowserBridge {
    pub fn new() -> Self {
        Self {
            chrome: find_chrome(),
        }
    }

    /// 是否检测到可用的 Chromium 可执行文件。
    pub fn is_available(&self) -> bool {
        self.chrome.is_some()
    }

    /// 启动 Chromium（headless）并返回调试端点。
    pub async fn launch(&self) -> Result<DevToolsEndpoint> {
        let chrome = self
            .chrome
            .clone()
            .ok_or_else(|| anyhow!("未找到 Chromium/Chrome 可执行文件；browser_bridge 不可用"))?;

        let mut child = TokioCommand::new(&chrome)
            .arg("--headless=new")
            .arg("--disable-gpu")
            .arg("--no-sandbox")
            .arg("--remote-debugging-port=0")
            .arg("about:blank")
            .stderr(Stdio::piped())
            .spawn()
            .with_context(|| format!("启动 Chromium 失败: {}", chrome.display()))?;

        // 从 stderr 解析 "DevTools listening on ws://127.0.0.1:<port>/devtools/browser/<id>"
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| anyhow!("无法捕获 Chromium stderr"))?;
        let mut reader = tokio::io::BufReader::new(stderr).lines();

        let ws_url = {
            let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
            let mut found = None;
            loop {
                if tokio::time::Instant::now() >= deadline {
                    break;
                }
                let line =
                    match tokio::time::timeout(Duration::from_secs(15), reader.next_line()).await {
                        // timeout 返回 Result<Result<Option<String>, io::Error>, Elapsed>
                        Ok(Ok(Some(l))) => l,
                        _ => break,
                    };
                if let Some(cap) = line
                    .find("DevTools listening on ")
                    .map(|i| &line[i + "DevTools listening on ".len()..])
                {
                    found = Some(cap.trim().to_string());
                    break;
                }
            }
            found
        };

        match ws_url {
            Some(u) => Ok(DevToolsEndpoint {
                web_socket_url: u,
                child,
            }),
            None => {
                let _ = child.start_kill();
                Err(anyhow!("未能从 Chromium stderr 解析 DevTools WS 地址"))
            }
        }
    }

    /// 启动并打开一个指向 `url` 的页面会话（内部：建 target → attach flatten → enable）。
    pub async fn open(&self, url: &str) -> Result<PageSession> {
        let ep = self.launch().await?;
        let mut browser = CdpSession::connect(&ep.web_socket_url).await?;

        // 建标签页
        let created = browser
            .send_command(target_create_target(browser.alloc_id(), url))
            .await?;
        let target_id = extract_target_id(&created)
            .ok_or_else(|| anyhow!("Target.createTarget 未返回 targetId"))?;

        // 附加（flatten=true 让目标事件/响应经浏览器连接回流，命令需带 sessionId）
        let attached = browser
            .send_command(target_attach_to_target(
                browser.alloc_id(),
                &target_id,
                true,
            ))
            .await?;
        let session_id = attached
            .get("sessionId")
            .and_then(|s| s.as_str())
            .map(|s| s.to_string())
            .ok_or_else(|| anyhow!("Target.attachToTarget 未返回 sessionId"))?;

        // 复用浏览器连接作为目标会话（带 sessionId）
        let mut page = CdpSession::from_existing(browser, session_id);
        page.enable_domains().await?;
        Ok(PageSession {
            endpoint: Some(ep),
            session: page,
        })
    }

    /// 浏览器 Relay：连接**用户真实 Chrome**（浏览器 Relay 范式）
    ///
    /// 通过 `--remote-debugging-port` 暴露的 HTTP 端点发现浏览器级 CDP WS 地址
    /// （`GET /json/version`），附加既有浏览器会话并新建标签页——复用用户登录态/
    /// cookies/localStorage，作为 headless 通道的补充。不持有子进程句柄（不 kill 用户浏览器）。
    pub async fn relay(&self, port: u16, url: &str) -> Result<PageSession> {
        let browser_ws = discover_browser_ws(port).await?;
        let mut browser = CdpSession::connect(&browser_ws).await?;

        let created = browser
            .send_command(target_create_target(browser.alloc_id(), url))
            .await?;
        let target_id = extract_target_id(&created)
            .ok_or_else(|| anyhow!("Target.createTarget 未返回 targetId"))?;
        let attached = browser
            .send_command(target_attach_to_target(
                browser.alloc_id(),
                &target_id,
                true,
            ))
            .await?;
        let session_id = attached
            .get("sessionId")
            .and_then(|s| s.as_str())
            .map(|s| s.to_string())
            .ok_or_else(|| anyhow!("Target.attachToTarget 未返回 sessionId"))?;

        let mut page = CdpSession::from_existing(browser, session_id);
        page.enable_domains().await?;
        Ok(PageSession {
            endpoint: None,
            session: page,
        })
    }
}

// ── 辅助：发现用户真实 Chrome 的 CDP 调试端点 ──

/// 从 `GET /json/version` 响应 JSON 中提取浏览器级 `webSocketDebuggerUrl`（纯函数，离线可测）
pub fn extract_browser_ws_from_version(body: &str) -> Option<String> {
    let v: Value = serde_json::from_str(body).ok()?;
    v.get("webSocketDebuggerUrl")?
        .as_str()
        .map(|s| s.to_string())
        .filter(|s| s.starts_with("ws://"))
}

/// 通过 `--remote-debugging-port` HTTP 端点发现浏览器级 CDP WS 地址
pub async fn discover_browser_ws(port: u16) -> Result<String> {
    let url = format!("http://127.0.0.1:{port}/json/version");
    let body = reqwest::get(&url)
        .await
        .with_context(|| format!("连接真实 Chrome 调试端点失败: {url}"))?
        .error_for_status()
        .with_context(|| format!("真实 Chrome 调试端点返回非 2xx: {url}"))?
        .text()
        .await
        .context("读取 /json/version 响应失败")?;
    extract_browser_ws_from_version(&body)
        .ok_or_else(|| anyhow!("{url} 响应缺少 webSocketDebuggerUrl: {body}"))
}

/// 已打开的页面会话（含可选端点句柄；relay 连接真实 Chrome 时无子进程句柄）
pub struct PageSession {
    #[allow(dead_code)]
    endpoint: Option<DevToolsEndpoint>,
    pub session: CdpSession,
}

// ── 辅助：发现 Chromium 可执行文件 ──

/// 在 PATH 中探测常见 Chromium/Chrome 可执行名；并补充主流安装路径（Windows/macOS/Linux）。
pub fn find_chrome() -> Option<PathBuf> {
    // 1) PATH 命令名探测
    let names = [
        "google-chrome-stable",
        "google-chrome",
        "chromium",
        "chromium-browser",
        "chrome",
        "chrome.exe",
        "chromium.exe",
        "msedge",
        "msedge.exe",
    ];
    for name in names {
        if let Ok(out) = std::process::Command::new(name).arg("--version").output() {
            if out.status.success() {
                return Some(PathBuf::from(name));
            }
        }
    }
    // 2) 标准安装路径探测（绕过 PATH 缺失）
    candidate_chrome_paths().into_iter().find(|p| p.exists())
}

/// 标准安装路径候选（按当前 OS 返回应探测的绝对路径；供检测与「未找到」引导共用）。
pub fn candidate_chrome_paths() -> Vec<PathBuf> {
    let home = std::env::var_os("LOCALAPPDATA").map(std::path::PathBuf::from);
    let mut paths = vec![
        PathBuf::from(r"C:\Program Files\Google\Chrome\Application\chrome.exe"),
        PathBuf::from(r"C:\Program Files (x86)\Google\Chrome\Application\chrome.exe"),
        PathBuf::from(r"C:\Program Files\Microsoft\Edge\Application\msedge.exe"),
        PathBuf::from(r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe"),
        PathBuf::from("/usr/bin/google-chrome"),
        PathBuf::from("/usr/bin/chromium"),
        PathBuf::from("/usr/bin/chromium-browser"),
        PathBuf::from("/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"),
        PathBuf::from("/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge"),
    ];
    if let Some(h) = home {
        paths.push(h.join(r"Google\Chrome\Application\chrome.exe"));
        paths.push(h.join(r"Microsoft\Edge\Application\msedge.exe"));
    }
    paths
}

/// 按当前操作系统返回 Chromium/Chrome 安装引导（纯文本，多行）。
pub fn browser_install_guidance() -> String {
    match std::env::consts::OS {
        "windows" => [
            "Windows 安装方式（任选其一）:",
            "  · winget install Google.Chrome        # 官方 Chrome（推荐）",
            "  · winget install Microsoft.Edge       # 系统常已预装 Edge，可直接用",
            "  · choco install googlechrome          # 若已装 Chocolatey",
            "  · 或手动下载: https://www.google.com/chrome/",
        ]
        .join("\n"),
        "macos" => [
            "macOS 安装方式（任选其一）:",
            "  · brew install --cask google-chrome   # 需 Homebrew",
            "  · 或手动下载: https://www.google.com/chrome/",
        ]
        .join("\n"),
        _ => [
            "Linux 安装方式（按发行版任选其一）:",
            "  · Debian/Ubuntu: sudo apt install chromium",
            "  · Fedora:        sudo dnf install chromium",
            "  · Arch:          sudo pacman -S chromium",
            "  · 或安装 chromium-browser / google-chrome-stable（见官网）",
        ]
        .join("\n"),
    }
}

/// 浏览器不可用时给用户的完整引导文案：已探测路径 + 安装指引 + 离线降级说明。
pub fn browser_unavailable_message() -> String {
    let candidates = candidate_chrome_paths();
    let mut msg = String::from("未检测到 Chromium/Chrome 可执行文件（browser_bridge 不可用）。\n");
    msg.push_str("\n已探测以下常见安装路径均未命中:\n");
    for p in &candidates {
        msg.push_str(&format!("  · {}\n", p.display()));
    }
    msg.push('\n');
    msg.push_str(&browser_install_guidance());
    msg.push_str("\n\n离线降级: 浏览器桥（截图 / DOM 坐标 / CDP 操控）当前不可用；");
    msg.push_str("其余功能（CLI 子命令、TUI、本地记忆、嵌入检索）不受影响、可离线使用。");
    msg
}

// 让 BrowserBridge::open 能复用已连接的浏览器会话并注入 sessionId
impl CdpSession {
    fn from_existing(mut base: CdpSession, session_id: String) -> Self {
        base.session_id = Some(session_id);
        base
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn find_chrome_returns_option_without_panic() {
        // 不断言是否找到（CI 环境可能无 Chromium），只确认不 panic。
        let _ = find_chrome();
    }

    #[test]
    fn browser_ws_url_from_stderr_line() {
        // 离线验证 stderr 解析逻辑（不启动浏览器）
        let line = "DevTools listening on ws://127.0.0.1:43981/devtools/browser/abc-123";
        let cap = line
            .find("DevTools listening on ")
            .map(|i| &line[i + "DevTools listening on ".len()..]);
        assert_eq!(cap, Some("ws://127.0.0.1:43981/devtools/browser/abc-123"));
    }

    #[test]
    fn candidate_chrome_paths_non_empty() {
        // 至少应包含跨平台标准路径；不因当前 OS 返回空
        assert!(!candidate_chrome_paths().is_empty());
    }

    #[test]
    fn install_guidance_matches_current_os() {
        let g = browser_install_guidance();
        assert!(!g.is_empty());
        match std::env::consts::OS {
            "windows" => assert!(g.contains("winget"), "Windows 指引应含 winget"),
            "macos" => assert!(g.contains("brew"), "macOS 指引应含 brew"),
            _ => assert!(
                g.contains("apt") || g.contains("dnf") || g.contains("pacman"),
                "Linux 指引应含包管理器"
            ),
        }
    }

    #[test]
    fn unavailable_message_includes_candidates_guidance_and_offline_note() {
        let m = browser_unavailable_message();
        assert!(m.contains("browser_bridge 不可用"), "应说明 bridge 不可用");
        assert!(m.contains("离线降级"), "应给出离线降级说明");
        // 至少一条候选路径 + 安装指引关键字
        assert!(
            m.contains("chrome.exe")
                || m.contains("/usr/bin/google-chrome")
                || m.contains("Google Chrome.app"),
            "应列出已探测候选路径"
        );
    }

    #[test]
    fn cdp_response_parse_error_path() {
        let raw = r#"{"id":5,"error":{"code":-32601,"message":"Method not found"}}"#;
        let r: CdpResponse = serde_json::from_str(raw).unwrap();
        assert!(r.into_result().is_err());
    }

    #[test]
    fn extract_browser_ws_from_version_parses() {
        let body = r#"{"Browser":"Chrome/130.0.0.0","webSocketDebuggerUrl":"ws://127.0.0.1:9333/devtools/browser/abc"}"#;
        assert_eq!(
            extract_browser_ws_from_version(body).as_deref(),
            Some("ws://127.0.0.1:9333/devtools/browser/abc")
        );
        // 缺字段 / 非 ws 前缀 / 非 JSON → None
        assert_eq!(extract_browser_ws_from_version(r#"{"Browser":"x"}"#), None);
        assert_eq!(
            extract_browser_ws_from_version(r#"{"webSocketDebuggerUrl":"http://x"}"#),
            None
        );
        assert_eq!(extract_browser_ws_from_version("not json"), None);
    }

    #[test]
    fn test_is_suspicious_screenshot_thresholds() {
        assert!(is_suspicious_screenshot(&[]), "空切片应判为可疑");
        assert!(is_suspicious_screenshot(&[0u8; 10]), "极小字节应判为可疑");
        assert!(
            !is_suspicious_screenshot(&[0u8; 2000]),
            "正常大小不应判为可疑"
        );
    }

    // 等待页面导航到目标 URL 且 readyState == complete（最多 10s），
    // 避免 about:blank（opaque origin 下 localStorage 抛异常）/ 加载中时序问题
    async fn wait_page_ready(session: &mut CdpSession, url_frag: &str) {
        for _ in 0..40 {
            let href = session
                .evaluate("location.href", false)
                .await
                .unwrap_or_default();
            let state = session
                .evaluate("document.readyState", false)
                .await
                .unwrap_or_default();
            if href.contains(url_frag) && state.trim() == "complete" {
                return;
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    }

    // 实时集成测试：Relay 连接用户真实 Chrome（非 headless，复用登录态）。
    #[tokio::test]
    #[ignore = "需要本机 Chromium；手动运行以验证 Relay 真实会话（登录态复用）"]
    async fn live_browser_relay_attaches_real_chrome() {
        use tokio::io::AsyncWriteExt;
        let bridge = BrowserBridge::new();
        assert!(bridge.is_available(), "未检测到 Chromium，跳过");
        let chrome = find_chrome().expect("应能找到 Chrome");
        let profile = std::env::temp_dir().join(format!("mgt-relay-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&profile);

        // 本地 mock 页面（同 origin 存储共享验证；循环 accept 以支撑多次导航）
        let mock = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mock_port = mock.local_addr().unwrap().port();
        let serve = tokio::spawn(async move {
            let resp = "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
            loop {
                let Ok((mut sock, _)) = mock.accept().await else {
                    break;
                };
                let _ = sock.write_all(resp.as_bytes()).await;
            }
        });
        let page_url = format!("http://127.0.0.1:{mock_port}/");

        // 启动真实 Chrome（非 headless，临时 profile，暴露调试端口）
        let mut child = TokioCommand::new(&chrome)
            .arg("--remote-debugging-port=9333")
            .arg(format!("--user-data-dir={}", profile.display()))
            .arg("--no-first-run")
            .arg("--disable-features=Translate")
            .arg("about:blank")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("启动真实 Chrome 失败");

        // 等待调试端口就绪
        let mut ready = false;
        for _ in 0..30 {
            if tokio::net::TcpStream::connect("127.0.0.1:9333")
                .await
                .is_ok()
            {
                ready = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(300)).await;
        }
        assert!(ready, "真实 Chrome 调试端口未就绪");

        // 第一次 relay 连接：写入登录态标记（等待页面就绪，避免 opaque origin 下 localStorage 抛异常）
        let mut s1 = bridge
            .relay(9333, &page_url)
            .await
            .expect("relay 应连上真实 Chrome");
        wait_page_ready(&mut s1.session, &mock_port.to_string()).await;
        let wrote = s1
            .session
            .evaluate(
                "localStorage.setItem('relay_session','relay-ok'); 'ok'",
                false,
            )
            .await
            .unwrap_or_else(|e| format!("EVAL_ERR: {e}"));
        assert_eq!(wrote.trim(), "ok", "写入登录态应成功，实际: {wrote}");
        let self_check = s1
            .session
            .evaluate("localStorage.getItem('relay_session')", false)
            .await
            .unwrap_or_default();
        assert_eq!(self_check.trim(), "relay-ok", "同一会话应立即读到写入值");

        // 第二次 relay 连接（模拟新工具调用）：读取同一登录态
        let mut s2 = bridge
            .relay(9333, &page_url)
            .await
            .expect("第二次 relay 应复用真实会话");
        wait_page_ready(&mut s2.session, &mock_port.to_string()).await;
        let v = s2
            .session
            .evaluate("localStorage.getItem('relay_session')", false)
            .await
            .unwrap_or_default();
        assert_eq!(
            v.trim(),
            "relay-ok",
            "真实 Chrome 会话应复用登录态（同 profile 同 origin）"
        );

        let _ = child.start_kill();
        let _ = child.wait().await;
        serve.abort();
        let _ = std::fs::remove_dir_all(&profile);
        println!("[live] Relay 真实会话通过：登录态复用验证成功");
    }

    // 实时集成测试：需本机 Chromium 且（截图时）联网。手动运行。
    #[tokio::test]
    #[ignore = "需要本机 Chromium；手动运行以验证 browser_bridge 重型桥"]
    async fn live_browser_navigate_and_evaluate() {
        let bridge = BrowserBridge::new();
        assert!(bridge.is_available(), "未检测到 Chromium，跳过");
        let mut page = bridge
            .open("data:text/html,<title>hi</title><h1>Hello</h1>")
            .await
            .unwrap();
        // 等待加载
        tokio::time::sleep(Duration::from_millis(800)).await;
        let title = page
            .session
            .evaluate("document.title", false)
            .await
            .unwrap();
        assert_eq!(title, "hi");
        let sum = page.session.evaluate("1+2", false).await.unwrap();
        assert_eq!(sum, "3");
        let _png = page.session.screenshot_png().await.unwrap();
        println!("[live] browser_bridge 端到端通过（导航+求值+截图）");
    }

    // 实时集成测试：Bloom 非视觉通道（a11y 语义快照 + ref 定位 + 真实坐标点击）。
    #[tokio::test]
    #[ignore = "需要本机 Chromium；手动运行以验证浏览器双通道（非视觉）"]
    async fn live_browser_a11y_snapshot_and_click() {
        let bridge = BrowserBridge::new();
        assert!(bridge.is_available(), "未检测到 Chromium，跳过");
        // data URL 需避免 #（会被解析为 URL fragment 截断内容）与非 ASCII（需 percent-encoding）
        let html = "data:text/html,<button id='b1' style='width:120px;height:40px' onclick='window.__clicked=1'>CLICK ME</button>\
            <a href='https://example.com'>Link</a><input placeholder='search box'><button disabled>DISABLED</button>";
        let mut page = bridge.open(html).await.unwrap();
        tokio::time::sleep(Duration::from_millis(1500)).await;

        // 1) a11y 语义快照：应包含 button/link/textbox 且剔除 generic/staticText
        let nodes = page.session.a11y_nodes().await.expect("a11y 快照应成功");
        assert!(!nodes.is_empty(), "页面应至少有一个可交互节点");
        let roles: Vec<&str> = nodes.iter().map(|n| n.role.as_str()).collect();
        assert!(roles.contains(&"button"), "应有 button，实际 {roles:?}");
        assert!(roles.contains(&"link"), "应有 link，实际 {roles:?}");
        assert!(roles.contains(&"textbox"), "应有 textbox，实际 {roles:?}");
        let btn = nodes
            .iter()
            .find(|n| n.role == "button")
            .expect("应有可点击 button");

        // 2) ref → DOM.getBoxModel 中心（扁平 8 元数组）→ Input 真实点击
        let (cx, cy) = page
            .session
            .box_center(btn.backend_node_id)
            .await
            .expect("盒模型中心");
        assert!(cx > 0.0 && cy > 0.0, "按钮坐标应位于视口内: ({cx},{cy})");
        page.session.click_at(cx, cy).await.expect("坐标点击应成功");

        // 3) 验证点击副作用生效
        let hit = page
            .session
            .evaluate("window.__clicked || 0", false)
            .await
            .unwrap();
        assert_eq!(hit, "1", "点击应触发 button onclick");
        println!(
            "[live] 非视觉通道通过：a11y 快照 {} 节点 → ref={} → 点击 @ ({cx:.0},{cy:.0}) 生效",
            nodes.len(),
            btn.backend_node_id
        );
    }

    // 实时集成测试：Bloom 视觉通道（截图 → 视口尺寸映射）。
    #[tokio::test]
    #[ignore = "需要本机 Chromium；手动运行以验证浏览器双通道（视觉截图）"]
    async fn live_browser_visual_screenshot_and_viewport() {
        let bridge = BrowserBridge::new();
        assert!(bridge.is_available(), "未检测到 Chromium，跳过");
        let mut page = bridge
            .open("data:text/html,<title>vis</title><button onclick='this.textContent=\"OK\"'>GO</button>")
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(900)).await;
        let png = page.session.screenshot_png().await.unwrap();
        assert!(png.len() > 1000, "截图应非空（{} bytes）", png.len());
        let (w, h) = page.session.viewport_size().await.unwrap();
        assert!(w > 0.0 && h > 0.0, "视口尺寸应有效: {w}x{h}");
        let meta = crate::vision::image_metadata(&png);
        println!(
            "[live] 视觉通道通过：截图 {} bytes（{}x{}）→ 视口 CSS {:.0}x{:.0}",
            png.len(),
            meta.width.unwrap_or(0),
            meta.height.unwrap_or(0),
            w,
            h
        );
    }
}
