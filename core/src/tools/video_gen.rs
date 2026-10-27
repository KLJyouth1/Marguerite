//! 生视频工具（`video_generate`）：文本 / 图片 → 视频。
//!
//! # 为什么需要（2026-10-24）
//!
//! 多模态能力层早已能解析 `MediaKind::Video`（默认端点 `/v1/videos/generations`、
//! 能力位 `caps.video_output`、端点覆盖 `caps.video_endpoint`、自定义参数
//! `caps.video_params`），但**工具本体一直不存在** —— 用户在设置里勾了「视频输出」
//! 之后，主智能体仍然只能回「我无法生成视频」。
//!
//! 本工具补的正是这一段：把「档案声明的视频输出能力」变成一次真实的视频生成调用，
//! 并把结果**落盘到工作区**（而不是只回一个很快会过期的临时 URL）。
//!
//! # 诚实边界
//!
//! - 没有声明该能力的档案时**明确报错**并告诉用户去哪个页面勾哪个框，
//!   不假装成功、不返回占位文件；
//! - 视频端点形状远不如生图统一：同步形状（`data[].url` / `data[].b64_json` /
//!   `video_url` / `output.video_url`）直接落盘；**返回任务 id 的异步端点**
//!   （Sora / 可灵 / Runway 一类）走「提交 → 轮询 → 取片」：
//!   状态地址优先取 `status_url`（含 `{id}` 占位；可来自工具参数或档案
//!   `video_params`），未配置时按 OpenAI 约定把 `/generations` 换成 `/{id}` 推导；
//!   两者都推不出时**明确报错并给配置示例**，不会静默当成功；
//! - 白名单只放行视频生成常用键，模型不能借此往请求体里塞任意字段。
//!   `status_url` / `status_method` / `poll_interval_ms` / `poll_timeout_s`
//!   是**控制参数**，单独消费，不进请求体。

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use base64::Engine as _;
use serde_json::{json, Value};

use crate::agent::tool_cycle::ToolResult;
use crate::mcp::registry::ToolExecutor;
use crate::model::media_capability::{self, MediaEndpoint, MediaKind};
use crate::tools::file_change::{record_artifact, GENERATED_KIND};
use crate::tools::fs_path::RootSet;

/// 生视频 HTTP 通道（可注入：单测用 mock 断言请求体，不真的联网）。
#[async_trait]
pub trait VideoTransport: Send + Sync {
    /// 发起生成请求，返回上游 JSON。
    async fn generate(&self, ep: &MediaEndpoint, body: &Value) -> Result<Value, String>;
    /// 查询异步任务状态（GET 或 POST，由 `method` 决定）。
    async fn status(
        &self,
        ep: &MediaEndpoint,
        url: &str,
        method: &str,
    ) -> Result<Value, String>;
    /// 下载视频 URL 的字节（上游只回 URL 时用；视频链接普遍会过期）。
    async fn download(&self, url: &str) -> Result<Vec<u8>, String>;
}

/// 生产实现：reqwest。
pub struct HttpVideoTransport;

#[async_trait]
impl VideoTransport for HttpVideoTransport {
    async fn generate(&self, ep: &MediaEndpoint, body: &Value) -> Result<Value, String> {
        // 视频生成比出图慢一个量级（数十秒到数分钟），内层预算留 540s，
        // 外层工具超时 600s，保证「慢但正常」的请求不会被内层先掐断。
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(540))
            .build()
            .map_err(|e| format!("构造 HTTP 客户端失败: {e}"))?;
        let mut req = client.post(&ep.url).json(body);
        if !ep.api_key.is_empty() {
            req = req.header("Authorization", format!("Bearer {}", ep.api_key));
        }
        let resp = req.send().await.map_err(|e| format!("请求失败: {e}"))?;
        let status = resp.status();
        let text = resp
            .text()
            .await
            .map_err(|e| format!("读取响应失败: {e}"))?;
        if !status.is_success() {
            return Err(format!(
                "上游返回 {}：{}",
                status.as_u16(),
                truncate(&text, 600)
            ));
        }
        serde_json::from_str::<Value>(&text)
            .map_err(|e| format!("上游返回非 JSON（{}）：{}", e, truncate(&text, 300)))
    }

    async fn status(
        &self,
        ep: &MediaEndpoint,
        url: &str,
        method: &str,
    ) -> Result<Value, String> {
        // 单次查询给 60s：状态接口本身很轻，慢就是上游出问题，不该拖住整个轮询预算。
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(60))
            .build()
            .map_err(|e| format!("构造 HTTP 客户端失败: {e}"))?;
        let mut req = if method.eq_ignore_ascii_case("POST") {
            // 少数服务商把查询也做成 POST（要带同一个任务体），这里只发空 JSON。
            client.post(url).json(&serde_json::json!({}))
        } else {
            client.get(url)
        };
        if !ep.api_key.is_empty() {
            req = req.header("Authorization", format!("Bearer {}", ep.api_key));
        }
        let resp = req.send().await.map_err(|e| format!("状态查询失败: {e}"))?;
        let code = resp.status();
        let text = resp
            .text()
            .await
            .map_err(|e| format!("读取状态响应失败: {e}"))?;
        if !code.is_success() {
            return Err(format!(
                "状态接口返回 {}：{}",
                code.as_u16(),
                truncate(&text, 400)
            ));
        }
        serde_json::from_str::<Value>(&text)
            .map_err(|e| format!("状态接口返回非 JSON（{}）：{}", e, truncate(&text, 300)))
    }

    async fn download(&self, url: &str) -> Result<Vec<u8>, String> {
        // 视频体积远大于图片，给 300s 下载窗口。
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(300))
            .build()
            .map_err(|e| format!("构造 HTTP 客户端失败: {e}"))?;
        let resp = client
            .get(url)
            .send()
            .await
            .map_err(|e| format!("下载失败: {e}"))?;
        if !resp.status().is_success() {
            return Err(format!("下载返回 {}", resp.status().as_u16()));
        }
        resp.bytes()
            .await
            .map(|b| b.to_vec())
            .map_err(|e| format!("下载内容读取失败: {e}"))
    }
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        let head: String = s.chars().take(n).collect();
        format!("{head}…")
    }
}

