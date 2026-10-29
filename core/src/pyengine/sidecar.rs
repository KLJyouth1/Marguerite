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

use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;

/// 最近一次侧车依赖预检的结论（进程内单例，供 `system.health` 读取）。
///
/// 为什么要有全局：预检发生在 spawn 时（没有 RPC 上下文），而**用户看到的**
/// 是 Engine Health 面板。不落一份可读状态，就只剩日志 —— 而真机审计证明
/// 用户不会看日志：22 条 `No module named 'numpy'` 在日志里刷了一整屏，
/// 界面上却什么都没有，于是「本地智能路由静默失效」被当成"这版本就这样"。
///
/// 用 `RwLock<Option<_>>` 而不是 `OnceLock`：侧车会重启，重启后结论可能变化
/// （用户补了依赖），必须允许覆盖。
static PREFLIGHT: std::sync::RwLock<Option<PreflightReport>> = std::sync::RwLock::new(None);

/// 侧车依赖预检结论。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreflightReport {
    /// 实际跑起来的解释器绝对路径（不是配置里写的那个 —— 两者可能不同）。
    pub python: String,
    /// 实际注入 `PYTHONPATH` 的 pyengine 源码目录（本仓有 3 份副本，必须可见）。
    pub pyengine_src: String,
    /// 缺失的模块名（空 = 全部就位）。
    pub missing: Vec<String>,
}

impl PreflightReport {
    /// 是否降级（有缺失模块）。
    pub fn degraded(&self) -> bool {
        !self.missing.is_empty()
    }

    /// 转成 `system.health` 里的 JSON 片段。
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "python": self.python,
            "pyengine_src": self.pyengine_src,
            "missing": self.missing,
            "degraded": self.degraded(),
            // 给前端的稳定文案键：UI 负责翻译，后端不拼中文（i18n 纪律）。
            "reason": if self.missing.iter().any(|m| m == "numpy") {
                "local_ml_router_disabled"
            } else if self.degraded() {
                "dependencies_missing"
            } else {
                "ok"
            },
        })
    }
}

/// 读最近一次预检结论（`None` = 还没跑过 / 预检失败）。
pub fn preflight_report() -> Option<PreflightReport> {
    PREFLIGHT
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
}

/// 覆盖预检结论（测试与预检本体共用）。
pub fn set_preflight_report(report: PreflightReport) {
    *PREFLIGHT.write().unwrap_or_else(|e| e.into_inner()) = Some(report);
}

#[cfg(test)]
mod preflight_report_tests {
    use super::*;

    fn report(missing: &[&str]) -> PreflightReport {
        PreflightReport {
            python: "C:\\Python314\\python.exe".to_string(),
            pyengine_src: "D:\\repo\\pyengine\\src".to_string(),
            missing: missing.iter().map(|m| (*m).to_string()).collect(),
        }
    }

    #[test]
    fn clean_report_is_not_degraded() {
        let j = report(&[]).to_json();
        assert_eq!(j["degraded"], false);
        assert_eq!(j["reason"], "ok");
        assert_eq!(j["missing"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn missing_numpy_maps_to_local_ml_router_reason() {
        // 这条 reason 是给前端做**具体**说明用的：不是笼统的"依赖缺失"，
        // 而是"本地 ML 路由被禁用"——用户才知道自己丢了哪个能力。
        let j = report(&["numpy"]).to_json();
        assert_eq!(j["degraded"], true);
        assert_eq!(j["reason"], "local_ml_router_disabled");
    }

    #[test]
    fn other_missing_modules_use_generic_reason() {
        let j = report(&["httpx"]).to_json();
        assert_eq!(j["degraded"], true);
        assert_eq!(j["reason"], "dependencies_missing");
    }

    #[test]
    fn structural_fields_are_exposed_for_troubleshooting() {
        // pyengine 有 3 份副本，界面上必须能直接看到"实际跑的是哪份"。
        let j = report(&[]).to_json();
        assert_eq!(j["python"], "C:\\Python314\\python.exe");
        assert_eq!(j["pyengine_src"], "D:\\repo\\pyengine\\src");
    }
}
use std::time::{Duration, Instant};

use anyhow::{anyhow, Context, Result};
use tokio::process::{Child, Command};
use tokio::sync::Mutex;
use tokio::time::{sleep, timeout};
use tracing::{error, info, warn};

use crate::proc::SilentCommand;

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
    /// P0-1：崩溃循环熔断——守护任务已放弃自动重启，等待人工显式 sidecar_start。
    Degraded,
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

    /// 复用已在跑的侧车时改写端口（探测到的真实端口取代随机空闲端口）。
    pub fn with_port(mut self, port: u16) -> Self {
        self.port = port;
        self
    }
}

/// 安装包内置 Python 的候选路径（纯函数，便于守卫）。
///
/// 安装布局是 `<install>/marguerite-gui.exe` + `<install>/resources/python/python.exe`；
/// 从 `target/release/` 直接跑时也允许上溯几层命中 `resources/`。
/// **内置优先于系统 Python**：用户机器上不一定装了 3.12，而内置那份的依赖是齐的。
pub(crate) fn bundled_python_candidates(exe: Option<&Path>) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Some(exe) = exe {
        if let Some(mut dir) = exe.parent() {
            for _ in 0..4 {
                out.push(dir.join("resources").join("python").join("python.exe"));
                match dir.parent() {
                    Some(p) => dir = p,
                    None => break,
                }
            }
        }
    }
    out
}

pub fn detect_python() -> Result<PathBuf> {
    // ① 安装包自带的解释器：无需用户预装任何 Python
    let exe = std::env::current_exe().ok();
    for candidate in bundled_python_candidates(exe.as_deref()) {
        if candidate.is_file() {
            return Ok(candidate);
        }
    }

    // ② 系统 Python：py launcher 的 3.14/3.13/3.12；找不到再查 PATH
    let candidates: Vec<(&str, Vec<&str>)> = vec![
        ("py", vec!["-3.14", "--version"]),
        ("py", vec!["-3.13", "--version"]),
        ("py", vec!["-3.12", "--version"]),
        ("python3.14", vec!["--version"]),
        ("python3.12", vec!["--version"]),
        ("python", vec!["--version"]),
    ];
    for (exe, args) in candidates {
        if let Ok(out) = std::process::Command::new(exe).silent().args(&args).output() {
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
                                .silent()
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
        "未找到 Python >=3.12：安装包应自带 `resources/python/python.exe`；系统安装可用 `py -3.12` / `python3.12` / `python --version` 验证"
    ))
}

pub fn parse_python_version(text: &str) -> Option<(u32, u32)> {
    let re = regex::Regex::new(r"Python\s+(\d+)\.(\d+)").ok()?;
    let caps = re.captures(text)?;
    let major: u32 = caps.get(1)?.as_str().parse().ok()?;
    let minor: u32 = caps.get(2)?.as_str().parse().ok()?;
    Some((major, minor))
}

/// 判定一个候选目录是不是真的 pyengine 源码根（含 `opensquilla/cli/main.py`）。
fn is_pyengine_src(dir: &Path) -> bool {
    dir.join("opensquilla").join("cli").join("main.py").is_file()
}

/// 在候选列表里挑第一个合法的源码根（纯函数，便于守卫）。
fn first_valid_src<I: IntoIterator<Item = PathBuf>>(candidates: I) -> Option<PathBuf> {
    candidates.into_iter().find(|c| is_pyengine_src(c))
}

/// 除显式覆盖外的候选顺序（纯函数，便于守卫）。
///
/// 顺序即优先级，**打包布局必须先于仓库布局**：安装版的 exe 与资源同在安装目录
/// （`<install>/marguerite-gui.exe` + `<install>/resources/pyengine/src`），
/// 而用户机器上恰好存在一份同名仓库目录时，仓库布局会指向**别人的**源码树。
pub(crate) fn pyengine_src_candidates(cwd: &Path, exe: Option<&Path>) -> Vec<PathBuf> {
    let mut out = Vec::new();
    // ① 打包资源布局（Tauri 2：resources/ 与 exe 同级；安装版就是这条）
    if let Some(exe) = exe {
        if let Some(dir) = exe.parent() {
            out.push(dir.join("resources/pyengine/src"));
        }
    }
    // ② cwd 相对候选（开发 / 脚本在仓库根运行）
    for rel in [
        "pyengine/src",
        "../pyengine/src",
        "marguerite/pyengine/src",
        "../marguerite/pyengine/src",
    ] {
        out.push(cwd.join(rel));
    }
    // ③ exe 上溯（`target/release/marguerite-gui.exe` 双击 → 仓库根）
    if let Some(exe) = exe {
        if let Some(mut dir) = exe.parent() {
            for _ in 0..6 {
                out.push(dir.join("pyengine/src"));
                out.push(dir.join("marguerite/pyengine/src"));
                // 每层也看一次 resources/：某些安装器把资源放在安装目录的子级
                out.push(dir.join("resources/pyengine/src"));
                match dir.parent() {
                    Some(p) => dir = p,
                    None => break,
                }
            }
        }
    }
    out
}

fn detect_pyengine_src() -> Result<PathBuf> {
    // 显式覆盖最高优先（开发期 / 特殊部署）
    if let Ok(p) = std::env::var("MARGUERITE_PYENGINE_SRC") {
        if let Some(hit) = first_valid_src([PathBuf::from(p)]) {
            return Ok(hit);
        }
    }
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let exe = std::env::current_exe().ok();
    if let Some(hit) = first_valid_src(pyengine_src_candidates(&cwd, exe.as_deref())) {
        return Ok(hit);
    }
    Err(anyhow!(
        "未找到 pyengine/src（需包含 opensquilla/cli/main.py）。可通过 MARGUERITE_PYENGINE_SRC 指定"
    ))
}

/// 只读的 pyengine 源码根探测（doctor 用）。
///
/// 与 sidecar 启动共用同一条候选链，因此 doctor 报告的路径**就是**运行时会加载的
/// 那一份，不会出现「体检看 A、运行读 B」的分裂。
pub(crate) fn locate_pyengine_src() -> Option<PathBuf> {
    detect_pyengine_src().ok()
}

pub fn free_loopback_port() -> Result<u16> {
    use std::net::TcpListener;
    let l = TcpListener::bind(("127.0.0.1", 0))?;
    Ok(l.local_addr()?.port())
}

// ── 侧车复用（profile 锁冲突的治本路径）────────────────────────────────────
//
// Python 侧车对同一 profile 是**单实例**（`OPENSQUILLA_PROFILE_IN_USE`）：进程持有
// profile 锁期间，第二次 `gateway run` 会立刻退出。而 `detect()` 每次都会挑一个
// 新的空闲端口并**再拉一个进程**，于是「上一次 Marguerite 被强杀、侧车变孤儿继续
// 占锁」这种残留会让**之后每一次**启动的侧车都秒退；`maybe_bootstrap_pyengine`
// 只打一条 warn 就降级，`py_client()` 恒为 `None` —— 所有侧车方法（`logs.tail` /
// `logs.status` / `config.get` / `onboarding.*` / `channels.*` …）静默回
// NOT_IMPLEMENTED，界面上表现为「日志加载失败」「设置读不到」等一连串怪现象。
//
// 治本方式：启动前先**探测可复用的侧车**（记录文件 → Python 侧 pidfile → 反查监听端口），
// 健康就附着使用，而不是无脑再拉一个。

// Rust 侧记下的「我拉起的侧车」：`{pid, port}`，用于下次启动复用。
//
// 之所以还要落这个文件：Rust 直接 spawn `gateway run`，绕过了Python 自己的
// 生命周期管理器，所以 `<py_home>/default/state/gateway.json`（含 port）不会被写。
//
// # 测试隔离
//
// `Config::set_config_dir_override` **只**覆盖 `config_dir()`，而这里用的是
// `data_dir()`（两者在 Windows 上分别落在 `~/.marguerite` 与 `%LOCALAPPDATA%`）。
// 任何测试若直接调用 [`save_record`] / [`save_last_exit`] 而不隔离，都会写到
// **真实用户数据目录**并可能覆盖用户真实的 `sidecar.json`——这正是本任务首次
// 引入落盘用例时踩到的坑。因此这里另开一个**模块内**的线程局部覆盖。
//
// 用 `//` 而非 `///`：`thread_local!` 是宏调用，rustdoc 不为宏产物生成文档，
// 挂 `///` 会触发 `unused_doc_comments` 警告。
#[cfg(test)]
thread_local! {
    static RECORD_PATH_OVERRIDE: std::cell::RefCell<Option<PathBuf>> =
        const { std::cell::RefCell::new(None) };
}

/// 测试辅助：把 [`record_path`] 指向临时目录，避免写真实用户数据。
#[cfg(test)]
pub fn set_record_path_override(path: PathBuf) {
    RECORD_PATH_OVERRIDE.with(|slot| *slot.borrow_mut() = Some(path));
}

pub fn record_path() -> PathBuf {
    #[cfg(test)]
    if let Some(p) = RECORD_PATH_OVERRIDE.with(|slot| slot.borrow().clone()) {
        return p;
    }
    Config::data_dir().join("sidecar.json")
}

/// 可复用的侧车记录。
///
/// `last_exit` 是**上一次**退出的结构化证据（S5）：侧车崩溃往往与网关自身的
/// 退出同时发生，若证据只活在内存里，最需要它的时刻（用户重开应用、UI 断连）
/// 恰恰读不到。落盘后由 `load_record()` 带出，供 `host.exit_evidence` 与
/// `candidate_sidecar_ports` 复用。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SidecarRecord {
    pub pid: u32,
    pub port: u16,
    /// 上一次退出的证据；从未退出过则为 `None`。
    pub last_exit: Option<HostExitRecord>,
}

/// 上一次侧车退出的落盘形态：在 [`HostExitEvidence`] 之上补一个墙钟时间戳
/// （`at_ms`，Unix 毫秒），让消费方能判断证据是否已过期。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct HostExitRecord {
    /// 退出码；被信号杀死（Unix）时为 `None`。
    pub code: Option<i32>,
    /// 从 spawn 成功到检测到退出的存活时长（毫秒）。
    pub uptime_ms: u64,
    /// 记录时刻（Unix 毫秒）。
    pub at_ms: u64,
    /// stderr 尾部（≤16KB，已按 UTF-8 字符边界回退）。
    pub stderr_tail: String,
    /// 是否**确实观测到**进程消失；缺省（老记录）视为 `false`。
    ///
    /// 复用路径的证据 `code` 恒为 `None`（拿不到 `ExitStatus`），若再没有 stderr，
    /// 消费方就再也分不清「真实发生过退出」与「从来没有证据」——见
    /// [`HostExitEvidence::observed`]。
    #[serde(default)]
    pub observed: bool,
}

