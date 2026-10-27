//! Composer：@引用 / !命令 / 粘贴落盘（B3）

use std::path::PathBuf;

use chrono::Utc;

use crate::tui::workbench::command;
use crate::tui::workbench::state::{ChatMessage, ChatRole, FileRef, UiEffect, WorkbenchState};

/// 输入协议处理器
pub struct Composer;

impl Composer {
    /// 解析输入协议，返回应执行的副作用
    pub fn submit(state: &mut WorkbenchState) -> Vec<UiEffect> {
        let text = std::mem::take(&mut state.composer.buffer);
        // 记录到命令历史（跨会话持久化，去重/空值由 history 内部处理）
        state.composer.history.push(&text);
        if let Some(cmd) = text.strip_prefix('!') {
            // !cmd → 经沙箱执行
            return vec![UiEffect::RunSandbox(shellsplit(cmd))];
        }
        if text.starts_with('/') {
            // /cmd → 命令分发
            return command::dispatch(state, &text);
        }
        // 普通消息：解析 @file 引用 → 注入内容/构建上下文变量
        let refs = resolve_refs(&text);
        state.composer.references = refs;
        // 把用户这句话记进对话流：此前 `submit` 只返回 `SubmitPrompt` 副作用、
        // **从不入列**，于是 TUI 的对话视图只有 AI 一侧，用户看不到自己说过什么，
        // 也读不出上下文（长会话尤其明显）。`!cmd` / `/cmd` 属命令通道，不入对话流。
        state.chat.messages.push(ChatMessage {
            role: ChatRole::User,
            content: text.clone(),
        });
        vec![UiEffect::SubmitPrompt(text)]
    }

    /// 粘贴处理：>16k 字符落盘并引用；否则直接入缓冲
    pub fn on_paste(state: &mut WorkbenchState, clip: &str) -> UiEffect {
        if clip.chars().count() > 16_000 {
            let path = paste_to_disk(clip);
            state.composer.pending_paste = Some(path.clone());
            UiEffect::PasteFile(path)
        } else {
            state.composer.buffer.push_str(clip);
            UiEffect::Render
        }
    }
}

/// 简单 shell 切分（按空白，支持双引号包裹；不展开变量）
pub fn shellsplit(s: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut in_q = false;
    for c in s.chars() {
        match c {
            '"' => in_q = !in_q,
            c if c.is_whitespace() && !in_q => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
            }
            c => cur.push(c),
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// 解析 @file 引用
pub fn resolve_refs(text: &str) -> Vec<FileRef> {
    let mut refs = Vec::new();
    for tok in text.split_whitespace() {
        if let Some(path) = tok.strip_prefix('@') {
            if !path.is_empty() {
                refs.push(FileRef {
                    path: PathBuf::from(path),
                    excerpt: None,
                });
            }
        }
    }
    refs
}

/// 大段粘贴落盘到 `~/.marguerite/pastes/paste-<ts>.md`
pub fn paste_to_disk(clip: &str) -> PathBuf {
    let dir = std::env::temp_dir().join("marguerite_pastes");
    let _ = std::fs::create_dir_all(&dir);
    let ts = Utc::now().timestamp();
    let path = dir.join(format!("paste-{ts}.md"));
    let _ = std::fs::write(&path, clip);
    path
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::workbench::state::WorkbenchState;

    #[test]
    fn test_shellsplit_simple() {
        assert_eq!(shellsplit("ls -la"), vec!["ls", "-la"]);
    }

    #[test]
    fn test_shellsplit_quoted() {
        assert_eq!(
            shellsplit(r#"echo "hello world""#),
            vec!["echo", "hello world"]
        );
    }

    #[test]
    fn test_submit_bang_becomes_sandbox() {
        let mut s = WorkbenchState::new("s1".into(), "m".into());
        s.composer.buffer = "!ls -la".into();
        let eff = Composer::submit(&mut s);
        match eff.as_slice() {
            [UiEffect::RunSandbox(args)] => {
                assert_eq!(args, &vec!["ls".to_string(), "-la".to_string()])
            }
            _ => panic!("应为 RunSandbox"),
        }
    }

    #[test]
    fn test_submit_slash_dispatches_command() {
        let mut s = WorkbenchState::new("s1".into(), "m".into());
        s.composer.buffer = "/mode yolo".into();
        let eff = Composer::submit(&mut s);
        assert!(eff.iter().any(|e| matches!(e, UiEffect::SwitchMode(_))));
    }

    #[test]
    fn test_submit_plain_becomes_prompt() {
        let mut s = WorkbenchState::new("s1".into(), "m".into());
        s.composer.buffer = "解释这段代码".into();
        let eff = Composer::submit(&mut s);
        match eff.as_slice() {
            [UiEffect::SubmitPrompt(t)] => assert_eq!(t, "解释这段代码"),
            _ => panic!("应为 SubmitPrompt"),
        }
    }

    /// 普通输入必须进入对话流（此前只返回副作用、从不入列 → 用户看不到自己说的话）。
    #[test]
    fn test_submit_plain_records_user_message_in_chat() {
        let mut s = WorkbenchState::new("s1".into(), "m".into());
        s.composer.buffer = "帮我重构这个模块".into();
        let _ = Composer::submit(&mut s);
        assert_eq!(s.chat.messages.len(), 1, "用户消息必须入列");
        assert_eq!(s.chat.messages[0].role, ChatRole::User);
        assert_eq!(s.chat.messages[0].content, "帮我重构这个模块");
    }

    /// 命令通道（`!cmd` / `/cmd`）不得混进对话流。
    #[test]
    fn test_submit_commands_do_not_pollute_chat_history() {
        let mut s = WorkbenchState::new("s1".into(), "m".into());
        s.composer.buffer = "!ls -la".into();
        let _ = Composer::submit(&mut s);
        s.composer.buffer = "/mode yolo".into();
        let _ = Composer::submit(&mut s);
        assert!(
            s.chat.messages.is_empty(),
            "沙箱命令与斜杠命令是控制通道，不是对话内容: {:?}",
            s.chat.messages
        );
    }

    #[test]
    fn test_resolve_refs() {
        let refs = resolve_refs("看 @src/main.rs 和 @README.md 谢谢");
        assert_eq!(refs.len(), 2);
        assert_eq!(refs[0].path, PathBuf::from("src/main.rs"));
    }

    #[test]
    fn test_on_paste_small_appends() {
        let mut s = WorkbenchState::new("s1".into(), "m".into());
        let eff = Composer::on_paste(&mut s, "hello");
        assert!(matches!(eff, UiEffect::Render));
        assert_eq!(s.composer.buffer, "hello");
    }

    #[test]
    fn test_on_paste_large_writes_file() {
        let big = "x".repeat(20_000);
        let mut s = WorkbenchState::new("s1".into(), "m".into());
        let eff = Composer::on_paste(&mut s, &big);
        match eff {
            UiEffect::PasteFile(p) => {
                assert!(p.exists());
                let _ = std::fs::remove_file(&p);
            }
            _ => panic!("大段粘贴应落盘"),
        }
    }
}
