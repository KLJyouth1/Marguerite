//! 音频工具：`audio_transcribe`（语音 → 文本）与 `audio_speak`（文本 → 语音）。
//!
//! # 为什么需要（2026-10-23）
//!
//! 盘点工具面时发现：`vis.audio` 在能力台账里是**纯叙事**（零实现），
//! 于是「把这段录音转成文字」「把这段话读出来」两类请求主智能体只能拒绝。
//! 而这两件事在 2026 年都有稳定的落地路径，缺的是把它们接进工具面。
//!
//! # 三条真实路径（按优先级，全部可验证）
//!
//! | 顺序 | 路径 | 前提 |
//! |---|---|---|
//! | 1 | 云端 OpenAI 兼容 ASR/TTS | 档案里勾选了「音频输入 / 语音输出」并配好密钥 |
//! | 2 | 本地 whisper CLI（whisper.cpp / faster-whisper） | PATH 里有可执行文件 + 模型权重 |
//! | 3 | Windows 系统语音（SAPI） | Windows + 已安装对应语言的语音包 |
//!
//! 三条都不成立时**明确报错**并列出缺什么，绝不返回空字符串假装成功。
//!
//! # 诚实边界
//!
//! - 云端的 multipart 请求体是手工拼的（不引入 reqwest `multipart` feature，
//!   避免为一个工具改动全局依赖特性）；字段名沿用 OpenAI 规范；
//! - Windows SAPI 的识别质量取决于系统语音包与音频采样率（16kHz 单声道 WAV 最稳）；
//! - whisper CLI 需要模型权重，本模块**不自动下载**（体积数百 MB，静默下载不可接受），
//!   只读 `MARGUERITE_WHISPER_MODEL` 或约定目录。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};

use crate::agent::tool_cycle::ToolResult;
use crate::mcp::registry::ToolExecutor;
use crate::model::media_capability::{self, MediaEndpoint, MediaKind};
use crate::proc::SilentCommand;
use crate::tools::file_change::{record_artifact, GENERATED_KIND};
use crate::tools::fs_path::RootSet;

/// 云通道（可注入：单测断言请求形状，不联网）。
#[async_trait]
pub trait AudioCloud: Send + Sync {
    /// 上传音频做转写，返回上游 JSON。
    async fn transcribe(&self, ep: &MediaEndpoint, file: &Path, language: Option<&str>) -> Result<Value, String>;
    /// 合成语音，返回音频字节。
    async fn speak(&self, ep: &MediaEndpoint, body: &Value) -> Result<Vec<u8>, String>;
}

pub struct HttpAudioCloud;

fn mime_of(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .as_deref()
    {
        Some("mp3") => "audio/mpeg",
        Some("wav") => "audio/wav",
        Some("m4a") => "audio/mp4",
        Some("ogg") | Some("oga") => "audio/ogg",
        Some("flac") => "audio/flac",
        Some("webm") => "audio/webm",
        _ => "application/octet-stream",
    }
}

/// 手工拼 multipart/form-data（文本字段 + 一个文件字段）。
fn multipart_body(boundary: &str, fields: &[(&str, &str)], file: &Path, bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len() + 512);
    for (k, v) in fields {
        out.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
        out.extend_from_slice(
            format!("Content-Disposition: form-data; name=\"{k}\"\r\n\r\n").as_bytes(),
        );
        out.extend_from_slice(v.as_bytes());
        out.extend_from_slice(b"\r\n");
    }
    let filename = file
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("audio.bin");
    out.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
    out.extend_from_slice(
        format!(
            "Content-Disposition: form-data; name=\"file\"; filename=\"{filename}\"\r\nContent-Type: {}\r\n\r\n",
            mime_of(file)
        )
        .as_bytes(),
    );
    out.extend_from_slice(bytes);
    out.extend_from_slice(b"\r\n");
    out.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
    out
}

