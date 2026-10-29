//! 侧车进程内存围栏（P1-5）。
//!
//! - **Windows**：把侧车进程放进一个 Job Object，设 `JOB_OBJECT_LIMIT_JOB_MEMORY`
//!   （外加 `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`），进程提交内存超过上限时由系统
//!   终止——正好走既有崩溃取证 / 受控重启路径。Job Object 句柄必须在进程存活期间
//!   保持打开（句柄随 [`MemoryFence`] 存放）。
//! - **Linux**：spawn 前在 `pre_exec` 里 `setrlimit(RLIMIT_AS)`（虚拟地址空间上限）。
//!
//! 阈值 env：`MARGUERITE_SIDECAR_MAX_MEM_MB`（默认 4096 MiB；`0` = 不设围栏）。
//! 纯函数 [`parse_sidecar_max_mem_mb`] 解析，便于单测与对账。
//!
//! 设计依据：docs/sidecar-stability-research.md §3.2 P1-5。

use anyhow::{anyhow, Context, Result};

/// 默认内存上限（MiB）。保守取值：覆盖 embedding 索引 / 正常多任务，
/// 仍能拦住失控泄漏（本机真实压测见报告）。
pub(crate) const DEFAULT_MAX_MEM_MB: u64 = 4096;

/// 解析侧车内存上限。
///
/// 返回 `None` 表示**不设围栏**（仅当显式给 `0`）；否则返回 MiB 上限。
/// - `None` / 空串：默认 [`DEFAULT_MAX_MEM_MB`]；
/// - `"0"`：`None`（关闭）；
/// - 合法正整数：该值；
/// - 非法（非数字）：回退默认值（配置错误时 fail-safe 到有围栏，而不是裸奔）。
pub(crate) fn parse_sidecar_max_mem_mb(raw: Option<&str>) -> Option<u64> {
    let s = match raw {
        Some(s) => s.trim(),
        None => return Some(DEFAULT_MAX_MEM_MB),
    };
    if s.is_empty() {
        return Some(DEFAULT_MAX_MEM_MB);
    }
    match s.parse::<u64>() {
        Ok(0) => None,
        Ok(n) => Some(n),
        Err(_) => Some(DEFAULT_MAX_MEM_MB),
    }
}

/// 读取当前进程环境的上限（便捷封装）。
pub(crate) fn limit_mb_from_env() -> Option<u64> {
    parse_sidecar_max_mem_mb(std::env::var("MARGUERITE_SIDECAR_MAX_MEM_MB").ok().as_deref())
}

/// MiB → 字节（用 MiB = 1024*1024，与任务/系统口径一致）。
pub(crate) fn mb_to_bytes(mb: u64) -> u64 {
    mb.saturating_mul(1024).saturating_mul(1024)
}

// =========================================================================
// Windows: Job Object
// =========================================================================
#[cfg(windows)]
mod imp {
    use super::*;
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, SetInformationJobObject,
        JOBOBJECT_BASIC_LIMIT_INFORMATION, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JOB_OBJECT_LIMIT_JOB_MEMORY,
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE, JobObjectExtendedLimitInformation,
    };

    /// 持有 Job Object 句柄；drop 时关闭（围栏随之结束）。
    pub(crate) struct MemoryFence {
        job: HANDLE,
    }

    // SAFETY: HANDLE 是一个进程级内核句柄，可在任意线程关闭；本结构只在
    // 单一线性所有权内移动（spawn 后创建、随 PyEngineSidecar 重启时替换），
    // 无并发访问。实现 Send 以让其可跨 await 存放。
    unsafe impl Send for MemoryFence {}

    impl MemoryFence {
        /// 对刚 spawn 的侧车进程套用 Job Object 内存围栏。
        ///
        /// `raw_process_handle` 为 tokio [`tokio::process::Child::raw_handle`]
        /// 返回的 Windows 进程句柄（裸 `HANDLE` 值）。
        pub(crate) fn apply_to_process(
            raw_process_handle: isize,
            limit_bytes: u64,
        ) -> Result<Self> {
            // 1. 创建（匿名）Job Object。
            // SAFETY: 安全属性 None（默认）、名称为 null PCWSTR（匿名 Job），
            // 返回值经 `windows` crate 封装为 Result，失败即上抛。
            let job = unsafe { CreateJobObjectW(None, PCWSTR::null()) }
                .context("CreateJobObjectW 失败")?;

            // 2. 装配扩展限制：JobMemoryLimit + LimitFlags（一次字面量初始化，
            // 避免 field_reassign_with_default）。
            let info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION {
                JobMemoryLimit: limit_bytes as usize,
                BasicLimitInformation: JOBOBJECT_BASIC_LIMIT_INFORMATION {
                    LimitFlags: JOB_OBJECT_LIMIT_JOB_MEMORY | JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
                    ..Default::default()
                },
                ..Default::default()
            };

            // 3. 设置限制。
            // SAFETY: `info` 在调用期间有效，指针/大小与结构匹配。
            unsafe {
                SetInformationJobObject(
                    job,
                    JobObjectExtendedLimitInformation,
                    &info as *const _ as *const core::ffi::c_void,
                    std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
                )
            }
            .context("SetInformationJobObject 失败")?;

            // 4. 把侧车进程分配进 Job。
            let process = HANDLE(raw_process_handle as *mut core::ffi::c_void);
            // SAFETY: 进程句柄来自 tokio 刚 spawn 的子进程，调用期间仍有效。
            unsafe { AssignProcessToJobObject(job, process) }
                .context("AssignProcessToJobObject 失败")?;

            Ok(Self { job })
        }
    }

    impl Drop for MemoryFence {
        fn drop(&mut self) {
            // SAFETY: 仅关闭一次自有句柄；KILL_ON_JOB_CLOSE 会让仍在 Job 内的
            // 进程随之终止（避免孤儿侧车）。
            unsafe {
                let _ = windows::Win32::Foundation::CloseHandle(self.job);
            }
        }
    }
}

