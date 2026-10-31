//! Shell 发现与画像（Linux / macOS / Windows 统一口径）
//!
//! # 为什么需要
//!
//! 旧实现把 shell 写死成 `cmd.exe`（Windows）与 `$SHELL`（Unix）：
//! - Windows 用户装了 PowerShell 7（`pwsh`）也永远进 `cmd`：没有 `&&`、没有现代补全、
//!   UTF-8 要靠 `chcp` 手工补；
//! - Git Bash / WSL 这类「用户已经装好并天天用」的 shell 完全不可选；
//! - 没有可用性探测，选错时只会在 spawn 阶段报一句「终端启动失败」，无法定位。
//!
//! 本模块只做**发现 + 画像**，不执行任何命令：把「机器上到底有哪些 shell、
//! 默认该用哪个、每个 shell 的启动参数是什么」收敛到一处，供 [`super::pty`]
//! （PTY 会话）与 RPC 视图共同消费。
//!
//! # 设计约束
//!
//! - **纯函数可测**：`discover_from(probe)` 不读真实环境，单测可注入假 PATH / 文件存在性；
//! - **不猜**：只有路径命中或 PATH 命中才标 `available`；
//! - **顺序稳定**：默认 shell 的优先级固定（pwsh > powershell > cmd > git-bash > wsl），
//!   不依赖哈希迭代顺序，避免同一台机器两次启动选到不同 shell。

use std::path::{Path, PathBuf};

/// 平台（编译期决定，测试可注入）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    Windows,
    Unix,
}

pub fn current_platform() -> Platform {
    if cfg!(windows) {
        Platform::Windows
    } else {
        Platform::Unix
    }
}

/// 已知 shell 种类。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShellKind {
    /// PowerShell 7+（跨平台，`pwsh`）
    Pwsh,
    /// Windows PowerShell 5.1（`powershell.exe`，仅 Windows）
    WindowsPowerShell,
    /// `cmd.exe`（仅 Windows）
    Cmd,
    /// Git for Windows 自带的 `bash.exe`
    GitBash,
    /// 通用 bash
    Bash,
    /// zsh
    Zsh,
    /// fish
    Fish,
    /// POSIX sh
    Sh,
    /// WSL 默认发行版（`wsl.exe`，仅 Windows）
    Wsl,
}