#[async_trait]
impl AudioCloud for HttpAudioCloud {
    async fn transcribe(
        &self,
        ep: &MediaEndpoint,
        file: &Path,
        language: Option<&str>,
    ) -> Result<Value, String> {
        let bytes = std::fs::read(file).map_err(|e| format!("读取音频失败: {e}"))?;
        if bytes.len() > 25 * 1024 * 1024 {
            return Err(format!(
                "音频 {:.1} MB 超过多数云端接口的 25 MB 上限，请先切分或压缩",
                bytes.len() as f64 / 1024.0 / 1024.0
            ));
        }
        let boundary = format!("----MargueriteBoundary{}", uuid::Uuid::new_v4().simple());
        let mut fields: Vec<(&str, &str)> = vec![("model", ep.model.as_str())];
        if let Some(lang) = language.map(str::trim).filter(|s| !s.is_empty()) {
            fields.push(("language", lang));
        }
        let body = multipart_body(&boundary, &fields, file, &bytes);
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(300))
            .build()
            .map_err(|e| format!("构造 HTTP 客户端失败: {e}"))?;
        let mut req = client
            .post(&ep.url)
            .header(
                "Content-Type",
                format!("multipart/form-data; boundary={boundary}"),
            )
            .body(body);
        if !ep.api_key.is_empty() {
            req = req.header("Authorization", format!("Bearer {}", ep.api_key));
        }
        let resp = req.send().await.map_err(|e| format!("请求失败: {e}"))?;
        let status = resp.status();
        let text = resp.text().await.map_err(|e| format!("读取响应失败: {e}"))?;
        if !status.is_success() {
            return Err(format!("上游返回 {}：{}", status.as_u16(), tail(&text, 600)));
        }
        serde_json::from_str(&text)
            .map_err(|e| format!("上游返回非 JSON（{e}）：{}", tail(&text, 300)))
    }

    async fn speak(&self, ep: &MediaEndpoint, body: &Value) -> Result<Vec<u8>, String> {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(180))
            .build()
            .map_err(|e| format!("构造 HTTP 客户端失败: {e}"))?;
        let mut req = client.post(&ep.url).json(body);
        if !ep.api_key.is_empty() {
            req = req.header("Authorization", format!("Bearer {}", ep.api_key));
        }
        let resp = req.send().await.map_err(|e| format!("请求失败: {e}"))?;
        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(format!("上游返回 {}：{}", status.as_u16(), tail(&text, 600)));
        }
        resp.bytes()
            .await
            .map(|b| b.to_vec())
            .map_err(|e| format!("读取音频流失败: {e}"))
    }
}

fn tail(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        let t: String = s.chars().skip(s.chars().count() - n).collect();
        format!("…{t}")
    }
}

/// 允许作为音频输入的根目录：全部工作区根、临时目录、应用配置目录（附件与产物都在这里）。
///
/// 不做「任意绝对路径」：ASR 会把音频内容变成文本回给模型，
/// 等于开了一条「读任意本地音频」的旁路。只放行这三处，覆盖所有真实场景。
fn allowed_roots(workspace: &RootSet) -> Vec<PathBuf> {
    let mut roots = workspace.all().to_vec();
    roots.extend([std::env::temp_dir(), crate::persistence::config::Config::config_dir()]);
    roots
}

/// 解析音频输入路径（相对路径按工作区解析；绝对路径必须落在白名单根目录内）。
fn resolve_audio_input(workspace: &RootSet, raw: &str) -> Result<PathBuf, String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Err("缺少音频路径".to_string());
    }
    let p = PathBuf::from(raw);
    if p.is_absolute() {
        let canon = canon_of(&p)?;
        for root in allowed_roots(workspace) {
            let root_c = canon_of(&root).unwrap_or(root);
            if canon.starts_with(&root_c) {
                if !canon.is_file() {
                    return Err(format!("文件不存在: {}", canon.display()));
                }
                return Ok(canon);
            }
        }
        return Err(format!(
            "出于安全考虑，只能读取工作区 / 临时目录 / 应用数据目录内的音频；\
             请先复制到工作区再转写：{raw}"
        ));
    }
    let resolved = workspace.resolve(raw).map_err(|e| e.to_string())?;
    if !resolved.is_file() {
        return Err(format!("文件不存在: {}", resolved.display()));
    }
    Ok(resolved)
}

fn canon_of(p: &Path) -> Result<PathBuf, String> {
    std::fs::canonicalize(p).map_err(|e| format!("解析路径失败（{}）: {e}", p.display()))
}