/// 宿主（Python 侧车）退出的结构化证据（S5）。
///
/// 此前 Rust 侧两处拿到了 `ExitStatus` 却直接丢弃（`early_exit_reason` 与守护任务），
/// 于是「侧车为什么没了」只剩一句「健康探测失败」。本结构把三样确定可得的事实固定下来：
///
/// - `code`：进程退出码（信号终止 → `None`）
/// - `uptime_ms`：从 spawn 成功到退出的时长（区分「秒退」与「跑了一阵才死」）
/// - `stderr_tail`：stderr 尾部（Python 侧 `typer` 的启动失败横幅、traceback 都在这里）
///
/// # 安全边界
///
/// `stderr_tail` 可能含路径 / URL / 用户名。**它只允许进日志与折叠详情**，
/// 绝不可做主文案或错误码插值（与 `crate::model::error_taxonomy` 的既有纪律一致）。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct HostExitEvidence {
    /// 退出码；被信号杀死（Unix）时为 `None`。
    pub code: Option<i32>,
    /// 从 spawn 成功到退出的存活时长（毫秒）。
    pub uptime_ms: u64,
    /// stderr 尾部，最多 [`STDERR_TAIL_MAX_BYTES`] 字节。
    pub stderr_tail: String,
    /// 这条证据是否**确实观测到了一次进程消失**（S5：复用路径修复）。
    ///
    /// 为什么需要它：`host.exit_evidence` 过去用
    /// `code.is_some() || !stderr_tail.is_empty()` 判断「有没有证据」。复用路径
    /// （[`PyEngineSidecar::attach`]）下我们**不是**子进程的父进程，拿不到
    /// `ExitStatus` → `code` 恒为 `None`；而被 `taskkill /F` 掉的健康侧车 stderr
    /// 往往**是空的**。两者同时成立时，那条真实发生的退出就会被这个判据过滤掉，
    /// RPC 退回 `exited:false` —— 恰好复现了本缺陷的现象。
    ///
    /// 有了这个显式标记，「观测到进程消失」本身即可断言，不再从 `code` /
    /// `stderr_tail` 是否为空去反推。`#[serde(default)]`：旧的落盘记录没有这个
    /// 字段，读出来是 `false`，行为与修复前完全一致（不破坏跨重启可读性）。
    #[serde(default)]
    pub observed: bool,
}

/// `stderr_tail` 的字节上限（16KB）。
///
/// 取 16KB 是权衡：Python 启动失败的 traceback 通常 < 4KB，16KB 足够装下完整
/// traceback + 若干条 warning；再大只会挤占 WS 帧预算（单帧上限 16MB，但 UI
/// 拉一次就要传一次），且真正的第一行错误几乎总在尾部。
pub const STDERR_TAIL_MAX_BYTES: usize = 16 * 1024;

/// 当前墙钟时间（Unix 毫秒）。仅用于证据落盘的时间戳，不参与任何判定。
fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// 读文件尾部最多 `max` 字节并按 UTF-8 解码。
///
/// # 为什么必须回退字符边界
///
/// 尾切是**按字节**做的（`seek` + 读 `max` 字节），而 UTF-8 多字节序列可能正好被
/// 切断在中间。直接 `String::from_utf8_lossy` 会把残字节渲染成 `U+FFFD` 替换符，
/// 在错误现场里出现一堆 `�` 反而更难读。这里向前回退到下一个字符起始边界，
/// 宁可少给 1–3 个字节，也不产出替换符。
///
/// 文件不存在 / 读失败（子进程还没落盘、日志被清）一律回退空串：证据缺失不该
/// 让「记录退出原因」这条路径本身失败。
pub fn tail_bytes(path: &Path, max: usize) -> String {
    if max == 0 {
        return String::new();
    }
    let Ok(mut file) = std::fs::File::open(path) else {
        return String::new();
    };
    let Ok(len) = file.metadata().map(|m| m.len()) else {
        return String::new();
    };
    let start = len.saturating_sub(max as u64);
    let skip = match file.seek(SeekFrom::Start(start)) {
        Ok(_) => start,
        // 少数文件系统（部分网络盘 / 管道）不支持 seek；退化为整读再尾切。
        Err(_) => 0,
    };
    let mut buf = Vec::new();
    if file.read_to_end(&mut buf).is_err() {
        return String::new();
    }
    // 整读退化路径下需要自己尾切；seek 成功时 `skip == start` 且 buf 已是尾部。
    if skip == 0 && (buf.len() as u64) > max as u64 {
        buf = buf.split_off(buf.len() - max);
    }
    match std::str::from_utf8(&buf) {
        Ok(text) => text.trim().to_string(),
        Err(_) => decode_after_boundary(&buf),
    }
}

/// 从 `buf` 的第一个**合法字符边界**开始解码，逐个跳过开头的不完整序列。
///
/// 关键点：`Utf8Error::valid_up_to()` 给的是「合法前缀长度」，而切在中间的
/// 那个多字节字符本身就落在 `[valid_up_to, valid_up_to + error_len)` 这段
/// **无效**字节里——只取 `&buf[valid_up_to..]` 仍然是非法 UTF-8，会再次报错并
/// 被 `unwrap_or("")` 吞成空串（这是本函数第一版的真实 bug：2 字节字符 + max=5
/// 时切点恰好落在续字节上，结果整个尾部变成空）。
///
/// 用 `while` 而不是 `if`：理论上切点只制造**一个**不完整字符，但 `error_len`
/// 为 `None`（截断在序列中间）时长度未知，循环能保证无论如何都收敛——要么找到
/// 合法边界，要么走到末尾返回空串，绝不 panic 也绝不返回半个字符。
fn decode_after_boundary(buf: &[u8]) -> String {
    let mut rest = buf;
    loop {
        match std::str::from_utf8(rest) {
            Ok(text) => return text.trim().to_string(),
            Err(err) => {
                let skip = err.valid_up_to() + err.error_len().unwrap_or(rest.len());
                // 防御：`error_len == Some(0)` 会让 rest 不变 → 死循环。
                let skip = skip.max(err.valid_up_to() + 1).min(rest.len());
                rest = &rest[skip..];
                if rest.is_empty() {
                    return String::new();
                }
            }
        }
    }
}

/// 解析记录文件内容（缺字段 / 非法数字 → None）。
///
/// `last_exit` 是可选增强：缺失或结构非法时**降级为 `None`**，绝不让一份损坏的
/// 证据把整条记录（pid/port，复用路径的唯一依据）一起拖垮。
pub fn parse_record(text: &str) -> Option<SidecarRecord> {
    let v: serde_json::Value = serde_json::from_str(text).ok()?;
    let pid = v.get("pid")?.as_u64()?;
    let port = v.get("port")?.as_u64()?;
    if pid == 0 || pid > u32::MAX as u64 || port == 0 || port > u16::MAX as u64 {
        return None;
    }
    let last_exit = v
        .get("last_exit")
        .filter(|e| e.is_object())
        .and_then(|e| serde_json::from_value::<HostExitRecord>(e.clone()).ok());
    Some(SidecarRecord {
        pid: pid as u32,
        port: port as u16,
        last_exit,
    })
}

/// 读记录文件（不存在 / 损坏 → None）。
pub fn load_record() -> Option<SidecarRecord> {
    std::fs::read_to_string(record_path())
        .ok()
        .and_then(|t| parse_record(&t))
}

/// 写记录文件（best-effort：失败只影响下次复用，不影响本次运行）。
///
/// 刻意**不清空**已有的 `last_exit`：一次健康启动不意味着「上一次崩溃的原因不再重要」，
/// 恰恰相反——最需要它的场景就是「网关自己也没了、重开后用户来问上次为什么坏」。
/// 新证据只在真正又崩一次时覆盖（同 `spawn_once` 用 `File::create` 覆盖日志的语义）。
pub fn save_record(pid: u32, port: u16) {
    let last_exit = load_record().and_then(|r| r.last_exit);
    write_record(&SidecarRecord {
        pid,
        port,
        last_exit,
    });
}

/// 把一条退出证据写回记录文件（best-effort，失败只影响跨重启可读性）。
///
/// `pid_hint` 用于「记录文件还不存在」的场景：侧车**启动即退出**时 `start()` 从
/// 走不到 `save_record`，若此时不肯新建记录，证据就只剩内存里那份——而这恰恰是
/// 最需要跨重启的场景（下次启动 UI 断连时用户问「上次为什么坏」）。早退子进程的
/// pid 仍可从 `Child::id()` 取到（进程死了句柄还在），足以让记录合法。
///
/// 读-改-写以保留既有 pid/port：调用方手上只有证据，不该为写一条日志去猜 pid。
pub fn save_last_exit(pid_hint: Option<u32>, port: u16, exit: &HostExitEvidence) {
    let base = match load_record() {
        Some(rec) => SidecarRecord {
            // 已有记录时以既有 pid/port 为准（它们对应「仍在位/已复用」的那个进程）；
            // 记录不存在时才用调用方给的参数新建。
            pid: rec.pid,
            port: rec.port,
            last_exit: None,
        },
        None => {
            let pid = match pid_hint {
                Some(pid) if pid != 0 => pid,
                // 没有记录可挂靠也没有可用 pid：证据仍留在内存槽里，只是跨重启不可读
                // ——可接受的降级，不是错误。
                _ => return,
            };
            // port 必须合法：`parse_record` 把 0 视为无效并整条丢弃，写 0 等于白写。
            if port == 0 {
                return;
            }
            SidecarRecord {
                pid,
                port,
                last_exit: None,
            }
        }
    };
    write_record(&SidecarRecord {
        last_exit: Some(HostExitRecord {
            code: exit.code,
            uptime_ms: exit.uptime_ms,
            at_ms: now_ms(),
            stderr_tail: exit.stderr_tail.clone(),
            observed: exit.observed,
        }),
        ..base
    });
}

/// 记录文件的实际写盘（抽出来供 `save_record` / `save_last_exit` 共用）。
fn write_record(rec: &SidecarRecord) {
    let payload = serde_json::json!({
        "pid": rec.pid,
        "port": rec.port,
        "last_exit": rec.last_exit,
    });
    if let Some(dir) = record_path().parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(record_path(), payload.to_string());
}

/// 清记录文件（侧车已确认死亡时调用）。
pub fn clear_record() {
    let _ = std::fs::remove_file(record_path());
}

/// 从 Windows `netstat -ano` 输出里找该 pid 的**所有** LISTENING 端口。
///
/// 为什么要「所有」：实测同一个 Python 侧车进程会同时监听多个端口
/// （7 万级随机高位端口之外还有内部端口，例如 32484 同时听 54069 与 61750）。
/// 只取第一个会挑到非网关端口，`/health` 探活必失败 → 复用路径静默失效。
pub fn parse_windows_netstat_listen_ports(text: &str, pid: u32) -> Vec<u16> {
    let mut ports = Vec::new();
    for line in text.lines() {
        let cols: Vec<&str> = line.split_whitespace().collect();
        // 形如：TCP  127.0.0.1:61750  0.0.0.0:0  LISTENING  32484
        if cols.len() < 5 || !cols[3].eq_ignore_ascii_case("LISTENING") {
            continue;
        }
        if cols[4].parse::<u32>().ok() != Some(pid) {
            continue;
        }
        if let Some(port) = port_after_last_colon(cols[1]) {
            if !ports.contains(&port) {
                ports.push(port);
            }
        }
    }
    ports
}

/// 从 `lsof -nP -a -p <pid> -iTCP -sTCP:LISTEN` 输出里取全部 LISTEN 端口。
pub fn parse_unix_lsof_listen_ports(text: &str) -> Vec<u16> {
    let mut ports = Vec::new();
    for line in text.lines() {
        if !line.to_ascii_uppercase().contains("LISTEN") {
            continue;
        }
        for col in line.split_whitespace() {
            if let Some(port) = port_after_last_colon(col) {
                if !ports.contains(&port) {
                    ports.push(port);
                }
                break;
            }
        }
    }
    ports
}

/// 从 `ss -ltnp` 输出里找该 pid 的全部 LISTEN 端口。
pub fn parse_unix_ss_listen_ports(text: &str, pid: u32) -> Vec<u16> {
    let needle = format!("pid={pid}");
    let mut ports = Vec::new();
    for line in text.lines() {
        if !line.contains(&needle) {
            continue;
        }
        for col in line.split_whitespace() {
            if let Some(port) = port_after_last_colon(col) {
                if !ports.contains(&port) {
                    ports.push(port);
                }
                break;
            }
        }
    }
    ports
}

/// `127.0.0.1:61750` / `[::1]:61750` / `*:61750` → `61750`。
fn port_after_last_colon(raw: &str) -> Option<u16> {
    let (_, port) = raw.rsplit_once(':')?;
    let port = port.trim_end_matches(|c: char| !c.is_ascii_digit());
    port.parse::<u16>().ok().filter(|p| *p != 0)
}

/// 读 Python 侧 pidfile（`<py_home>/default/state/gateway.pid`）里的 pid。
pub fn read_python_gateway_pid(py_home: &Path) -> Option<u32> {
    let path = py_home
        .join("default")
        .join("state")
        .join("gateway.pid");
    let text = std::fs::read_to_string(path).ok()?;
    let v: serde_json::Value = serde_json::from_str(&text).ok()?;
    let pid = v.get("pid")?.as_u64()?;
    u32::try_from(pid).ok().filter(|p| *p != 0)
}

/// 反查某进程的**所有**监听端口（平台分派；探测失败返回空表）。
pub fn listening_ports_of(pid: u32) -> Vec<u16> {
    #[cfg(windows)]
    {
        match std::process::Command::new("netstat").silent().arg("-ano").output() {
            Ok(out) => parse_windows_netstat_listen_ports(&String::from_utf8_lossy(&out.stdout), pid),
            Err(_) => Vec::new(),
        }
    }
    #[cfg(not(windows))]
    {
        if let Ok(out) = std::process::Command::new("lsof")
            .silent()
            .args(["-nP", "-a", "-p", &pid.to_string(), "-iTCP", "-sTCP:LISTEN"])
            .output()
        {
            let ports = parse_unix_lsof_listen_ports(&String::from_utf8_lossy(&out.stdout));
            if !ports.is_empty() {
                return ports;
            }
        }
        match std::process::Command::new("ss").silent().args(["-ltnp"]).output() {
            Ok(out) => parse_unix_ss_listen_ports(&String::from_utf8_lossy(&out.stdout), pid),
            Err(_) => Vec::new(),
        }
    }
}

/// 解析 `tasklist /FI "PID eq <n>" /NH /FO CSV` 的输出，判断该 pid 是否仍在运行。
///
/// 纯函数（不做进程调用），便于单测覆盖「找得到 / 找不到 / 名字里带数字」三种形态。
///
/// 必须**按 CSV 字段精确比对**而不是 `contains(&pid.to_string())`：`tasklist` 的
/// 内存列（`"35,000 K"`）与镜像名都可能含相同数字串，子串匹配会误判成「还活着」，
/// 于是永远等不到退出证据 —— 正是本缺陷的另一种表现。
pub fn parse_tasklist_has_pid(text: &str, pid: u32) -> bool {
    let target = pid.to_string();
    text.lines()
        // 数据行形如 `"python.exe","11908","Console","1","35,000 K"`；
        // 未命中时 tasklist 输出一行 `INFO: No tasks are running ...`（不以 `"` 开头）。
        .map(str::trim)
        .filter(|line| line.starts_with('"') && line.ends_with('"'))
        .any(|line| line.trim_matches('"').split("\",\"").any(|f| f == target))
}

