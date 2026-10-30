//! 终端白名单（v2 工作台）
//!
//! 三分法：DENY（无条件拒绝）/ READONLY（只读探测）/ WRITE（写/构建，经模式门控）。
//! 未知命令默认拒绝；git 子命令级复核；重定向与 `sed -i` 参数级复核。

use std::path::Path;

/// 命令类别
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandClass {
    /// 无条件拒绝
    Deny,
    /// 只读探测（所有模式放行）
    ReadOnly,
    /// 写/构建（经模式门控）
    Write,
}

/// DENY：破坏性文件操作 / 系统控制 / shell 与解释器 / 网络下载执行链 / 系统信息
const DENY: &[&str] = &[
    // 破坏性文件操作
    "rm",
    "mv",
    "cp",
    "chmod",
    "chown",
    "chgrp",
    "ln",
    "mklink",
    "format",
    "mkfs",
    "dd",
    // 系统控制
    "kill",
    "pkill",
    "taskkill",
    "sudo",
    "su",
    "shutdown",
    "reboot",
    "poweroff",
    // shell 与解释器（禁用交互/内联）
    "bash",
    "sh",
    "zsh",
    "fish",
    "powershell",
    "pwsh",
    "cmd",
    "wscript",
    "cscript",
    // 网络下载/执行链
    "wget",
    "curl",
    "nc",
    "ncat",
    "telnet",
    "ssh",
    "scp",
    "sftp",
    "ftp",
    // 注册表/系统信息泄露
    "reg",
    "net",
    "netsh",
    "whoami",
    "systeminfo",
    "sc",
];

/// READONLY：只读探测（部分需参数级复核，见 `classify_full`）
const READONLY: &[&str] = &[
    "ls",
    "cat",
    "head",
    "tail",
    "less",
    "more",
    "grep",
    "rg",
    "find",
    "locate",
    "wc",
    "diff",
    "cmp",
    "sort",
    "uniq",
    "cut",
    "tr",
    "awk",
    "sed",
    "sha256sum",
    "md5sum",
    "file",
    "stat",
    "du",
    "df",
    "env",
    "pwd",
    "echo",
    "date",
    "tree",
    "lsblk",
    "uname",
    "uptime",
];

/// WRITE：写/构建类（经模式门控）
const WRITE: &[&str] = &[
    "git", "npm", "pnpm", "yarn", "cargo", "rustc", "python", "python3", "node", "deno", "bun",
    "tsc", "vite", "webpack", "esbuild", "jest", "vitest", "make", "cmake", "tar", "zip", "unzip",
    "7z", "touch", "mkdir", "rmdir", "tee",
];

/// git 子命令级黑名单（无条件拒绝）
const GIT_DENY_SUB: &[&str] = &[
    "push",
    "reset",
    "clean",
    "gc",
    "fsck",
    "archive",
    "filter-branch",
];
/// git 子命令级只读
const GIT_READONLY_SUB: &[&str] = &[
    "status",
    "log",
    "diff",
    "show",
    "branch",
    "remote",
    "tag",
    "ls-files",
    "rev-parse",
    "describe",
];
/// git 子命令级写
const GIT_WRITE_SUB: &[&str] = &[
    "add", "commit", "init", "stash", "checkout", "switch", "merge", "rebase", "restore", "mv",
    "rm", "config",
];

/// 解释器列表（内联载荷检测对象）
pub const INTERPRETERS: &[&str] = &["python", "python3", "node", "deno", "bun", "ruby", "perl"];

/// 取命令 basename（识别 `./rm` 伪装）
pub fn basename(cmd: &str) -> &str {
    Path::new(cmd)
        .file_name()
        .map(|s| s.to_str().unwrap_or(cmd))
        .unwrap_or(cmd)
}

/// 基础三分法分类（不含子命令/参数复核）
pub fn classify(argv: &[String]) -> CommandClass {
    let base = basename(argv.first().map(String::as_str).unwrap_or(""));
    if DENY.contains(&base) {
        CommandClass::Deny
    } else if READONLY.contains(&base) {
        CommandClass::ReadOnly
    } else if WRITE.contains(&base) {
        CommandClass::Write
    } else {
        CommandClass::Deny // 未列入 = 默认拒绝
    }
}

