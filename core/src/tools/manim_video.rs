//! Manim 生视频工具（`manim_video`）：把 Manim 场景源码渲染成 mp4。
//!
//! # 为什么需要（2026-10-24）
//!
//! 本机实测：`python`（3.14.0）可用，但 `import manim` 失败。此前视频类能力只有
//! 云端 `video_generate` 与 `media` 转码，**没有任何「用代码画数学动画」的本地路径**。
//! 本工具把「写一段 Manim 场景 → 得到 mp4」收成一次结构化调用：argv 由这里拼，
//! 产物落到工作区并登记为产物。
//!
//! # 诚实边界
//!
//! - Python / Manim 只要缺一个就**明确报错并给安装指引**，绝不返回占位视频或谎报成功；
//! - 场景类识别按源码行做保守匹配（见 [`detect_scene_class`]）：只认**同一行**内
//!   完成的 `class Xxx(...Scene...):`，多行基类列表、动态 `type(...)` 等都识别不到，
//!   此时明确要求显式传 `scene_class`；
//! - 渲染完全交给 `manim render`，本工具不解析 Manim 的内部日志，只认产物文件；
//! - stderr 只回传尾部 1500 字符 —— Manim 日志很长，全量回传会挤爆模型上下文。
//!
//! # 输出目录约定
//!
//! `manim render --media_dir <tmp>/media` 会把成片写到
//! `<tmp>/media/videos/<场景文件名>/<分辨率目录>/*.mp4`（场景文件名固定为 `scene`），
//! 因此 [`pick_output`] 在 `media/videos` 下做有界递归并取**最新**的一个 mp4。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime};

use anyhow::Result;
use async_trait::async_trait;
use serde_json::json;

use crate::agent::tool_cycle::ToolResult;
use crate::mcp::registry::ToolExecutor;
use crate::proc::SilentCommand;
use crate::tools::file_change::{record_artifact, GENERATED_KIND};
use crate::tools::fs_path::RootSet;

/// 能力探测用的最小 Python 片段（同时校验 `import` 与版本可读）。
const PROBE_CODE: &str = "import manim, sys; print(manim.__version__)";
/// 探测超时：解释器冷启动 + `import manim` 的常见耗时上限。
const PROBE_TIMEOUT: Duration = Duration::from_secs(30);
/// [`pick_output`] 的最大向下递归层数（`media/videos/<场景>/<分辨率>/` 已足够）。
const PICK_MAX_DEPTH: usize = 5;
/// 错误回传里保留的 stderr 字符上限。
const STDERR_LIMIT: usize = 1500;

/// `manim_video` 工具。
pub struct ManimVideoTool {
    workspace: RootSet,
}

impl ManimVideoTool {
    pub fn new(workspace: impl Into<RootSet>) -> Self {
        let workspace: RootSet = workspace.into();
        Self { workspace }
    }
}

#[async_trait]
impl ToolExecutor for ManimVideoTool {
    fn name(&self) -> &str {
        "manim_video"
    }

