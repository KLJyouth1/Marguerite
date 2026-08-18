//! 交互式对话 REPL（标准 REPL 风格纯终端流式会话）
//!
//! 对标主流 CLI 工作台的纯终端多轮对话体验：逐行读取用户输入，
//! 维护 `Vec<Message>` 多轮历史，调用 `AgentRuntime::chat_stream` 逐 token 流式打印，
//! 支持斜杠命令。与 ratatui 工作台并存——`marguerite chat` 进本 REPL，
//! 无参 `marguerite` 仍进 ratatui 工作台（视觉更丰富）。二者共用同一 `AgentRuntime` 核心层。
//!
//! 会话持久化与 `marguerite session list/delete` 共用同一 SQLite
//! （`~/.marguerite/sessions.db`），因此 REPL 落盘的对话可被现有子命令直接管理。

use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};

use anyhow::Result;
use chrono::Utc;
use std::sync::Arc;
use uuid::Uuid;

use crate::agent::runtime::AgentRuntime;
use crate::mcp::registry::ToolRegistry;
use crate::model::llm::ToolDef;
use crate::model::StreamReceiver;
use crate::persistence::config::Config;
use crate::persistence::session::{SessionInfo, SessionManager};
use crate::tools::builtin::register_builtin_tools;
use crate::types::Message;

/// 会话落盘时记录的状态标记（与 `session list` 展示的 state 列一致）
const SESSION_STATE: &str = "active";

/// 消费流式接收器，逐块打印到 stdout，返回拼接后的完整文本。
///
/// 供 CLI 直接模式（`-p`）与 REPL 共用。stderr 仍走 tracing 日志，不污染 stdout 业务流。
/// 遇到错误块时打印到 stderr 并停止消费，返回已累积文本。
///
/// `print` 为 `false` 时静默累积（用于 `--json` 单发模式：先收齐文本再包成 JSON，
/// 避免流式文本与 JSON 信封混进 stdout）。
pub async fn stream_assistant(mut rx: StreamReceiver, print: bool) -> Result<String> {
    let mut full = String::new();
    let mut had_error = false;
    let mut error_msg = String::new();
    while let Some(chunk) = rx.recv().await {
        match chunk {
            Ok(c) => {
                if let Some(content) = c.content {
                    if print {
                        print!("{content}");
                        let _ = std::io::stdout().flush();
                    }
                    full.push_str(&content);
                }
                // 思考过程（thinking）在 CLI 默认不渲染，保留给 GUI 层可选展示。
            }
            Err(e) => {
                had_error = true;
                error_msg = e.to_string();
                if print {
                    eprintln!("\n[stream error] {e}");
                }
                break;
            }
        }
    }
    if had_error {
        Err(anyhow::anyhow!("stream error: {error_msg}"))
    } else {
        Ok(full)
    }
}

/// REPL 会话持久化封装。
///
/// 把「可失败的落盘」与「必须可用的对话」解耦：数据库打开、恢复、保存任一环节失败时
/// 降级为纯内存模式（`manager == None`），REPL 仍能正常对话，只是不落盘并给出一次提示。
pub struct ReplStore {
    manager: Option<SessionManager>,
    session_id: String,
    /// 已提示过降级，避免每轮重复刷屏
    warned: bool,
}

impl ReplStore {
    /// 按默认数据库路径（`~/.marguerite/sessions.db`）打开，与 `session` 子命令同源。
    ///
    /// 返回 `(store, 恢复出的历史)`。`resume` 优先于 `continue_last`。
    pub fn open(resume: Option<&str>, continue_last: bool) -> (Self, Vec<Message>) {
        let db_path = Config::config_dir().join("sessions.db");
        Self::open_at(&db_path, resume, continue_last)
    }

    /// 按指定数据库路径打开（供测试与非默认部署复用）。
    pub fn open_at(
        db_path: &Path,
        resume: Option<&str>,
        continue_last: bool,
    ) -> (Self, Vec<Message>) {
        let manager = Self::init_manager(db_path);
        let mut store = Self {
            manager,
            session_id: Uuid::new_v4().to_string(),
            warned: false,
        };
        let history = store.restore_on_open(resume, continue_last);
        (store, history)
    }

