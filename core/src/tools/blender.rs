//! Blender 工具（`blender`）：无头 Blender 真实桥
//!
//! # 为什么需要
//!
//! 产品要「在 3D 领域领先」，此前 Rust 工具层没有任何 Blender 能力 ——
//! 模型只能靠 `shell_exec` 手拼命令行（路径定位、参数拼装、产物校验都不可控）。
//! 本模块把这条路径收成**结构化动作**：模型表达意图（跑脚本 / 渲染 / 探测），
//! 命令行由这里拼装。
//!
//! # 安全模型（与 `tools/media.rs` 同口径）
//!
//! - **不经 shell**：`Command::new(blender).args([...])`，免疫注入；
//! - **路径双向收口**：脚本/场景/产物都经 [`safe_resolve`] 解析到工作区内；
//! - **产物真实性**：exit 0 但没产出文件（脚本静默失败）判失败，不谎报成功；
//! - **超时**：Blender 启动 + 脚本/渲染常见数十秒到数分钟，外层超时由
//!   `ToolCall::default_timeout_for("blender")` 给 900s，内部再按动作收紧。
//!
//! # 二进制发现（确定性顺序）
//!
//! `MARGUERITE_BLENDER` 环境变量 → PATH → `Program Files/Blender Foundation/**`
//! → 仓库 `third_party/blender/**/blender.exe`。找不到时返回**结构化错误**
//! `BLENDER_BINARY_MISSING`（「没装」与「脚本失败」必须可区分）。

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

/// 一次 Blender 进程输出
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlenderOutput {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
}

/// Blender 进程执行器（可注入，单测断言 argv 不真跑进程）
pub trait BlenderRunner: Send + Sync {
    /// `cwd`：子进程工作目录。**必须是工作区** —— bpy 脚本里 `bpy.path.abspath("//…")`
    /// 的 `//` 指向 .blend 所在目录，而保存前的相对解析落在进程 CWD；若 CWD 不是
    /// 工作区，产物会写到别处（实测缺陷：e2e 里 GLB 写进了 cargo 运行目录）。
    fn run(
        &self,
        program: &str,
        args: &[String],
        timeout: Duration,
        cwd: Option<&Path>,
    ) -> Result<BlenderOutput>;
}

/// 生产实现：直接调 blender.exe（`SilentCommand` 抑制控制台黑窗）
pub struct SystemBlenderRunner;

impl BlenderRunner for SystemBlenderRunner {
    fn run(
        &self,
        program: &str,
        args: &[String],
        timeout: Duration,
        cwd: Option<&Path>,
    ) -> Result<BlenderOutput> {
        let mut cmd = Command::new(program);
        cmd.silent();
        if let Some(dir) = cwd {
            cmd.current_dir(dir);
        }
        cmd.args(args);
        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());
        let mut child = cmd
            .spawn()
            .map_err(|e| anyhow!("启动 blender 失败：{e}"))?;
        let started = std::time::Instant::now();
        loop {
            match child.try_wait() {
                Ok(Some(_)) => break,
                Ok(None) => {
                    if started.elapsed() > timeout {
                        let _ = child.kill();
                        let _ = child.wait();
                        return Err(anyhow!(
                            "TIMEOUT:blender 超过 {}s 未完成（渲染/烘焙请用 run_script 分步执行）",
                            timeout.as_secs()
                        ));
                    }
                    std::thread::sleep(Duration::from_millis(100));
                }
                Err(e) => return Err(anyhow!("等待 blender 失败：{e}")),
            }
        }
        let out = child
            .wait_with_output()
            .map_err(|e| anyhow!("读取 blender 输出失败：{e}"))?;
        Ok(BlenderOutput {
            exit_code: out.status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        })
    }
}

/// 支持的动作（白名单）
const ACTIONS: &[&str] = &["probe", "run_script", "render"];

pub struct BlenderTool {
    workspace: RootSet,
    runner: Arc<dyn BlenderRunner>,
    /// 显式二进制路径（发现链的结果缓存到实例）
    binary: PathBuf,
}

