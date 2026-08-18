//! Python 引擎侧车子进程管理
//!
//! 运行形态：
//! ```text
//! <python_exe> -m Marguerite.cli.main gateway run \
//!     --bind 127.0.0.1 --port <port> [--debug]
//! ```
//! 环境变量：
//! - `PYTHONPATH` → `<repo>/pyengine/src`（确保 `import Marguerite`）
//! - `Marguerite_HOME` → `<data_dir>/pyengine-home`（与 Rust 数据目录隔离但同根）
//! - `Marguerite_GATEWAY_PORT` → 传递给生命周期管理器
//! - `RUST_LOG` 级别的日志级别透传到 `Marguerite_LOG_LEVEL`（如果设置）
//!
//! 生命周期：
//! 1. `SidecarConfig::detect()` → 定位 python 解释器 / pyengine 源码目录 / 空闲端口
//! 2. `PyEngineSidecar::start(cfg)` → spawn + 健康等待（带超时）
//! 3. 后台守护任务：poll `/api/health`，失败则按指数退避重启
//! 4. `drop` / `stop()` → SIGTERM（Win 下无 SIGTERM → `kill()`）

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use tokio::process::{Child, Command};
use tokio::sync::Mutex;
use tokio::time::{sleep, timeout};
use tracing::{error, info, warn};

use crate::persistence::config::Config;
use crate::pyengine::auth::write_python_home_token;

/// 侧车运行状态
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PyEngineStatus {
    Stopped,
    Starting,
    Running,
    Restarting,
    Failed,
}

/// 侧车配置（定位二进制、路径、端口）
#[derive(Debug, Clone)]
pub struct SidecarConfig {
    pub python_exe: PathBuf,
    pub pyengine_src: PathBuf,
    pub py_home: PathBuf,
    pub port: u16,
    pub shared_token: String,
    pub debug: bool,
    pub restart_backoff_min: Duration,
    pub restart_backoff_max: Duration,
    pub health_timeout: Duration,
}

impl SidecarConfig {
    /// 自动探测：找可用的 Python 3.12+ 解释器、pyengine 源码位置、空闲端口
    pub fn detect(shared_token: String) -> Result<Self> {
        let python_exe = detect_python()?;
        let pyengine_src = detect_pyengine_src()?;
        // pyengine-home 在 marguerite 数据目录同级下
        let py_home = Config::data_dir().join("pyengine-home");
        let port = free_loopback_port()?;
        Ok(Self {
            python_exe,
            pyengine_src,
            py_home,
            port,
            shared_token,
            debug: false,
            restart_backoff_min: Duration::from_millis(500),
            restart_backoff_max: Duration::from_secs(30),
            // Python 网关 boot 较慢（embedding 模型加载等），放宽到 90s
            health_timeout: Duration::from_secs(90),
        })
    }
}

fn detect_python() -> Result<PathBuf> {
    // 在 Windows 上使用 py launcher 的 3.12+；找不到再查 PATH
    let candidates: Vec<(&str, Vec<&str>)> = vec![
        ("py", vec!["-3.14", "--version"]),
        ("py", vec!["-3.13", "--version"]),
        ("py", vec!["-3.12", "--version"]),
        ("python3.14", vec!["--version"]),
        ("python3.12", vec!["--version"]),
        ("python", vec!["--version"]),
    ];
    for (exe, args) in candidates {
        if let Ok(out) = std::process::Command::new(exe).args(&args).output() {
            if out.status.success() {
                // 解析 version，要求 >=3.12
                let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
                let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
                let combined = format!("{stdout}{stderr}");
                if let Some(ver) = parse_python_version(&combined) {
                    if ver >= (3, 12) {
                        // 对于 py launcher，补回版本参数
                        if exe == "py" && args.len() >= 2 {
                            // e.g. py -3.14 需要带 -3.14 作为参数；通过 py -c 解析到真正 exe
                            let which_out = std::process::Command::new(exe)
                                .arg(args[0])
                                .arg("-c")
                                .arg("import sys; print(sys.executable)")
                                .output();
                            if let Ok(o) = which_out {
                                if o.status.success() {
                                    let path =
                                        String::from_utf8_lossy(&o.stdout).trim().to_string();
                                    if !path.is_empty() {
                                        return Ok(PathBuf::from(path));
                                    }
                                }
                            }
                        }
                        return Ok(PathBuf::from(exe));
                    }
                }
            }
        }
    }
    Err(anyhow!(
        "未找到 Python >=3.12。请安装 Python 3.12+，并确保 `py -3.14`/`python3.12` 或 `python --version` 可达"
    ))
}