// =========================================================================
// 非 Windows：占位（Linux 用 RLIMIT，见下）
// =========================================================================
#[cfg(not(windows))]
mod imp {
    /// 非 Windows 不需要长期持有句柄；占位空结构。
    pub(crate) struct MemoryFence;
}

pub(crate) use imp::MemoryFence;

// =========================================================================
// Linux: RLIMIT_AS（在 spawn 前对 Command 套用）
// =========================================================================
#[cfg(unix)]
pub(crate) fn apply_rlimit_as(cmd: &mut tokio::process::Command, limit_bytes: u64) -> Result<()> {
    // 不要在这里 `use std::os::unix::process::CommandExt;`：`pre_exec` 是 tokio
    // `Command` 的**固有方法**，那个 trait 导入在 Linux/macOS 上恒为 unused，
    // 而 CI 的 `clippy -D warnings` 会把它升级成错误（0.2.18 的 Linux 构建里就是一条 warning）。
    // 不引入 libc 依赖：手工声明 setrlimit（POSIX 稳定符号）。
    // RLIMIT_AS 在 Linux = 9。
    #[cfg(target_os = "linux")]
    const RLIMIT_AS: u32 = 9;
    #[cfg(not(target_os = "linux"))]
    const RLIMIT_AS: u32 = 10; // *BSD/macOS 的 RLIMIT_AS 数值
    #[repr(C)]
    struct Rlimit {
        cur: u64,
        max: u64,
    }
    extern "C" {
        fn setrlimit(resource: u32, rlim: *const Rlimit) -> i32;
    }
    // SAFETY: pre_exec 内只做一个 async-signal-safe 的 setrlimit 调用。
    unsafe {
        cmd.pre_exec(move || {
            let lim = Rlimit {
                cur: limit_bytes,
                max: limit_bytes,
            };
            if setrlimit(RLIMIT_AS, &lim) == 0 {
                Ok(())
            } else {
                Err(std::io::Error::last_os_error())
            }
        });
    }
    Ok(())
}

/// 便捷封装：在 spawn 前对命令套用（Linux 用 RLIMIT；Windows 无需预操作）。
#[cfg(unix)]
pub(crate) fn prepare_command(cmd: &mut tokio::process::Command, limit_bytes: u64) -> Result<()> {
    apply_rlimit_as(cmd, limit_bytes)
}

#[cfg(windows)]
pub(crate) fn prepare_command(_cmd: &mut tokio::process::Command, _limit_bytes: u64) -> Result<()> {
    Ok(())
}

/// Windows：spawn 后对 tokio 子进程套用 Job Object 围栏。
#[cfg(windows)]
pub(crate) fn fence_after_spawn(
    raw_process_handle: isize,
    limit_bytes: u64,
) -> Result<MemoryFence> {
    MemoryFence::apply_to_process(raw_process_handle, limit_bytes)
        .map_err(|e| anyhow!("侧车内存围栏套用失败: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_sidecar_max_mem_mb_table() {
        // 默认
        assert_eq!(parse_sidecar_max_mem_mb(None), Some(4096));
        assert_eq!(parse_sidecar_max_mem_mb(Some("   ")), Some(4096));
        // 0 = 关闭
        assert_eq!(parse_sidecar_max_mem_mb(Some("0")), None);
        // 正常值
        assert_eq!(parse_sidecar_max_mem_mb(Some("256")), Some(256));
        assert_eq!(parse_sidecar_max_mem_mb(Some(" 1024 ")), Some(1024));
        // 非法 → 默认（fail-safe）
        assert_eq!(parse_sidecar_max_mem_mb(Some("abc")), Some(4096));
        assert_eq!(parse_sidecar_max_mem_mb(Some("-5")), Some(4096));
    }

    #[test]
    fn mb_to_bytes_is_mebibyte() {
        assert_eq!(mb_to_bytes(1), 1024 * 1024);
        assert_eq!(mb_to_bytes(4), 4 * 1024 * 1024);
    }
}
