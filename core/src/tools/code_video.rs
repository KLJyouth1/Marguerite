//! 代码打字动画工具（`code_video`）：源码 → 「抖音程序员浪漫」风格 mp4。
//!
//! # 为什么需要（2026-10-24）
//!
//! 视频能力层此前只有两条路：云端 `video_generate`（要模型/网络）与 `media`
//! （转码已有素材）。用户想发一条「把源码逐字打出来」的短视频时，智能体只能
//! 建议去装剪映、录屏或用第三方网站 —— 这既不可复现，也无法交付到工作区。
//!
//! 本工具把这条路径收成一次结构化调用：**纯 Rust 字形栅格化 + PPM 直通 ffmpeg**，
//! 不依赖浏览器、不依赖图形库、不联网，产物落到工作区并登记为产物。
//!
//! # 实现要点
//!
//! - 用 `ab_glyph` 把字形栅格化进 RGB 帧缓冲，逐帧写 **PPM(P6)**，
//!   ffmpeg 原生支持 ppm 序列，因此不需要任何图片编码依赖；
//! - 只画「已经打出来的字符」：未显示部分**完全不画**（不是画成灰色），
//!   避免观众误以为代码已经写好；光标是实心方块，每 0.5s 闪一次；
//! - 行数超过可视高度时整体上滚，保证光标行始终可见；
//! - 帧序列写在系统临时目录的唯一子目录里，无论成败都由 RAII 守卫删除。
//!
//! # 诚实边界
//!
//! - **语法高亮是启发式的**：按语言匹配关键字 / 字符串 / 注释 / 数字 / 函数名
//!   五类（见 [`classify`]），不做语法树解析。未知语言一律按 `plain` 处理，
//!   不猜语义（正则字面量、宏、模板字符串里的代码等都可能着色不完美）；
//! - **不做自动换行**：超长行会被画布右边界裁掉。需要完整展示时请自行折行；
//! - **CJK 回退有限**：主字体缺字时依次尝试 `msyh.ttc` / `simsun.ttc` /
//!   `msjh.ttc`；这些字体也不存在时，汉字会显示为方框（本工具不谎称支持）；
//! - **PPM 直通有磁盘代价**：临时帧占用约为 `宽 × 高 × 3 × 帧数`
//!   （720p / 3 分钟约 14GB），因此帧数硬上限为 `fps × 180`，超限明确报错，
//!   绝不静默截断。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use ab_glyph::{point, Font, FontVec, GlyphId, PxScale, ScaleFont};
use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};

use crate::agent::tool_cycle::ToolResult;
use crate::mcp::registry::ToolExecutor;
use crate::proc::SilentCommand;
use crate::tools::ffmpeg_bin;
use crate::tools::file_change::{record_artifact, GENERATED_KIND};
use crate::tools::fs_path::RootSet;

/// 输入代码的行数上限。
const MAX_CODE_LINES: usize = 400;
/// 输入代码的字符上限。
const MAX_CODE_CHARS: usize = 20_000;
/// 成片时长上限（秒）：超过即拒绝，不做静默截断。
const MAX_DURATION_SECONDS: u32 = 180;

const MIN_WIDTH: u32 = 320;
const MAX_WIDTH: u32 = 3840;
const MIN_HEIGHT: u32 = 240;
const MAX_HEIGHT: u32 = 2160;
const MIN_FPS: u32 = 12;
const MAX_FPS: u32 = 60;
const MIN_CPS: u32 = 4;
const MAX_CPS: u32 = 200;
const MIN_FONT_SIZE: f32 = 10.0;
const MAX_FONT_SIZE: f32 = 48.0;

/// 颜色类别：纯正文。
const COLOR_PLAIN: u8 = 0;
/// 颜色类别：关键字。
const COLOR_KEYWORD: u8 = 1;
/// 颜色类别：字符串。
const COLOR_STRING: u8 = 2;
/// 颜色类别：注释。
const COLOR_COMMENT: u8 = 3;
/// 颜色类别：数字。
const COLOR_NUMBER: u8 = 4;
/// 颜色类别：函数名。
const COLOR_FUNCTION: u8 = 5;

/// `code_video` 工具。
pub struct CodeVideoTool {
    workspace: RootSet,
}

impl CodeVideoTool {
    pub fn new(workspace: impl Into<RootSet>) -> Self {
        let workspace: RootSet = workspace.into();
        Self { workspace }
    }
}

#[async_trait]
impl ToolExecutor for CodeVideoTool {
    fn name(&self) -> &str {
        "code_video"
    }

    fn description(&self) -> &str {
        "把一段源码渲染成「代码打字动画」mp4（1280×720、深色/浅色主题、逐字符出现、\
         带行号与启发式语法高亮）。纯本地渲染，不依赖浏览器/图形库；需要 ffmpeg 合成。\
         code 必填（最多 400 行 / 20000 字符）；language 支持 rust/python/js/ts/go/shell/json/plain；\
         out 默认 generated/videos/code-<时间戳>.mp4。\
         代码过长或预计超过 3 分钟时会明确报错，请拆分或调大 chars_per_second。"
    }

    fn input_schema(&self) -> Value {
        json!({"type":"object","properties":{
            "code":{"type":"string","description":"要逐字符打出的源码（最多 400 行 / 20000 字符，超出请拆分）"},
            "language":{"type":"string","description":"语言：rust/python/js/ts/go/shell/json/plain（默认 plain；未知语言按 plain 处理）"},
            "out":{"type":"string","description":"输出 mp4 的工作区相对路径（默认 generated/videos/code-<时间戳>.mp4）"},
            "title":{"type":"string","description":"顶部窗口条标题（默认 main.rs）"},
            "width":{"type":"number","description":"画布宽度，默认 1280（320–3840）"},
            "height":{"type":"number","description":"画布高度，默认 720（240–2160）"},
            "fps":{"type":"number","description":"帧率，默认 30（12–60）"},
            "chars_per_second":{"type":"number","description":"每秒打出的字符数，默认 28（4–200）"},
            "theme":{"type":"string","enum":["dark","light"],"description":"配色主题，默认 dark"},
            "font":{"type":"string","description":"等宽字体绝对路径（.ttf/.otf/.ttc）；默认依次找 Consolas / Cascadia Mono / Lucida Console / Segoe UI"},
            "font_size":{"type":"number","description":"正文字号，默认 18（10–48）"},
            "highlight":{"type":"boolean","description":"是否启用启发式语法高亮，默认 true"},
            "tail_seconds":{"type":"number","description":"打完后静置秒数，默认 2.5"}
        },"required":["code"]})
    }

    async fn execute(&self, args: &HashMap<String, String>) -> Result<ToolResult> {
        Ok(match self.run(args) {
            Ok(result) => result,
            Err(e) => ToolResult::Error {
                message: e.message,
                code: e.code,
            },
        })
    }
}

/// 工具内部错误（消息 + 稳定错误码），在 `execute` 里统一转成 [`ToolResult::Error`]。
#[derive(Debug)]
struct CodeVideoError {
    code: String,
    message: String,
}

impl CodeVideoError {
    fn new(code: &str, message: impl Into<String>) -> Self {
        Self {
            code: code.to_string(),
            message: message.into(),
        }
    }
}

type Outcome<T> = std::result::Result<T, CodeVideoError>;

/// 一次渲染的完整参数（解析并校验后的形态）。
#[derive(Debug, Clone)]
struct VideoSpec {
    code: String,
    language: String,
    out: String,
    title: String,
    width: u32,
    height: u32,
    fps: u32,
    chars_per_second: u32,
    theme_dark: bool,
    font: Option<String>,
    font_size: f32,
    highlight: bool,
    tail_seconds: f64,
}