fn parse_python_version(text: &str) -> Option<(u32, u32)> {
    let re = regex::Regex::new(r"Python\s+(\d+)\.(\d+)").ok()?;
    let caps = re.captures(text)?;
    let major: u32 = caps.get(1)?.as_str().parse().ok()?;
    let minor: u32 = caps.get(2)?.as_str().parse().ok()?;
    Some((major, minor))
}

fn detect_pyengine_src() -> Result<PathBuf> {
    // 显式覆盖最高优先（开发期 / 特殊部署）
    if let Ok(p) = std::env::var("MARGUERITE_PYENGINE_SRC") {
        let cand = PathBuf::from(p);
        if cand
            .join("opensquilla")
            .join("cli")
            .join("main.py")
            .is_file()
        {
            return Ok(cand);
        }
    }
    // cwd 相对候选（开发 / 脚本在仓库根运行）
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    for rel in [
        "pyengine/src",
        "../pyengine/src",
        "marguerite/pyengine/src",
        "../marguerite/pyengine/src",
    ] {
        let cand = cwd.join(rel);
        if cand
            .join("opensquilla")
            .join("cli")
            .join("main.py")
            .is_file()
        {
            return Ok(cand);
        }
    }
    // exe 所在目录上溯（GUI 打包产物双击场景：target/release → 仓库根）。
    // 桌面壳的 cwd 随启动方式漂移（资源管理器双击 = exe 目录），仅靠 cwd
    // 相对候选会让双引擎静默降级；从 exe 路径向上找仓库根更稳。
    if let Ok(exe) = std::env::current_exe() {
        if let Some(mut dir) = exe.parent() {
            for _ in 0..6 {
                let cand = dir.join("pyengine/src");
                if cand
                    .join("opensquilla")
                    .join("cli")
                    .join("main.py")
                    .is_file()
                {
                    return Ok(cand);
                }
                let nested = dir.join("marguerite/pyengine/src");
                if nested
                    .join("opensquilla")
                    .join("cli")
                    .join("main.py")
                    .is_file()
                {
                    return Ok(nested);
                }
                match dir.parent() {
                    Some(p) => dir = p,
                    None => break,
                }
            }
        }
    }
    Err(anyhow!(
        "未找到 pyengine/src（需包含 opensquilla/cli/main.py）。可通过 MARGUERITE_PYENGINE_SRC 指定"
    ))
}

fn free_loopback_port() -> Result<u16> {
    use std::net::TcpListener;
    let l = TcpListener::bind(("127.0.0.1", 0))?;
    Ok(l.local_addr()?.port())
}

/// Python 侧车子进程句柄 + 守护任务
pub struct PyEngineSidecar {
    pub config: SidecarConfig,
    child: Arc<Mutex<Option<Child>>>,
    status: Arc<std::sync::atomic::AtomicU8>,
    _guard: Option<tokio::task::JoinHandle<()>>,
}

const STATUS_STOPPED: u8 = 0;
const STATUS_STARTING: u8 = 1;
const STATUS_RUNNING: u8 = 2;
const STATUS_RESTARTING: u8 = 3;
const STATUS_FAILED: u8 = 4;

impl PyEngineSidecar {
    pub fn status(&self) -> PyEngineStatus {
        match self.status.load(std::sync::atomic::Ordering::Acquire) {
            STATUS_STOPPED => PyEngineStatus::Stopped,
            STATUS_STARTING => PyEngineStatus::Starting,
            STATUS_RUNNING => PyEngineStatus::Running,
            STATUS_RESTARTING => PyEngineStatus::Restarting,
            _ => PyEngineStatus::Failed,
        }
    }

