//! 会话分支 / 恢复 / 回放（M2 / P0-2）：`sessions.fork` / `sessions.resume` / `sessions.replay`
//! 的**纯 Rust 实现**。
//!
//! 数据源是既有真实存储，不新建并行库：
//! - 会话转录（有序消息行）→ [`crate::persistence::session::SessionManager`]（`sessions.db`）
//! - 模型上下文 / 最近一份 [`CompactState`] → `crate::context::session` 台账（`<config>/context/`）
//!
//! # 三条红线
//!
//! 1. **父会话只读**：`fork` 只读父会话，绝不改动父会话的消息行、台账或元数据；
//! 2. **失败必带码**：做不到的路径返回 [`SessionError`]（`SESSION_NOT_FOUND` /
//!    `FORK_POINT_INVALID` / `CHECKPOINT_INVALID` / `SESSION_STORE_ERROR`），
//!    **绝不**返回空结果冒充成功；
//! 3. **宁缺毋滥**：拿不到的状态（例如前缀分支的 `CompactState`）明确置空，并在
//!    `notes` 里写明原因 —— 不猜、不补默认值（见 `context::compact_state` 的设计红线）。
//!
//! # 台账文件名的耦合（重要）
//!
//! 子会话的台账要写进 `context::session` 的目录，而该模块的文件名编码
//! （`safe_session_file_name`）是私有的。这里用 [`ledger_stem`] 复刻同一编码，并由
//! `ledger_stem_matches_context_directory_encoding` 用例**钉住**：一旦上游编码漂移，
//! 该用例立刻变红，不会出现「写了但读不到」的静默串会话。

use std::io::{self, ErrorKind};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde::Serialize;

use crate::context::compact_state::CompactState;
use crate::context::session::{
    context_dir, estimate_tokens, read_session_context, ContextEntry, SessionContext,
    DEFAULT_BUDGET,
};
use crate::persistence::session::SessionManager;
use crate::types::Message;

/// `sessions.fork` / `resume` / `replay` 的机器可判定错误码。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionError {
    /// 会话不存在（`sessions.db` 里没有该行）。
    SessionNotFound { session: String },
    /// 父会话存在但没有任何消息 —— 分支出来只会得到一个空壳，属拒绝而非成功。
    EmptyParentSession { session: String },
    /// 分支点越界（合法区间 `1..=available`）。
    ForkPointInvalid { requested: usize, available: usize },
    /// 指定的 checkpoint 不在该会话可用的预压缩快照里。
    CheckpointInvalid {
        requested: String,
        available: Vec<String>,
    },
    /// 存储层失败（SQLite / 台账 JSON / 原子写）。
    Store { detail: String },
}

impl SessionError {
    /// 对外 RPC 的错误码（稳定字符串，前端据此分支）。
    pub fn code(&self) -> &'static str {
        match self {
            Self::SessionNotFound { .. } => "SESSION_NOT_FOUND",
            Self::EmptyParentSession { .. } => "SESSION_EMPTY",
            Self::ForkPointInvalid { .. } => "FORK_POINT_INVALID",
            Self::CheckpointInvalid { .. } => "CHECKPOINT_INVALID",
            Self::Store { .. } => "SESSION_STORE_ERROR",
        }
    }
}

impl std::fmt::Display for SessionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SessionNotFound { session } => {
                write!(f, "会话不存在：{session}（请先确认 sessions.list 里的 key）")
            }
            Self::EmptyParentSession { session } => write!(
                f,
                "会话 {session} 没有任何消息，无可分支的前缀（请先聊至少一轮）"
            ),
            Self::ForkPointInvalid {
                requested,
                available,
            } => write!(
                f,
                "分支点 {requested} 越界：该会话只有 {available} 条消息，合法区间 1..={available}"
            ),
            Self::CheckpointInvalid { requested, .. } => write!(
                f,
                "checkpoint {requested} 不存在于该会话的预压缩快照（请先确认 resume 返回的 availableCheckpoints）"
            ),
            Self::Store { detail } => write!(f, "会话存储故障：{detail}"),
        }
    }
}

impl std::error::Error for SessionError {}

/// 只读消息帧（`replay` 与 `resume` 共用同一形状）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MessageFrame {
    /// 会话内从 0 开始的行序（= `session_messages.id` 的读取顺序）。
    pub index: usize,
    /// `user` / `assistant` / `system` / `tool`。
    pub role: String,
    /// 正文；工具帧为工具输出。
    pub text: String,
    /// 工具名（仅 `tool` 帧）。
    pub tool: Option<String>,
    /// RFC3339 时间戳（原样透传，不改写）。
    pub timestamp: String,
    /// 毫秒时间戳。
    pub at_ms: i64,
}