impl CodeVideoTool {
    /// 工具主流程：校验 → 解析字体 → 逐帧渲染 → ffmpeg 合成 → 登记产物。
    ///
    /// 全部同步执行（与 `tools/media.rs` 一致），不引入 `spawn_blocking` / `block_on`。
    fn run(&self, args: &HashMap<String, String>) -> Outcome<ToolResult> {
        let spec = parse_spec(args)?;

        // 行尾 `\r\n` 由 `str::lines` 统一处理；制表符展开成 4 个空格，
        // 否则等宽字体下 tab 的宽度与行号对齐都不可控。
        let lines: Vec<String> = spec.code.lines().map(|l| l.replace('\t', "    ")).collect();
        let line_lens: Vec<usize> = lines.iter().map(|l| l.chars().count()).collect();
        let total_chars: usize = line_lens.iter().sum();
        // 换行也占一个「打字步」，让跨行切换有节奏，而不是瞬间跳行。
        let total_steps = total_chars + line_lens.len().saturating_sub(1);

        let frames = frame_count(
            total_steps,
            spec.chars_per_second,
            spec.tail_seconds,
            spec.fps,
        );
        let budget = spec.fps.saturating_mul(MAX_DURATION_SECONDS);
        if frames > budget {
            return Err(CodeVideoError::new(
                "TOO_MANY_FRAMES",
                format!(
                    "预计 {frames} 帧，超过 {MAX_DURATION_SECONDS} 秒上限（{budget} 帧 @ {}fps）。\
                     请把 chars_per_second 调大（如 60–120）、缩短代码，或降低 fps / tail_seconds 后重试。",
                    spec.fps
                ),
            ));
        }

        // 字体与 ffmpeg 都先解析：缺任何一样都不必先花几分钟写帧。
        let font_path = pick_font_path(spec.font.as_deref(), &default_font_candidates())?;
        let mut fonts = FontSet::load(&font_path, spec.font_size)?;
        let ffmpeg = ffmpeg_bin::resolve_or_error("ffmpeg")
            .map_err(|m| CodeVideoError::new("NO_FFMPEG", m))?;

        // 输出路径必须落在工作区内。
        let target = self.workspace.resolve(&spec.out).map_err(|e| {
            CodeVideoError::new(
                "INVALID_PATH",
                format!("输出路径非法（必须落在工作区内）：{e}"),
            )
        })?;
        if let Some(dir) = target.parent() {
            std::fs::create_dir_all(dir)
                .map_err(|e| CodeVideoError::new("IO_ERROR", format!("创建输出目录失败：{e}")))?;
        }

        // 高亮分词只做一次；帧循环里只做「画到第几个字符」。
        let tokens: Vec<Vec<(String, u8)>> = lines
            .iter()
            .map(|line| {
                if line.is_empty() {
                    Vec::new()
                } else if spec.highlight {
                    classify(&spec.language, line)
                } else {
                    vec![(line.clone(), COLOR_PLAIN)]
                }
            })
            .collect();

        // 预热字形缓存：正文、标题、行号数字与光标宽度字符。
        for line in &lines {
            fonts.prepare(line);
        }
        fonts.prepare(&spec.title);
        fonts.prepare("0123456789M ");

        let layout = Layout::new(&spec, &fonts, lines.len());
        let scene = Scene {
            layout,
            palette: Palette::new(spec.theme_dark),
            fonts: &fonts,
            title: &spec.title,
            lines: &lines,
            line_lens: &line_lens,
            tokens: &tokens,
        };

        // 唯一临时目录：系统临时目录下，守卫保证无论成败都删除。
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let temp =
            std::env::temp_dir().join(format!("mq-codevideo-{}-{}", std::process::id(), nanos));
        std::fs::create_dir_all(&temp)
            .map_err(|e| CodeVideoError::new("IO_ERROR", format!("创建临时目录失败：{e}")))?;
        let _guard = TempDirGuard(temp.clone());

        for i in 0..frames {
            // 用「本帧结束时刻」取打字进度，保证 tail_seconds=0 时最后一帧也能打完整。
            let elapsed = (i as f64 + 1.0) / spec.fps as f64;
            let typed = typed_steps_at(elapsed, spec.chars_per_second, total_steps);
            let rgb = scene.render(typed, elapsed);
            let path = temp.join(frame_name(i));
            write_ppm(&path, spec.width as usize, spec.height as usize, &rgb).map_err(|e| {
                CodeVideoError::new("IO_ERROR", format!("写入第 {} 帧失败：{e}", i + 1))
            })?;
        }

        // ffmpeg 合成：工作目录切到临时目录，输入用相对模式串，避免 Windows 路径转义。
        let mut cmd = Command::new(&ffmpeg);
        cmd.silent();
        cmd.args(ffmpeg_args(spec.fps, &target))
            .current_dir(&temp)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped());
        let output = cmd.output().map_err(|e| {
            CodeVideoError::new(
                "FFMPEG_FAILED",
                format!(
                    "执行 ffmpeg 失败：{e}\n\n{}",
                    ffmpeg_bin::missing_message("ffmpeg")
                ),
            )
        })?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(CodeVideoError::new(
                "FFMPEG_FAILED",
                format!(
                    "ffmpeg 合成失败（退出码 {:?}）：\n{}\n\n\
                     可执行修复：确认磁盘空间充足、输出路径未被占用，或改用其它 out 路径重试。",
                    output.status.code(),
                    tail(stderr.trim(), 800)
                ),
            ));
        }

        let bytes = std::fs::metadata(&target).map(|m| m.len()).unwrap_or(0);
        if bytes == 0 {
            return Err(CodeVideoError::new(
                "FFMPEG_FAILED",
                "ffmpeg 报告成功但输出文件为空；请检查输出路径是否可写（或被其它程序占用）。",
            ));
        }
        record_artifact("", &target.display().to_string(), GENERATED_KIND);

        let duration_seconds = ((frames as f64 / spec.fps as f64) * 100.0).round() / 100.0;
        Ok(ToolResult::Success {
            data: json!({
                "ok": true,
                "path": target.display().to_string(),
                "frames": frames,
                "durationSeconds": duration_seconds,
                "bytes": bytes,
                "language": spec.language,
                "charsPerSecond": spec.chars_per_second,
                "width": spec.width,
                "height": spec.height,
                "fps": spec.fps,
                "font": font_path.display().to_string(),
            })
            .to_string(),
            metadata: HashMap::new(),
        })
    }
}

// ───────────────────────────── 纯函数（可单测） ─────────────────────────────

/// 语言名归一化；未知语言一律归到 `plain`（不猜语义）。
fn normalize_lang(raw: &str) -> &'static str {
    match raw.trim().to_ascii_lowercase().as_str() {
        "rust" | "rs" => "rust",
        "python" | "py" => "python",
        "js" | "javascript" | "node" => "js",
        "ts" | "typescript" => "ts",
        "go" | "golang" => "go",
        "shell" | "sh" | "bash" | "zsh" | "powershell" | "ps1" => "shell",
        "json" => "json",
        "plain" | "text" | "txt" => "plain",
        _ => "plain",
    }
}