/// 按文件头判定视频扩展名（上游有时不给 mime）。
///
/// 判定顺序有讲究：AVI 与 WebM 都有固定魔数，MP4 的 `ftyp` 在偏移 4
/// （前面是 box 长度），所以三者互不冲突；认不出时兜底 mp4。
fn sniff_ext(bytes: &[u8]) -> &'static str {
    if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"AVI " {
        "avi"
    } else if bytes.starts_with(b"\x1aE\xdf\xa3") {
        "webm"
    } else {
        // MP4 的 `ftyp` 在偏移 4（前面是 box 长度），与上面两个魔数不冲突；
        // 命中 ftyp 与认不出都归 mp4，所以合成一个兜底分支。
        "mp4"
    }
}

/// 从单个对象里取一条视频：优先 `b64_json`，其次 `url` / `video_url`。
fn pick_from(obj: &Value) -> Option<(Option<String>, Option<String>)> {
    if let Some(b64) = obj
        .get("b64_json")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        return Some((Some(b64.to_string()), None));
    }
    for key in ["url", "video_url"] {
        if let Some(u) = obj
            .get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            return Some((None, Some(u.to_string())));
        }
    }
    None
}

/// 从上游 JSON 里取出第一条视频。
///
/// 视频端点形状不统一，按真实出现的四种由内到外依次尝试：
/// 1. `{"data":[{"url":…}]}` / `{"data":[{"b64_json":…}]}`（OpenAI 兼容，最常见）
/// 2. `{"data":{…}}`（把 data 写成对象的小众实现）
/// 3. `{"video_url":…}` / `{"url":…}`（扁平返回）
/// 4. `{"output":{"video_url":…}}`（套一层 output）
///
/// 都取不到时返回错误并带上截断后的原始响应 —— 用户要能看到上游到底回了什么，
/// 否则只能反复重试同一个形状问题。
fn parse_video(v: &Value) -> Result<(Option<String>, Option<String>), String> {
    if let Some(arr) = v.get("data").and_then(Value::as_array) {
        let item = arr.first().ok_or_else(|| {
            format!(
                "上游返回 data 为空数组（未生成任何视频）：{}",
                truncate(&v.to_string(), 300)
            )
        })?;
        if let Some(hit) = pick_from(item) {
            return Ok(hit);
        }
        return Err(format!(
            "上游响应 data[0] 既无 url 也无 b64_json：{}",
            truncate(&item.to_string(), 300)
        ));
    }
    if let Some(item) = v.get("data").filter(|d| d.is_object()) {
        if let Some(hit) = pick_from(item) {
            return Ok(hit);
        }
    }
    if let Some(hit) = pick_from(v) {
        return Ok(hit);
    }
    if let Some(out) = v.get("output").filter(|o| o.is_object()) {
        if let Some(hit) = pick_from(out) {
            return Ok(hit);
        }
    }
    Err(format!(
        "上游响应中找不到视频地址或 b64 数据（可能是需要轮询的异步任务端点）：{}",
        truncate(&v.to_string(), 300)
    ))
}

/// 从上游 JSON 里找出异步任务 id（找不到 = 不是异步任务，或形状不认识）。
///
/// 覆盖真实见过的四种位置：顶层 `id/task_id/request_id`、`data.id`、
/// `output.id`、`operation.name`（Google 风格长资源名）。
fn find_task_id(v: &Value) -> Option<String> {
    const TOP_KEYS: &[&str] = &[
        "id",
        "task_id",
        "taskId",
        "request_id",
        "requestId",
        "operation_name",
    ];
    for key in TOP_KEYS {
        if let Some(s) = v
            .get(*key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            return Some(s.to_string());
        }
    }
    for parent in ["data", "output", "result", "operation"] {
        let Some(obj) = v.get(parent) else { continue };
        for key in ["id", "task_id", "name", "operation_name"] {
            if let Some(s) = obj
                .get(key)
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
            {
                return Some(s.to_string());
            }
        }
    }
    None
}