    /// 打开并初始化数据库，失败则返回 `None`（降级为纯内存）。
    fn init_manager(db_path: &Path) -> Option<SessionManager> {
        if let Some(parent) = db_path.parent() {
            if std::fs::create_dir_all(parent).is_err() {
                return None;
            }
        }
        let mut manager = SessionManager::new(db_path);
        match manager.init() {
            Ok(()) => Some(manager),
            Err(_) => None,
        }
    }

    /// 启动时按 `--resume` / `--continue` 恢复历史。
    ///
    /// 恢复成功则复用原 `session_id`（后续落盘继续写同一会话），失败则保留新建 id。
    fn restore_on_open(&mut self, resume: Option<&str>, continue_last: bool) -> Vec<Message> {
        let target = match (resume, continue_last) {
            (Some(id), _) => Some(id.to_string()),
            (None, true) => self.latest_session_id(),
            _ => None,
        };
        let Some(id) = target else {
            return Vec::new();
        };
        let Some(manager) = self.manager.as_ref() else {
            eprintln!("[repl] 会话存储不可用，无法恢复历史，已进入纯内存模式。");
            return Vec::new();
        };
        match manager.restore_session(&id) {
            Ok(messages) if !messages.is_empty() => {
                self.session_id = id;
                println!(
                    "已恢复会话 {} （{} 条消息）。输入 /history 查看，/clear 重新开始。",
                    Self::short_id(&self.session_id),
                    messages.len()
                );
                messages
            }
            Ok(_) => {
                if resume.is_some() {
                    eprintln!(
                        "[repl] 会话 {} 无历史消息，已新建会话。",
                        Self::short_id(&id)
                    );
                }
                Vec::new()
            }
            Err(e) => {
                eprintln!("[repl] 恢复会话失败：{e}；已新建会话。");
                Vec::new()
            }
        }
    }

    /// 取最近一次会话 id（供 `--continue` 使用）
    fn latest_session_id(&self) -> Option<String> {
        let manager = self.manager.as_ref()?;
        let sessions = manager.list_sessions(1).ok()?;
        sessions.into_iter().next().map(|s| s.session_id)
    }

    /// 当前会话 id
    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    /// 是否处于持久化可用状态
    pub fn is_persistent(&self) -> bool {
        self.manager.is_some()
    }

    /// 落盘当前历史。失败仅提示一次，不中断对话。
    pub fn persist(&mut self, history: &[Message]) {
        if history.is_empty() {
            return;
        }
        let Some(manager) = self.manager.as_ref() else {
            return;
        };
        if let Err(e) = manager.save_snapshot(&self.session_id, SESSION_STATE, history, 0) {
            if !self.warned {
                eprintln!("[repl] 会话落盘失败：{e}（后续不再重复提示）");
                self.warned = true;
            }
        }
    }

    /// 列出最近会话
    pub fn list(&self, limit: usize) -> Vec<SessionInfo> {
        self.manager
            .as_ref()
            .and_then(|m| m.list_sessions(limit).ok())
            .unwrap_or_default()
    }

    /// 切换到指定会话，返回其历史。
    ///
    /// 支持 id 前缀匹配（与 `session list` 展示的 8 位短 id 对齐）。
    pub fn switch(&mut self, id_or_prefix: &str) -> Result<Vec<Message>> {
        let manager = self
            .manager
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("会话存储不可用（纯内存模式）"))?;
        let full_id = Self::resolve_id(manager, id_or_prefix)?;
        let messages = manager.restore_session(&full_id)?;
        self.session_id = full_id;
        Ok(messages)
    }

    /// 把短 id 前缀解析为完整 session id；精确匹配优先。
    fn resolve_id(manager: &SessionManager, id_or_prefix: &str) -> Result<String> {
        if manager.get_session(id_or_prefix)?.is_some() {
            return Ok(id_or_prefix.to_string());
        }
        let candidates: Vec<String> = manager
            .list_sessions(100)?
            .into_iter()
            .map(|s| s.session_id)
            .filter(|id| id.starts_with(id_or_prefix))
            .collect();
        match candidates.len() {
            0 => Err(anyhow::anyhow!("未找到会话：{id_or_prefix}")),
            1 => Ok(candidates.into_iter().next().unwrap_or_default()),
            n => Err(anyhow::anyhow!(
                "会话前缀 {id_or_prefix} 匹配到 {n} 个结果，请提供更长的 id"
            )),
        }
    }

    /// 截取展示用短 id（与 `session list` 的 8 位一致）
    fn short_id(id: &str) -> String {
        id.chars().take(8).collect()
    }
}