/// 语法高亮分类：返回 `(token 文本, 颜色类别)`，按顺序拼接后必须等于原行。
///
/// 规则是**启发式**的：先识别注释与字符串（避免把注释里的引号当字符串），
/// 再识别数字与标识符；标识符命中关键字表 → 关键字，
/// 后面紧跟 `(` 或前一个词是 `fn/def/function/func/class` → 函数名。
fn classify(lang: &str, line: &str) -> Vec<(String, u8)> {
    let lang = normalize_lang(lang);
    if lang == "plain" {
        return if line.is_empty() {
            Vec::new()
        } else {
            vec![(line.to_string(), COLOR_PLAIN)]
        };
    }

    let chars: Vec<char> = line.chars().collect();
    let mut out: Vec<(String, u8)> = Vec::new();
    let mut prev_word: Option<String> = None;
    let mut i = 0usize;

    while i < chars.len() {
        let c = chars[i];

        // ① 行注释：命中后整行剩余部分都是注释。
        if let Some(marker) = comment_marker(lang) {
            if starts_with_chars(&chars, i, marker) {
                out.push((chars[i..].iter().collect(), COLOR_COMMENT));
                break;
            }
        }

        // ② 字符串：消费到同名引号（支持反斜杠转义；未闭合则到行尾）。
        if is_string_quote(lang, c) {
            let start = i;
            i += 1;
            while i < chars.len() {
                if chars[i] == '\\' {
                    i += 2;
                    continue;
                }
                if chars[i] == c {
                    i += 1;
                    break;
                }
                i += 1;
            }
            let end = i.min(chars.len());
            out.push((chars[start..end].iter().collect(), COLOR_STRING));
            prev_word = None;
            i = end;
            continue;
        }

        // ③ 数字：吞掉小数/下划线/进制前缀（`0x1F`、`1_000`、`3.14`）。
        if c.is_ascii_digit() {
            let start = i;
            i += 1;
            while i < chars.len()
                && (chars[i].is_ascii_alphanumeric() || chars[i] == '.' || chars[i] == '_')
            {
                i += 1;
            }
            out.push((chars[start..i].iter().collect(), COLOR_NUMBER));
            prev_word = None;
            continue;
        }

        // ④ 标识符：关键字 / 函数名 / 正文。
        if is_ident_start(c) {
            let start = i;
            i += 1;
            while i < chars.len() && is_ident_continue(chars[i]) {
                i += 1;
            }
            let word: String = chars[start..i].iter().collect();
            let followed_by_paren = next_non_space(&chars, i) == Some('(');
            let declared = matches!(
                prev_word.as_deref(),
                Some("fn" | "def" | "function" | "func" | "class")
            );
            let cls = if is_keyword(lang, &word) {
                COLOR_KEYWORD
            } else if followed_by_paren || declared {
                COLOR_FUNCTION
            } else {
                COLOR_PLAIN
            };
            out.push((word.clone(), cls));
            prev_word = Some(word);
            continue;
        }

        // ⑤ 空白：单独成 token，且不影响「前一个词」的判断（`fn  foo` 也要认函数名）。
        if c.is_whitespace() {
            let start = i;
            while i < chars.len() && chars[i].is_whitespace() {
                i += 1;
            }
            out.push((chars[start..i].iter().collect(), COLOR_PLAIN));
            continue;
        }

        // ⑥ 其它符号：连续符号合成一个 token（遇到注释/字符串/标识符/数字即停）。
        let start = i;
        i += 1;
        while i < chars.len() {
            let n = chars[i];
            if n.is_whitespace()
                || is_ident_start(n)
                || n.is_ascii_digit()
                || is_string_quote(lang, n)
            {
                break;
            }
            if let Some(marker) = comment_marker(lang) {
                if starts_with_chars(&chars, i, marker) {
                    break;
                }
            }
            i += 1;
        }
        out.push((chars[start..i].iter().collect(), COLOR_PLAIN));
        prev_word = None;
    }

    out
}

/// 各语言的单行注释前缀（json 没有注释）。
fn comment_marker(lang: &str) -> Option<&'static str> {
    match lang {
        "rust" | "js" | "ts" | "go" => Some("//"),
        "python" | "shell" => Some("#"),
        _ => None,
    }
}

/// 哪些字符可以开启字符串 token。
///
/// Rust 的 `'` 同时是生命周期/字符字面量前缀，按字符串处理会误吞整行，
/// 因此这里只把 `"` 视作字符串起点 —— 这是刻意的保守取舍。
fn is_string_quote(lang: &str, c: char) -> bool {
    match lang {
        "rust" => c == '"',
        "python" => c == '"' || c == '\'',
        "js" | "ts" => c == '"' || c == '\'' || c == '`',
        "go" => c == '"' || c == '\'' || c == '`',
        "shell" => c == '"' || c == '\'' || c == '`',
        "json" => c == '"',
        _ => false,
    }
}

fn is_ident_start(c: char) -> bool {
    c.is_alphabetic() || c == '_' || c == '$'
}

fn is_ident_continue(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '$'
}

fn starts_with_chars(chars: &[char], at: usize, pat: &str) -> bool {
    for (j, pc) in (at..).zip(pat.chars()) {
        if chars.get(j) != Some(&pc) {
            return false;
        }
    }
    true
}

fn next_non_space(chars: &[char], from: usize) -> Option<char> {
    chars
        .iter()
        .skip(from)
        .copied()
        .find(|c| !c.is_whitespace())
}

fn is_keyword(lang: &str, word: &str) -> bool {
    let keywords: &[&str] = match lang {
        "rust" => &[
            "as", "async", "await", "break", "const", "continue", "crate", "dyn", "else", "enum",
            "extern", "false", "fn", "for", "if", "impl", "in", "let", "loop", "match", "mod",
            "move", "mut", "pub", "ref", "return", "self", "Self", "static", "struct", "super",
            "trait", "true", "type", "unsafe", "use", "where", "while", "yield",
        ],
        "python" => &[
            "and", "as", "assert", "async", "await", "break", "class", "continue", "def", "del",
            "elif", "else", "except", "False", "finally", "for", "from", "global", "if", "import",
            "in", "is", "lambda", "None", "nonlocal", "not", "or", "pass", "raise", "return",
            "True", "try", "while", "with", "yield",
        ],
        "js" | "ts" => &[
            "as",
            "async",
            "await",
            "break",
            "case",
            "catch",
            "class",
            "const",
            "continue",
            "debugger",
            "default",
            "delete",
            "do",
            "else",
            "enum",
            "export",
            "extends",
            "false",
            "finally",
            "for",
            "from",
            "function",
            "get",
            "if",
            "implements",
            "import",
            "in",
            "instanceof",
            "interface",
            "let",
            "new",
            "null",
            "of",
            "private",
            "protected",
            "public",
            "readonly",
            "return",
            "set",
            "static",
            "super",
            "switch",
            "this",
            "throw",
            "true",
            "try",
            "type",
            "typeof",
            "undefined",
            "var",
            "void",
            "while",
            "with",
            "yield",
        ],
        "go" => &[
            "break",
            "case",
            "chan",
            "const",
            "continue",
            "default",
            "defer",
            "else",
            "fallthrough",
            "false",
            "for",
            "func",
            "go",
            "goto",
            "if",
            "import",
            "interface",
            "iota",
            "map",
            "nil",
            "package",
            "range",
            "return",
            "select",
            "struct",
            "switch",
            "true",
            "type",
            "var",
        ],
        "shell" => &[
            "alias", "case", "cd", "do", "done", "echo", "elif", "else", "esac", "exit", "export",
            "false", "fi", "for", "function", "if", "in", "local", "readonly", "return", "set",
            "shift", "source", "test", "then", "true", "unalias", "unset", "until", "while",
        ],
        "json" => &["true", "false", "null"],
        _ => &[],
    };
    keywords.contains(&word)
}

/// 总帧数：打字时长 + 结尾静置，向上取整，至少 1 帧。
fn frame_count(total_steps: usize, chars_per_second: u32, tail_seconds: f64, fps: u32) -> u32 {
    let cps = chars_per_second.max(1) as f64;
    let typing_seconds = total_steps as f64 / cps;
    let total_seconds = typing_seconds + tail_seconds.max(0.0);
    ((total_seconds * fps as f64).ceil() as u32).max(1)
}