/// 在 PATH 里找可执行文件（不引入 which crate）。
fn which(program: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let cand = dir.join(program);
        if cand.is_file() {
            return Some(cand);
        }
        #[cfg(windows)]
        {
            let exe = dir.join(format!("{program}.exe"));
            if exe.is_file() {
                return Some(exe);
            }
        }
    }
    None
}

/// 本地 whisper 可执行文件 + 模型权重（都没有则 None，由调用方继续降级）。
fn local_whisper() -> Option<(PathBuf, PathBuf)> {
    let bin = ["whisper-cli", "whisper", "main"]
        .iter()
        .find_map(|n| which(n))?;
    let model = std::env::var_os("MARGUERITE_WHISPER_MODEL")
        .map(PathBuf::from)
        .filter(|p| p.is_file())
        .or_else(|| {
            let p = crate::persistence::config::Config::config_dir()
                .join("models")
                .join("whisper")
                .join("ggml-base.bin");
            p.is_file().then_some(p)
        })?;
    Some((bin, model))
}

/// 跑一次 PowerShell，返回 (成功?, stdout, stderr)。
fn run_powershell(script: &str) -> Result<(bool, String, String), String> {
    let mut cmd = std::process::Command::new("powershell");
    cmd.silent();
    cmd.args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-Command", script]);
    let out = cmd.output().map_err(|e| format!("启动 PowerShell 失败: {e}"))?;
    Ok((
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).trim().to_string(),
        String::from_utf8_lossy(&out.stderr).trim().to_string(),
    ))
}

/// 单引号转义（PS 字面量）。
fn ps_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

// ---------------------------------------------------------------------------
// 转写工具
// ---------------------------------------------------------------------------

pub struct AudioTranscribeTool {
    workspace: RootSet,
    cloud: Arc<dyn AudioCloud>,
}

impl AudioTranscribeTool {
    pub fn new(workspace: impl Into<RootSet>, cloud: Arc<dyn AudioCloud>) -> Self {
        Self { workspace: workspace.into(), cloud }
    }
    pub fn with_http(workspace: impl Into<RootSet>) -> Self {
        Self::new(workspace, Arc::new(HttpAudioCloud))
    }
}

#[async_trait]
impl ToolExecutor for AudioTranscribeTool {
    fn name(&self) -> &str {
        "audio_transcribe"
    }

    fn description(&self) -> &str {
        "把音频/录音转成文字（多模态·音频输入）。路径顺序：①档案声明「音频输入」→ 云端 ASR；\
         ②PATH 里有 whisper-cli 且设置了 MARGUERITE_WHISPER_MODEL → 本地转写；\
         ③Windows 系统语音识别（SAPI）。三条都不成立时明确报错并说明缺什么。\
         支持 wav/mp3/m4a/ogg/flac/webm；云端上限 25 MB。"
    }

    fn input_schema(&self) -> Value {
        json!({"type":"object","properties":{
            "path":{"type":"string","description":"音频文件路径（相对工作区，或工作区/临时目录/应用数据目录内的绝对路径）"},
            "language":{"type":"string","description":"语言代码（zh / en …），留空自动检测"},
            "out":{"type":"string","description":"转写结果另存为文本文件（相对工作区）"}
        },"required":["path"]})
    }

