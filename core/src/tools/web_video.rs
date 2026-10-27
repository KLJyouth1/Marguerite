//! 网页逐帧渲染视频工具（`web_video`）：HTML/CSS/JS → 逐帧截图 → mp4。
//!
//! # 为什么需要（2026-10-24）
//!
//! 代码生视频（Manim / 前端动画）此前只有两条路：模型自己写脚本调 ffmpeg（多数时候
//! 环境里根本没有可用的渲染管线），或者接一个外部视频模型（出的是「想象出来的画面」，
//! 不是用户给的那份页面）。而「把一段 HTML/CSS/JS 动效原样录成 mp4」是最常见的交付
//! 形态：数据大屏、加载动画、产品演示页、CSS 逐帧动画。
//!
//! 本工具补的正是这一段：用真实 Chromium（复用 [`crate::browser`] 的 CDP 桥）导航到
//! 临时 HTML，按时间轴逐帧调用页面里的 seek 钩子并截图，最后交给 ffmpeg 编码。
//!
//! # 工作方式
//!
//! 1. [`compose_html`] 把输入组装成完整文档：注入基础样式 + 帧时钟钩子
//!    `window.__mqSeek(ms)` + 就绪标记 `window.__mqReady`；
//! 2. 启动无头 Chromium，`file:///` 导航到临时文件，并按 `width × height` 覆盖视口；
//! 3. 逐帧 `__mqSeek(ms)` → `Page.captureScreenshot` → 落盘 `frame_%05d.png`；
//! 4. ffmpeg 以 `libx264 / yuv420p / +faststart` 合成，产物登记进产物台账。
//!
//! 用户的页面可以用两种方式接收帧时钟（任选其一，也可都不用）：
//! - CSS 变量：`--mq-t` 会被设为 `"<ms>ms"`（可配合 `@property`/`transition` 做插值）；
//! - 事件：`document.addEventListener('mq-frame', e => { const ms = e.detail; ... })`；
//! - 若页面自己实现了 `window.__seek(ms)`，本工具会优先调用它（完全接管帧时钟）。
//!
//! # 诚实边界
//!
//! - **只做画面，不含音轨**：产物是纯视频流，页面里的 `<audio>`/WebAudio 不会进入 mp4；
//! - **不做屏幕录制**：这是逐帧离屏渲染（每帧一张截图），不是实时录屏；页面依赖
//!   `requestAnimationFrame` 的动画不会被工具自动推进——必须在 `__seek`/`mq-frame`
//!   里按毫秒参数自行计算状态；
//! - 页面依赖的网络资源（CDN 字体、远程图片、接口数据）若加载不到，就按**实际渲染
//!   结果**出片，不伪造、不补图；就绪检查超时也会继续渲染静态首帧，并在返回体里用
//!   `readyTimedOut: true` 如实标注；
//! - 找不到 Chromium 或 ffmpeg 时直接报错并给出可执行的安装修复指引，不假装成功。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};

use crate::agent::tool_cycle::ToolResult;
use crate::browser::{
    browser_install_guidance, browser_unavailable_message, BrowserBridge, CdpCommand,
};
use crate::mcp::registry::ToolExecutor;
use crate::proc::SilentCommand;
use crate::tools::ffmpeg_bin::{missing_message, resolve_or_error};
use crate::tools::file_change::{record_artifact, GENERATED_KIND};
use crate::tools::fs_path::RootSet;

/// 默认背景色（深色，避免白底视频刺眼；页面自带背景时会被页面自己的样式覆盖）。
const DEFAULT_BACKGROUND: &str = "#0b0e14";
/// 默认时长（秒）。
const DEFAULT_DURATION_SECONDS: f64 = 3.0;
/// 默认帧率。
const DEFAULT_FPS: u32 = 24;
/// 默认宽度（CSS 像素）。
const DEFAULT_WIDTH: u32 = 1280;
/// 默认高度（CSS 像素）。
const DEFAULT_HEIGHT: u32 = 720;
/// 默认总超时（秒）。
const DEFAULT_TIMEOUT_SECONDS: u64 = 240;
/// 允许的最短时长（秒）。
const MIN_DURATION_SECONDS: f64 = 0.5;
/// 允许的最长时长（秒）。
const MAX_DURATION_SECONDS: f64 = 30.0;
/// 允许的最低帧率。
const MIN_FPS: u32 = 6;
/// 允许的最高帧率。
const MAX_FPS: u32 = 60;
/// 就绪检查的最长等待（毫秒）。超时后继续渲染静态首帧，并在返回体里标注。
const READY_TIMEOUT_MS: u64 = 5_000;
/// 就绪检查轮询间隔（毫秒）。
const READY_POLL_MS: u64 = 50;