/// REPL 启动选项
#[derive(Debug, Clone, Default)]
pub struct ReplOptions {
    /// `--resume <id>`：恢复指定会话
    pub resume: Option<String>,
    /// `--continue`：恢复最近一次会话
    pub continue_last: bool,
    /// 自定义会话数据库路径（`None` 用默认 `~/.marguerite/sessions.db`）
    pub db_path: Option<PathBuf>,
    /// `--tools`：启用 LLM function calling 工具回路（原生内置工具集）
    pub enable_tools: bool,
}

/// 进入交互式对话 REPL。
///
/// 维护多轮历史 `Vec<Message>`，每轮：读取输入 → 推入 User 消息 →
/// `chat_stream` 流式打印 → 推入 Assistant 消息 → 落盘快照。
/// 支持斜杠命令 /help /exit /clear /model /save /sessions /load /history。
pub async fn run_repl(runtime: AgentRuntime, options: ReplOptions) -> Result<()> {
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();

    let (mut store, mut history) = match options.db_path.as_deref() {
        Some(p) => ReplStore::open_at(p, options.resume.as_deref(), options.continue_last),
        None => ReplStore::open(options.resume.as_deref(), options.continue_last),
    };

    // 工具回路依赖：仅在 --tools 时构建，避免无谓开销（注册 20 个原生工具 + 可选能力）
    let (tool_registry, tool_defs): (Option<Arc<ToolRegistry>>, Option<Vec<ToolDef>>) =
        if options.enable_tools {
            let workspace = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
            let mut reg = ToolRegistry::new();
            match register_builtin_tools(&mut reg, &workspace, None, None, None) {
                Ok(()) => {
                    let defs = reg
                        .describe_tools_for_llm()
                        .into_iter()
                        .map(|(name, description, parameters)| ToolDef {
                            name,
                            description,
                            parameters,
                        })
                        .collect();
                    (Some(Arc::new(reg)), Some(defs))
                }
                Err(e) => {
                    eprintln!("[repl] 工具注册失败，将以纯对话模式运行：{e}");
                    (None, None)
                }
            }
        } else {
            (None, None)
        };

    println!("Marguerite REPL — 输入消息开始对话；/help 查看命令，/exit 退出。");
    if store.is_persistent() {
        println!(
            "会话 {}（自动落盘，可用 `marguerite session list` 查看）",
            ReplStore::short_id(store.session_id())
        );
    } else {
        println!("会话存储不可用，本次对话仅保留在内存中。");
    }
    if tool_registry.is_some() {
        println!(
            "工具回路已启用（{} 个内置工具）。模型可在对话中调用工具。",
            tool_defs.as_ref().map(|d| d.len()).unwrap_or(0)
        );
    }

    loop {
        print!("> ");
        let _ = stdout.flush();
        let mut line = String::new();
        if stdin.lock().read_line(&mut line)? == 0 {
            // EOF（Ctrl-D）：优雅退出前落盘
            println!();
            store.persist(&history);
            break;
        }
        let input = line.trim().to_string();
        if input.is_empty() {
            continue;
        }
        if input.starts_with('/') {
            if handle_slash(&input, &mut store, &mut history) == SlashOutcome::Exit {
                store.persist(&history);
                break;
            }
            continue;
        }

        history.push(Message::User {
            content: input.clone(),
            timestamp: Utc::now(),
        });

        if let (Some(registry), Some(defs)) = (tool_registry.as_ref(), tool_defs.as_ref()) {
            // 工具回路模式：plan → act → observe 闭环，工具活动由本循环打印
            let before = history.len();
            match runtime
                .run_tool_loop(&mut history, registry.clone(), defs)
                .await
            {
                Ok(reply) => {
                    // 仅打印本轮新增的 Message::Tool 活动（run_tool_loop 已把最终 Assistant 推入历史）
                    for m in &history[before..] {
                        if let Message::Tool {
                            name,
                            input,
                            output,
                            ..
                        } = m
                        {
                            let out_preview: String = output.chars().take(800).collect();
                            let ellipsis = if output.chars().count() > 800 {
                                "…"
                            } else {
                                ""
                            };
                            println!("  [tool:{name}] {input}");
                            println!("  → {out_preview}{ellipsis}");
                        }
                    }
                    println!();
                    if !reply.is_empty() {
                        println!("{reply}");
                    }
                    store.persist(&history);
                }
                Err(e) => {
                    eprintln!("[tool loop error] {e}");
                    // 出错时回滚刚推入的 User 消息，避免污染历史
                    history.pop();
                }
            }
        } else {
            match runtime.chat_stream(&history).await {
                Ok(rx) => match stream_assistant(rx, true).await {
                    Ok(reply) => {
                        println!();
                        if !reply.is_empty() {
                            history.push(Message::Assistant {
                                content: reply,
                                thinking: None,
                                timestamp: Utc::now(),
                            });
                        }
                        store.persist(&history);
                    }
                    Err(e) => {
                        eprintln!("[chat error] {e}");
                        history.pop();
                    }
                },
                Err(e) => {
                    eprintln!("[chat error] {e}");
                    history.pop();
                }
            }
        }
    }
    Ok(())
}

