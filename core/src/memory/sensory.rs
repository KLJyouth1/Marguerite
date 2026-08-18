//! 感知记忆：当前会话的上下文存储
//!
//! 纯内存操作，毫秒级读写。支持可选的 SQLite 持久化。

use std::path::Path;

use anyhow::Result;
use chrono::Utc;
use rusqlite::Connection;

use crate::types::Message;

/// 感知记忆
///
/// 当前会话的上下文存储，纯内存 + SQLite 持久化，毫秒级读写。
pub struct SensoryMemory {
    session_id: String,
    messages: Vec<Message>,
    db: Option<Connection>,
    capacity: usize,
}

impl SensoryMemory {
    /// 创建新感知记忆
    pub fn new(session_id: &str) -> Self {
        Self {
            session_id: session_id.to_string(),
            messages: Vec::new(),
            db: None,
            capacity: 1000,
        }
    }

    /// 带持久化的感知记忆
    pub fn with_db(session_id: &str, db_path: &Path) -> Result<Self> {
        let db = Connection::open(db_path)?;
        db.execute_batch(
            "CREATE TABLE IF NOT EXISTS sensory_memory (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                session_id TEXT NOT NULL,
                role TEXT NOT NULL,
                content TEXT NOT NULL,
                thinking TEXT,
                tool_name TEXT,
                tool_input TEXT,
                tool_output TEXT,
                timestamp TEXT NOT NULL,
                created_at TEXT DEFAULT (datetime('now'))
            );
            CREATE INDEX IF NOT EXISTS idx_sensory_session ON sensory_memory(session_id);",
        )?;
        Ok(Self {
            session_id: session_id.to_string(),
            messages: Vec::new(),
            db: Some(db),
            capacity: 1000,
        })
    }

    /// 添加消息
    pub fn add_message(&mut self, msg: Message) -> Result<()> {
        if self.messages.len() >= self.capacity {
            self.messages.remove(0);
        }
        self.messages.push(msg);
        Ok(())
    }

    /// 获取最近 N 条消息
    pub fn get_messages(&self, limit: usize) -> Vec<&Message> {
        let start = self.messages.len().saturating_sub(limit);
        self.messages[start..].iter().collect()
    }

    /// 获取所有消息
    pub fn get_all_messages(&self) -> Vec<&Message> {
        self.messages.iter().collect()
    }

    /// 在当前会话中搜索
    pub fn search(&self, query: &str) -> Result<Vec<&Message>> {
        let query_lower = query.to_lowercase();
        let results: Vec<&Message> = self
            .messages
            .iter()
            .filter(|msg| {
                let content = match msg {
                    Message::User { content, .. } => content.as_str(),
                    Message::Assistant { content, .. } => content.as_str(),
                    Message::System { content, .. } => content.as_str(),
                    Message::Tool { output, .. } => output.as_str(),
                };
                content.to_lowercase().contains(&query_lower)
            })
            .collect();
        Ok(results)
    }

    /// 清空
    pub fn clear(&mut self) -> Result<()> {
        self.messages.clear();
        Ok(())
    }

    /// 消息数量
    pub fn len(&self) -> usize {
        self.messages.len()
    }

    /// 是否为空
    pub fn is_empty(&self) -> bool {
        self.messages.is_empty()
    }

    /// 会话 ID（供联邦记忆后端标识来源）
    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    /// 估算 Token 数（字符数 / 4）
    pub fn estimated_tokens(&self) -> usize {
        self.messages
            .iter()
            .map(|msg| {
                let char_count = match msg {
                    Message::User { content, .. } => content.len(),
                    Message::Assistant {
                        content, thinking, ..
                    } => content.len() + thinking.as_deref().unwrap_or("").len(),
                    Message::System { content, .. } => content.len(),
                    Message::Tool {
                        name,
                        input,
                        output,
                        ..
                    } => name.len() + input.len() + output.len(),
                };
                char_count / 4
            })
            .sum()
    }

    /// 持久化到 SQLite
    ///
    /// 将当前内存中的消息持久化到 SQLite 数据库。
    pub fn save(&self) -> Result<()> {
        let db = self
            .db
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("No database connection"))?;

        // 删除该会话的旧记录
        db.execute(
            "DELETE FROM sensory_memory WHERE session_id = ?1",
            rusqlite::params![self.session_id],
        )?;

        let tx = db.unchecked_transaction()?;
        {
            let mut stmt = tx.prepare(
                "INSERT INTO sensory_memory (session_id, role, content, thinking, tool_name, tool_input, tool_output, timestamp)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            )?;
            for msg in &self.messages {
                let (role, content, thinking, tool_name, tool_input, tool_output) =
                    extract_message_fields(msg);
                stmt.execute(rusqlite::params![
                    self.session_id,
                    role,
                    content,
                    thinking,
                    tool_name,
                    tool_input,
                    tool_output,
                    msg.timestamp().to_rfc3339(),
                ])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// 从 SQLite 加载
    pub fn load(session_id: &str, db: Connection) -> Result<Self> {
        let mut stmt = db.prepare(
            "SELECT role, content, thinking, tool_name, tool_input, tool_output, timestamp
             FROM sensory_memory WHERE session_id = ?1 ORDER BY id",
        )?;
        let rows = stmt.query_map(rusqlite::params![session_id], |row| {
            let role: String = row.get(0)?;
            let content: String = row.get(1)?;
            let thinking: Option<String> = row.get(2)?;
            let tool_name: Option<String> = row.get(3)?;
            let tool_input: Option<String> = row.get(4)?;
            let tool_output: Option<String> = row.get(5)?;
            let timestamp_str: String = row.get(6)?;
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
                },
                _ => Message::System { content, timestamp },
            };
            Ok(msg)
        })?;
        let messages = rows.filter_map(|r| r.ok()).collect::<Vec<Message>>();
        drop(stmt);

        Ok(Self {
            session_id: session_id.to_string(),
            messages,
            db: Some(db),
            capacity: 1000,
        })
    }
}