impl ShellKind {
    /// 稳定 id（RPC / 前端存储用，不随展示文案变化）。
    pub fn id(self) -> &'static str {
        match self {
            ShellKind::Pwsh => "pwsh",
            ShellKind::WindowsPowerShell => "powershell",
            ShellKind::Cmd => "cmd",
            ShellKind::GitBash => "git-bash",
            ShellKind::Bash => "bash",
            ShellKind::Zsh => "zsh",
            ShellKind::Fish => "fish",
            ShellKind::Sh => "sh",
            ShellKind::Wsl => "wsl",
        }
    }

    /// 展示名（前端下拉框用；带版本语义，避免用户分不清两个 PowerShell）。
    pub fn label(self) -> &'static str {
        match self {
            ShellKind::Pwsh => "PowerShell 7 (pwsh)",
            ShellKind::WindowsPowerShell => "Windows PowerShell 5.1",
            ShellKind::Cmd => "Command Prompt (cmd)",
            ShellKind::GitBash => "Git Bash",
            ShellKind::Bash => "Bash",
            ShellKind::Zsh => "Zsh",
            ShellKind::Fish => "Fish",
            ShellKind::Sh => "POSIX sh",
            ShellKind::Wsl => "WSL",
        }
    }

    /// 解析 id；同时接受常见别名（前端手输 / 旧配置兼容）。
    pub fn from_id(raw: &str) -> Option<Self> {
        Some(match raw.trim().to_ascii_lowercase().as_str() {
            "pwsh" | "powershell7" | "ps7" | "pwsh.exe" => ShellKind::Pwsh,
            "powershell" | "powershell.exe" | "windows-powershell" | "ps" | "ps5" => {
                ShellKind::WindowsPowerShell
            }
            "cmd" | "cmd.exe" | "command-prompt" => ShellKind::Cmd,
            "git-bash" | "gitbash" | "git_bash" => ShellKind::GitBash,
            "bash" => ShellKind::Bash,
            "zsh" => ShellKind::Zsh,
            "fish" => ShellKind::Fish,
            "sh" => ShellKind::Sh,
            "wsl" | "wsl2" => ShellKind::Wsl,
            _ => return None,
        })
    }

    /// 该 shell 的默认启动参数。
    ///
    /// 刻意保留用户 profile：终端是用户自己的交互环境，跳过 `.bashrc` / `$PROFILE`
    /// 会让 `nvm` / `conda` / 自定义 PATH 全部失效（这是「终端里找不到 node」的常见根因）。
    pub fn default_args(self) -> Vec<String> {
        match self {
            // `-NoLogo` 只去掉版权横幅，不跳过 profile
            ShellKind::Pwsh | ShellKind::WindowsPowerShell => vec!["-NoLogo".to_string()],
            ShellKind::Cmd => Vec::new(),
            ShellKind::GitBash => vec!["--login".to_string(), "-i".to_string()],
            ShellKind::Bash | ShellKind::Zsh | ShellKind::Fish | ShellKind::Sh => {
                vec!["-i".to_string()]
            }
            ShellKind::Wsl => Vec::new(),
        }
    }

    /// 该 shell 可执行文件的主名（用于 `$SHELL` 同类判定）。
    pub fn stem(self) -> &'static str {
        match self {
            ShellKind::Pwsh => "pwsh",
            ShellKind::WindowsPowerShell => "powershell",
            ShellKind::Cmd => "cmd",
            ShellKind::GitBash | ShellKind::Bash => "bash",
            ShellKind::Zsh => "zsh",
            ShellKind::Fish => "fish",
            ShellKind::Sh => "sh",
            ShellKind::Wsl => "wsl",
        }
    }

    pub fn is_windows_only(self) -> bool {
        matches!(
            self,
            ShellKind::WindowsPowerShell | ShellKind::Cmd | ShellKind::GitBash | ShellKind::Wsl
        )
    }
}

/// 一个可用于 spawn 的 shell 画像。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShellProfile {
    pub kind: ShellKind,
    /// 真实可执行文件路径（PATH 命中时为绝对路径，避免二次 PATH 解析歧义）。
    pub program: String,
    pub args: Vec<String>,
    pub available: bool,
    /// 发现来源：`env:ComSpec` / `known:C:\...` / `path:C:\Windows` —— 用于诊断「为什么选了这个」。
    pub source: String,
}

impl ShellProfile {
    pub fn id(&self) -> &'static str {
        self.kind.id()
    }

    pub fn label(&self) -> &'static str {
        self.kind.label()
    }

    /// 供前端展示的一行摘要。
    pub fn summary(&self) -> String {
        format!("{} · {}", self.label(), self.program)
    }
}

/// 环境探针（可注入，便于离线单测）。
pub struct Probe<'a> {
    pub platform: Platform,
    pub env: &'a dyn Fn(&str) -> Option<String>,
    pub exists: &'a dyn Fn(&Path) -> bool,
}

/// 真实环境发现（不做进程级缓存：shell 可能在会话期间被安装，缓存会永久说谎）。
pub fn discover() -> Vec<ShellProfile> {
    discover_from(&Probe {
        platform: current_platform(),
        env: &|k| std::env::var(k).ok(),
        exists: &|p| p.is_file(),
    })
}

