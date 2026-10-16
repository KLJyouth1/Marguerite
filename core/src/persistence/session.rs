//! 会话持久化
//!
//! 会话快照的保存、恢复、列表管理。

use std::path::{Path, PathBuf};

use anyhow::Result;
use chrono::Utc;
use rusqlite::Connection;

use crate::types::Message;

/// 会话信息
#[derive(Debug, Clone)]
pub struct SessionInfo {
    /// 会话 ID
    pub session_id: String,
    /// 摘要内容
    pub summary: String,
    /// 消息数量
    pub message_count: usize,
    /// 状态
    pub state: String,
    /// 创建时间
    pub created_at: String,
    /// 更新时间
    pub updated_at: String,
    /// 使用的 Token 数
    pub tokens_used: u32,
    /// 关联的项目路径（None = 未关联到具体项目）
    pub project_path: Option<String>,
    /// 会话级思考强度覆盖（off/normal/deep/max；None = 回退全局模型配置）
    pub reasoning_effort: Option<String>,
    /// 会话级上下文窗口覆盖（token 数；None = 回退全局模型配置）
    pub context_window: Option<u32>,
}

/// 会话快照
#[derive(Debug, Clone)]
pub struct SessionSnapshot {
    /// 会话 ID
    pub session_id: String,
    /// 状态
    pub state: String,
    /// 消息列表
    pub messages: Vec<Message>,
    /// 使用的 Token 数
    pub tokens_used: u32,
    /// 创建时间
    pub created_at: String,
    /// 更新时间
    pub updated_at: String,
}

/// 会话管理器
///
/// 负责会话快照的保存、恢复、列表管理。
pub struct SessionManager {
    db_path: PathBuf,
    db: Option<Connection>,
}

impl SessionManager {
    /// 创建会话管理器
    pub fn new(path: &Path) -> Self {
        Self {
            db_path: path.to_path_buf(),
            db: None,
        }
    }

    /// 初始化数据库
    pub fn init(&mut self) -> Result<()> {
        let db = Connection::open(&self.db_path)?;
        db.execute_batch(
            "CREATE TABLE IF NOT EXISTS sessions (
                session_id TEXT PRIMARY KEY,
                summary TEXT NOT NULL,
                state TEXT NOT NULL DEFAULT 'Ready',
                message_count INTEGER DEFAULT 0,
                tokens_used INTEGER DEFAULT 0,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS session_messages (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                session_id TEXT NOT NULL,
                role TEXT NOT NULL,
                content TEXT NOT NULL,
                thinking TEXT,
                tool_name TEXT,
                tool_input TEXT,
                tool_output TEXT,
                elapsed_ms INTEGER,
                timestamp TEXT NOT NULL,
                FOREIGN KEY (session_id) REFERENCES sessions(session_id)
            );",
        )?;
        // 迁移：为旧库补充 project_path 列（幂等：先查列是否存在）
        let has_project_col: bool = db
            .prepare("PRAGMA table_info(sessions)")?
            .query_map([], |row| row.get::<_, String>(1))?
            .filter_map(|r| r.ok())
            .any(|name| name == "project_path"); // 幂等迁移范本（本文件后续迁移照此）
        if !has_project_col {
            db.execute_batch("ALTER TABLE sessions ADD COLUMN project_path TEXT;")?;
        }
        // 迁移（P2-10/T14）：为旧库补充工具耗时列。**只增列**，旧快照该列为 NULL
        // → 历史工具卡不显示耗时（诚实降级）。
        let has_elapsed_col: bool = db
            .prepare("PRAGMA table_info(session_messages)")?
            .query_map([], |row| row.get::<_, String>(1))?
            .filter_map(|r| r.ok())
            .any(|name| name == "elapsed_ms");
        if !has_elapsed_col {
            db.execute_batch("ALTER TABLE session_messages ADD COLUMN elapsed_ms INTEGER;")?;
        }
        // 迁移（成本治理）：会话级「思考强度 + 上下文窗口」持久化。会话可独立
        // 于全局模型配置指定 reasoning_effort（off/normal/deep/max）与 context_window，
        // 重启后按会话恢复，避免长程任务被全局默认档位拖垮成本或质量。
        // 幂等：只增列，旧库 NULL → 回退全局配置（诚实降级）。
        let has_effort_col: bool = db
            .prepare("PRAGMA table_info(sessions)")?
            .query_map([], |row| row.get::<_, String>(1))?
            .filter_map(|r| r.ok())
            .any(|name| name == "reasoning_effort");
        if !has_effort_col {
            db.execute_batch("ALTER TABLE sessions ADD COLUMN reasoning_effort TEXT;")?;
        }
        let has_ctx_col: bool = db
            .prepare("PRAGMA table_info(sessions)")?
            .query_map([], |row| row.get::<_, String>(1))?
            .filter_map(|r| r.ok())
            .any(|name| name == "context_window");
        if !has_ctx_col {
            db.execute_batch("ALTER TABLE sessions ADD COLUMN context_window INTEGER;")?;
        }
        self.db = Some(db);
        Ok(())
    }