/// 从消息中提取数据库字段
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
) {
    match msg {
        Message::User { content, .. } => (msg.role(), content.as_str(), None, None, None, None),
        Message::Assistant {
            content, thinking, ..
        } => (
            msg.role(),
            content.as_str(),
            thinking.as_deref(),
            None,
            None,
            None,
        ),
        Message::System { content, .. } => (msg.role(), content.as_str(), None, None, None, None),
        Message::Tool {
            name,
            input,
            output,
            ..
        } => (
            msg.role(),
            "",
            None,
            Some(name.as_str()),
            Some(input.as_str()),
            Some(output.as_str()),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Message;
    use chrono::Utc;

    #[test]
    fn test_sensory_new_and_len() {
        let memory = SensoryMemory::new("test-session");
        assert_eq!(memory.len(), 0);
        assert!(memory.is_empty());
    }

    #[test]
    fn test_sensory_add_and_get_messages() {
        let mut memory = SensoryMemory::new("test-session");
        let now = Utc::now();

        memory
            .add_message(Message::User {
                content: "hello".to_string(),
                timestamp: now,
            })
            .unwrap();
        memory
            .add_message(Message::Assistant {
                content: "hi".to_string(),
                timestamp: now,
                thinking: None,
            })
            .unwrap();

        assert_eq!(memory.len(), 2);
        let recent = memory.get_messages(1);
        assert_eq!(recent.len(), 1);
        assert_eq!(recent[0].role(), "assistant");

        let all = memory.get_all_messages();
        assert_eq!(all.len(), 2);
    }

    #[test]
    fn test_sensory_search() {
        let mut memory = SensoryMemory::new("test-session");
        let now = Utc::now();

        memory
            .add_message(Message::User {
                content: "What is the weather in Beijing?".to_string(),
                timestamp: now,
            })
            .unwrap();
        memory
            .add_message(Message::Assistant {
                content: "The weather in Beijing is sunny.".to_string(),
                timestamp: now,
                thinking: None,
            })
            .unwrap();
        memory
            .add_message(Message::User {
                content: "Tell me a joke.".to_string(),
                timestamp: now,
            })
            .unwrap();

        let results = memory.search("weather").unwrap();
        assert_eq!(results.len(), 2);

        let results = memory.search("joke").unwrap();
        assert_eq!(results.len(), 1);

        let results = memory.search("nonexistent").unwrap();
        assert_eq!(results.len(), 0);
    }

    #[test]
    fn test_sensory_clear() {
        let mut memory = SensoryMemory::new("test-session");
        let now = Utc::now();

        memory
            .add_message(Message::User {
                content: "hello".to_string(),
                timestamp: now,
            })
            .unwrap();
        assert_eq!(memory.len(), 1);
        memory.clear().unwrap();
        assert_eq!(memory.len(), 0);
    }

    #[test]
    fn test_sensory_estimated_tokens() {
        let mut memory = SensoryMemory::new("test-session");
        let now = Utc::now();

        // "hello" = 5 chars / 4 = 1 token
        memory
            .add_message(Message::User {
                content: "hello".to_string(),
                timestamp: now,
            })
            .unwrap();
        assert_eq!(memory.estimated_tokens(), 1);

        // "hello world" = 11 chars / 4 = 2 tokens, total = 1 + 2 = 3
        memory
            .add_message(Message::User {
                content: "hello world".to_string(),
                timestamp: now,
            })
            .unwrap();
        assert_eq!(memory.estimated_tokens(), 3);
    }

    #[test]
    fn test_sensory_capacity() {
        let mut memory = SensoryMemory {
            session_id: "test".to_string(),
            messages: Vec::new(),
            db: None,
            capacity: 2,
        };
        let now = Utc::now();

        memory
            .add_message(Message::User {
                content: "msg1".to_string(),
                timestamp: now,
            })
            .unwrap();
        memory
            .add_message(Message::User {
                content: "msg2".to_string(),
                timestamp: now,
            })
            .unwrap();
        memory
            .add_message(Message::User {
                content: "msg3".to_string(),
                timestamp: now,
            })
            .unwrap();

        assert_eq!(memory.len(), 2);
    }

    #[test]
    fn test_sensory_with_db_and_save_load() {
        let tmp_dir = std::env::temp_dir().join("marguerite_test_sensory");
        std::fs::create_dir_all(&tmp_dir).unwrap();
        let db_path = tmp_dir.join("sensory.db");
        let now = Utc::now();

        {
            let mut memory = SensoryMemory::with_db("test-session", &db_path).unwrap();
            memory
                .add_message(Message::User {
                    content: "hello".to_string(),
                    timestamp: now,
                })
                .unwrap();
            memory
                .add_message(Message::Assistant {
                    content: "hi there".to_string(),
                    timestamp: now,
                    thinking: Some("thinking...".to_string()),
                })
                .unwrap();
            memory
                .add_message(Message::Tool {
                    name: "calculator".to_string(),
                    input: "1+1".to_string(),
                    output: "2".to_string(),
                    timestamp: now,
                })
                .unwrap();
            memory.save().unwrap();
        }

        // Load from DB
        {
            let db = Connection::open(&db_path).unwrap();
            let memory = SensoryMemory::load("test-session", db).unwrap();
            assert_eq!(memory.len(), 3);
            let all = memory.get_all_messages();
            assert_eq!(all[0].role(), "user");
            assert_eq!(all[1].role(), "assistant");
            assert_eq!(all[2].role(), "tool");
        }

        // Cleanup
        let _ = std::fs::remove_dir_all(&tmp_dir);
    }

    #[test]
    fn test_sensory_tool_message_search() {
        let mut memory = SensoryMemory::new("test-session");
        let now = Utc::now();

        memory
            .add_message(Message::User {
                content: "Calculate 1+1".to_string(),
                timestamp: now,
            })
            .unwrap();
        memory
            .add_message(Message::Tool {
                name: "calculator".to_string(),
                input: "1+1".to_string(),
                output: "Result: 2".to_string(),
                timestamp: now,
            })
            .unwrap();

        // Search in tool output
        let results = memory.search("Result: 2").unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].role(), "tool");
    }
}
