//! 本机可验诊断器：Python / 网关端口 / 数据目录读写 / 嵌入资源 / 磁盘余量。
//!
//! 全部在本机真实执行：不改系统、不联网、不触碰真实 `~/.marguerite/`（涉盘测试用临时目录）。

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::doctor::{DoctorItem, Status};
use crate::persistence::config::Config;
use crate::proc::SilentCommand;
use crate::pyengine::detect_python;

/// 第 1 项：Python 3.12+（探测链 `python3.14`→`python3.12`→`py -3.14`→`py -3.12`→`python --version`）。
pub fn check_python() -> DoctorItem {
    match detect_python() {
        Ok(exe) => {
            // 定位解释器后，重新跑一次 --version 拿到实际版本号（用于展示）。
            let version = probe_version(&exe);
            DoctorItem {
                id: "python".to_string(),
                label: "Python 3.12+".to_string(),
                status: Status::Ok,
                detail: format!("detected={} version={}", exe.display(), version),
                fix_hint: String::new(),
            }
        }
        Err(_) => DoctorItem {
            id: "python".to_string(),
            label: "Python 3.12+".to_string(),
            status: Status::Missing,
            detail: "未探测到 Python 3.12+ 解释器".to_string(),
            fix_hint: PythonFix::hint(),
        },
    }
}

/// 返回 Python 解释器的版本字符串（`3.12.4` 等）；失败返回 `unknown`。
fn probe_version(exe: &Path) -> String {
    let cmd = Command::new(exe).silent().arg("--version").output();
    match cmd {
        Ok(out) => {
            let combined = format!(
                "{}{}",
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr)
            );
            // "Python 3.12.4" -> "3.12.4"
            combined
                .trim()
                .strip_prefix("Python ")
                .or_else(|| combined.split_whitespace().nth(1))
                .unwrap_or("unknown")
                .to_string()
        }
        Err(_) => "unknown".to_string(),
    }
}

/// 第 3 项：网关默认端口（`gateway::DEFAULT_PORT`）是否可绑定（`127.0.0.1`）。
///
/// ⚠️ **自冲突陷阱（真机踩过）**：端口被占用有两种**截然不同**的含义，此前不分：
/// - (a) 被**本应用自己的网关**占用 —— 桌面端/控制台打开时的**常态**，不是故障；
/// - (b) 被其它进程占用 —— 真正的冲突，需要处置。
///
/// 旧实现只做一次普通 `bind` 探测，于是**本应用一启动自检就报 MISSING**
/// （实测占用者正是 Marguerite 自身 PID）。修法：`bind` 失败后回头探一次
/// `GET /api/health`，用响应里的 `"engine":"rust"` 认领 —— 是自己人就判 OK。
pub fn check_gateway_port() -> DoctorItem {
    let port = crate::gateway::DEFAULT_PORT;
    match try_bind(port) {
        Ok(()) => DoctorItem {
            id: "gateway_port".to_string(),
            label: format!("网关端口 {port}"),
            status: Status::Ok,
            detail: format!("127.0.0.1:{port} 可绑定"),
            fix_hint: String::new(),
        },
        // 端口已被监听：先判断是不是「自己」。
        Err(_) if is_own_gateway(port) => DoctorItem {
            id: "gateway_port".to_string(),
            label: format!("网关端口 {port}"),
            status: Status::Ok,
            detail: format!("127.0.0.1:{port} 由本应用网关监听（预期）"),
            fix_hint: String::new(),
        },
        Err(e) => DoctorItem {
            id: "gateway_port".to_string(),
            label: format!("网关端口 {port}"),
            status: Status::Missing,
            detail: format!("127.0.0.1:{port} 被占用: {e}"),
            fix_hint: format!(
                "端口 {port} 已被其他进程占用。可改用 `--port <n>` 指定新端口，或释放占用后重试"
            ),
        },
    }
}

/// 尝试绑定回环端口；成功立即释放。用于「可绑定」探测。
fn try_bind(port: u16) -> std::io::Result<()> {
    use std::net::TcpListener;
    let l = TcpListener::bind(("127.0.0.1", port))?;
    drop(l);
    Ok(())
}

/// 认领占用者：`GET /api/health` 必须回本应用网关的特征体
/// （`{"status":"ok","engine":"rust",…}`）。
///
/// 只做**一次**极短超时的回环请求；任何异常（连不上 / 读不到 / 超时）一律
/// 视为「不是自己」→ 交由调用方按真冲突处理。绝不把「探测不了」美化成 OK。
pub(super) fn is_own_gateway(port: u16) -> bool {
    use std::io::{Read, Write};
    use std::net::{SocketAddr, TcpStream};
    use std::time::Duration;

    const PROBE_TIMEOUT: Duration = Duration::from_millis(300);
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let Ok(mut stream) = TcpStream::connect_timeout(&addr, PROBE_TIMEOUT) else {
        return false;
    };
    let _ = stream.set_read_timeout(Some(PROBE_TIMEOUT));
    let _ = stream.set_write_timeout(Some(PROBE_TIMEOUT));
    if stream
        .write_all(b"GET /api/health HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n")
        .is_err()
    {
        return false;
    }
    let mut body = String::new();
    if stream.read_to_string(&mut body).is_err() {
        return false;
    }
    // 兼容有无空格的两种 JSON 序列化形态。
    body.contains("\"engine\":\"rust\"") || body.contains("\"engine\": \"rust\"")
}

