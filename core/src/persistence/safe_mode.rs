//! Safe Mode（S10）：以「**绝不读用户数据**」的最小环境启动一次。
//!
//! # 为什么需要它
//!
//! 用户配置/缓存/凭据损坏时，普通启动会**先读到坏数据再崩**，且崩在哪一步不确定；
//! 更糟的是「读一半、写一半」会进一步污染本已损坏的状态。Safe Mode 的做法是：
//! 把**配置目录、数据目录、日志目录同时**指向一个全新的一次性目录，于是它
//! 既不读用户数据、也不会写坏用户数据 —— 用来回答「到底是配置问题还是程序问题」。
//!
//! # 为什么三个目录必须一起换
//!
//! 只隔离其中之一是不够的：
//! - `config_dir()`（可由 `MARGUERITE_CONFIG_DIR` 覆盖）：`config.yaml` / `.env` /
//!   `models.json` / `sessions.db`；
//! - `data_dir()`（由 `MARGUERITE_DATA_DIR` 覆盖）：`audit.db` / `credentials.json` /
//!   `gateway-token` / 各类 cache / `sidecar.json`；
//! - 侧车日志目录（`MARGUERITE_SIDECAR_LOG_DIR`）。
//!
//! 这三者分别落在 `~/.marguerite` 与平台数据目录两处，**任何一个没盖住都会漏读用户数据**。
//!
//! # 与 S9（三槽健康启动 checkpoint）的关系
//!
//! S9 负责「回滚到某个好状态」，S10 负责「用一个干净环境起一次」。两者互补：
//! 用户可以先 Safe Mode 起来把诊断包导出来（S6），再决定要不要回滚。

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

/// 标记文件名：一次性目录里存在它，即表明该目录是 Safe Mode 的产物。
///
/// 存在的意义是**可回收**：下次普通启动时可以据此识别并清理历史残留，
/// 而不会误删用户自己放在同层的目录。
pub const MARKER_FILE: &str = "SAFE_MODE";

/// 三个隔离用的环境变量名（与各自解析点的真源一致）。
pub const ENV_CONFIG_DIR: &str = "MARGUERITE_CONFIG_DIR";
pub const ENV_DATA_DIR: &str = "MARGUERITE_DATA_DIR";
pub const ENV_SIDECAR_LOG_DIR: &str = "MARGUERITE_SIDECAR_LOG_DIR";

/// 命令行是否请求 Safe Mode（`marguerite --safe …`）。
///
/// 只认精确的 `--safe`：不接受 `--safe=1` 之类的变体，避免把用户随手写的
/// 参数误当成「启用隔离」而让人误以为数据被保护了。
pub fn requested(args: &[String]) -> bool {
    args.iter().any(|a| a == "--safe")
}

/// 一次性目录根：`<base>/marguerite-safe-<时间戳>-<pid>`。
///
/// 带 pid 与时间戳是为了**可并存**：同一台机器上多个 Safe Mode 实例互不覆盖。
/// 纯函数，便于单测（`base` 传 `std::env::temp_dir()`）。
pub fn root_under(base: &Path, stamp_ms: i64, pid: u32) -> PathBuf {
    base.join(format!("marguerite-safe-{stamp_ms}-{pid}"))
}

/// Safe Mode 的三个隔离目录 + 标记文件路径。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SafeModePaths {
    pub root: PathBuf,
}

impl SafeModePaths {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    pub fn config_dir(&self) -> PathBuf {
        self.root.join("config")
    }

    pub fn data_dir(&self) -> PathBuf {
        self.root.join("data")
    }

    pub fn log_dir(&self) -> PathBuf {
        self.root.join("logs")
    }

    pub fn marker(&self) -> PathBuf {
        self.root.join(MARKER_FILE)
    }
}

/// 建目录 + 写标记（**不动环境变量**，便于单测）。
pub fn prepare(root: PathBuf) -> Result<SafeModePaths> {
    let paths = SafeModePaths::new(root);
    for dir in [paths.config_dir(), paths.data_dir(), paths.log_dir()] {
        std::fs::create_dir_all(&dir)
            .with_context(|| format!("Safe Mode 建目录失败: {}", dir.display()))?;
    }
    std::fs::write(paths.marker(), b"marguerite safe mode\n")
        .with_context(|| format!("Safe Mode 写标记失败: {}", paths.marker().display()))?;
    Ok(paths)
}