/// 已打出的步数（字符数 + 换行步数），不超过总步数。
fn typed_steps_at(elapsed_seconds: f64, chars_per_second: u32, total_steps: usize) -> usize {
    if elapsed_seconds <= 0.0 {
        return 0;
    }
    ((elapsed_seconds * chars_per_second as f64).floor() as usize).min(total_steps)
}

/// 把「已打出步数」映射为光标所在 `(行号, 列号)`；换行自身占一步。
fn cursor_position(steps: usize, line_lens: &[usize]) -> (usize, usize) {
    let mut left = steps;
    for (line, &len) in line_lens.iter().enumerate() {
        if left <= len {
            return (line, left);
        }
        left -= len + 1;
    }
    let last = line_lens.len().saturating_sub(1);
    (last, line_lens.get(last).copied().unwrap_or(0))
}

/// 可视窗口的顶部行号：保证光标行始终可见。
fn scroll_offset(cursor_line: usize, visible_lines: usize, total_lines: usize) -> usize {
    if visible_lines == 0 || total_lines <= visible_lines {
        return 0;
    }
    let max_top = total_lines - visible_lines;
    cursor_line.saturating_sub(visible_lines - 1).min(max_top)
}

/// 光标每 0.5 秒切换一次亮灭。
fn blink_on(elapsed_seconds: f64) -> bool {
    if elapsed_seconds < 0.0 {
        return true;
    }
    ((elapsed_seconds / 0.5).floor() as u64).is_multiple_of(2)
}

/// PPM(P6) 文件头：`P6\n{w} {h}\n255\n`。
fn ppm_header(width: usize, height: usize) -> String {
    format!("P6\n{width} {height}\n255\n")
}

/// ffmpeg 帧文件名（从 1 开始，ffmpeg 会在默认的 start_number 范围内自动找到它）。
fn frame_name(index: u32) -> String {
    format!("frame_{:05}.ppm", index + 1)
}

/// ffmpeg 合成参数（纯函数，便于断言「没有额外注入面」）。
fn ffmpeg_args(fps: u32, out: &Path) -> Vec<String> {
    vec![
        "-y".to_string(),
        "-framerate".to_string(),
        fps.to_string(),
        "-i".to_string(),
        "frame_%05d.ppm".to_string(),
        "-c:v".to_string(),
        "libx264".to_string(),
        "-pix_fmt".to_string(),
        "yuv420p".to_string(),
        "-movflags".to_string(),
        "+faststart".to_string(),
        out.to_string_lossy().into_owned(),
    ]
}

/// 输入规模校验：空代码、超过行数/字符上限都明确拒绝。
fn validate_code(code: &str) -> Outcome<()> {
    if code.trim().is_empty() {
        return Err(CodeVideoError::new(
            "INVALID_PARAMS",
            "code_video 需要非空的 code（要逐字符打出的源码）",
        ));
    }
    let lines = code.lines().count();
    let chars = code.chars().count();
    if lines > MAX_CODE_LINES || chars > MAX_CODE_CHARS {
        return Err(CodeVideoError::new(
            "CODE_TOO_LONG",
            format!(
                "代码过长（{lines} 行 / {chars} 字符），上限 {MAX_CODE_LINES} 行 / \
                 {MAX_CODE_CHARS} 字符。请拆分成多段分别生成，或精简后再试。"
            ),
        ));
    }
    Ok(())
}

/// 默认等宽字体候选（Windows 常见路径，按可读性与等宽程度排序）。
fn default_font_candidates() -> Vec<PathBuf> {
    [
        r"C:\Windows\Fonts\consola.ttf",
        r"C:\Windows\Fonts\CascadiaMono.ttf",
        r"C:\Windows\Fonts\lucon.ttf",
        r"C:\Windows\Fonts\segoeui.ttf",
    ]
    .iter()
    .map(PathBuf::from)
    .collect()
}

/// CJK 回退字体候选（主字体缺字时才尝试；加载失败会被忽略）。
fn cjk_fallback_candidates() -> Vec<PathBuf> {
    [
        r"C:\Windows\Fonts\msyh.ttc",
        r"C:\Windows\Fonts\simsun.ttc",
        r"C:\Windows\Fonts\msjh.ttc",
    ]
    .iter()
    .map(PathBuf::from)
    .collect()
}

/// 找不到字体时的用户可见文案：必须给出「怎么传 font」。
fn no_font_message() -> String {
    format!(
        "找不到可用字体。已依次查找：{}。\n\
         修复方式（任选其一）：\n\
         ① 显式传 font 参数，指向等宽字体文件的**绝对路径**，如 \
         \"C:\\\\Windows\\\\Fonts\\\\consola.ttf\"；\n\
         ② 安装 Consolas 或 Cascadia Mono（Windows Terminal 自带）；\n\
         ③ 传 .ttc 字体集合也可以（只使用第 0 个字面）。",
        default_font_candidates()
            .iter()
            .map(|p| p.display().to_string())
            .collect::<Vec<_>>()
            .join("、")
    )
}

/// 选择主字体：显式参数优先（必须存在），否则按默认列表找第一个存在的。
fn pick_font_path(explicit: Option<&str>, defaults: &[PathBuf]) -> Outcome<PathBuf> {
    if let Some(raw) = explicit.map(str::trim).filter(|s| !s.is_empty()) {
        let path = PathBuf::from(raw);
        if !path.is_absolute() {
            return Err(CodeVideoError::new(
                "NO_FONT",
                format!("font 必须是绝对路径，收到：{raw}。例如 \"C:\\\\Windows\\\\Fonts\\\\consola.ttf\"。"),
            ));
        }
        if !path.is_file() {
            return Err(CodeVideoError::new(
                "NO_FONT",
                format!(
                    "font 指向的文件不存在：{raw}。请确认路径拼写（.ttf/.otf/.ttc），\
                     或省略 font 让工具自动查找系统等宽字体。"
                ),
            ));
        }
        return Ok(path);
    }
    for path in defaults {
        if path.is_file() {
            return Ok(path.clone());
        }
    }
    Err(CodeVideoError::new("NO_FONT", no_font_message()))
}

// ───────────────────────────── 字体与字形缓存 ─────────────────────────────

/// 一个已栅格化的字形：覆盖度位图 + 相对基线的像素偏移 + 前进宽度。
struct CachedGlyph {
    width: usize,
    height: usize,
    left: i32,
    top: i32,
    coverage: Vec<u8>,
    advance: f32,
}

/// 主字体 + CJK 回退字体 + 字形缓存。
///
/// 逐帧重新 `outline_glyph` 会把同一批字形栅格化上千次，因此这里在渲染前
/// 把出现过的字符全部栅格化一次，帧循环里只做位图混合。
struct FontSet {
    faces: Vec<FontVec>,
    size: f32,
    cache: HashMap<char, CachedGlyph>,
}

impl FontSet {
    fn load(primary: &Path, size: f32) -> Outcome<Self> {
        let first = load_face(primary)?;
        let mut faces = vec![first];
        for path in cjk_fallback_candidates() {
            if path.as_path() == primary || !path.is_file() {
                continue;
            }
            // 回退字体失败不影响主流程：解析失败就跳过，后面缺字显示方框。
            if let Ok(face) = load_face(&path) {
                faces.push(face);
            }
        }
        Ok(Self {
            faces,
            size,
            cache: HashMap::new(),
        })
    }