/// 按平台列出候选（顺序即默认优先级）。
fn candidates(platform: Platform) -> Vec<(ShellKind, Vec<&'static str>)> {
    match platform {
        Platform::Windows => vec![
            (
                ShellKind::Pwsh,
                vec![
                    "pwsh.exe",
                    "%ProgramFiles%\\PowerShell\\7\\pwsh.exe",
                    "%ProgramFiles%\\PowerShell\\7-preview\\pwsh.exe",
                    "%LOCALAPPDATA%\\Microsoft\\WindowsApps\\pwsh.exe",
                ],
            ),
            (
                ShellKind::WindowsPowerShell,
                vec![
                    "%SystemRoot%\\System32\\WindowsPowerShell\\v1.0\\powershell.exe",
                    "powershell.exe",
                ],
            ),
            (
                ShellKind::Cmd,
                vec!["%ComSpec%", "%SystemRoot%\\System32\\cmd.exe", "cmd.exe"],
            ),
            (
                ShellKind::GitBash,
                vec![
                    "%ProgramFiles%\\Git\\bin\\bash.exe",
                    "%ProgramFiles(x86)%\\Git\\bin\\bash.exe",
                    "%LOCALAPPDATA%\\Programs\\Git\\bin\\bash.exe",
                ],
            ),
            (
                ShellKind::Wsl,
                vec!["%SystemRoot%\\System32\\wsl.exe", "wsl.exe"],
            ),
        ],
        Platform::Unix => vec![
            (
                ShellKind::Bash,
                vec!["%SHELL%", "/bin/bash", "/usr/bin/bash", "bash"],
            ),
            (
                ShellKind::Zsh,
                vec!["%SHELL%", "/bin/zsh", "/usr/bin/zsh", "zsh"],
            ),
            (
                ShellKind::Fish,
                vec!["%SHELL%", "/usr/bin/fish", "/usr/local/bin/fish", "fish"],
            ),
            (ShellKind::Sh, vec!["%SHELL%", "/bin/sh", "/usr/bin/sh", "sh"]),
        ],
    }
}

/// 纯发现实现：先按平台列候选，再逐个做「环境变量 → 已知路径 → PATH」解析。
pub fn discover_from(probe: &Probe<'_>) -> Vec<ShellProfile> {
    let mut out: Vec<ShellProfile> = Vec::new();
    for (kind, patterns) in candidates(probe.platform) {
        let hit = patterns
            .iter()
            .find_map(|raw| resolve_candidate(kind, raw, probe));
        match hit {
            Some((program, source)) => out.push(ShellProfile {
                kind,
                program,
                args: kind.default_args(),
                available: true,
                source,
            }),
            None => out.push(ShellProfile {
                kind,
                // 展示展开后的期望路径（`%LOCALAPPDATA%\...` 这种写法对用户没有信息量）
                program: patterns
                    .last()
                    .map(|s| expand_windows_env(s, probe))
                    .unwrap_or_default(),
                args: kind.default_args(),
                available: false,
                source: "not-found".to_string(),
            }),
        }
    }
    out
}

/// 默认 shell：候选顺序里的第一个可用项。
pub fn default_profile(list: &[ShellProfile]) -> Option<&ShellProfile> {
    list.iter().find(|p| p.available)
}

/// 按 id 找画像（大小写不敏感；未知 id 返回 None）。
pub fn find_profile<'a>(list: &'a [ShellProfile], id: &str) -> Option<&'a ShellProfile> {
    let kind = ShellKind::from_id(id)?;
    list.iter().find(|p| p.kind == kind)
}

/// 按**目标平台**判断绝对路径，而不是按当前宿主。
///
/// `Path::is_absolute()` 看的是宿主语义：在 Linux 上 `C:\Windows\...` 会被当成
/// 一个普通文件名，于是「Windows 候选路径」直接掉进 PATH 搜索分支，结论从
/// 「命中已知路径」变成「找不到」—— 同一份发现逻辑在两套 CI 上给出两套结果。
/// 探测的是**哪个平台**的 shell，就按哪个平台的绝对路径规则来判。
fn is_absolute_for(platform: Platform, path: &Path) -> bool {
    if path.is_absolute() {
        return true;
    }
    if platform != Platform::Windows {
        return false;
    }
    let s = path.to_string_lossy();
    let b = s.as_bytes();
    let drive_absolute =
        b.len() >= 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && (b[2] == b'\\' || b[2] == b'/');
    drive_absolute || s.starts_with("\\\\")
}