/// 解析状态查询地址模板（未做 `{id}` 替换）。
///
/// 优先级：工具参数 `status_url` > 档案 `video_params.status_url` > OpenAI 约定推导
/// （把生成地址结尾的 `/generations` 换成 `/{id}`）。全都拿不到 → `None`。
fn status_url_template(ep: &MediaEndpoint, extra: &HashMap<String, String>) -> Option<String> {
    if let Some(t) = extra
        .get("status_url")
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
    {
        return Some(t.to_string());
    }
    if let Some(t) = ep
        .params
        .as_ref()
        .and_then(|p| p.get("status_url"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        return Some(t.to_string());
    }
    let url = ep.url.trim_end_matches('/');
    url.strip_suffix("/generations")
        .map(|base| format!("{base}/{{id}}"))
}

/// 把 `{id}` 占位替换成真实任务 id。
fn expand_status_url(template: &str, id: &str) -> String {
    template.replace("{id}", id)
}

/// 异步任务状态（解析后的三态）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TaskState {
    Pending,
    Succeeded,
    Failed,
}

/// 解析状态字段。缺状态键时按「仍在进行」处理 —— 由轮询预算兜底，
/// 不把「字段缺失」当成成功或失败。
fn classify_status(v: &Value) -> TaskState {
    let mut raw = None;
    for scope in [Some(v), v.get("data"), v.get("output"), v.get("result")] {
        let Some(obj) = scope else { continue };
        for key in ["status", "state", "task_status", "taskStatus"] {
            if let Some(s) = obj.get(key).and_then(Value::as_str) {
                raw = Some(s);
                break;
            }
        }
        if raw.is_some() {
            break;
        }
    }
    let Some(status) = raw.map(|s| s.trim().to_ascii_lowercase()) else {
        return TaskState::Pending;
    };
    match status.as_str() {
        "succeeded" | "success" | "completed" | "complete" | "done" | "finished" | "ready" => {
            TaskState::Succeeded
        }
        "failed" | "failure" | "error" | "cancelled" | "canceled" | "rejected" | "expired"
        | "blocked" => TaskState::Failed,
        _ => TaskState::Pending,
    }
}

/// 失败原因（只取一小段，避免把整包响应糊到用户脸上）。
fn failure_reason(v: &Value) -> String {
    if let Some(e) = v.get("error") {
        if let Some(s) = e.as_str() {
            return truncate(s.trim(), 300);
        }
        if let Some(s) = e.get("message").and_then(Value::as_str) {
            return truncate(s.trim(), 300);
        }
    }
    for key in ["message", "detail", "failure_reason", "reason"] {
        if let Some(s) = v.get(key).and_then(Value::as_str).map(str::trim) {
            if !s.is_empty() {
                return truncate(s, 300);
            }
        }
    }
    truncate(&v.to_string(), 300)
}

/// 轮询参数（毫秒间隔、总预算秒、查询方法），全部有界，防止模型给出离谱值。
fn poll_settings(ep: &MediaEndpoint, extra: &HashMap<String, String>) -> (u64, u64, String) {
    let pick = |key: &str| -> Option<String> {
        extra
            .get(key)
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .or_else(|| {
                ep.params
                    .as_ref()
                    .and_then(|p| p.get(key))
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_string)
            })
    };
    let interval = pick("poll_interval_ms")
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(5000)
        .clamp(500, 30_000);
    let timeout = pick("poll_timeout_s")
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(480)
        .clamp(1, 900);
    let method = pick("status_method")
        .map(|s| s.to_ascii_uppercase())
        .filter(|s| s == "GET" || s == "POST")
        .unwrap_or_else(|| "GET".to_string());
    (interval, timeout, method)
}

/// 把档案里的自定义参数与工具显式参数合并进请求体。
///
/// 优先级（低 → 高）：内置最简默认 < 档案自定义参数 < 本次工具调用参数。
fn build_body(ep: &MediaEndpoint, prompt: &str, extra: &HashMap<String, String>) -> Value {
    let mut body = json!({
        "model": ep.model,
        "prompt": prompt,
    });
    let merge = |body: &mut Value, src: &Value| {
        if let (Some(dst), Some(from)) = (body.as_object_mut(), src.as_object()) {
            for (k, v) in from {
                dst.insert(k.clone(), v.clone());
            }
        }
    };
    if let Some(p) = &ep.params {
        merge(&mut body, p);
    }
    // 工具显式参数：只接受白名单键，避免模型往请求体里塞任意字段。
    for key in [
        "duration",
        "size",
        "resolution",
        "aspect_ratio",
        "fps",
        "seed",
        "negative_prompt",
        "image",
        "response_format",
    ] {
        if let Some(v) = extra.get(key).map(|s| s.trim()).filter(|s| !s.is_empty()) {
            body[key] = match key {
                "duration" | "fps" | "seed" => json_number_or_text(v),
                _ => json!(v),
            };
        }
    }
    body
}

