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
//! 违背「能跑通才算做完」。检测到 blender 后按同一模式加 `blender` 工具即可
//! （已做：见 `tools/blender.rs`）。
//!
//! # 素材源（material source）—— 本模块同时是它的**唯一权威实现**
//!
//! 用户口径的「素材从哪来」四选一（A 网络 / B 自动生成 / C 指定目录 / D 用户自填），
//! 由 [`parse_material_source`] 在**工具边界**上解析：
//! - 参数被**兑现**（真的据此取到字节 / 定位到文件）或**如实拒绝**（结构化 code + 原因），
//!   绝不被静默忽略 —— 静默忽略就是「假装按素材生成」，属 R-10 禁止的伪成功；
//! - 路径收口走 [`RootSet::resolve`]（越界即拒），网络收口走 [`SsrfGuard`]
//!   （含逐跳重定向复检），单次素材体积走 [`MATERIAL_MAX_BYTES`]。
//!
//! 消费方：[`crate::tools::image_gen`]（A/C/D = 交付既有素材）与
//! [`crate::tools::video_gen`]（A/C/D = 图生视频的首帧）。两者能兑现的**动作**不同，
//! 因为各自的上游端点形状不同 —— 这一点在两个工具里各自写死并各自有测试，
//! 不在这里做「看起来统一」的折中。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use base64::Engine as _;
use serde_json::json;

use crate::agent::tool_cycle::ToolResult;
use crate::mcp::registry::ToolExecutor;
use crate::proc::SilentCommand;
use crate::security::ssrf_guard::SsrfGuard;
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

// ===========================================================================
// 素材源（material source）：四选一语义的**后端权威实现**
// ===========================================================================

/// 素材源四态的**后端权威口径**。
///
/// 名字与顺序与 `webui/src/components/multimodal/materialSource.ts` 的
/// `MATERIAL_SOURCE_KINDS` 逐字相同 —— 两侧由跨栈测试
/// `material_source_kinds_match_the_frontend_surface` 咬合：面板能选的选项必须
/// 与工具边界能兑现的选项一一对应，否则用户在 UI 里选得到、工具吃不下。
pub const MATERIAL_SOURCE_KINDS: &[&str] = &["web", "generate", "directory", "custom"];

/// 单个素材的体积上限（12 MiB）。
///
/// 为什么必须有：图生视频要把首帧**整段**塞进 JSON 请求体（data URI），
/// 不设上限等于让一次工具调用打爆内存 / 换来上游 413。
pub const MATERIAL_MAX_BYTES: u64 = 12 * 1024 * 1024;

/// `data:` URI 的字符上限（base64 膨胀 ~1.37×，留 2× 余量）。
const MATERIAL_DATA_URI_MAX_CHARS: usize = (MATERIAL_MAX_BYTES as usize) * 2;

/// 图片扩展名白名单（小写；素材要能当首帧/被交付，必须先是一张图）。
pub const MATERIAL_IMAGE_EXTS: &[&str] = &["png", "jpg", "jpeg", "webp", "gif", "bmp"];

/// 视频扩展名白名单（`media` 的 concat / slideshow 输入）。
pub const MATERIAL_VIDEO_EXTS: &[&str] = &["mp4", "mov", "webm", "mkv", "avi", "m4v"];

/// 「既有素材被登记为产物」时台账里用的 kind。
///
/// ⚠️ 刻意**不用** `GENERATED_KIND`：C/D 这条路交付的是用户目录里本来就有的文件，
/// 标成 `generated` 会让产物中心把「取来的图」显示成「AI 生成的图」—— 那是台账撒谎。
pub const MATERIAL_ARTIFACT_KIND: &str = "material";

/// 解析后的素材源（四个语义里的「可兑现形态」）。
///
/// `custom`（D 用户自填）不是一种独立形态：它在 [`parse_material_source`] 里被
/// **判定**成 URL / 目录 / 文件三者之一（判不出来就结构化拒绝）。判据写在那里。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MaterialSource {
    /// B 自动生成：不携带任何既有素材。
    Generate,
    /// A 网络：http(s) 已过 [`SsrfGuard`]，或 `data:` URI（不产生出站请求）。
    Web { url: String },
    /// C 指定目录：**已收口在工作区内**且真实存在的目录。
    Directory { dir: PathBuf },
    /// D 用户自填：判定为工作区内真实存在的单个文件。
    File { path: PathBuf },
}