    /// 为字符挑选字体：主字体有该字形就用主字体，否则尝试 CJK 回退。
    fn face_for(&self, ch: char) -> usize {
        let primary_id = self.faces[0].glyph_id(ch);
        if primary_id.0 != 0 || ch.is_whitespace() {
            return 0;
        }
        for (index, face) in self.faces.iter().enumerate().skip(1) {
            if face.glyph_id(ch).0 != 0 {
                return index;
            }
        }
        0
    }

    fn prepare(&mut self, text: &str) {
        for ch in text.chars() {
            self.cache_char(ch);
        }
    }

    fn cache_char(&mut self, ch: char) {
        if self.cache.contains_key(&ch) {
            return;
        }
        let face_index = self.face_for(ch);
        let (advance, width, height, left, top, coverage) = {
            let face = &self.faces[face_index];
            let scale = PxScale::from(self.size);
            let scaled = face.as_scaled(scale);
            let id: GlyphId = face.glyph_id(ch);
            let advance = scaled.h_advance(id);
            let mut width = 0usize;
            let mut height = 0usize;
            let mut left = 0i32;
            let mut top = 0i32;
            let mut coverage: Vec<u8> = Vec::new();

            // 空白字符没有轮廓，只需前进宽度。
            if !ch.is_whitespace() {
                let glyph = id.with_scale_and_position(scale, point(0.0, 0.0));
                if let Some(outlined) = face.outline_glyph(glyph) {
                    let bounds = outlined.px_bounds();
                    width = (bounds.max.x - bounds.min.x).round().max(0.0) as usize;
                    height = (bounds.max.y - bounds.min.y).round().max(0.0) as usize;
                    left = bounds.min.x.round() as i32;
                    top = bounds.min.y.round() as i32;
                    if width > 0 && height > 0 {
                        coverage = vec![0u8; width * height];
                        outlined.draw(|x, y, cov| {
                            let index = y as usize * width + x as usize;
                            if index < coverage.len() {
                                coverage[index] = (cov.clamp(0.0, 1.0) * 255.0).round() as u8;
                            }
                        });
                    }
                }
            }
            (advance, width, height, left, top, coverage)
        };
        self.cache.insert(
            ch,
            CachedGlyph {
                width,
                height,
                left,
                top,
                coverage,
                advance,
            },
        );
    }

    /// 字符前进宽度；缓存未命中时现场计算（不画字，仅用于布局兜底）。
    fn advance(&self, ch: char) -> f32 {
        if let Some(glyph) = self.cache.get(&ch) {
            return glyph.advance;
        }
        let face_index = self.face_for(ch);
        let face = &self.faces[face_index];
        face.as_scaled(PxScale::from(self.size))
            .h_advance(face.glyph_id(ch))
    }

    fn text_width(&self, text: &str) -> f32 {
        text.chars().map(|c| self.advance(c)).sum()
    }

    fn ascent(&self) -> f32 {
        self.faces[0].as_scaled(PxScale::from(self.size)).ascent()
    }

    fn descent(&self) -> f32 {
        self.faces[0].as_scaled(PxScale::from(self.size)).descent()
    }

    /// 画一段文本，返回结束时的 x 坐标。
    fn draw_text(
        &self,
        canvas: &mut Canvas,
        text: &str,
        x: f32,
        baseline: f32,
        color: [u8; 3],
    ) -> f32 {
        let mut cursor = x;
        for ch in text.chars() {
            if let Some(glyph) = self.cache.get(&ch) {
                if glyph.width > 0 && glyph.height > 0 {
                    let ox = cursor.round() as i32 + glyph.left;
                    let oy = baseline.round() as i32 + glyph.top;
                    for row in 0..glyph.height {
                        for col in 0..glyph.width {
                            let alpha = glyph.coverage[row * glyph.width + col];
                            if alpha > 0 {
                                canvas.blend(
                                    ox + col as i32,
                                    oy + row as i32,
                                    color,
                                    alpha as f32 / 255.0,
                                );
                            }
                        }
                    }
                }
                cursor += glyph.advance;
            } else {
                cursor += self.advance(ch);
            }
        }
        cursor
    }
}

/// 读取并解析字体文件；`.ttc` 集合取第 0 个字面。
fn load_face(path: &Path) -> Outcome<FontVec> {
    let data = std::fs::read(path).map_err(|e| {
        CodeVideoError::new(
            "NO_FONT",
            format!("读取字体失败（{}）：{e}", path.display()),
        )
    })?;
    let is_collection = path
        .extension()
        .map(|ext| ext.eq_ignore_ascii_case("ttc"))
        .unwrap_or(false);
    let parsed = if is_collection {
        FontVec::try_from_vec_and_index(data, 0)
    } else {
        FontVec::try_from_vec(data)
    };
    parsed.map_err(|_| {
        CodeVideoError::new(
            "INVALID_FONT",
            format!(
                "无法解析字体文件：{}。请确认它是有效的 TTF/OTF/TTC 字体。",
                path.display()
            ),
        )
    })
}

// ───────────────────────────── 画布与场景 ─────────────────────────────

/// RGB 帧缓冲。
struct Canvas {
    width: usize,
    height: usize,
    rgb: Vec<u8>,
}

impl Canvas {
    fn new(width: usize, height: usize, background: [u8; 3]) -> Self {
        let mut rgb = Vec::with_capacity(width * height * 3);
        for _ in 0..width * height {
            rgb.extend_from_slice(&background);
        }
        Self { width, height, rgb }
    }

    fn blend(&mut self, x: i32, y: i32, color: [u8; 3], alpha: f32) {
        if x < 0 || y < 0 {
            return;
        }
        let (x, y) = (x as usize, y as usize);
        if x >= self.width || y >= self.height {
            return;
        }
        let a = alpha.clamp(0.0, 1.0);
        let index = (y * self.width + x) * 3;
        for (offset, &src_u8) in color.iter().enumerate() {
            let dst = self.rgb[index + offset] as f32;
            let src = src_u8 as f32;
            self.rgb[index + offset] = (dst * (1.0 - a) + src * a).round() as u8;
        }
    }

    fn fill_rect(&mut self, x: i32, y: i32, width: i32, height: i32, color: [u8; 3]) {
        if width <= 0 || height <= 0 {
            return;
        }
        let x0 = x.max(0) as usize;
        let y0 = y.max(0) as usize;
        let x1 = ((x + width).max(0) as usize).min(self.width);
        let y1 = ((y + height).max(0) as usize).min(self.height);
        for yy in y0..y1 {
            for xx in x0..x1 {
                let index = (yy * self.width + xx) * 3;
                self.rgb[index..index + 3].copy_from_slice(&color);
            }
        }
    }

    fn fill_circle(&mut self, cx: f32, cy: f32, radius: f32, color: [u8; 3]) {
        if radius <= 0.0 {
            return;
        }
        let x0 = (cx - radius - 1.0).floor() as i32;
        let x1 = (cx + radius + 1.0).ceil() as i32;
        let y0 = (cy - radius - 1.0).floor() as i32;
        let y1 = (cy + radius + 1.0).ceil() as i32;
        for y in y0..=y1 {
            for x in x0..=x1 {
                let dx = x as f32 + 0.5 - cx;
                let dy = y as f32 + 0.5 - cy;
                let distance = (dx * dx + dy * dy).sqrt();
                let alpha = (radius + 0.5 - distance).clamp(0.0, 1.0);
                if alpha > 0.0 {
                    self.blend(x, y, color, alpha);
                }
            }
        }
    }
}

/// 主题配色。
#[derive(Clone, Copy)]
struct Palette {
    background: [u8; 3],
    bar: [u8; 3],
    bar_border: [u8; 3],
    bar_text: [u8; 3],
    gutter: [u8; 3],
    gutter_active: [u8; 3],
    text: [u8; 3],
    keyword: [u8; 3],
    string: [u8; 3],
    comment: [u8; 3],
    number: [u8; 3],
    function: [u8; 3],
    cursor: [u8; 3],
}