/// 进程是否仍在运行（跨平台判活）。
///
/// **为什么复用路径需要它**：见 [`PyEngineSidecar::attach`] —— 附着的进程不是本进程
/// 的子进程，没有 [`Child`] 句柄，`try_wait()` 无从谈起，只能按 pid 问系统。
///
/// 代价：Windows 上每问一次要起一个 `tasklist` 子进程（这是本函数只在**复用**路径
/// 被低频调用、而主路径坚持用 `try_wait()` 的原因）。
pub fn pid_is_alive(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    #[cfg(windows)]
    {
        match std::process::Command::new("tasklist")
            .silent()
            .args(["/FI", &format!("PID eq {pid}"), "/NH", "/FO", "CSV"])
            .output()
        {
            Ok(out) => {
                let text = String::from_utf8_lossy(&out.stdout).into_owned();
                parse_tasklist_has_pid(&text, pid)
            }
            // 查不动 ≠ 死了：判活失败必须保守地当作「还活着」，否则会产出一条
            // 「进程已死」的错误证据，把用户引向错误结论。
            Err(_) => true,
        }
    }
    #[cfg(not(windows))]
    {
        // `kill -0 <pid>`：存在且有权限 → 0；不存在(ESRCH) → 非 0；存在但无权限(EPERM)
        // → 非 0 但 stderr 含 "Operation not permitted"，此时进程**确实存在**。
        match std::process::Command::new("kill")
            .silent()
            .arg("-0")
            .arg(pid.to_string())
            .output()
        {
            Ok(out) => {
                out.status.success()
                    || String::from_utf8_lossy(&out.stderr).contains("not permitted")
            }
            Err(_) => true,
        }
    }
}

/// 一个「可能存在、值得尝试握手」的侧车候选（S5：复用路径需要 pid 才能判活）。
///
/// `pid` 来自记录文件或 Python 侧 pidfile，都属于**间接**来源，可能为 `None`
/// （如候选来自 `netstat` 反查）。拿不到 pid 时复用仍可进行，只是无法启用退出
/// 证据守护——如实降级，不猜。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SidecarCandidate {
    pub pid: Option<u32>,
    pub port: u16,
}

/// 列出**可能存在一个已在运行的侧车**的候选（记录文件优先，其次 pidfile 反查）。
///
/// 注意：这里刻意**不做健康判定**。`check_health` 把 404 也算「活着」（对通用探活是对的），
/// 但同一个侧车进程还会监听别的内部端口（实测 32484 同时听 54069 与 61750，
/// 其中 54069 对 `/health` 回 404）—— 用探活区分会挑错端口。调用方必须用
/// **真实的网关 WS 握手**来确认候选端口。
pub fn candidate_sidecars(cfg: &SidecarConfig) -> Vec<SidecarCandidate> {
    let mut candidates: Vec<SidecarCandidate> = Vec::new();
    if let Some(rec) = load_record() {
        candidates.push(SidecarCandidate {
            pid: Some(rec.pid),
            port: rec.port,
        });
    }
    if let Some(pid) = read_python_gateway_pid(&cfg.py_home) {
        for port in listening_ports_of(pid) {
            if !candidates.iter().any(|c| c.port == port) {
                candidates.push(SidecarCandidate {
                    pid: Some(pid),
                    port,
                });
            }
        }
    }
    candidates
}

/// 进程级「最近一次退出证据」槽（S5）。
///
/// 为什么是进程级而不是只在 `PyEngineSidecar` 实例上：最需要证据的场景恰恰是
/// **侧车启动即退出**（`OPENSQUILLA_PROFILE_IN_USE` 之类）——那时 `start()` 直接
/// 返回 `Err`，`PyEngineSidecar` 从未被构造、更没进 `PY_SIDECAR_REG`，挂在实例上
/// 的证据会随实例一起消失，读证据的 RPC 反而拿不到任何东西。
///
/// 用 `std::sync::Mutex` 而非 `tokio::sync::Mutex`：读方是**同步** RPC handler
/// （`dispatch_rust`），不能 await。临界区只有一次指针级别的 clone，不跨 await。
fn global_exit_slot() -> &'static Arc<std::sync::Mutex<Option<HostExitEvidence>>> {
    static SLOT: std::sync::OnceLock<Arc<std::sync::Mutex<Option<HostExitEvidence>>>> =
        std::sync::OnceLock::new();
    SLOT.get_or_init(|| Arc::new(std::sync::Mutex::new(None)))
}

/// 记下一次退出证据：写内存槽 + 尝试落盘（S5）。
///
/// `pid_hint` / `port` 供 [`save_last_exit`] 在记录文件缺失时新建一条——见该函数
/// 注释。`None` / `0` 是合法的「不知道」，落盘会跳过而不会写坏记录。
///
/// 落盘失败只 warn 不报错：证据的价值在于「有比没有好」，跨重启可读性是增强项，
/// 不该让记录动作本身成为新的失败源。
pub fn record_exit_evidence(pid_hint: Option<u32>, port: u16, exit: HostExitEvidence) {
    // stderr_tail 单独 warn 长度即可，绝不打印内容（可能含路径 / 用户名）。
    let tail_len = exit.stderr_tail.len();
    info!(
        code = ?exit.code,
        uptime_ms = exit.uptime_ms,
        stderr_tail_bytes = tail_len,
        "记录 Python 侧车退出证据"
    );
    match global_exit_slot().lock() {
        Ok(mut slot) => *slot = Some(exit.clone()),
        // 毒化的锁意味着别处 panic 过；证据是尽力而为的诊断信息，不值得连带失败。
        Err(poisoned) => *poisoned.into_inner() = Some(exit.clone()),
    }
    save_last_exit(pid_hint, port, &exit);
}

/// 读最近一次退出证据（从未退出过 → `None`）。
pub fn last_exit_evidence() -> Option<HostExitEvidence> {
    match global_exit_slot().lock() {
        Ok(slot) => slot.clone(),
        Err(poisoned) => poisoned.into_inner().clone(),
    }
}

/// 清空内存里的退出证据槽（**仅测试用**）。
///
/// 内存槽是进程级的，同一测试进程内某个用例真实记录过证据后，
/// 后续用例会读到它。需要断言「从未退出过」的用例必须先把槽清空，
/// 否则会变成「依赖同进程其它用例是否先跑」的顺序相关假红。
#[cfg(test)]
pub fn clear_last_exit_evidence() {
    match global_exit_slot().lock() {
        Ok(mut slot) => *slot = None,
        Err(poisoned) => *poisoned.into_inner() = None,
    }
}

/// Python 侧车子进程句柄 + 守护任务
pub struct PyEngineSidecar {
    pub config: SidecarConfig,
    child: Arc<Mutex<Option<Child>>>,
    /// P1-5：当前侧车进程的内存围栏（Job Object / RLIMIT 归属）。每次
    /// `spawn_once` 成功都会替换为新进程的围栏；旧围栏 drop 即关闭旧 Job。
    fence: Arc<Mutex<Option<crate::pyengine::memfence::MemoryFence>>>,
    status: Arc<std::sync::atomic::AtomicU8>,
    /// 当前子进程的 spawn 时刻；每次 `spawn_once` 成功都会重置（S5：算存活时长）。
    started_at: Arc<std::sync::Mutex<Instant>>,
    /// 最近一次退出的结构化证据（S5）。与 [`global_exit_slot`] 是**同一个**
    /// `Arc`，所以无论证据由哪一代子进程（甚至一次失败的 `start`）写下，
    /// 持有本实例的 RPC 与 [`last_exit_evidence`] 读到的都是同一份。
    last_exit: Arc<std::sync::Mutex<Option<HostExitEvidence>>>,
    _guard: Option<tokio::task::JoinHandle<()>>,
}

const STATUS_STOPPED: u8 = 0;
const STATUS_STARTING: u8 = 1;
const STATUS_RUNNING: u8 = 2;
const STATUS_RESTARTING: u8 = 3;
const STATUS_FAILED: u8 = 4;
/// P0-1：崩溃循环熔断后停止自动重启（人工 sidecar_start 会构造全新实例，窗口随之重置）。
const STATUS_DEGRADED: u8 = 5;

impl PyEngineSidecar {
    pub fn status(&self) -> PyEngineStatus {
        match self.status.load(std::sync::atomic::Ordering::Acquire) {
            STATUS_STOPPED => PyEngineStatus::Stopped,
            STATUS_STARTING => PyEngineStatus::Starting,
            STATUS_RUNNING => PyEngineStatus::Running,
            STATUS_RESTARTING => PyEngineStatus::Restarting,
            STATUS_DEGRADED => PyEngineStatus::Degraded,
            _ => PyEngineStatus::Failed,
        }
    }

    /// 附着到一个**已在运行**的侧车（复用路径）：不 spawn，但**照样起守护任务**。
    ///
    /// 为什么不复用 `start`：`start` 一定会 spawn 新进程，而 Python 侧车对同一
    /// profile 是单实例（第二次启动会以 `OPENSQUILLA_PROFILE_IN_USE` 秒退）。
    ///
    /// # S5 缺陷修复：复用路径原本**根本没有守护任务**（`_guard: None`）
    ///
    /// 旧实现让复用路径完全裸奔——没有轮询、没有 `try_wait()`、没有退出证据。
    /// 实机复现：网关复用孤儿侧车后 `taskkill` 掉它，`host.exit_evidence` 永远
    /// 返回 `exited:false` / `evidence:null`，日志里 `记录 Python 侧车退出证据`
    /// 一次都没出现（`grep -c` = 0），侧车也不会重启。
    ///
    /// 复用恰恰是**最需要**这份证据的路径：用户看到的现象就是「上次好好的，这次
    /// 侧车没了」，而复用路径下崩溃的侧车根本没人负责记录死因。
    ///
    /// 与主路径的差别：附着的进程**不是本进程的子进程**，没有 [`Child`]
    /// 句柄，因此初始判活只能按 pid 问系统（[`pid_is_alive`]），拿不到 `ExitStatus`
    /// → 取证 `code` 恒为 `None`（这是事实，不是缺失）。存活时长从**附着时刻**起算。
    ///
    /// # P1-7：复用路径失活后自动接管
    ///
    /// 附着的外部侧车一旦失活，统一监督循环先取证（code=None），然后与主路径
    /// **同一策略** `spawn_once` 接管拉起（见 [`run_sidecar_supervisor`] /
    /// [`InitialTarget::Attached`]），解决「孤儿侧车被杀→只取证不重启→控制台永久
    /// 降级」；侧车崩溃不拖死网关。
    pub fn attach(cfg: SidecarConfig, pid: Option<u32>) -> Self {
        let started_at = Arc::new(std::sync::Mutex::new(Instant::now()));
        let child = Arc::new(Mutex::new(None));
        let fence = Arc::new(Mutex::new(None));
        let status = Arc::new(std::sync::atomic::AtomicU8::new(STATUS_RUNNING));
        // P1-7：复用路径也跑统一监督循环。附着 pid 失活时先取证，再自动
        // `spawn_once` 接管（InitialTarget::Attached）。
        let guard = pid.map(|attached_pid| {
            tokio::spawn(run_sidecar_supervisor(
                cfg.clone(),
                cfg.py_home.clone(),
                child.clone(),
                fence.clone(),
                status.clone(),
                started_at.clone(),
                InitialTarget::Attached(attached_pid),
            ))
        });
        Self {
            config: cfg,
            child,
            fence,
            status,
            // 复用路径初始无自有子进程，spawn 起点用当前时刻，保证 uptime 非负。
            started_at,
            last_exit: global_exit_slot().clone(),
            _guard: guard,
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
        let fence = Arc::new(Mutex::new(None));
        let status = Arc::new(std::sync::atomic::AtomicU8::new(STATUS_STARTING));
        let started_at = Arc::new(std::sync::Mutex::new(Instant::now()));

        // 首次 spawn。返回的 pid 用于在「启动即退出」时仍能写出一条合法记录
        // （此时 `Child::id()` 已被 tokio 融合成 None，见 `spawn_once` 注释）。
        let spawned_pid = spawn_once(&cfg, &eff_home, &child, &started_at, &fence).await?;
        // 启动即退出（最典型：`OPENSQUILLA_PROFILE_IN_USE` —— 同 profile 已有侧车）
        // 必须**快速报错**：否则 `wait_healthy` 会白等满 90s，调用方只看到「启动失败」，
        // 真正原因（profile 被占用）留在子进程 stdout 里无人查看。
        //
        // S5：此处已经拿到了 `ExitStatus`（旧代码写作 `_status` 直接丢弃），现在把它
        // 连同存活时长与 stderr 尾部一起变成结构化证据：既进返回错误，也进跨重启的
        // 证据槽——启动失败后 `PyEngineSidecar` 不会被构造，实例字段无处可挂，
        // 只有进程级槽能保证 UI 后来仍读得到死因。
        if let Some(evidence) = early_exit_reason(&cfg, &child, &started_at, Duration::from_secs(12)).await {
            // 用 spawn 时就取到的 pid（而非此刻的 `Child::id()`——后者已被 tokio
            // 融合成 None），这样记录文件缺失时也能新建一条，证据得以跨重启可读。
            record_exit_evidence(spawned_pid, cfg.port, evidence.clone());
            return Err(anyhow!(
                "Python 侧车启动即退出（port={}, exit_code={:?}, uptime={}ms）：{}",
                cfg.port,
                evidence.code,
                evidence.uptime_ms,
                // stderr 尾部**只进错误链**（用户可见的启动失败详情），不进 i18n 文案：
                // 它可能含路径 / URL。这与 error_taxonomy 的既有纪律一致。
                if evidence.stderr_tail.is_empty() {
                    "子进程退出且未留下任何 stderr 输出".to_string()
                } else {
                    evidence.stderr_tail.clone()
                }
            ));
        }
        wait_healthy(&cfg, cfg.health_timeout)
            .await
            .map_err(|e| anyhow!("Python 侧车启动后健康探测失败（port={}）: {e:#}", cfg.port))?;
        status.store(STATUS_RUNNING, std::sync::atomic::Ordering::Release);
        // 记下 pid/port，供**下次启动**复用（GUI 被强杀时侧车会变孤儿继续占 profile 锁，
        // 复用它比再拉一个更符合 Python 侧的单实例设计）。
        if let Some(pid) = child_pid(&child).await {
            save_record(pid, cfg.port);
        }
        info!(port = cfg.port, "Python 引擎侧车已启动并健康");

        // 守护任务：周期检查进程存活 + HTTP 健康；死亡则退避重启
        // 守护任务：统一监督循环（主路径初始即持有自有子进程）。
        let handle = tokio::spawn(run_sidecar_supervisor(
            cfg.clone(),
            eff_home.clone(),
            child.clone(),
            fence.clone(),
            status.clone(),
            started_at.clone(),
            InitialTarget::Owned,
        ));

        Ok(Self {
            config: cfg,
            child,
            fence,
            status,
            started_at,
            last_exit: global_exit_slot().clone(),
            _guard: Some(handle),
        })
    }

    /// 最近一次退出的结构化证据（从未退出过 → `None`）。
    pub fn last_exit(&self) -> Option<HostExitEvidence> {
        match self.last_exit.lock() {
            Ok(slot) => slot.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        }
    }

    /// 当前子进程已存活的毫秒数（S5）。
    ///
    /// 复用路径（[`Self::attach`]）下没有本进程 spawn 的子进程，这个值只是
    /// 「附着至今」，不代表 Python 侧的真实年龄——UI 只在**已退出**时才展示
    /// `uptime_ms`（取自证据），所以不会误导。
    pub fn uptime_ms(&self) -> u64 {
        current_uptime_ms(&self.started_at)
    }

    /// P1-5（QA）：当前侧车进程是否处于内存围栏（Job Object / RLIMIT）内。
    /// 也用于让 `fence` 字段被实际读取（围栏句柄仅需持有存活，但保留一个
    /// 可对账的自查入口）。
    pub async fn memory_fence_present(&self) -> bool {
        self.fence.lock().await.is_some()
    }

    /// 基础 URL（例如 `http://127.0.0.1:18792`）
    pub fn base_url(&self) -> String {
        format!("http://127.0.0.1:{}", self.config.port)
    }

    /// 主动停止（drop 也会触发）。
    ///
    /// P0-4 三段式：① POST 侧车 graceful shutdown 端点（`/api/system/shutdown`，
    /// 触发 Python `GatewayServer.close()` 排空在飞 turn）；② 等 drain 自然退出
    /// ≤ [`SIDECAR_DRAIN_TIMEOUT`]（对齐 Python deadline）；③ 超时才硬杀。
    /// 旧实现直接 `kill()`——Python 侧精心实现的 drain 在 Windows 桌面上永远
    /// 拿不到执行机会，进行中的 agent turn 被硬杀。
    pub async fn stop(&self) {
        request_sidecar_shutdown(&self.base_url(), &self.config.shared_token).await;
        let mut lock = self.child.lock().await;
        if let Some(mut c) = lock.take() {
            drain_or_kill(&mut c, SIDECAR_DRAIN_TIMEOUT).await;
        }
        self.status
            .store(STATUS_STOPPED, std::sync::atomic::Ordering::Release);
    }
}

impl Drop for PyEngineSidecar {
    fn drop(&mut self) {
        // 最佳努力：同步层面无法 await；守护任务在引用 child 时若发现被 drop 则自行退出
        // 实际停止由 self.stop().await 保证；drop 只做一次后台 fire-and-forget。
        // P0-4：Drop 路径同样优先优雅（短 drain 预算再 kill），不放弃在飞 turn。
        let child = self.child.clone();
        let status = self.status.clone();
        let base = self.base_url();
        let token = self.config.shared_token.clone();
        tokio::spawn(async move {
            request_sidecar_shutdown(&base, &token).await;
            let mut lock = child.lock().await;
            if let Some(mut c) = lock.take() {
                drain_or_kill(&mut c, SIDECAR_DROP_DRAIN_TIMEOUT).await;
            }
            status.store(STATUS_STOPPED, std::sync::atomic::Ordering::Release);
        });
    }
}

// ---------------------------------------------------------------------------
// 内部工具
// ---------------------------------------------------------------------------

/// 侧车日志目录（排障用；默认放系统临时目录）。
///
/// # 为什么留在 `temp_dir()` 而不迁到 `Config::data_dir()`
///
/// S6 要求给日志加**轮转 + 总量封顶**，这一点已由
/// [`crate::observability::rotation`] 落实（10MB/文件、200MB/目录、删最旧）。
/// 目录位置则刻意保持不变，理由是**故障时它必须还能写**：
/// `spawn_once` 的既有注释写明，pyengine-home 被沙箱/杀软拦截时，
/// 日志落盘失败会连带丢失侧车崩溃现场——而「崩溃现场写不进去」正是
/// 最需要它的时刻。迁到 `%LOCALAPPDATA%` 会把这个已知的降级路径
/// 变成常态风险。诊断包导出侧已通过 [`sidecar_log_dir`] 读取同一目录，
/// 因此位置不影响 `--export-diagnostics` 的收集能力。
///
/// # 可配置
///
/// `MARGUERITE_SIDECAR_LOG_DIR` 可覆盖（测试隔离与多实例并存场景），
/// 与 `MARGUERITE_CONFIG_DIR` 同为显式覆盖入口。
pub fn sidecar_log_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("MARGUERITE_SIDECAR_LOG_DIR").filter(|v| !v.is_empty()) {
        return PathBuf::from(dir);
    }
    std::env::temp_dir().join("marguerite-sidecar")
}