/// 把隔离目录写进进程环境。
///
/// ⚠️ **必须在任何 `Config` 读取之前调用**（当前唯一调用点是 `main()` 的首行逻辑）：
/// 一旦 `Config::load()` 先跑过，用户 `config.yaml` 就已经被读进来了。
///
/// 这里用 `std::env::set_var` 是**有意**的：进程环境正是各解析点读取的真源，
/// 注入别的间接层反而会漏掉直接读 env 的调用方（如 `MARGUERITE_SIDECAR_LOG_DIR`）。
/// 只在启动初期、尚未派生 worker 线程时调用，故不存在多线程 `set_var` 竞态。
pub fn apply_env(paths: &SafeModePaths) {
    std::env::set_var(ENV_CONFIG_DIR, paths.config_dir());
    std::env::set_var(ENV_DATA_DIR, paths.data_dir());
    std::env::set_var(ENV_SIDECAR_LOG_DIR, paths.log_dir());
}

/// 一步到位：在临时目录下建可弃目录 + 写标记 + 注入环境变量。
///
/// 时间戳由调用方给出（毫秒），避免此函数自身依赖时钟而不易测试。
pub fn activate_under(base: &Path, stamp_ms: i64, pid: u32) -> Result<SafeModePaths> {
    let paths = prepare(root_under(base, stamp_ms, pid))?;
    apply_env(&paths);
    Ok(paths)
}

/// [`activate_under`] 的生产入口：以系统临时目录 + 当前时间/pid 建立。
pub fn activate() -> Result<SafeModePaths> {
    let stamp_ms = chrono::Utc::now().timestamp_millis();
    activate_under(&std::env::temp_dir(), stamp_ms, std::process::id())
}

/// 该目录是否是 Safe Mode 产物（存在标记文件）。
///
/// 供未来的残留清理使用：**只认标记**，不按名字猜，避免误删用户同名目录。
pub fn is_marked(root: &Path) -> bool {
    root.join(MARKER_FILE).is_file()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "marguerite-safe-mode-test-{tag}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn requested_accepts_exact_flag_only() {
        let a = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert!(requested(&a(&["gateway", "--safe"])));
        assert!(requested(&a(&["--safe"])));
        // 变体不认：避免「以为隔离了、其实没有」的安全错觉
        assert!(!requested(&a(&["--safe=1"])));
        assert!(!requested(&a(&["--safely"])));
        assert!(!requested(&a(&["gateway"])));
    }

    #[test]
    fn root_is_unique_per_stamp_and_pid() {
        let base = PathBuf::from("D:\\tmp");
        let a = root_under(&base, 1000, 42);
        let b = root_under(&base, 1000, 43);
        let c = root_under(&base, 1001, 42);
        assert_ne!(a, b, "不同 pid 必须互不覆盖");
        assert_ne!(a, c, "不同时刻必须互不覆盖");
        assert!(a.starts_with(&base));
    }

    #[test]
    fn prepare_creates_three_dirs_and_marker() {
        let dir = scratch("prepare");
        let paths = prepare(dir.join("safe")).expect("prepare 应成功");
        assert!(paths.config_dir().is_dir(), "config 目录必须建立");
        assert!(paths.data_dir().is_dir(), "data 目录必须建立");
        assert!(paths.log_dir().is_dir(), "logs 目录必须建立");
        assert!(is_marked(&paths.root), "标记文件必须存在");
        // 三个目录必须互不相同：重合会导致配置与数据互相覆盖
        assert_ne!(paths.config_dir(), paths.data_dir());
        assert_ne!(paths.data_dir(), paths.log_dir());
        let _ = std::fs::remove_dir_all(&paths.root);
    }

    #[test]
    fn root_of_not_yet_prepared_place_is_unmarked() {
        // 未建目录时不得被误判为 Safe Mode 产物（清理逻辑据此保护用户目录）
        let dir = scratch("unmarked");
        std::fs::create_dir_all(&dir).unwrap();
        assert!(!is_marked(&dir));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
