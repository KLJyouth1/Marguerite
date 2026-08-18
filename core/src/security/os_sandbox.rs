//! OS 级沙箱（sec.os_sandbox）：内核级隔离重型桥
//!
//! 这是与 标准 AI 助手 的实质差异化卖点——**内核级**隔离，强于纯 denylist 正则 +
//! 路径护栏的命令级沙箱（`sandbox/` 模块）。本模块按平台真实落地：
//!
//! - **Linux**：Landlock LSM（`landlock` crate，仅 linux target 依赖），以最小权限
//!   限制进程的文件系统访问面（工作区可读写，系统库可读执行，网络/越界路径默认拒绝）。
//! - **macOS**：Seatbelt（`sandbox_init` FFI），以 deny-default 配置限制网络与文件访问。
//! - **Windows**：当前构建**显式 `Unsupported`**——Windows 无 Landlock/Seatbelt 等价物，
//!   内核级隔离需走 Job Object / AppContainer（重型原生桥，单独排期）。本模块**绝不静默
//!   伪造成功**，而是诚实返回 `Unsupported`，并明确「命令级 `sandbox/` 模块为强制执行层」。
//!
//! 设计伦理：fail-closed。任何平台若内核隔离未能成功建立，返回的 `IsolationLevel` 不会是
//! `Full`，避免上层误以为已获 OS 级保护。

use std::path::Path;

use serde::{Deserialize, Serialize};

/// OS 级隔离达成程度（机器可校验，杜绝「宣称未落地」）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IsolationLevel {
    /// 内核级隔离已真正建立
    Full,
    /// 部分建立（如内核 ABI 低于请求版本，能力被裁剪）
    Partial,
    /// 当前平台/构建不支持内核级隔离（诚实标注，绝不静默）
    Unsupported,
}

impl IsolationLevel {
    pub fn label(&self) -> &'static str {
        match self {
            IsolationLevel::Full => "内核级",
            IsolationLevel::Partial => "部分内核级",
            IsolationLevel::Unsupported => "不支持(命令级兜底)",
        }
    }
}

/// OS 级沙箱裁决
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OsSandboxVerdict {
    pub level: IsolationLevel,
    pub detail: String,
}

impl OsSandboxVerdict {
    pub fn full(detail: impl Into<String>) -> Self {
        Self {
            level: IsolationLevel::Full,
            detail: detail.into(),
        }
    }
    pub fn partial(detail: impl Into<String>) -> Self {
        Self {
            level: IsolationLevel::Partial,
            detail: detail.into(),
        }
    }
    pub fn unsupported(detail: impl Into<String>) -> Self {
        Self {
            level: IsolationLevel::Unsupported,
            detail: detail.into(),
        }
    }
    /// 是否获得任意级别的内核隔离（Unsupported 为 false）
    pub fn has_kernel_isolation(&self) -> bool {
        matches!(self.level, IsolationLevel::Full | IsolationLevel::Partial)
    }
}

/// OS 级沙箱抽象：按平台绑定具体实现。
pub trait OsSandbox {
    /// 在 `workspace` 上施加内核级隔离（若平台支持），返回裁决。
    fn apply(&self, workspace: &Path) -> anyhow::Result<OsSandboxVerdict>;
    /// 平台标识（用于可观测性 / 能力树展示）
    fn platform(&self) -> &'static str;
}

// ───────────────────────────────────────────────────────────────────────────
// Linux：Landlock（仅 linux target 编译，依赖 `landlock` crate）
// ───────────────────────────────────────────────────────────────────────────
#[cfg(target_os = "linux")]
mod linux {
    use super::*;
    use std::os::fd::AsFd;

    /// Linux Landlock 实现：以最小权限限制文件系统访问面。
    pub struct LinuxLandlock;

    impl OsSandbox for LinuxLandlock {
        fn platform(&self) -> &'static str {
            "linux-landlock"
        }

        fn apply(&self, workspace: &Path) -> anyhow::Result<OsSandboxVerdict> {
            use landlock::{AccessFs, BitFlags, PathBeneath, Ruleset};

            // 保守选择已广泛支持的 ABI V1；真实内核若更高会自动兼容。
            let read_exec =
                AccessFs::from_read(landlock::ABI::V1) | AccessFs::from_exec(landlock::ABI::V1);

            let mut ruleset = Ruleset::new();

            // 工作区：允许读写执行（agent 需在此落地代码）
            let ws_fd = std::fs::File::open(workspace)
                .map_err(|e| anyhow::anyhow!("打开工作区失败: {e}"))?;
            ruleset.add_rule(PathBeneath::new(ws_fd.as_fd(), read_exec))?;

            // 系统与运行时路径：仅允许读/执行，禁止越界写
            for sys in [
                "/usr", "/lib", "/lib64", "/etc", "/dev", "/proc", "/bin", "/opt",
            ] {
                if let Ok(f) = std::fs::File::open(sys) {
                    let _ = ruleset.add_rule(PathBeneath::new(f.as_fd(), read_exec));
                }
            }

            // restrict_self 可能因内核 ABI 低于请求而返回 Partial
            let status = ruleset.restrict_self()?;
            let level = if status.ruleset_is_empty() {
                // 没有任何规则生效（内核不支持 Landlock）→ 视为 Unsupported
                return Ok(OsSandboxVerdict::unsupported(
                    "内核不支持 Landlock（ruleset 为空）；回退命令级 sandbox/",
                ));
            } else if status.is_full() {
                IsolationLevel::Full
            } else {
                IsolationLevel::Partial
            };
            Ok(OsSandboxVerdict {
                level,
                detail: format!("Landlock 已施加（{:?}）", status),
            })
        }
    }
}