    fn description(&self) -> &str {
        "用 Manim 把数学动画场景代码渲染成 mp4。scene 必填（完整的 Python 源码，\
         必须含一个 Manim 场景类）；scene_class 不填时自动识别；\
         quality 可选 low/medium/high/production（默认 medium）；\
         out 默认 generated/videos/manim-<时间戳>.mp4。\
         未安装 Python 或 Manim 时会明确报错并给出安装指引，不会返回占位视频。"
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "scene": {
                    "type": "string",
                    "description": "Python 源码，必须包含一个继承自 Scene / ThreeDScene / MovingCameraScene 的场景类"
                },
                "scene_class": {
                    "type": "string",
                    "description": "要渲染的场景类名；不填时从源码自动识别"
                },
                "quality": {
                    "type": "string",
                    "enum": ["low", "medium", "high", "production"],
                    "description": "渲染质量档，默认 medium（对应 -qm）"
                },
                "out": {
                    "type": "string",
                    "description": "输出文件（工作区内相对路径），默认 generated/videos/manim-<时间戳>.mp4"
                },
                "timeout_seconds": {
                    "type": "integer",
                    "description": "渲染超时秒数，默认 600"
                },
                "python": {
                    "type": "string",
                    "description": "Python 解释器绝对路径；不填时依次读 MARGUERITE_PYTHON、PATH"
                }
            },
            "required": ["scene"]
        })
    }

    async fn execute(&self, args: &HashMap<String, String>) -> Result<ToolResult> {
        let scene = args
            .get("scene")
            .map(|s| s.trim().to_string())
            .unwrap_or_default();
        if scene.is_empty() {
            return Ok(ToolResult::Error {
                message: "manim_video 需要非空的 scene（完整的 Manim 场景 Python 源码）".to_string(),
                code: "INVALID_PARAMS".to_string(),
            });
        }

        // ── 场景类：显式优先，否则从源码识别 ─────────────────────────────
        let explicit_class = args
            .get("scene_class")
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        let scene_class = match explicit_class {
            Some(c) => {
                if !is_identifier(&c) {
                    return Ok(ToolResult::Error {
                        message: format!(
                            "scene_class「{c}」不是合法的 Python 类名（只允许字母、数字、下划线，且不能以数字开头）"
                        ),
                        code: "INVALID_PARAMS".to_string(),
                    });
                }
                c
            }
            None => match detect_scene_class(&scene) {
                Some(c) => c,
                None => {
                    return Ok(ToolResult::Error {
                        message: "未能在 scene 源码中识别出 Manim 场景类。\
                                 本工具只识别同一行内完成的 `class 类名(Scene):`、\
                                 `class 类名(ThreeDScene):`、`class 类名(MovingCameraScene):`；\
                                 如果基类列表换行书写或类名是动态生成的，请显式传入 scene_class。"
                            .to_string(),
                        code: "SCENE_CLASS_NOT_FOUND".to_string(),
                    })
                }
            },
        };

        // ── 质量档 ──────────────────────────────────────────────────────
        let quality = args
            .get("quality")
            .map(|s| s.trim().to_ascii_lowercase())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "medium".to_string());
        let flag = match quality_flag(&quality) {
            Some(f) => f,
            None => {
                return Ok(ToolResult::Error {
                    message: format!(
                        "quality「{quality}」不支持；只能是 low / medium / high / production"
                    ),
                    code: "INVALID_PARAMS".to_string(),
                })
            }
        };

        // ── 超时 ────────────────────────────────────────────────────────
        let timeout_seconds = match args
            .get("timeout_seconds")
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
        {
            None => 600u64,
            Some(v) => match v.parse::<u64>() {
                Ok(n) if n > 0 => n,
                _ => {
                    return Ok(ToolResult::Error {
                        message: format!("timeout_seconds 必须是正整数秒，实际：{v}"),
                        code: "INVALID_PARAMS".to_string(),
                    })
                }
            },
        };

        // ── 输出路径（先解析，避免白渲染一场才发现路径越界）─────────────
        let out = args
            .get("out")
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| {
                format!(
                    "generated/videos/manim-{}.mp4",
                    chrono::Local::now().format("%Y%m%d-%H%M%S")
                )
            });
        let target = match self.workspace.resolve(&out) {
            Ok(p) => p,
            Err(e) => {
                return Ok(ToolResult::Error {
                    message: format!("输出路径非法（必须落在工作区内）：{e}"),
                    code: "INVALID_PATH".to_string(),
                })
            }
        };

        // ── Python 解释器解析 ───────────────────────────────────────────
        let explicit_python = args
            .get("python")
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .map(PathBuf::from);
        if let Some(p) = &explicit_python {
            if !p.is_file() {
                return Ok(ToolResult::Error {
                    message: format!(
                        "参数 python 指定的解释器不存在：{}。请传入 python.exe 的绝对路径。",
                        p.display()
                    ),
                    code: "NO_PYTHON".to_string(),
                });
            }
        }
        let env_python = std::env::var_os("MARGUERITE_PYTHON")
            .map(PathBuf::from)
            .filter(|p| !p.as_os_str().is_empty());
        let path_dirs: Vec<PathBuf> = std::env::var_os("PATH")
            .map(|p| std::env::split_paths(&p).collect())
            .unwrap_or_default();
        let python = match pick_python(
            explicit_python.as_deref(),
            env_python.as_deref(),
            managed_python().as_deref(),
            &path_dirs,
        )
        {
            Some(p) => p,
            None => {
                return Ok(ToolResult::Error {
                    message: no_python_message(),
                    code: "NO_PYTHON".to_string(),
                })
            }
        };

        // ── 能力探测：Manim 是否真的可 import ───────────────────────────
        let probe_args = vec!["-c".to_string(), PROBE_CODE.to_string()];
        let manim_version = match run_captured(&python, &probe_args, PROBE_TIMEOUT) {
            Ok(out) if out.exit_code == 0 => {
                let v = last_nonempty_line(&out.stdout);
                if v.is_empty() {
                    "unknown".to_string()
                } else {
                    v
                }
            }
            Ok(out) => {
                let detail = if out.stderr.trim().is_empty() {
                    out.stdout
                } else {
                    out.stderr
                };
                return Ok(ToolResult::Error {
                    message: no_manim_message(&python, &detail),
                    code: "NO_MANIM".to_string(),
                });
            }
            Err(RunFailure::Timeout(msg)) => {
                return Ok(ToolResult::Error {
                    message: format!(
                        "探测 Manim 是否安装超时：{msg}。请检查 Python 环境是否卡住，\
                         或换一个干净的解释器后重试。"
                    ),
                    code: "MANIM_PROBE_TIMEOUT".to_string(),
                })
            }
            Err(RunFailure::Spawn(msg)) => {
                return Ok(ToolResult::Error {
                    message: format!("{msg}。请确认 python 参数指向可执行的解释器。"),
                    code: "NO_PYTHON".to_string(),
                })
            }
            Err(RunFailure::Wait(msg)) => {
                return Ok(ToolResult::Error {
                    message: msg,
                    code: "MANIM_EXEC_ERROR".to_string(),
                })
            }
        };

        // ── 落地临时目录并渲染 ──────────────────────────────────────────
        let tmp = std::env::temp_dir().join(format!(
            "marguerite-manim-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        if let Err(e) = std::fs::create_dir_all(&tmp) {
            return Ok(ToolResult::Error {
                message: format!("创建临时目录失败（{}）：{e}", tmp.display()),
                code: "IO_ERROR".to_string(),
            });
        }
        let scene_py = tmp.join("scene.py");
        if let Err(e) = std::fs::write(&scene_py, scene.as_bytes()) {
            let _ = std::fs::remove_dir_all(&tmp);
            return Ok(ToolResult::Error {
                message: format!("写入场景源码失败（{}）：{e}", scene_py.display()),
                code: "IO_ERROR".to_string(),
            });
        }

        let media_dir = tmp.join("media");
        let job = RenderJob {
            python: python.as_path(),
            flag,
            media_dir: media_dir.as_path(),
            scene_py: scene_py.as_path(),
            scene_class: scene_class.as_str(),
            quality: quality.as_str(),
            target: target.as_path(),
            timeout_seconds,
            manim_version: manim_version.as_str(),
        };
        let result = self.render_and_deliver(&job);
        // 尽力清理：成功路径按约定删除；失败路径也一并清理，避免临时目录越积越多。
        let _ = std::fs::remove_dir_all(&tmp);
        Ok(result)
    }
}

