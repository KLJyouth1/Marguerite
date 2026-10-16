//! 跨平台终端后端抽象层
//!
//! 解决的问题：
//! Windows 原生 `cmd.exe` / `powershell` 对 AI agent 执行命令不友好 —— 命令集、
//! 行结束符、管道语义、常用工具（`bash`/`grep`/`sed`/`ssh`）都与 Linux 差异巨大。
//! 本模块提供 **后端自动探测 + 统一执行**，让 Windows 用户也能在 Linux 环境里跑命令，
//! 同时在没有 Linux 运行时的机器上平滑回退到原生命令，不让产品变砖。
//!
//! # 后端优先级
//!
//! 1. **WSL**（Windows Subsystem for Linux）：`wsl.exe` 直接把命令转发到已安装的
//!    Linux 发行版，最自然的「Windows 上跑 Linux」方案；
//! 2. **Docker**：`docker run --rm -v <cwd>:/work -w /work <image> <cmd>`，
//!    无需系统级 WSL，任何装了 Docker Desktop 的机器都能用；
//! 3. **Native**：当前 OS 的原生 shell（Windows=`cmd.exe`，Unix=`$SHELL`/`/bin/sh`）。
//!
//! 用户可通过 `TerminalBackend::from_config()` 显式指定（`wsl` / `docker` / `native`），
//! 也可通过 `TerminalBackend::auto()` 自动选择最佳可用后端。
//!
//! # 安全
//!
//! - 所有后端都走 `CommandGuard`/`Preflight` 分层护栏（与 `terminal::TerminalSession` 同源），
//!   不会因为切到 WSL/Docker 就绕过 L1/L2/L3；
//! - WSL/Docker 执行时显式关闭 `-t`（伪终端），避免交互 prompt 卡死 agent；
//! - 工作目录经 `canonicalize` 后再绑定进容器，避免路径穿越。

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::Instant;

use crate::proc::SilentCommand;
use crate::security::{normalize_workspace, CommandGuard, Preflight};

/// 可用终端后端。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerminalBackend {
    /// 当前 OS 的原生 shell（Windows=cmd.exe, Unix=$SHELL/sh）。
    Native,
    /// Windows Subsystem for Linux（仅 Windows 可用，其它平台自动回退 Native）。
    Wsl,
    /// Docker 容器内执行（任意装好 docker 的平台可用）。
    Docker,
}

impl TerminalBackend {
    /// 按字符串解析，未知值返回 `None`（由调用方决定回退策略）。
    #[allow(clippy::should_implement_trait)] // 命名仿 FromStr 但返回 Option，保留为解析辅助
    pub fn from_str(s: &str) -> Option<Self> {
        Some(match s.trim().to_ascii_lowercase().as_str() {
            "wsl" | "wsl2" => Self::Wsl,
            "docker" | "container" => Self::Docker,
            "native" | "host" | "local" => Self::Native,
            _ => return None,
        })
    }

    /// 人类可读的名字（日志 / UI 展示）。
    pub fn name(self) -> &'static str {
        match self {
            Self::Native => "native",
            Self::Wsl => "wsl",
            Self::Docker => "docker",
        }
    }

    /// 在当前机器上探测该后端是否可用（有对应 CLI 且能跑一个最小探针命令）。
    pub fn is_available(self) -> bool {
        match self {
            Self::Native => true, // 原生 shell 永远可用
            Self::Wsl => {
                if !cfg!(windows) {
                    return false; // WSL CLI 仅在 Windows 上有意义
                }
                // 先找 wsl.exe，再跑 `wsl -l -q` 看是否至少有一个发行版
                which_silent("wsl.exe") || which_silent("wsl")
            }
            Self::Docker => which_silent("docker") && docker_daemon_up(),
        }
    }

    /// 自动选择当前机器上最优的可用后端（WSL > Docker > Native）。
    pub fn auto() -> Self {
        resolve_auto_backend(
            Self::Wsl.is_available(),
            Self::Docker.is_available(),
            cfg!(windows),
        )
        .0
    }
}

/// 按宿主平台解析自动后端，并在 Windows 缺少 Linux 运行时​​时返回明确错误。
///
/// 该函数把“选哪个后端”和“是否允许执行”放在同一个结果里，避免调用方
/// 只拿到 Native 作为兜底后误启动 cmd.exe。
fn resolve_auto_backend(
    wsl_available: bool,
    docker_available: bool,
    is_windows: bool,
) -> (TerminalBackend, Result<(), String>) {
    if wsl_available {
        return (TerminalBackend::Wsl, Ok(()));
    }
    if docker_available {
        return (TerminalBackend::Docker, Ok(()));
    }
    if is_windows {
        return (
            TerminalBackend::Native,
            Err(
                "Linux 语义终端不可用：请安装并启动 WSL2 或 Docker Desktop；不会回退到 cmd.exe"
                    .to_string(),
            ),
        );
    }
    (TerminalBackend::Native, Ok(()))
}