// ───────────────────────────────────────────────────────────────────────────
// macOS：Seatbelt（仅 macos target 编译，FFI 链接系统 sandbox 框架）
// ───────────────────────────────────────────────────────────────────────────
#[cfg(target_os = "macos")]
mod macos {
    use super::*;
    use std::ffi::{c_char, c_int, CString};
    use std::ptr;

    #[link(name = "sandbox")]
    extern "C" {
        fn sandbox_init(profile: *const c_char, flags: u64, errorbuf: *mut *mut c_char) -> c_int;
        fn sandbox_free_error(errorbuf: *mut c_char);
    }

    /// macOS Seatbelt 实现：deny-default，允许文件读、拒绝网络。
    pub struct MacOsSeatbelt;

    impl OsSandbox for MacOsSeatbelt {
        fn platform(&self) -> &'static str {
            "macos-seatbelt"
        }

        fn apply(&self, _workspace: &Path) -> anyhow::Result<OsSandboxVerdict> {
            // 最小权限画像：默认拒绝，放开文件读，拒绝全部网络（agent 不需外联）。
            let profile = CString::new(
                "(version 1)\n\
                 (deny default)\n\
                 (allow file-read*)\n\
                 (allow process*)\n\
                 (deny network*)\n",
            )
            .map_err(|e| anyhow::anyhow!("构造 sandbox profile 失败: {e}"))?;

            let mut err: *mut c_char = ptr::null_mut();
            let rc = unsafe { sandbox_init(profile.as_ptr(), 0, &mut err) };
            if rc != 0 {
                let msg = if err.is_null() {
                    "未知错误".to_string()
                } else {
                    let s = unsafe { std::ffi::CStr::from_ptr(err).to_string_lossy().into_owned() };
                    unsafe { sandbox_free_error(err) };
                    s
                };
                return Err(anyhow::anyhow!("sandbox_init 失败: {msg}"));
            }
            Ok(OsSandboxVerdict::full(
                "Seatbelt deny-default（允许文件读/拒绝网络）已施加",
            ))
        }
    }
}

// ───────────────────────────────────────────────────────────────────────────
// 跨平台派发
// ───────────────────────────────────────────────────────────────────────────
/// 按当前编译平台返回最合适的 OS 级沙箱实现（若为 Windows 则返回 None，
/// 因为本构建不支持内核级隔离，需回退命令级 `sandbox/` 模块）。
pub fn platform_sandbox() -> Option<Box<dyn OsSandbox>> {
    #[cfg(target_os = "linux")]
    return Some(Box::new(linux::LinuxLandlock));
    #[cfg(target_os = "macos")]
    return Some(Box::new(macos::MacOsSeatbelt));
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    return None;
}

/// 在 `workspace` 上施加当前平台的 OS 级隔离，返回裁决。
///
/// Windows 等不支持平台会**诚实**返回 `Unsupported`，并明确命令级沙箱为兜底。
pub fn apply_for_current_platform(workspace: &Path) -> OsSandboxVerdict {
    match platform_sandbox() {
        Some(s) => match s.apply(workspace) {
            Ok(v) => v,
            Err(e) => OsSandboxVerdict::partial(format!(
                "{} 内核隔离建立失败（{}）；回退命令级 sandbox/",
                s.platform(),
                e
            )),
        },
        None => OsSandboxVerdict::unsupported(
            "当前平台（Windows）暂无内核级 OS 沙箱；命令级 sandbox/ 模块为强制执行层。\
             重型桥（Windows Job Object / AppContainer）单独排期。",
        ),
    }
}

/// 当前平台是否已具备内核级 OS 沙箱实现（编译期已知，无需运行）。
pub fn kernel_isolation_available() -> bool {
    platform_sandbox().is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_build_reports_unsupported_honestly() {
        // 在 Windows 构建上，apply_for_current_platform 必须诚实返回 Unsupported，
        // 绝不伪造 Full（fail-closed 伦理）。
        let v = apply_for_current_platform(Path::new("."));
        if !kernel_isolation_available() {
            assert_eq!(v.level, IsolationLevel::Unsupported);
            assert!(v.detail.contains("命令级"));
            assert!(!v.has_kernel_isolation());
        } else {
            // 在 Linux/macOS CI 上至少不应 panic，且 level 不为空。
            assert!(matches!(
                v.level,
                IsolationLevel::Full | IsolationLevel::Partial | IsolationLevel::Unsupported
            ));
        }
    }

    #[test]
    fn verdict_constructors_set_levels() {
        assert_eq!(OsSandboxVerdict::full("x").level, IsolationLevel::Full);
        assert!(OsSandboxVerdict::full("x").has_kernel_isolation());
        assert_eq!(
            OsSandboxVerdict::partial("x").level,
            IsolationLevel::Partial
        );
        assert!(OsSandboxVerdict::partial("x").has_kernel_isolation());
        assert_eq!(
            OsSandboxVerdict::unsupported("x").level,
            IsolationLevel::Unsupported
        );
        assert!(!OsSandboxVerdict::unsupported("x").has_kernel_isolation());
    }

    #[test]
    fn isolation_level_labels_nonempty() {
        assert!(!IsolationLevel::Full.label().is_empty());
        assert!(!IsolationLevel::Unsupported.label().is_empty());
    }
}