    async fn execute(&self, args: &HashMap<String, String>) -> Result<ToolResult> {
        let raw = args.get("path").cloned().unwrap_or_default();
        let file = match resolve_audio_input(&self.workspace, &raw) {
            Ok(p) => p,
            Err(e) => {
                return Ok(ToolResult::Error {
                    message: e,
                    code: "INVALID_PARAMS".to_string(),
                })
            }
        };
        let language = args.get("language").cloned().unwrap_or_default();

        // ① 云端
        if let Ok(ep) = media_capability::resolve(MediaKind::Transcription) {
            match self
                .cloud
                .transcribe(&ep, &file, Some(language.as_str()))
                .await
            {
                Ok(v) => {
                    let text = v
                        .get("text")
                        .and_then(Value::as_str)
                        .map(str::trim)
                        .unwrap_or_default()
                        .to_string();
                    if text.is_empty() {
                        return Ok(ToolResult::Error {
                            message: format!(
                                "云端转写返回空文本（档案 {}）：{}",
                                ep.profile_id,
                                tail(&v.to_string(), 300)
                            ),
                            code: "ASR_EMPTY".to_string(),
                        });
                    }
                    return Ok(self.finish(&text, "cloud", &ep.profile_id, args));
                }
                Err(e) => {
                    // 云端失败继续走本地路径，但把原因带进最终报告（不静默吞）。
                    let fallback = self.local_paths(&file, &language).await;
                    return Ok(match fallback {
                        Ok((text, via)) => self.finish(
                            &text,
                            &via,
                            &format!("云端失败已降级：{e}"),
                            args,
                        ),
                        Err(local_err) => ToolResult::Error {
                            message: format!(
                                "云端转写失败（{e}）；本机本地识别也不可用：{local_err}"
                            ),
                            code: "ASR_FAILED".to_string(),
                        },
                    });
                }
            }
        }

        // ②③ 本地
        match self.local_paths(&file, &language).await {
            Ok((text, via)) => Ok(self.finish(&text, &via, "", args)),
            Err(local_err) => Ok(ToolResult::Error {
                message: format!(
                    "没有可用的音频转写通道。\n\
                     ① 云端：没有任何档案勾选「音频输入」（设置 → 模型服务 → 多模态能力）；\n\
                     ②③ 本机：{local_err}"
                ),
                code: "NO_ASR".to_string(),
            }),
        }
    }
}

impl AudioTranscribeTool {
    fn finish(
        &self,
        text: &str,
        via: &str,
        note: &str,
        args: &HashMap<String, String>,
    ) -> ToolResult {
        let mut payload = json!({
            "ok": true,
            "text": text,
            "via": via,
            "chars": text.chars().count(),
        });
        if !note.is_empty() {
            payload["note"] = json!(note);
        }
        if let Some(rel) = args.get("out").map(|s| s.trim()).filter(|s| !s.is_empty()) {
            match self.workspace.resolve(rel) {
                Ok(target) => {
                    if let Some(dir) = target.parent() {
                        let _ = std::fs::create_dir_all(dir);
                    }
                    match std::fs::write(&target, text) {
                        Ok(()) => {
                            record_artifact("", &target.display().to_string(), GENERATED_KIND);
                            payload["saved_to"] = json!(target.display().to_string());
                        }
                        Err(e) => payload["save_error"] = json!(e.to_string()),
                    }
                }
                Err(e) => payload["save_error"] = json!(e.to_string()),
            }
        }
        ToolResult::Success {
            data: payload.to_string(),
            metadata: HashMap::new(),
        }
    }

    /// 本地两条路径（whisper CLI → Windows SAPI）。
    ///
    /// 失败时把**每条路径的具体原因**拼起来返回，用户才知道缺的是二进制、
    /// 模型权重还是语音包——「不可用」三个字对排查毫无帮助。
    async fn local_paths(&self, file: &Path, language: &str) -> Result<(String, String), String> {
        let mut reasons: Vec<String> = Vec::new();
        if let Some((bin, model)) = local_whisper() {
            let out_base = std::env::temp_dir().join(format!(
                "marguerite-asr-{}",
                uuid::Uuid::new_v4().simple()
            ));
            let mut cmd = std::process::Command::new(&bin);
            cmd.silent();
            cmd.args([
                "-m",
                &model.display().to_string(),
                "-f",
                &file.display().to_string(),
                "-otxt",
                "-of",
                &out_base.display().to_string(),
            ]);
            if !language.trim().is_empty() {
                cmd.args(["-l", language.trim()]);
            }
            match cmd.output() {
                Ok(out) if out.status.success() => {
                    let txt = out_base.with_extension("txt");
                    let text = std::fs::read_to_string(&txt).unwrap_or_default();
                    let _ = std::fs::remove_file(&txt);
                    let text = text.trim().to_string();
                    if !text.is_empty() {
                        return Ok((text, "whisper-cli".to_string()));
                    }
                    reasons.push(format!(
                        "whisper-cli 执行成功但输出为空（{}）",
                        bin.display()
                    ));
                }
                Ok(out) => reasons.push(format!(
                    "whisper-cli 退出码 {:?}：{}",
                    out.status.code(),
                    tail(&String::from_utf8_lossy(&out.stderr), 200)
                )),
                Err(e) => reasons.push(format!("whisper-cli 启动失败: {e}")),
            }
        } else {
            reasons.push(
                "PATH 里没有 whisper-cli/whisper，或未设置 MARGUERITE_WHISPER_MODEL 指向 ggml 模型"
                    .to_string(),
            );
        }

        match self.windows_sapi(file).await {
            Ok(text) => Ok((text, "windows-sapi".to_string())),
            Err(e) => {
                reasons.push(format!("Windows 系统语音: {e}"));
                Err(reasons.join("；"))
            }
        }
    }