    /// 启动侧车：spawn 一次 + 健康等待；然后 spawn 守护任务
    pub async fn start(cfg: SidecarConfig) -> Result<Self> {
        // 共享 token 落盘到 Python home（非致命：Python 网关对空 token 走 guest 放行，
        // 且 spawn_once 同时注入 Marguerite_GATEWAY_TOKEN 环境变量作为主通道）。
        // 失败只降级：Rust 客户端以空 auth 连接仍可直通 Python 方法。
        let token_ok = match write_python_home_token(&cfg.py_home, &cfg.shared_token) {
            Ok(()) => true,
            Err(e) => {
                warn!(err = ?e, "共享 token 落盘失败（降级 guest 鉴权，不影响双引擎桥）");
                false
            }
        };
        // token 落盘被拒（沙箱/杀软拦截对 pyengine-home 的写入）时，把 Python 的
        // Marguerite_HOME 降级到系统临时目录。注意：不要由 Rust 预创建该目录——
        // 沙箱里 Rust 创建的目录带受限 DACL，Python 子进程反而无法建 profile 锁；
        // 直接交给 Python 自行创建（正常权限继承）。
        let eff_home = if token_ok {
            cfg.py_home.clone()
        } else {
            let fb = std::env::temp_dir().join("marguerite-pyhome");
            warn!(home = %fb.display(), "pyengine-home 不可写，降级使用临时 home");
            fb
        };
        let child = Arc::new(Mutex::new(None));
        let status = Arc::new(std::sync::atomic::AtomicU8::new(STATUS_STARTING));

        // 首次 spawn
        spawn_once(&cfg, &eff_home, &child).await?;
        wait_healthy(&cfg, cfg.health_timeout)
            .await
            .map_err(|e| anyhow!("Python 侧车启动后健康探测失败（port={}）: {e:#}", cfg.port))?;
        status.store(STATUS_RUNNING, std::sync::atomic::Ordering::Release);
        info!(port = cfg.port, "Python 引擎侧车已启动并健康");

        // 守护任务：周期检查进程存活 + HTTP 健康；死亡则退避重启
        let child_guard = child.clone();
        let cfg_guard = cfg.clone();
        let eff_home_guard = eff_home.clone();
        let status_guard = status.clone();
        let handle = tokio::spawn(async move {
            let mut backoff = cfg_guard.restart_backoff_min;
            loop {
                sleep(Duration::from_secs(2)).await;
                let alive = {
                    let mut lock = child_guard.lock().await;
                    match lock.as_mut() {
                        Some(c) => match c.try_wait() {
                            Ok(Some(_exit)) => false,
                            Ok(None) => true,
                            Err(_) => false,
                        },
                        None => false,
                    }
                };
                let http_ok = check_health(&cfg_guard, Duration::from_secs(3)).await;
                if alive && http_ok {
                    backoff = cfg_guard.restart_backoff_min;
                    continue;
                }
                warn!(
                    alive,
                    http_ok,
                    "Python 侧车健康异常，{}ms 后重启",
                    backoff.as_millis()
                );
                status_guard.store(STATUS_RESTARTING, std::sync::atomic::Ordering::Release);
                sleep(backoff).await;
                // kill 旧进程（若有残尸）
                let mut lock = child_guard.lock().await;
                if let Some(mut c) = lock.take() {
                    let _ = c.kill().await;
                }
                drop(lock);
                let spawned = spawn_once(&cfg_guard, &eff_home_guard, &child_guard).await;
                if let Err(e) = spawned {
                    error!(err = %e, "Python 侧车重启失败");
                    status_guard.store(STATUS_FAILED, std::sync::atomic::Ordering::Release);
                } else if wait_healthy(&cfg_guard, cfg_guard.health_timeout)
                    .await
                    .is_ok()
                {
                    status_guard.store(STATUS_RUNNING, std::sync::atomic::Ordering::Release);
                    info!(port = cfg_guard.port, "Python 侧车重启成功");
                } else {
                    error!("Python 侧车重启后健康探测超时");
                    status_guard.store(STATUS_FAILED, std::sync::atomic::Ordering::Release);
                }
                backoff = (backoff * 2).min(cfg_guard.restart_backoff_max);
            }
        });

        Ok(Self {
            config: cfg,
            child,
            status,
            _guard: Some(handle),
        })
    }

    /// 基础 URL（例如 `http://127.0.0.1:18792`）
    pub fn base_url(&self) -> String {
        format!("http://127.0.0.1:{}", self.config.port)
    }

    /// 主动停止（drop 也会触发）
    pub async fn stop(&self) {
        let mut lock = self.child.lock().await;
        if let Some(mut c) = lock.take() {
            let _ = c.kill().await;
        }
        self.status
            .store(STATUS_STOPPED, std::sync::atomic::Ordering::Release);
    }
}

impl Drop for PyEngineSidecar {
    fn drop(&mut self) {
        // 最佳努力：同步层面无法 await；守护任务在引用 child 时若发现被 drop 则自行退出
        // 实际停止由 self.stop().await 保证；drop 只做一次后台 fire-and-forget
        let child = self.child.clone();
        let status = self.status.clone();
        tokio::spawn(async move {
            let mut lock = child.lock().await;
            if let Some(mut c) = lock.take() {
                let _ = c.kill().await;
            }
            status.store(STATUS_STOPPED, std::sync::atomic::Ordering::Release);
        });
    }
}

// ---------------------------------------------------------------------------
// 内部工具
// ---------------------------------------------------------------------------