/// 素材源的**结构化拒绝**：code + 可直接给用户看的原因。
///
/// 与 `ToolResult::Error` 的转换只在这里做一次（[`MaterialError::into_tool_result`]），
/// 保证两个生成工具报出来的 code 口径一致。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MaterialError {
    pub code: String,
    pub message: String,
}

impl MaterialError {
    pub fn new(code: &str, message: impl Into<String>) -> Self {
        Self {
            code: code.to_string(),
            message: message.into(),
        }
    }

    pub fn into_tool_result(self) -> ToolResult {
        ToolResult::Error {
            message: self.message,
            code: self.code,
        }
    }
}

/// 素材源相关错误的统一出口（`Result<T, MaterialError>`）。
pub type MaterialResult<T> = std::result::Result<T, MaterialError>;

fn ref_arg(args: &HashMap<String, String>, key: &str) -> String {
    args.get(key)
        .map(|s| s.trim().to_string())
        .unwrap_or_default()
}

/// 校验并归类一个「网络」素材值：只接受 http(s)（过 SSRF 防护）与 `data:`。
///
/// 为什么 http(s) 必须过 [`SsrfGuard`]：素材 URL 是**用户/模型给的**，工具随后会
/// 自己去 GET 它。不设防就等于把 `web_fetch` 挡住的那条线（内网 / 回环 /
/// 云元数据 `169.254.169.254`）从这里重新打开。
pub fn classify_web_material(url: &str) -> MaterialResult<String> {
    let v = url.trim();
    if v.is_empty() {
        return Err(MaterialError::new(
            "MATERIAL_REF_REQUIRED",
            "material_source=web 必须给出 material_ref（http(s) URL 或 data: URI）。",
        ));
    }
    if v.starts_with("data:") {
        if v.len() > MATERIAL_DATA_URI_MAX_CHARS {
            return Err(MaterialError::new(
                "MATERIAL_TOO_LARGE",
                format!(
                    "data URI 体积超限：{} 字符 > {MATERIAL_DATA_URI_MAX_CHARS}。",
                    v.len()
                ),
            ));
        }
        return Ok(v.to_string());
    }
    let parsed = reqwest::Url::parse(v)
        .map_err(|e| MaterialError::new(
            "MATERIAL_URL_INVALID",
            format!("material_ref 不是合法 URL：{v}（{e}）"),
        ))?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(MaterialError::new(
            "MATERIAL_URL_SCHEME",
            format!(
                "素材 URL 只接受 http/https（或 data: URI），实际协议 `{}` 被拒绝。",
                parsed.scheme()
            ),
        ));
    }
    // 逐跳复检在 fetch_material_bytes 的 client 里；这里是**初始 URL** 的那一道。
    SsrfGuard::new()
        .check(v)
        .map_err(|e| MaterialError::new(
            "MATERIAL_URL_BLOCKED",
            format!("素材 URL 被 SSRF 防护拒绝：{e}"),
        ))?;
    Ok(v.to_string())
}

/// 把用户给的路径收口到工作区（**绝对路径也必须在登记根内**）。
fn contained_path(workspace: &RootSet, raw: &str) -> MaterialResult<PathBuf> {
    workspace
        .resolve(raw)
        .map_err(|e| MaterialError::new(
            "MATERIAL_PATH_ESCAPE",
            format!("素材路径必须落在工作区（或会话附加根）内：{e}"),
        ))
}