impl Palette {
    fn new(dark: bool) -> Self {
        if dark {
            Self {
                background: [15, 18, 24],
                bar: [30, 34, 43],
                bar_border: [44, 49, 60],
                bar_text: [158, 164, 176],
                gutter: [88, 96, 112],
                gutter_active: [178, 186, 200],
                text: [208, 214, 224],
                keyword: [199, 120, 221],
                string: [152, 195, 121],
                comment: [105, 115, 130],
                number: [209, 154, 102],
                function: [97, 175, 239],
                cursor: [238, 240, 244],
            }
        } else {
            Self {
                background: [250, 250, 252],
                bar: [232, 234, 239],
                bar_border: [212, 215, 222],
                bar_text: [86, 92, 104],
                gutter: [158, 164, 175],
                gutter_active: [70, 76, 88],
                text: [42, 46, 54],
                keyword: [166, 38, 164],
                string: [64, 120, 64],
                comment: [130, 138, 150],
                number: [152, 104, 1],
                function: [0, 92, 197],
                cursor: [38, 42, 50],
            }
        }
    }

    fn token_color(&self, class: u8) -> [u8; 3] {
        match class {
            COLOR_KEYWORD => self.keyword,
            COLOR_STRING => self.string,
            COLOR_COMMENT => self.comment,
            COLOR_NUMBER => self.number,
            COLOR_FUNCTION => self.function,
            _ => self.text,
        }
    }
}

/// 静态布局：只在渲染前算一次，帧循环里不再重复测量。
struct Layout {
    width: usize,
    height: usize,
    font_size: f32,
    bar_height: f32,
    body_top: f32,
    body_x: f32,
    line_height: f32,
    visible_lines: usize,
    ascent: f32,
    descent: f32,
    gutter_right: f32,
    cursor_width: f32,
}

impl Layout {
    fn new(spec: &VideoSpec, fonts: &FontSet, total_lines: usize) -> Self {
        let font_size = spec.font_size;
        let width = spec.width as usize;
        let height = spec.height as usize;
        let bar_height = (font_size * 2.1).max(34.0);
        let margin = (font_size * 1.3).max(16.0);
        let digits = total_lines.max(1).to_string().len().max(2);
        let gutter_width = fonts.advance('0') * digits as f32 + font_size * 0.9;
        let body_x = margin + gutter_width + font_size * 0.9;
        let body_top = bar_height + font_size * 0.8;
        let line_height = (font_size * 1.55).max(font_size + 4.0);
        let content_height = (height as f32 - body_top - font_size * 0.8).max(line_height);
        let visible_lines = ((content_height / line_height).floor() as usize).max(1);
        let cursor_width = fonts.advance('M').max(font_size * 0.45).max(2.0);
        Self {
            width,
            height,
            font_size,
            bar_height,
            body_top,
            body_x,
            line_height,
            visible_lines,
            ascent: fonts.ascent(),
            descent: fonts.descent(),
            gutter_right: margin + gutter_width,
            cursor_width,
        }
    }

    fn baseline(&self, row: usize) -> f32 {
        self.body_top + self.ascent + row as f32 * self.line_height
    }
}

/// 一帧的绘制上下文（引用预计算数据，避免每帧重新分词/测量）。
struct Scene<'a> {
    layout: Layout,
    palette: Palette,
    fonts: &'a FontSet,
    title: &'a str,
    lines: &'a [String],
    line_lens: &'a [usize],
    tokens: &'a [Vec<(String, u8)>],
}

impl Scene<'_> {
    fn render(&self, typed_steps: usize, elapsed_seconds: f64) -> Vec<u8> {
        let (cursor_line, cursor_col) = cursor_position(typed_steps, self.line_lens);
        let scroll = scroll_offset(cursor_line, self.layout.visible_lines, self.lines.len());

        let mut canvas = Canvas::new(
            self.layout.width,
            self.layout.height,
            self.palette.background,
        );
        self.draw_bar(&mut canvas);

        // 只画「已经开始」的行：光标行之后的代码完全不存在于画面中。
        for row in 0..self.layout.visible_lines {
            let line_index = scroll + row;
            if line_index >= self.lines.len() || line_index > cursor_line {
                break;
            }
            let baseline = self.layout.baseline(row);
            let number = (line_index + 1).to_string();
            let number_width = self.fonts.text_width(&number);
            let number_color = if line_index == cursor_line {
                self.palette.gutter_active
            } else {
                self.palette.gutter
            };
            self.fonts.draw_text(
                &mut canvas,
                &number,
                self.layout.gutter_right - number_width,
                baseline,
                number_color,
            );
            let limit = if line_index == cursor_line {
                Some(cursor_col)
            } else {
                None
            };
            self.draw_line(&mut canvas, line_index, limit, baseline);
        }

        // 实心方块光标：停在当前打字位置，每 0.5s 闪一次。
        if blink_on(elapsed_seconds) {
            let prefix: String = self.lines[cursor_line].chars().take(cursor_col).collect();
            let cursor_x = self.layout.body_x + self.fonts.text_width(&prefix);
            let baseline = self.layout.baseline(cursor_line - scroll);
            let top = (baseline - self.layout.ascent).round() as i32 + 1;
            let height = ((self.layout.ascent - self.layout.descent).round() as i32 - 2).max(2);
            canvas.fill_rect(
                cursor_x.round() as i32,
                top,
                self.layout.cursor_width.round() as i32,
                height,
                self.palette.cursor,
            );
        }

        canvas.rgb
    }

    fn draw_bar(&self, canvas: &mut Canvas) {
        let bar_height = self.layout.bar_height.round() as i32;
        canvas.fill_rect(0, 0, self.layout.width as i32, bar_height, self.palette.bar);
        if bar_height >= 2 {
            canvas.fill_rect(
                0,
                bar_height - 1,
                self.layout.width as i32,
                1,
                self.palette.bar_border,
            );
        }

        // 左侧三个圆点（macOS 窗口灯风格）。
        let radius = (self.layout.font_size * 0.28).clamp(4.0, 6.0);
        let gap = radius * 2.4;
        let start_x = self.layout.font_size * 1.1 + radius;
        let center_y = bar_height as f32 * 0.5;
        canvas.fill_circle(start_x, center_y, radius, [255, 95, 86]);
        canvas.fill_circle(start_x + gap, center_y, radius, [255, 189, 46]);
        canvas.fill_circle(start_x + gap * 2.0, center_y, radius, [39, 201, 63]);

        // 居中标题。
        if !self.title.is_empty() {
            let title_width = self.fonts.text_width(self.title);
            let x =
                ((self.layout.width as f32 - title_width) * 0.5).max(self.layout.font_size * 3.0);
            let baseline = bar_height as f32 * 0.5 + self.layout.font_size * 0.36;
            self.fonts
                .draw_text(canvas, self.title, x, baseline, self.palette.bar_text);
        }
    }

    /// 画一行正文；`limit` 为 `Some(n)` 时只画前 n 个字符（未打出的不画）。
    fn draw_line(
        &self,
        canvas: &mut Canvas,
        line_index: usize,
        limit: Option<usize>,
        baseline: f32,
    ) {
        let mut budget = limit.unwrap_or(usize::MAX);
        let mut x = self.layout.body_x;
        for (text, class) in &self.tokens[line_index] {
            if budget == 0 {
                break;
            }
            let color = self.palette.token_color(*class);
            if budget == usize::MAX {
                x = self.fonts.draw_text(canvas, text, x, baseline, color);
            } else {
                let count = text.chars().count();
                if count <= budget {
                    x = self.fonts.draw_text(canvas, text, x, baseline, color);
                    budget -= count;
                } else {
                    let head: String = text.chars().take(budget).collect();
                    self.fonts.draw_text(canvas, &head, x, baseline, color);
                    budget = 0;
                }
            }
        }
    }
}