/// 数值型参数尽量发数字（多数服务商只认数字），解析不了就原样发字符串。
fn json_number_or_text(v: &str) -> Value {
    if let Ok(n) = v.parse::<i64>() {
        json!(n)
    } else if let Ok(f) = v.parse::<f64>() {
        json!(f)
    } else {
        json!(v)
    }
}

/// `video_generate` 工具。
pub struct VideoGenerateTool {
    workspace: RootSet,
    transport: Arc<dyn VideoTransport>,
}

impl VideoGenerateTool {
    pub fn new(workspace: impl Into<RootSet>, transport: Arc<dyn VideoTransport>) -> Self {
        let workspace: RootSet = workspace.into();
        Self {
            workspace,
            transport,
        }
    }

    pub fn with_http(workspace: impl Into<RootSet>) -> Self {
        Self::new(workspace, Arc::new(HttpVideoTransport))
    }

    /// 异步任务轮询：上游只回任务 id 时用。
    ///
    /// 返回 `(b64, url, 查询次数)`；失败返回 `(错误码, 用户可见文案)`。
    /// 单独抽成方法是为了能**不依赖模型档案**做单测（`execute` 会先解析能力，
    /// 测试环境里没有档案就跑不到轮询）。
    async fn poll_task(
        &self,
        ep: &MediaEndpoint,
        task_id: &str,
        args: &HashMap<String, String>,
    ) -> std::result::Result<(Option<String>, Option<String>, u32), (String, String)> {
        let Some(template) = status_url_template(ep, args) else {
            return Err((
                "VIDEO_ASYNC_NO_STATUS_URL".to_string(),
                format!(
                    "上游返回了异步任务（id={task_id}），但推不出状态查询地址。\n\
                     生成地址 `{}` 不以 `/generations` 结尾，档案里也没有 `status_url`。\n\
                     修复方式：设置 → 模型服务 → 选中该档案 → 多模态能力 → 视频的额外参数填\n\
                     {{\"status_url\":\"<查询地址，用 {{id}} 占位>\",\"poll_interval_ms\":5000}}\n\
                     例如 \"https://api.example.com/v1/videos/{{id}}\"。",
                    ep.url
                ),
            ));
        };
        if !template.contains("{id}") {
            return Err((
                "VIDEO_ASYNC_BAD_STATUS_URL".to_string(),
                format!(
                    "status_url 必须包含 `{{id}}` 占位符（当前是 `{template}`），\
                     否则每次轮询都会打到同一个地址、永远拿不到这次任务的结果。"
                ),
            ));
        }
        let poll_url = expand_status_url(&template, task_id);
        let (interval_ms, timeout_s, method) = poll_settings(ep, args);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(timeout_s);
        let mut attempts = 0u32;
        loop {
            attempts += 1;
            let state = match self.transport.status(ep, &poll_url, &method).await {
                Ok(v) => v,
                Err(e) => {
                    return Err((
                        "VIDEO_ASYNC_STATUS_FAILED".to_string(),
                        format!(
                            "异步任务已提交（id={task_id}），但第 {attempts} 次状态查询失败：{e}\n\
                             查询地址：{poll_url}\n\
                             任务可能仍在执行，可稍后用同一 id 手动查询。"
                        ),
                    ))
                }
            };
            match classify_status(&state) {
                TaskState::Succeeded => match parse_video(&state) {
                    Ok((b64, url)) => return Ok((b64, url, attempts)),
                    Err(e) => {
                        return Err((
                            "VIDEO_ASYNC_BAD_RESULT".to_string(),
                            format!(
                                "任务已标记完成，但结果里取不到视频地址：{e}\n原始响应：{}",
                                truncate(&state.to_string(), 400)
                            ),
                        ))
                    }
                },
                TaskState::Failed => {
                    return Err((
                        "VIDEO_ASYNC_TASK_FAILED".to_string(),
                        format!(
                            "视频生成任务失败（id={task_id}，第 {attempts} 次查询）：{}",
                            failure_reason(&state)
                        ),
                    ))
                }
                TaskState::Pending => {
                    if std::time::Instant::now() >= deadline {
                        return Err((
                            "VIDEO_ASYNC_TIMEOUT".to_string(),
                            format!(
                                "视频生成任务在 {timeout_s}s 内没有完成（id={task_id}，已查询 {attempts} 次）。\n\
                                 任务可能仍在执行：稍后可用同一 id 到 {poll_url} 查询，\
                                 或调大档案 video_params 的 poll_timeout_s（上限 900）。"
                            ),
                        ));
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(interval_ms)).await;
                }
            }
        }
    }
}

