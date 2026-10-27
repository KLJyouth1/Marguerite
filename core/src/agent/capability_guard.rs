//! 反脚本守卫（W4.4，软门控）。
//!
//! 用户观察：「它不懂智能地调用工具，都是写脚本来偏盲控电脑」。根因之一是
//! 默认工具面里 `shell_exec`/`code_exec` 永远可见，而结构化能力（桌面/浏览器/
//! 文件读写）不一定被模型想起来。本守卫在**执行前**识别「用脚本模拟一个已可用
//! 的结构化能力」，返回一条可识别的纠偏错误（`CAPABILITY_MISROUTE`），让模型
//! 下一轮改用正确工具。
//!
//! 纪律（不做的事）：
//! - 只在对应结构化工具**真的可用**时触发；工具不可用就放行（不能因缺能力而阻断）；
//! - 同一 turn 内每个能力只纠偏一次，第二次放行（避免死循环；误伤可观测）；
//! - `pytest`/`cargo test`/`git`/构建脚本一律放行 —— 脚本本身是正当能力；
//! - 默认 `warn`（本次拒绝 + 提示重试），可用 `MARGUERITE_CAPABILITY_GUARD=off` 关闭。

use std::collections::{HashMap, HashSet};

use crate::agent::tool_cycle::ToolResult;

/// 桌面 GUI 自动化库：出现即视为「想用脚本模拟结构化桌面能力」。
const DESKTOP_SCRIPT_LIBS: &[&str] = &[
    "pyautogui",
    "uiautomation",
    "pywinauto",
    "autoit",
    "ahk",
    "xdotool",
];

/// 浏览器自动化库（`browser` 工具可用时视为绕路）。
const BROWSER_SCRIPT_LIBS: &[&str] = &["selenium", "playwright", "puppeteer"];

/// 正当脚本的显式白名单：测试运行器 / 构建 / 包管理。
///
/// 不能只靠「出现 playwright/selenium 就当绕路」——本仓的 e2e 就是
/// `npx playwright test`，开发者每天都在跑。命中这些前缀一律放行。
const LEGIT_SCRIPT_HINTS: &[&str] = &[
    "playwright test",
    "npx playwright",
    "pytest",
    "cargo test",
    "cargo build",
    "cargo check",
    "cargo clippy",
    "npm test",
    "npm run test",
    "npm run build",
    "npm ci",
    "npm install",
    "pnpm ",
    "yarn ",
    "vitest",
    "jest",
    "go test",
    "mvn test",
    "gradle test",
];

/// 用脚本写文件的信号：`write_text(`/`write_bytes(`，或 `open(...)` 且模式含
/// w/a/x（任意引号风格、允许 encoding/其它 kwargs）。读模式（r/缺省）不拦。
fn opens_for_write(text: &str) -> bool {
    if text.contains("write_text(") || text.contains("write_bytes(") {
        return true;
    }
    let mut rest = text;
    while let Some(idx) = rest.find("open(") {
        let after = &rest[idx + "open(".len()..];
        let window = &after[..after.len().min(160)];
        for quote in ['\'', '"'] {
            for mode in ['w', 'a', 'x'] {
                let needle = format!("{quote}{mode}{quote}");
                if window.contains(&needle) {
                    return true;
                }
            }
        }
        rest = after;
    }
    false
}

fn guard_enabled() -> bool {
    match std::env::var("MARGUERITE_CAPABILITY_GUARD") {
        Ok(raw) => !matches!(
            raw.trim().to_ascii_lowercase().as_str(),
            "off" | "0" | "false" | "no"
        ),
        Err(_) => true,
    }
}

fn available_has(available: &[String], tool: &str) -> bool {
    available.iter().any(|name| name == tool)
}