/// 解析一条候选：`%VAR%` 展开 → 绝对路径直查 → 裸名走 PATH。
fn resolve_candidate(
    kind: ShellKind,
    raw: &str,
    probe: &Probe<'_>,
) -> Option<(String, String)> {
    if let Some(var) = raw.strip_prefix('%').and_then(|r| r.strip_suffix('%')) {
        let value = (probe.env)(var)?;
        if value.trim().is_empty() {
            return None;
        }
        // `$SHELL` 可能指向任意 shell：只有 basename 与候选同类才算命中，
        // 否则第一个 `%SHELL%` 候选会把 zsh 误报成 bash。
        if var.eq_ignore_ascii_case("SHELL") && shell_stem_of(&value) != kind.stem() {
            return None;
        }
        let path = PathBuf::from(&value);
        if (probe.exists)(&path) {
            return Some((value, format!("env:{var}")));
        }
        return None;
    }
    let expanded = expand_windows_env(raw, probe);
    let path = Path::new(&expanded);
    if is_absolute_for(probe.platform, path) {
        return (probe.exists)(path).then(|| (expanded.clone(), format!("known:{expanded}")));
    }
    // 裸程序名：Windows 补 PATHEXT，Unix 直接查。
    let suffixes: &[&str] = if probe.platform == Platform::Windows {
        &["", ".exe", ".cmd", ".bat"]
    } else {
        &[""]
    };
    let path_var = (probe.env)("PATH").unwrap_or_default();
    let sep = if probe.platform == Platform::Windows {
        ';'
    } else {
        ':'
    };
    for dir in path_var.split(sep).filter(|d| !d.trim().is_empty()) {
        // 用**目标平台**的分隔符拼字符串，不要用 `Path::join`：后者在 Linux 上会拼成
        // `C:\Tools/pwsh.exe`（正斜杠），既不符合 Windows 的真实路径形状，也让
        // 「拼出来的 program 字符串」随宿主漂移（同一条断言在两套 CI 上不一样）。
        let sep_char = if probe.platform == Platform::Windows {
            '\\'
        } else {
            '/'
        };
        let dir_trimmed = dir.trim_end_matches(['\\', '/']);
        for suffix in suffixes {
            let candidate = format!("{dir_trimmed}{sep_char}{expanded}{suffix}");
            if (probe.exists)(Path::new(&candidate)) {
                return Some((candidate, format!("path:{dir_trimmed}")));
            }
        }
    }
    None
}