/// `sessions.fork` 的结果（父会话不变，返回新会话 key 与分支点）。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ForkedSession {
    /// 新会话 key（前端 `sessions.fork` 契约里的 `key`）。
    pub key: String,
    /// 同上（与 `key` 等值，兼容按 `session` 命名的读取方）。
    pub session: String,
    /// 父会话 key（契约里的 `parentKey`）。
    pub parent_key: String,
    /// 分支点 = 复制进子会话的消息条数（`1..=父会话消息数`）。
    pub fork_point: usize,
    /// `full`（到末尾全量分支）或 `prefix`（前缀分支）。
    ///
    /// ⚠️ 接线 `sessions.forkThroughTurn` 时，调用方必须把该字段覆写为
    /// `through_turn`（前端 `validatedForkChildKey` 会校验 `forkMode` 与
    /// `throughTurnId`），本模块不持有 turn id。
    pub fork_mode: String,
    /// 实际复制进子会话的消息条数（= `fork_point`）。
    pub message_count: usize,
    /// 子会话消息的 token 估算（启发式，非 tokenizer 精确值）。
    pub tokens_used: u32,
    /// 子会话标题（显式传入 > 父标题加 `(fork)`；父标题为空时为空串）。
    pub title: String,
    /// 是否把父会话的 [`CompactState`] 复制进了子会话台账。
    pub compact_state_copied: bool,
    /// 降级/未复制项的具体原因（不为空时必须原样展示给用户）。
    pub notes: Vec<String>,
}

/// `sessions.resume` 的结果：按 session id（+ 可选 checkpoint）恢复可继续状态。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResumedSession {
    /// 恢复的会话 key。
    pub session: String,
    /// 实际使用的 checkpoint：`latest` 或预压缩快照 id（`cmp-<uuid>`）。
    pub checkpoint: String,
    /// `live`（当前台账）或 `snapshot`（历史预压缩快照）。
    pub checkpoint_source: String,
    /// 该会话全部消息帧（有序）。
    pub frames: Vec<MessageFrame>,
    /// 消息条数。
    pub frame_count: usize,
    /// 最近一份压缩交接状态（拿不到时为 `null`，原因见 `notes`）。
    pub compact_state: Option<CompactState>,
    /// 最近一次压缩摘要。
    pub summary: Option<String>,
    /// 上下文预算（token）。
    pub budget: u64,
    /// 模型原始上下文窗口（0 = 从未设置）。
    pub context_window: u64,
    /// 会话绑定的项目目录（多根优先，旧单值回退）。
    pub project_paths: Vec<String>,
    /// 该会话可用的预压缩快照 id（新→旧）。
    pub available_checkpoints: Vec<String>,
    /// 是否有可继续的状态（有消息或有台账条目）。
    pub resumable: bool,
    /// 降级项的具体原因。
    pub notes: Vec<String>,
}

/// `sessions.replay` 的结果：**只读**、有序的消息帧。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReplayView {
    /// 会话 key。
    pub session: String,
    /// 恒为 `true`：回放只读，任何调用方都不得据此写回。
    pub read_only: bool,
    /// 有序消息帧（顺序 = 落库行序）。
    pub frames: Vec<MessageFrame>,
    /// 帧数。
    pub frame_count: usize,
    /// 降级项的具体原因。
    pub notes: Vec<String>,
}