impl BlenderTool {
    pub fn new(
        workspace: impl Into<RootSet>,
        runner: Arc<dyn BlenderRunner>,
        binary: PathBuf,
    ) -> Self {
        let workspace: RootSet = workspace.into();
        Self {
            workspace,
            runner,
            binary,
        }
    }

    pub fn with_discovered_binary(workspace: impl Into<RootSet>) -> Result<Self> {
        let binary = discover_blender_binary().ok_or_else(|| {
            anyhow!(
                "未找到 blender.exe：请安装 Blender（winget install BlenderFoundation.Blender）\
                 或设置 MARGUERITE_BLENDER 指向 blender.exe"
            )
        })?;
        Ok(Self::new(
            workspace,
            Arc::new(SystemBlenderRunner),
            binary,
        ))
    }

    fn resolve_arg(&self, args: &HashMap<String, String>, key: &str) -> Result<PathBuf, String> {
        let raw = args
            .get(key)
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .ok_or_else(|| format!("缺少参数 `{key}`"))?;
        self.workspace.resolve(raw).map_err(|e| format!("{e}"))
    }

    /// `run_script` 的 argv（纯函数，单测断言无注入面）。
    fn run_script_args(
        binary: &Path,
        script: &Path,
        blend: Option<&Path>,
        passthrough: &[String],
    ) -> Vec<String> {
        let mut a: Vec<String> = vec!["--background".into(), "--factory-startup".into()];
        if let Some(b) = blend {
            a.push(b.to_string_lossy().into_owned());
        }
        a.push("--python".into());
        a.push(script.to_string_lossy().into_owned());
        if !passthrough.is_empty() {
            a.push("--".into());
            a.extend(passthrough.iter().cloned());
        }
        a.insert(0, binary.to_string_lossy().into_owned());
        a
    }

    /// `render` 的 argv（Workbench 引擎对无 GPU/无显示环境最稳；EEVEE/Cycles 由
    /// 脚本内切换）。
    fn render_args(binary: &Path, blend: &Path, frame: u32, out_prefix: &Path) -> Vec<String> {
        vec![
            binary.to_string_lossy().into_owned(),
            "--background".into(),
            blend.to_string_lossy().into_owned(),
            "--render-output".into(),
            out_prefix.to_string_lossy().into_owned(),
            "--render-frame".into(),
            format!("{frame}"),
        ]
    }
}

