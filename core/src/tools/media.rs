//! 媒体工具（`media`）：ffmpeg / ffprobe 真实桥
//!
//! # 为什么需要
//!
//! 产品定位里的「视频与图片编辑」此前在 **Rust 工具层完全不存在**（`grep` 全仓
//! `ffmpeg|blender|video_|image_edit` 零命中），模型只能靠 `shell_exec` 手拼命令行 ——
//! 那等于把「转码参数正确性」和「命令注入面」一起交给模型。本模块把这条路径收成
//! **结构化动作 + 白名单参数**：模型只表达意图（探测 / 转码 / 抽帧 / 封面），
//! 命令行由这里拼。
//!
//! # 安全模型（与 `tools/git.rs` 同口径）
//!
//! - **不经过 shell**：`Command::new("ffmpeg").args([...])`，参数是 `Vec<String>`，
//!   天然免疫 shell 注入；
//! - **路径双向收口**：输入与输出都经 [`safe_resolve`] 解析到工作区内（越界即拒），
//!   避免「读任意文件 / 写任意位置」；
//! - **不暴露任意滤镜**：`-vf` 只由本模块按白名单拼装（`scale` / `fps`），
//!   不接受模型传入的滤镜表达式 —— 部分 ffmpeg 滤镜可读文件（如 `subtitles=`），
//!   放开等于绕过上面的路径收口；
//! - **超时显式**：转码属长任务，外层工具超时由 `ToolCall::default_timeout_for`
//!   放宽到 10 分钟（历史缺陷：统一 30s 会把视频/3D/Office 长任务直接杀掉）。
//!
//! # 未做（诚实边界）
//!
//! Blender 不在本模块：本机实测 `Program Files/Blender Foundation` 不存在、
//! `which blender` 为空，**没有可验证的运行时**，写一个无法端到端验证的桥
//! 违背「能跑通才算做完」。检测到 blender 后按同一模式加 `blender` 工具即可。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use serde_json::json;

use crate::agent::tool_cycle::ToolResult;
use crate::mcp::registry::ToolExecutor;
use crate::proc::SilentCommand;
use crate::tools::fs_path::RootSet;

/// 一次外部媒体命令的输出
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaOutput {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
}

/// 媒体命令执行器（可注入，便于单测断言 argv 而不真的起进程）
pub trait MediaRunner: Send + Sync {
    fn run(&self, program: &str, args: &[String], timeout: Duration) -> Result<MediaOutput>;
}

/// 生产实现：直接调 ffmpeg / ffprobe 二进制（Windows 下经 `SilentCommand`
/// 抑制控制台黑窗 —— GUI 宿主无控制台，子进程弹窗是可见缺陷）。
pub struct SystemMediaRunner;

impl MediaRunner for SystemMediaRunner {
    fn run(&self, program: &str, args: &[String], timeout: Duration) -> Result<MediaOutput> {
        // 内置优先：安装包自带 ffmpeg 时不再依赖用户 PATH（见 ffmpeg_bin 的优先级链）。
        let bin = crate::tools::ffmpeg_bin::resolve(program).unwrap_or_else(|| PathBuf::from(program));
        let mut cmd = Command::new(&bin);
        cmd.silent();
        cmd.args(args);
        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());
        let mut child = cmd
            .spawn()
            .map_err(|e| anyhow!("启动 {program} 失败：{e}"))?;
        // 阻塞式等待 + 超时轮询：`wait_timeout` 不在标准库，这里用最小实现。
        let started = std::time::Instant::now();
        loop {
            match child.try_wait() {
                Ok(Some(_)) => break,
                Ok(None) => {
                    if started.elapsed() > timeout {
                        let _ = child.kill();
                        let _ = child.wait();
                        return Err(anyhow!("TIMEOUT:{program} 超过 {}s 未完成", timeout.as_secs()));
                    }
                    std::thread::sleep(Duration::from_millis(80));
                }
                Err(e) => return Err(anyhow!("等待 {program} 失败：{e}")),
            }
        }
        let out = child
            .wait_with_output()
            .map_err(|e| anyhow!("读取 {program} 输出失败：{e}"))?;
        Ok(MediaOutput {
            exit_code: out.status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        })
    }
}