/// 分支一个会话：把「消息前缀 + 最近一份 `CompactState` + 工作区/模型元数据」复制到新会话。
///
/// - `fork_point`：要复制的前缀长度（`None` = 复制全部消息，等价 `sessions.fork` 的全量语义）；
///   合法区间 `1..=父会话消息数`，越界返回 [`SessionError::ForkPointInvalid`]；
/// - `title`：子会话标题；`None` 时用「父标题 + `(fork)`」，父标题为空则留空。
pub fn fork_session(
    store: &mut SessionManager,
    parent_key: &str,
    fork_point: Option<usize>,
    title: Option<&str>,
) -> Result<ForkedSession, SessionError> {
    let parent_info = store
        .get_session(parent_key)
        .map_err(store_err)?
        .ok_or_else(|| SessionError::SessionNotFound {
            session: parent_key.to_string(),
        })?;
    let parent_messages = store.restore_session(parent_key).map_err(store_err)?;
    let available = parent_messages.len();
    if available == 0 {
        return Err(SessionError::EmptyParentSession {
            session: parent_key.to_string(),
        });
    }
    let point = fork_point.unwrap_or(available);
    if point == 0 || point > available {
        return Err(SessionError::ForkPointInvalid {
            requested: point,
            available,
        });
    }
    let prefix = &parent_messages[..point];
    let tokens_used = estimate_messages_tokens(prefix);

    // 1) 新会话的转录行（父会话只读）
    let child_key = allocate_child_key(store, parent_key);
    store
        .save_snapshot(&child_key, "idle", prefix, tokens_used)
        .map_err(store_err)?;

    // 2) 工作区（项目目录）与模型元数据。DB 归属优先；DB 为空时回退父台账的归属
    //    （`save_snapshot` 路径创建的会话只在台账里记项目根）。
    let parent_ctx = read_session_context(parent_key);
    let mut projects = store.project_paths_of(parent_key).map_err(store_err)?;
    if projects.is_empty() {
        if let Some(ctx) = parent_ctx.as_ref() {
            projects = projects_of(ctx);
        }
    }
    if !projects.is_empty() {
        store
            .bind_projects(&child_key, &projects)
            .map_err(store_err)?;
    }
    let (effort, context_window) = store.get_reasoning_settings(parent_key).map_err(store_err)?;
    store
        .set_reasoning_settings(&child_key, effort.as_deref(), context_window)
        .map_err(store_err)?;

    let child_title = resolve_child_title(title, &parent_info.summary);
    if !child_title.is_empty() {
        store
            .update_summary(&child_key, Some(child_title.as_str()))
            .map_err(store_err)?;
    }

    // 3) 台账（模型上下文 + 最近一份 CompactState）
    let is_full = point == available;
    let mut notes: Vec<String> = Vec::new();
    let mut compact_state_copied = false;
    let mut child_ctx = SessionContext {
        session: child_key.clone(),
        entries: Vec::new(),
        budget: DEFAULT_BUDGET,
        compaction_count: 0,
        last_compact_ms: None,
        summary: None,
        last_mode: None,
        compression_ratio: None,
        snapshot_count: 0,
        project_path: projects.first().cloned(),
        project_paths: projects.clone(),
        partial_text: None,
        overhead_tokens: 0,
        context_window: 0,
        compact_state: None,
    };
    match parent_ctx {
        Some(parent) => {
            child_ctx.budget = parent.budget;
            child_ctx.context_window = parent.context_window;
            if parent.entries.is_empty() {
                child_ctx.entries = entries_from_messages(prefix);
                notes.push(
                    "entries: 父会话台账无条目，子会话按转录前缀重建（token 为估算值）".to_string(),
                );
            } else if is_full {
                child_ctx.entries = parent.entries.clone();
            } else {
                child_ctx.entries = entries_from_messages(prefix);
                notes.push(
                    "entries: 前缀分支按转录前缀重建（父台账条目与消息行不是一一对应，无法按索引裁剪）"
                        .to_string(),
                );
            }
            if is_full {
                child_ctx.summary = parent.summary.clone();
                child_ctx.last_mode = parent.last_mode.clone();
                child_ctx.compression_ratio = parent.compression_ratio;
                child_ctx.compaction_count = parent.compaction_count;
                child_ctx.last_compact_ms = parent.last_compact_ms;
                child_ctx.overhead_tokens = parent.overhead_tokens;
                child_ctx.compact_state = parent.compact_state.clone();
                compact_state_copied = child_ctx.compact_state.is_some();
                if !compact_state_copied {
                    notes.push(
                        "compact_state: 父会话没有压缩交接状态（尚未压缩，或旧台账缺该键）"
                            .to_string(),
                    );
                }
            } else {
                notes.push(
                    "compact_state: 前缀分支无法还原「该轮之后才产生」的压缩交接状态，已置空"
                        .to_string(),
                );
                notes.push(
                    "overhead_tokens: 前缀分支无法拆分父会话的工具文本开销，已归零".to_string(),
                );
            }
        }
        None => {
            child_ctx.entries = entries_from_messages(prefix);
            notes.push("ledger: 父会话没有上下文台账，子会话按转录前缀重建".to_string());
            notes.push("compact_state: 父会话没有上下文台账，无交接状态可复制".to_string());
        }
    }
    notes.push(
        "snapshot_count: 预压缩快照不随分支复制（子会话不能回滚父会话的压缩前状态）".to_string(),
    );

    if let Err(err) = write_ledger(&child_ctx) {
        // 台账写不进去就不留半成品会话：否则侧栏会出现一条「有消息但模型没有记忆」的假分支。
        let _ = store.delete_session(&child_key);
        return Err(SessionError::Store {
            detail: format!("写入子会话台账失败（已回滚子会话行）：{err}"),
        });
    }

    let fork_mode = if is_full { "full" } else { "prefix" };
    Ok(ForkedSession {
        key: child_key.clone(),
        session: child_key,
        parent_key: parent_key.to_string(),
        fork_point: point,
        fork_mode: fork_mode.to_string(),
        message_count: point,
        tokens_used,
        title: child_title,
        compact_state_copied,
        notes,
    })
}