/// 侧车子进程的 stdout / stderr 落盘路径（排障用）。
pub fn sidecar_log_paths(port: u16) -> (PathBuf, PathBuf) {
    let log_dir = sidecar_log_dir();
    (
        log_dir.join(format!("sidecar-{port}.out.log")),
        log_dir.join(format!("sidecar-{port}.err.log")),
    )
}

/// 取当前子进程 pid（未 spawn / 已回收 → None）。
pub async fn child_pid(child: &Arc<Mutex<Option<Child>>>) -> Option<u32> {
    child.lock().await.as_ref().and_then(|c| c.id())
}

/// 当前子进程已存活的毫秒数（`started_at` 未初始化时回退 0，绝不 panic）。
fn current_uptime_ms(started_at: &Arc<std::sync::Mutex<Instant>>) -> u64 {
    let Ok(start) = started_at.lock() else {
        return 0;
    };
    start.elapsed().as_millis() as u64
}

/// 若子进程在 `budget` 内自行退出，返回其结构化退出证据（S5）。
///
/// 用于把「启动即退出」从「白等 90s 探活超时」变成一条可执行的错误信息。
///
/// 旧实现已经拿到了 `ExitStatus`（写作 `_status`）却只取 stderr 的 600 字符、
/// 连退出码一起丢掉。现在返回 [`HostExitEvidence`]：退出码 + 存活时长 +
/// stderr 尾部（16KB，按 UTF-8 边界回退）。调用方负责落盘 / 进错误链。
pub async fn early_exit_reason(
    cfg: &SidecarConfig,
    child: &Arc<Mutex<Option<Child>>>,
    started_at: &Arc<std::sync::Mutex<Instant>>,
    budget: Duration,
) -> Option<HostExitEvidence> {
    let deadline = tokio::time::Instant::now() + budget;
    loop {
        let exited = {
            let mut lock = child.lock().await;
            // 子进程句柄缺失（已被 stop/attach 接管）→ 无从判断，不当作「已退出」。
            let c = lock.as_mut()?;
            match c.try_wait() {
                // 拿到了退出状态：这是唯一能给出退出码的时刻（S5）。
                Ok(Some(status)) => Some(status.code()),
                // 还活着：继续观察
                Ok(None) => None,
                // 取不到状态：不把它当「已退出」，交给后续探活判定
                Err(_) => return None,
            }
        };
        if let Some(code) = exited {
            let (_, err_path) = sidecar_log_paths(cfg.port);
            return Some(HostExitEvidence {
                code,
                uptime_ms: current_uptime_ms(started_at),
                stderr_tail: tail_bytes(&err_path, STDERR_TAIL_MAX_BYTES),
                observed: true,
            });
        }
        // 锁已随上面的块释放，此处可以安全 await。
        if tokio::time::Instant::now() >= deadline {
            return None;
        }
        sleep(Duration::from_millis(200)).await;
    }
}

/// 监督循环的初始目标（P1-7：统一主路径与复用路径）。
pub(crate) enum InitialTarget {
    /// `start()`：child 槽已持有本进程 spawn 的子进程。
    Owned,
    /// `attach()`：复用一个已存在的外部侧车（仅知 pid，无 Child 句柄）。
    Attached(u32),
}

/// 统一侧车监督循环（主路径与复用路径共用）。
///
/// - **主路径（[`InitialTarget::Owned`]）**：初始 child 槽已持有自有子进程，按
///   `try_wait()` + HTTP `/ready` 判活。
/// - **复用路径（[`InitialTarget::Attached`]）**：先按 `pid_is_alive` 盯着外部
///   pid；外部进程失活时**先取证（code=None）**，随后与主路径**同一策略**
///   `spawn_once` 接管拉起（此后是自有子进程，回到 try_wait）。这正是 P1-7：
///   解决「孤儿侧车被杀→只取证不重启→控制台永久降级」，且侧车崩溃不拖死网关。
///
/// 其余行为（指数退避、[`classify_restart_loop`] 崩溃循环熔断、[`classify_health`]
/// 连续失手保护、P1-6 RSS 采样）两路径完全一致。
async fn run_sidecar_supervisor(
    cfg: SidecarConfig,
    eff_home: PathBuf,
    child: Arc<Mutex<Option<Child>>>,
    fence: Arc<Mutex<Option<crate::pyengine::memfence::MemoryFence>>>,
    status: Arc<std::sync::atomic::AtomicU8>,
    started_at: Arc<std::sync::Mutex<Instant>>,
    initial: InitialTarget,
) {
    // 复用路径：记录当前附着的外部 pid；主路径为 None。
    let mut attached_pid: Option<u32> = match initial {
        InitialTarget::Attached(p) => Some(p),
        InitialTarget::Owned => None,
    };
    let mut backoff = cfg.restart_backoff_min;
    let mut misses: u32 = 0;
    // 崩溃循环熔断窗口（单调时钟）；人工 sidecar_start 构造全新实例时窗口重置。
    let loop_policy = RestartLoopPolicy::default();
    let mut restart_events: Vec<Instant> = Vec::new();
    let mut fast_exit_events: Vec<Instant> = Vec::new();
    // P1-6：指标采样计数（主循环 tick 2s，每 15 tick = 30s 采一次 RSS）。
    let mut metrics_ticks: u32 = 0;

    loop {
        sleep(Duration::from_secs(2)).await;
        let now = Instant::now();
        restart_events.retain(|t| now.saturating_duration_since(*t) <= loop_policy.window);
        fast_exit_events.retain(|t| now.saturating_duration_since(*t) <= loop_policy.window);

        // 判活 + 取证。
        let alive = if let Some(ap) = attached_pid {
            // 复用路径：外部进程。
            if pid_is_alive(ap) {
                true
            } else {
                // 外部进程已消失：取证（code=None），随后走接管重启。
                let uptime_ms = current_uptime_ms(&started_at);
                let (_, err_path) = sidecar_log_paths(cfg.port);
                record_exit_evidence(
                    Some(ap),
                    cfg.port,
                    HostExitEvidence {
                        code: None,
                        uptime_ms,
                        stderr_tail: tail_bytes(&err_path, STDERR_TAIL_MAX_BYTES),
                        observed: true,
                    },
                );
                warn!(
                    pid = ap,
                    port = cfg.port,
                    uptime_ms,
                    "复用的 Python 侧车已消失，已取证并将自动接管拉起"
                );
                // 清空附着态：本循环重启分支将 spawn_once 接管。
                attached_pid = None;
                false
            }
        } else {
            // 主路径：自有子进程。
            let mut lock = child.lock().await;
            match lock.as_mut() {
                Some(c) => match c.try_wait() {
                    Ok(Some(exit)) => {
                        let uptime_ms = current_uptime_ms(&started_at);
                        let (_, err_path) = sidecar_log_paths(cfg.port);
                        record_exit_evidence(
                            c.id(),
                            cfg.port,
                            HostExitEvidence {
                                code: exit.code(),
                                uptime_ms,
                                stderr_tail: tail_bytes(&err_path, STDERR_TAIL_MAX_BYTES),
                                observed: true,
                            },
                        );
                        if uptime_ms < loop_policy.fast_exit.as_millis() as u64 {
                            fast_exit_events.push(Instant::now());
                        }
                        false
                    }
                    Ok(None) => true,
                    Err(_) => false,
                },
                None => false,
            }
        };

        let http_ok = check_health(&cfg, HEALTH_PROBE_TIMEOUT).await;
        // 进程还活着但一次探活失败 → 长超时复核（防繁忙机器误杀，见 HEALTH_MISS_LIMIT）。
        let reprobe_ok = if alive && !http_ok {
            check_health(&cfg, HEALTH_REPROBE_TIMEOUT).await
        } else {
            false
        };
        let healthy = alive && (http_ok || reprobe_ok);
        misses = if healthy { 0 } else { misses.saturating_add(1) };
        let verdict = classify_health(alive, healthy, misses, HEALTH_MISS_LIMIT);
        if verdict == HealthVerdict::Healthy {
            backoff = cfg.restart_backoff_min;
            if (!restart_events.is_empty() || !fast_exit_events.is_empty())
                && current_uptime_ms(&started_at) >= loop_policy.stable_clear.as_millis() as u64
            {
                restart_events.clear();
                fast_exit_events.clear();
                info!("Python 侧车稳定运行已满阈值，崩溃重启窗口清零");
            }
            // P1-6：周期采样 RSS。
            metrics_ticks = metrics_ticks.saturating_add(1);
            if metrics_ticks >= METRICS_SAMPLE_TICKS {
                metrics_ticks = 0;
                // 复用外部进程也可只读采样；优先自有 child pid，否则用附着 pid。
                let pid_for_metrics = child_pid(&child).await.or(attached_pid);
                if let Some(pid) = pid_for_metrics {
                    let rss = read_process_rss_bytes(pid).unwrap_or(0);
                    record_sidecar_metrics(SidecarMetrics {
                        pid,
                        rss_bytes: rss,
                        sampled_at_ms: now_ms(),
                        uptime_ms: current_uptime_ms(&started_at),
                    });
                }
            }
            continue;
        }
        if verdict == HealthVerdict::Suspect {
            warn!(
                misses,
                limit = HEALTH_MISS_LIMIT,
                "Python 侧车探活失手（进程仍在），继续观察，不重启"
            );
            continue;
        }
        warn!(
            alive,
            http_ok,
            reprobe_ok,
            misses,
            "Python 侧车健康异常，{}ms 后重启",
            backoff.as_millis()
        );
        misses = 0;
        restart_events.push(Instant::now());
        if classify_restart_loop(
            &restart_events,
            &fast_exit_events,
            Instant::now(),
            &loop_policy,
        ) == RestartLoopVerdict::Degraded
        {
            warn!(
                restarts = restart_events.len(),
                fast_exits = fast_exit_events.len(),
                window_secs = loop_policy.window.as_secs(),
                "Python 侧车进入崩溃循环，熔断为 Degraded：停止自动重启，等待人工 sidecar_start"
            );
            status.store(STATUS_DEGRADED, std::sync::atomic::Ordering::Release);
            return;
        }
        status.store(STATUS_RESTARTING, std::sync::atomic::Ordering::Release);
        sleep(backoff).await;
        // kill 旧进程（复用接管时 child 为 None，no-op）。
        let mut lock = child.lock().await;
        if let Some(mut c) = lock.take() {
            let _ = c.kill().await;
        }
        drop(lock);
        let spawned = spawn_once(&cfg, &eff_home, &child, &started_at, &fence).await;
        if let Err(e) = spawned {
            error!(err = %e, "Python 侧车重启失败");
            status.store(STATUS_FAILED, std::sync::atomic::Ordering::Release);
        } else if wait_healthy(&cfg, cfg.health_timeout).await.is_ok() {
            status.store(STATUS_RUNNING, std::sync::atomic::Ordering::Release);
            info!(port = cfg.port, "Python 侧车重启成功");
        } else {
            error!("Python 侧车重启后健康探测超时");
            status.store(STATUS_FAILED, std::sync::atomic::Ordering::Release);
        }
        backoff = (backoff * 2).min(cfg.restart_backoff_max);
    }
}