/// 支持的动作（白名单，fail-closed）
const ACTIONS: &[&str] = &["probe", "convert", "frames", "thumbnail"];

/// `media` 工具：`action` 驱动四种结构化操作。
pub struct MediaTool {
    workspace: RootSet,
    runner: Arc<dyn MediaRunner>,
}

impl MediaTool {
    pub fn new(workspace: impl Into<RootSet>, runner: Arc<dyn MediaRunner>) -> Self {
        let workspace: RootSet = workspace.into();
        Self { workspace, runner }
    }

    pub fn with_system_runner(workspace: impl Into<RootSet>) -> Self {
        Self::new(workspace, Arc::new(SystemMediaRunner))
    }

    /// 解析入参路径（工作区内）；缺失时给出可操作错误。
    fn resolve_arg(&self, args: &HashMap<String, String>, key: &str) -> Result<PathBuf, String> {
        let raw = args
            .get(key)
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .ok_or_else(|| format!("缺少参数 `{key}`"))?;
        self.workspace.resolve(raw).map_err(|e| format!("{e}"))
    }

    fn num_arg(args: &HashMap<String, String>, key: &str) -> Result<Option<f64>, String> {
        match args.get(key).map(|s| s.trim()).filter(|s| !s.is_empty()) {
            None => Ok(None),
            Some(v) => v
                .parse::<f64>()
                .map(Some)
                .map_err(|_| format!("参数 `{key}` 必须是数字，实际：{v}")),
        }
    }

    /// 组装 `convert` 的 ffmpeg 参数（纯函数，便于单测断言「没有注入面」）。
    fn convert_args(
        input: &Path,
        output: &Path,
        scale: Option<(u32, u32)>,
        fps: Option<f64>,
        crf: Option<u32>,
        start: Option<f64>,
        duration: Option<f64>,
    ) -> Vec<String> {
        let mut a: Vec<String> = vec!["-y".into()];
        if let Some(s) = start {
            a.push("-ss".into());
            a.push(format!("{s}"));
        }
        a.push("-i".into());
        a.push(input.to_string_lossy().into_owned());
        if let Some(d) = duration {
            a.push("-t".into());
            a.push(format!("{d}"));
        }
        // 只允许两种滤镜，且由本模块格式化 —— 不接受外部滤镜表达式
        let mut vf: Vec<String> = Vec::new();
        if let Some((w, h)) = scale {
            vf.push(format!("scale={w}:{h}"));
        }
        if let Some(f) = fps {
            vf.push(format!("fps={f}"));
        }
        if !vf.is_empty() {
            a.push("-vf".into());
            a.push(vf.join(","));
        }
        a.push("-c:v".into());
        a.push("libx264".into());
        a.push("-crf".into());
        a.push(format!("{}", crf.unwrap_or(23)));
        a.push("-pix_fmt".into());
        a.push("yuv420p".into());
        a.push("-movflags".into());
        a.push("+faststart".into());
        a.push(output.to_string_lossy().into_owned());
        a
    }
}

#[async_trait]
impl ToolExecutor for MediaTool {
    fn name(&self) -> &str {
        "media"
    }