/// 终端后端的配置（容器镜像 / WSL 发行版名等可覆盖项）。
#[derive(Debug, Clone)]
pub struct TerminalBackendConfig {
    /// 强制指定后端，`None` 表示自动探测。
    pub backend: Option<TerminalBackend>,
    /// WSL 发行版名（空字符串=默认发行版）。
    pub wsl_distro: String,
    /// Docker 镜像名（默认 `ubuntu:24.04`，可换成 `debian:stable` 等）。
    pub docker_image: String,
    /// 挂载到容器内的工作目录（Linux 路径），默认 `/work`。
    pub docker_workdir: String,
}

impl Default for TerminalBackendConfig {
    fn default() -> Self {
        Self {
            backend: None,
            wsl_distro: String::new(),
            docker_image: "ubuntu:24.04".to_string(),
            docker_workdir: "/work".to_string(),
        }
    }
}

impl TerminalBackendConfig {
    pub fn with_backend(mut self, b: TerminalBackend) -> Self {
        self.backend = Some(b);
        self
    }
    pub fn with_distro(mut self, d: impl Into<String>) -> Self {
        self.wsl_distro = d.into();
        self
    }
    pub fn with_image(mut self, img: impl Into<String>) -> Self {
        self.docker_image = img.into();
        self
    }
}

/// 后端解析结果（给调用方看的最终执行后端）。
#[derive(Debug, Clone)]
pub struct ResolvedBackend {
    pub backend: TerminalBackend,
    /// 该后端的可用状态（`Ok`=可用，`Err`=不可用的原因）。
    pub available: Result<(), String>,
}

impl ResolvedBackend {
    /// 根据配置解析最终后端：优先用户指定，再回退自动探测。
    pub fn resolve(cfg: &TerminalBackendConfig) -> Self {
        let (chosen, auto_status) = match cfg.backend {
            Some(b) => {
                let status = if cfg!(windows) && b == TerminalBackend::Native {
                    Err(
                        "Windows 上禁止显式选择 native：Linux 语义终端不可用时不会回退到 cmd.exe"
                            .to_string(),
                    )
                } else if b.is_available() {
                    Ok(())
                } else {
                    Err(format!("后端 {} 不可用（未安装或未启动）", b.name()))
                };
                (b, status)
            }
            None => {
                
                resolve_auto_backend(
                    TerminalBackend::Wsl.is_available(),
                    TerminalBackend::Docker.is_available(),
                    cfg!(windows),
                )
            }
        };
        let available = auto_status;
        // Windows 缺少 WSL/Docker 时保留不可用状态，禁止静默启动 cmd.exe。
        let final_backend = chosen;
        Self {
            backend: final_backend,
            available,
        }
    }
}

// --------------------------------------------------------------------------- #
// 命令执行器
// --------------------------------------------------------------------------- #

/// 统一的命令执行结果。
#[derive(Debug, Clone)]
pub struct ExecResult {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
    pub backend: TerminalBackend,
    pub duration_ms: u64,
}

/// 在给定后端下执行一条命令（行模式，阻塞）。
///
/// 所有后端统一走 `Preflight` + `CommandGuard` 分层护栏，工作目录会被规范化后再
/// 传递给底层。WSL / Docker 会显式关闭 `-t` 以避免交互 prompt 卡死。
pub fn run_command(
    backend: TerminalBackend,
    cfg: &TerminalBackendConfig,
    cwd: &Path,
    cmd: &str,
) -> std::result::Result<ExecResult, String> {
    // 1) 分层安全检查（与 `terminal::TerminalSession` 同源）
    let preflight = Preflight::new();
    let guard = CommandGuard::new();
    let workspace = normalize_workspace(cwd);
    let decision = preflight.check_exec(cmd, &workspace);
    if let Some(reason) = decision.reason() {
        return Err(format!(
            "[{}] 命令被分层安全护栏阻断: {}",
            decision.layer().unwrap_or("L?"),
            reason
        ));
    }
    if let Err(e) = guard.check(cmd) {
        return Err(format!("[L1] 命令被安全策略阻断: {e}"));
    }

    // 2) 规范化工作目录（避免越界 + 容器挂载路径稳定）
    let canon = cwd
        .canonicalize()
        .map_err(|e| format!("工作目录无法规范化: {e}"))?;

    // 3) 根据后端构造 Command
    let started = Instant::now();
    let (stdout, stderr, exit_code) = match backend {
        TerminalBackend::Native => run_native(&canon, cmd)?,
        TerminalBackend::Wsl => run_wsl(&canon, cfg, cmd)?,
        TerminalBackend::Docker => run_docker(&canon, cfg, cmd)?,
    };
    let duration_ms = started.elapsed().as_millis() as u64;

    Ok(ExecResult {
        stdout,
        stderr,
        exit_code,
        backend,
        duration_ms,
    })
}