#[async_trait]
impl ToolExecutor for VideoGenerateTool {
    fn name(&self) -> &str {
        "video_generate"
    }

    fn description(&self) -> &str {
        "文本/图片生成视频（多模态·视频输出）。使用当前档案声明的视频模型，\
         调用 OpenAI 兼容的 videos/generations 端点，把结果保存到工作区并登记为产物。\
         prompt 必填；duration/size/resolution/aspect_ratio/fps/seed/negative_prompt/image 可选；\
         out 可指定文件名。未声明「视频输出」能力时会报错并给出配置指引（不会假装生成）。\
         兼容同步返回与异步任务：上游只回任务 id 时自动轮询状态地址（status_url 可含 {id} 占位），\
         默认每 5s 查一次、最多 480s，可用 poll_interval_ms / poll_timeout_s 覆盖。"
    }

    fn input_schema(&self) -> Value {
        json!({"type":"object","properties":{
            "prompt":{"type":"string","description":"视频描述（主体/动作/镜头运动/风格/氛围）"},
            "image":{"type":"string","description":"图生视频的首帧图片：URL 或 data URI（部分服务商支持）"},
            "duration":{"type":"number","description":"时长（秒），如 5 / 10；上限由上游决定"},
            "size":{"type":"string","description":"画幅尺寸，如 1280x720 / 720x1280"},
            "resolution":{"type":"string","description":"分辨率档位，如 720p / 1080p（部分服务商与 size 二选一）"},
            "aspect_ratio":{"type":"string","description":"宽高比，如 16:9 / 9:16 / 1:1"},
            "fps":{"type":"number","description":"帧率，如 24 / 30"},
            "seed":{"type":"number","description":"随机种子（复现同一结果时用，部分服务商支持）"},
            "negative_prompt":{"type":"string","description":"反向提示词（部分服务商支持）"},
            "out":{"type":"string","description":"输出文件名或相对路径（默认 generated/videos/<时间戳>.mp4）"},
            "status_url":{"type":"string","description":"异步任务的状态查询地址，用 {id} 占位；留空按 /generations → /{id} 推导"},
            "status_method":{"type":"string","description":"状态查询方法：GET（默认）或 POST"},
            "poll_interval_ms":{"type":"number","description":"轮询间隔毫秒（默认 5000，范围 500–30000）"},
            "poll_timeout_s":{"type":"number","description":"轮询总预算秒（默认 480，范围 10–900）"}
        },"required":["prompt"]})
    }