/// 定位 blender.exe（确定性顺序；纯环境探测，无副作用）。
pub fn discover_blender_binary() -> Option<PathBuf> {
    // 1) 显式环境变量
    if let Ok(v) = std::env::var("MARGUERITE_BLENDER") {
        let p = PathBuf::from(v.trim());
        if p.is_file() {
            return Some(p);
        }
    }
    // 2) PATH（交由 create_process 解析，但要确认可执行）
    if let Ok(path_var) = std::env::var("PATH") {
        for dir in std::env::split_paths(&path_var) {
            let candidate = dir.join("blender.exe");
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    // 3) 标准安装位置
    let pf = std::env::var("ProgramFiles").unwrap_or_else(|_| r"C:\Program Files".into());
    let foundation = Path::new(&pf).join("Blender Foundation");
    if let Ok(rd) = std::fs::read_dir(&foundation) {
        let mut versions: Vec<PathBuf> = rd
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.is_dir())
            .collect();
        // 版本目录降序：取最新
        versions.sort_by(|a, b| b.file_name().cmp(&a.file_name()));
        for v in versions {
            let candidate = v.join("blender.exe");
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    // 4) 仓库 third_party（便携版）
    if let Ok(cwd) = std::env::current_dir() {
        let mut probe = Some(cwd.as_path());
        while let Some(dir) = probe {
            let tp = dir.join("third_party/blender");
            if tp.is_dir() {
                if let Ok(rd) = std::fs::read_dir(&tp) {
                    let mut dirs: Vec<PathBuf> = rd
                        .filter_map(|e| e.ok())
                        .map(|e| e.path())
                        .filter(|p| p.is_dir())
                        .collect();
                    dirs.sort_by(|a, b| b.file_name().cmp(&a.file_name()));
                    for v in dirs {
                        let candidate = v.join("blender.exe");
                        if candidate.is_file() {
                            return Some(candidate);
                        }
                    }
                }
            }
            probe = dir.parent();
        }
    }
    None
}

#[async_trait]
impl ToolExecutor for BlenderTool {
    fn name(&self) -> &str {
        "blender"
    }

    fn description(&self) -> &str {
        "Blender 无头执行（真实跑 blender.exe）：run_script 跑 bpy 脚本（建模/材质/导出）、\
         render 渲染场景帧、probe 探测版本。脚本/场景/产物限工作区内；产物为空判失败。"
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "action": {"type": "string", "enum": ACTIONS},
                "script": {"type": "string", "description": "bpy 脚本（工作区内 .py）"},
                "blend": {"type": "string", "description": "输入 .blend（可选；缺省空场景）"},
                "frame": {"type": "integer", "description": "render：帧号（默认 1）"},
                "output": {"type": "string", "description": "render 输出前缀（如 renders/shot_，会自动补帧号后缀）"},
                "args": {"type": "string", "description": "传给脚本的 -- 参数（空格分隔）"},
                "expect_file": {"type": "string", "description": "run_script 成功后必须存在的产物（可空）"}
            },
            "required": ["action"]
        })
    }

    async fn execute(&self, args: &HashMap<String, String>) -> Result<ToolResult> {
        let action = args
            .get("action")
            .map(|s| s.trim().to_string())
            .unwrap_or_default();
        if !ACTIONS.contains(&action.as_str()) {
            return Ok(ToolResult::Error {
                message: format!("不支持的 action「{action}」；可用：{}", ACTIONS.join(" / ")),
                code: "BLENDER_UNSUPPORTED_ACTION".into(),
            });
        }

        if action == "probe" {
            let argv = [
                self.binary.to_string_lossy().into_owned(),
                "--version".into(),
            ];
            return match self.runner.run(&argv[0], &argv[1..], Duration::from_secs(60), Some(self.workspace.primary())) {
                Ok(out) if out.exit_code == 0 => {
                    let version = out
                        .stdout
                        .lines()
                        .find(|l| l.contains("Blender"))
                        .unwrap_or("Blender")
                        .trim()
                        .to_string();
                    Ok(ToolResult::Success {
                        data: json!({
                            "ok": true,
                            "version": version,
                            "binary": self.binary.to_string_lossy(),
                        })
                        .to_string(),
                        metadata: HashMap::new(),
                    })
                }
                Ok(out) => Ok(ToolResult::Error {
                    message: format!(
                        "blender --version 失败（exit {}）：{}",
                        out.exit_code,
                        tail(&out.stderr, 400)
                    ),
                    code: "BLENDER_PROBE_FAILED".into(),
                }),
                Err(e) => Ok(map_runner_err(e)),
            };
        }

        if action == "run_script" {
            let script = match self.resolve_arg(args, "script") {
                Ok(p) => p,
                Err(e) => {
                    return Ok(ToolResult::Error {
                        message: e,
                        code: "BLENDER_BAD_PATH".into(),
                    })
                }
            };
            if !script.is_file() {
                return Ok(ToolResult::Error {
                    message: format!("脚本不存在：{}", script.display()),
                    code: "BLENDER_INPUT_MISSING".into(),
                });
            }
            let blend = match args.get("blend").map(|s| s.trim()).filter(|s| !s.is_empty()) {
                Some(_) => match self.resolve_arg(args, "blend") {
                    Ok(p) if p.is_file() => Some(p),
                    Ok(p) => {
                        return Ok(ToolResult::Error {
                            message: format!("场景文件不存在：{}", p.display()),
                            code: "BLENDER_INPUT_MISSING".into(),
                        })
                    }
                    Err(e) => {
                        return Ok(ToolResult::Error {
                            message: e,
                            code: "BLENDER_BAD_PATH".into(),
                        })
                    }
                },
                None => None,
            };
            let passthrough: Vec<String> = args
                .get("args")
                .map(|s| s.split_whitespace().map(str::to_string).collect())
                .unwrap_or_default();
            let argv = Self::run_script_args(&self.binary, &script, blend.as_deref(), &passthrough);

            let expect = match args.get("expect_file").map(|s| s.trim()).filter(|s| !s.is_empty())
            {
                Some(_) => match self.resolve_arg(args, "expect_file") {
                    Ok(p) => Some(p),
                    Err(e) => {
                        return Ok(ToolResult::Error {
                            message: e,
                            code: "BLENDER_BAD_PATH".into(),
                        })
                    }
                },
                None => None,
            };
            return match self.runner.run(
                &argv[0],
                &argv[1..],
                Duration::from_secs(840),
                Some(self.workspace.primary()),
            ) {
                Ok(out) if out.exit_code == 0 => {
                    // exit 0 不代表成功：bpy 脚本静默失败很常见，必须核对产物
                    if let Some(expect) = &expect {
                        let size = std::fs::metadata(expect).map(|m| m.len()).unwrap_or(0);
                        if size == 0 {
                            return Ok(ToolResult::Error {
                                message: format!(
                                    "blender 退出码 0，但预期产物为空或不存在：{}。\
                                     检查脚本是否真的写出了该文件。",
                                    expect.display()
                                ),
                                code: "BLENDER_EMPTY_OUTPUT".into(),
                            });
                        }
                        Ok(ToolResult::Success {
                            data: json!({
                                "ok": true,
                                "expect_file": expect.to_string_lossy(),
                                "bytes": size,
                                "stderr_tail": tail(&out.stderr, 600),
                            })
                            .to_string(),
                            metadata: HashMap::new(),
                        })
                    } else {
                        Ok(ToolResult::Success {
                            data: json!({
                                "ok": true,
                                "stdout_tail": tail(&out.stdout, 1200),
                                "stderr_tail": tail(&out.stderr, 600),
                            })
                            .to_string(),
                            metadata: HashMap::new(),
                        })
                    }
                }
                Ok(out) => Ok(ToolResult::Error {
                    message: format!(
                        "blender 脚本失败（exit {}）：{}",
                        out.exit_code,
                        tail(&out.stderr, 900)
                    ),
                    code: "BLENDER_SCRIPT_FAILED".into(),
                }),
                Err(e) => Ok(map_runner_err(e)),
            };
        }

        // render
        let blend = match self.resolve_arg(args, "blend") {
            Ok(p) if p.is_file() => p,
            Ok(p) => {
                return Ok(ToolResult::Error {
                    message: format!("场景文件不存在：{}", p.display()),
                    code: "BLENDER_INPUT_MISSING".into(),
                })
            }
            Err(e) => {
                return Ok(ToolResult::Error {
                    message: e,
                    code: "BLENDER_BAD_PATH".into(),
                })
            }
        };
        let frame = args
            .get("frame")
            .and_then(|v| v.trim().parse::<u32>().ok())
            .unwrap_or(1);
        let out_prefix = match self.resolve_arg(args, "output") {
            Ok(p) => p,
            Err(e) => {
                return Ok(ToolResult::Error {
                    message: e,
                    code: "BLENDER_BAD_PATH".into(),
                })
            }
        };
        let argv = Self::render_args(&self.binary, &blend, frame, &out_prefix);
        match self.runner.run(&argv[0], &argv[1..], Duration::from_secs(840), Some(self.workspace.primary())) {
            Ok(out) if out.exit_code == 0 => {
                // blender 的 --render-output 前缀会自动补 ####/帧号后缀，精确文件名
                // 由场景的输出设置决定；这里按「前缀 + 帧号」的常见形态与通配两种方式核对。
                let rendered = find_rendered_file(&out_prefix, frame);
                match rendered {
                    Some(f) if std::fs::metadata(&f).map(|m| m.len()).unwrap_or(0) > 0 => {
                        Ok(ToolResult::Success {
                            data: json!({
                                "ok": true,
                                "rendered": f.to_string_lossy(),
                                "bytes": std::fs::metadata(&f).map(|m| m.len()).unwrap_or(0),
                            })
                            .to_string(),
                            metadata: HashMap::new(),
                        })
                    }
                    _ => Ok(ToolResult::Error {
                        message: format!(
                            "blender 退出码 0，但未找到渲染产物（前缀 {}）。\
                             可能场景输出分辨率/引擎未配置。",
                            out_prefix.display()
                        ),
                        code: "BLENDER_EMPTY_OUTPUT".into(),
                    }),
                }
            }
            Ok(out) => Ok(ToolResult::Error {
                message: format!(
                    "blender 渲染失败（exit {}）：{}",
                    out.exit_code,
                    tail(&out.stderr, 900)
                ),
                code: "BLENDER_RENDER_FAILED".into(),
            }),
            Err(e) => Ok(map_runner_err(e)),
        }
    }
}