/// 按会话 key（+ 可选 checkpoint）恢复可继续状态。
///
/// `checkpoint`：
/// - `None` / `Some("latest")` → 当前台账（`live`）；
/// - `Some("cmp-<uuid>")` → 该会话的预压缩快照（`snapshot`），即压缩前的完整上下文；
/// - 其它值 → [`SessionError::CheckpointInvalid`]（附可用列表，绝不静默回落）。
pub fn resume_session(
    store: &SessionManager,
    session_key: &str,
    checkpoint: Option<&str>,
) -> Result<ResumedSession, SessionError> {
    let _info = store
        .get_session(session_key)
        .map_err(store_err)?
        .ok_or_else(|| SessionError::SessionNotFound {
            session: session_key.to_string(),
        })?;
    let frames = frames_of(store, session_key)?;
    let checkpoints = checkpoint_files(session_key);
    let available: Vec<String> = checkpoints
        .iter()
        .rev()
        .filter_map(|(_, path)| file_stem(path))
        .collect();

    let requested = checkpoint
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .filter(|text| *text != "latest");

    let mut notes: Vec<String> = Vec::new();
    let (checkpoint_source, ledger) = match requested {
        Some(id) => {
            let path = checkpoints
                .iter()
                .find(|(_, path)| file_stem(path).as_deref() == Some(id))
                .map(|(_, path)| path.clone())
                .ok_or_else(|| SessionError::CheckpointInvalid {
                    requested: id.to_string(),
                    available: available.clone(),
                })?;
            let raw =
                std::fs::read_to_string(&path).map_err(|err: io::Error| SessionError::Store {
                    detail: format!("读取 checkpoint 失败：{err}"),
                })?;
            let ctx: SessionContext = serde_json::from_str(&raw).map_err(
                |err: serde_json::Error| SessionError::Store {
                    detail: format!("解析 checkpoint 失败：{err}"),
                },
            )?;
            // 旧文件名不单射：内容里的 session 才是权威，不符即视为该 checkpoint 不可用。
            if ctx.session != session_key {
                return Err(SessionError::CheckpointInvalid {
                    requested: id.to_string(),
                    available: available.clone(),
                });
            }
            ("snapshot", Some(ctx))
        }
        None => ("live", read_session_context(session_key)),
    };

    let (budget, context_window, compact_state, summary, entries, project_paths) = match &ledger {
        Some(ctx) => (
            ctx.budget,
            ctx.context_window,
            ctx.compact_state.clone(),
            ctx.summary.clone(),
            ctx.entries.len(),
            projects_of(ctx),
        ),
        None => {
            notes.push(
                "ledger: 该会话没有上下文台账，只能恢复转录消息（模型上下文为空）".to_string(),
            );
            (DEFAULT_BUDGET, 0, None, None, 0, Vec::new())
        }
    };
    // 台账没记项目归属时回退 `sessions.db` 的归属（两条落盘路径可能只写了一边）。
    let project_paths = if project_paths.is_empty() {
        store.project_paths_of(session_key).map_err(store_err)?
    } else {
        project_paths
    };
    if compact_state.is_none() && ledger.is_some() {
        notes.push("compact_state: 该会话没有保存过压缩交接状态".to_string());
    }
    let resumable = !frames.is_empty() || entries > 0;

    Ok(ResumedSession {
        session: session_key.to_string(),
        checkpoint: requested.unwrap_or("latest").to_string(),
        checkpoint_source: checkpoint_source.to_string(),
        frame_count: frames.len(),
        frames,
        compact_state,
        summary,
        budget,
        context_window,
        project_paths,
        available_checkpoints: available,
        resumable,
        notes,
    })
}

/// 回放一个会话：按落库行序返回**只读**消息帧，不读取也不改动台账。
pub fn replay_session(
    store: &SessionManager,
    session_key: &str,
) -> Result<ReplayView, SessionError> {
    store
        .get_session(session_key)
        .map_err(store_err)?
        .ok_or_else(|| SessionError::SessionNotFound {
            session: session_key.to_string(),
        })?;
    let frames = frames_of(store, session_key)?;
    Ok(ReplayView {
        session: session_key.to_string(),
        read_only: true,
        frame_count: frames.len(),
        frames,
        notes: Vec::new(),
    })
}

fn store_err(err: anyhow::Error) -> SessionError {
    SessionError::Store {
        detail: err.to_string(),
    }
}

/// 消息行（`ORDER BY id`，由 `SessionManager::restore_session` 保证）→ 只读帧。
fn frames_of(store: &SessionManager, session_key: &str) -> Result<Vec<MessageFrame>, SessionError> {
    Ok(store
        .restore_session(session_key)
        .map_err(store_err)?
        .iter()
        .enumerate()
        .map(|(index, message)| frame_from_message(index, message))
        .collect())
}

fn frame_from_message(index: usize, message: &Message) -> MessageFrame {
    let (text, tool) = match message {
        Message::Tool { name, output, .. } => (output.clone(), Some(name.clone())),
        Message::User { content, .. }
        | Message::Assistant { content, .. }
        | Message::System { content, .. } => (content.clone(), None),
    };
    MessageFrame {
        index,
        role: message.role().to_string(),
        text,
        tool,
        timestamp: message.timestamp().to_rfc3339(),
        at_ms: message.timestamp().timestamp_millis(),
    }
}