/// 解析工具参数里的素材源。
///
/// 真值表（每条都有单测）：
/// | `material_source` | `material_ref` | 结果 |
/// |---|---|---|
/// | 缺省/空 | 缺省/空 | `Generate`（**与加参数之前的行为完全一致**，老调用方零风险）|
/// | 缺省/空 | 非空 | 拒绝 `INVALID_PARAMS`：给了值却不给来源＝要么静默忽略要么瞎猜 |
/// | `generate` | 非空 | 拒绝 `INVALID_PARAMS`：自相矛盾 |
/// | 其它未知值 | — | 拒绝 `MATERIAL_SOURCE_UNKNOWN`（列出合法值）|
/// | `web` | URL/data | `Web`（协议 + SSRF + 体积校验）|
/// | `directory` | 工作区内目录 | `Directory`（不存在 `MATERIAL_DIR_MISSING`；不是目录 `MATERIAL_NOT_A_DIR`）|
/// | `custom` | URL / 文件 / 目录 | 判定为对应形态；判不出 → `MATERIAL_NOT_ACTIONABLE` |
pub fn parse_material_source(
    args: &HashMap<String, String>,
    workspace: &RootSet,
) -> MaterialResult<MaterialSource> {
    let kind = ref_arg(args, "material_source").to_ascii_lowercase();
    let value = ref_arg(args, "material_ref");

    if kind.is_empty() {
        if !value.is_empty() {
            return Err(MaterialError::new(
                "INVALID_PARAMS",
                format!(
                    "给了 material_ref=\"{}\" 却没有 material_source。\
                     本工具不猜来源、也不会静默丢掉它 —— 请显式声明：{}。",
                    value,
                    MATERIAL_SOURCE_KINDS.join(" / ")
                ),
            ));
        }
        return Ok(MaterialSource::Generate);
    }
    if !MATERIAL_SOURCE_KINDS.contains(&kind.as_str()) {
        return Err(MaterialError::new(
            "MATERIAL_SOURCE_UNKNOWN",
            format!(
                "不支持的 material_source「{kind}」；可用：{}。",
                MATERIAL_SOURCE_KINDS.join(" / ")
            ),
        ));
    }
    match kind.as_str() {
        "generate" => {
            if !value.is_empty() {
                return Err(MaterialError::new(
                    "INVALID_PARAMS",
                    format!(
                        "material_source=generate 表示「凭空生成」，不能再给 material_ref（实际：\"{value}\"）。"
                    ),
                ));
            }
            Ok(MaterialSource::Generate)
        }
        "web" => Ok(MaterialSource::Web {
            url: classify_web_material(&value)?,
        }),
        "directory" => {
            if value.is_empty() {
                return Err(MaterialError::new(
                    "MATERIAL_REF_REQUIRED",
                    "material_source=directory 必须给出目录路径（工作区内的相对路径或其中的绝对路径）。",
                ));
            }
            let p = contained_path(workspace, &value)?;
            if !p.exists() {
                return Err(MaterialError::new(
                    "MATERIAL_DIR_MISSING",
                    format!("素材目录不存在：{}", p.display()),
                ));
            }
            if !p.is_dir() {
                return Err(MaterialError::new(
                    "MATERIAL_NOT_A_DIR",
                    format!("material_ref 指向的不是目录：{}（要用单个文件请改 material_source=custom）", p.display()),
                ));
            }
            Ok(MaterialSource::Directory { dir: p })
        }
        // D「用户自填」= 形态未知的自由输入。判定顺序刻意是 URL → 目录 → 文件 → 拒绝：
        // 三者都是「工具真能兑现」的形态；剩下的纯描述文字**不能**兑现，必须响。
        "custom" => {
            if value.is_empty() {
                return Err(MaterialError::new(
                    "MATERIAL_REF_REQUIRED",
                    "material_source=custom 必须填写具体内容（URL、工作区内的文件或目录路径）。",
                ));
            }
            let lower = value.to_ascii_lowercase();
            if lower.starts_with("http://")
                || lower.starts_with("https://")
                || lower.starts_with("data:")
            {
                return Ok(MaterialSource::Web {
                    url: classify_web_material(&value)?,
                });
            }
            let p = contained_path(workspace, &value)?;
            if p.is_dir() {
                return Ok(MaterialSource::Directory { dir: p });
            }
            if p.is_file() {
                return Ok(MaterialSource::File { path: p });
            }
            Err(MaterialError::new(
                "MATERIAL_NOT_ACTIONABLE",
                format!(
                    "material_ref=\"{value}\" 既不是 http(s)/data URL，也不指向工作区内真实存在的文件或目录。\
                     素材来源必须是工具**能兑现**的东西；只是想描述素材请把它写进 prompt，并把 material_source 设为 generate。"
                ),
            ))
        }
        other => Err(MaterialError::new(
            "MATERIAL_SOURCE_UNKNOWN",
            format!("不支持的 material_source「{other}」。"),
        )),
    }
}

/// 扩展名是否属于图片白名单（大小写不敏感）。
pub fn is_material_image_ext(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| MATERIAL_IMAGE_EXTS.iter().any(|x| x.eq_ignore_ascii_case(e)))
        .unwrap_or(false)
}