/// 注入页面的帧时钟钩子（内联脚本）。
///
/// - `window.__mqSeek(ms)`：优先调用用户自实现的 `window.__seek(ms)`；否则写 CSS 变量
///   `--mq-t` 并派发 `mq-frame` 事件，让页面用声明式或事件式接收帧时钟；
/// - `window.__mqReady`：在内联脚本都执行完之后（`load` 事件 / readyState=complete）
///   置位，供上层做就绪检查。
///
/// 注意：本字符串会原样内联进 `<script>`，因此**不得**包含 `</script>`。
const HOOK_JS: &str = r#"(function(){
  document.documentElement.style.setProperty('--mq-t', '0ms');
  window.__mqSeek = function(ms){
    try {
      if (typeof window.__seek === 'function') { window.__seek(ms); return; }
      document.documentElement.style.setProperty('--mq-t', ms + 'ms');
      document.dispatchEvent(new CustomEvent('mq-frame', { detail: ms }));
    } catch (e) {
      /* 用户钩子内部异常不得中断逐帧渲染 */
    }
  };
  function mqMarkReady(){ window.__mqReady = true; }
  if (document.readyState === 'complete') { setTimeout(mqMarkReady, 0); }
  else { window.addEventListener('load', mqMarkReady); }
})();"#;

/// 组装页面：片段自动包成完整文档；完整文档则把样式与钩子注入 `<head>`。
///
/// 注入内容（顺序固定）：
/// 1. `html,body{margin:0;padding:0;overflow:hidden;background:<background>}`；
/// 2. 帧时钟钩子 [`HOOK_JS`]。
///
/// 纯函数，便于离线断言「钩子只注入一次、基础样式确实存在」。
pub fn compose_html(html: &str, background: &str) -> String {
    let bg = {
        let t = background.trim();
        if t.is_empty() {
            DEFAULT_BACKGROUND
        } else {
            t
        }
    };
    let style = format!(
        "<style>html,body{{margin:0;padding:0;overflow:hidden;background:{bg}}}</style>"
    );
    let injection = format!("{style}<script>{HOOK_JS}</script>");
    let lower = html.to_ascii_lowercase();

    if lower.contains("<html") {
        // 完整文档：优先插到 </head> 之前，保证钩子在页面脚本之前就绪。
        if let Some(idx) = lower.rfind("</head>") {
            let mut out = String::with_capacity(html.len() + injection.len());
            out.push_str(&html[..idx]);
            out.push_str(&injection);
            out.push_str(&html[idx..]);
            return out;
        }
        // 没有 </head> 的「半完整」文档：插到 <html ...> 开标签之后。
        if let Some(start) = lower.find("<html") {
            if let Some(rel_end) = html[start..].find('>') {
                let at = start + rel_end + 1;
                let mut out = String::with_capacity(html.len() + injection.len());
                out.push_str(&html[..at]);
                out.push_str(&injection);
                out.push_str(&html[at..]);
                return out;
            }
        }
        return format!("{injection}{html}");
    }

    // 片段：包成完整文档，钩子脚本放在 </body> 前，确保它晚于用户脚本执行。
    format!(
        "<!DOCTYPE html>\n<html lang=\"zh-CN\"><head><meta charset=\"utf-8\">\n{style}\n</head>\
         <body>\n{html}\n<script>{HOOK_JS}</script>\n</body></html>"
    )
}