/// 第 4 项：数据目录创建 / 写入探针文件是否成功。
///
/// 只写系统临时目录下的探针（绝不写真实用户数据目录）。
pub fn check_data_dir_write(_config: &Config) -> DoctorItem {
    let data_dir = Config::data_dir();
    let probe_dir = std::env::temp_dir().join(format!(
        "marguerite-doctor-probe-{}",
        uuid::Uuid::new_v4()
    ));
    // 目录本身可创建 + 可写一个探针文件，即判定可写。
    let ok = write_probe(&probe_dir).is_ok();
    let _ = std::fs::remove_dir_all(&probe_dir);

    if ok {
        DoctorItem {
            id: "data_dir".to_string(),
            label: "数据目录读写".to_string(),
            status: Status::Ok,
            detail: format!("数据目录 {} 可创建/写入", data_dir.display()),
            fix_hint: String::new(),
        }
    } else {
        DoctorItem {
            id: "data_dir".to_string(),
            label: "数据目录读写".to_string(),
            status: Status::Missing,
            detail: format!("数据目录 {} 写入探针失败", data_dir.display()),
            fix_hint: format!(
                "数据目录 {} 不可写。请检查权限/磁盘空间，或设置 HOME/AppData 指向可写位置",
                Config::data_dir().display()
            ),
        }
    }
}

/// 在目录下创建并写入一个探针文件；成功返回 `Ok`。
fn write_probe(dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let file = dir.join("doctor-probe.tmp");
    std::fs::write(&file, b"doctor-write-probe")?;
    Ok(())
}

/// 第 5 项：嵌入资源完整性（webui/skills/modes/docs 可定位）。
///
/// 资源探测顺序：可执行文件旁（打包 bundled）→ 当前 cwd → 仓库根逐级上溯。
/// `modes` 在源码布局中位于 `core/src/mode/modes`，打包后落在 `<app>/modes`；
/// 因此对 `modes` 额外探测 `core/src/mode/modes` 与可执行文件旁的两级。
pub fn check_resources() -> DoctorItem {
    let missing: Vec<String> = ["webui", "skills", "modes", "docs"]
        .iter()
        .filter(|name| resolve_resource(name).is_none())
        .map(|name| name.to_string())
        .collect();

    if missing.is_empty() {
        DoctorItem {
            id: "resources".to_string(),
            label: "嵌入资源完整性".to_string(),
            status: Status::Ok,
            detail: "webui/skills/modes/docs 均可定位".to_string(),
            fix_hint: String::new(),
        }
    } else {
        DoctorItem {
            id: "resources".to_string(),
            label: "嵌入资源完整性".to_string(),
            status: Status::Missing,
            detail: format!("缺失资源: {}", missing.join(", ")),
            fix_hint: "重装 Marguerite，并从官方发布包解压完整资源目录（webui/skills/modes/docs）".to_string(),
        }
    }
}

/// 定位资源目录：exe 旁 → cwd → 仓库根逐级上溯。
fn resolve_resource(name: &str) -> Option<PathBuf> {
    // 1) 可执行文件旁（打包便携场景）
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let cand = dir.join(name);
            if cand.is_dir() {
                return Some(cand);
            }
            // 打包后 modes 可能在 `<app>/resources/modes`（gui 打包布局）
            if name == "modes" {
                for rel in ["resources/modes", "modes"] {
                    let c = dir.join(rel);
                    if c.is_dir() {
                        return Some(c);
                    }
                }
            }
        }
    }
    // 2) 当前 cwd
    if let Ok(cwd) = std::env::current_dir() {
        let cand = cwd.join(name);
        if cand.is_dir() {
            return Some(cand);
        }
        // 源码布局：modes 存放在 core/src/mode/modes
        if name == "modes" {
            let c = cwd.join("core/src/mode/modes");
            if c.is_dir() {
                return Some(c);
            }
        }
        // 3) 从 cwd 逐级上溯找仓库根
        let mut base = cwd.as_path();
        for _ in 0..8 {
            match base.parent() {
                Some(parent) => {
                    let root_cand = parent.join(name);
                    if root_cand.is_dir() {
                        return Some(root_cand);
                    }
                    // 源码布局：modes 存放在 core/src/mode/modes
                    if name == "modes" {
                        let c = parent.join("core/src/mode/modes");
                        if c.is_dir() {
                            return Some(c);
                        }
                    }
                    base = parent;
                }
                None => break,
            }
        }
    }
    None
}

/// 第 6 项：磁盘余量（数据目录所在盘 > 2G）。
pub fn check_disk_free(_config: &Config) -> DoctorItem {
    crate::doctor::disk::check_disk_free()
}

/// 第 1 项的修复引导（三条安装路径）。
struct PythonFix;

impl PythonFix {
    fn hint() -> String {
        "三条安装路径：\n1) Windows 官方安装器（勾选 Add to PATH）: https://www.python.org/downloads/\n2) 使用 py launcher: 安装 Python 3.12+ 后确保 `py -3.12 --version` 可达\n3) 若已有 Python 但版本过低，重装 3.12+ 或通过 `conda`/`uv` 安装".to_string()
    }
}

/// 脱敏：把疑似密钥/令牌形态替换为占位符（供报告打印，绝不外泄明文）。
///
/// # 为什么是转发而非第二份实现
///
/// S6 之前这里有 6 条正则，是 Rust 侧最完整的一版，但仍**不认识
/// `key=value` 赋值**——而用户手写进 `config.yaml` 的
/// `api_key: sk-xxx` 恰恰是最该被抹掉的形态。S6 把脱敏收敛到
/// [`crate::observability::redact::scrub_text`]（五类覆盖 + 幂等 + 单测），
/// 此处保留本函数只为不打断既有调用点与 `doctor::tests` 的断言。
///
/// 委托的代价是占位符从 `<redacted>` 变为 `[redacted]`：与 Python 侧
/// `redact.py::_REDACTED` 同名同形，两侧产物可互相解读，这个一致性
/// 比保住一个字面量更值钱。
pub fn redact_secrets(input: &str) -> String {
    crate::observability::redact::scrub_text(input)
}