/// 渲染产物定位：`--render-output` 前缀 + Blender 自动补的帧号后缀。
///
/// 依次尝试常见形态：`<prefix>0001.png` / `<prefix>.0001.png` / `<prefix>0001` / `<prefix>.png`。
fn find_rendered_file(prefix: &Path, frame: u32) -> Option<PathBuf> {
    let p = prefix.to_string_lossy().into_owned();
    let candidates = [
        format!("{p}{frame:04}.png"),
        format!("{p}.{frame:04}.png"),
        format!("{p}{frame:04}"),
        format!("{p}.png"),
        p.clone(),
    ];
    candidates
        .iter()
        .map(PathBuf::from)
        .find(|c| c.is_file())
        .or_else(|| {
            // 兜底：同目录下最新的、以前缀为基名的文件
            let dir = prefix.parent()?;
            let base = prefix.file_stem()?.to_string_lossy().into_owned();
            let mut best: Option<(std::time::SystemTime, PathBuf)> = None;
            if let Ok(rd) = std::fs::read_dir(dir) {
                for e in rd.filter_map(|e| e.ok()) {
                    let path = e.path();
                    let name = path.file_name()?.to_string_lossy().into_owned();
                    if name.starts_with(&base) && path.is_file() {
                        let mtime = e.metadata().ok()?.modified().ok()?;
                        if best.as_ref().map(|(t, _)| mtime > *t).unwrap_or(true) {
                            best = Some((mtime, path));
                        }
                    }
                }
            }
            best.map(|(_, p)| p)
        })
}