    fn description(&self) -> &str {
        "视频/音频/图片的真实处理（ffmpeg）：probe 探测元数据、convert 转码压缩、\
         frames 抽帧、thumbnail 取封面。路径限工作区内；参数结构化，不接受任意滤镜。"
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "action": {"type": "string", "enum": ACTIONS},
                "input": {"type": "string", "description": "工作区内的输入文件"},
                "output": {"type": "string", "description": "工作区内的输出路径（convert/thumbnail/frames 必填）"},
                "start": {"type": "number", "description": "起始秒（可选）"},
                "duration": {"type": "number", "description": "时长秒（可选）"},
                "width": {"type": "integer"},
                "height": {"type": "integer"},
                "fps": {"type": "number", "description": "convert 输出帧率 / frames 抽帧频率"},
                "crf": {"type": "integer", "description": "x264 质量（0-51，越小越清晰，默认 23）"},
                "count": {"type": "integer", "description": "frames 最多抽多少张（默认 9）"}
            },
            "required": ["action", "input"]
        })
    }

    async fn execute(&self, args: &HashMap<String, String>) -> Result<ToolResult> {
        // 数值参数解析失败要变成**结构化工具错误**（让模型知道是哪个参数错了），
        // 而不是冒泡成 anyhow 的「内部错误」—— 后者在前端只显示一句无法行动的话。
        macro_rules! num {
            ($key:expr) => {
                match Self::num_arg(args, $key) {
                    Ok(v) => v,
                    Err(message) => {
                        return Ok(ToolResult::Error {
                            message,
                            code: "MEDIA_BAD_ARG".into(),
                        })
                    }
                }
            };
        }
        let action = args
            .get("action")
            .map(|s| s.trim().to_string())
            .unwrap_or_default();
        if !ACTIONS.contains(&action.as_str()) {
            return Ok(ToolResult::Error {
                message: format!(
                    "不支持的 action「{action}」；可用：{}",
                    ACTIONS.join(" / ")
                ),
                code: "MEDIA_UNSUPPORTED_ACTION".into(),
            });
        }
        let input = match self.resolve_arg(args, "input") {
            Ok(p) => p,
            Err(e) => {
                return Ok(ToolResult::Error {
                    message: e,
                    code: "MEDIA_BAD_PATH".into(),
                })
            }
        };
        if !input.is_file() {
            return Ok(ToolResult::Error {
                message: format!("输入文件不存在：{}", input.display()),
                code: "MEDIA_INPUT_MISSING".into(),
            });
        }

        // 超时：探测快、转码慢；外层工具超时另由 default_timeout_for 放宽到 10 分钟。
        let timeout = if action == "probe" {
            Duration::from_secs(30)
        } else {
            Duration::from_secs(540)
        };

        match action.as_str() {
            "probe" => {
                let args_v: Vec<String> = vec![
                    "-v".into(),
                    "error".into(),
                    "-print_format".into(),
                    "json".into(),
                    "-show_format".into(),
                    "-show_streams".into(),
                    input.to_string_lossy().into_owned(),
                ];
                match self.runner.run("ffprobe", &args_v, timeout) {
                    Ok(out) if out.exit_code == 0 => {
                        // 只回传摘要字段，避免把整份 streams JSON 塞进上下文
                        let summary = summarize_probe(&out.stdout);
                        Ok(ToolResult::Success {
                            data: summary.to_string(),
                            metadata: HashMap::new(),
                        })
                    }
                    Ok(out) => Ok(ToolResult::Error {
                        message: format!(
                            "ffprobe 失败（exit {}）：{}",
                            out.exit_code,
                            tail(&out.stderr, 600)
                        ),
                        code: "MEDIA_PROBE_FAILED".into(),
                    }),
                    Err(e) => Ok(map_runner_err(e)),
                }
            }
            "convert" => {
                let output = match self.resolve_arg(args, "output") {
                    Ok(p) => p,
                    Err(e) => {
                        return Ok(ToolResult::Error {
                            message: e,
                            code: "MEDIA_BAD_PATH".into(),
                        })
                    }
                };
                let scale = match (
                    num!("width"),
                    num!("height"),
                ) {
                    (Some(w), Some(h)) if w > 0.0 && h > 0.0 => {
                        Some((w as u32, h as u32))
                    }
                    (None, None) => None,
                    _ => {
                        return Ok(ToolResult::Error {
                            message: "width 与 height 必须同时给出且为正整数".into(),
                            code: "MEDIA_BAD_ARG".into(),
                        })
                    }
                };
                let crf = num!("crf").map(|v| v.clamp(0.0, 51.0) as u32);
                let args_v = Self::convert_args(
                    &input,
                    &output,
                    scale,
                    num!("fps"),
                    crf,
                    num!("start"),
                    num!("duration"),
                );
                self.finish_file_op("ffmpeg", &args_v, &output, timeout, "MEDIA_CONVERT_FAILED")
            }
            "thumbnail" => {
                let output = match self.resolve_arg(args, "output") {
                    Ok(p) => p,
                    Err(e) => {
                        return Ok(ToolResult::Error {
                            message: e,
                            code: "MEDIA_BAD_PATH".into(),
                        })
                    }
                };
                let at = num!("start").unwrap_or(0.0);
                let args_v: Vec<String> = vec![
                    "-y".into(),
                    "-ss".into(),
                    format!("{at}"),
                    "-i".into(),
                    input.to_string_lossy().into_owned(),
                    "-frames:v".into(),
                    "1".into(),
                    output.to_string_lossy().into_owned(),
                ];
                self.finish_file_op("ffmpeg", &args_v, &output, timeout, "MEDIA_THUMBNAIL_FAILED")
            }
            "frames" => {
                let output = match self.resolve_arg(args, "output") {
                    Ok(p) => p,
                    Err(e) => {
                        return Ok(ToolResult::Error {
                            message: e,
                            code: "MEDIA_BAD_PATH".into(),
                        })
                    }
                };
                let fps = num!("fps").unwrap_or(1.0);
                let count = num!("count").unwrap_or(9.0).max(1.0) as u32;
                let args_v: Vec<String> = vec![
                    "-y".into(),
                    "-i".into(),
                    input.to_string_lossy().into_owned(),
                    "-vf".into(),
                    format!("fps={fps}"),
                    "-frames:v".into(),
                    format!("{count}"),
                    output.to_string_lossy().into_owned(),
                ];
                self.finish_file_op("ffmpeg", &args_v, &output, timeout, "MEDIA_FRAMES_FAILED")
            }
            _ => unreachable!("action 已白名单校验"),
        }
    }
}