/// 一次渲染所需的全部输入。
struct RenderJob<'a> {
    python: &'a Path,
    flag: &'a str,
    media_dir: &'a Path,
    scene_py: &'a Path,
    scene_class: &'a str,
    quality: &'a str,
    target: &'a Path,
    timeout_seconds: u64,
    manim_version: &'a str,
}

impl ManimVideoTool {
    /// 跑 `manim render` → 找产物 → 复制进工作区 → 登记产物。
    fn render_and_deliver(&self, job: &RenderJob<'_>) -> ToolResult {
        let argv = build_command(
            job.python,
            job.flag,
            job.media_dir,
            job.scene_py,
            job.scene_class,
        );
        let captured = match run_captured(
            job.python,
            &argv[1..],
            Duration::from_secs(job.timeout_seconds),
        ) {
            Ok(c) => c,
            Err(RunFailure::Timeout(msg)) => {
                return ToolResult::Error {
                    message: format!(
                        "Manim 渲染超时：{msg}（timeout_seconds={}）。已终止子进程；\
                         可调大 timeout_seconds，或先用 quality=low 验证代码能否正常渲染。",
                        job.timeout_seconds
                    ),
                    code: "MANIM_TIMEOUT".to_string(),
                }
            }
            Err(RunFailure::Spawn(msg)) => {
                return ToolResult::Error {
                    message: format!("{msg}。请确认 python 参数指向可执行的解释器。"),
                    code: "NO_PYTHON".to_string(),
                }
            }
            Err(RunFailure::Wait(msg)) => {
                return ToolResult::Error {
                    message: msg,
                    code: "MANIM_EXEC_ERROR".to_string(),
                }
            }
        };

        if captured.exit_code != 0 {
            return ToolResult::Error {
                message: format!(
                    "Manim 渲染失败（退出码 {}）。常见原因：scene_class 与源码中的类名不一致、\
                     场景运行时报错、依赖缺失。\nstderr（截断至 {STDERR_LIMIT} 字符）：{}",
                    captured.exit_code,
                    tail(&captured.stderr, STDERR_LIMIT)
                ),
                code: "MANIM_RENDER_FAILED".to_string(),
            };
        }

        // 退出码 0 不代表真有产物：Manim 的失败有时只体现在日志里，必须以文件为准。
        let Some(mp4) = pick_output(job.media_dir) else {
            return ToolResult::Error {
                message: format!(
                    "Manim 退出码为 0，但在 {} 下没有找到 mp4 产物。\
                     请确认场景的 construct 里有实际动画（如 self.play(...)）。\n\
                     stderr（截断至 {STDERR_LIMIT} 字符）：{}",
                    job.media_dir.display(),
                    tail(&captured.stderr, STDERR_LIMIT)
                ),
                code: "MANIM_NO_OUTPUT".to_string(),
            };
        };

        if let Some(dir) = job.target.parent() {
            if let Err(e) = std::fs::create_dir_all(dir) {
                return ToolResult::Error {
                    message: format!("创建输出目录失败（{}）：{e}", dir.display()),
                    code: "IO_ERROR".to_string(),
                };
            }
        }
        if let Err(e) = std::fs::copy(&mp4, job.target) {
            return ToolResult::Error {
                message: format!(
                    "复制渲染结果到工作区失败（{} → {}）：{e}",
                    mp4.display(),
                    job.target.display()
                ),
                code: "IO_ERROR".to_string(),
            };
        }

        let bytes = std::fs::metadata(job.target).map(|m| m.len()).unwrap_or(0);
        if bytes == 0 {
            let _ = std::fs::remove_file(job.target);
            return ToolResult::Error {
                message: format!("渲染产物为空：{}", job.target.display()),
                code: "MANIM_EMPTY_OUTPUT".to_string(),
            };
        }
        record_artifact("", &job.target.display().to_string(), GENERATED_KIND);

        let mut payload = json!({
            "ok": true,
            "path": job.target.display().to_string(),
            "bytes": bytes,
            "sceneClass": job.scene_class,
            "quality": job.quality,
            "python": job.python.display().to_string(),
            "manimVersion": job.manim_version,
        });
        // 时长来自 ffprobe；拿不到（未内置 ffprobe / 探测失败）就省略，不编造。
        if let Some(d) = probe_duration(job.target) {
            payload["durationSeconds"] = json!((d * 1000.0).round() / 1000.0);
        }

        ToolResult::Success {
            data: payload.to_string(),
            metadata: HashMap::new(),
        }
    }
}