/// 执行器错误 → 结构化工具错误
fn map_runner_err(e: anyhow::Error) -> ToolResult {
    let msg = e.to_string();
    if msg.starts_with("TIMEOUT:") {
        return ToolResult::Error {
            message: format!("Blender 命令超时：{}", msg.trim_start_matches("TIMEOUT:")),
            code: "BLENDER_TIMEOUT".into(),
        };
    }
    if msg.contains("启动 blender 失败") {
        return ToolResult::Error {
            message: format!(
                "{msg}\n未找到可用的 blender.exe。请安装 Blender 或设置 MARGUERITE_BLENDER 环境变量。"
            ),
            code: "BLENDER_BINARY_MISSING".into(),
        };
    }
    ToolResult::Error {
        message: msg,
        code: "BLENDER_EXEC_ERROR".into(),
    }
}

fn tail(s: &str, max_chars: usize) -> String {
    let n = s.chars().count();
    if n <= max_chars {
        return s.trim().to_string();
    }
    let skip = n - max_chars;
    format!("…{}", s.chars().skip(skip).collect::<String>().trim())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    struct RecordingRunner {
        calls: Mutex<Vec<(String, Vec<String>)>>,
        exit: i32,
        stdout: String,
        stderr: String,
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

    impl BlenderRunner for RecordingRunner {
        fn run(
            &self,
            program: &str,
            args: &[String],
            _t: Duration,
            _cwd: Option<&Path>,
        ) -> Result<BlenderOutput> {
            self.calls
                .lock()
                .unwrap()
                .push((program.to_string(), args.to_vec()));
            if self.write_output {
                if let Some(last) = args.last() {
                    let p = Path::new(last);
                    if p.extension().is_some() {
                        let _ = std::fs::write(p, b"fake");
                    }
                }
            }
            Ok(BlenderOutput {
                exit_code: self.exit,
                stdout: self.stdout.clone(),
                stderr: self.stderr.clone(),
            })
        }
    }

    fn ws() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "marg_blender_{}_{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn args_of(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    fn fake_binary() -> PathBuf {
        let p = std::env::temp_dir().join("marg_blender_fake_blender.exe");
        let _ = std::fs::write(&p, b"fake");
        p
    }

    #[test]
    fn run_script_args_are_structured_without_shell_metacharacters() {
        let a = BlenderTool::run_script_args(
            Path::new("blender.exe"),
            Path::new("job.py"),
            Some(Path::new("scene.blend")),
            &["--frame".into(), "12".into()],
        );
        assert_eq!(a[0], "blender.exe");
        assert!(a.windows(2).any(|w| w[0] == "--background"));
        assert!(a.windows(2).any(|w| w[0] == "--factory-startup"));
        assert!(a.windows(2).any(|w| w[0] == "--python" && w[1] == "job.py"));
        // -- 后是透传参数，其余部分不得包含任意拼接面
        let dd = a.iter().position(|s| s == "--").unwrap();
        assert_eq!(a[dd + 1], "--frame");
        assert!(
            !a.iter().any(|s| s.contains("&&") || s.contains(';') || s.contains('|')),
            "argv 不得含 shell 连接符：{a:?}"
        );
    }

    #[tokio::test]
    async fn unknown_action_rejected() {
        let tool = BlenderTool::new(ws(), RecordingRunner::new(), fake_binary());
        let r = tool
            .execute(&args_of(&[("action", "delete everything")]))
            .await
            .unwrap();
        match r {
            ToolResult::Error { code, message } => {
                assert_eq!(code, "BLENDER_UNSUPPORTED_ACTION");
                assert!(message.contains("probe"));
            }
            other => panic!("应为错误：{other:?}"),
        }
    }

    #[tokio::test]
    async fn path_escape_rejected_before_spawning() {
        let runner = RecordingRunner::new();
        let tool = BlenderTool::new(ws(), runner.clone(), fake_binary());
        let r = tool
            .execute(&args_of(&[
                ("action", "run_script"),
                ("script", "../../../Windows/System32/evil.py"),
            ]))
            .await
            .unwrap();
        match r {
            ToolResult::Error { code, .. } => assert_eq!(code, "BLENDER_BAD_PATH"),
            other => panic!("应为路径错误：{other:?}"),
        }
        assert!(
            runner.calls.lock().unwrap().is_empty(),
            "越界必须在起进程前被拒"
        );
    }

    #[tokio::test]
    async fn exit_zero_without_expected_artifact_is_failure() {
        let dir = ws();
        let script = dir.join("job.py");
        std::fs::write(&script, b"pass").unwrap();
        let runner = Arc::new(RecordingRunner {
            calls: Mutex::new(Vec::new()),
            exit: 0,
            stdout: String::new(),
            stderr: String::new(),
            write_output: false, // 模拟「退出 0 但没写产物」
        });
        let tool = BlenderTool::new(dir.clone(), runner, fake_binary());
        let r = tool
            .execute(&args_of(&[
                ("action", "run_script"),
                ("script", "job.py"),
                ("expect_file", "out.glb"),
            ]))
            .await
            .unwrap();
        match r {
            ToolResult::Error { code, .. } => assert_eq!(code, "BLENDER_EMPTY_OUTPUT"),
            other => panic!("空产物必须判失败：{other:?}"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn render_file_locator_finds_common_suffix_forms() {
        let dir = ws();
        let prefix = dir.join("renders/shot_");
        std::fs::create_dir_all(prefix.parent().unwrap()).unwrap();
        std::fs::write(dir.join("renders/shot_0001.png"), b"png").unwrap();
        let found = find_rendered_file(&prefix, 1);
        assert!(found.is_some(), "应找到 shot_0001.png");
        let _ = std::fs::remove_dir_all(dir);
    }

    /// 真实 Blender 端到端（默认忽略）：建模 → 保存 → 导出 GLB → 渲染。
    /// 本机实测：`cargo test -p marguerite-core --lib -- blender::tests --ignored`
    #[tokio::test]
    #[ignore = "需要本机 blender.exe；用 --ignored 显式运行"]
    async fn real_blender_model_export_render_end_to_end() {
        let binary = discover_blender_binary().expect("本机应已装 Blender");
        let dir = std::env::temp_dir().join(format!(
            "marg_blender_e2e_{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        // 脚本：建一个带材质的立方体 → 保存 .blend → 导出 GLB
        let script = dir.join("make_asset.py");
        std::fs::write(
            &script,
            r#"
import bpy
bpy.ops.mesh.primitive_cube_add(size=2)
obj = bpy.context.active_object
obj.name = "SM_Cube_Demo"
mat = bpy.data.materials.new("M_Cube_Demo")
mat.use_nodes = True
obj.data.materials.append(mat)
bpy.ops.wm.save_as_mainfile(filepath=bpy.path.abspath("//asset.blend"))
bpy.ops.export_scene.gltf(filepath=bpy.path.abspath("//asset.glb"), export_format='GLB')
print("E2E_DONE")
"#,
        )
        .unwrap();
        let tool = BlenderTool::new(dir.clone(), Arc::new(SystemBlenderRunner), binary);
        let r = tool
            .execute(&args_of(&[
                ("action", "run_script"),
                ("script", "make_asset.py"),
                ("expect_file", "asset.glb"),
            ]))
            .await
            .unwrap();
        let data = match r.data() {
            Some(d) => d,
            None => panic!("建模+导出应成功：{r:?}"),
        };
        assert!(data.contains("\"bytes\""), "应回报产物体积：{data}");
        assert!(dir.join("asset.blend").is_file(), ".blend 应落盘");
        assert!(dir.join("asset.glb").is_file(), "GLB 应落盘");

        // 渲染一帧（Workbench 由场景默认？这里显式用脚本建的场景默认 EEVEE——
        // 无 GPU 环境可能失败，故 render 用 --factory-startup 的默认场景另存测试）
        let render_script = dir.join("prep_render.py");
        std::fs::write(
            &render_script,
            r#"
import bpy
scn = bpy.context.scene
scn.render.engine = 'BLENDER_WORKBENCH'
bpy.ops.mesh.primitive_monkey_add()
bpy.ops.wm.save_as_mainfile(filepath=bpy.path.abspath("//render.blend"))
"#,
        )
        .unwrap();
        let r2 = tool
            .execute(&args_of(&[
                ("action", "run_script"),
                ("script", "prep_render.py"),
            ]))
            .await
            .unwrap();
        assert!(r2.data().is_some(), "渲染准备应成功：{r2:?}");

        let r3 = tool
            .execute(&args_of(&[
                ("action", "render"),
                ("blend", "render.blend"),
                ("frame", "1"),
                ("output", "renders/shot_"),
            ]))
            .await
            .unwrap();
        let data3 = r3.data().expect("渲染应成功");
        assert!(data3.contains("rendered"), "应回报渲染产物路径：{data3}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
