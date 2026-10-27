//! 「用系统默认程序打开」的安全入口（W3.2）。
//!
//! 旧实现直接把任意路径交给 `cmd /c start`：既能打开任意位置的可执行文件，
//! 也没有工作区边界。这里把两件事拆开：
//! 1. [`open_path_checked`] 做**纯校验**（存在性、控制字符、可执行扩展名、工作区包含）；
//! 2. [`launch_checked`] 只接收已经 canonicalize 的路径，Windows 走 `ShellExecuteW`
//!    （不经 shell，路径里的 `&`/`|` 不会再被当成命令分隔符）。

use std::path::{Path, PathBuf};

/// 默认拒绝的可执行/可触发扩展名（打开它们等于执行代码）。
const BLOCKED_EXTENSIONS: &[&str] = &[
    // 可执行/脚本/安装/快捷方式/注册表：打开即可能执行代码。
    "exe", "bat", "cmd", "com", "scr", "ps1", "vbs", "vbe", "js", "jse", "wsf", "ws", "msi",
    "lnk", "url", "reg", "hta", "cpl", "scf", "pif", "msc",
    // 常见脚本宿主（.py 在 Windows 上默认关联 python 执行；.jar 关联 Java）。
    "py", "pyw", "jar",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpenError {
    /// 空路径或含控制字符（会污染命令行/日志）。
    InvalidPath,
    /// canonicalize 失败（不存在 / 无权限）——如实按「找不到」处理。
    NotFound,
    /// 扩展名在拒绝清单内。
    ExecutableBlocked(&'static str),
    /// 未登记的裸路径落在了工作区根之外。
    EscapesWorkspace,
}

impl OpenError {
    /// 稳定的错误码（前端可识别；数值码与既有 RPC 信封兼容）。
    pub fn code(&self) -> u32 {
        match self {
            OpenError::InvalidPath => 1401,
            OpenError::NotFound => 1404,
            OpenError::ExecutableBlocked(_) | OpenError::EscapesWorkspace => 1403,
        }
    }

    pub fn message(&self) -> String {
        match self {
            OpenError::InvalidPath => "INVALID_PATH: 路径为空或包含控制字符".to_string(),
            OpenError::NotFound => "NOT_FOUND: 路径不存在或无权限访问".to_string(),
            OpenError::ExecutableBlocked(ext) => {
                format!("EXECUTABLE_BLOCKED: 不允许用系统默认程序打开 .{ext} 文件")
            }
            OpenError::EscapesWorkspace => {
                "PATH_NOT_ALLOWED_FOR_OPEN: 未登记的产物只能打开工作区根目录内的文件".to_string()
            }
        }
    }
}

/// 校验一个待打开的路径。
///
/// - `roots`：允许的根目录（`allow_outside = false` 时生效；空 roots = fail-closed）；
/// - `allow_outside`：已登记产物用 `true`（台账路径可能在临时目录），裸路径必须 `false`。
pub fn open_path_checked(
    path: &str,
    roots: &[PathBuf],
    allow_outside: bool,
) -> Result<PathBuf, OpenError> {
    let raw = path.trim();
    if raw.is_empty() || raw.chars().any(char::is_control) {
        return Err(OpenError::InvalidPath);
    }
    let canonical = std::fs::canonicalize(raw).map_err(|_| OpenError::NotFound)?;
    if let Some(ext) = canonical.extension().and_then(|e| e.to_str()) {
        let ext = ext.to_ascii_lowercase();
        if let Some(blocked) = BLOCKED_EXTENSIONS.iter().find(|candidate| **candidate == ext) {
            return Err(OpenError::ExecutableBlocked(blocked));
        }
    }
    if !allow_outside {
        let inside = roots
            .iter()
            .any(|root| crate::tools::fs_path::safe_resolve(root, raw).is_ok());
        if !inside {
            return Err(OpenError::EscapesWorkspace);
        }
    }
    Ok(canonical)
}

/// Windows `canonicalize` 返回扩展长度路径（`\\?\C:\...`）；ShellExecuteW 不接受，
/// 必须剥掉前缀再交给 shell（仓内 `agent/worktree.rs` 对同一问题已有先例）。
#[cfg(windows)]
fn strip_extended_prefix(path: &Path) -> PathBuf {
    let raw = path.to_string_lossy();
    if let Some(rest) = raw.strip_prefix(r"\\?\UNC\") {
        return PathBuf::from(format!(r"\\{rest}"));
    }
    if let Some(rest) = raw.strip_prefix(r"\\?\") {
        return PathBuf::from(rest);
    }
    path.to_path_buf()
}

/// 用系统默认程序打开一个**已经校验过**的路径。
///
/// Windows 走 `ShellExecuteW("open")`：不经过 cmd，路径中的 shell 元字符不具备特殊含义。
#[cfg(windows)]
pub fn launch_checked(path: &Path) -> bool {
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

    let launch_path = strip_extended_prefix(path);
    let verb: Vec<u16> = "open\0".encode_utf16().collect();
    let file: Vec<u16> = launch_path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    // SAFETY: 两个指针都指向以 NUL 结尾、在调用期间存活的 UTF-16 缓冲；
    // 返回值是 HINSTANCE 形式的成功/错误码，不保留也不释放任何资源。
    let result = unsafe {
        ShellExecuteW(
            None,
            PCWSTR(verb.as_ptr()),
            PCWSTR(file.as_ptr()),
            PCWSTR::null(),
            PCWSTR::null(),
            SW_SHOWNORMAL,
        )
    };
    result.0 as isize > 32
}

#[cfg(not(windows))]
pub fn launch_checked(path: &Path) -> bool {
    let cmd = if cfg!(target_os = "macos") { "open" } else { "xdg-open" };
    std::process::Command::new(cmd)
        .arg(path)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "marguerite-open-path-{tag}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp root");
        dir
    }

    fn write_file(dir: &Path, name: &str) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, b"probe").expect("write probe");
        path
    }

    #[test]
    fn allows_a_text_file_inside_a_workspace_root() {
        let root = temp_root("inside");
        let file = write_file(&root, "notes.txt");
        let resolved = open_path_checked(&file.display().to_string(), std::slice::from_ref(&root), false)
            .expect("工作区内的 txt 应允许");
        assert!(resolved.is_absolute());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn rejects_a_file_outside_the_workspace() {
        let root = temp_root("root");
        let elsewhere = temp_root("outside");
        let file = write_file(&elsewhere, "notes.txt");
        let err = open_path_checked(&file.display().to_string(), std::slice::from_ref(&root), false)
            .expect_err("工作区外必须拒绝");
        assert_eq!(err, OpenError::EscapesWorkspace);
        assert_eq!(err.code(), 1403);
        let _ = std::fs::remove_dir_all(root);
        let _ = std::fs::remove_dir_all(elsewhere);
    }

    #[test]
    fn registered_artifacts_may_live_outside_the_workspace() {
        let elsewhere = temp_root("registered");
        let file = write_file(&elsewhere, "report.csv");
        assert!(
            open_path_checked(&file.display().to_string(), &[], true).is_ok(),
            "已登记产物允许在工作区外（allow_outside=true）"
        );
        let _ = std::fs::remove_dir_all(elsewhere);
    }

    #[test]
    fn rejects_executables_even_inside_the_workspace() {
        let root = temp_root("exe");
        let file = write_file(&root, "setup.exe");
        let err = open_path_checked(&file.display().to_string(), std::slice::from_ref(&root), true)
            .expect_err("可执行扩展名必须拒绝");
        assert_eq!(err, OpenError::ExecutableBlocked("exe"));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn rejects_control_characters_but_allows_shell_metacharacters_in_names() {
        assert_eq!(
            open_path_checked("bad\u{0}name.txt", &[], true),
            Err(OpenError::InvalidPath)
        );
        // `&` 在 Windows 文件名字面量里合法：我们用 ShellExecuteW，不经 shell，
        // 所以它不能被当成注入特征（旧实现的全文匹配会误杀）。
        let root = temp_root("meta");
        let file = write_file(&root, "a&b notes.txt");
        assert!(open_path_checked(&file.display().to_string(), std::slice::from_ref(&root), false).is_ok());
        let _ = std::fs::remove_dir_all(root);
    }

    #[cfg(windows)]
    #[test]
    fn extended_length_prefix_is_stripped_before_shellexecute() {
        assert_eq!(
            strip_extended_prefix(Path::new(r"\\?\C:\work\a.txt")),
            PathBuf::from(r"C:\work\a.txt")
        );
        assert_eq!(
            strip_extended_prefix(Path::new(r"\\?\UNC\server\share\a.txt")),
            PathBuf::from(r"\\server\share\a.txt")
        );
        assert_eq!(
            strip_extended_prefix(Path::new(r"C:\work\a.txt")),
            PathBuf::from(r"C:\work\a.txt")
        );
    }

    #[test]
    fn script_hosts_are_blocked_too() {
        let root = temp_root("script");
        for name in ["deploy.py", "tool.jar", "page.hta"] {
            let file = write_file(&root, name);
            let err = open_path_checked(&file.display().to_string(), std::slice::from_ref(&root), true)
                .expect_err(name);
            assert!(
                matches!(err, OpenError::ExecutableBlocked(_)),
                "{name}: {err:?}"
            );
        }
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn missing_paths_are_not_found() {
        let err = open_path_checked("definitely-not-here-12345.txt", &[], true)
            .expect_err("不存在必须拒绝");
        assert_eq!(err, OpenError::NotFound);
    }
}