/// 质量档 → Manim 渲染标志。
fn quality_flag(quality: &str) -> Option<&'static str> {
    match quality {
        "low" => Some("-ql"),
        "medium" => Some("-qm"),
        "high" => Some("-qh"),
        "production" => Some("-qk"),
        _ => None,
    }
}

/// 组装 `manim render` 的完整 argv（第 0 项是解释器本身）。
///
/// 纯函数：单测直接断言顺序，不启动进程。
fn build_command(
    python: &Path,
    flag: &str,
    media_dir: &Path,
    scene_py: &Path,
    scene_class: &str,
) -> Vec<String> {
    vec![
        python.to_string_lossy().into_owned(),
        "-m".to_string(),
        "manim".to_string(),
        "render".to_string(),
        flag.to_string(),
        "--format=mp4".to_string(),
        "--media_dir".to_string(),
        media_dir.to_string_lossy().into_owned(),
        scene_py.to_string_lossy().into_owned(),
        scene_class.to_string(),
    ]
}

/// 按优先级挑选 Python 解释器（参数全部注入，便于单测）。
///
/// 顺序：显式参数 → 环境变量 `MARGUERITE_PYTHON` → Marguerite 托管虚拟环境
/// （`%LOCALAPPDATA%\Marguerite\python\manim\Scripts\python.exe`）→ PATH 中第一个存在的
/// `python.exe` / `python3.exe`（Windows 优先 `python.exe`）。只接受真实存在的文件。
///
/// 托管环境排第三：用户显式指定永远最大；系统 Python 常没有 manim（且新版 Python
/// 可能没有 manim 轮子），而托管环境是随应用准备好的，命中就该用它。
fn pick_python(
    explicit: Option<&Path>,
    env: Option<&Path>,
    managed: Option<&Path>,
    path_dirs: &[PathBuf],
) -> Option<PathBuf> {
    if let Some(p) = explicit.filter(|p| p.is_file()) {
        return Some(p.to_path_buf());
    }
    if let Some(p) = env.filter(|p| p.is_file()) {
        return Some(p.to_path_buf());
    }
    if let Some(p) = managed.filter(|p| p.is_file()) {
        return Some(p.to_path_buf());
    }
    for dir in path_dirs {
        for name in python_candidates().iter().copied() {
            let cand = dir.join(name);
            if cand.is_file() {
                return Some(cand);
            }
        }
    }
    None
}