/// Native：当前 OS 的原生 shell（Windows `cmd.exe /C`，Unix `$SHELL -c`）。
fn run_native(cwd: &Path, cmd: &str) -> std::result::Result<(String, String, i32), String> {
    let shell = if cfg!(windows) {
        PathBuf::from("cmd.exe")
    } else {
        PathBuf::from(std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string()))
    };
    let mut command = Command::new(&shell);
    command
        .arg(if cfg!(windows) { "/C" } else { "-c" })
        .arg(cmd)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }
    spawn_and_capture(command)
}

/// WSL：`wsl.exe [--distro X] -- bash -lc <cmd>`
///
/// 使用 `bash -lc` 确保加载 `.profile`/`.bashrc` 里的 PATH，且 `-l` login shell
/// 语义让常用命令（`apt`/`npm`/`python3`）都能在 WSL 内找到。
fn run_wsl(
    cwd: &Path,
    cfg: &TerminalBackendConfig,
    cmd: &str,
) -> std::result::Result<(String, String, i32), String> {
    if !cfg!(windows) {
        return Err("WSL 后端仅在 Windows 平台可用".to_string());
    }
    let mut command = Command::new("wsl.exe");
    if !cfg.wsl_distro.is_empty() {
        command.arg("--distro").arg(&cfg.wsl_distro);
    }
    // -- 关闭伪终端，避免交互 prompt 卡死；-e 使用默认 shell，-d 指定发行版
    command.arg("--cd").arg(cwd);
    command.arg("--").arg("bash").arg("-lc").arg(cmd);
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }
    spawn_and_capture(command)
}

/// Docker：`docker run --rm -v <cwd>:/work -w /work <image> bash -lc <cmd>`
///
/// - `--rm`：执行完即删容器，不堆积僵尸容器；
/// - `-v cwd:/work`：把本地工作目录原封不动挂进容器；
/// - `-w /work`：容器内 cwd 指向挂载点，agent 操作的就是用户真实文件；
/// - 不开启 `-t`（无伪终端），避免交互 prompt 卡死。
fn run_docker(
    cwd: &Path,
    cfg: &TerminalBackendConfig,
    cmd: &str,
) -> std::result::Result<(String, String, i32), String> {
    let workdir = cfg.docker_workdir.trim_start_matches('/');
    let mut command = Command::new("docker");
    command
        .silent()
        .arg("run")
        .arg("--rm")
        .arg("-v")
        .arg(format!("{}:{}", cwd.to_string_lossy(), cfg.docker_workdir))
        .arg("-w")
        .arg(if workdir.is_empty() {
            "/".to_string()
        } else {
            format!("/{}", workdir)
        })
        .arg(&cfg.docker_image)
        .arg("bash")
        .arg("-lc")
        .arg(cmd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    spawn_and_capture(command)
}

/// 通用 spawn + 合并捕获 + ANSI 剥离。
fn spawn_and_capture(mut command: Command) -> std::result::Result<(String, String, i32), String> {
    let mut child = command
        .spawn()
        .map_err(|e| format!("启动子进程失败: {e}"))?;
    let out_h = spawn_reader(child.stdout.take());
    let err_h = spawn_reader(child.stderr.take());
    let status = child.wait().map_err(|e| format!("等待子进程失败: {e}"))?;
    let out = out_h.join().unwrap_or_default();
    let err = err_h.join().unwrap_or_default();
    let code = status.code().unwrap_or(-1);
    Ok((
        crate::terminal::strip_ansi(&out),
        crate::terminal::strip_ansi(&err),
        code,
    ))
}

fn spawn_reader<R: Read + Send + 'static>(r: Option<R>) -> thread::JoinHandle<String> {
    thread::spawn(move || {
        let mut s = String::new();
        if let Some(mut r) = r {
            let _ = r.read_to_string(&mut s);
        }
        s
    })
}

// --------------------------------------------------------------------------- #
// 探测工具
// --------------------------------------------------------------------------- #