/// 临时目录守卫：无论提前返回还是成功结束，都删除帧序列。
struct TempDirGuard(PathBuf);

impl Drop for TempDirGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// 写入一帧 PPM(P6)。
fn write_ppm(path: &Path, width: usize, height: usize, rgb: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let file = std::fs::File::create(path)?;
    let mut writer = std::io::BufWriter::new(file);
    writer.write_all(ppm_header(width, height).as_bytes())?;
    writer.write_all(rgb)?;
    writer.flush()
}

/// 参数解析与区间校验。
fn parse_spec(args: &HashMap<String, String>) -> Outcome<VideoSpec> {
    let code = args.get("code").cloned().unwrap_or_default();
    validate_code(&code)?;

    let language = normalize_lang(arg_value(args, "language").unwrap_or("plain")).to_string();
    let out = arg_value(args, "out")
        .map(str::to_string)
        .unwrap_or_else(|| {
            format!(
                "generated/videos/code-{}.mp4",
                chrono::Local::now().format("%Y%m%d-%H%M%S")
            )
        });
    let title = arg_value(args, "title")
        .unwrap_or("main.rs")
        .replace(['\n', '\r'], " ");

    let width = parse_u32_arg(args, "width", 1280)?;
    if !(MIN_WIDTH..=MAX_WIDTH).contains(&width) {
        return Err(CodeVideoError::new(
            "INVALID_PARAMS",
            format!("width 必须在 {MIN_WIDTH}–{MAX_WIDTH} 之间，实际：{width}"),
        ));
    }
    let height = parse_u32_arg(args, "height", 720)?;
    if !(MIN_HEIGHT..=MAX_HEIGHT).contains(&height) {
        return Err(CodeVideoError::new(
            "INVALID_PARAMS",
            format!("height 必须在 {MIN_HEIGHT}–{MAX_HEIGHT} 之间，实际：{height}"),
        ));
    }
    let fps = parse_u32_arg(args, "fps", 30)?;
    if !(MIN_FPS..=MAX_FPS).contains(&fps) {
        return Err(CodeVideoError::new(
            "INVALID_PARAMS",
            format!("fps 必须在 {MIN_FPS}–{MAX_FPS} 之间，实际：{fps}"),
        ));
    }
    let chars_per_second = parse_u32_arg(args, "chars_per_second", 28)?;
    if !(MIN_CPS..=MAX_CPS).contains(&chars_per_second) {
        return Err(CodeVideoError::new(
            "INVALID_PARAMS",
            format!("chars_per_second 必须在 {MIN_CPS}–{MAX_CPS} 之间，实际：{chars_per_second}"),
        ));
    }
    let font_size = parse_f32_arg(args, "font_size", 18.0)?;
    if !(MIN_FONT_SIZE..=MAX_FONT_SIZE).contains(&font_size) {
        return Err(CodeVideoError::new(
            "INVALID_PARAMS",
            format!("font_size 必须在 {MIN_FONT_SIZE}–{MAX_FONT_SIZE} 之间，实际：{font_size}"),
        ));
    }
    let tail_seconds = parse_f32_arg(args, "tail_seconds", 2.5)? as f64;
    if tail_seconds < 0.0 {
        return Err(CodeVideoError::new(
            "INVALID_PARAMS",
            format!("tail_seconds 不能为负数，实际：{tail_seconds}"),
        ));
    }

    let theme = arg_value(args, "theme")
        .unwrap_or("dark")
        .to_ascii_lowercase();
    let theme_dark = match theme.as_str() {
        "dark" => true,
        "light" => false,
        other => {
            return Err(CodeVideoError::new(
                "INVALID_PARAMS",
                format!("theme 只能是 dark 或 light，实际：{other}"),
            ))
        }
    };

    Ok(VideoSpec {
        code,
        language,
        out,
        title,
        width,
        height,
        fps,
        chars_per_second,
        theme_dark,
        font: arg_value(args, "font").map(str::to_string),
        font_size,
        highlight: parse_bool_arg(args, "highlight", true)?,
        tail_seconds,
    })
}

fn arg_value<'a>(args: &'a HashMap<String, String>, key: &str) -> Option<&'a str> {
    args.get(key).map(|s| s.trim()).filter(|s| !s.is_empty())
}

fn parse_u32_arg(args: &HashMap<String, String>, key: &str, default: u32) -> Outcome<u32> {
    match arg_value(args, key) {
        None => Ok(default),
        Some(raw) => raw.parse::<u32>().map_err(|_| {
            CodeVideoError::new(
                "INVALID_PARAMS",
                format!("参数 {key} 必须是正整数，实际：{raw}"),
            )
        }),
    }
}

fn parse_f32_arg(args: &HashMap<String, String>, key: &str, default: f32) -> Outcome<f32> {
    match arg_value(args, key) {
        None => Ok(default),
        Some(raw) => raw.parse::<f32>().map_err(|_| {
            CodeVideoError::new(
                "INVALID_PARAMS",
                format!("参数 {key} 必须是数字，实际：{raw}"),
            )
        }),
    }
}

fn parse_bool_arg(args: &HashMap<String, String>, key: &str, default: bool) -> Outcome<bool> {
    match arg_value(args, key) {
        None => Ok(default),
        Some(raw) => match raw.to_ascii_lowercase().as_str() {
            "true" | "1" | "yes" | "on" => Ok(true),
            "false" | "0" | "no" | "off" => Ok(false),
            other => Err(CodeVideoError::new(
                "INVALID_PARAMS",
                format!("参数 {key} 必须是布尔值（true/false），实际：{other}"),
            )),
        },
    }
}