/// Marguerite 托管的 Manim 虚拟环境解释器（不存在则 `None`）。
///
/// 位置与设置页里「托管工具链目录」同一套约定：`%LOCALAPPDATA%\Marguerite\python\manim`。
fn managed_python() -> Option<PathBuf> {
    let base = std::env::var_os("LOCALAPPDATA").map(PathBuf::from)?;
    let cand = if cfg!(windows) {
        base.join("Marguerite")
            .join("python")
            .join("manim")
            .join("Scripts")
            .join("python.exe")
    } else {
        base.join("Marguerite")
            .join("python")
            .join("manim")
            .join("bin")
            .join("python")
    };
    cand.is_file().then_some(cand)
}

/// PATH 中需要尝试的解释器文件名（Windows 带 `.exe`，且 `python.exe` 优先）。
fn python_candidates() -> &'static [&'static str] {
    if cfg!(windows) {
        &["python.exe", "python3.exe"]
    } else {
        &["python", "python3"]
    }
}

/// 从源码里识别第一个 Manim 场景类。
///
/// 只认**同一行**内完成的 `class <Name>(<基类列表>):`，基类列表中任意一项的
/// 末尾类名等于 `Scene` 或以 `Scene` 结尾即算命中（覆盖 `Scene` / `ThreeDScene` /
/// `MovingCameraScene` 以及用户自定义的 `XxxScene`）。
///
/// 识别不到的写法（此时调用方必须显式给 `scene_class`）：
/// - 基类列表跨行书写（`class Foo(\n    Scene,\n):`）；
/// - 冒号不在基类列表同一行；
/// - 基类列表里含嵌套括号（如 `metaclass=type("x", (), {})`）；
/// - 动态生成（`type("Foo", (Scene,), {})`）；
/// - `class` 出现在行首的三引号字符串内部（没有做完整 Python 解析）。
fn detect_scene_class(src: &str) -> Option<String> {
    src.lines().find_map(parse_scene_class_line)
}

/// 解析单独一行，成功时返回场景类名。
fn parse_scene_class_line(line: &str) -> Option<String> {
    let trimmed = line.trim_start();
    // 注释行不是类定义（`from manim import Scene` 也不会以 `class` 开头）。
    if trimmed.starts_with('#') {
        return None;
    }
    let rest = trimmed.strip_prefix("class")?;
    // `class` 后必须是空白，避免把 `classify(...)` 之类当成类定义。
    if !rest.starts_with(|c: char| c.is_whitespace()) {
        return None;
    }
    let rest = rest.trim_start();
    let name_end = rest.find(|c: char| !(c.is_alphanumeric() || c == '_'))?;
    let name = &rest[..name_end];
    if name.is_empty() || name.starts_with(|c: char| c.is_numeric()) {
        return None;
    }

    let rest = rest[name_end..].trim_start();
    let inner_start = rest.strip_prefix('(')?;
    let inner_end = inner_start.find(')')?;
    let bases = &inner_start[..inner_end];
    let after = inner_start[inner_end + 1..].trim_start();
    // 基类列表后必须紧跟冒号，才算真正的 class 语句（允许行尾注释）。
    if !after.starts_with(':') {
        return None;
    }

    if bases.split(',').any(is_scene_base) {
        Some(name.to_string())
    } else {
        None
    }
}

/// 判断一个基类项是否是场景类（支持 `manim.Scene`、`VGroup, ThreeDScene` 等写法）。
fn is_scene_base(base: &str) -> bool {
    let head = base
        .split(['<', '['])
        .next()
        .unwrap_or(base)
        .trim();
    let last = head.rsplit(['.', ':']).next().unwrap_or(head).trim();
    last == "Scene" || last.ends_with("Scene")
}

/// 类名是否是可安全放进 argv 的 Python 标识符。
fn is_identifier(s: &str) -> bool {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) if c.is_alphabetic() || c == '_' => {}
        _ => return false,
    }
    chars.all(|c| c.is_alphanumeric() || c == '_')
}

/// 在 Manim 的媒体目录里找最新的 mp4。
///
/// Manim 的输出布局是 `<media_dir>/videos/<场景文件名>/<分辨率目录>/*.mp4`；
/// 这里从 `videos`（不存在时退回 `media_dir`）做有界递归，按 mtime 取最新，
/// mtime 相同时用路径字典序兜底，保证结果确定。
fn pick_output(media_dir: &Path) -> Option<PathBuf> {
    let videos = media_dir.join("videos");
    let root = if videos.is_dir() {
        videos
    } else {
        media_dir.to_path_buf()
    };
    let mut best: Option<(SystemTime, PathBuf)> = None;
    collect_newest_mp4(&root, 0, &mut best);
    best.map(|(_, p)| p)
}