    async fn execute(&self, args: &HashMap<String, String>) -> Result<ToolResult> {
        let prompt = args
            .get("prompt")
            .map(|s| s.trim().to_string())
            .unwrap_or_default();
        if prompt.is_empty() {
            return Ok(ToolResult::Error {
                message: "video_generate 需要非空的 prompt".to_string(),
                code: "INVALID_PARAMS".to_string(),
            });
        }
        let ep = match media_capability::resolve(MediaKind::Video) {
            Ok(ep) => ep,
            Err(e) => {
                return Ok(ToolResult::Error {
                    message: e,
                    code: "NO_VIDEO_CAPABILITY".to_string(),
                })
            }
        };

        let body = build_body(&ep, &prompt, args);
        let resp = match self.transport.generate(&ep, &body).await {
            Ok(v) => v,
            Err(e) => {
                return Ok(ToolResult::Error {
                    message: format!(
                        "视频生成失败（档案 {}，端点 {}）：{e}",
                        ep.profile_id, ep.url
                    ),
                    code: "VIDEO_GENERATE_FAILED".to_string(),
                })
            }
        };

        // ① 同步形状：一次就拿到片子。② 只有任务 id：走「轮询 → 取片」。
        let mut poll_info: Option<(u32, String)> = None;
        let (b64, url) = match parse_video(&resp) {
            Ok(hit) => hit,
            Err(sync_err) => {
                let Some(task_id) = find_task_id(&resp) else {
                    return Ok(ToolResult::Error {
                        message: format!(
                            "{sync_err}\n（也没有找到任务 id，无法按异步任务轮询。）"
                        ),
                        code: "VIDEO_GENERATE_BAD_RESPONSE".to_string(),
                    });
                };
                match self.poll_task(&ep, &task_id, args).await {
                    Ok((b64, url, attempts)) => {
                        poll_info = Some((attempts, task_id));
                        (b64, url)
                    }
                    Err((code, message)) => return Ok(ToolResult::Error { message, code }),
                }
            }
        };

        let bytes = if let Some(b64) = b64 {
            match base64::engine::general_purpose::STANDARD.decode(b64.trim()) {
                Ok(b) => b,
                Err(e) => {
                    return Ok(ToolResult::Error {
                        message: format!("上游 b64_json 解码失败: {e}"),
                        code: "VIDEO_GENERATE_BAD_RESPONSE".to_string(),
                    })
                }
            }
        } else {
            match self
                .transport
                .download(url.as_deref().unwrap_or_default())
                .await
            {
                Ok(b) => b,
                Err(e) => {
                    // 下载失败也要如实回传 URL，让用户至少能手动取视频。
                    return Ok(ToolResult::Error {
                        message: format!("视频已生成但下载失败（{e}）；上游 URL：{url:?}"),
                        code: "VIDEO_DOWNLOAD_FAILED".to_string(),
                    });
                }
            }
        };

        let rel = args
            .get("out")
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| {
                format!(
                    "generated/videos/{}.{}",
                    chrono::Local::now().format("%Y%m%d-%H%M%S"),
                    sniff_ext(&bytes)
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
        if let Err(e) = std::fs::write(&target, &bytes) {
            return Ok(ToolResult::Error {
                message: format!("写入视频失败: {e}"),
                code: "IO_ERROR".to_string(),
            });
        }
        record_artifact("", &target.display().to_string(), GENERATED_KIND);

        Ok(ToolResult::Success {
            data: json!({
                "ok": true,
                "path": target.display().to_string(),
                "bytes": bytes.len(),
                "profile": ep.profile_id,
                "model": ep.model,
                "endpoint": ep.url,
                "prompt": prompt,
                // 异步任务：如实回传查询次数与任务 id，便于用户核对上游记录。
                "asyncTask": poll_info
                    .as_ref()
                    .map(|(attempts, id)| json!({"attempts": attempts, "taskId": id})),
            })
            .to_string(),
            metadata: HashMap::new(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ep() -> MediaEndpoint {
        MediaEndpoint {
            profile_id: "p".into(),
            provider: "openai".into(),
            model: "sora-2".into(),
            url: "https://x/v1/videos/generations".into(),
            api_key: "k".into(),
            params: None,
        }
    }

    #[test]
    fn body_defaults_and_whitelist() {
        let mut extra = HashMap::new();
        extra.insert("duration".to_string(), "8".to_string());
        extra.insert("evil".to_string(), "x".to_string());
        let b = build_body(&ep(), "a running cat", &extra);
        assert_eq!(b["model"], "sora-2");
        assert_eq!(b["prompt"], "a running cat");
        assert_eq!(b["duration"], 8, "数值型参数应发数字而非字符串");
        assert!(b.get("evil").is_none(), "非白名单键不得进入请求体");
        // 默认请求体只有 model + prompt，不塞任何供应商专属字段。
        let plain = build_body(&ep(), "x", &HashMap::new());
        assert_eq!(
            plain.as_object().map(|o| o.len()),
            Some(2),
            "默认请求体应只有 model 与 prompt：{plain}"
        );
    }

    #[test]
    fn profile_params_override_defaults() {
        let mut e = ep();
        e.params = Some(json!({"resolution":"1080p","seed":7,"duration":5}));
        let b = build_body(&e, "x", &HashMap::new());
        assert_eq!(b["resolution"], "1080p");
        assert_eq!(b["seed"], 7);
        assert_eq!(b["duration"], 5);
    }

    #[test]
    fn tool_args_beat_profile_params() {
        let mut e = ep();
        e.params = Some(json!({"duration":5,"aspect_ratio":"1:1"}));
        let mut extra = HashMap::new();
        extra.insert("duration".to_string(), "10".to_string());
        let b = build_body(&e, "x", &extra);
        assert_eq!(b["duration"], 10);
        assert_eq!(b["aspect_ratio"], "1:1", "未显式覆盖的档案参数应保留");
    }

    #[test]
    fn parses_all_known_response_shapes() {
        // ① OpenAI 兼容 URL
        let v = json!({"data":[{"url":"https://x/a.mp4"}]});
        assert_eq!(
            parse_video(&v).unwrap().1.as_deref(),
            Some("https://x/a.mp4")
        );
        // ② b64_json
        let v = json!({"data":[{"b64_json":"AAA"}]});
        assert_eq!(parse_video(&v).unwrap().0.as_deref(), Some("AAA"));
        // ③ 顶层 video_url
        let v = json!({"video_url":"https://x/b.mp4"});
        assert_eq!(
            parse_video(&v).unwrap().1.as_deref(),
            Some("https://x/b.mp4")
        );
        // ④ output.video_url
        let v = json!({"output":{"video_url":"https://x/c.mp4"}});
        assert_eq!(
            parse_video(&v).unwrap().1.as_deref(),
            Some("https://x/c.mp4")
        );
        // 取不到时用错误码路径，并把原始响应带出来
        let err = parse_video(&json!({"task_id":"t-1"})).unwrap_err();
        assert!(err.contains("task_id"), "错误信息应携带原始响应: {err}");
        assert!(parse_video(&json!({"data":[]})).is_err());
    }

    #[test]
    fn sniffs_video_extensions() {
        assert_eq!(sniff_ext(b"\x00\x00\x00\x18ftypisom"), "mp4");
        assert_eq!(sniff_ext(b"\x1aE\xdf\xa3\x01\x00"), "webm");
        assert_eq!(sniff_ext(b"RIFF0000AVI LIST"), "avi");
        assert_eq!(sniff_ext(b"????"), "mp4", "认不出时兜底 mp4");
    }

    /// 一旦被调用就 panic 的通道：用来证明空 prompt 不会走到网络层。
    struct PanicTransport;

    #[async_trait]
    impl VideoTransport for PanicTransport {
        async fn generate(&self, _ep: &MediaEndpoint, _body: &Value) -> Result<Value, String> {
            panic!("空 prompt 不得发起生成请求");
        }
        async fn status(&self, _ep: &MediaEndpoint, _u: &str, _m: &str) -> Result<Value, String> {
            panic!("空 prompt 不得查询状态");
        }
        async fn download(&self, _url: &str) -> Result<Vec<u8>, String> {
            panic!("空 prompt 不得下载");
        }
    }

    /// 空 prompt 必须在**解析能力 / 联网之前**被拒，绝不空跑一次生成。
    #[tokio::test]
    async fn empty_prompt_is_rejected_without_calling_upstream() {
        let tool = VideoGenerateTool::new(std::env::temp_dir(), Arc::new(PanicTransport));
        let mut args = HashMap::new();
        args.insert("prompt".to_string(), "   ".to_string());
        match tool.execute(&args).await.unwrap() {
            ToolResult::Error { code, .. } => assert_eq!(code, "INVALID_PARAMS"),
            other => panic!("空 prompt 应返回 INVALID_PARAMS，实际：{other:?}"),
        }
    }

    // ── 异步任务（提交 → 轮询 → 取片） ─────────────────────────────────────

    #[test]
    fn finds_task_ids_in_all_known_shapes() {
        assert_eq!(find_task_id(&json!({"id":"a"})).as_deref(), Some("a"));
        assert_eq!(find_task_id(&json!({"task_id":"b"})).as_deref(), Some("b"));
        assert_eq!(find_task_id(&json!({"requestId":"c"})).as_deref(), Some("c"));
        assert_eq!(
            find_task_id(&json!({"data":{"id":"d"}})).as_deref(),
            Some("d")
        );
        assert_eq!(
            find_task_id(&json!({"operation":{"name":"operations/e"}})).as_deref(),
            Some("operations/e")
        );
        // 空白 id 不算命中，避免拿着空串去轮询
        assert_eq!(find_task_id(&json!({"id":"   "})), None);
        assert_eq!(find_task_id(&json!({"data":[{"url":"u"}]})), None);
    }

    #[test]
    fn derives_and_overrides_status_url_template() {
        // 默认按 OpenAI 约定：/generations → /{id}
        assert_eq!(
            status_url_template(&ep(), &HashMap::new()).as_deref(),
            Some("https://x/v1/videos/{id}")
        );
        // 工具参数最优先
        let mut extra = HashMap::new();
        extra.insert(
            "status_url".to_string(),
            "https://api.example.com/tasks/{id}".to_string(),
        );
        assert_eq!(
            status_url_template(&ep(), &extra).as_deref(),
            Some("https://api.example.com/tasks/{id}")
        );
        // 档案参数次之
        let mut e = ep();
        e.params = Some(json!({"status_url":"https://p.example.com/j/{id}"}));
        assert_eq!(
            status_url_template(&e, &HashMap::new()).as_deref(),
            Some("https://p.example.com/j/{id}")
        );
        // 推不出时返回 None（不猜）
        let mut odd = ep();
        odd.url = "https://x/custom/video".into();
        assert_eq!(status_url_template(&odd, &HashMap::new()), None);
        assert_eq!(
            expand_status_url("https://x/videos/{id}", "t-9"),
            "https://x/videos/t-9"
        );
    }

    #[test]
    fn classifies_status_words_and_reasons() {
        for s in ["completed", "succeeded", "SUCCESS", "done", "ready"] {
            assert_eq!(classify_status(&json!({"status": s})), TaskState::Succeeded, "{s}");
        }
        for s in ["failed", "error", "cancelled", "expired"] {
            assert_eq!(classify_status(&json!({"status": s})), TaskState::Failed, "{s}");
        }
        for s in ["queued", "in_progress", "running", "pending"] {
            assert_eq!(classify_status(&json!({"status": s})), TaskState::Pending, "{s}");
        }
        // 嵌套 data.status 也要认
        assert_eq!(
            classify_status(&json!({"data":{"status":"completed"}})),
            TaskState::Succeeded
        );
        // 没有状态键 → 仍在进行（由轮询预算兜底，不误判成功）
        assert_eq!(classify_status(&json!({"foo":1})), TaskState::Pending);
        assert!(failure_reason(&json!({"error":{"message":"额度不足"}})).contains("额度不足"));
        assert!(failure_reason(&json!({"message":"被拒"})).contains("被拒"));
    }

    #[test]
    fn poll_settings_are_bounded() {
        let mut extra = HashMap::new();
        extra.insert("poll_interval_ms".into(), "10".into());
        extra.insert("poll_timeout_s".into(), "99999".into());
        extra.insert("status_method".into(), "post".into());
        let (i, t, m) = poll_settings(&ep(), &extra);
        assert_eq!(i, 500, "间隔下限 500ms");
        assert_eq!(t, 900, "预算上限 900s");
        assert_eq!(m, "POST", "方法大小写不敏感");
        // 非法方法退回 GET，不把错误方法发出去
        let mut bad = HashMap::new();
        bad.insert("status_method".into(), "DELETE".into());
        assert_eq!(poll_settings(&ep(), &bad).2, "GET");
        assert_eq!(poll_settings(&ep(), &HashMap::new()), (5000, 480, "GET".to_string()));
    }

    /// 第一次 queued、第二次 completed 的假通道（同时证明「没有提前取片」）。
    struct AsyncTransport {
        polls: std::sync::atomic::AtomicU32,
    }

    #[async_trait]
    impl VideoTransport for AsyncTransport {
        async fn generate(&self, _ep: &MediaEndpoint, _body: &Value) -> Result<Value, String> {
            Ok(json!({"id":"t-1","status":"queued"}))
        }
        async fn status(&self, _ep: &MediaEndpoint, url: &str, _m: &str) -> Result<Value, String> {
            assert!(url.ends_with("/t-1"), "状态地址必须替换任务 id: {url}");
            let n = self
                .polls
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if n == 0 {
                Ok(json!({"status":"in_progress"}))
            } else {
                Ok(json!({"status":"completed","data":[{"url":"https://x/done.mp4"}]}))
            }
        }
        async fn download(&self, _url: &str) -> Result<Vec<u8>, String> {
            Ok(b"\x00\x00\x00\x18ftypisomfake".to_vec())
        }
    }

    #[tokio::test]
    async fn async_task_polls_until_completed() {
        let tool = VideoGenerateTool::new(
            std::env::temp_dir(),
            Arc::new(AsyncTransport {
                polls: std::sync::atomic::AtomicU32::new(0),
            }),
        );
        let mut args = HashMap::new();
        args.insert("poll_interval_ms".into(), "500".into());
        let (b64, url, attempts) = tool.poll_task(&ep(), "t-1", &args).await.unwrap();
        assert_eq!(attempts, 2, "第一次 in_progress、第二次 completed");
        assert!(b64.is_none(), "完成态给的是 URL 不是 b64");
        assert_eq!(url.as_deref(), Some("https://x/done.mp4"));
    }

    /// 永远 pending：必须在预算耗尽后如实报超时并回传任务 id。
    struct ForeverPending;

    #[async_trait]
    impl VideoTransport for ForeverPending {
        async fn generate(&self, _ep: &MediaEndpoint, _b: &Value) -> Result<Value, String> {
            Ok(json!({"task_id":"t-forever"}))
        }
        async fn status(&self, _ep: &MediaEndpoint, _u: &str, _m: &str) -> Result<Value, String> {
            Ok(json!({"status":"pending"}))
        }
        async fn download(&self, _url: &str) -> Result<Vec<u8>, String> {
            panic!("超时场景不得下载");
        }
    }

    #[tokio::test]
    async fn async_task_timeout_is_reported_with_task_id() {
        let tool = VideoGenerateTool::new(std::env::temp_dir(), Arc::new(ForeverPending));
        let mut args = HashMap::new();
        args.insert("poll_interval_ms".into(), "500".into());
        args.insert("poll_timeout_s".into(), "1".into());
        let err = tool.poll_task(&ep(), "t-forever", &args).await.unwrap_err();
        assert_eq!(err.0, "VIDEO_ASYNC_TIMEOUT");
        assert!(err.1.contains("t-forever"), "必须回传任务 id: {}", err.1);
    }

    /// 推不出状态地址时必须在提交后就停下，而不是轮询一个猜出来的地址。
    #[tokio::test]
    async fn async_task_without_status_url_is_rejected() {
        let tool = VideoGenerateTool::new(std::env::temp_dir(), Arc::new(ForeverPending));
        let mut odd = ep();
        odd.url = "https://x/custom/video".into();
        let err = tool.poll_task(&odd, "t-1", &HashMap::new()).await.unwrap_err();
        assert_eq!(err.0, "VIDEO_ASYNC_NO_STATUS_URL");
        assert!(err.1.contains("status_url"), "必须告诉用户配哪个字段: {}", err.1);
    }
}