/// 列出目录内**确定顺序**的素材文件（按小写文件名排序）。
///
/// 排序是刻意的：`read_dir` 的顺序在不同文件系统/平台上不稳定，
/// 「同一次请求两次拿到不同素材」比报错更难查。
pub fn list_material_files(dir: &Path, exts: &[&str]) -> Vec<PathBuf> {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = rd
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.is_file())
        .filter(|p| {
            p.extension()
                .and_then(|e| e.to_str())
                .map(|e| exts.iter().any(|x| x.eq_ignore_ascii_case(e)))
                .unwrap_or(false)
        })
        .collect();
    files.sort_by(|a, b| {
        a.file_name()
            .map(|n| n.to_string_lossy().to_lowercase())
            .unwrap_or_default()
            .cmp(&b.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default())
    });
    files
}

/// 从目录里按 `material_index`（0 起，缺省 0）取一个素材。
pub fn pick_material_in_dir(
    dir: &Path,
    exts: &[&str],
    index_arg: &str,
) -> MaterialResult<PathBuf> {
    let index: usize = if index_arg.is_empty() {
        0
    } else {
        index_arg
            .parse::<usize>()
            .map_err(|_| MaterialError::new(
                "INVALID_PARAMS",
                format!("material_index 必须是非负整数（0 起），实际：{index_arg}"),
            ))?
    };
    let files = list_material_files(dir, exts);
    if files.is_empty() {
        return Err(MaterialError::new(
            "MATERIAL_DIR_EMPTY",
            format!(
                "目录里没有可用素材：{}（只认扩展名 {}）",
                dir.display(),
                exts.join(" / ")
            ),
        ));
    }
    if let Some(p) = files.get(index) {
        return Ok(p.clone());
    }
    // 越界时把**真实数量与前几个文件名**报出来：让模型/用户一次就能改成对的 index，
    // 而不是拿到一句「越界」再去猜目录里到底有什么。
    let names: Vec<String> = files
        .iter()
        .take(5)
        .map(|p| {
            p.file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default()
        })
        .collect();
    Err(MaterialError::new(
        "MATERIAL_INDEX_OUT_OF_RANGE",
        format!(
            "material_index={index} 越界：{} 里只有 {} 个可用素材（前几个：{}）。",
            dir.display(),
            files.len(),
            names.join(", ")
        ),
    ))
}

/// 按文件头判定图片格式（与 `image_gen` 的落盘嗅探同一套魔数）。
pub fn sniff_image_ext(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG") {
        Some("png")
    } else if bytes.starts_with(&[0xFF, 0xD8]) {
        Some("jpg")
    } else if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some("webp")
    } else if bytes.starts_with(b"GIF8") {
        Some("gif")
    } else if bytes.starts_with(b"BM") {
        Some("bmp")
    } else {
        None
    }
}

/// 读取素材字节（先查体积再读，避免超限文件进内存）。
pub fn read_material_bytes(path: &Path) -> MaterialResult<Vec<u8>> {
    let len = std::fs::metadata(path)
        .map(|m| m.len())
        .map_err(|e| MaterialError::new(
            "MATERIAL_READ_FAILED",
            format!("读不到素材 {}：{e}", path.display()),
        ))?;
    if len > MATERIAL_MAX_BYTES {
        return Err(MaterialError::new(
            "MATERIAL_TOO_LARGE",
            format!(
                "素材体积超限：{} 字节 > {} 字节（{path_display}）。\
                 请压缩或换用小图 —— 首帧素材要整段进请求体，放大文件只会让上游直接拒绝。",
                len,
                MATERIAL_MAX_BYTES,
                path_display = path.display()
            ),
        ));
    }
    std::fs::read(path).map_err(|e| {
        MaterialError::new(
            "MATERIAL_READ_FAILED",
            format!("读取素材失败 {}：{e}", path.display()),
        )
    })
}