fn collect_newest_mp4(dir: &Path, depth: usize, best: &mut Option<(SystemTime, PathBuf)>) {
    if depth > PICK_MAX_DEPTH {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let Ok(ft) = entry.file_type() else {
            continue;
        };
        let path = entry.path();
        if ft.is_dir() {
            collect_newest_mp4(&path, depth + 1, best);
            continue;
        }
        if !ft.is_file() {
            continue;
        }
        let is_mp4 = path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.eq_ignore_ascii_case("mp4"))
            .unwrap_or(false);
        if !is_mp4 {
            continue;
        }
        let mtime = entry
            .metadata()
            .ok()
            .and_then(|m| m.modified().ok())
            .unwrap_or(SystemTime::UNIX_EPOCH);
        let replace = match best {
            None => true,
            Some((best_time, best_path)) => {
                mtime > *best_time
                    || (mtime == *best_time && path.as_path() > best_path.as_path())
            }
        };
        if replace {
            *best = Some((mtime, path));
        }
    }
}

/// 子进程输出。
struct Captured {
    exit_code: i32,
    stdout: String,
    stderr: String,
}

/// 子进程失败原因（超时 / 启动失败 / 等待失败必须可区分）。
enum RunFailure {
    Timeout(String),
    Spawn(String),
    Wait(String),
}

/// 同步执行并捕获输出，用「轮询 try_wait」实现超时（与 `tools/media.rs` 一致）。
fn run_captured(program: &Path, args: &[String], timeout: Duration) -> Result<Captured, RunFailure> {
    let mut cmd = Command::new(program);
    cmd.silent();
    cmd.args(args);
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());
    let mut child = cmd.spawn().map_err(|e| {
        RunFailure::Spawn(format!("启动 Python 失败（{}）：{e}", program.display()))
    })?;

    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) => {
                if started.elapsed() > timeout {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(RunFailure::Timeout(format!(
                        "超过 {}s 未完成",
                        timeout.as_secs()
                    )));
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(e) => return Err(RunFailure::Wait(format!("等待子进程失败：{e}"))),
        }
    }

    let out = child
        .wait_with_output()
        .map_err(|e| RunFailure::Wait(format!("读取子进程输出失败：{e}")))?;
    Ok(Captured {
        exit_code: out.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
    })
}

/// 用 ffprobe 读时长；未内置 ffprobe 或解析失败时返回 None（调用方省略该字段）。
fn probe_duration(path: &Path) -> Option<f64> {
    let ffprobe = crate::tools::ffmpeg_bin::resolve("ffprobe")?;
    let args: Vec<String> = vec![
        "-v".to_string(),
        "error".to_string(),
        "-show_entries".to_string(),
        "format=duration".to_string(),
        "-of".to_string(),
        "default=noprint_wrappers=1:nokey=1".to_string(),
        path.to_string_lossy().into_owned(),
    ];
    let out = run_captured(&ffprobe, &args, Duration::from_secs(30)).ok()?;
    if out.exit_code != 0 {
        return None;
    }
    out.stdout
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())?
        .parse::<f64>()
        .ok()
        .filter(|d| d.is_finite() && *d > 0.0)
}

/// 取最后一个非空行（探测输出里可能夹着警告）。
fn last_nonempty_line(s: &str) -> String {
    s.lines()
        .map(str::trim)
        .rfind(|l| !l.is_empty())
        .unwrap_or("")
        .to_string()
}

/// 取字符串尾部（错误信息通常在末尾，且整段可能极长）。
fn tail(s: &str, max_chars: usize) -> String {
    let n = s.chars().count();
    if n <= max_chars {
        return s.trim().to_string();
    }
    let skip = n - max_chars;
    format!("…{}", s.chars().skip(skip).collect::<String>().trim())
}

fn no_python_message() -> String {
    "找不到可用的 Python 解释器，无法调用 Manim。\n\
     修复方式（任选其一）：\n\
     ① 安装 Python 3.11 / 3.12（安装时勾选“Add python.exe to PATH”）；\n\
     ② 用参数 python 指定解释器绝对路径，例如 C:\\Python312\\python.exe；\n\
     ③ 设置环境变量 MARGUERITE_PYTHON 指向该解释器。\n\
     注意：Manim 对 Python 3.14 的依赖轮子尚不完整，建议使用 3.11 / 3.12。"
        .to_string()
}