impl MediaTool {
    /// 产出文件的公共收尾：跑命令 → 校验产物真的存在 → 回报真实体积。
    fn finish_file_op(
        &self,
        program: &str,
        args: &[String],
        output: &Path,
        timeout: Duration,
        err_code: &str,
    ) -> Result<ToolResult> {
        match self.runner.run(program, args, timeout) {
            Ok(out) if out.exit_code == 0 => {
                // ffmpeg 有时 exit 0 但没写文件（如 frames 的 %03d 模板路径不匹配）。
                // 不校验就等于「报告成功但用户拿不到文件」。
                let size = std::fs::metadata(output).map(|m| m.len()).unwrap_or(0);
                if size == 0 {
                    return Ok(ToolResult::Error {
                        message: format!(
                            "{program} 退出码为 0，但产物为空或不存在：{}。\
                             frames 动作的 output 需带序号模板（如 frames_%03d.png）。",
                            output.display()
                        ),
                        code: "MEDIA_EMPTY_OUTPUT".into(),
                    });
                }
                Ok(ToolResult::Success {
                    data: json!({
                        "ok": true,
                        "output": output.to_string_lossy(),
                        "bytes": size,
                        "action_args": args,
                    })
                    .to_string(),
                    metadata: HashMap::new(),
                })
            }
            Ok(out) => Ok(ToolResult::Error {
                message: format!(
                    "{program} 失败（exit {}）：{}",
                    out.exit_code,
                    tail(&out.stderr, 900)
                ),
                code: err_code.into(),
            }),
            Err(e) => Ok(map_runner_err(e)),
        }
    }
}

/// 执行器错误 → 结构化工具错误（超时与「二进制缺失」必须可区分：
/// 前者是任务太大，后者是环境没装，处置完全不同）。
fn map_runner_err(e: anyhow::Error) -> ToolResult {
    let msg = e.to_string();
    if msg.starts_with("TIMEOUT:") {
        return ToolResult::Error {
            message: format!("媒体命令超时：{}", msg.trim_start_matches("TIMEOUT:")),
            code: "MEDIA_TIMEOUT".into(),
        };
    }
    if msg.contains("启动") && msg.contains("失败") {
        return ToolResult::Error {
            message: crate::tools::ffmpeg_bin::missing_message("ffmpeg"),
            code: "MEDIA_BINARY_MISSING".into(),
        };
    }
    ToolResult::Error {
        message: msg,
        code: "MEDIA_EXEC_ERROR".into(),
    }
}