/// 取 git 子命令名（跳过全局 flag 与 `-C dir`，与 `check_git_subcommand` 一致）
pub fn git_subcommand_name(args: &[String]) -> Option<&str> {
    let mut iter = args.iter();
    while let Some(a) = iter.next() {
        if a == "-C" {
            iter.next(); // 跳过 -C 的目录参数
            continue;
        }
        if a.starts_with('-') {
            continue;
        }
        return Some(a);
    }
    None
}

/// git 子命令级分类：`git push` → Deny；`git status` → ReadOnly；`git commit` → Write
pub fn check_git_subcommand(args: &[String]) -> CommandClass {
    match git_subcommand_name(args) {
        Some(s) if GIT_DENY_SUB.contains(&s) => CommandClass::Deny,
        Some(s) if GIT_READONLY_SUB.contains(&s) => CommandClass::ReadOnly,
        Some(s) if GIT_WRITE_SUB.contains(&s) => CommandClass::Write,
        // 未知 git 子命令 / 仅 `git` → 保守拒绝 / 归为写
        Some(_) => CommandClass::Deny,
        None => CommandClass::Write,
    }
}

/// 生成拒绝原因（区分 DENY 表/未知命令 与 git 子命令黑名单）
pub fn deny_reason(argv: &[String]) -> String {
    let base = basename(argv.first().map(String::as_str).unwrap_or(""));
    if base == "git" {
        if let Some(sub) = git_subcommand_name(&argv[1..]) {
            return format!("git 子命令 '{sub}' 被禁止");
        }
    }
    format!("命令 '{base}' 不在白名单")
}

/// 是否解释器内联载荷（`python -c` / `node -e` / `bash -c`…，重组 argv 检测）
pub fn has_inline_payload(prog: &str, args: &[String]) -> bool {
    if !INTERPRETERS.contains(&prog) {
        return false;
    }
    args.iter().any(|a| {
        matches!(
            a.as_str(),
            "-c" | "-e" | "-p" | "--command" | "--eval" | "-exec"
        )
    })
}

/// 是否 shell 重定向 / 管道（`>` `>>` `|` `2>`），提示写意图
pub fn has_redirect(args: &[String]) -> bool {
    args.iter().any(|a| {
        let s = a.as_str();
        s == "|" || s == ">" || s == ">>" || s == "2>" || s == "2>>" || s.contains(">") && s != "=>"
    })
}