fn no_manim_message(python: &Path, detail: &str) -> String {
    let mut msg = format!(
        "已找到 Python（{}），但它没有安装 Manim，无法渲染。\n\
         安装方式（任选其一）：\n\
         ① 基础安装：python -m pip install manim\n\
         ② 需要 Manim 的交互式预览窗口：pip install manim[gui]\n\
         重要限制：当前系统是 Python 3.14，Manim 依赖的 pycairo / manimpango 等\
         可能暂无对应的预编译轮子，安装容易失败；建议改用 Python 3.11 / 3.12\
         （可用本工具的 python 参数或环境变量 MARGUERITE_PYTHON 指向该解释器）。\n\
         安装后自检：python -c \"import manim\"",
        python.display()
    );
    if !detail.trim().is_empty() {
        msg.push_str(&format!(
            "\nPython 报告的错误（截断至 800 字符）：{}",
            tail(detail.trim(), 800)
        ));
    }
    msg
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_root(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "mq-manim-{}-{}-{tag}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).expect("建临时目录");
        dir
    }

    #[test]
    fn detect_scene_class_covers_three_base_classes() {
        let plain = "from manim import *\n\nclass Intro(Scene):\n    def construct(self):\n        pass\n";
        assert_eq!(detect_scene_class(plain).as_deref(), Some("Intro"));

        let three_d = "class Rotate(ThreeDScene):\n    pass\n";
        assert_eq!(detect_scene_class(three_d).as_deref(), Some("Rotate"));

        let moving = "class Follow(MovingCameraScene):\n    pass\n";
        assert_eq!(detect_scene_class(moving).as_deref(), Some("Follow"));

        // 点分基类 / 多基类同样命中
        assert_eq!(
            detect_scene_class("class A(manim.MovingCameraScene):\n    pass\n").as_deref(),
            Some("A")
        );
        assert_eq!(
            detect_scene_class("class B(VGroup, ThreeDScene):\n    pass\n").as_deref(),
            Some("B")
        );
    }

    #[test]
    fn detect_scene_class_takes_first_match_and_ignores_noise() {
        let src = "# class Commented(Scene):\n\
                   from manim import Scene\n\
                   class Helper(VMobject):\n    pass\n\
                   class First(Scene):\n    pass\n\
                   class Second(ThreeDScene):\n    pass\n";
        assert_eq!(detect_scene_class(src).as_deref(), Some("First"));

        // 非 Scene 基类 / 缺冒号 / 只有 import 都不算命中
        assert_eq!(
            detect_scene_class("from manim import Scene\nclass Helper(VMobject):\n    pass\n"),
            None
        );
        assert_eq!(detect_scene_class("class Broken(Scene)\n    pass\n"), None);
        assert_eq!(detect_scene_class("classify(Scene):"), None);

        // 基类列表跨行：识别不到（调用方应显式传 scene_class）
        assert_eq!(detect_scene_class("class Multi(\n    Scene,\n):\n    pass\n"), None);
    }

    #[test]
    fn pick_python_prefers_explicit_then_env_then_path() {
        let root = tmp_root("pick-python");
        let explicit = root.join("explicit-python.exe");
        let from_env = root.join("env-python.exe");
        let path_dir = root.join("path");
        std::fs::create_dir_all(&path_dir).unwrap();
        std::fs::write(&explicit, b"x").unwrap();
        std::fs::write(&from_env, b"x").unwrap();
        // PATH 目录里同时放两个候选，验证 `python.exe` 优先于 `python3.exe`
        for name in python_candidates().iter().copied() {
            std::fs::write(path_dir.join(name), b"x").unwrap();
        }
        let expected_on_path = path_dir.join(python_candidates()[0]);
        let managed = root.join("managed-python.exe");
        std::fs::write(&managed, b"x").unwrap();

        assert_eq!(
            pick_python(
                Some(&explicit),
                Some(&from_env),
                Some(&managed),
                std::slice::from_ref(&path_dir)
            ),
            Some(explicit.clone()),
            "显式参数必须最优先"
        );
        assert_eq!(
            pick_python(
                None,
                Some(&from_env),
                Some(&managed),
                std::slice::from_ref(&path_dir)
            ),
            Some(from_env.clone()),
            "环境变量必须压过托管环境与 PATH"
        );
        assert_eq!(
            pick_python(None, None, Some(&managed), std::slice::from_ref(&path_dir)),
            Some(managed.clone()),
            "托管虚拟环境必须压过 PATH（系统 Python 常没有 manim）"
        );
        assert_eq!(
            pick_python(None, None, None, std::slice::from_ref(&path_dir)),
            Some(expected_on_path),
            "PATH 内优先 python.exe"
        );
        assert_eq!(pick_python(None, None, None, &[root.join("absent")]), None);

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn build_command_has_stable_argv_order() {
        let argv = build_command(
            Path::new(r"C:\Python312\python.exe"),
            "-qm",
            Path::new(r"C:\tmp\manim\media"),
            Path::new(r"C:\tmp\manim\scene.py"),
            "Demo",
        );
        assert_eq!(
            argv,
            vec![
                r"C:\Python312\python.exe".to_string(),
                "-m".to_string(),
                "manim".to_string(),
                "render".to_string(),
                "-qm".to_string(),
                "--format=mp4".to_string(),
                "--media_dir".to_string(),
                r"C:\tmp\manim\media".to_string(),
                r"C:\tmp\manim\scene.py".to_string(),
                "Demo".to_string(),
            ]
        );
    }

    #[test]
    fn quality_maps_to_manim_flags() {
        assert_eq!(quality_flag("low"), Some("-ql"));
        assert_eq!(quality_flag("medium"), Some("-qm"));
        assert_eq!(quality_flag("high"), Some("-qh"));
        assert_eq!(quality_flag("production"), Some("-qk"));
        assert_eq!(quality_flag("ultra"), None);
    }

    #[test]
    fn pick_output_takes_newest_mp4_from_manim_layout() {
        let root = tmp_root("pick-output");
        let media = root.join("media");
        let low = media.join("videos").join("scene").join("480p15");
        let high = media.join("videos").join("scene").join("1080p60");
        std::fs::create_dir_all(&low).unwrap();
        std::fs::create_dir_all(&high).unwrap();

        let older = low.join("a.mp4");
        let newest = high.join("z.mp4");
        std::fs::write(&older, b"old").unwrap();
        // 确保 mtime 有可分辨的先后；即便时间粒度相同，字典序兜底仍会选 z.mp4。
        std::thread::sleep(Duration::from_millis(20));
        std::fs::write(&newest, b"new").unwrap();
        // 非 mp4 不得参与挑选
        std::fs::write(high.join("poster.png"), b"png").unwrap();

        assert_eq!(pick_output(&media), Some(newest));
        assert_eq!(pick_output(&root.join("does-not-exist")), None);

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn no_manim_message_has_install_commands_and_py314_limit() {
        let msg = no_manim_message(
            Path::new(r"C:\Python314\python.exe"),
            "ModuleNotFoundError: No module named 'manim'",
        );
        assert!(msg.contains("python -m pip install manim"), "{msg}");
        assert!(msg.contains("pip install manim[gui]"), "{msg}");
        assert!(msg.contains("Python 3.14"), "{msg}");
        assert!(msg.contains("3.11"), "{msg}");
        assert!(msg.contains("3.12"), "{msg}");
        assert!(msg.contains("ModuleNotFoundError"), "{msg}");
    }

    /// 端到端：真跑一次 Manim 渲染（默认忽略，需要本机 Python + manim）。
    ///
    /// 用 `cargo test --lib -- --ignored real_manim_end_to_end` 显式执行。
    #[tokio::test]
    #[ignore = "需要本机 Python + manim；用 --ignored 显式运行"]
    async fn real_manim_end_to_end() {
        let dir = std::env::temp_dir().join(format!(
            "mq-manim-e2e-{}-{}",
            std::process::id(),
            chrono::Local::now().timestamp_nanos_opt().unwrap_or(0)
        ));
        std::fs::create_dir_all(&dir).expect("建工作区");
        let tool = ManimVideoTool::new(dir.clone());
        let mut args = HashMap::new();
        args.insert(
            "scene".to_string(),
            "from manim import *\n\nclass Hello(Scene):\n    def construct(self):\n        \
             self.play(Write(Text(\"Hi\").scale(0.6)), run_time=0.6)\n"
                .to_string(),
        );
        args.insert("out".to_string(), "e2e.mp4".to_string());
        args.insert("quality".to_string(), "low".to_string());
        args.insert("timeout_seconds".to_string(), "420".to_string());

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
                assert!(data.contains("Hello"), "返回体应含识别到的场景类: {data}");
            }
            ToolResult::Error { code, message } => {
                panic!("Manim 端到端失败（{code}）：{message}")
            }
            other => panic!("端到端返回了非预期结果：{other:?}"),
        }
    }
}