/// 取输出尾部（错误信息通常在末尾，且整段可能极长）
fn tail(s: &str, max_chars: usize) -> String {
    let n = s.chars().count();
    if n <= max_chars {
        return s.trim().to_string();
    }
    let skip = n - max_chars;
    format!("…{}", s.chars().skip(skip).collect::<String>().trim())
}

/// ffprobe JSON → 精简摘要（时长/分辨率/编码/码率/音轨数）。
///
/// 解析失败时如实回传原文尾部，而不是编造字段。
pub fn summarize_probe(raw: &str) -> serde_json::Value {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(raw) else {
        return json!({"ok": false, "error": "ffprobe 输出不是 JSON", "raw": tail(raw, 400)});
    };
    let format = v.get("format").cloned().unwrap_or(json!({}));
    let streams = v
        .get("streams")
        .and_then(|s| s.as_array())
        .cloned()
        .unwrap_or_default();
    let video = streams
        .iter()
        .find(|s| s.get("codec_type").and_then(|x| x.as_str()) == Some("video"));
    let audio_count = streams
        .iter()
        .filter(|s| s.get("codec_type").and_then(|x| x.as_str()) == Some("audio"))
        .count();
    json!({
        "ok": true,
        "duration_s": format.get("duration").and_then(|x| x.as_str()).and_then(|x| x.parse::<f64>().ok()),
        "size_bytes": format.get("size").and_then(|x| x.as_str()).and_then(|x| x.parse::<u64>().ok()),
        "format": format.get("format_name").and_then(|x| x.as_str()),
        "bit_rate": format.get("bit_rate").and_then(|x| x.as_str()).and_then(|x| x.parse::<u64>().ok()),
        "video": video.map(|s| json!({
            "codec": s.get("codec_name").and_then(|x| x.as_str()),
            "width": s.get("width").and_then(|x| x.as_u64()),
            "height": s.get("height").and_then(|x| x.as_u64()),
            "fps": s.get("r_frame_rate").and_then(|x| x.as_str()),
        })),
        "audio_streams": audio_count,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// 记录 argv 的假执行器：断言「拼出来的命令行没有注入面」而不真的跑 ffmpeg。
    struct RecordingRunner {
        calls: Mutex<Vec<(String, Vec<String>)>>,
        exit: i32,
        stdout: String,
        stderr: String,
        /// 是否在「产出文件」时真的写一个文件（模拟 ffmpeg 落盘）
        write_output: bool,
    }

    impl RecordingRunner {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                calls: Mutex::new(Vec::new()),
                exit: 0,
                stdout: String::new(),
                stderr: String::new(),
                write_output: true,
            })
        }
    }

    impl MediaRunner for RecordingRunner {
        fn run(&self, program: &str, args: &[String], _t: Duration) -> Result<MediaOutput> {
            self.calls
                .lock()
                .unwrap()
                .push((program.to_string(), args.to_vec()));
            if self.write_output {
                if let Some(last) = args.last() {
                    let p = Path::new(last);
                    if p.extension().is_some() {
                        let _ = std::fs::write(p, b"fake-media-bytes");
                    }
                }
            }
            Ok(MediaOutput {
                exit_code: self.exit,
                stdout: self.stdout.clone(),
                stderr: self.stderr.clone(),
            })
        }
    }

    /// 每个用例独立目录（并发跑时共享目录会互相删掉对方的输入文件——
    /// 这正是本用例第一版失败的原因，不是实现缺陷）。
    fn ws(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("marg_media_{}_{tag}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        dir
    }

    fn args_of(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn convert_args_are_structured_and_carry_no_shell_metacharacters() {
        let a = MediaTool::convert_args(
            Path::new("in.mp4"),
            Path::new("out.mp4"),
            Some((1280, 720)),
            Some(30.0),
            Some(20),
            Some(1.5),
            Some(10.0),
        );
        assert_eq!(a[0], "-y");
        assert!(a.windows(2).any(|w| w[0] == "-ss" && w[1] == "1.5"));
        assert!(a.windows(2).any(|w| w[0] == "-t" && w[1] == "10"));
        // 滤镜只由本模块拼装：scale + fps 合并且无自由表达式
        let vf = a
            .windows(2)
            .find(|w| w[0] == "-vf")
            .map(|w| w[1].clone())
            .expect("应有 -vf");
        assert_eq!(vf, "scale=1280:720,fps=30");
        assert!(a.windows(2).any(|w| w[0] == "-crf" && w[1] == "20"));
        assert_eq!(a.last().unwrap(), "out.mp4");
        // 参数是离散元素，不存在 `;`/`&&`/`|` 之类的拼接面
        assert!(
            !a.iter().any(|s| s.contains("&&") || s.contains(';') || s.contains('|')),
            "参数不得包含 shell 连接符：{a:?}"
        );
    }

    #[tokio::test]
    async fn unknown_action_is_rejected_with_whitelist() {
        let tool = MediaTool::new(ws("unknown_action"), RecordingRunner::new());
        let r = tool
            .execute(&args_of(&[("action", "rm -rf"), ("input", "x.mp4")]))
            .await
            .unwrap();
        match r {
            ToolResult::Error { code, message } => {
                assert_eq!(code, "MEDIA_UNSUPPORTED_ACTION");
                assert!(message.contains("probe"), "应列出可用动作：{message}");
            }
            other => panic!("应为错误：{other:?}"),
        }
    }

    #[tokio::test]
    async fn path_escape_is_rejected_before_running_anything() {
        let runner = RecordingRunner::new();
        let tool = MediaTool::new(ws("escape"), runner.clone());
        let r = tool
            .execute(&args_of(&[
                ("action", "probe"),
                ("input", "../../../../Windows/System32/drivers/etc/hosts"),
            ]))
            .await
            .unwrap();
        match r {
            ToolResult::Error { code, .. } => assert_eq!(code, "MEDIA_BAD_PATH"),
            other => panic!("应为路径错误：{other:?}"),
        }
        assert!(
            runner.calls.lock().unwrap().is_empty(),
            "越界路径必须在起进程之前被拒"
        );
    }

    #[tokio::test]
    async fn probe_returns_summary_fields() {
        let dir = ws("probe");
        let input = dir.join("sample.mp4");
        std::fs::write(&input, b"x").unwrap();
        let runner = RecordingRunner::new();
        // 注入 ffprobe 的典型 JSON
        let with_json = Arc::new(RecordingRunner {
            calls: Mutex::new(Vec::new()),
            exit: 0,
            stdout: r#"{"format":{"duration":"12.5","size":"1024","format_name":"mov,mp4","bit_rate":"650000"},
                        "streams":[{"codec_type":"video","codec_name":"h264","width":1920,"height":1080,"r_frame_rate":"30/1"},
                                   {"codec_type":"audio","codec_name":"aac"}]}"#
                .to_string(),
            stderr: String::new(),
            write_output: false,
        });
        let tool = MediaTool::new(dir.clone(), with_json);
        let r = tool
            .execute(&args_of(&[
                ("action", "probe"),
                ("input", "sample.mp4"),
            ]))
            .await
            .unwrap();
        let data = r.data().expect("probe 应成功");
        assert!(data.contains("\"duration_s\":12.5"), "摘要: {data}");
        assert!(data.contains("\"width\":1920"), "摘要: {data}");
        assert!(data.contains("\"audio_streams\":1"), "摘要: {data}");
        let _ = runner;
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn empty_output_is_reported_as_failure_not_success() {
        let dir = ws("empty_output");
        let input = dir.join("in.mp4");
        std::fs::write(&input, b"x").unwrap();
        let runner = Arc::new(RecordingRunner {
            calls: Mutex::new(Vec::new()),
            exit: 0, // 退出码 0 但没写文件 —— 真实会发生
            stdout: String::new(),
            stderr: String::new(),
            write_output: false,
        });
        let tool = MediaTool::new(dir.clone(), runner);
        let r = tool
            .execute(&args_of(&[
                ("action", "convert"),
                ("input", "in.mp4"),
                ("output", "out.mp4"),
            ]))
            .await
            .unwrap();
        match r {
            ToolResult::Error { code, .. } => assert_eq!(code, "MEDIA_EMPTY_OUTPUT"),
            other => panic!("空产物必须判失败：{other:?}"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 真实 ffmpeg 端到端（默认忽略：CI/他机未必装 ffmpeg）。
    /// 本机实测方式：`cargo test -p marguerite-core --lib -- media::tests --ignored`
    #[tokio::test]
    #[ignore = "需要本机 ffmpeg；用 --ignored 显式运行"]
    async fn real_ffmpeg_convert_and_probe_end_to_end() {
        let dir = std::env::temp_dir().join(format!("marg_media_e2e_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let src = dir.join("src.mp4");
        // 用 lavfi 生成 1 秒测试源（不依赖任何外部素材）
        let gen = Command::new("ffmpeg")
            .silent()
            .args([
                "-y",
                "-f",
                "lavfi",
                "-i",
                "testsrc=size=320x240:rate=10:duration=1",
                "-pix_fmt",
                "yuv420p",
                src.to_string_lossy().as_ref(),
            ])
            .output()
            .expect("生成测试源");
        assert!(gen.status.success(), "lavfi 生成失败：{:?}", gen.stderr);

        let tool = MediaTool::with_system_runner(dir.clone());
        let probe = tool
            .execute(&args_of(&[("action", "probe"), ("input", "src.mp4")]))
            .await
            .unwrap();
        assert!(probe.data().is_some(), "probe 应成功：{probe:?}");

        let conv = tool
            .execute(&args_of(&[
                ("action", "convert"),
                ("input", "src.mp4"),
                ("output", "out.mp4"),
                ("width", "160"),
                ("height", "120"),
            ]))
            .await
            .unwrap();
        let data = conv.data().expect("convert 应成功");
        assert!(data.contains("\"bytes\""), "应回报体积：{data}");
        assert!(dir.join("out.mp4").is_file(), "产物应真的落盘");

        let thumb = tool
            .execute(&args_of(&[
                ("action", "thumbnail"),
                ("input", "src.mp4"),
                ("output", "cover.png"),
            ]))
            .await
            .unwrap();
        assert!(thumb.data().is_some(), "封面应成功：{thumb:?}");
        assert!(dir.join("cover.png").is_file());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// P2-9 多根：已存在的输入跟随它真实所在的根，新建产物落主根。
    #[test]
    fn multi_root_resolves_existing_input_in_extra_and_new_output_in_primary() {
        let base = std::env::temp_dir().join(format!("marg_media_multi_{}", std::process::id()));
        let primary = base.join("primary");
        let extra = base.join("extra");
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&primary).unwrap();
        std::fs::create_dir_all(&extra).unwrap();
        std::fs::write(extra.join("only-extra.mp4"), b"x").unwrap();

        let tool = MediaTool::new(
            RootSet::new(vec![primary.clone(), extra.clone()]),
            RecordingRunner::new(),
        );
        let input = tool
            .resolve_arg(&args_of(&[("input", "only-extra.mp4")]), "input")
            .expect("附加根内已存在的输入应被解析");
        assert!(
            input.starts_with(std::fs::canonicalize(&extra).unwrap_or(extra.clone())),
            "已存在文件必须跟随真实所在根：{}",
            input.display()
        );
        let output = tool
            .resolve_arg(&args_of(&[("output", "brand-new.mp4")]), "output")
            .expect("新产物路径应落在主根");
        assert!(
            output.starts_with(std::fs::canonicalize(&primary).unwrap_or(primary.clone())),
            "新建产物必须落主根：{}",
            output.display()
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn summarize_probe_degrades_honestly_on_garbage() {
        let v = summarize_probe("not json at all");
        assert_eq!(v["ok"], false);
        assert!(v["raw"].as_str().unwrap_or("").contains("not json"));
    }
}