/// 斜杠命令处理结果
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlashOutcome {
    /// 已处理，继续 REPL 循环
    Continue,
    /// 请求退出 REPL
    Exit,
}

/// 处理斜杠命令。
///
/// 抽成独立函数以便离线单测（不依赖 stdin / 模型调用）。
pub fn handle_slash(
    input: &str,
    store: &mut ReplStore,
    history: &mut Vec<Message>,
) -> SlashOutcome {
    let mut parts = input.splitn(2, char::is_whitespace);
    let cmd = parts.next().unwrap_or("");
    let arg = parts.next().map(str::trim).unwrap_or("");

    match cmd {
        "/exit" | "/quit" => return SlashOutcome::Exit,
        "/help" => print_help(),
        "/clear" => {
            history.clear();
            println!(
                "已清空对话历史（当前会话仍为 {}）。",
                ReplStore::short_id(store.session_id())
            );
        }
        "/model" => {
            println!(
                "模型由配置（model router）按意图/复杂度自动路由；用 `model list` 查看可用模型。"
            );
        }
        "/save" => {
            if !store.is_persistent() {
                println!("会话存储不可用，无法保存。");
            } else if history.is_empty() {
                // 空历史不落盘（避免产生无内容的会话记录），提示需与实际行为一致
                println!("当前会话暂无消息，无需保存。");
            } else {
                store.persist(history);
                println!(
                    "已保存会话 {}（{} 条消息）。",
                    ReplStore::short_id(store.session_id()),
                    history.len()
                );
            }
        }
        "/sessions" => {
            let sessions = store.list(10);
            if sessions.is_empty() {
                println!("暂无会话记录。");
            } else {
                println!("最近会话（{}）：", sessions.len());
                for s in &sessions {
                    let marker = if s.session_id == store.session_id() {
                        "*"
                    } else {
                        " "
                    };
                    println!(
                        " {} [{}] {} | {} 条 | {}",
                        marker,
                        ReplStore::short_id(&s.session_id),
                        s.summary.chars().take(36).collect::<String>(),
                        s.message_count,
                        s.updated_at
                    );
                }
                println!("用 /load <id> 切换（支持短 id 前缀）。");
            }
        }
        "/load" => {
            if arg.is_empty() {
                println!("用法：/load <session-id>（支持短 id 前缀，可先用 /sessions 查看）");
            } else {
                match store.switch(arg) {
                    Ok(messages) => {
                        let n = messages.len();
                        *history = messages;
                        println!(
                            "已切换到会话 {}（{} 条消息）。",
                            ReplStore::short_id(store.session_id()),
                            n
                        );
                    }
                    Err(e) => println!("切换失败：{e}"),
                }
            }
        }
        "/history" => {
            if history.is_empty() {
                println!("当前会话暂无消息。");
            } else {
                println!("当前会话历史（{} 条）：", history.len());
                for (i, m) in history.iter().enumerate() {
                    let (role, content) = match m {
                        Message::User { content, .. } => ("user", content.as_str()),
                        Message::Assistant { content, .. } => ("assistant", content.as_str()),
                        Message::System { content, .. } => ("system", content.as_str()),
                        Message::Tool { name, output, .. } => (name.as_str(), output.as_str()),
                    };
                    let preview: String = content.chars().take(60).collect();
                    let ellipsis = if content.chars().count() > 60 {
                        "..."
                    } else {
                        ""
                    };
                    println!("  {:>2}. [{}] {}{}", i + 1, role, preview, ellipsis);
                }
            }
        }
        other => {
            println!("未知命令：{other}（输入 /help 查看可用命令）");
        }
    }
    SlashOutcome::Continue
}