/// 启动前的能力预检：把「跑的是哪个解释器、缺哪些包」写成**一行可 grep** 的日志。
///
/// 为什么必须有（真机审计 2026-10-29 P1-2）：安装版侧车自己起得来，但 bundled Python
/// 环境缺 `numpy`，于是 `squilla_router` 初始化失败 → **本地 ML 路由被静默禁用**、
/// 回退 default-tier，日志里刷 22 条 `No module named 'numpy'` 而用户毫无感知。
///
/// 为什么日志第一步就够：本仓 pyengine 有 **3 份副本**（`pyengine/src` 源码、
/// `gui/src-tauri/resources/pyengine` 打包源、`target/*/resources/pyengine` **实际加载**）。
/// `detect_pyengine_src()` 选哪份、`detect_python()` 选中哪个解释器，此前**完全不可见** ——
/// 不先把这两个事实打出来，修依赖必然是「改 A、验 A、跑的是 B」。
///
/// 行为约束：
/// - 只做 `importlib.util.find_spec`，**不装包、不改环境**；
/// - 5s 硬超时，超时只 `warn!`，**绝不阻塞侧车启动**；
/// - 缺包只降级（warn + 明说会影响什么），不影响 spawn。
async fn preflight_python_dependencies(cfg: &SidecarConfig) {
    const PROBE_TIMEOUT: Duration = Duration::from_secs(5);
    // 顺序即"缺了会怎样"的重要性：numpy 掉的是本地 ML 路由，其余掉的是网关本体能力。
    const MODULES: [&str; 4] = ["numpy", "httpx", "structlog", "anyio"];
    let script = format!(
        "import sys, importlib.util as u\n\
         print('PY=' + sys.executable)\n\
         for m in {modules:?}:\n\
         \x20   try:\n\
         \x20       print(('FOUND:' if u.find_spec(m) else 'MISSING:') + m)\n\
         \x20   except Exception:\n\
         \x20       print('MISSING:' + m)\n",
        modules = MODULES
    );
    let mut cmd = Command::new(&cfg.python_exe);
    cmd.arg("-c").arg(script);
    // 与 spawn 同口径设置 PYTHONPATH：预检必须探到**侧车真正会看到的**模块集合，
    // 否则会出现「预检说有、启动说没有」的第二种假象。
    cmd.env("PYTHONPATH", &cfg.pyengine_src);
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());

    let output = match timeout(PROBE_TIMEOUT, cmd.output()).await {
        Ok(Ok(o)) => o,
        Ok(Err(e)) => {
            warn!(
                python = %cfg.python_exe.display(),
                err = %e,
                "侧车依赖预检无法执行（解释器可能不可用）；继续尝试启动，失败原因见后续日志"
            );
            return;
        }
        Err(_) => {
            warn!(
                python = %cfg.python_exe.display(),
                timeout_secs = PROBE_TIMEOUT.as_secs(),
                "侧车依赖预检超时；不阻塞启动，但下面的依赖结论缺失"
            );
            return;
        }
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut detected = String::new();
    let mut missing: Vec<&str> = Vec::new();
    for line in stdout.lines() {
        if let Some(p) = line.strip_prefix("PY=") {
            detected = p.trim().to_string();
        } else if let Some(m) = line.strip_prefix("MISSING:") {
            missing.push(MODULES.iter().find(|x| **x == m.trim()).copied().unwrap_or("?"));
        }
    }
    if detected.is_empty() {
        // exit != 0 且没有 PY 行：解释器连 `-c` 都跑不起来（版本不对/被拦截）。
        warn!(
            python = %cfg.python_exe.display(),
            status = ?output.status.code(),
            stderr = %String::from_utf8_lossy(&output.stderr).lines().next().unwrap_or(""),
            "侧车依赖预检没拿到解释器路径：侧车大概率起不来"
        );
        return;
    }

    info!(
        python = %detected,
        python_cfg = %cfg.python_exe.display(),
        pyengine_src = %cfg.pyengine_src.display(),
        "侧车运行时事实（排障请对照这三行：改源 ≠ 改运行）"
    );
    // 落一份可读状态给 `system.health`：日志只有排障的人会看，界面上必须有。
    set_preflight_report(PreflightReport {
        python: detected.clone(),
        pyengine_src: cfg.pyengine_src.display().to_string(),
        missing: missing.iter().map(|m| (*m).to_string()).collect(),
    });
    if missing.is_empty() {
        info!(modules = ?MODULES, "侧车依赖预检通过");
        return;
    }
    for m in &missing {
        match *m {
            "numpy" => warn!(
                module = "numpy",
                python = %detected,
                "侧车缺 numpy：本地 ML 路由（V4 Phase 3）会被禁用并回退 default-tier；聊天可用但「本地智能路由」静默失效"
            ),
            other => warn!(
                module = other,
                python = %detected,
                "侧车缺依赖：对应的网关能力将回落或不可用"
            ),
        }
    }
}

async fn spawn_once(
    cfg: &SidecarConfig,
    eff_home: &Path,
    child_slot: &Arc<Mutex<Option<Child>>>,
    started_at_slot: &Arc<std::sync::Mutex<Instant>>,
    fence_slot: &Arc<Mutex<Option<crate::pyengine::memfence::MemoryFence>>>,
) -> Result<Option<u32>> {
    preflight_python_dependencies(cfg).await;
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
    //
    // S6：改用轮转写入（10MB/文件、200MB/目录、删最旧），见
    // `observability::rotation`。此前这里是裸 `File::create`——既不轮转也
    // 无上限，长跑 + 反复崩溃会让该目录无限增长，而它又在
    // `git ls-files` 视野之外，`check-secrets.py` 扫不到。
    //
    // ⚠️ 与 S5 的时序契约：证据仍必须在**重启之前**采集（见守护任务里的
    // 注释）。轮转不破坏该契约——它只在 spawn 时刻切文件，且切走的是
    // 「上一代」的日志（保留为 `.1`），本次运行的证据仍完整存在于原路径。
    // 反过来，轮转让「上一代现场被`File::create` 直接抹掉」这个老问题
    // 一并消失：崩溃现场现在跨重启可读。
    let log_dir = sidecar_log_dir();
    let _ = std::fs::create_dir_all(&log_dir);
    let (out_path, err_path) = sidecar_log_paths(cfg.port);
    let out_file = crate::observability::rotation::open_rotating(&out_path)
        .with_context(|| format!("打开侧车 stdout 日志失败: {}", out_path.display()))?;
    let err_file = crate::observability::rotation::open_rotating(&err_path)
        .with_context(|| format!("打开侧车 stderr 日志失败: {}", err_path.display()))?;
    cmd.stdout(Stdio::from(out_file))
        .stderr(Stdio::from(err_file))
        .kill_on_drop(true);
    // P1-5：内存围栏。
    let limit_mb = crate::pyengine::memfence::limit_mb_from_env();
    let limit_bytes = limit_mb.map(crate::pyengine::memfence::mb_to_bytes);
    // Linux: spawn 前在 pre_exec 内 setrlimit(RLIMIT_AS)。
    if let Some(bytes) = limit_bytes {
        crate::pyengine::memfence::prepare_command(&mut cmd, bytes)
            .context("套用侧车内存围栏(RLIMIT)失败")?;
    }
    // Windows: 静默后台运行，不弹出 Python 控制台窗口
    #[cfg(windows)]
    {
        cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }
    let spawned = cmd
        .spawn()
        .with_context(|| format!("spawn Python 失败: {}", cfg.python_exe.display()))?;
    // Windows: spawn 后把进程放进 Job Object 并设 JobMemoryLimit。
    #[cfg(windows)]
    {
        // tokio 1.53 的 raw_handle 返回 Option<RawHandle>（*mut c_void）。
        let raw_handle = spawned.raw_handle().map(|h| h as isize);
        let new_fence = match (limit_bytes, raw_handle) {
            (Some(bytes), Some(h)) => Some(
                crate::pyengine::memfence::fence_after_spawn(h, bytes)
                    .with_context(|| "侧车 Job Object 内存围栏套用失败")?,
            ),
            (Some(_), None) => {
                return Err(anyhow!("侧车刚 spawn 即融合，拿不到进程句柄套用围栏"));
            }
            (None, _) => None,
        };
        // 替换为新围栏（旧围栏 drop → 关闭旧 Job）。
        *fence_slot.lock().await = new_fence;
        if let Some(mb) = limit_mb {
            info!(limit_mb = mb, "Python 侧车已套用 Job Object 内存围栏");
        }
    }
    #[cfg(not(windows))]
    {
        if limit_bytes.is_some() {
            *fence_slot.lock().await = Some(crate::pyengine::memfence::MemoryFence);
        } else {
            *fence_slot.lock().await = None;
        }
    }
    // S5：pid 必须在**此刻**取。tokio 的 `Child::id()` 在 `try_wait()` 回收之后
    // 会转为 `FusedChild::Done` 并返回 `None`（见 tokio process/mod.rs）——
    // 也就是说等我们发现进程退出时再问 pid，永远只能拿到 None，早退证据就
    // 永远写不进 `<data_dir>/sidecar.json`。这正是本次实机验证暴露的问题。
    let pid = spawned.id();
    *child_slot.lock().await = Some(spawned);
    // S5：spawn 成功的**这一刻**才是存活时长的起点。放在 `child_slot` 写入之后，
    // 保证「已记账的时刻」不早于「进程真的存在」——否则退出时会算出一段
    // 包含 spawn 耗时的虚假 uptime。
    if let Ok(mut start) = started_at_slot.lock() {
        *start = Instant::now();
    }
    info!(
        port = cfg.port,
        pid,
        python = %cfg.python_exe.display(),
        out = %out_path.display(),
        err = %err_path.display(),
        "Python 侧车子进程已 spawn"
    );
    Ok(pid)
}

/// 侧车判死所需**连续**失手次数。单次失手（尤其「进程还活着但探活超时」）不足以判死 ——
/// 繁忙机器上把活着的引擎杀掉，代价是任务中断 + 引擎答案在 UI 上翻转（实测）。
const HEALTH_MISS_LIMIT: u32 = 3;
/// 常规探活超时（每 2s 一轮）。
const HEALTH_PROBE_TIMEOUT: Duration = Duration::from_secs(3);
/// 进程仍在时的复核超时（更宽，用来区分「忙」与「真死」）。
const HEALTH_REPROBE_TIMEOUT: Duration = Duration::from_secs(10);

/// 健康判定（纯函数，便于守卫）：由「是否存活 + 本轮是否健康 + 连续失手数」决定下一步。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HealthVerdict {
    /// 本轮健康（连续失手清零）
    Healthy,
    /// 进程仍在但连续失手未达上限：继续观察，**不重启**
    Suspect,
    /// 进程已退出，或连续失手达上限：退避重启
    Dead,
}

/// HTTP 状态码是否算「引擎活着」：任何 **< 500** 的回答都算（429 限流、4xx 都证明服务在响应）；
/// 只有 5xx（应用自己坏了）与传输失败才算不健康。
pub(crate) fn health_status_is_alive(status: u16) -> bool {
    status < 500
}

pub(crate) fn classify_health(
    alive: bool,
    healthy: bool,
    misses: u32,
    limit: u32,
) -> HealthVerdict {
    if alive && healthy {
        return HealthVerdict::Healthy;
    }
    if !alive {
        // 进程真的没了（退出码已知）→ 立即重启，不必等满失手上限
        return HealthVerdict::Dead;
    }
    if misses >= limit {
        HealthVerdict::Dead
    } else {
        HealthVerdict::Suspect
    }
}

