//! 子进程静默执行助手（Windows）。
//!
//! GUI 宿主（`gui/src-tauri/src/main.rs` 的 `windows_subsystem = "windows"`）**没有控制台**。
//! 此时启动任何控制台子系统子进程（cmd / powershell / wsl / docker / fsutil / netstat /
//! git / ssh / node …），Windows 会为它**新建一个可见控制台窗口**——表现就是启动后不断闪
//! 黑窗（健康自检每 30s 跑一次 `wsl.exe` / `docker` / `fsutil`，尤为明显）。
//!
//! 统一走本模块的 [`SilentCommand::silent`] 给子进程加 `CREATE_NO_WINDOW`，从根上消除闪窗；
//! 非 Windows 平台是空操作，调用点无需写 `cfg`。

/// 让子进程在 Windows 上静默启动（不弹控制台窗口）。
pub trait SilentCommand {
    /// Windows：加 `CREATE_NO_WINDOW`；其它平台：无操作。
    /// 返回 `&mut Self` 以便在链式调用中内联使用。
    fn silent(&mut self) -> &mut Self;
}

impl SilentCommand for std::process::Command {
    fn silent(&mut self) -> &mut Self {
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            self.creation_flags(0x08000000);
        }
        self
    }
}

impl SilentCommand for tokio::process::Command {
    fn silent(&mut self) -> &mut Self {
        #[cfg(windows)]
        {
            // tokio 的 `Command` 在 Windows 上自带 `creation_flags`（无需 `CommandExt`）。
            self.creation_flags(0x08000000);
        }
        self
    }
}