/// 完整分类：三分法 + git 子命令 + 重定向/`sed -i` 参数复核
pub fn classify_full(argv: &[String]) -> CommandClass {
    let base = basename(argv.first().map(String::as_str).unwrap_or(""));
    if DENY.contains(&base) {
        return CommandClass::Deny;
    }
    let rest = if argv.len() > 1 { &argv[1..] } else { &[] };

    // git 子命令级
    if base == "git" {
        return check_git_subcommand(rest);
    }
    // 基础三分法
    let mut class = if READONLY.contains(&base) {
        CommandClass::ReadOnly
    } else if WRITE.contains(&base) {
        CommandClass::Write
    } else {
        return CommandClass::Deny;
    };
    // 重定向提升为写
    if class == CommandClass::ReadOnly && has_redirect(rest) {
        class = CommandClass::Write;
    }
    // `sed -i` 就地编辑 → 写
    if base == "sed" && rest.iter().any(|a| a == "-i" || a.starts_with("-i")) {
        class = CommandClass::Write;
    }
    class
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn test_basename_strips_path() {
        assert_eq!(basename("./rm"), "rm");
        // 用本机分隔符拼：`basename` 走 `Path::file_name()`，`\` 在 Unix 上不是分隔符。
        let sep = std::path::MAIN_SEPARATOR;
        let nested = format!("C:{sep}Windows{sep}System32{sep}net.exe");
        assert_eq!(basename(&nested), "net.exe");
        assert_eq!(basename("ls"), "ls");
    }

    #[test]
    fn test_classify_basic() {
        assert_eq!(classify(&v(&["ls"])), CommandClass::ReadOnly);
        assert_eq!(classify(&v(&["cargo", "build"])), CommandClass::Write);
        assert_eq!(classify(&v(&["rm", "-rf", "/"])), CommandClass::Deny);
        assert_eq!(classify(&v(&["evil_tool"])), CommandClass::Deny);
        // ./rm 伪装 → basename=rm → Deny
        assert_eq!(classify(&v(&["./rm"])), CommandClass::Deny);
    }

    #[test]
    fn test_git_subcommands() {
        assert_eq!(check_git_subcommand(&v(&["push"])), CommandClass::Deny);
        assert_eq!(
            check_git_subcommand(&v(&["reset", "--hard"])),
            CommandClass::Deny
        );
        assert_eq!(
            check_git_subcommand(&v(&["clean", "-fd"])),
            CommandClass::Deny
        );
        assert_eq!(
            check_git_subcommand(&v(&["status"])),
            CommandClass::ReadOnly
        );
        assert_eq!(
            check_git_subcommand(&v(&["log", "--oneline"])),
            CommandClass::ReadOnly
        );
        assert_eq!(
            check_git_subcommand(&v(&["commit", "-m", "x"])),
            CommandClass::Write
        );
        assert_eq!(check_git_subcommand(&v(&["add", "."])), CommandClass::Write);
        assert_eq!(
            check_git_subcommand(&v(&["unknown_sub"])),
            CommandClass::Deny
        );
        assert_eq!(check_git_subcommand(&v(&[])), CommandClass::Write);
        // 全局 flag 跳过
        assert_eq!(
            check_git_subcommand(&v(&["-C", "dir", "status"])),
            CommandClass::ReadOnly
        );
    }

    #[test]
    fn test_classify_full_git() {
        assert_eq!(classify_full(&v(&["git", "push"])), CommandClass::Deny);
        assert_eq!(
            classify_full(&v(&["git", "status"])),
            CommandClass::ReadOnly
        );
        assert_eq!(
            classify_full(&v(&["git", "commit", "-m", "x"])),
            CommandClass::Write
        );
    }

    #[test]
    fn test_redirect_promotes_to_write() {
        assert_eq!(
            classify_full(&v(&["cat", "a.txt", ">", "b.txt"])),
            CommandClass::Write
        );
        assert_eq!(
            classify_full(&v(&["echo", "x", ">>", "log"])),
            CommandClass::Write
        );
        assert_eq!(classify_full(&v(&["cat", "a"])), CommandClass::ReadOnly);
        // 管道也是写意图
        assert_eq!(
            classify_full(&v(&["grep", "foo", "|", "wc"])),
            CommandClass::Write
        );
    }

    #[test]
    fn test_sed_inplace_promotes_to_write() {
        assert_eq!(
            classify_full(&v(&["sed", "-i", "s/a/b/", "f.rs"])),
            CommandClass::Write
        );
        assert_eq!(
            classify_full(&v(&["sed", "s/a/b/", "f.rs"])),
            CommandClass::ReadOnly
        );
    }

    #[test]
    fn test_inline_payload() {
        assert!(has_inline_payload("python", &v(&["-c", "print(1)"])));
        assert!(has_inline_payload("node", &v(&["-e", "console.log(1)"])));
        assert!(has_inline_payload("ruby", &v(&["-p", "x"])));
        assert!(!has_inline_payload("python", &v(&["script.py"])));
        assert!(!has_inline_payload(
            "node",
            &v(&["server.js", "--eval-mode"])
        ));
        assert!(!has_inline_payload("ls", &v(&["-l"])));
    }

    #[test]
    fn test_unknown_command_denied() {
        assert_eq!(
            classify_full(&v(&["evil_tool", "--flag"])),
            CommandClass::Deny
        );
        assert_eq!(classify_full(&v(&[""])), CommandClass::Deny);
    }
}