    /// Windows 系统语音识别（离线，需语音包；只吃 WAV）。
    async fn windows_sapi(&self, file: &Path) -> Result<String, String> {
        if !cfg!(target_os = "windows") {
            return Err("非 Windows 平台".to_string());
        }
        if file
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| !e.eq_ignore_ascii_case("wav"))
            .unwrap_or(true)
        {
            // SAPI 只认 WAV；不猜转码（ffmpeg 可能不存在）。
            return Err("系统语音识别只接受 WAV，请先转成 WAV".to_string());
        }
        let script = format!(
            "$ErrorActionPreference='Stop';\
             Add-Type -AssemblyName System.Speech;\
             $r=New-Object System.Speech.Recognition.SpeechRecognitionEngine;\
             $r.LoadGrammar((New-Object System.Speech.Recognition.DictationGrammar));\
             $r.SetInputToWaveFile({});\
             $sb=New-Object System.Text.StringBuilder;\
             while($true){{$x=$r.Recognize();if($x -eq $null){{break}};[void]$sb.Append($x.Text);[void]$sb.Append(' ')}};\
             $r.Dispose();\
             $sb.ToString().Trim()",
            ps_quote(&file.display().to_string())
        );
        let (ok, stdout, stderr) = run_powershell(&script)?;
        if ok && !stdout.trim().is_empty() {
            Ok(stdout.trim().to_string())
        } else if ok {
            Err("识别结束但没有输出（音频无声/语音包语言不匹配）".to_string())
        } else if stderr.is_empty() {
            Err("系统语音识别执行失败（无错误输出，通常是没有安装语音包）".to_string())
        } else {
            Err(format!("系统语音识别失败：{}", tail(&stderr, 300)))
        }
    }
}

// ---------------------------------------------------------------------------
// 语音合成工具
// ---------------------------------------------------------------------------

pub struct AudioSpeakTool {
    workspace: RootSet,
    cloud: Arc<dyn AudioCloud>,
}

impl AudioSpeakTool {
    pub fn new(workspace: impl Into<RootSet>, cloud: Arc<dyn AudioCloud>) -> Self {
        Self { workspace: workspace.into(), cloud }
    }
    pub fn with_http(workspace: impl Into<RootSet>) -> Self {
        Self::new(workspace, Arc::new(HttpAudioCloud))
    }
}

/// 合成请求体：档案参数 > 内置默认。
fn speak_body(ep: &MediaEndpoint, text: &str, args: &HashMap<String, String>) -> Value {
    let mut body = json!({
        "model": ep.model,
        "input": text,
        "voice": "alloy",
        "response_format": "mp3",
    });
    if let Some(p) = &ep.params {
        if let (Some(dst), Some(from)) = (body.as_object_mut(), p.as_object()) {
            for (k, v) in from {
                // input 必须是被合成的文本，不能被档案参数覆盖
                if k != "input" {
                    dst.insert(k.clone(), v.clone());
                }
            }
        }
    }
    for key in ["voice", "response_format", "speed"] {
        if let Some(v) = args.get(key).map(|s| s.trim()).filter(|s| !s.is_empty()) {
            body[key] = json!(v);
        }
    }
    body
}

#[async_trait]
impl ToolExecutor for AudioSpeakTool {
    fn name(&self) -> &str {
        "audio_speak"
    }

    fn description(&self) -> &str {
        "文本转语音（多模态·语音输出）。①档案声明「语音输出」→ 云端 TTS（OpenAI 兼容 \
         /v1/audio/speech）；②否则用 Windows 系统语音（离线）。结果保存为音频文件并登记为产物。"
    }