async fn spawn_once(
    cfg: &SidecarConfig,
    eff_home: &Path,
    child_slot: &Arc<Mutex<Option<Child>>>,
) -> Result<()> {
    let mut cmd = Command::new(&cfg.python_exe);
    cmd.args(["-m", "opensquilla.cli.main", "gateway", "run"])
        .arg("--bind")
        .arg("127.0.0.1")
        .arg("--port")
        .arg(cfg.port.to_string());
    if cfg.debug {
        cmd.arg("--debug");
    }
    // 环境变量
    let pythonpath = match std::env::var_os("PYTHONPATH") {
        Some(existing) => {
            let mut p = std::env::split_paths(&existing).collect::<Vec<_>>();
            p.insert(0, cfg.pyengine_src.clone());
            std::env::join_paths(p).ok()
        }
        None => Some(std::ffi::OsString::from(&cfg.pyengine_src)),
    };
    if let Some(pp) = pythonpath {
        cmd.env("PYTHONPATH", pp);
    }
    cmd.env("OPENSQUILLA_HOME", eff_home);
    // token 主通道：Python default_gateway_token() 优先读该环境变量；
    // 落盘失败（如沙箱/杀软拦截）时仍保证双引擎桥可用。
    cmd.env("OPENSQUILLA_GATEWAY_TOKEN", &cfg.shared_token);
    if let Ok(level) = std::env::var("RUST_LOG") {
        // 粗略映射 rust trace level → python logging level
        let py_level = if level.contains("trace") || level.contains("debug") {
            "DEBUG"
        } else if level.contains("warn") {
            "WARNING"
        } else if level.contains("error") {
            "ERROR"
        } else {
            "INFO"
        };
        cmd.env("OPENSQUILLA_LOG_LEVEL", py_level);
    }
    // 子进程 stdout/stderr 落盘（排障用）：放系统临时目录，避免 pyengine-home
    // 被沙箱/杀软拦截时连带丢失侧车崩溃现场。
    let log_dir = std::env::temp_dir().join("marguerite-sidecar");
    let _ = std::fs::create_dir_all(&log_dir);
    let out_path = log_dir.join(format!("sidecar-{}.out.log", cfg.port));
    let err_path = log_dir.join(format!("sidecar-{}.err.log", cfg.port));
    let out_file = std::fs::File::create(&out_path)
        .with_context(|| format!("创建侧车 stdout 日志失败: {}", out_path.display()))?;
    let err_file = std::fs::File::create(&err_path)
        .with_context(|| format!("创建侧车 stderr 日志失败: {}", err_path.display()))?;
    cmd.stdout(Stdio::from(out_file))
        .stderr(Stdio::from(err_file))
        .kill_on_drop(true);
    // Windows: 静默后台运行，不弹出 Python 控制台窗口
    #[cfg(windows)]
    {
        cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }
    let spawned = cmd
        .spawn()
        .with_context(|| format!("spawn Python 失败: {}", cfg.python_exe.display()))?;
    *child_slot.lock().await = Some(spawned);
    info!(
        port = cfg.port,
        python = %cfg.python_exe.display(),
        out = %out_path.display(),
        err = %err_path.display(),
        "Python 侧车子进程已 spawn"
    );
    Ok(())
}

async fn check_health(cfg: &SidecarConfig, wait: Duration) -> bool {
    // Python 网关实际健康端点是 /health（app.py: Route("/health", health)），
    // 非 /api/health（那是 Rust 网关自己的端点）。
    let url = format!("http://127.0.0.1:{}/health", cfg.port);
    timeout(wait, async {
        match reqwest::Client::builder()
            .timeout(Duration::from_secs(2))
            .build()
        {
            Ok(c) => match c.get(&url).send().await {
                Ok(resp) => resp.status().is_success(),
                Err(_) => false,
            },
            Err(_) => false,
        }
    })
    .await
    .unwrap_or(false)
}

async fn wait_healthy(cfg: &SidecarConfig, total: Duration) -> Result<()> {
    let start = std::time::Instant::now();
    while start.elapsed() < total {
        if check_health(cfg, Duration::from_secs(2)).await {
            return Ok(());
        }
        sleep(Duration::from_millis(300)).await;
    }
    Err(anyhow!("Python 侧车 {}s 内未健康", total.as_secs()))
}

/// 从任意目录计算绝对 `pyengine/src` 路径（供集成测试覆盖）
pub fn resolve_src_for(dir: &Path) -> Option<PathBuf> {
    for rel in ["pyengine/src", "../pyengine/src"] {
        let cand = dir.join(rel);
        if cand
            .join("opensquilla")
            .join("cli")
            .join("main.py")
            .is_file()
        {
            return Some(cand);
        }
    }
    None
}