/// 取可执行文件主名（去掉目录与 `.exe`，小写）。
fn shell_stem_of(value: &str) -> String {
    Path::new(value)
        .file_stem()
        .map(|s| s.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default()
}

/// 展开候选里的 `%VAR%`（仅 Windows 风格；Unix 候选只有 `%SHELL%`）。
fn expand_windows_env(raw: &str, probe: &Probe<'_>) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut rest = raw;
    while let Some(open) = rest.find('%') {
        out.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        let Some(close) = after.find('%') else {
            out.push_str(&rest[open..]);
            return out;
        };
        let name = &after[..close];
        if let Some(value) = (probe.env)(name) {
            out.push_str(&value);
        }
        rest = &after[close + 1..];
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    /// 直接完成一次发现：`Probe` 内部的闭包借用入参 map，必须在同一表达式里用掉，
    /// 不能作为返回值带出（否则返回的是指向临时闭包的引用 → E0515）。
    fn discover_with(
        platform: Platform,
        env: &HashMap<String, String>,
        files: &HashMap<String, bool>,
    ) -> Vec<ShellProfile> {
        discover_from(&Probe {
            platform,
            env: &|k| env.get(k).cloned(),
            exists: &|p| files.get(&p.to_string_lossy().to_lowercase()).copied().unwrap_or(false),
        })
    }

    #[test]
    fn windows_prefers_pwsh_then_powershell_then_cmd() {
        let mut env = HashMap::new();
        env.insert("SystemRoot".into(), "C:\\Windows".into());
        env.insert("ComSpec".into(), "C:\\Windows\\System32\\cmd.exe".into());
        let mut files = HashMap::new();
        let lower = |s: &str| s.to_lowercase();
        files.insert(lower("C:\\Windows\\System32\\WindowsPowerShell\\v1.0\\powershell.exe"), true);
        files.insert(lower("C:\\Windows\\System32\\cmd.exe"), true);

        let list = discover_with(Platform::Windows, &env, &files);
        let default = default_profile(&list).expect("至少 cmd 可用");
        assert_eq!(
            default.id(),
            "powershell",
            "pwsh 不存在时应退到 Windows PowerShell 而不是直接掉到 cmd"
        );

        // 装上 pwsh 后应优先选它
        let mut files2 = files.clone();
        files2.insert(lower("C:\\Program Files\\PowerShell\\7\\pwsh.exe"), true);
        let mut env2 = env.clone();
        env2.insert("ProgramFiles".into(), "C:\\Program Files".into());
        let list2 = discover_with(Platform::Windows, &env2, &files2);
        assert_eq!(default_profile(&list2).unwrap().id(), "pwsh");
    }

    #[test]
    fn unix_shell_env_only_matches_its_own_kind() {
        let mut env = HashMap::new();
        env.insert("SHELL".into(), "/usr/bin/zsh".into());
        env.insert("PATH".into(), "/usr/bin:/bin".into());
        let mut files = HashMap::new();
        files.insert("/usr/bin/zsh".into(), true);
        files.insert("/bin/sh".into(), true);

        let list = discover_with(Platform::Unix, &env, &files);
        let bash = find_profile(&list, "bash").unwrap();
        assert!(!bash.available, "zsh 不能被误报成 bash");
        let zsh = find_profile(&list, "zsh").unwrap();
        assert!(zsh.available, "$SHELL=zsh 时 zsh 必须可用");
        assert_eq!(zsh.program, "/usr/bin/zsh");
        assert_eq!(default_profile(&list).unwrap().id(), "zsh");
    }

    #[test]
    fn unavailable_shells_are_listed_with_reason_not_hidden() {
        let env = HashMap::new();
        let files = HashMap::new();
        let list = discover_with(Platform::Windows, &env, &files);
        assert_eq!(list.len(), 5, "五个 Windows 候选都要列出（可解释为什么缺）");
        assert!(list.iter().all(|p| !p.available));
        assert!(list.iter().all(|p| p.source == "not-found"));
        assert!(default_profile(&list).is_none());
    }

    #[test]
    fn path_lookup_appends_pathext_on_windows() {
        let mut env = HashMap::new();
        env.insert("PATH".into(), "C:\\Tools;C:\\Windows".into());
        let mut files = HashMap::new();
        files.insert("c:\\tools\\pwsh.exe".into(), true);
        let list = discover_with(Platform::Windows, &env, &files);
        let pwsh = find_profile(&list, "pwsh").unwrap();
        assert!(pwsh.available);
        assert_eq!(pwsh.program, "C:\\Tools\\pwsh.exe");
        assert!(pwsh.source.starts_with("path:"));
    }

    #[test]
    fn aliases_resolve_to_stable_ids() {
        assert_eq!(ShellKind::from_id("PowerShell"), Some(ShellKind::WindowsPowerShell));
        assert_eq!(ShellKind::from_id("ps7"), Some(ShellKind::Pwsh));
        assert_eq!(ShellKind::from_id("git_bash"), Some(ShellKind::GitBash));
        assert_eq!(ShellKind::from_id("nope"), None);
    }

    #[test]
    fn default_args_keep_user_profiles() {
        assert_eq!(ShellKind::Pwsh.default_args(), vec!["-NoLogo"]);
        assert!(ShellKind::Bash.default_args().contains(&"-i".to_string()));
        assert!(ShellKind::Cmd.default_args().is_empty());
    }
}