/// 截取文本末尾的 n 个字符（ffmpeg stderr 可能非常长）。
fn tail(text: &str, n: usize) -> String {
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= n {
        text.to_string()
    } else {
        chars[chars.len() - n..].iter().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn concat_tokens(tokens: &[(String, u8)]) -> String {
        tokens.iter().map(|(text, _)| text.as_str()).collect()
    }

    #[test]
    fn classify_rust_keywords_strings_numbers_comments() {
        let line = r#"let x = 42; let s = "hi"; // 注释"#;
        let tokens = classify("rust", line);
        assert_eq!(
            concat_tokens(&tokens),
            line,
            "分词必须无损拼接回原行（否则会漏字符或错位）"
        );
        assert!(
            tokens
                .iter()
                .any(|(t, c)| t == "let" && *c == COLOR_KEYWORD),
            "关键字应着色: {tokens:?}"
        );
        assert!(
            tokens.iter().any(|(t, c)| t == "42" && *c == COLOR_NUMBER),
            "数字应着色: {tokens:?}"
        );
        assert!(
            tokens
                .iter()
                .any(|(t, c)| t == "\"hi\"" && *c == COLOR_STRING),
            "字符串应着色: {tokens:?}"
        );
        assert!(
            tokens
                .iter()
                .any(|(t, c)| t == "// 注释" && *c == COLOR_COMMENT),
            "注释应着色: {tokens:?}"
        );
    }

    #[test]
    fn classify_detects_function_names() {
        let rust = classify("rust", "fn main() {");
        assert!(
            rust.iter()
                .any(|(t, c)| t == "main" && *c == COLOR_FUNCTION),
            "fn 之后的名字应识别为函数: {rust:?}"
        );
        let python = classify("python", "def hello(name):");
        assert!(
            python
                .iter()
                .any(|(t, c)| t == "hello" && *c == COLOR_FUNCTION),
            "def 之后的名字应识别为函数: {python:?}"
        );
        let js = classify("js", "console.log(1)");
        assert!(
            js.iter().any(|(t, c)| t == "log" && *c == COLOR_FUNCTION),
            "紧跟 ( 的名字应识别为函数: {js:?}"
        );
    }

    #[test]
    fn classify_unknown_language_is_plain() {
        let line = "SELECT * FROM t -- 不算注释";
        let tokens = classify("sql", line);
        assert_eq!(tokens.len(), 1, "未知语言必须整行一个 token");
        assert_eq!(tokens[0].1, COLOR_PLAIN, "未知语言必须全部正文色");
        assert_eq!(tokens[0].0, line);

        // Python 的注释是 #，不能套用 Rust/JS 的 //
        let py = classify("python", "x = 1  # 注释");
        assert!(
            py.iter().any(|(t, c)| t == "# 注释" && *c == COLOR_COMMENT),
            "Python 注释应被识别: {py:?}"
        );
        assert!(
            concat_tokens(&py) == "x = 1  # 注释",
            "分词必须无损拼接: {py:?}"
        );
    }

    #[test]
    fn frame_count_covers_typing_and_tail() {
        // 280 字符 @28cps = 10s，加 2.5s 静置 = 12.5s × 30fps
        assert_eq!(frame_count(280, 28, 2.5, 30), 375);
        // 只有静置也要有画面
        assert_eq!(frame_count(0, 28, 2.5, 30), 75);
        // 至少一帧
        assert_eq!(frame_count(0, 28, 0.0, 30), 1);
        // 10 字符 @4cps = 2.5s × 60fps
        assert_eq!(frame_count(10, 4, 0.0, 60), 150);
    }

    #[test]
    fn scroll_offset_keeps_cursor_visible() {
        assert_eq!(scroll_offset(0, 10, 100), 0);
        assert_eq!(scroll_offset(9, 10, 100), 0, "光标仍在首屏时不滚动");
        assert_eq!(scroll_offset(10, 10, 100), 1, "光标越过末行时只滚一行");
        assert_eq!(scroll_offset(99, 10, 100), 90, "末行时窗口贴底");
        assert_eq!(scroll_offset(5, 10, 4), 0, "总行数少于可视行时不滚动");
        assert_eq!(scroll_offset(3, 0, 100), 0, "异常可视行数不 panic");
    }

    #[test]
    fn ppm_header_is_p6_rgb() {
        assert_eq!(ppm_header(1280, 720), "P6\n1280 720\n255\n");
        assert_eq!(ppm_header(2, 3), "P6\n2 3\n255\n");
        assert_eq!(frame_name(0), "frame_00001.ppm");
        assert_eq!(frame_name(41), "frame_00042.ppm");
    }

    #[test]
    fn oversized_code_is_rejected() {
        let too_many_lines = "x\n".repeat(MAX_CODE_LINES + 1);
        assert_eq!(
            validate_code(&too_many_lines).unwrap_err().code,
            "CODE_TOO_LONG"
        );
        let too_many_chars = "a".repeat(MAX_CODE_CHARS + 1);
        assert_eq!(
            validate_code(&too_many_chars).unwrap_err().code,
            "CODE_TOO_LONG"
        );
        assert_eq!(
            validate_code("   \n\t ").unwrap_err().code,
            "INVALID_PARAMS"
        );
        assert!(validate_code("fn main() {}").is_ok());
    }

    #[test]
    fn missing_font_path_returns_no_font_code() {
        let missing = std::env::temp_dir().join(format!(
            "mq-codevideo-no-such-font-{}-{}.ttf",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let _ = std::fs::remove_file(&missing);
        let raw = missing.to_string_lossy().to_string();

        let err = pick_font_path(Some(raw.as_str()), &[]).unwrap_err();
        assert_eq!(err.code, "NO_FONT", "不存在的字体必须报 NO_FONT");
        assert!(
            err.message.contains("不存在") && err.message.contains("font"),
            "必须告诉用户怎么传 font: {}",
            err.message
        );

        let err = pick_font_path(Some("relative.ttf"), &[]).unwrap_err();
        assert_eq!(err.code, "NO_FONT", "相对路径必须报 NO_FONT");
        assert!(err.message.contains("绝对路径"));

        let err = pick_font_path(None, &[]).unwrap_err();
        assert_eq!(err.code, "NO_FONT", "无任何可用字体必须报 NO_FONT");
        assert!(
            err.message.contains("C:\\Windows\\Fonts"),
            "必须列出候选路径"
        );
    }

    #[test]
    fn typed_prefix_and_cursor_follow_line_breaks() {
        let lens = [3usize, 2];
        assert_eq!(cursor_position(0, &lens), (0, 0));
        assert_eq!(cursor_position(3, &lens), (0, 3));
        assert_eq!(
            cursor_position(4, &lens),
            (1, 0),
            "第 4 步是换行，光标应进入下一行行首"
        );
        assert_eq!(cursor_position(5, &lens), (1, 1));
        assert_eq!(cursor_position(6, &lens), (1, 2), "全部打完停在末行行尾");

        assert_eq!(typed_steps_at(0.0, 28, 6), 0, "第 0 帧不显示任何字符");
        assert_eq!(
            typed_steps_at(1.0 / 30.0, 28, 6),
            0,
            "不足一个字符的间隔不应提前显示"
        );
        assert_eq!(typed_steps_at(2.0 / 30.0, 28, 6), 1);
        assert_eq!(typed_steps_at(10.0, 28, 6), 6, "不得超过总步数");
    }

    #[test]
    fn ffmpeg_args_are_whitelisted() {
        let args = ffmpeg_args(30, Path::new("C:/out/video.mp4"));
        assert_eq!(args[0], "-y");
        assert_eq!(args[1], "-framerate");
        assert_eq!(args[2], "30");
        assert_eq!(args[3], "-i");
        assert_eq!(args[4], "frame_%05d.ppm");
        assert!(args.windows(2).any(|w| w[0] == "-c:v" && w[1] == "libx264"));
        assert!(args
            .windows(2)
            .any(|w| w[0] == "-pix_fmt" && w[1] == "yuv420p"));
        assert_eq!(args.last().map(String::as_str), Some("C:/out/video.mp4"));
    }

    /// 端到端：真的渲染出 mp4（默认忽略，CI/未装 ffmpeg 的机器不跑）。
    ///
    /// 用 `cargo test --lib -- --ignored tools::code_video::real_ffmpeg_end_to_end` 显式执行。
    #[tokio::test]
    #[ignore = "需要本机 ffmpeg；用 --ignored 显式运行"]
    async fn real_ffmpeg_end_to_end() {
        use crate::mcp::registry::ToolExecutor;

        let dir = std::env::temp_dir().join(format!(
            "mq-codevideo-e2e-{}-{}",
            std::process::id(),
            chrono::Local::now().timestamp_nanos_opt().unwrap_or(0)
        ));
        std::fs::create_dir_all(&dir).expect("建工作区");
        let tool = CodeVideoTool::new(dir.clone());
        let mut args = HashMap::new();
        args.insert(
            "code".to_string(),
            "fn main() {\n    println!(\"hello marguerite\");\n}\n".to_string(),
        );
        args.insert("language".to_string(), "rust".to_string());
        args.insert("out".to_string(), "e2e.mp4".to_string());
        args.insert("fps".to_string(), "12".to_string());
        args.insert("chars_per_second".to_string(), "120".to_string());
        args.insert("tail_seconds".to_string(), "0.5".to_string());

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
                panic!("端到端渲染失败（{code}）：{message}")
            }
            other => panic!("端到端渲染返回了非预期结果：{other:?}"),
        }
    }
}