/// 一条消息在模型上下文里的文本形态（工具消息把入参与输出一并计入）。
fn message_text(message: &Message) -> String {
    match message {
        Message::User { content, .. }
        | Message::Assistant { content, .. }
        | Message::System { content, .. } => content.clone(),
        Message::Tool {
            name,
            input,
            output,
            ..
        } => format!("[{name}] {input} -> {output}"),
    }
}

/// 按转录前缀重建台账条目（token 为启发式估算，非 tokenizer 精确值）。
fn entries_from_messages(messages: &[Message]) -> Vec<ContextEntry> {
    messages
        .iter()
        .map(|message| {
            let text = message_text(message);
            ContextEntry {
                role: message.role().to_string(),
                tokens: estimate_tokens(&text),
                text,
                at_ms: message.timestamp().timestamp_millis().max(0) as u64,
            }
        })
        .collect()
}

/// 消息 token 估算（饱和到 `u32`，不会溢出也不会回绕）。
fn estimate_messages_tokens(messages: &[Message]) -> u32 {
    let total: u64 = messages
        .iter()
        .map(|message| estimate_tokens(&message_text(message)))
        .sum();
    total.min(u32::MAX as u64) as u32
}

/// 子会话标题：显式标题优先；否则「父标题 + `(fork)`」（父标题为空则留空）。
fn resolve_child_title(title: Option<&str>, parent_title: &str) -> String {
    if let Some(explicit) = title.map(str::trim).filter(|text| !text.is_empty()) {
        return explicit.to_string();
    }
    let parent = parent_title.trim();
    if parent.is_empty() {
        String::new()
    } else {
        format!("{parent} (fork)")
    }
}

/// 项目目录集合：多根优先、旧单值回退（与台账读取口径一致）。
fn projects_of(ctx: &SessionContext) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for path in &ctx.project_paths {
        let path = path.trim();
        if !path.is_empty() && !out.iter().any(|seen| seen == path) {
            out.push(path.to_string());
        }
    }
    if out.is_empty() {
        if let Some(single) = ctx.project_path.as_deref().map(str::trim) {
            if !single.is_empty() {
                out.push(single.to_string());
            }
        }
    }
    out
}

/// 为子会话分配 key：`agent:<id>:webchat:fork-<12 hex>`，确保不与既有会话撞车。
fn allocate_child_key(store: &SessionManager, parent_key: &str) -> String {
    let agent_id = crate::agent::store::agent_id_from_session(parent_key).unwrap_or("main");
    for _ in 0..8 {
        let suffix: String = uuid::Uuid::new_v4()
            .simple()
            .to_string()
            .chars()
            .take(12)
            .collect();
        let key = format!("agent:{agent_id}:webchat:fork-{suffix}");
        if store.get_session(&key).ok().flatten().is_none() {
            return key;
        }
    }
    format!(
        "agent:{agent_id}:webchat:fork-{}",
        uuid::Uuid::new_v4().simple()
    )
}

/// 台账文件名（**必须**与 `context::session::safe_session_file_name` 逐字节一致）。
///
/// 复刻而不是调用：上游该函数是私有的。`ledger_stem_matches_context_directory_encoding`
/// 用例负责钉住编码，漂移即红。
fn ledger_stem(session: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut out = String::with_capacity(session.len());
    for ch in session.chars() {
        if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' || ch == '.' {
            out.push(ch);
        } else {
            let mut buf = [0u8; 4];
            for byte in ch.encode_utf8(&mut buf).bytes() {
                out.push('%');
                out.push(HEX[(byte >> 4) as usize] as char);
                out.push(HEX[(byte & 0x0f) as usize] as char);
            }
        }
    }
    out
}

/// 迁移前的旧文件名口径（把非法字符一律换成 `_`），只用于读旧快照。
fn legacy_ledger_stem(session: &str) -> String {
    session
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' || ch == '.' {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

/// 写入一份会话台账（原子写，父目录自动创建）。
fn write_ledger(ctx: &SessionContext) -> io::Result<()> {
    let stem = ledger_stem(&ctx.session);
    // 会话键可以来自客户端：`"."` / `".."` 会拼出逃逸路径，直接拒绝。
    if stem == "." || stem == ".." {
        return Err(io::Error::new(
            ErrorKind::InvalidInput,
            "会话键无法映射到安全的台账文件名",
        ));
    }
    let path = context_dir().join(format!("{stem}.json"));
    let raw = serde_json::to_string_pretty(ctx)
        .map_err(|err| io::Error::new(ErrorKind::InvalidData, err))?;
    crate::persistence::write_json_atomic(&path, &raw)
}

/// 该会话的预压缩快照（按 mtime **升序**，最旧在前）：新目录优先，空则回退旧目录。
fn checkpoint_files(session: &str) -> Vec<(SystemTime, PathBuf)> {
    let base = context_dir().join("snapshots");
    let candidates = [
        base.join(ledger_stem(session)),
        base.join(legacy_ledger_stem(session)),
    ];
    for dir in candidates {
        let files = files_by_mtime(&dir);
        if !files.is_empty() {
            return files;
        }
    }
    Vec::new()
}

fn files_by_mtime(dir: &Path) -> Vec<(SystemTime, PathBuf)> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut files: Vec<(SystemTime, PathBuf)> = entries
        .flatten()
        .filter(|entry| {
            entry
                .path()
                .extension()
                .map(|ext| ext == "json")
                .unwrap_or(false)
        })
        .map(|entry| {
            let modified = entry
                .metadata()
                .and_then(|meta| meta.modified())
                .unwrap_or(SystemTime::UNIX_EPOCH);
            (modified, entry.path())
        })
        .collect();
    files.sort_by_key(|(modified, _)| *modified);
    files
}