/// 把本地路径转成 Chromium 可导航的 `file:///` URL。
///
/// - Windows 反斜杠统一换成 `/`；
/// - 非 ASCII（中文目录）与空格等按 UTF-8 做百分号编码，避免 `Page.navigate`
///   把空格截断或把中文当成非法字符；
/// - `:` `/` `-` `_` `.` `~` 与字母数字保持原样。
pub fn to_file_url(path: &Path) -> String {
    let raw = path.to_string_lossy().replace('\\', "/");
    let trimmed = raw.trim_start_matches('/');
    let mut out = String::from("file:///");
    for b in trimmed.bytes() {
        let keep = b.is_ascii_alphanumeric()
            || matches!(b, b'/' | b':' | b'-' | b'_' | b'.' | b'~');
        if keep {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// 计算帧数：`round(duration * fps)`，至少 1 帧；上限 `fps * 60`（60 秒）。
///
/// 同时校验时长与帧率区间（0.5–30 秒、6–60 fps），非法值返回可读错误。
pub fn frame_count(duration_seconds: f64, fps: u32) -> Result<u32, String> {
    if !(MIN_DURATION_SECONDS..=MAX_DURATION_SECONDS).contains(&duration_seconds) {
        return Err(format!(
            "duration_seconds 必须在 {MIN_DURATION_SECONDS}–{MAX_DURATION_SECONDS} 秒之间，收到：{duration_seconds}"
        ));
    }
    if !(MIN_FPS..=MAX_FPS).contains(&fps) {
        return Err(format!(
            "fps 必须在 {MIN_FPS}–{MAX_FPS} 之间，收到：{fps}"
        ));
    }
    // 上限即 60 秒的帧数；超过会被拒（防止误传超大时长把机器拖死）。
    let cap = fps.saturating_mul(60);
    let n = (duration_seconds * f64::from(fps)).round().max(1.0) as u32;
    if n > cap {
        return Err(format!(
            "帧数 {n} 超过上限 {cap}（{fps}fps × 60 秒）；请缩短时长或降低帧率"
        ));
    }
    Ok(n)
}

/// 帧文件名（1 起始，匹配 ffmpeg 的 `frame_%05d.png` 约定）。
pub fn frame_name(index: usize) -> String {
    format!("frame_{:05}.png", index + 1)
}

/// 从 PNG 字节里读取真实像素尺寸（IHDR：偏移 16 宽 / 20 高，大端 u32）。
///
/// 用于如实回报**实际**出片分辨率：视口覆盖失败时也能给出真实值，而不是照抄参数。
pub fn png_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    if !bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return None;
    }
    if bytes.get(12..16)? != b"IHDR".as_slice() {
        return None;
    }
    let w = u32::from_be_bytes(bytes.get(16..20)?.try_into().ok()?);
    let h = u32::from_be_bytes(bytes.get(20..24)?.try_into().ok()?);
    Some((w, h))
}

/// 取字符串末尾 `n` 个字符（ffmpeg 的报错重点通常在末尾）。
fn tail(s: &str, n: usize) -> String {
    let count = s.chars().count();
    if count <= n {
        s.to_string()
    } else {
        s.chars().skip(count - n).collect()
    }
}

/// 内部失败（带错误码），统一在 `execute` 里转成 [`ToolResult::Error`]。
struct Fail {
    code: &'static str,
    message: String,
}

impl Fail {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

/// 临时目录守卫：无论成功、失败还是超时，离开作用域时都删掉渲染中间文件。
struct TempDirGuard(PathBuf);

impl Drop for TempDirGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// 渲染结果（成功返回体所需字段）。
struct RenderOutcome {
    frames: u32,
    width: u32,
    height: u32,
    ready_timed_out: bool,
}

/// `web_video` 工具。
pub struct WebVideoTool {
    workspace: RootSet,
}

impl WebVideoTool {
    pub fn new(workspace: impl Into<RootSet>) -> Self {
        let workspace: RootSet = workspace.into();
        Self { workspace }
    }
}

/// 从参数表读一个可解析的数字，缺省时用默认值。
fn parse_num<T: std::str::FromStr>(
    args: &HashMap<String, String>,
    key: &str,
    default: T,
) -> Result<T, Fail> {
    match args.get(key).map(|s| s.trim()).filter(|s| !s.is_empty()) {
        None => Ok(default),
        Some(raw) => raw.parse::<T>().map_err(|_| {
            Fail::new(
                "INVALID_PARAMS",
                format!("参数 {key} 必须是数字，收到：{raw}"),
            )
        }),
    }
}

#[async_trait]
impl ToolExecutor for WebVideoTool {
    fn name(&self) -> &str {
        "web_video"
    }

    fn description(&self) -> &str {
        "把一段 HTML/CSS/JS 逐帧渲染成 mp4（离屏渲染，不是录屏）。\
         给 html（完整文档或片段），工具会注入帧时钟钩子 window.__mqSeek(ms)：\
         页面可用 CSS 变量 --mq-t、监听 mq-frame 事件，或自己实现 window.__seek(ms) 来驱动动效。\
         可选 out/duration_seconds/fps/width/height/background/timeout_seconds。\
         产物只有画面、没有音轨；页面依赖的网络资源加载不到时按实际渲染结果出片，\
         就绪检查超时会用 readyTimedOut=true 标注。"
    }

    fn input_schema(&self) -> Value {
        json!({"type":"object","properties":{
            "html":{"type":"string","description":"页面源码：完整 HTML 文档或片段（片段会被包成完整文档）"},
            "out":{"type":"string","description":"输出 mp4 路径（相对工作区，默认 generated/videos/web-<时间戳>.mp4）"},
            "duration_seconds":{"type":"number","description":"时长（秒），默认 3，允许 0.5–30"},
            "fps":{"type":"number","description":"帧率，默认 24，允许 6–60"},
            "width":{"type":"number","description":"视口宽度（CSS 像素），默认 1280"},
            "height":{"type":"number","description":"视口高度（CSS 像素），默认 720"},
            "background":{"type":"string","description":"页面背景色（CSS 颜色），默认 #0b0e14"},
            "timeout_seconds":{"type":"number","description":"整个渲染流程的总超时（秒），默认 240"}
        },"required":["html"]})
    }

    async fn execute(&self, args: &HashMap<String, String>) -> Result<ToolResult> {
        // ① 源码：空内容必须在启动浏览器之前拒绝，绝不空跑一次渲染。
        let html = args
            .get("html")
            .map(|s| s.to_string())
            .unwrap_or_default();
        if html.trim().is_empty() {
            return Ok(ToolResult::Error {
                message: "web_video 需要非空的 html（完整文档或片段）".to_string(),
                code: "INVALID_PARAMS".to_string(),
            });
        }

        // ② 参数解析与区间校验（帧数校验同时覆盖 duration/fps 的合法区间）。
        let duration_seconds =
            match parse_num(args, "duration_seconds", DEFAULT_DURATION_SECONDS) {
                Ok(v) => v,
                Err(f) => return Ok(ToolResult::Error { message: f.message, code: f.code.to_string() }),
            };
        let fps = match parse_num(args, "fps", DEFAULT_FPS) {
            Ok(v) => v,
            Err(f) => return Ok(ToolResult::Error { message: f.message, code: f.code.to_string() }),
        };
        let width = match parse_num(args, "width", DEFAULT_WIDTH) {
            Ok(v) => v,
            Err(f) => return Ok(ToolResult::Error { message: f.message, code: f.code.to_string() }),
        };
        let height = match parse_num(args, "height", DEFAULT_HEIGHT) {
            Ok(v) => v,
            Err(f) => return Ok(ToolResult::Error { message: f.message, code: f.code.to_string() }),
        };
        let timeout_seconds = match parse_num(args, "timeout_seconds", DEFAULT_TIMEOUT_SECONDS) {
            Ok(v) => v,
            Err(f) => return Ok(ToolResult::Error { message: f.message, code: f.code.to_string() }),
        };
        if width == 0 || height == 0 {
            return Ok(ToolResult::Error {
                message: format!("width/height 必须大于 0，收到：{width}x{height}"),
                code: "INVALID_PARAMS".to_string(),
            });
        }
        // libx264 的 yuv420p 要求宽高为偶数；奇数会得到一句难以理解的 ffmpeg 报错，
        // 这里提前拒绝并说明原因。
        if width % 2 != 0 || height % 2 != 0 {
            return Ok(ToolResult::Error {
                message: format!(
                    "width/height 必须为偶数（libx264 + yuv420p 的色度采样要求），收到：{width}x{height}"
                ),
                code: "INVALID_PARAMS".to_string(),
            });
        }
        if timeout_seconds == 0 {
            return Ok(ToolResult::Error {
                message: "timeout_seconds 必须大于 0".to_string(),
                code: "INVALID_PARAMS".to_string(),
            });
        }
        let frames = match frame_count(duration_seconds, fps) {
            Ok(n) => n,
            Err(e) => {
                return Ok(ToolResult::Error {
                    message: e,
                    code: "INVALID_PARAMS".to_string(),
                })
            }
        };

        // ③ ffmpeg：先解析，缺了就立刻给安装指引（不要先开浏览器白跑一遍）。
        let ffmpeg = match resolve_or_error("ffmpeg") {
            Ok(p) => p,
            Err(msg) => {
                return Ok(ToolResult::Error {
                    message: msg,
                    code: "NO_FFMPEG".to_string(),
                })
            }
        };

        // ④ 输出路径：必须落在工作区内。
        let rel = args
            .get("out")
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| {
                format!(
                    "generated/videos/web-{}.mp4",
                    chrono::Local::now().format("%Y%m%d-%H%M%S")
                )
            });
        let target = match self.workspace.resolve(&rel) {
            Ok(p) => p,
            Err(e) => {
                return Ok(ToolResult::Error {
                    message: format!("输出路径非法（必须落在工作区内）: {e}"),
                    code: "INVALID_PATH".to_string(),
                })
            }
        };
        if let Some(dir) = target.parent() {
            if let Err(e) = std::fs::create_dir_all(dir) {
                return Ok(ToolResult::Error {
                    message: format!("创建输出目录失败: {e}"),
                    code: "IO_ERROR".to_string(),
                });
            }
        }

        let background = args
            .get("background")
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| DEFAULT_BACKGROUND.to_string());

        // ⑤ 真正渲染：整个流程包一层总超时（含浏览器启动、逐帧截图与 ffmpeg 合成）。
        let job = render(
            &html,
            &background,
            &target,
            frames,
            fps,
            width,
            height,
            &ffmpeg,
        );
        match tokio::time::timeout(Duration::from_secs(timeout_seconds), job).await {
            Ok(Ok(outcome)) => {
                let bytes = std::fs::metadata(&target).map(|m| m.len()).unwrap_or(0);
                record_artifact("", &target.display().to_string(), GENERATED_KIND);
                Ok(ToolResult::Success {
                    data: json!({
                        "ok": true,
                        "path": target.display().to_string(),
                        "frames": outcome.frames,
                        "durationSeconds": duration_seconds,
                        "fps": fps,
                        "width": outcome.width,
                        "height": outcome.height,
                        "bytes": bytes,
                        "readyTimedOut": outcome.ready_timed_out,
                    })
                    .to_string(),
                    metadata: HashMap::new(),
                })
            }
            Ok(Err(f)) => Ok(ToolResult::Error {
                message: f.message,
                code: f.code.to_string(),
            }),
            Err(_) => Ok(ToolResult::Error {
                message: format!(
                    "web_video 渲染超时（>{timeout_seconds}s）。可执行修复：\
                     ① 降低 duration_seconds / fps 或检查页面是否有死循环脚本；\
                     ② 调大 timeout_seconds（如 600）；\
                     ③ 确认无头 Chrome 能正常启动（杀毒软件拦截、端口占用都会让启动变慢）。"
                ),
                code: "TIMEOUT".to_string(),
            }),
        }
    }
}