/// 检查一次工具调用；命中返回纠偏 `ToolResult::Error`，否则 `None`（放行）。
///
/// `corrected` 是**本 turn** 的已纠偏能力集合；调用方每 turn 新建一次。
pub fn check(
    tool: &str,
    args: &HashMap<String, String>,
    available: &[String],
    corrected: &mut HashSet<String>,
) -> Option<ToolResult> {
    if !guard_enabled() || (tool != "shell_exec" && tool != "code_exec") {
        return None;
    }
    let text = ["command", "code", "script"]
        .iter()
        .filter_map(|key| args.get(*key))
        .cloned()
        .collect::<Vec<_>>()
        .join("\n")
        .to_ascii_lowercase();
    if text.trim().is_empty() {
        return None;
    }
    // 测试/构建/包管理命令是正当能力，绝不纠偏。
    if LEGIT_SCRIPT_HINTS
        .iter()
        .any(|hint| text.contains(&hint.to_ascii_lowercase()))
    {
        return None;
    }

    let (capability, replacement) = if DESKTOP_SCRIPT_LIBS
        .iter()
        .any(|lib| text.contains(lib))
        && (available_has(available, "desktop_action")
            || available_has(available, "desktop_launch"))
    {
        (
            "desktop.uia",
            if available_has(available, "desktop_action") {
                "desktop_action"
            } else {
                "desktop_launch"
            },
        )
    } else if BROWSER_SCRIPT_LIBS
        .iter()
        .any(|lib| text.contains(lib))
        && available_has(available, "browser")
    {
        ("browser.a11y", "browser")
    } else if opens_for_write(&text) && available_has(available, "write_file") {
        ("file.edit", "write_file")
    } else {
        return None;
    };

    if !corrected.insert(capability.to_string()) {
        // 同一 turn 已纠偏过一次：放行（模型坚持要脚本，不再打断）。
        return None;
    }
    Some(ToolResult::Error {
        message: format!(
            "检测到你用脚本替代「{capability}」；请改用 `{replacement}`。\
             若确需脚本（例如结构化工具无法覆盖的场景），请在下一轮明确说明理由并重试。"
        ),
        code: "CAPABILITY_MISROUTE".to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect()
    }

    fn available(names: &[&str]) -> Vec<String> {
        names.iter().map(|name| (*name).to_string()).collect()
    }

    #[test]
    fn gui_script_is_redirected_to_desktop_action_when_available() {
        let mut corrected = HashSet::new();
        let out = check(
            "shell_exec",
            &args(&[("command", "python -c \"import pyautogui; pyautogui.click(10,10)\"")]),
            &available(&["shell_exec", "desktop_launch", "desktop_action"]),
            &mut corrected,
        )
        .expect("应纠偏");
        match out {
            ToolResult::Error { code, message } => {
                assert_eq!(code, "CAPABILITY_MISROUTE");
                assert!(message.contains("desktop_action"), "{message}");
            }
            other => panic!("应为 Error，实际 {other:?}"),
        }
        // 同一 turn 第二次放行（不打断坚持要脚本的模型）。
        assert!(check(
            "shell_exec",
            &args(&[("command", "import pyautogui")]),
            &available(&["desktop_action"]),
            &mut corrected,
        )
        .is_none());
    }

    #[test]
    fn script_is_allowed_when_the_structured_tool_is_unavailable() {
        let mut corrected = HashSet::new();
        assert!(check(
            "shell_exec",
            &args(&[("command", "python -c \"import pyautogui\"")]),
            &available(&["shell_exec"]),
            &mut corrected,
        )
        .is_none());
    }

    #[test]
    fn legitimate_scripts_are_never_blocked() {
        let mut corrected = HashSet::new();
        for command in [
            "cargo test -p marguerite-core",
            "pytest -q tests/unit",
            "git status --short",
            "npm run build",
        ] {
            assert!(
                check(
                    "shell_exec",
                    &args(&[("command", command)]),
                    &available(&["desktop_action", "browser", "write_file"]),
                    &mut corrected,
                )
                .is_none(),
                "正当脚本被误拦：{command}"
            );
        }
    }

    #[test]
    fn browser_script_is_redirected_only_when_browser_tool_exists() {
        let mut corrected = HashSet::new();
        let blocked = check(
            "code_exec",
            &args(&[("code", "from playwright.sync_api import sync_playwright")]),
            &available(&["browser"]),
            &mut corrected,
        );
        assert!(blocked.is_some());

        let mut corrected2 = HashSet::new();
        let allowed = check(
            "code_exec",
            &args(&[("code", "from playwright.sync_api import sync_playwright")]),
            &available(&["read_file"]),
            &mut corrected2,
        );
        assert!(allowed.is_none(), "没有 browser 工具时不应阻断");
    }

    #[test]
    fn legitimate_test_and_build_commands_are_never_redirected() {
        let mut corrected = HashSet::new();
        for command in [
            "npx playwright test e2e/chat-theme.spec.ts",
            "npm run build >> build.log",
            "cargo test -p marguerite-core --lib",
            "pytest -q tests/unit",
        ] {
            assert!(
                check(
                    "shell_exec",
                    &args(&[("command", command)]),
                    &available(&["browser", "write_file", "desktop_action"]),
                    &mut corrected,
                )
                .is_none(),
                "正当命令被误拦：{command}"
            );
        }
    }

    #[test]
    fn python_file_write_is_redirected_to_write_file() {
        let mut corrected = HashSet::new();
        let out = check(
            "shell_exec",
            &args(&[("command", "python -c \"open('a.txt','w').write('x')\"")]),
            &available(&["write_file"]),
            &mut corrected,
        );
        assert!(out.is_some());
        // 最常见的 kwargs 写法也必须命中。
        let mut corrected2 = HashSet::new();
        assert!(check(
            "shell_exec",
            &args(&[("command", "python -c \"open('a.txt', 'w', encoding='utf-8').write('x')\"")]),
            &available(&["write_file"]),
            &mut corrected2,
        )
        .is_some());
        let mut only_read = HashSet::new();
        assert!(check(
            "shell_exec",
            &args(&[("command", "python -c \"print(open('a.txt').read())\"")]),
            &available(&["read_file"]),
            &mut only_read,
        )
        .is_none(), "只读脚本不属于文件写绕路");
    }
}