fn file_stem(path: &Path) -> Option<String> {
    path.file_stem()
        .and_then(|stem| stem.to_str())
        .map(|stem| stem.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    use chrono::Utc;

    fn temp_store(tag: &str) -> (SessionManager, PathBuf) {
        let dir = std::env::temp_dir().join(format!("mgt-fork-{tag}-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut store = SessionManager::new(&dir.join("sessions.db"));
        store.init().unwrap();
        (store, dir)
    }

    fn parent_key(tag: &str) -> String {
        format!("agent:main:webchat:parent-{tag}-{}", uuid::Uuid::new_v4())
    }

    fn user(text: &str) -> Message {
        Message::User {
            content: text.to_string(),
            timestamp: Utc::now(),
        }
    }

    fn assistant(text: &str) -> Message {
        Message::Assistant {
            content: text.to_string(),
            timestamp: Utc::now(),
            thinking: None,
        }
    }

    fn tool(name: &str, output: &str) -> Message {
        Message::Tool {
            name: name.to_string(),
            input: "{}".to_string(),
            output: output.to_string(),
            timestamp: Utc::now(),
            elapsed_ms: Some(12),
        }
    }

    fn seed(store: &SessionManager, key: &str, messages: &[Message]) {
        store.save_snapshot(key, "idle", messages, 0).unwrap();
    }

    fn sample_ledger(session: &str, messages: &[Message]) -> SessionContext {
        SessionContext {
            session: session.to_string(),
            entries: entries_from_messages(messages),
            budget: 96_000,
            compaction_count: 1,
            last_compact_ms: Some(7),
            summary: Some("## 会话摘要\n- 目标: 验证分支".to_string()),
            last_mode: Some("deterministic".to_string()),
            compression_ratio: Some(0.4),
            snapshot_count: 1,
            project_path: Some("D:/proj/demo".to_string()),
            project_paths: vec!["D:/proj/demo".to_string()],
            partial_text: None,
            overhead_tokens: 12,
            context_window: 128_000,
            compact_state: Some(CompactState {
                branch: Some("dev".to_string()),
                last_commit: Some("abc1234".to_string()),
                open_files: vec!["src/main.rs".to_string()],
                generated_at_ms: 99,
                ..Default::default()
            }),
        }
    }

    fn cleanup_ledger(keys: &[&str]) {
        for key in keys {
            for stem in [ledger_stem(key), legacy_ledger_stem(key)] {
                let _ = std::fs::remove_file(context_dir().join(format!("{stem}.json")));
                let _ = std::fs::remove_dir_all(context_dir().join("snapshots").join(stem));
            }
        }
    }

    fn cleanup(keys: &[&str], dir: &Path) {
        cleanup_ledger(keys);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// 复刻的台账文件名必须与 `context::session` 的私有编码一致：
    /// 写进去的台账必须能被 `read_session_context` 读回来（读路径用的是上游编码）。
    #[test]
    fn ledger_stem_matches_context_directory_encoding() {
        let session = format!("agent:main:webchat:enc-{}", uuid::Uuid::new_v4());
        let messages = vec![user("编码往返")];
        let ctx = sample_ledger(&session, &messages);
        write_ledger(&ctx).expect("写台账");
        let reread = read_session_context(&session).expect("上游编码必须能读到本模块写出的台账");
        assert_eq!(reread.session, session);
        assert_eq!(reread.entries.len(), 1);
        assert!(reread.compact_state.is_some());
        cleanup_ledger(&[session.as_str()]);
    }

    /// fork：复制前缀、父会话不变、两边各自追加互不影响。
    #[test]
    fn fork_copies_message_prefix_and_leaves_parent_untouched() {
        let (mut store, dir) = temp_store("prefix");
        let parent = parent_key("copy");
        seed(&store, &parent, &[user("Q1"), assistant("A1"), user("Q2")]);

        let forked = fork_session(&mut store, &parent, Some(2), Some("分支标题")).unwrap();
        assert_eq!(forked.fork_point, 2, "分支点应等于复制条数");
        assert_eq!(forked.message_count, 2);
        assert_eq!(forked.fork_mode, "prefix");
        assert_ne!(forked.key, parent, "必须返回新会话 key");
        assert_eq!(forked.parent_key, parent);

        let child = store.restore_session(&forked.key).unwrap();
        assert_eq!(child.len(), 2);
        assert_eq!(frames_of(&store, &forked.key).unwrap()[0].text, "Q1");
        assert_eq!(frames_of(&store, &forked.key).unwrap()[1].text, "A1");
        assert_eq!(
            store.get_session(&forked.key).unwrap().unwrap().summary,
            "分支标题",
            "显式标题必须写进子会话"
        );

        // 两边各自追加，互不影响（两份消息独立）
        store
            .append_message(
                &forked.key,
                "user",
                "子会话新增",
                "2026-10-30T00:00:00Z",
                None,
            )
            .unwrap();
        store
            .append_message(&parent, "user", "父会话新增", "2026-10-30T00:00:01Z", None)
            .unwrap();
        assert_eq!(store.restore_session(&forked.key).unwrap().len(), 3);
        assert_eq!(
            store.restore_session(&parent).unwrap().len(),
            4,
            "父会话只应增加自己的那条，不得被分支影响"
        );
        assert!(
            !store.restore_session(&parent).unwrap().iter().any(|message| matches!(
                message,
                Message::User { content, .. } if content == "子会话新增"
            )),
            "子会话的消息不得回流父会话"
        );

        drop(store);
        cleanup(&[parent.as_str(), forked.key.as_str()], &dir);
    }

    /// fork 点越界 / 空父会话 / 会话不存在都必须带明确错误码，不能伪成功。
    #[test]
    fn fork_point_out_of_range_and_missing_session_carry_error_codes() {
        let (mut store, dir) = temp_store("range");
        let parent = parent_key("range");
        seed(&store, &parent, &[user("only")]);

        let err = fork_session(&mut store, &parent, Some(5), None).unwrap_err();
        assert_eq!(err.code(), "FORK_POINT_INVALID");
        match err {
            SessionError::ForkPointInvalid {
                requested,
                available,
            } => {
                assert_eq!(requested, 5);
                assert_eq!(available, 1);
            }
            other => panic!("错误类型不符：{other:?}"),
        }

        let zero = fork_session(&mut store, &parent, Some(0), None).unwrap_err();
        assert_eq!(zero.code(), "FORK_POINT_INVALID", "0 条前缀不是可继续的分支");

        let missing = fork_session(&mut store, "agent:main:webchat:missing", None, None).unwrap_err();
        assert_eq!(missing.code(), "SESSION_NOT_FOUND");

        let empty_parent = parent_key("empty");
        store.touch_session(&empty_parent).unwrap();
        let empty = fork_session(&mut store, &empty_parent, None, None).unwrap_err();
        assert_eq!(empty.code(), "SESSION_EMPTY");

        drop(store);
        cleanup(&[parent.as_str(), empty_parent.as_str()], &dir);
    }

    /// 全量分支：resume 能取回消息**与**压缩状态（CompactState 从父台账复制）。
    #[test]
    fn resume_returns_messages_and_copied_compact_state() {
        let (mut store, dir) = temp_store("resume");
        let parent = parent_key("resume");
        let messages = vec![user("Q1"), assistant("A1"), user("Q2")];
        seed(&store, &parent, &messages);
        let parent_ledger = sample_ledger(&parent, &messages);
        write_ledger(&parent_ledger).unwrap();

        let forked = fork_session(&mut store, &parent, None, None).unwrap();
        assert_eq!(forked.fork_mode, "full");
        assert!(forked.compact_state_copied, "全量分支必须复制 CompactState");

        let resumed = resume_session(&store, &forked.key, None).unwrap();
        assert_eq!(resumed.checkpoint, "latest");
        assert_eq!(resumed.checkpoint_source, "live");
        assert_eq!(resumed.frame_count, 3, "消息必须全部取回");
        assert_eq!(resumed.frames[0].text, "Q1");
        assert_eq!(resumed.frames[1].role, "assistant");
        let state = resumed
            .compact_state
            .as_ref()
            .expect("resume 必须能取回压缩状态");
        assert_eq!(state.branch.as_deref(), Some("dev"));
        assert_eq!(state.last_commit.as_deref(), Some("abc1234"));
        assert_eq!(resumed.summary.as_deref(), parent_ledger.summary.as_deref());
        assert_eq!(resumed.budget, 96_000);
        assert_eq!(resumed.context_window, 128_000);
        assert_eq!(resumed.project_paths, vec!["D:/proj/demo".to_string()]);
        assert!(resumed.resumable);

        // 父台账必须原封未动
        let parent_reread = read_session_context(&parent).expect("父台账必须还在");
        assert_eq!(parent_reread.compact_state.as_ref().unwrap().branch.as_deref(), Some("dev"));
        assert_eq!(parent_reread.entries.len(), 3);

        drop(store);
        cleanup(&[parent.as_str(), forked.key.as_str()], &dir);
    }

    /// 前缀分支不得冒充拥有「父会话末尾的」压缩交接状态（宁缺毋滥）。
    #[test]
    fn prefix_fork_does_not_inherit_tip_compact_state() {
        let (mut store, dir) = temp_store("prefix-state");
        let parent = parent_key("prefix-state");
        let messages = vec![user("Q1"), assistant("A1")];
        seed(&store, &parent, &messages);
        write_ledger(&sample_ledger(&parent, &messages)).unwrap();

        let forked = fork_session(&mut store, &parent, Some(1), None).unwrap();
        assert!(!forked.compact_state_copied, "前缀分支不得复制末尾的 CompactState");
        assert!(
            forked.notes.iter().any(|note| note.contains("compact_state")),
            "未复制必须写明原因：{:?}",
            forked.notes
        );

        let resumed = resume_session(&store, &forked.key, None).unwrap();
        assert!(resumed.compact_state.is_none());
        assert!(resumed.notes.iter().any(|note| note.contains("compact_state")));
        assert_eq!(resumed.frame_count, 1);

        drop(store);
        cleanup(&[parent.as_str(), forked.key.as_str()], &dir);
    }

    /// resume 支持按预压缩快照 checkpoint 恢复；未知 checkpoint 必须报错并给出可用列表。
    #[test]
    fn resume_accepts_real_checkpoint_and_rejects_unknown_one() {
        let (mut store, dir) = temp_store("checkpoint");
        let parent = parent_key("checkpoint");
        let messages = vec![user("Q1"), assistant("A1")];
        seed(&store, &parent, &messages);
        write_ledger(&sample_ledger(&parent, &messages)).unwrap();
        let forked = fork_session(&mut store, &parent, None, None).unwrap();

        // 造一份真实的压缩前快照（与 ContextLedger::save_snapshot 同路径同结构）
        let snap_dir = context_dir().join("snapshots").join(ledger_stem(&forked.key));
        std::fs::create_dir_all(&snap_dir).unwrap();
        let snapshot = sample_ledger(&forked.key, &messages);
        let snapshot = SessionContext {
            summary: Some("## 会话摘要\n- 压缩前".to_string()),
            compact_state: None,
            ..snapshot
        };
        std::fs::write(
            snap_dir.join("cmp-before.json"),
            serde_json::to_string(&snapshot).unwrap(),
        )
        .unwrap();

        let resumed = resume_session(&store, &forked.key, Some("cmp-before")).unwrap();
        assert_eq!(resumed.checkpoint, "cmp-before");
        assert_eq!(resumed.checkpoint_source, "snapshot");
        assert_eq!(
            resumed.summary.as_deref(),
            Some("## 会话摘要\n- 压缩前"),
            "checkpoint 恢复必须使用快照内容，而不是当前台账"
        );
        assert!(resumed
            .available_checkpoints
            .contains(&"cmp-before".to_string()));

        let missing = resume_session(&store, &forked.key, Some("cmp-nope")).unwrap_err();
        assert_eq!(missing.code(), "CHECKPOINT_INVALID");
        match missing {
            SessionError::CheckpointInvalid { available, .. } => {
                assert!(available.contains(&"cmp-before".to_string()));
            }
            other => panic!("错误类型不符：{other:?}"),
        }

        drop(store);
        cleanup(&[parent.as_str(), forked.key.as_str()], &dir);
    }

    /// replay：顺序稳定、只读、工具帧带工具名；会话不存在报 SESSION_NOT_FOUND。
    #[test]
    fn replay_keeps_transcript_order_and_is_read_only() {
        let (store, dir) = temp_store("replay");
        let key = parent_key("replay");
        seed(
            &store,
            &key,
            &[
                user("第一步"),
                assistant("回复一"),
                tool("read_file", "文件内容"),
                user("第二步"),
            ],
        );

        let view = replay_session(&store, &key).unwrap();
        assert!(view.read_only, "回放必须只读");
        assert_eq!(view.frame_count, 4);
        let roles: Vec<&str> = view.frames.iter().map(|frame| frame.role.as_str()).collect();
        assert_eq!(roles, vec!["user", "assistant", "tool", "user"]);
        let texts: Vec<&str> = view.frames.iter().map(|frame| frame.text.as_str()).collect();
        assert_eq!(texts, vec!["第一步", "回复一", "文件内容", "第二步"]);
        assert_eq!(view.frames[2].tool.as_deref(), Some("read_file"));
        assert_eq!(view.frames[0].index, 0);
        assert_eq!(view.frames[3].index, 3);

        // 再回放一次：帧必须逐字段相同（顺序稳定，不含任何随机/时间漂移）
        let again = replay_session(&store, &key).unwrap();
        assert_eq!(again.frames, view.frames);

        let missing = replay_session(&store, "agent:main:webchat:missing").unwrap_err();
        assert_eq!(missing.code(), "SESSION_NOT_FOUND");

        drop(store);
        cleanup(&[key.as_str()], &dir);
    }
}