/// 打印 REPL 帮助
fn print_help() {
    println!(
        "可用命令：\n  \
         /help      显示本帮助\n  \
         /clear     清空对话历史（不删除已落盘会话）\n  \
         /history   显示当前会话历史\n  \
         /save      立即保存当前会话\n  \
         /sessions  列出最近会话\n  \
         /load <id> 切换到指定会话（支持短 id 前缀）\n  \
         /model     显示当前模型路由说明\n  \
         /exit      退出 REPL（或按 Ctrl-D）"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 每个测试独立的临时数据库路径，避免相互污染
    fn temp_db() -> PathBuf {
        std::env::temp_dir()
            .join(format!("marguerite_repl_test_{}", Uuid::new_v4()))
            .join("sessions.db")
    }

    fn user(content: &str) -> Message {
        Message::User {
            content: content.to_string(),
            timestamp: Utc::now(),
        }
    }

    fn assistant(content: &str) -> Message {
        Message::Assistant {
            content: content.to_string(),
            thinking: None,
            timestamp: Utc::now(),
        }
    }

    #[test]
    fn test_store_opens_fresh_session_without_history() {
        let db = temp_db();
        let (store, history) = ReplStore::open_at(&db, None, false);
        assert!(store.is_persistent(), "临时目录应可创建数据库");
        assert!(history.is_empty());
        assert!(!store.session_id().is_empty());
    }

    #[test]
    fn test_persist_then_resume_roundtrip() {
        let db = temp_db();
        let (mut store, _) = ReplStore::open_at(&db, None, false);
        let sid = store.session_id().to_string();
        let history = vec![user("你好"), assistant("你好，有什么可以帮你？")];
        store.persist(&history);

        // 以 --resume <id> 重开，应恢复出同样的两条消息
        let (store2, restored) = ReplStore::open_at(&db, Some(&sid), false);
        assert_eq!(store2.session_id(), sid);
        assert_eq!(restored.len(), 2);
        match &restored[0] {
            Message::User { content, .. } => assert_eq!(content, "你好"),
            other => panic!("首条应为 User，实际 {other:?}"),
        }
    }

    #[test]
    fn test_continue_last_picks_most_recent_session() {
        let db = temp_db();
        let (mut first, _) = ReplStore::open_at(&db, None, false);
        first.persist(&[user("第一轮")]);

        let (mut second, _) = ReplStore::open_at(&db, None, false);
        let second_id = second.session_id().to_string();
        second.persist(&[user("第二轮"), assistant("收到")]);

        let (store, restored) = ReplStore::open_at(&db, None, true);
        assert_eq!(store.session_id(), second_id, "--continue 应恢复最近会话");
        assert_eq!(restored.len(), 2);
    }

    #[test]
    fn test_switch_by_short_id_prefix() {
        let db = temp_db();
        let (mut a, _) = ReplStore::open_at(&db, None, false);
        let a_id = a.session_id().to_string();
        a.persist(&[user("会话A")]);

        let (mut b, _) = ReplStore::open_at(&db, None, false);
        b.persist(&[user("会话B")]);

        let prefix: String = a_id.chars().take(8).collect();
        let restored = b.switch(&prefix).expect("短 id 前缀应能解析");
        assert_eq!(b.session_id(), a_id);
        assert_eq!(restored.len(), 1);
    }

    #[test]
    fn test_switch_unknown_id_errors() {
        let db = temp_db();
        let (mut store, _) = ReplStore::open_at(&db, None, false);
        assert!(store.switch("no-such-session").is_err());
    }

    #[test]
    fn test_resume_unknown_id_falls_back_to_empty_history() {
        let db = temp_db();
        let (_store, history) = ReplStore::open_at(&db, Some("ghost-id"), false);
        assert!(history.is_empty(), "未知会话应回退为空历史而非 panic");
    }

    #[test]
    fn test_persist_empty_history_is_noop() {
        let db = temp_db();
        let (mut store, _) = ReplStore::open_at(&db, None, false);
        store.persist(&[]);
        assert!(store.list(10).is_empty(), "空历史不应产生会话记录");
    }

    #[test]
    fn test_slash_exit_returns_exit_outcome() {
        let db = temp_db();
        let (mut store, _) = ReplStore::open_at(&db, None, false);
        let mut history = vec![user("hi")];
        assert_eq!(
            handle_slash("/exit", &mut store, &mut history),
            SlashOutcome::Exit
        );
        assert_eq!(
            handle_slash("/quit", &mut store, &mut history),
            SlashOutcome::Exit
        );
    }

    #[test]
    fn test_slash_clear_empties_history_but_keeps_session() {
        let db = temp_db();
        let (mut store, _) = ReplStore::open_at(&db, None, false);
        let sid = store.session_id().to_string();
        let mut history = vec![user("hi"), assistant("hello")];
        assert_eq!(
            handle_slash("/clear", &mut store, &mut history),
            SlashOutcome::Continue
        );
        assert!(history.is_empty());
        assert_eq!(store.session_id(), sid, "/clear 不应更换会话 id");
    }

    #[test]
    fn test_slash_save_then_listed() {
        let db = temp_db();
        let (mut store, _) = ReplStore::open_at(&db, None, false);
        let mut history = vec![user("需要保存")];
        handle_slash("/save", &mut store, &mut history);
        let sessions = store.list(10);
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].session_id, store.session_id());
    }

    #[test]
    fn test_slash_save_with_empty_history_creates_no_session() {
        // 回归：空历史时 persist 是 noop，提示不得声称「已保存」
        let db = temp_db();
        let (mut store, _) = ReplStore::open_at(&db, None, false);
        let mut history: Vec<Message> = Vec::new();
        handle_slash("/save", &mut store, &mut history);
        assert!(store.list(10).is_empty(), "空历史不应产生会话记录");
    }

    #[test]
    fn test_slash_load_switches_history() {
        let db = temp_db();
        let (mut a, _) = ReplStore::open_at(&db, None, false);
        let a_id = a.session_id().to_string();
        a.persist(&[user("旧会话内容")]);

        let (mut b, _) = ReplStore::open_at(&db, None, false);
        let mut history: Vec<Message> = Vec::new();
        handle_slash(&format!("/load {a_id}"), &mut b, &mut history);
        assert_eq!(b.session_id(), a_id);
        assert_eq!(history.len(), 1);
    }

    #[test]
    fn test_slash_load_without_arg_keeps_history() {
        let db = temp_db();
        let (mut store, _) = ReplStore::open_at(&db, None, false);
        let mut history = vec![user("保持不变")];
        handle_slash("/load", &mut store, &mut history);
        assert_eq!(history.len(), 1, "缺少参数时不应改动历史");
    }

    #[test]
    fn test_slash_unknown_command_is_continue() {
        let db = temp_db();
        let (mut store, _) = ReplStore::open_at(&db, None, false);
        let mut history: Vec<Message> = Vec::new();
        assert_eq!(
            handle_slash("/nope", &mut store, &mut history),
            SlashOutcome::Continue
        );
    }

    #[test]
    fn test_slash_history_and_sessions_do_not_mutate() {
        let db = temp_db();
        let (mut store, _) = ReplStore::open_at(&db, None, false);
        let mut history = vec![user("a"), assistant("b")];
        handle_slash("/history", &mut store, &mut history);
        handle_slash("/sessions", &mut store, &mut history);
        assert_eq!(history.len(), 2, "只读命令不应改动历史");
    }

    #[test]
    fn test_short_id_is_eight_chars() {
        let id = "0123456789abcdef";
        assert_eq!(ReplStore::short_id(id), "01234567");
    }
}