    fn input_schema(&self) -> Value {
        json!({"type":"object","properties":{
            "text":{"type":"string","description":"要朗读的文本"},
            "voice":{"type":"string","description":"音色（如 alloy / echo / nova；SAPI 用系统音色名）"},
            "speed":{"type":"string","description":"语速（云端为倍率如 1.0；SAPI 为 -10..10）"},
            "out":{"type":"string","description":"输出文件名（默认 generated/audio/<时间戳>.mp3 或 .wav）"}
        },"required":["text"]})
    }

    async fn execute(&self, args: &HashMap<String, String>) -> Result<ToolResult> {
        let text = args.get("text").cloned().unwrap_or_default();
        if text.trim().is_empty() {
            return Ok(ToolResult::Error {
                message: "audio_speak 需要非空的 text".to_string(),
                code: "INVALID_PARAMS".to_string(),
            });
        }

        // ① 云端
        if let Ok(ep) = media_capability::resolve(MediaKind::Speech) {
            let body = speak_body(&ep, &text, args);
            match self.cloud.speak(&ep, &body).await {
                Ok(bytes) if !bytes.is_empty() => {
                    let ext = body["response_format"].as_str().unwrap_or("mp3");
                    return Ok(self.save(&bytes, ext, args, "cloud"));
                }
                Ok(_) => {
                    // 空字节继续走本地，但记下原因
                    tracing::warn!(profile = %ep.profile_id, "云端 TTS 返回空音频，降级到系统语音");
                }
                Err(e) => {
                    let local = self.local_speak(&text, args).await;
                    return Ok(match local {
                        Ok((bytes, ext)) => {
                            let mut r = self.save(&bytes, &ext, args, "windows-sapi");
                            if let ToolResult::Success { data, .. } = &mut r {
                                if let Ok(mut v) = serde_json::from_str::<Value>(data) {
                                    v["note"] = json!(format!("云端 TTS 失败已降级：{e}"));
                                    *data = v.to_string();
                                }
                            }
                            r
                        }
                        Err(local_err) => ToolResult::Error {
                            message: format!(
                                "云端 TTS 失败（{e}）；本机 Windows 系统语音也不可用（{local_err}）。\n\
                                 修复方式：设置 → 模型服务 → 多模态能力 → 勾选「语音输出」并配置密钥；\
                                 或在 Windows 设置 → 时间和语言 → 语音 → 添加语音包。"
                            ),
                            code: "TTS_FAILED".to_string(),
                        },
                    });
                }
            }
        }

        // ② Windows SAPI
        match self.local_speak(&text, args).await {
            Ok((bytes, ext)) => Ok(self.save(&bytes, &ext, args, "windows-sapi")),
            Err(e) => Ok(ToolResult::Error {
                message: format!(
                    "没有可用的语音合成通道。\n\
                     ① 云端：没有任何档案勾选「语音输出」（设置 → 模型服务 → 多模态能力）；\n\
                     ② Windows 系统语音：{e}"
                ),
                code: "NO_TTS".to_string(),
            }),
        }
    }
}

impl AudioSpeakTool {
    /// Windows SAPI 合成到 WAV 文件（离线，无需任何密钥）。
    ///
    /// 失败时返回**原因**而不是 None：本机实测「No voice installed on the system」
    /// 是最常见的失败（系统没装语音包），用户需要知道这句原文才知道去装语音。
    async fn local_speak(
        &self,
        text: &str,
        args: &HashMap<String, String>,
    ) -> Result<(Vec<u8>, String), String> {
        if !cfg!(target_os = "windows") {
            return Err("非 Windows 平台没有系统语音".to_string());
        }
        let tmp = std::env::temp_dir().join(format!(
            "marguerite-tts-{}.wav",
            uuid::Uuid::new_v4().simple()
        ));
        let mut script = String::from(
            "$ErrorActionPreference='Stop';Add-Type -AssemblyName System.Speech;\
             $s=New-Object System.Speech.Synthesis.SpeechSynthesizer;",
        );
        if let Some(voice) = args.get("voice").map(|s| s.trim()).filter(|s| !s.is_empty()) {
            script.push_str(&format!(
                "try{{$s.SelectVoice({})}}catch{{}};",
                ps_quote(voice)
            ));
        }
        if let Some(rate) = args.get("speed").and_then(|s| s.trim().parse::<i32>().ok()) {
            script.push_str(&format!("$s.Rate={};\n", rate.clamp(-10, 10)));
        }
        script.push_str(&format!(
            "$s.SetOutputToWaveFile({});$s.Speak({});$s.Dispose()",
            ps_quote(&tmp.display().to_string()),
            ps_quote(text)
        ));
        let (ok, _out, err) = run_powershell(&script)?;
        if !ok {
            let _ = std::fs::remove_file(&tmp);
            return Err(if err.is_empty() {
                "Windows 系统语音执行失败（无错误输出）".to_string()
            } else {
                format!("Windows 系统语音执行失败：{err}")
            });
        }
        let bytes = std::fs::read(&tmp).map_err(|e| format!("读取合成的 WAV 失败: {e}"))?;
        let _ = std::fs::remove_file(&tmp);
        if bytes.is_empty() {
            return Err("Windows 系统语音返回空音频".to_string());
        }
        Ok((bytes, "wav".to_string()))
    }