/// 渲染主流程（不含参数校验与产物登记）。
///
/// 调用序列：
/// `BrowserBridge::launch` → `CdpSession::connect` → `Target.createTarget` →
/// `Target.attachToTarget(flatten)` → `Page.enable` / `Runtime.enable` → `Page.navigate`
/// （以上封装在 `BrowserBridge::open` 里）→ `Emulation.setDeviceMetricsOverride` →
/// 轮询 `Runtime.evaluate` → 每帧 `Runtime.evaluate` + `Page.captureScreenshot`。
#[allow(clippy::too_many_arguments)]
async fn render(
    html: &str,
    background: &str,
    target: &Path,
    frames: u32,
    fps: u32,
    width: u32,
    height: u32,
    ffmpeg: &Path,
) -> Result<RenderOutcome, Fail> {
    // 临时目录：唯一子目录，结束时由守卫删除。
    let dir = std::env::temp_dir().join(format!(
        "mq-web-video-{}-{}",
        std::process::id(),
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&dir)
        .map_err(|e| Fail::new("IO_ERROR", format!("创建临时目录失败: {e}")))?;
    let _guard = TempDirGuard(dir.clone());

    let page_path = dir.join("index.html");
    std::fs::write(&page_path, compose_html(html, background))
        .map_err(|e| Fail::new("IO_ERROR", format!("写入临时页面失败: {e}")))?;

    // 浏览器：不可用时给出「已探测路径 + 安装命令」的完整指引。
    let bridge = BrowserBridge::new();
    if !bridge.is_available() {
        return Err(Fail::new("NO_BROWSER", browser_unavailable_message()));
    }

    let file_url = to_file_url(&page_path);
    let mut page = bridge.open(&file_url).await.map_err(|e| {
        Fail::new(
            "BROWSER_LAUNCH_FAILED",
            format!(
                "启动无头浏览器并打开临时页面失败：{e}\n\
                 可执行修复：① 关闭占用调试端口的其它 Chrome/Chromium；\
                 ② 确认杀毒软件未拦截 Chrome 子进程；\
                 ③ 手动运行一次 Chrome 看是否能正常启动。\n\n{}",
                browser_install_guidance()
            ),
        )
    })?;

    // 视口尺寸：CSS 像素 = 截图像素（deviceScaleFactor=1）。失败不致命——
    // 返回体里的 width/height 会按首帧 PNG 的真实尺寸回报。
    let _ = page
        .session
        .send_command(CdpCommand {
            id: 0,
            method: "Emulation.setDeviceMetricsOverride".to_string(),
            params: Some(json!({
                "width": width,
                "height": height,
                "deviceScaleFactor": 1,
                "mobile": false,
            })),
        })
        .await;

    // 就绪检查：最多等 5s（50ms 轮询）；超时继续渲染静态首帧，但如实标注。
    let mut ready_timed_out = false;
    let deadline = tokio::time::Instant::now() + Duration::from_millis(READY_TIMEOUT_MS);
    loop {
        let ready = page
            .session
            .evaluate("window.__mqReady === true", false)
            .await
            .map(|v| v.trim() == "true")
            .unwrap_or(false);
        if ready {
            break;
        }
        if tokio::time::Instant::now() >= deadline {
            ready_timed_out = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(READY_POLL_MS)).await;
    }

    // 逐帧：seek → 截图 → 落盘。
    let mut actual: Option<(u32, u32)> = None;
    for i in 0..frames {
        let ms = (f64::from(i) * 1000.0 / f64::from(fps)).round() as u64;
        // 必须包成返回 true 的 IIFE：帧时钟钩子通常没有返回值，
        // 直接 evaluate 会拿到 undefined，被 Runtime.evaluate 的取值逻辑判成失败。
        page.session
            .evaluate(
                &format!("(() => {{ window.__mqSeek({ms}); return true; }})()"),
                false,
            )
            .await
            .map_err(|e| {
                Fail::new(
                    "RENDER_FRAME_FAILED",
                    format!(
                        "第 {}/{frames} 帧（t={ms}ms）执行帧时钟钩子失败：{e}。\
                         页面若自定义了 window.__seek(ms)，请确认它不会抛异常或返回未决 Promise。",
                        i + 1
                    ),
                )
            })?;
        let png = page.session.screenshot_png().await.map_err(|e| {
            Fail::new(
                "SCREENSHOT_FAILED",
                format!("第 {}/{frames} 帧截图失败：{e}", i + 1),
            )
        })?;
        if actual.is_none() {
            actual = png_dimensions(&png);
        }
        std::fs::write(dir.join(frame_name(i as usize)), &png)
            .map_err(|e| Fail::new("IO_ERROR", format!("写入第 {} 帧失败: {e}", i + 1)))?;
    }

    // ffmpeg 合成：工作目录切到临时目录，输入用相对模式串，避免路径转义问题。
    let mut cmd = std::process::Command::new(ffmpeg);
    cmd.silent();
    cmd.arg("-y")
        .arg("-framerate")
        .arg(fps.to_string())
        .arg("-i")
        .arg("frame_%05d.png")
        .arg("-c:v")
        .arg("libx264")
        .arg("-pix_fmt")
        .arg("yuv420p")
        .arg("-movflags")
        .arg("+faststart")
        .arg(target)
        .current_dir(&dir)
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    let output = cmd
        .output()
        .map_err(|e| Fail::new("FFMPEG_FAILED", format!("执行 ffmpeg 失败：{e}\n\n{}", missing_message("ffmpeg"))))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(Fail::new(
            "FFMPEG_FAILED",
            format!(
                "ffmpeg 合成失败（退出码 {:?}）：\n{}\n\n\
                 可执行修复：确认磁盘空间充足、输出路径未被占用，或改用其它 out 路径重试。",
                output.status.code(),
                tail(stderr.trim(), 800)
            ),
        ));
    }
    let bytes = std::fs::metadata(target).map(|m| m.len()).unwrap_or(0);
    if bytes == 0 {
        return Err(Fail::new(
            "FFMPEG_FAILED",
            "ffmpeg 报告成功但输出文件为空；请检查输出路径是否可写（或被其它程序占用）。"
                .to_string(),
        ));
    }

    let (w, h) = actual.unwrap_or((width, height));
    Ok(RenderOutcome {
        frames,
        width: w,
        height: h,
        ready_timed_out,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── to_file_url ──

    #[test]
    fn to_file_url_handles_windows_drive_and_chinese_space() {
        let p = Path::new(r"C:\Users\张三\My Videos\index.html");
        assert_eq!(
            to_file_url(p),
            "file:///C:/Users/%E5%BC%A0%E4%B8%89/My%20Videos/index.html",
            "Windows 盘符、中文目录与空格都要正确编码"
        );
        // 已经是正斜杠的路径也要幂等
        let q = Path::new("C:/tmp/a b.html");
        assert_eq!(to_file_url(q), "file:///C:/tmp/a%20b.html");
    }

    // ── compose_html ──

    #[test]
    fn compose_html_wraps_fragment_with_style_and_hooks() {
        let out = compose_html("<div>hi</div>", "#0b0e14");
        assert!(out.contains("<html"), "片段必须被包成完整文档");
        assert!(
            out.contains("html,body{margin:0;padding:0;overflow:hidden;background:#0b0e14}"),
            "必须注入基础样式与背景色: {out}"
        );
        assert!(out.contains("window.__mqSeek"), "必须注入帧时钟钩子");
        assert!(out.contains("window.__mqReady"), "必须注入就绪标记");
        assert!(out.contains("<div>hi</div>"), "用户片段必须原样保留");
        // 自定义背景色应进入基础样式
        let custom = compose_html("x", "rgb(0, 0, 0)");
        assert!(custom.contains("background:rgb(0, 0, 0)"));
    }

    #[test]
    fn compose_html_injects_into_full_document_once() {
        let doc = "<!DOCTYPE html><html><head><title>t</title></head><body><b>x</b></body></html>";
        let out = compose_html(doc, "#123456");
        assert_eq!(
            out.matches("<html").count(),
            1,
            "完整文档不得被再包一层 <html>"
        );
        assert_eq!(out.matches("window.__mqSeek").count(), 1, "钩子只注入一次");
        assert!(
            out.contains("background:#123456"),
            "完整文档也要注入基础样式"
        );
        // 注入点必须在 </head> 之前（样式/钩子先于页面脚本生效）
        let style_at = out.find("<style>").unwrap();
        let head_end = out.find("</head>").unwrap();
        assert!(style_at < head_end, "样式应位于 </head> 之前");
        assert!(out.contains("<title>t</title>"), "原文档内容不得丢失");
        assert!(out.contains("<b>x</b>"), "原文档正文不得丢失");
    }

    // ── 帧数 / 帧命名 ──

    #[test]
    fn frame_count_rounds_and_enforces_limits() {
        assert_eq!(frame_count(3.0, 24).unwrap(), 72);
        assert_eq!(frame_count(0.5, 6).unwrap(), 3);
        assert_eq!(frame_count(30.0, 60).unwrap(), 1800, "30s × 60fps 正好等于上限");
        // 时长 / 帧率越界
        assert!(frame_count(0.4, 24).is_err(), "低于 0.5s 应拒绝");
        assert!(frame_count(31.0, 24).is_err(), "超过 30s 应拒绝");
        assert!(frame_count(3.0, 5).is_err(), "低于 6fps 应拒绝");
        assert!(frame_count(3.0, 61).is_err(), "高于 60fps 应拒绝");
    }

    #[test]
    fn frame_names_match_ffmpeg_pattern() {
        assert_eq!(frame_name(0), "frame_00001.png");
        assert_eq!(frame_name(41), "frame_00042.png");
        assert_eq!(frame_name(99_999), "frame_100000.png");
    }

    // ── 空 html 拒绝（不触碰浏览器） ──

    #[tokio::test]
    async fn empty_html_is_rejected_without_launching_browser() {
        let tool = WebVideoTool::new(std::env::temp_dir());
        let mut args = HashMap::new();
        args.insert("html".to_string(), "   \n\t ".to_string());
        match tool.execute(&args).await.unwrap() {
            ToolResult::Error { code, .. } => assert_eq!(code, "INVALID_PARAMS"),
            other => panic!("空 html 应返回 INVALID_PARAMS，实际：{other:?}"),
        }
    }

    // ── PNG 尺寸解析 ──

    #[test]
    fn png_dimensions_reads_ihdr() {
        let mut png = vec![0u8; 33];
        png[0..8].copy_from_slice(b"\x89PNG\r\n\x1a\n");
        png[12..16].copy_from_slice(b"IHDR");
        png[16..20].copy_from_slice(&1280u32.to_be_bytes());
        png[20..24].copy_from_slice(&720u32.to_be_bytes());
        assert_eq!(png_dimensions(&png), Some((1280, 720)));
        assert_eq!(png_dimensions(b"not a png"), None, "非 PNG 必须返回 None");
    }

    /// 端到端：真起无头 Chrome 逐帧截图 + ffmpeg 合成（默认忽略）。
    ///
    /// 用 `cargo test --lib -- --ignored real_browser_end_to_end` 显式执行。
    #[tokio::test]
    #[ignore = "需要本机 Chrome + ffmpeg；用 --ignored 显式运行"]
    async fn real_browser_end_to_end() {
        let dir = std::env::temp_dir().join(format!(
            "mq-webvideo-e2e-{}-{}",
            std::process::id(),
            chrono::Local::now().timestamp_nanos_opt().unwrap_or(0)
        ));
        std::fs::create_dir_all(&dir).expect("建工作区");
        let tool = WebVideoTool::new(dir.clone());
        let mut args = HashMap::new();
        args.insert(
            "html".to_string(),
            "<style>#b{width:200px;height:200px;background:#e44;transition:none}</style>\
             <div id=b></div><script>window.__seek=function(ms){\
               document.getElementById('b').style.transform='translateX('+(ms/4)+'px)'}\
             </script>"
                .to_string(),
        );
        args.insert("out".to_string(), "e2e.mp4".to_string());
        args.insert("duration_seconds".to_string(), "0.5".to_string());
        args.insert("fps".to_string(), "8".to_string());
        args.insert("width".to_string(), "320".to_string());
        args.insert("height".to_string(), "240".to_string());
        args.insert("timeout_seconds".to_string(), "120".to_string());

        let result = tool.execute(&args).await.expect("execute 不应 panic");
        let out = dir.join("e2e.mp4");
        let bytes = std::fs::read(&out).unwrap_or_default();
        let _ = std::fs::remove_dir_all(&dir);

        match result {
            ToolResult::Success { data, .. } => {
                assert!(bytes.len() > 1024, "mp4 太小，疑似空壳: {} 字节", bytes.len());
                assert!(
                    bytes.len() >= 8 && &bytes[4..8] == b"ftyp",
                    "不是合法 mp4 容器（缺 ftyp）"
                );
                assert!(data.contains("frames"), "返回体应含帧数: {data}");
            }
            ToolResult::Error { code, message } => {
                panic!("浏览器端到端失败（{code}）：{message}")
            }
            other => panic!("端到端返回了非预期结果：{other:?}"),
        }
    }
}