/// 静默探测某 CLI 是否存在（`where` on Windows, `which` on Unix）。
fn which_silent(name: &str) -> bool {
    let mut probe = if cfg!(windows) {
        let mut c = Command::new("where");
        c.silent().arg(name);
        c
    } else {
        let mut c = Command::new("which");
        c.silent().arg(name);
        c
    };
    probe
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// 轻量探测 docker daemon 是否在线（`docker info` 3 秒超时）。
fn docker_daemon_up() -> bool {
    // 在 Windows 上 `docker` CLI 也能直接找到（Docker Desktop 装完后 PATH 里就有），
    // 跟 `which` 保持一致；不做额外 cmd 包裹。
    let mut probe = Command::new("docker");
    probe
        .silent()
        .arg("info")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    // 用一个独立线程做超时保护（std::process::Command 没有 timeout）
    let handle = thread::spawn(move || probe.status().map(|s| s.success()).unwrap_or(false));
    // 3 秒超时
    let start = Instant::now();
    while !handle.is_finished() {
        if start.elapsed().as_millis() > 3000 {
            return false;
        }
        thread::yield_now();
    }
    handle.join().unwrap_or(false)
}

/// 汇总所有后端的可用状态（供前端 `terminal.backends.status` RPC 使用）。
pub fn status_all(cfg: &TerminalBackendConfig) -> serde_json::Value {
    let mut backends = Vec::new();
    for b in [
        TerminalBackend::Native,
        TerminalBackend::Wsl,
        TerminalBackend::Docker,
    ] {
        let available = b.is_available();
        backends.push(serde_json::json!({
            "name": b.name(),
            "available": available,
        }));
    }
    let resolved = ResolvedBackend::resolve(cfg);
    serde_json::json!({
        "autoPreferred": TerminalBackend::auto().name(),
        "resolved": resolved.backend.name(),
        "warning": resolved.available.err(),
        "backends": backends,
        "dockerImage": cfg.docker_image,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backend_from_str_accepts_known_values() {
        assert_eq!(TerminalBackend::from_str("wsl"), Some(TerminalBackend::Wsl));
        assert_eq!(
            TerminalBackend::from_str("WSL2"),
            Some(TerminalBackend::Wsl)
        );
        assert_eq!(
            TerminalBackend::from_str("docker"),
            Some(TerminalBackend::Docker)
        );
        assert_eq!(
            TerminalBackend::from_str("native"),
            Some(TerminalBackend::Native)
        );
        assert_eq!(
            TerminalBackend::from_str("host"),
            Some(TerminalBackend::Native)
        );
        assert_eq!(TerminalBackend::from_str("unknown"), None);
    }

    #[test]
    fn native_backend_always_available() {
        assert!(TerminalBackend::Native.is_available());
    }

    #[test]
    fn run_native_echo_roundtrip() {
        let dir =
            std::env::temp_dir().join(format!("marguerite_backend_test_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let cfg = TerminalBackendConfig::default();
        let res = run_command(TerminalBackend::Native, &cfg, &dir, "echo hello-backend")
            .expect("native exec ok");
        assert_eq!(res.backend, TerminalBackend::Native);
        assert!(
            res.stdout.contains("hello-backend"),
            "应捕获 stdout，实际: {:?}",
            res.stdout
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn run_native_blocks_l1_dangerous_commands() {
        let dir =
            std::env::temp_dir().join(format!("marguerite_backend_block_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let cfg = TerminalBackendConfig::default();
        let res = run_command(TerminalBackend::Native, &cfg, &dir, "sudo rm -rf /");
        assert!(res.is_err(), "L1 高危命令必须被拦截: {res:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn status_all_returns_all_backends() {
        let cfg = TerminalBackendConfig::default();
        let s = status_all(&cfg);
        let backends = s["backends"].as_array().expect("backends 数组存在");
        assert_eq!(backends.len(), 3);
        // 当前解析结果 = auto() 的输出（WSL / Docker / Native 中最优）
        let auto = TerminalBackend::auto().name();
        assert_eq!(s["resolved"].as_str(), Some(auto));
    }

    #[test]
    fn resolve_fallback_when_requested_backend_unavailable() {
        let cfg = TerminalBackendConfig::default().with_backend(TerminalBackend::Docker);
        let resolved = ResolvedBackend::resolve(&cfg);
        // 不可用后端必须保留失败状态，不能静默执行宿主 shell。
        if !TerminalBackend::Docker.is_available() {
            assert!(resolved.available.is_err());
        }
    }

    #[cfg(windows)]
    #[test]
    fn windows_auto_resolution_blocks_execution_without_linux_backend() {
        let (backend, available) = resolve_auto_backend(false, false, true);
        assert_eq!(backend, TerminalBackend::Native);
        assert!(available.is_err());
        assert!(available.unwrap_err().contains("Linux 语义终端不可用"));
    }

    #[cfg(unix)]
    #[test]
    fn unix_auto_resolution_uses_native_shell() {
        let (backend, available) = resolve_auto_backend(false, false, false);
        assert_eq!(backend, TerminalBackend::Native);
        assert!(available.is_ok());
    }

    #[cfg(windows)]
    #[test]
    fn windows_explicit_native_backend_is_rejected() {
        let cfg = TerminalBackendConfig::default().with_backend(TerminalBackend::Native);
        let resolved = ResolvedBackend::resolve(&cfg);
        assert!(resolved.available.is_err());
        assert!(resolved
            .available
            .unwrap_err()
            .contains("不会回退到 cmd.exe"));
    }
}