/// 图片字节 → `data:image/<ext>;base64,…`（按**文件头**判定格式，不看扩展名）。
///
/// 为什么按头不按名：`photo.jpg` 实际是 PNG 的情况很常见，按扩展名拼出的
/// `data:image/jpg` 前缀会让上游解码失败，而失败信息完全不指向真正的原因。
pub fn image_bytes_to_data_uri(bytes: &[u8]) -> MaterialResult<String> {
    let ext = sniff_image_ext(bytes).ok_or_else(|| MaterialError::new(
        "MATERIAL_NOT_IMAGE",
        "素材文件头不是可识别的图片格式（png / jpg / webp / gif / bmp）。\
         图生视频的首帧只接受图片字节；若要按视频素材做剪辑，请改用 `media` 工具的 concat/slideshow。",
    ))?;
    let b64 = base64::engine::general_purpose::STANDARD.encode(bytes);
    Ok(format!("data:image/{ext};base64,{b64}"))
}

/// `data:` URI → 字节（带体积上限与解码错误如实上报）。
pub fn data_uri_to_bytes(uri: &str) -> MaterialResult<Vec<u8>> {
    let rest = uri
        .split_once(',')
        .map(|(_, r)| r)
        .ok_or_else(|| MaterialError::new(
            "MATERIAL_URL_INVALID",
            "data URI 缺少 `,` 分隔的载荷部分。",
        ))?;
    if rest.len() > MATERIAL_DATA_URI_MAX_CHARS {
        return Err(MaterialError::new(
            "MATERIAL_TOO_LARGE",
            format!("data URI 载荷超限：{} 字符。", rest.len()),
        ));
    }
    base64::engine::general_purpose::STANDARD
        .decode(rest.trim())
        .map_err(|e| MaterialError::new(
            "MATERIAL_URL_INVALID",
            format!("data URI 的 base64 载荷解码失败：{e}"),
        ))
}

/// 网络素材的取回通道（可注入：单测不联网，生产用 reqwest）。
#[async_trait]
pub trait MaterialFetcher: Send + Sync {
    /// 取回字节；超过 `max_bytes` 时**中止**并返回 `TOO_LARGE:` 前缀的错误。
    async fn fetch(&self, url: &str, max_bytes: usize) -> Result<Vec<u8>, String>;
}

/// 生产实现：带**逐跳 SSRF 复检**与硬超时的 reqwest。
///
/// 与 `tools/builtin/web.rs` 的 `guarded_http_client` 同一口径（那边是 `pub(crate)`
/// 且模块私有，这里不能直接复用，于是按同一判据实现一份）：
/// 只校验初始 URL 会被「公网 URL 302 → `169.254.169.254`」绕过。
pub struct HttpMaterialFetcher;

#[async_trait]
impl MaterialFetcher for HttpMaterialFetcher {
    async fn fetch(&self, url: &str, max_bytes: usize) -> Result<Vec<u8>, String> {
        let ssrf = Arc::new(SsrfGuard::new());
        let redirect_ssrf = ssrf.clone();
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(15))
            .timeout(Duration::from_secs(120))
            .redirect(reqwest::redirect::Policy::custom(move |attempt| {
                if attempt.previous().len() >= 5 {
                    return attempt.error("too many redirects");
                }
                if redirect_ssrf.check(attempt.url().as_str()).is_err() {
                    return attempt.error("redirect to blocked address (ssrf)");
                }
                attempt.follow()
            }))
            .build()
            .map_err(|e| format!("构造 HTTP 客户端失败: {e}"))?;
        // 初始 URL 再判一次：调用方可能来自单测注入的假 fetcher，这道防线不能只靠上游。
        ssrf.check(url).map_err(|e| format!("SSRF_BLOCKED: {e}"))?;
        let resp = client
            .get(url)
            .send()
            .await
            .map_err(|e| format!("请求失败: {e}"))?;
        let status = resp.status();
        if !status.is_success() {
            return Err(format!("上游返回 {}", status.as_u16()));
        }
        // 逐块累加并**边下边查**体积：`resp.bytes()` 会把整条响应读进内存，
        // 一个「Content-Length 谎报 / 无 Length」的大文件就能打爆引擎 ——
        // 而素材上限是本模块对用户的承诺（MATERIAL_MAX_BYTES），承诺必须兑现。
        let mut stream = Box::pin(resp.bytes_stream());
        let mut acc: Vec<u8> = Vec::new();
        while let Some(chunk) = futures_util::StreamExt::next(&mut stream).await {
            let chunk = chunk.map_err(|e| format!("下载中断: {e}"))?;
            if acc.len() + chunk.len() > max_bytes {
                return Err(format!("TOO_LARGE: 素材超过 {max_bytes} 字节上限"));
            }
            acc.extend_from_slice(&chunk);
        }
        Ok(acc)
    }
}