    fn save(&self, bytes: &[u8], ext: &str, args: &HashMap<String, String>, via: &str) -> ToolResult {
        let rel = args
            .get("out")
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| {
                format!(
                    "generated/audio/{}.{}",
                    chrono::Local::now().format("%Y%m%d-%H%M%S"),
                    ext.trim_start_matches('.')
                )
            });
        let target = match self.workspace.resolve(&rel) {
            Ok(p) => p,
            Err(e) => {
                return ToolResult::Error {
                    message: format!("输出路径非法（必须落在工作区内）: {e}"),
                    code: "INVALID_PATH".to_string(),
                }
            }
        };
        if let Some(dir) = target.parent() {
            if let Err(e) = std::fs::create_dir_all(dir) {
                return ToolResult::Error {
                    message: format!("创建输出目录失败: {e}"),
                    code: "IO_ERROR".to_string(),
                };
            }
        }
        if let Err(e) = std::fs::write(&target, bytes) {
            return ToolResult::Error {
                message: format!("写入音频失败: {e}"),
                code: "IO_ERROR".to_string(),
            };
        }
        record_artifact("", &target.display().to_string(), GENERATED_KIND);
        ToolResult::Success {
            data: json!({
                "ok": true,
                "path": target.display().to_string(),
                "bytes": bytes.len(),
                "via": via,
            })
            .to_string(),
            metadata: HashMap::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn multipart_has_both_parts_and_terminator() {
        let body = multipart_body(
            "BOUND",
            &[("model", "whisper-1")],
            Path::new("a.wav"),
            b"RIFFdata",
        );
        let s = String::from_utf8_lossy(&body);
        assert!(s.contains("name=\"model\""));
        assert!(s.contains("filename=\"a.wav\""));
        assert!(s.contains("Content-Type: audio/wav"));
        assert!(s.trim_end().ends_with("--BOUND--"));
    }

    #[test]
    fn mime_mapping() {
        assert_eq!(mime_of(Path::new("x.mp3")), "audio/mpeg");
        assert_eq!(mime_of(Path::new("x.wav")), "audio/wav");
        assert_eq!(mime_of(Path::new("x.unknown")), "application/octet-stream");
    }

    #[test]
    fn speak_body_defaults_and_override() {
        let ep = MediaEndpoint {
            profile_id: "p".into(),
            provider: "openai".into(),
            model: "tts-1".into(),
            url: "u".into(),
            api_key: "k".into(),
            params: Some(json!({"voice":"nova","input":"IGNORED"})),
        };
        let mut args = HashMap::new();
        args.insert("speed".to_string(), "1.5".to_string());
        let b = speak_body(&ep, "你好", &args);
        assert_eq!(b["voice"], "nova");
        assert_eq!(b["input"], "你好", "档案参数不得覆盖待合成文本");
        assert_eq!(b["speed"], "1.5");
    }

    #[test]
    fn relative_path_outside_workspace_rejected() {
        let ws = std::env::temp_dir().join("mq-audio-ws");
        let _ = std::fs::create_dir_all(&ws);
        let roots = RootSet::from(ws);
        assert!(resolve_audio_input(&roots, "../escape.wav").is_err());
        assert!(resolve_audio_input(&roots, "").is_err());
    }
}