    /// 获取数据库连接引用
    fn db(&self) -> Result<&Connection> {
        self.db
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Database not initialized"))
    }

    /// 保存快照
    pub fn save_snapshot(
        &self,
        session_id: &str,
        state: &str,
        messages: &[Message],
        tokens: u32,
    ) -> Result<()> {
        let db = self.db()?;
        let now = Utc::now().to_rfc3339();
        let message_count = messages.len();
        let summary = Self::generate_summary(messages);

        let tx = db.unchecked_transaction()?;
        {
            // COALESCE 保留已有 project_path：无归属的调用（INSERT OR REPLACE 会重置未列出的列）
            // 不得把曾绑定的项目会话抹成 NULL，否则会话重启后掉进「最近」栏。
            tx.execute(
                "INSERT OR REPLACE INTO sessions (session_id, summary, state, message_count, tokens_used, created_at, updated_at, project_path)
                 VALUES (?1, ?2, ?3, ?4, ?5, COALESCE((SELECT created_at FROM sessions WHERE session_id = ?1), ?6), ?6, COALESCE((SELECT project_path FROM sessions WHERE session_id = ?1), NULL))",
                rusqlite::params![session_id, summary, state, message_count as i32, tokens, now],
            )?;

            // 删除该会话的旧消息
            tx.execute(
                "DELETE FROM session_messages WHERE session_id = ?1",
                rusqlite::params![session_id],
            )?;

            let mut stmt = tx.prepare(
                "INSERT INTO session_messages (session_id, role, content, thinking, tool_name, tool_input, tool_output, elapsed_ms, timestamp)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            )?;
            for msg in messages {
                let (role, content, thinking, tool_name, tool_input, tool_output, elapsed_ms) =
                    extract_message_fields(msg);
                stmt.execute(rusqlite::params![
                    session_id,
                    role,
                    content,
                    thinking,
                    tool_name,
                    tool_input,
                    tool_output,
                    elapsed_ms,
                    msg.timestamp().to_rfc3339(),
                ])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// 保存快照（关联到具体项目路径）
    ///
    /// 与 [`save_snapshot`] 等价，但额外写入 `project_path`，
    /// 供桌面端「任务列表 → 新建会话」按项目归类与恢复。
    pub fn save_snapshot_for_project(
        &self,
        session_id: &str,
        project_path: &str,
        state: &str,
        messages: &[Message],
        tokens: u32,
    ) -> Result<()> {
        let db = self.db()?;
        let now = Utc::now().to_rfc3339();
        let message_count = messages.len();
        let summary = Self::generate_summary(messages);

        let tx = db.unchecked_transaction()?;
        {
            tx.execute(
                "INSERT OR REPLACE INTO sessions (session_id, summary, state, message_count, tokens_used, created_at, updated_at, project_path)
                 VALUES (?1, ?2, ?3, ?4, ?5, COALESCE((SELECT created_at FROM sessions WHERE session_id = ?1), ?6), ?6, ?7)",
                rusqlite::params![session_id, summary, state, message_count as i32, tokens, now, project_path],
            )?;

            tx.execute(
                "DELETE FROM session_messages WHERE session_id = ?1",
                rusqlite::params![session_id],
            )?;

            let mut stmt = tx.prepare(
                "INSERT INTO session_messages (session_id, role, content, thinking, tool_name, tool_input, tool_output, elapsed_ms, timestamp)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            )?;
            for msg in messages {
                let (role, content, thinking, tool_name, tool_input, tool_output, elapsed_ms) =
                    extract_message_fields(msg);
                stmt.execute(rusqlite::params![
                    session_id,
                    role,
                    content,
                    thinking,
                    tool_name,
                    tool_input,
                    tool_output,
                    elapsed_ms,
                    msg.timestamp().to_rfc3339(),
                ])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// 按项目路径列出会话（任务列表按项目归类）。
    pub fn list_sessions_by_project(
        &self,
        project_path: &str,
        limit: usize,
    ) -> Result<Vec<SessionInfo>> {
        let db = self.db()?;
        let mut stmt = db.prepare(
            "SELECT session_id, summary, message_count, state, created_at, updated_at, tokens_used, project_path, reasoning_effort, context_window
             FROM sessions WHERE project_path = ?1 ORDER BY updated_at DESC LIMIT ?2",
        )?;

        let results = stmt
            .query_map(rusqlite::params![project_path, limit as i64], |row| {
                Ok(SessionInfo {
                    session_id: row.get(0)?,
                    summary: row.get(1)?,
                    message_count: row.get::<_, i32>(2)? as usize,
                    state: row.get(3)?,
                    created_at: row.get(4)?,
                    updated_at: row.get(5)?,
                    tokens_used: row.get::<_, i32>(6)? as u32,
                    project_path: row.get(7)?,
                    reasoning_effort: row.get(8)?,
                    context_window: row.get::<_, Option<i64>>(9)?.map(|v| v as u32),
                })
            })?
            .filter_map(|r| r.ok())
            .collect();

        Ok(results)
    }

    /// 读取会话级「思考强度 + 上下文窗口」覆盖（成本治理）。
    /// 返回 `(reasoning_effort, context_window)`；两者均可为 None = 未覆盖，
    /// 调用方应回退全局模型配置（诚实降级，不撒谎说"已设置"）。
    pub fn get_reasoning_settings(
        &self,
        session_id: &str,
    ) -> Result<(Option<String>, Option<u32>)> {
        let db = self.db()?;
        let row = db.query_row(
            "SELECT reasoning_effort, context_window FROM sessions WHERE session_id = ?1",
            rusqlite::params![session_id],
            |row| {
                Ok((
                    row.get::<_, Option<String>>(0)?,
                    row.get::<_, Option<i64>>(1)?.map(|v| v as u32),
                ))
            },
        );
        match row {
            Ok(v) => Ok(v),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok((None, None)),
            Err(e) => Err(e.into()),
        }
    }

    /// 写入会话级「思考强度 + 上下文窗口」覆盖（成本治理）。
    /// 传 `None` 表示清除覆盖、回退全局配置；`context_window` 传 `Some(0)` 同样视为清除
    /// （0 不是合法上下文长度）。
    pub fn set_reasoning_settings(
        &self,
        session_id: &str,
        reasoning_effort: Option<&str>,
        context_window: Option<u32>,
    ) -> Result<()> {
        let db = self.db()?;
        // 归一化：空串 / 0 一律视作"清除覆盖"，避免存入语义模糊的哨兵值。
        let effort = reasoning_effort
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string);
        let ctx = context_window.filter(|v| *v > 0).map(|v| v as i64);
        let n = db.execute(
            "UPDATE sessions SET reasoning_effort = ?2, context_window = ?3 WHERE session_id = ?1",
            rusqlite::params![session_id, effort, ctx],
        )?;
        if n == 0 {
            anyhow::bail!("会话不存在: {session_id}");
        }
        Ok(())
    }

    /// 加载快照
    pub fn load_snapshot(&self, session_id: &str) -> Result<SessionSnapshot> {
        let db = self.db()?;

        let session = db.query_row(
            "SELECT session_id, state, tokens_used, created_at, updated_at
             FROM sessions WHERE session_id = ?1",
            rusqlite::params![session_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i32>(2)? as u32,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                ))
            },
        )?;

        let messages = self.restore_session(session_id)?;

        Ok(SessionSnapshot {
            session_id: session.0,
            state: session.1,
            messages,
            tokens_used: session.2,
            created_at: session.3,
            updated_at: session.4,
        })
    }

    /// 列出会话
    pub fn list_sessions(&self, limit: usize) -> Result<Vec<SessionInfo>> {
        let db = self.db()?;
        let mut stmt = db.prepare(
            "SELECT session_id, summary, message_count, state, created_at, updated_at, tokens_used, project_path, reasoning_effort, context_window
             FROM sessions ORDER BY updated_at DESC LIMIT ?1",
        )?;

        let results = stmt
            .query_map(rusqlite::params![limit as i64], |row| {
                Ok(SessionInfo {
                    session_id: row.get(0)?,
                    summary: row.get(1)?,
                    message_count: row.get::<_, i32>(2)? as usize,
                    state: row.get(3)?,
                    created_at: row.get(4)?,
                    updated_at: row.get(5)?,
                    tokens_used: row.get::<_, i32>(6)? as u32,
                    project_path: row.get(7)?,
                    reasoning_effort: row.get(8)?,
                    context_window: row.get::<_, Option<i64>>(9)?.map(|v| v as u32),
                })
            })?
            .filter_map(|r| r.ok())
            .collect();

        Ok(results)
    }

    /// 删除会话
    pub fn delete_session(&self, session_id: &str) -> Result<()> {
        let db = self.db()?;
        let tx = db.unchecked_transaction()?;
        {
            tx.execute(
                "DELETE FROM session_messages WHERE session_id = ?1",
                rusqlite::params![session_id],
            )?;
            tx.execute(
                "DELETE FROM sessions WHERE session_id = ?1",
                rusqlite::params![session_id],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// 获取单个会话信息
    pub fn get_session(&self, session_id: &str) -> Result<Option<SessionInfo>> {
        let db = self.db()?;
        let result = db.query_row(
            "SELECT session_id, summary, message_count, state, created_at, updated_at, tokens_used, project_path, reasoning_effort, context_window
             FROM sessions WHERE session_id = ?1",
            rusqlite::params![session_id],
            |row| {
                Ok(SessionInfo {
                    session_id: row.get(0)?,
                    summary: row.get(1)?,
                    message_count: row.get::<_, i32>(2)? as usize,
                    state: row.get(3)?,
                    created_at: row.get(4)?,
                    updated_at: row.get(5)?,
                    tokens_used: row.get::<_, i32>(6)? as u32,
                    project_path: row.get(7)?,
                    reasoning_effort: row.get(8)?,
                    context_window: row.get::<_, Option<i64>>(9)?.map(|v| v as u32),
                })
            },
        );

        match result {
            Ok(info) => Ok(Some(info)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// 更新会话摘要/标题（sessions.patch RPC 后端实现）
    pub fn update_summary(&self, session_id: &str, title: Option<&str>) -> Result<()> {
        let db = self.db()?;
        match title {
            Some(t) if !t.is_empty() => {
                db.execute(
                    "UPDATE sessions SET summary = ?1, updated_at = ?2 WHERE session_id = ?3",
                    rusqlite::params![t, Utc::now().to_rfc3339(), session_id],
                )?;
            }
            _ => {}
        }
        Ok(())
    }

    /// 清除会话所有消息（保留会话记录本身，sessions.reset RPC 后端实现）
    pub fn clear_messages(&self, session_id: &str) -> Result<()> {
        let db = self.db()?;
        db.execute(
            "DELETE FROM session_messages WHERE session_id = ?1",
            rusqlite::params![session_id],
        )?;
        db.execute(
            "UPDATE sessions SET message_count = 0, updated_at = ?1 WHERE session_id = ?2",
            rusqlite::params![Utc::now().to_rfc3339(), session_id],
        )?;
        Ok(())
    }

    /// 恢复会话的所有消息
    pub fn restore_session(&self, session_id: &str) -> Result<Vec<Message>> {
        let db = self.db()?;
        let mut stmt = db.prepare(
            "SELECT role, content, thinking, tool_name, tool_input, tool_output, timestamp, elapsed_ms
             FROM session_messages WHERE session_id = ?1 ORDER BY id",
        )?;

        let messages: Vec<Message> = stmt
            .query_map(rusqlite::params![session_id], |row| {
                let role: String = row.get(0)?;
                let content: String = row.get(1)?;
                let thinking: Option<String> = row.get(2)?;
                let tool_name: Option<String> = row.get(3)?;
                let tool_input: Option<String> = row.get(4)?;
                let tool_output: Option<String> = row.get(5)?;
                let timestamp_str: String = row.get(6)?;
                // P2-10/T14：工具耗时列（旧库迁移后为 NULL → None，历史卡不显示耗时）。
                let elapsed_ms: Option<i64> = row.get(7)?;
                let timestamp = timestamp_str
                    .parse::<chrono::DateTime<Utc>>()
                    .unwrap_or_else(|_| Utc::now());

                let msg = match role.as_str() {
                    "user" => Message::User { content, timestamp },
                    "assistant" => Message::Assistant {
                        content,
                        thinking,
                        timestamp,
                    },
                    "system" => Message::System { content, timestamp },
                    "tool" => Message::Tool {
                        name: tool_name.unwrap_or_default(),
                        input: tool_input.unwrap_or_default(),
                        output: tool_output.unwrap_or_default(),
                        timestamp,
                        elapsed_ms: elapsed_ms.and_then(|ms| u64::try_from(ms).ok()),
                    },
                    _ => Message::System { content, timestamp },
                };
                Ok(msg)
            })?
            .filter_map(|r| r.ok())
            .collect();

        Ok(messages)
    }

    /// 生成会话摘要（基于消息内容）
    fn generate_summary(messages: &[Message]) -> String {
        if messages.is_empty() {
            return "Empty session".to_string();
        }
        // 取第一条用户消息的前 100 个字符作为摘要
        for msg in messages {
            if let Message::User { content, .. } = msg {
                let truncated: String = content.chars().take(100).collect();
                return if content.len() > 100 {
                    format!("{}...", truncated)
                } else {
                    truncated
                };
            }
        }
        // 如果没有用户消息，取第一条消息
        let first = &messages[0];
        let content = match first {
            Message::User { content, .. } => content.as_str(),
            Message::Assistant { content, .. } => content.as_str(),
            Message::System { content, .. } => content.as_str(),
            Message::Tool { name, .. } => name.as_str(),
        };
        let truncated: String = content.chars().take(100).collect();
        if content.len() > 100 {
            format!("{}...", truncated)
        } else {
            truncated
        }
    }
}

/// 从消息中提取数据库字段（P2-10/T14：末位为工具耗时 `elapsed_ms`，非工具消息为 None）
#[allow(clippy::type_complexity)]
fn extract_message_fields(
    msg: &Message,
) -> (
    &str,
    &str,
    Option<&str>,
    Option<&str>,
    Option<&str>,
    Option<&str>,
    Option<i64>,
) {
    match msg {
        Message::User { content, .. } => (msg.role(), content.as_str(), None, None, None, None, None),
        Message::Assistant {
            content, thinking, ..
        } => (
            msg.role(),
            content.as_str(),
            thinking.as_deref(),
            None,
            None,
            None,
            None,
        ),
        Message::System { content, .. } => (msg.role(), content.as_str(), None, None, None, None, None),
        Message::Tool {
            name,
            input,
            output,
            elapsed_ms,
            ..
        } => (
            msg.role(),
            "",
            None,
            Some(name.as_str()),
            Some(input.as_str()),
            Some(output.as_str()),
            elapsed_ms.map(|ms| ms as i64),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Message;
    use chrono::Utc;

    fn setup_manager(name: &str) -> (SessionManager, PathBuf) {
        let tmp_dir = std::env::temp_dir().join(format!("marguerite_test_{}", name));
        std::fs::create_dir_all(&tmp_dir).unwrap();
        let db_path = tmp_dir.join("sessions.db");
        let mut manager = SessionManager::new(&db_path);
        manager.init().unwrap();
        (manager, tmp_dir)
    }

    #[test]
    fn test_session_save_and_load_snapshot() {
        let (manager, tmp_dir) = setup_manager("ss_save_load");
        let now = Utc::now();

        let messages = vec![
            Message::User {
                content: "Hello".to_string(),
                timestamp: now,
            },
            Message::Assistant {
                content: "Hi there".to_string(),
                timestamp: now,
                thinking: None,
            },
        ];

        manager
            .save_snapshot("session-1", "Ready", &messages, 100)
            .unwrap();

        let snapshot = manager.load_snapshot("session-1").unwrap();
        assert_eq!(snapshot.session_id, "session-1");
        assert_eq!(snapshot.state, "Ready");
        assert_eq!(snapshot.messages.len(), 2);
        assert_eq!(snapshot.tokens_used, 100);

        std::fs::remove_dir_all(&tmp_dir).unwrap_or(());
    }

    #[test]
    fn test_session_list_sessions() {
        let (manager, tmp_dir) = setup_manager("ss_list");
        let now = Utc::now();

        for i in 0..3 {
            let messages = vec![Message::User {
                content: format!("msg {}", i),
                timestamp: now,
            }];
            manager
                .save_snapshot(&format!("session-{}", i), "Ready", &messages, i as u32 * 10)
                .unwrap();
        }

        let sessions = manager.list_sessions(10).unwrap();
        assert_eq!(sessions.len(), 3);

        // 最新的在前
        assert_eq!(sessions[0].session_id, "session-2");

        std::fs::remove_dir_all(&tmp_dir).unwrap_or(());
    }

    #[test]
    fn test_session_delete_session() {
        let (manager, tmp_dir) = setup_manager("ss_delete");
        let now = Utc::now();

        let messages = vec![Message::User {
            content: "test".to_string(),
            timestamp: now,
        }];
        manager
            .save_snapshot("session-1", "Ready", &messages, 50)
            .unwrap();
        assert_eq!(manager.list_sessions(10).unwrap().len(), 1);

        manager.delete_session("session-1").unwrap();
        assert_eq!(manager.list_sessions(10).unwrap().len(), 0);

        std::fs::remove_dir_all(&tmp_dir).unwrap_or(());
    }

    #[test]
    fn test_session_get_session() {
        let (manager, tmp_dir) = setup_manager("ss_get");
        let now = Utc::now();

        let messages = vec![Message::User {
            content: "Hello world".to_string(),
            timestamp: now,
        }];
        manager
            .save_snapshot("session-1", "Running", &messages, 75)
            .unwrap();

        let info = manager.get_session("session-1").unwrap().unwrap();
        assert_eq!(info.session_id, "session-1");
        assert_eq!(info.state, "Running");
        assert_eq!(info.message_count, 1);
        assert_eq!(info.tokens_used, 75);

        // 不存在的会话
        let info = manager.get_session("non-existent").unwrap();
        assert!(info.is_none());

        std::fs::remove_dir_all(&tmp_dir).unwrap_or(());
    }

    #[test]
    fn test_session_restore_session() {
        let (manager, tmp_dir) = setup_manager("ss_restore");
        let now = Utc::now();

        let messages = vec![
            Message::User {
                content: "Hello".to_string(),
                timestamp: now,
            },
            Message::Assistant {
                content: "Hi".to_string(),
                timestamp: now,
                thinking: Some("thinking...".to_string()),
            },
            Message::Tool {
                name: "calc".to_string(),
                input: "1+1".to_string(),
                output: "2".to_string(),
                timestamp: now,
                // P2-10/T14：带上真实耗时，验证迁移列往返。
                elapsed_ms: Some(1234),
            },
        ];

        manager
            .save_snapshot("session-1", "Ready", &messages, 100)
            .unwrap();

        let restored = manager.restore_session("session-1").unwrap();
        assert_eq!(restored.len(), 3);
        assert_eq!(restored[0].role(), "user");
        assert_eq!(restored[1].role(), "assistant");
        assert_eq!(restored[2].role(), "tool");
        // P2-10/T14：工具耗时列往返（迁移列 + INSERT + SELECT）。
        match &restored[2] {
            Message::Tool { elapsed_ms, .. } => {
                assert_eq!(*elapsed_ms, Some(1234), "工具耗时必须原样往返")
            }
            other => panic!("第三条应为工具消息，实际 {:?}", other.role()),
        }

        std::fs::remove_dir_all(&tmp_dir).unwrap_or(());
    }

    #[test]
    fn test_session_summary_generation() {
        let (manager, tmp_dir) = setup_manager("ss_summary");
        let now = Utc::now();

        let messages = vec![Message::User {
            content: "What is the weather today in Beijing China?".to_string(),
            timestamp: now,
        }];

        manager
            .save_snapshot("session-1", "Ready", &messages, 10)
            .unwrap();

        let info = manager.get_session("session-1").unwrap().unwrap();
        assert!(info.summary.contains("What is the weather"));

        // 测试没有用户消息的情况
        let messages = vec![Message::Assistant {
            content: "Hello, how can I help?".to_string(),
            timestamp: now,
            thinking: None,
        }];
        manager
            .save_snapshot("session-2", "Ready", &messages, 5)
            .unwrap();
        let info = manager.get_session("session-2").unwrap().unwrap();
        assert!(info.summary.contains("Hello"));

        std::fs::remove_dir_all(&tmp_dir).unwrap_or(());
    }

    #[test]
    fn test_session_empty_snapshot() {
        let (manager, tmp_dir) = setup_manager("ss_empty");

        let messages = vec![];
        manager
            .save_snapshot("session-1", "Ready", &messages, 0)
            .unwrap();

        let info = manager.get_session("session-1").unwrap().unwrap();
        assert_eq!(info.summary, "Empty session");

        std::fs::remove_dir_all(&tmp_dir).unwrap_or(());
    }

    #[test]
    fn test_session_project_scoped_apis() {
        let (manager, tmp_dir) = setup_manager("ss_project");
        let now = Utc::now();

        let messages = vec![Message::User {
            content: "优化桌面端".to_string(),
            timestamp: now,
        }];
        manager
            .save_snapshot_for_project("proj-a-1", "/dev/marguerite", "Ready", &messages, 12)
            .unwrap();
        manager
            .save_snapshot_for_project("proj-a-2", "/dev/marguerite", "Ready", &[], 0)
            .unwrap();
        manager
            .save_snapshot_for_project("proj-b-1", "/sandbox/7f3a", "Ready", &[], 0)
            .unwrap();

        // 按项目过滤
        let a = manager
            .list_sessions_by_project("/dev/marguerite", 10)
            .unwrap();
        assert_eq!(a.len(), 2);
        assert!(a
            .iter()
            .all(|s| s.project_path.as_deref() == Some("/dev/marguerite")));
        let b = manager
            .list_sessions_by_project("/sandbox/7f3a", 10)
            .unwrap();
        assert_eq!(b.len(), 1);

        // 全量列表也带 project_path
        let all = manager.list_sessions(10).unwrap();
        assert!(all
            .iter()
            .any(|s| s.session_id == "proj-a-1" && s.project_path.is_some()));

        std::fs::remove_dir_all(&tmp_dir).unwrap_or(());
    }

    #[test]
    fn test_save_snapshot_preserves_project_path() {
        // 回归：无归属的 save_snapshot 不得把已绑定项目的会话抹成 NULL（否则重启后掉进「最近」）
        let (manager, tmp_dir) = setup_manager("ss_preserve");
        let now = Utc::now();

        manager
            .save_snapshot_for_project(
                "s1",
                "/dev/marguerite",
                "Ready",
                &[Message::User {
                    content: "a".to_string(),
                    timestamp: now,
                }],
                0,
            )
            .unwrap();

        // 后续无 project 的 save_snapshot 不应清除 project_path
        manager
            .save_snapshot(
                "s1",
                "idle",
                &[Message::User {
                    content: "b".to_string(),
                    timestamp: now,
                }],
                0,
            )
            .unwrap();

        let info = manager.get_session("s1").unwrap().unwrap();
        assert_eq!(info.project_path.as_deref(), Some("/dev/marguerite"));
        assert_eq!(info.message_count, 1); // 消息被覆盖重建，仍为 1 条
        assert_eq!(info.summary, "b");

        std::fs::remove_dir_all(&tmp_dir).unwrap_or(());
    }

    #[test]
    fn test_session_migration_adds_project_column() {
        // 旧库（无 project_path 列）init 后应自动补列，不 panic
        let tmp_dir = std::env::temp_dir().join("mg_test_migration");
        let _ = std::fs::remove_dir_all(&tmp_dir); // 清理残留，保证幂等
        std::fs::create_dir_all(&tmp_dir).unwrap();
        let db_path = tmp_dir.join("sessions.db");
        {
            let db = rusqlite::Connection::open(&db_path).unwrap();
            db.execute_batch(
                "CREATE TABLE sessions (
                    session_id TEXT PRIMARY KEY,
                    summary TEXT NOT NULL,
                    state TEXT NOT NULL DEFAULT 'Ready',
                    message_count INTEGER DEFAULT 0,
                    tokens_used INTEGER DEFAULT 0,
                    created_at TEXT NOT NULL,
                    updated_at TEXT NOT NULL
                );",
            )
            .unwrap();
        }
        let mut manager = SessionManager::new(&db_path);
        manager.init().unwrap();
        let info = manager.get_session("none").unwrap();
        assert!(info.is_none(), "旧库迁移后查询应正常");
        std::fs::remove_dir_all(&tmp_dir).unwrap_or(());
    }
}