/// 取回一个网络素材（http(s) 走 fetcher；`data:` 就地解码）。
pub async fn fetch_material_bytes(
    url: &str,
    fetcher: &dyn MaterialFetcher,
) -> MaterialResult<Vec<u8>> {
    let checked = classify_web_material(url)?;
    if checked.starts_with("data:") {
        return data_uri_to_bytes(&checked);
    }
    fetcher
        .fetch(&checked, MATERIAL_MAX_BYTES as usize)
        .await
        .map_err(|e| {
            if e.starts_with("TOO_LARGE:") {
                MaterialError::new(
                    "MATERIAL_TOO_LARGE",
                    format!("{e}（上限 {} 字节）", MATERIAL_MAX_BYTES),
                )
            } else {
                MaterialError::new("MATERIAL_FETCH_FAILED", format!("素材下载失败：{e}"))
            }
        })
}

/// 素材源的**人话描述**（进工具返回值，让用户/模型看得见「这次到底用了哪份素材」）。
pub fn describe_material_source(src: &MaterialSource) -> String {
    match src {
        MaterialSource::Generate => "generate（模型凭空生成，未使用既有素材）".to_string(),
        MaterialSource::Web { url } => format!("web（网络素材：{}）", truncate(url, 120)),
        MaterialSource::Directory { dir } => format!("directory（指定目录：{}）", dir.display()),
        MaterialSource::File { path } => format!("custom（用户自填文件：{}）", path.display()),
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

/// 支持的动作（白名单，fail-closed）
const ACTIONS: &[&str] = &["probe", "convert", "frames", "thumbnail", "slideshow", "concat"];

// ===========================================================================
// 剪辑编排（slideshow / concat）：多素材 → 一个成片
// ===========================================================================
//
// 用户口径里的「多图/视频剪切编排」此前完全不存在：`media` 只有**单输入**动作
// （probe/convert/frames/thumbnail），多张图片拼成一段视频、几段视频接成一个片子，
// 只能靠模型手写 `shell_exec` 命令行 —— 于是既没有路径收口，也没有产物校验。
// 这里把它收成两个结构化动作：命令行由本模块拼，输入来自**素材源**（C/D 目录或
// 显式 `inputs` 列表），产物必须真实落盘才算成功。

// ⚠️ 以下五项属于**已写好但尚未接线**的「图片转视频 / 视频拼接」结构化动作：
// 参数拼接与清单解析都已实现并有单测，缺的是把它们挂成工具 action 的那一步。
// 因此 lib 构建里它们暂时是 dead_code —— 用 allow 显式记账，而不是删掉在办工作。
/// 单次剪辑最多处理的输入数（上限刻意保守：清单过长意味着「把整个目录倒进来」，
/// 那种请求既慢又几乎一定不是用户要的成片）。
#[allow(dead_code)]
const MAX_EDIT_INPUTS: usize = 120;

/// 每张图片默认停留秒数（slideshow）。
#[allow(dead_code)]
const DEFAULT_FRAME_DURATION: f64 = 3.0;

/// 清单里的一条：路径 + 可选停留时长。
#[derive(Debug, Clone, PartialEq)]
pub struct ManifestEntry {
    pub path: PathBuf,
    pub duration: Option<f64>,
}

/// 清单内的路径写法：统一正斜杠。
///
/// Windows 的反斜杠在 ffmpeg concat demuxer 里会被当转义符处理（不同版本行为不一），
/// 而 `/` 在 Windows 与 Unix 上都被 ffmpeg 接受 —— 归一化比赌版本行为可靠。
pub fn manifest_path_text(p: &Path) -> String {
    p.to_string_lossy().replace('\\', "/")
}

/// 清单文件名是否可安全写入（**fail-closed**：判不出来就拒）。
///
/// `file '<path>'` 的引号语法里，路径含 `'` 需要转义，含换行则直接破坏清单结构 ——
/// 与其实现一套我们无法端到端验证的转义规则，不如拒绝并说清楚原因。
fn manifest_name_ok(p: &Path) -> MaterialResult<()> {
    let s = p.to_string_lossy();
    if s.contains('\'') || s.contains('\n') || s.contains('\r') {
        return Err(MaterialError::new(
            "MEDIA_UNSAFE_NAME",
            format!(
                "素材路径含单引号或换行，无法安全写进 ffmpeg 拼接清单：{}",
                truncate(&s.replace('\n', "\\n").replace('\r', "\\r"), 120)
            ),
        ));
    }
    Ok(())
}

/// 拼 concat demuxer 的清单内容（纯函数：不依赖 ffmpeg，单测直接断言文本）。
///
/// `repeat_last` 是 slideshow 必需的一行：ffmpeg 读到最后一条 `file` 若没有配对的
/// `duration`，**不会**在它上面停留 —— 成片会「少最后一张」。不重复就没有这个停留，
/// 而这是产物层面的静默错误（用户只会说「最后一张没出现」，没人能猜到原因）。
pub fn concat_manifest(entries: &[ManifestEntry], repeat_last: bool) -> MaterialResult<String> {
    let mut out = String::new();
    for e in entries {
        manifest_name_ok(&e.path)?;
        out.push_str(&format!("file '{}'\n", manifest_path_text(&e.path)));
        if let Some(d) = e.duration {
            out.push_str(&format!("duration {d}\n"));
        }
    }
    if let (true, Some(last)) = (repeat_last, entries.last()) {
        // 只有按 duration 排片（slideshow）才需要补这一行；纯视频接片不需要。
        if last.duration.is_some() {
            out.push_str(&format!("file '{}'\n", manifest_path_text(&last.path)));
        }
    }
    Ok(out)
}

/// slideshow 的 ffmpeg 参数（纯函数）。
///
/// 每张图先归一到同一画布（`force_original_aspect_ratio=decrease` + `pad` 居中补边），
/// 否则 concat demuxer 会因为「相邻两帧分辨率不同」直接失败 —— 而它的报错完全不指向
/// 「你的图片尺寸不一致」这个真实原因。滤镜表达式**由本函数写死**，不接受外部输入。
#[allow(dead_code)]
fn slideshow_args(
    manifest: &Path,
    output: &Path,
    canvas: (u32, u32),
    fps: f64,
    crf: u32,
) -> Vec<String> {
    let (w, h) = canvas;
    vec![
        "-y".into(),
        "-f".into(),
        "concat".into(),
        "-safe".into(),
        "0".into(),
        "-i".into(),
        manifest_path_text(manifest),
        "-vf".into(),
        format!(
            "scale={w}:{h}:force_original_aspect_ratio=decrease,pad={w}:{h}:(ow-iw)/2:(oh-ih)/2,format=yuv420p"
        ),
        "-r".into(),
        format!("{fps}"),
        "-c:v".into(),
        "libx264".into(),
        "-crf".into(),
        format!("{crf}"),
        "-pix_fmt".into(),
        "yuv420p".into(),
        "-movflags".into(),
        "+faststart".into(),
        manifest_path_text(output),
    ]
}

/// concat 的 ffmpeg 参数（纯函数）。
///
/// 默认 `-c copy`（快、无损，但要求各路素材编码一致）；`reencode=Some(crf)` 时
/// 强制重编（慢，能容忍混源）—— 两种模式给模型显式选择，而不是让它猜失败原因。
#[allow(dead_code)]
fn video_concat_args(manifest: &Path, output: &Path, reencode: Option<u32>) -> Vec<String> {
    let mut a: Vec<String> = vec![
        "-y".into(),
        "-f".into(),
        "concat".into(),
        "-safe".into(),
        "0".into(),
        "-i".into(),
        manifest_path_text(manifest),
    ];
    match reencode {
        None => {
            a.push("-c".into());
            a.push("copy".into());
        }
        Some(crf) => {
            a.push("-c:v".into());
            a.push("libx264".into());
            a.push("-crf".into());
            a.push(format!("{crf}"));
            a.push("-pix_fmt".into());
            a.push("yuv420p".into());
            a.push("-c:a".into());
            a.push("aac".into());
        }
    }
    a.push("-movflags".into());
    a.push("+faststart".into());
    a.push(manifest_path_text(output));
    a
}

/// 剪辑产物的**清单文件**路径：贴在输出旁边，命名一眼看出是辅助文件。
#[allow(dead_code)]
fn manifest_path_for(output: &Path) -> PathBuf {
    let stem = output
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "edit".to_string());
    let dir = output.parent().unwrap_or_else(|| Path::new("."));
    dir.join(format!("{stem}.concat-list.txt"))
}

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