async fn check_health(cfg: &SidecarConfig, wait: Duration) -> bool {
    // P0-2：探活从静态 /health（写死 200，只证明 TCP 在）改为 /ready
    // （Python 侧把"事件循环是否真的在转"自证进去：loop tick 停滞 >15s → 503）。
    // /health 与 /healthz 仍是纯存活探针（app.py: Route("/health", health)），
    // 非 /api/health（那是 Rust 网关自己的端点）。
    let url = format!("http://127.0.0.1:{}{}", cfg.port, ready_probe_path());
    timeout(wait, async {
        match reqwest::Client::builder()
            // 客户端超时必须与调用方给的上限一致：固定 2s 会让「长超时复核」
            // 形同虚设（外层给 10s，内层 2s 先断，永远区分不出「忙」）。
            .timeout(wait)
            // 回环健康探测绝不能被系统代理接管：代理会替回环目标回 502（本机实测
            // `ProxyServer=127.0.0.1:7890` 的 clash），于是「侧车已经起来了」被读成
            // 「健康检查失败」→ 整台机器静默降级成纯 Rust 模式（能力子集）。
            .no_proxy()
            .build()
        {
            Ok(c) => match c.get(&url).send().await {
                // 「有回答」本身即存活证据：侧车限流曾对 /health 回 **429**（实测 21/90 次），
                // 旧判定 `is_success()` 会把限流读成「引擎已死」→ 杀掉健康引擎。
                // 只有 5xx（应用真的坏了）与传输失败才算不健康。
                Ok(resp) => health_status_is_alive(resp.status().as_u16()),
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

/// P0-2：看门狗探活路径。用 `/ready`（事件循环新鲜度自证）而非静态 `/health`。
///
/// 抽成常量函数而非内联字符串：纯函数守卫（[`health_verdict_tests`] 内）钉住
/// "不得回退成静态 /health"——事故复盘里最常见的回退就是换探针时把 URL 改回去。
pub(crate) fn ready_probe_path() -> &'static str {
    "/ready"
}

// ---------------------------------------------------------------------------
// P0-1：崩溃循环熔断（crash-loop circuit breaker，对齐 supervisord
//       `startretries` + `startsecs`：退避封顶 30s 后无限重启必须被熔断终结）
// ---------------------------------------------------------------------------

/// 熔断参数（10 分钟窗口 / 重启 ≥5 次 / 30s 内秒退 ≥3 次 / 稳定 5 分钟清零）。
#[derive(Debug, Clone)]
pub(crate) struct RestartLoopPolicy {
    /// 滑动窗口长度：窗口外的重启事件不计入熔断。
    pub window: Duration,
    /// 窗口内重启达到该次数 → Degraded（停止自动重启）。
    pub restart_threshold: usize,
    /// 一次「秒退」的判定：本代进程存活不足该时长即退出。
    pub fast_exit: Duration,
    /// 窗口内秒退达到该次数 → Degraded（即便总重启数未达阈值——profile 锁坏 /
    /// 缺依赖的典型形态正是「每次起来 1 秒就死」）。
    pub fast_exit_threshold: usize,
    /// 连续健康运行达到该时长 → 清零熔断窗口（故障已自愈）。
    pub stable_clear: Duration,
}

impl Default for RestartLoopPolicy {
    fn default() -> Self {
        Self {
            window: Duration::from_secs(600),        // 10 分钟
            restart_threshold: 5,
            fast_exit: Duration::from_secs(30),
            fast_exit_threshold: 3,
            stable_clear: Duration::from_secs(300), // 稳定跑满 5 分钟清零
        }
    }
}

/// 熔断判定结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RestartLoopVerdict {
    /// 继续按退避自动重启。
    Continue,
    /// 熔断：停止自动重启，状态转 `Degraded`，等待人工显式 `sidecar_start`。
    Degraded,
}

/// P0-1 纯函数：判定重启事件序列是否已构成崩溃循环。
///
/// 输入全部是**单调时钟**（[`Instant`]）上的事件时刻，不受系统校时影响：
/// - `restarts`：守护任务每决定重启一次即记一笔（含本次待执行的这次）；
/// - `fast_exits`：每发现一代进程存活 < [`RestartLoopPolicy::fast_exit`] 即退出，记一笔；
/// - `now`：判定时刻。
///
/// 熔断条件（任一命中即 [`RestartLoopVerdict::Degraded`]）：
/// ① 滑动窗口内重启 ≥ `restart_threshold`；
/// ② 滑动窗口内秒退 ≥ `fast_exit_threshold`。
///
/// 「稳定运行满 stable_clear 清零」**刻意不放在本函数**：它是时间推进下的窗口修剪，
/// 由守护循环在「本轮健康且本代已跑满 stable_clear」时直接 `clear()`——纯函数只
/// 负责判定，副作用留在调用方，便于两边各自单测。
pub(crate) fn classify_restart_loop(
    restarts: &[Instant],
    fast_exits: &[Instant],
    now: Instant,
    policy: &RestartLoopPolicy,
) -> RestartLoopVerdict {
    let count_in_window = |events: &[Instant]| -> usize {
        events
            .iter()
            .filter(|t| now.saturating_duration_since(**t) <= policy.window)
            .count()
    };
    if count_in_window(restarts) >= policy.restart_threshold {
        return RestartLoopVerdict::Degraded;
    }
    if count_in_window(fast_exits) >= policy.fast_exit_threshold {
        return RestartLoopVerdict::Degraded;
    }
    RestartLoopVerdict::Continue
}

/// P0-4：Python 侧 graceful shutdown 端点（app.py: `POST /api/system/shutdown`，
/// owner-only，202 accepted 后异步执行 `GatewayServer.close()` 排空在飞 turn）。
///
/// 我们**只调用、不改动**该路径（Python 侧契约）：不带 Origin 的 Rust 直连请求过
/// `_same_origin`，带共享 token → `resolve_auth` 判 operator → `is_owner=True`。
pub(crate) const SIDECAR_SHUTDOWN_PATH: &str = "/api/system/shutdown";
/// `stop()` 的 drain 预算（对齐 Python `gateway_shutdown_deadline`）。
const SIDECAR_DRAIN_TIMEOUT: Duration = Duration::from_secs(10);
/// Drop 路径的 drain 预算（应用关闭流程不容久等）。
const SIDECAR_DROP_DRAIN_TIMEOUT: Duration = Duration::from_secs(2);
/// shutdown POST 自身的超时（端点 202 立即返回，drain 是它自己异步做的）。
const SIDECAR_SHUTDOWN_POST_TIMEOUT: Duration = Duration::from_secs(2);

/// P0-4：向侧车请求 graceful shutdown（最佳努力）。
///
/// 任何失败（侧车没起来 / 不支持 shutdown / 超时）都静默吞掉——调用方随后还有
/// drain→kill 两段兜底，这里绝不能因为「优雅关停请求发不出去」而把 stop() 本身
/// 变成错误源。
async fn request_sidecar_shutdown(base_url: &str, token: &str) {
    let url = format!("{base_url}{SIDECAR_SHUTDOWN_PATH}");
    let Ok(client) = reqwest::Client::builder().no_proxy().build() else {
        return;
    };
    let _ = timeout(
        SIDECAR_SHUTDOWN_POST_TIMEOUT,
        client.post(&url).bearer_auth(token).send(),
    )
    .await;
}

/// P0-4：先等子进程自然退出（drain 完成后 Python 自行 exit），超时再硬杀。
///
/// 抽成独立函数是为了让「drain 预算内自行退出 = 不硬杀」这条契约可单测：
/// 用一个立刻退出的真子进程验证 wait 自然返回，用一个 sleep 长进程验证超时硬杀。
pub(crate) async fn drain_or_kill(child: &mut Child, drain: Duration) {
    match timeout(drain, child.wait()).await {
        Ok(Ok(_)) => { /* drain 窗口内自然退出：优雅关停成功 */ }
        Ok(Err(_)) => {
            // wait 句柄异常：按硬杀兜底（不重复在文档里猜原因）
            let _ = child.kill().await;
        }
        Err(_) => {
            // drain 超时：排空未完成，硬杀收尾（既有行为，仅推迟到此刻）
            let _ = child.kill().await;
        }
    }
}

// ---------------------------------------------------------------------------
// P1-6：侧车资源指标（RSS）——只采集暴露，不做预警重启（避免与重启路径耦合）
// ---------------------------------------------------------------------------

/// 侧车最近一次资源采样（P1-6）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize)]
pub struct SidecarMetrics {
    /// 采样时的子进程 pid（复用路径没有子进程句柄时为 0）。
    pub pid: u32,
    /// 常驻集 RSS（字节）；采样失败为 0（诚实降级，不伪造读数）。
    pub rss_bytes: u64,
    /// 采样时刻（Unix 毫秒）。
    pub sampled_at_ms: u64,
    /// 采样时已存活毫秒。
    pub uptime_ms: u64,
}

/// P1-6：采样槽（进程级，与 [`global_exit_slot`] 同型——读方是同步 RPC handler）。
fn global_metrics_slot() -> &'static std::sync::Mutex<Option<SidecarMetrics>> {
    static SLOT: std::sync::OnceLock<std::sync::Mutex<Option<SidecarMetrics>>> =
        std::sync::OnceLock::new();
    SLOT.get_or_init(|| std::sync::Mutex::new(None))
}

/// 守护任务写入一次采样。
pub(crate) fn record_sidecar_metrics(m: SidecarMetrics) {
    match global_metrics_slot().lock() {
        Ok(mut slot) => *slot = Some(m),
        Err(poisoned) => *poisoned.into_inner() = Some(m),
    }
}

/// 读最近一次采样（从未采样过 → `None`）。
pub fn sidecar_metrics_snapshot() -> Option<SidecarMetrics> {
    match global_metrics_slot().lock() {
        Ok(slot) => *slot,
        Err(poisoned) => *poisoned.into_inner(),
    }
}

/// 守护任务的指标采样间隔（30s；主循环 tick 是 2s，即每 15 轮采一次）。
const METRICS_SAMPLE_TICKS: u32 = 15;

/// Linux：从 `/proc/<pid>/status` 文本解析 `VmRSS:`（kB）。纯函数，便于单测。
pub fn parse_vmrss_kb(text: &str) -> Option<u64> {
    for line in text.lines() {
        let mut it = line.split_whitespace();
        if it.next()? == "VmRSS:" {
            let kb: u64 = it.next()?.parse().ok()?;
            return Some(kb);
        }
    }
    None
}

/// 读某 pid 的常驻内存 RSS（字节）；任何失败一律 `None`（只读指标，不允许拖垮守护）。
pub fn read_process_rss_bytes(pid: u32) -> Option<u64> {
    if pid == 0 {
        return None;
    }
    #[cfg(windows)]
    {
        read_process_rss_windows(pid)
    }
    #[cfg(not(windows))]
    {
        let text = std::fs::read_to_string(format!("/proc/{pid}/status")).ok()?;
        parse_vmrss_kb(&text).map(|kb| kb.saturating_mul(1024))
    }
}

#[cfg(windows)]
fn read_process_rss_windows(pid: u32) -> Option<u64> {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
    use windows::Win32::System::Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut counters = PROCESS_MEMORY_COUNTERS::default();
        let result = GetProcessMemoryInfo(
            handle,
            &mut counters as *mut _ as *mut _,
            std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32,
        );
        let _ = CloseHandle(handle);
        result.ok()?;
        // WorkingSetSize 即进程当前常驻物理内存（RSS 语义，usize 无符号）。
        Some(counters.WorkingSetSize as u64)
    }
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

#[cfg(test)]
mod health_verdict_tests {
    use super::*;

    /// 回归守卫（R61 实测缺陷）：进程**还活着**、只是这一轮探活超时 → 必须只判 Suspect。
    ///
    /// 旧实现是 `if alive && http_ok { continue } else { 重启 }` —— 单次失手就 kill。
    /// 全量 e2e（4 worker）期间实测出现过 `alive=true http_ok=false`（网关日志原文），
    /// 侧车被误杀后 21s 才重启，界面上的引擎答案在 Rust/侧车之间翻转。
    #[test]
    fn alive_but_slow_probe_is_not_dead() {
        assert_eq!(
            classify_health(true, false, 1, HEALTH_MISS_LIMIT),
            HealthVerdict::Suspect,
            "单次探活超时不得判死（否则会杀掉正在干活的引擎）"
        );
        assert_eq!(
            classify_health(true, false, HEALTH_MISS_LIMIT - 1, HEALTH_MISS_LIMIT),
            HealthVerdict::Suspect,
            "未达连续失手上限不得判死"
        );
        assert_eq!(
            classify_health(true, false, HEALTH_MISS_LIMIT, HEALTH_MISS_LIMIT),
            HealthVerdict::Dead,
            "连续失手达上限才判死"
        );
    }

    /// 进程真的退出了 → 立即判死（不必等满失手上限，重启越早越好）。
    #[test]
    fn exited_process_is_dead_immediately() {
        assert_eq!(
            classify_health(false, false, 1, HEALTH_MISS_LIMIT),
            HealthVerdict::Dead
        );
        assert_eq!(
            classify_health(false, false, 0, HEALTH_MISS_LIMIT),
            HealthVerdict::Dead
        );
    }

    /// 复核通过（进程忙但活着）→ Healthy：调用方据此把连续失手清零。
    #[test]
    fn reprobe_success_counts_as_healthy() {
        assert_eq!(
            classify_health(true, true, 0, HEALTH_MISS_LIMIT),
            HealthVerdict::Healthy
        );
        assert_eq!(
            classify_health(true, true, 2, HEALTH_MISS_LIMIT),
            HealthVerdict::Healthy,
            "本轮健康即为 Healthy（计数由调用方清零）"
        );
    }

    /// 回归守卫（R61 实测缺陷）：**安装版**的 pyengine 在 `<install>/resources/pyengine/src`
    /// （Tauri 2 把资源放 exe 同级的 `resources/`），而旧探测只找 `pyengine/src` /
    /// `../pyengine/src` / exe 上溯 —— 安装目录里三者都不存在 → `detect` 失败 →
    /// 桌面端**静默降级成纯 Rust 模式**，打包进来的 Python 引擎永远用不上。
    #[test]
    fn packaged_resource_layout_is_searched_and_ranks_first() {
        let exe = PathBuf::from("/install/Marguerite/marguerite-gui.exe");
        let cwd = PathBuf::from("/somewhere/else");
        let cands = pyengine_src_candidates(&cwd, Some(&exe));
        assert_eq!(
            cands.first().map(|p| p.to_string_lossy().replace('\\', "/")),
            Some("/install/Marguerite/resources/pyengine/src".to_string()),
            "打包资源布局必须排在首位（否则会被别的仓库目录抢走）"
        );
    }

    /// 真实目录夹具：只有打包路径里有 `opensquilla/cli/main.py` 时必须命中它，
    /// 且前面的非法候选要被跳过（不是「第一个候选存在就算」）。
    #[test]
    fn first_valid_src_picks_the_packaged_tree_and_skips_invalid_candidates() {
        let root = std::env::temp_dir().join(format!("marg_src_fixture_{}", std::process::id()));
        let install = root.join("install");
        let packaged = install.join("resources/pyengine/src");
        std::fs::create_dir_all(packaged.join("opensquilla/cli")).unwrap();
        std::fs::write(packaged.join("opensquilla/cli/main.py"), "# fixture\n").unwrap();
        // 干扰项：cwd 下有一个目录名对但内容不对（无 main.py）的 `pyengine/src`
        let decoy = root.join("repo").join("pyengine/src");
        std::fs::create_dir_all(&decoy).unwrap();

        let exe = install.join("marguerite-gui.exe");
        let cands = pyengine_src_candidates(&root.join("repo"), Some(&exe));
        let hit = first_valid_src(cands).expect("必须找到打包进来的引擎源码");
        assert_eq!(hit, packaged, "应命中打包资源布局");
        assert!(!is_pyengine_src(&decoy), "干扰项不合法（缺 main.py）");

        std::fs::remove_dir_all(&root).ok();
    }

    /// 内置解释器必须排在首位：安装版是 `<install>/resources/python/python.exe`。
    #[test]
    fn bundled_python_path_is_the_install_resource_layout() {
        let exe = PathBuf::from("D:/install/Marguerite/marguerite-gui.exe");
        let cands = bundled_python_candidates(Some(exe.as_path()));
        assert_eq!(
            cands.first().map(|p| p.to_string_lossy().replace('\\', "/")),
            Some("D:/install/Marguerite/resources/python/python.exe".to_string()),
            "内置 Python 必须是第一候选（否则会退回系统 Python 或直接失败）"
        );
    }

    /// 真实目录夹具：`resources/python/python.exe` 存在时必须选中它。
    #[test]
    fn detect_python_prefers_bundled_runtime() {
        let root = std::env::temp_dir().join(format!("marg_py_fixture_{}", std::process::id()));
        let install = root.join("install");
        let bundled = install.join("resources/python/python.exe");
        std::fs::create_dir_all(bundled.parent().unwrap()).unwrap();
        std::fs::write(&bundled, b"").unwrap();

        let exe = install.join("marguerite-gui.exe");
        let hit = bundled_python_candidates(Some(exe.as_path()))
            .into_iter()
            .find(|p| p.is_file());
        assert_eq!(hit.as_deref(), Some(bundled.as_path()), "应命中内置运行时");

        std::fs::remove_dir_all(&root).ok();
    }

    /// 回归守卫（R61 实测缺陷）：限流回 **429** 必须算「活着」。
    ///
    /// 侧车限流中间件在负载下会对 `/health` 回 429（实测探针 21/90 次），而旧判定
    /// `status.is_success()` 把它读成「引擎已死」→ 看门狗杀掉健康引擎（日志原文
    /// `alive=true http_ok=false`）→ 21s 空窗，期间控制台所有 RPC 失败。
    #[test]
    fn rate_limited_probe_still_proves_liveness() {
        assert!(health_status_is_alive(200), "200 = 健康");
        assert!(
            health_status_is_alive(429),
            "429 是服务在回答问题，必须算活着（否则限流会被当成死亡信号）"
        );
        assert!(health_status_is_alive(404), "404 同样证明进程在服务");
        assert!(!health_status_is_alive(500), "5xx 说明应用自己坏了 → 不健康");
        assert!(!health_status_is_alive(503), "503 同上");
    }
}

/// P0-1 / P0-2 / P0-4 / P1-6：侧车稳定性加固单测。
#[cfg(test)]
mod stability_hardening_tests {
    use super::*;

    /// P0-2：看门狗探活必须打 `/ready`（事件循环新鲜度自证），不得回退到静态 `/health`。
    #[test]
    fn watchdog_probe_targets_ready_not_static_health() {
        assert_eq!(ready_probe_path(), "/ready");
        assert_ne!(ready_probe_path(), "/health", "静态 /health 只证明 TCP 在，会放过卡死的引擎");
    }

    fn policy() -> RestartLoopPolicy {
        RestartLoopPolicy {
            window: Duration::from_secs(600),
            restart_threshold: 5,
            fast_exit: Duration::from_secs(30),
            fast_exit_threshold: 3,
            stable_clear: Duration::from_secs(300),
        }
    }

    /// 测试用的"当前时刻"：基准 + 1 小时。
    ///
    /// 这些用例要构造 `now - 20min` 的事件流，而 `Instant` 的零点是**开机时刻**：
    /// 机器刚启动 12 分钟时 `Instant::now() - 20min` 会直接 panic
    /// （overflow when subtracting duration from instant，真机 05:0x 复现）。
    /// 加 1 小时对 `classify_restart_loop` 的语义没有影响（它只做时间差），
    /// 但保证任何开机时长下都不会下溢。
    fn test_now() -> Instant {
        Instant::now() + Duration::from_secs(3_600)
    }

    /// P0-1：10 分钟窗口内第 5 次重启 → Degraded（停止自动重启）。
    #[test]
    fn crash_loop_trips_on_five_restarts_in_window() {
        let now = test_now();
        let p = policy();
        let ev: Vec<Instant> = (0..4)
            .map(|i| now - Duration::from_secs(60 * (i + 1)))
            .collect();
        // 4 次（都在窗口内）→ 继续
        assert_eq!(
            classify_restart_loop(&ev, &[], now, &p),
            RestartLoopVerdict::Continue
        );
        // 第 5 次（含本次）→ 熔断
        let mut ev5 = ev.clone();
        ev5.push(now);
        assert_eq!(
            classify_restart_loop(&ev5, &[], now, &p),
            RestartLoopVerdict::Degraded,
            "窗口内第 5 次重启必须熔断"
        );
    }

    /// P0-1：总重启数没到 5，但 30s 内秒退已达 3 次 → 同样熔断
    /// （profile 锁坏 / 缺依赖的典型形态：退避封顶 30s 也救不回来）。
    #[test]
    fn crash_loop_trips_on_three_fast_exits_even_with_few_restarts() {
        let now = test_now();
        let p = policy();
        let fast: Vec<Instant> = (0..3)
            .map(|i| now - Duration::from_secs(90 * (i + 1)))
            .collect();
        assert_eq!(
            classify_restart_loop(&[now - Duration::from_secs(60)], &fast, now, &p),
            RestartLoopVerdict::Degraded,
            "3 次秒退（即便总重启只有 2 次）必须熔断"
        );
        let mut fast2 = fast.clone();
        fast2.pop();
        assert_eq!(
            classify_restart_loop(&[now - Duration::from_secs(60)], &fast2, now, &p),
            RestartLoopVerdict::Continue,
            "2 次秒退还在容忍区"
        );
    }

    /// P0-1：窗口外的重启事件不计入（11 分钟前的 100 次重启 ≠ 崩溃循环）。
    #[test]
    fn restarts_outside_the_window_do_not_trip() {
        let now = test_now();
        let p = policy();
        let old: Vec<Instant> = (0..10)
            .map(|i| now - Duration::from_secs(60 * (11 + i)))
            .collect();
        assert_eq!(
            classify_restart_loop(&old, &[], now, &p),
            RestartLoopVerdict::Continue,
            "窗口外的历史重启不得触发熔断"
        );
    }

    /// P1-6：`/proc/<pid>/status` 的 VmRSS 解析（纯函数）。
    #[test]
    fn parse_vmrss_kb_extracts_value_and_rejects_garbage() {
        let sample = "Name:\tpython3\nState:\tS (sleeping)\nVmRSS:\t  40960 kB\nThreads:\t5\n";
        assert_eq!(parse_vmrss_kb(sample), Some(40_960));
        assert_eq!(parse_vmrss_kb("VmPeak:\t10000 kB\n"), None, "只认 VmRSS:");
        assert_eq!(parse_vmrss_kb("VmRSS:\tnotanumber kB\n"), None);
        assert_eq!(parse_vmrss_kb(""), None);
    }

    /// P1-6：本进程 RSS 必须读得到且 > 0（Windows GetProcessMemoryInfo /
    /// Linux /proc 两条实现至少在本平台上真实可用——这是「采集不是空转」的证据）。
    #[test]
    fn own_process_rss_is_readable() {
        let rss = read_process_rss_bytes(std::process::id())
            .expect("本进程 RSS 必须读得到（实现至少在本平台可用）");
        assert!(rss > 0, "本进程 RSS 必须 > 0，实际 {rss}");
    }

    /// P0-4：drain 窗口内子进程自然退出 → 不硬杀（wait 正常返回）。
    #[tokio::test]
    async fn drain_or_kill_accepts_natural_exit_without_killing() {
        #[cfg(windows)]
        let mut child = {
            let mut c = Command::new("cmd");
            c.args(["/C", "exit", "0"]);
            c.spawn().unwrap()
        };
        #[cfg(not(windows))]
        let mut child = {
            let mut c = Command::new("sh");
            c.args(["-c", "exit 0"]);
            c.spawn().unwrap()
        };
        drain_or_kill(&mut child, Duration::from_secs(5)).await;
        let status = child.try_wait().unwrap();
        assert!(
            status.is_some(),
            "自然退出后 wait 应已回收（drain_or_kill 没有硬杀）"
        );
        assert!(status.unwrap().success());
    }

    /// P0-4：drain 窗口内子进程不肯退 → 超时硬杀（200ms 预算 vs 30s 睡眠）。
    #[tokio::test]
    async fn drain_or_kill_kills_after_budget() {
        #[cfg(windows)]
        let mut child = {
            let mut c = Command::new("cmd");
            c.args(["/C", "ping", "-n", "30", "127.0.0.1", ">nul"]);
            c.spawn().unwrap()
        };
        #[cfg(not(windows))]
        let mut child = {
            let mut c = Command::new("sh");
            c.args(["-c", "sleep 30"]);
            c.spawn().unwrap()
        };
        let started = Instant::now();
        drain_or_kill(&mut child, Duration::from_millis(200)).await;
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "drain 超时后必须立刻硬杀，实际 {:?}",
            started.elapsed()
        );
        let status = child.try_wait().unwrap();
        assert!(status.is_some(), "睡眠子进程应被 drain 超时后硬杀");
    }
}


/// S5：`save_last_exit` 在**记录文件不存在**时必须新建一条。
///
/// 这是「侧车启动即退出」跨重启可读的唯一路径：那条路上 `start()` 从走不到
/// `save_record`，若不肯新建，证据就只剩内存里那份——而这恰恰是 S5 最想解决的
/// 场景（下次启动 UI 断连时用户问「上次为什么坏」）。
///
/// 用 [`set_record_path_override`] 把记录隔离到临时目录，避免测试写真实用户数据
/// （`Config::set_config_dir_override` 只管 `config_dir()`，管不到这里用的
/// `data_dir()`——详见 [`record_path`] 的注释）。
#[test]
fn save_last_exit_creates_a_record_when_none_exists() {
    let dir = std::env::temp_dir().join(format!(
        "marguerite-save-exit-test-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    set_record_path_override(dir.join("sidecar.json"));

    let exit = HostExitEvidence {
        code: Some(7),
        uptime_ms: 202,
        stderr_tail: "boom".to_string(),
        observed: true,
    };

    // 无记录 + 有 pid → 必须落盘
    save_last_exit(Some(4242), 51214, &exit);
    let rec = load_record().expect("早退证据必须落盘，否则跨重启不可读");
    assert_eq!(rec.pid, 4242);
    assert_eq!(rec.port, 51214);
    let got = rec.last_exit.expect("last_exit 必须写入");
    assert_eq!(got.code, Some(7));
    assert_eq!(got.uptime_ms, 202);
    assert_eq!(got.stderr_tail, "boom");
    assert!(got.at_ms > 0, "必须带墙钟时间戳供消费方判断新鲜度");

    // 已有记录 + 无 pid → 保留既有 pid/port，只更新证据（守护任务重启路径）
    save_last_exit(None, 0, &exit);
    let rec = load_record().expect("记录仍在");
    assert_eq!(rec.pid, 4242, "无 pid 时不得覆盖既有 pid");
    assert_eq!(rec.port, 51214, "无 pid 时不得把port 写成 0（会被整条丢弃）");

    // 无记录 + 无 pid → 不写（无法构成合法记录），且不得留下半成品
    set_record_path_override(dir.join("fresh.json"));
    save_last_exit(None, 1234, &exit);
    assert!(
        !dir.join("fresh.json").exists(),
        "既无 pid 又无既有记录时不应写出不可解析的记录"
    );

    let _ = std::fs::remove_dir_all(&dir);
    // OVERRIDE_CONFIG_DIR 是线程局部，测试线程结束即失效，无需手动清理。
}

#[cfg(test)]
mod sidecar_reuse_tests {
    use super::*;

    /// 记录文件解析：正常 / 缺字段 / 非法数字 / 越界全部不得 panic，且只在合法时给出结果。
    ///
    /// S5：`last_exit` 是可选增强——缺失时必须是 `None`（而不是让整条记录解析失败，
    /// 否则一条损坏的证据会把 pid/port 一起拖垮，而 pid/port 是复用路径的唯一依据）。
    #[test]
    fn parse_record_accepts_only_well_formed_payloads() {
        assert_eq!(
            parse_record(r#"{"pid": 32484, "port": 61750}"#),
            Some(SidecarRecord {
                pid: 32484,
                port: 61750,
                last_exit: None
            })
        );
        assert!(parse_record("{}").is_none(), "缺字段");
        assert!(parse_record("not json").is_none(), "非 JSON");
        assert!(parse_record(r#"{"pid": 0, "port": 1}"#).is_none(), "pid=0 无意义");
        assert!(
            parse_record(r#"{"pid": 1, "port": 0}"#).is_none(),
            "port=0 无意义"
        );
        assert!(
            parse_record(r#"{"pid": 1, "port": 99999}"#).is_none(),
            "port 越界 u16"
        );
    }

    /// S5：`last_exit` 必须能往返解析，且**坏证据不得拖垮 pid/port**。
    #[test]
    fn parse_record_roundtrips_exit_evidence_and_tolerates_garbage() {
        let text = r#"{"pid": 32484, "port": 61750, "last_exit": {
            "code": 3, "uptime_ms": 1234, "at_ms": 1700000000000,
            "stderr_tail": "boom"
        }}"#;
        let rec = parse_record(text).expect("合法证据必须解析成功");
        let exit = rec.last_exit.expect("证据必须被读出");
        assert_eq!(rec.pid, 32484);
        assert_eq!(rec.port, 61750);
        assert_eq!(exit.code, Some(3));
        assert_eq!(exit.uptime_ms, 1234);
        assert_eq!(exit.at_ms, 1_700_000_000_000);
        assert_eq!(exit.stderr_tail, "boom");

        // 坏证据（字段类型不对 / 结构非法）→ 降级为 None，但 pid/port 必须保住
        for bad in [
            r#"{"pid": 1, "port": 2, "last_exit": 42}"#,
            r#"{"pid": 1, "port": 2, "last_exit": {"code": "three"}}"#,
            r#"{"pid": 1, "port": 2, "last_exit": {}}"#,
            r#"{"pid": 1, "port": 2, "last_exit": null}"#,
        ] {
            let rec = parse_record(bad).unwrap_or_else(|| panic!("坏证据不得影响 pid/port: {bad}"));
            assert!(rec.last_exit.is_none(), "坏证据必须降级为 None: {bad}");
        }
    }

    /// 端口抽取：IPv4 / IPv6 / 通配地址 / 无端口标签都要正确。
    #[test]
    fn port_after_last_colon_handles_common_forms() {
        assert_eq!(port_after_last_colon("127.0.0.1:61750"), Some(61750));
        assert_eq!(port_after_last_colon("[::1]:61750"), Some(61750));
        assert_eq!(port_after_last_colon("*:61750"), Some(61750));
        assert_eq!(port_after_last_colon("61750"), None, "没有冒号不算地址");
        assert_eq!(port_after_last_colon("127.0.0.1:0"), None, "0 端口无效");
        assert_eq!(port_after_last_colon("127.0.0.1:abc"), None);
    }

    /// Windows `netstat -ano`：只认目标 pid 的 LISTENING 行，且必须给出**全部**端口。
    ///
    /// 实测回归：同一个 Python 侧车进程同时监听 54069（内部）与 61750（网关），
    /// 只取第一个会让 `/health` 探活打错端口 → 复用静默失效。
    #[test]
    fn windows_netstat_picks_all_target_pid_listening_ports() {
        let text = "\
活动连接\r\n\
  协议  本地地址          外部地址        状态           PID\r\n\
  TCP    127.0.0.1:18790        0.0.0.0:0              LISTENING       35632\r\n\
  TCP    127.0.0.1:54069        0.0.0.0:0              LISTENING       32484\r\n\
  TCP    127.0.0.1:61750        0.0.0.0:0              LISTENING       32484\r\n\
  TCP    127.0.0.1:61761        127.0.0.1:61762        ESTABLISHED     32484\r\n\
  UDP    127.0.0.1:54069        *:*                                    32484\r\n";
        assert_eq!(
            parse_windows_netstat_listen_ports(text, 32484),
            vec![54069, 61750],
            "必须两个 LISTENING 端口都给，且忽略 ESTABLISHED/UDP"
        );
        assert_eq!(parse_windows_netstat_listen_ports(text, 35632), vec![18790]);
        assert!(
            parse_windows_netstat_listen_ports(text, 999).is_empty(),
            "不存在该 pid 时不得误取别人的端口"
        );
    }

    /// Unix `ss -ltnp`：必须匹配 `pid=<pid>`，并给出全部端口。
    #[test]
    fn unix_ss_picks_all_target_pid_listening_ports() {
        let text = "\
State  Recv-Q Send-Q Local Address:Port Peer Address:Port Process\n\
LISTEN 0      128    127.0.0.1:18790    0.0.0.0:*     users:((\"node\",pid=1234,fd=20))\n\
LISTEN 0      128    127.0.0.1:54069    0.0.0.0:*     users:((\"python\",pid=32484,fd=7))\n\
LISTEN 0      128    127.0.0.1:61750    0.0.0.0:*     users:((\"python\",pid=32484,fd=8))\n";
        assert_eq!(parse_unix_ss_listen_ports(text, 32484), vec![54069, 61750]);
        assert_eq!(parse_unix_ss_listen_ports(text, 1234), vec![18790]);
        assert!(parse_unix_ss_listen_ports(text, 42).is_empty());
    }

    /// Unix `lsof` 输出（无 pid 参数可匹配，取所有 LISTEN 行）。
    #[test]
    fn unix_lsof_picks_all_listening_ports() {
        let text = "\
COMMAND  PID  USER   FD   TYPE DEVICE SIZE/OFF NODE NAME\n\
python  32484 user   11u  IPv4 0x1233      0t0  TCP 127.0.0.1:54069 (LISTEN)\n\
python  32484 user   12u  IPv4 0x1234      0t0  TCP 127.0.0.1:61750 (LISTEN)\n";
        assert_eq!(parse_unix_lsof_listen_ports(text), vec![54069, 61750]);
        assert!(parse_unix_lsof_listen_ports("").is_empty());
    }

    /// Python 侧 pidfile：读得到 pid；文件缺失 / 坏 JSON → None（不得 panic）。
    #[test]
    fn python_pidfile_yields_pid_when_present() {
        let dir = std::env::temp_dir().join(format!(
            "marguerite-sidecar-pidfile-test-{}",
            std::process::id()
        ));
        let state = dir.join("default").join("state");
        std::fs::create_dir_all(&state).unwrap();
        let path = state.join("gateway.pid");
        std::fs::write(&path, r#"{"pid": 32484, "start_ts": "2026-09-28T10:34:10Z"}"#).unwrap();
        assert_eq!(read_python_gateway_pid(&dir), Some(32484));
        std::fs::write(&path, "broken").unwrap();
        assert_eq!(read_python_gateway_pid(&dir), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// stdout/stderr 落盘路径要成对且带端口，便于排障时按端口定位。
    #[test]
    fn sidecar_log_paths_are_port_scoped_pairs() {
        let (out, err) = sidecar_log_paths(61750);
        assert!(out.to_string_lossy().ends_with("sidecar-61750.out.log"));
        assert!(err.to_string_lossy().ends_with("sidecar-61750.err.log"));
    }

    /// 启动即退出（典型：`OPENSQUILLA_PROFILE_IN_USE`）必须在预算内被识别为失败，
    /// 而不是拖满整段健康探测超时（那条路径实测白等 90s，把真正原因埋掉）。
    ///
    /// S5：返回值已从 `Option<String>` 升级为 `Option<HostExitEvidence>`，
    /// 因此本测试同时断言**退出码真的被捡到了**（Windows / Unix 上
    /// `cmd /C exit 3` 与 `sh -c "exit 3"` 都映射为 code 3）——这正是旧代码
    /// 写作 `_status` 时丢掉的信息。
    #[tokio::test]
    async fn early_exit_is_detected_for_dead_child() {
        #[cfg(windows)]
        let mut cmd = {
            let mut c = Command::new("cmd");
            c.args(["/C", "exit", "3"]);
            c
        };
        #[cfg(not(windows))]
        let mut cmd = {
            let mut c = Command::new("sh");
            c.args(["-c", "exit 3"]);
            c
        };
        let child = Arc::new(Mutex::new(Some(cmd.spawn().unwrap())));
        let started_at = Arc::new(std::sync::Mutex::new(Instant::now()));
        let cfg = SidecarConfig {
            python_exe: PathBuf::from("unused"),
            pyengine_src: PathBuf::from("unused"),
            py_home: std::env::temp_dir(),
            // 用一个几乎不可能被占用的端口，避免读到别的测试留下的日志内容
            port: 1,
            shared_token: String::new(),
            debug: false,
            restart_backoff_min: Duration::from_millis(100),
            restart_backoff_max: Duration::from_secs(1),
            health_timeout: Duration::from_secs(1),
        };
        let reason =
            early_exit_reason(&cfg, &child, &started_at, Duration::from_secs(5)).await;
        let evidence = reason.expect("子进程退出必须被识别，而不是等满健康超时");
        assert_eq!(
            evidence.code,
            Some(3),
            "S5：退出码必须被捡到（旧代码 `_status` 直接丢弃）"
        );
        assert!(
            evidence.uptime_ms < 5_000,
            "uptime 必须落在启动预算内，实际 {}ms",
            evidence.uptime_ms
        );
    }

    /// S5：`tail_bytes` 必须按 UTF-8 字符边界尾切，绝不产出 `U+FFFD` 替换符。
    ///
    /// 这是本任务最容易写错的一处：尾切是按**字节**做的（`seek` 到 `len-max` 再读），
    /// 而中文 / 希腊字母都是多字节序列，切在中间就会解码失败。`β`（U+03B2）恰好
    /// 2 字节，`max=5` 会落在「一个 β 的第二个字节」上——无边界处理时这里就会炸。
    #[test]
    fn tail_bytes_never_splits_a_multibyte_character() {
        let dir = std::env::temp_dir().join(format!("marguerite-tail-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("tail.txt");

        // 全ASCII：max 小于长度 → 取**尾部** max 字节；max 足够大 → 原样返回
        std::fs::write(&path, b"abcdefghij").unwrap();
        assert_eq!(tail_bytes(&path, 4), "ghij");
        assert_eq!(tail_bytes(&path, 100), "abcdefghij");

        // 多字节：β = 0xCE 0xB2（2 字节/字），总长 12 字节。
        // max=5 → 起点 7 落在第一个 β 的**第二个字节**上，必须回退到下一个边界。
        std::fs::write(&path, "αααβββ").unwrap();
        let tail = tail_bytes(&path, 5);
        assert!(
            !tail.contains('\u{FFFD}'),
            "不得产出替换符，实际 {tail:?}"
        );
        assert_eq!(tail, "ββ", "必须回退到下一个字符边界");

        // max 落在多字节字符内部的各种偏移都要保持合法 UTF-8，且结果始终是原文后缀
        let text = "启动失败：端口被占用";
        std::fs::write(&path, text).unwrap();
        for max in 0..=text.len() + 4 {
            let tail = tail_bytes(&path, max);
            assert!(
                !tail.contains('\u{FFFD}'),
                "max={max} 产出了替换符：{tail:?}"
            );
            assert!(
                text.ends_with(&tail),
                "max={max} 的结果必须是原文的后缀：{tail:?}"
            );
        }

        // 空文件 / max=0 / 文件不存在：回退空串，绝不 panic
        std::fs::write(&path, b"").unwrap();
        assert_eq!(tail_bytes(&path, 16), "");
        assert_eq!(tail_bytes(&path, 0), "");
        assert_eq!(tail_bytes(&dir.join("nope.log"), 16), "");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// S5：超过上限时必须真的截断到 ≤16KB（否则 UI 折叠详情会变成传整个日志）。
    #[test]
    fn tail_bytes_respects_the_size_cap() {
        let dir = std::env::temp_dir().join(format!("marguerite-tail-cap-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("big.log");
        // 100_000 个 ASCII 字符（UTF-8 下 1 字节/字符）
        std::fs::write(&path, "E".repeat(100_000)).unwrap();
        let tail = tail_bytes(&path, STDERR_TAIL_MAX_BYTES);
        assert!(
            tail.len() <= STDERR_TAIL_MAX_BYTES,
            "必须截断到上限内，实际 {} 字节",
            tail.len()
        );
        assert_eq!(tail.len(), STDERR_TAIL_MAX_BYTES);
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ── S5：复用路径退出证据（attach 守护）─────────────────────────────────

    /// `tasklist` 输出解析：命中 / 未命中 / 名字里含相同数字串（防子串误判）。
    ///
    /// 「名字里含数字」这条是真实踩过的坑：`tasklist` 的内存列形如 `"35,000 K"`，
    /// 若用 `contains(pid)` 判活会把**已死**的进程判成活着，于是永远等不到退出
    /// 证据——也就是本缺陷换一种方式复现。
    #[test]
    fn parse_tasklist_has_pid_matches_fields_exactly() {
        let hit = "\"python.exe\",\"11908\",\"Console\",\"1\",\"35,000 K\"\r\n";
        assert!(parse_tasklist_has_pid(hit, 11908));
        assert!(
            !parse_tasklist_has_pid(hit, 1190),
            "pid=1190 不应被 11908 的行误命中"
        );
        let miss = "INFO: No tasks are running which match the specified criteria.\r\n";
        assert!(!parse_tasklist_has_pid(miss, 11908));
        assert!(!parse_tasklist_has_pid("", 11908));
        // 内存列里恰好含 pid 数字串，也不得判成命中
        let mem_only = "\"python.exe\",\"4242\",\"Console\",\"1\",\"4,242 K\"\r\n";
        assert!(!parse_tasklist_has_pid(mem_only, 242));
    }

    /// 判活：真实起一个进程 → 活着；杀掉 → 不活着。
    ///
    /// 这条**真的 spawn 并 kill 一个进程**（不依赖 Python / 不依赖侧车），
    /// 因此能在 CI 上守住 `pid_is_alive` 这条判据本身。
    #[test]
    #[cfg(windows)]
    fn pid_is_alive_tracks_a_real_process() {
        let mut child = std::process::Command::new("cmd")
            .args(["/c", "ping -n 30 127.0.0.1 > nul"])
            .spawn()
            .expect("起测试进程");
        let pid = child.id();
        assert!(pid != 0);
        // 给它一点真正起来的时间；刚 spawn 的 pid 可能尚未出现在 tasklist 里。
        let mut alive = false;
        for _ in 0..40 {
            if pid_is_alive(pid) {
                alive = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(alive, "刚 spawn 的进程 pid={pid} 应被判定为存活");
        child.kill().expect("杀掉测试进程");
        let _ = child.wait();
        // 判死要轮询：tasklist 快照有延迟，进程刚退出时可能还残留一行。
        let mut dead = false;
        for _ in 0..40 {
            if !pid_is_alive(pid) {
                dead = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(dead, "已被 kill 的进程 pid={pid} 必须判定为不存活");
    }

    /// `pid=0` 恒判不存活（0 从来不是合法 pid）。
    #[test]
    fn pid_zero_is_never_alive() {
        assert!(!pid_is_alive(0));
    }

    /// 🔴 S5 缺陷回归：**复用路径 attach 后仍必须采到运行期退出证据**。
    ///
    /// 这是实机失败的守卫：`taskkill` 掉一个被复用的真实进程后，
    /// `host.exit_evidence` 曾恒为 `exited:false` / `evidence:null`。
    ///
    /// 为什么之前所有测试都拦不住：桩测的都是 `raise SystemExit(7)`（启动即早退，
    /// 走 `early_exit_reason`），而缺陷发生在**运行期**、且只在**复用路径**上。
    /// 所以这里必须**真的 spawn 一个进程并真的杀它**。
    #[tokio::test]
    #[cfg(windows)]
    async fn attach_guard_records_evidence_when_adopted_process_dies() {
        // 隔离记录文件，避免写到真实用户数据目录。
        let dir = std::env::temp_dir().join(format!(
            "marguerite-attach-guard-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        set_record_path_override(dir.join("sidecar.json"));

        let mut child = std::process::Command::new("cmd")
            .args(["/c", "ping -n 30 127.0.0.1 > nul"])
            .spawn()
            .expect("起测试进程");
        let pid = child.id();

        // 等它真正在 tasklist 里出现，保证 attach 的「进程在跑」前提成立。
        let mut spawned_ok = false;
        for _ in 0..40 {
            if pid_is_alive(pid) {
                spawned_ok = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(spawned_ok, "测试进程未能在 tasklist 中出现");

        // 清理进程级证据槽，让断言只反映本次 attach 守护的产出。
        record_exit_evidence(
            Some(1),
            1,
            HostExitEvidence {
                code: Some(-999),
                uptime_ms: 0,
                stderr_tail: "sentinel".to_string(),
                observed: true,
            },
        );

        let cfg = SidecarConfig {
            python_exe: PathBuf::from("python"),
            pyengine_src: dir.clone(),
            py_home: dir.clone(),
            // 用一个几乎不可能被占用的端口，仅用于日志路径。
            port: 59001,
            shared_token: String::new(),
            debug: false,
            restart_backoff_min: Duration::from_millis(10),
            restart_backoff_max: Duration::from_millis(50),
            health_timeout: Duration::from_millis(50),
        };
        let mut sidecar = PyEngineSidecar::attach(cfg, Some(pid));
        // attach 必须真的挂上守护任务——这正是原缺陷（`_guard: None`）。
        assert!(
            sidecar._guard.is_some(),
            "复用路径必须挂守护任务，否则运行期退出证据永远采不到"
        );

        // 真的杀掉被附着的进程。
        child.kill().expect("杀掉被附着的进程");
        let _ = child.wait();

        // 等守护任务跑到判活分支并落证据。
        let mut ev = None;
        for _ in 0..60 {
            if let Some(e) = last_exit_evidence() {
                if e.code != Some(-999) {
                    ev = Some(e);
                    break;
                }
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        let ev = ev.expect("进程被杀后必须采到退出证据");
        assert!(
            ev.observed,
            "复用路径证据必须标记 observed，否则 RPC 过滤会退回 exited:false"
        );
        // 存活时长必须落在「附着 → 被杀 → 采集」这个真实区间内。
        //
        // 刻意**不**断言与 `sidecar.uptime_ms()` 完全相等：守护任务落证据的时刻与
        // 断言读取的时刻之间隔着一次监督轮询（2s tick），逐毫秒漂移
        // 是正常的（实测 3329 vs 3330）。断言相等于把测试绑死在调度时序上。
        assert!(
            ev.uptime_ms > 0,
            "存活时长必须为正，实际 {}",
            ev.uptime_ms
        );
        assert!(
            ev.uptime_ms <= sidecar.uptime_ms() + 5_000,
            "存活时长 {} 不应超过「附着至今 + 一轮轮询余量」",
            ev.uptime_ms
        );
        // 收尾：先显式中止监督任务并 drop，避免接管路径在删除目录后继续 spawn。
        if let Some(g) = sidecar._guard.take() {
            g.abort();
        }
        drop(sidecar);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 🔴 S5 契约守卫：复用路径的证据（`code=None` + 空 stderr）**不得**被
    /// `host.exit_evidence` 的过滤判据吞掉。
    ///
    /// 这是本缺陷的最后一环：即使守护采到了证据，旧判据
    /// `code.is_some() || !stderr_tail.is_empty()` 在 `code=None` 且 stderr 为空时
    /// 仍会把它过滤掉，RPC 照样回 `exited:false`。
    #[test]
    fn observed_evidence_survives_the_rpc_filter_without_code_or_tail() {
        let ev = HostExitEvidence {
            code: None,
            uptime_ms: 12_345,
            stderr_tail: String::new(),
            observed: true,
        };
        assert!(
            ev.observed || ev.code.is_some() || !ev.stderr_tail.is_empty(),
            "复用路径的真实退出证据必须通过过滤判据"
        );
        // 反向守卫：`observed=false`（老记录）时仍走旧判据，不被当成新观测。
        let legacy = HostExitEvidence {
            code: None,
            uptime_ms: 1,
            stderr_tail: String::new(),
            observed: false,
        };
        assert!(
            !(legacy.observed || legacy.code.is_some() || !legacy.stderr_tail.is_empty()),
            "缺 observed 的老记录不应被误判为观测结果"
        );
    }

    /// 落盘往返：`observed` 必须活下来，否则跨重启后复用路径证据又变回不可见。
    #[test]
    fn save_last_exit_roundtrips_the_observed_flag() {
        let dir = std::env::temp_dir().join(format!(
            "marguerite-observed-roundtrip-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        set_record_path_override(dir.join("sidecar.json"));
        let _ = std::fs::remove_file(record_path());

        let exit = HostExitEvidence {
            code: None,
            uptime_ms: 777,
            stderr_tail: String::new(),
            observed: true,
        };
        save_last_exit(Some(3210), 51999, &exit);
        let rec = load_record().expect("证据必须落盘");
        let last = rec.last_exit.expect("last_exit 必须存在");
        assert!(last.observed, "observed 必须往返持久化");
        assert_eq!(last.uptime_ms, 777);
        assert_eq!(last.code, None);

        // 老记录（无 observed 字段）必须仍能解析，且 observed=false。
        let legacy_text = r#"{"pid": 1, "port": 2, "last_exit": {
            "code": 3, "uptime_ms": 4, "at_ms": 5, "stderr_tail": ""
        }}"#;
        let parsed = parse_record(legacy_text).expect("老记录必须可解析");
        assert!(
            !parsed.last_exit.expect("老记录 last_exit").observed,
            "缺字段的老记录 observed 必须是 false"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 候选去重：同端口只出现一次，且携带 pid（attach 判活靠它）。
    #[test]
    fn candidate_sidecars_dedups_ports_and_carries_pid() {
        let cands = vec![
            SidecarCandidate { pid: Some(111), port: 61000 },
            SidecarCandidate { pid: Some(222), port: 61000 },
            SidecarCandidate { pid: None, port: 62000 },
        ];
        let mut seen = Vec::new();
        for c in &cands {
            if !seen.contains(&c.port) {
                seen.push(c.port);
            }
        }
        assert_eq!(seen, vec![61000, 62000]);
        assert_eq!(cands[0].pid, Some(111));
    }
}
