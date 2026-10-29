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

/// `sessions.search` 的单条对话命中。
#[derive(Debug, Clone)]
pub struct MessageSearchHit {
    /// 命中消息所属的会话（前端据此跳转）
    pub session_id: String,
    /// 会话标题（摘要）
    pub title: String,
    /// 消息角色（user / assistant / system / tool）
    pub role: String,
    /// 命中片段（`>>>`/`<<<` 包裹命中处）
    pub snippet: String,
    /// 消息时间（毫秒）
    pub created_at_ms: i64,
}

/// `LIKE` 模式里的元字符转义（查询里有 `%`/`_` 时不能当通配符）。
fn like_escape(query: &str) -> String {
    query
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

/// 把命中处截成短片段并用 `>>>`/`<<<` 标记（前端高亮约定）。
///
/// 按**字符**而非字节定位：小写化会改变字节长度，用字节下标切片可能落在
/// 字符边界中间直接 panic。
fn search_snippet(content: &str, query: &str) -> String {
    const LEAD: usize = 40;
    const TAIL: usize = 80;
    let chars: Vec<char> = content.chars().collect();
    let hay: Vec<char> = chars
        .iter()
        .map(|c| c.to_lowercase().next().unwrap_or(*c))
        .collect();
    let needle: Vec<char> = query
        .chars()
        .map(|c| c.to_lowercase().next().unwrap_or(c))
        .collect();
    let hit = if needle.is_empty() || needle.len() > hay.len() {
        None
    } else {
        hay.windows(needle.len())
            .position(|window| window == needle.as_slice())
    };
    let Some(start) = hit else {
        let mut plain: String = chars.iter().take(160).collect();
        if chars.len() > 160 {
            plain.push('…');
        }
        return plain;
    };
    let end = (start + needle.len()).min(chars.len());
    let from = start.saturating_sub(LEAD);
    let to = (end + TAIL).min(chars.len());
    let mut out = String::new();
    if from > 0 {
        out.push('…');
    }
    out.extend(&chars[from..start]);
    out.push_str(">>>");
    out.extend(&chars[start..end]);
    out.push_str("<<<");
    out.extend(&chars[end..to]);
    if to < chars.len() {
        out.push('…');
    }
    out
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
        // 迁移（P2-9 多根）：会话可绑定多个项目目录。`project_paths` 存 JSON
        // 数组字符串，`project_path` 仍是**主根**（= 数组首项），旧读取方无需改动。
        // 幂等：只增列；旧库该列为 NULL → 回退 `project_path` 单值（诚实降级）。
        let has_paths_col: bool = db
            .prepare("PRAGMA table_info(sessions)")?
            .query_map([], |row| row.get::<_, String>(1))?
            .filter_map(|r| r.ok())
            .any(|name| name == "project_paths");
        if !has_paths_col {
            db.execute_batch("ALTER TABLE sessions ADD COLUMN project_paths TEXT;")?;
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
        // 迁移（任务复盘）：工具行的成功/失败标记。只增列；旧行的工具结果没有
        // 这个信号 → NULL（复盘时按「未知」统计，不臆测成功）。
        let has_tool_ok_col: bool = db
            .prepare("PRAGMA table_info(session_messages)")?
            .query_map([], |row| row.get::<_, String>(1))?
            .filter_map(|r| r.ok())
            .any(|name| name == "tool_ok");
        if !has_tool_ok_col {
            db.execute_batch("ALTER TABLE session_messages ADD COLUMN tool_ok INTEGER;")?;
        }
        // 数据修复（幂等）：旧版本会把「有内容但行先由 touch_session / 空快照
        // 创建」的会话标题永久留在占位符上（真机库实测 3 个真实 webchat 会话
        // 顶着 "Empty session"）。运行时回填只在追加消息时触发，这里补一次
        // 存量修复：标题仍等于占位符时，取首条用户消息（没有则首条真实消息）
        // 的前 100 字。用户重命名过的标题不等于占位符，不会被覆盖。
        db.execute(
            "UPDATE sessions SET summary = COALESCE(
                (SELECT substr(trim(m.content), 1, 100) FROM session_messages m
                  WHERE m.session_id = sessions.session_id AND m.role = 'user'
                    AND trim(m.content) <> '' ORDER BY m.id LIMIT 1),
                (SELECT substr(trim(m.content), 1, 100) FROM session_messages m
                  WHERE m.session_id = sessions.session_id AND m.role IN ('user','assistant')
                    AND trim(m.content) <> '' ORDER BY m.id LIMIT 1),
                summary)
             WHERE summary = ?1 AND message_count > 0",
            rusqlite::params![PLACEHOLDER_SUMMARY],
        )?;
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
    ///
    /// P2-9：这里按**主根索引** `project_path` 精确匹配（主根 = `project_paths`
    /// 首项）。多根场景下「命中任一子根」的统计等后续再扩，避免本轮改变既有语义。
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

    /// 按「多根归属」列出会话：`project_path` 精确命中 **或** `project_paths`
    /// 数组包含该路径（P2-9 多项目工作区统计 / 历史删除共用）。
    ///
    /// SQL 先按 `project_path = ?` 或 `project_paths LIKE %"path"%` 粗筛，
    /// 再在 Rust 侧解析 JSON 做精确二次过滤——避免路径里的 `%`/`_` 被 LIKE
    /// 当通配符造成误命中。
    pub fn list_sessions_by_any_project(
        &self,
        project_path: &str,
        limit: usize,
    ) -> Result<Vec<SessionInfo>> {
        let db = self.db()?;
        // JSON 序列化得到 `"D:\\proj"`（含引号与转义），正好用作 LIKE 锚点。
        let needle = serde_json::to_string(project_path)?;
        let pattern = format!("%{needle}%");
        let mut stmt = db.prepare(
            "SELECT session_id, summary, message_count, state, created_at, updated_at, tokens_used, project_path, reasoning_effort, context_window, project_paths
             FROM sessions WHERE project_path = ?1 OR project_paths LIKE ?2
             ORDER BY updated_at DESC LIMIT ?3",
        )?;
        let rows = stmt.query_map(
            rusqlite::params![project_path, pattern, limit as i64],
            |row| {
                let info = SessionInfo {
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
                };
                let raw: Option<String> = row.get(10)?;
                Ok((info, raw))
            },
        )?;
        let mut results = Vec::new();
        for row in rows.filter_map(Result::ok) {
            let (info, raw) = row;
            let matched = info.project_path.as_deref() == Some(project_path)
                || raw
                    .as_deref()
                    .and_then(|r| serde_json::from_str::<Vec<String>>(r).ok())
                    .map(|paths| paths.iter().any(|p| p == project_path))
                    .unwrap_or(false);
            if matched {
                results.push(info);
            }
        }
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
        self.query_sessions(Some(limit))
    }

    /// 列出**全部**会话（新的在前，无上限）。
    ///
    /// `sessions.list` 必须**先按 key 契约过滤再截断**：先 `LIMIT` 再过滤会让
    /// 历史探针残留（真机库 196/325 行是 `ws.*` / `qa*` 这类裸键）挤掉真实
    /// 会话。截断由调用方负责。
    pub fn list_all_sessions(&self) -> Result<Vec<SessionInfo>> {
        self.query_sessions(None)
    }

    fn query_sessions(&self, limit: Option<usize>) -> Result<Vec<SessionInfo>> {
        let db = self.db()?;
        // SQLite 的 `LIMIT -1` 表示不限量。
        let limit_param: i64 = limit.map(|l| l as i64).unwrap_or(-1);
        let mut stmt = db.prepare(
            "SELECT session_id, summary, message_count, state, created_at, updated_at, tokens_used, project_path, reasoning_effort, context_window
             FROM sessions ORDER BY updated_at DESC LIMIT ?1",
        )?;

        let results = stmt
            .query_map(rusqlite::params![limit_param], |row| {
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

    /// 会话消息的全文子串搜索（`sessions.search` 的对话面）。
    ///
    /// 只做 `LIKE` 子串匹配（本仓没有 FTS 索引），按消息 id 倒序取 `limit` 条；
    /// 工具行的正文在 `tool_input` / `tool_output` 两列（`content` 为空），
    /// 所以三列都要搜，否则「搜工具输出」永远是空。命中片段用 `>>>`/`<<<` 包裹
    /// —— 前端 `highlightFtsSnippet` 按该约定高亮。
    pub fn search_messages(&self, query: &str, limit: usize) -> Result<Vec<MessageSearchHit>> {
        let db = self.db()?;
        let mut stmt = db.prepare(
            "SELECT m.session_id, m.role, m.content, m.tool_input, m.tool_output, m.timestamp,
                    COALESCE(s.summary, '')
             FROM session_messages m
             LEFT JOIN sessions s ON s.session_id = m.session_id
             WHERE m.content LIKE ?1 ESCAPE '\\'
                OR m.tool_input LIKE ?1 ESCAPE '\\'
                OR m.tool_output LIKE ?1 ESCAPE '\\'
             ORDER BY m.id DESC LIMIT ?2",
        )?;
        let pattern = format!("%{}%", like_escape(query));
        let rows = stmt
            .query_map(
                rusqlite::params![pattern, limit.clamp(1, 50) as i64],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, Option<String>>(3)?,
                        row.get::<_, Option<String>>(4)?,
                        row.get::<_, String>(5)?,
                        row.get::<_, String>(6)?,
                    ))
                },
            )?
            .filter_map(|r| r.ok())
            .map(
                |(session_id, role, content, tool_input, tool_output, timestamp, title)| {
                    let mut snippet = search_snippet(&content, query);
                    for field in [tool_input.as_deref(), tool_output.as_deref()]
                        .into_iter()
                        .flatten()
                    {
                        if !snippet.contains(">>>") {
                            snippet = search_snippet(field, query);
                        }
                    }
                    MessageSearchHit {
                        created_at_ms: chrono::DateTime::parse_from_rfc3339(&timestamp)
                            .map(|d| d.timestamp_millis())
                            .unwrap_or(0),
                        snippet,
                        session_id,
                        role,
                        title,
                    }
                },
            )
            .collect();
        Ok(rows)
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

    /// 绑定会话到项目根目录（R70：从 `save_snapshot_for_project` 拆出）。
    ///
    /// 消息已改为逐条追加（[`Self::append_message`]），项目归属不再需要随整段
    /// 快照重写——本方法只更新归属列，行不存在时先创建（与 `touch_session` 同口径）。
    ///
    /// P2-9：本方法只写**主根索引**（`project_path`，供精确匹配用）；一个会话的
    /// 完整多根集合由 [`Self::bind_projects`] 写入 `project_paths`。
    pub fn bind_project(&self, session_id: &str, project_path: &str) -> Result<()> {
        let db = self.db()?;
        let now = Utc::now().to_rfc3339();
        db.execute(
            "INSERT OR IGNORE INTO sessions
                (session_id, summary, state, message_count, tokens_used, created_at, updated_at, project_path)
             VALUES (?1, 'Empty session', 'idle', 0, 0, ?2, ?2, ?3)",
            rusqlite::params![session_id, now, project_path],
        )?;
        db.execute(
            "UPDATE sessions SET
                project_path = ?2,
                message_count = (SELECT COUNT(*) FROM session_messages WHERE session_id = ?1),
                updated_at = ?3
             WHERE session_id = ?1",
            rusqlite::params![session_id, project_path, now],
        )?;
        Ok(())
    }

    /// 绑定会话到多个项目目录（P2-9）。
    ///
    /// 双写：`project_path` = 第一个（主根索引，旧读取方不变），
    /// `project_paths` = 去重保序后的 JSON 数组字符串。
    /// 空列表为无操作（不得把已有归属清空）。
    pub fn bind_projects(&mut self, session_id: &str, projects: &[String]) -> Result<()> {
        let normalized = normalize_projects(projects);
        let Some(primary) = normalized.first() else {
            return Ok(());
        };
        let raw = serde_json::to_string(&normalized)?;
        let db = self.db()?;
        let now = Utc::now().to_rfc3339();
        db.execute(
            "INSERT OR IGNORE INTO sessions
                (session_id, summary, state, message_count, tokens_used, created_at, updated_at, project_path, project_paths)
             VALUES (?1, 'Empty session', 'idle', 0, 0, ?2, ?2, ?3, ?4)",
            rusqlite::params![session_id, now, primary, raw],
        )?;
        db.execute(
            "UPDATE sessions SET
                project_path = ?2,
                project_paths = ?3,
                message_count = (SELECT COUNT(*) FROM session_messages WHERE session_id = ?1),
                updated_at = ?4
             WHERE session_id = ?1",
            rusqlite::params![session_id, primary, raw, now],
        )?;
        Ok(())
    }

    /// 读取会话绑定的全部项目目录（P2-9）。
    ///
    /// `project_paths` JSON 有效且非空 → 原样返回（去重保序）；空/损坏 → 回退
    /// `project_path` 单值；都空 → 空 Vec。
    pub fn project_paths_of(&self, session_id: &str) -> Result<Vec<String>> {
        let db = self.db()?;
        let row = db.query_row(
            "SELECT project_paths, project_path FROM sessions WHERE session_id = ?1",
            rusqlite::params![session_id],
            |row| {
                Ok((
                    row.get::<_, Option<String>>(0)?,
                    row.get::<_, Option<String>>(1)?,
                ))
            },
        );
        let (raw_paths, legacy) = match row {
            Ok(v) => v,
            Err(rusqlite::Error::QueryReturnedNoRows) => return Ok(Vec::new()),
            Err(e) => return Err(e.into()),
        };
        if let Some(raw) = raw_paths {
            if let Ok(parsed) = serde_json::from_str::<Vec<String>>(&raw) {
                let parsed = normalize_projects(&parsed);
                if !parsed.is_empty() {
                    return Ok(parsed);
                }
            }
        }
        Ok(legacy
            .map(|p| p.trim().to_string())
            .filter(|p| !p.is_empty())
            .map(|p| vec![p])
            .unwrap_or_default())
    }

    /// 清除会话所有消息（保留会话记录本身，sessions.reset RPC 后端实现）
    pub fn clear_messages(&self, session_id: &str) -> Result<()> {        let db = self.db()?;
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

    /// **只追加**一条会话消息（R70：`sessions.db` 为只追加的会话记录）。
    ///
    /// # 与 [`Self::save_snapshot`] 的区别是本方法的全部意义
    ///
    /// `save_snapshot` 是「整段重写」（先 DELETE 该会话全部消息再重插）。它的输入
    /// 是 context 台账的当前内容，而台账会因**上下文压缩**折叠成「摘要 + 最近几条」
    /// —— 压缩本是**模型侧**的优化，一旦用它重写 DB，用户翻回去的历史就没了。
    ///
    /// 本方法供逐条落盘使用：每条对话消息到达时追加一行，**绝不删除已有行**。
    /// 上下文压缩从此只影响台账（模型上下文），不再影响展示历史。
    ///
    /// `summary` 只在**首次创建**会话行时使用（取首条用户消息前缀，与
    /// `generate_summary` 口径一致）；已存在的行保持用户/系统设置过的标题不被覆盖。
    pub fn append_message(
        &self,
        session_id: &str,
        role: &str,
        content: &str,
        timestamp_rfc3339: &str,
        summary_seed: Option<&str>,
    ) -> Result<()> {
        self.append_message_with(
            session_id,
            role,
            content,
            timestamp_rfc3339,
            summary_seed,
            None,
        )
    }

    /// 追加一条消息，并可带上**思考正文**（`thinking`）。
    ///
    /// 网关 `chat.send` 此前只落 `content`，于是 `session_messages.thinking`
    /// 在会话页历史里永远为空，复盘也拿不到「智能体思路」。本方法供网关路径使用。
    pub fn append_message_with(
        &self,
        session_id: &str,
        role: &str,
        content: &str,
        timestamp_rfc3339: &str,
        summary_seed: Option<&str>,
        thinking: Option<&str>,
    ) -> Result<()> {
        let db = self.db()?;
        let now = Utc::now().to_rfc3339();
        let tx = db.unchecked_transaction()?;
        // 先确保会话行存在：session_messages 有指向 sessions 的外键，
        // 消息行必须后插（否则 FOREIGN KEY constraint failed）。
        let inserted = tx.execute(
            "INSERT OR IGNORE INTO sessions
                (session_id, summary, state, message_count, tokens_used, created_at, updated_at)
             VALUES (?1, ?2, 'idle', 0, 0, ?3, ?3)",
            rusqlite::params![
                session_id,
                summary_seed.unwrap_or(PLACEHOLDER_SUMMARY),
                now
            ],
        )?;
        // 行已存在时 INSERT OR IGNORE 不生效（inserted==0）→ 既有 summary 保持不变
        // （尊重用户重命名）；两分支的计数/时间刷新相同，合并为一条 UPDATE。
        //
        // 但「占位标题」不是用户选择：会话行可能先由 `touch_session` /
        // `sessions.create` 的空快照 / 工具行创建（'Empty session'），随后才有
        // 真实消息。不回填的话，这些有内容的会话永远顶着一个空标题（真机 DB
        // 实测有带完整回复的 `agent:main:webchat:*` 会话仍叫 "Empty session"，
        // 侧栏看上去像重复的空行）。回填只在标题**仍是占位符**时发生，用户
        // 重命名过的会话不受影响。
        let _ = inserted;
        backfill_placeholder_summary(&tx, session_id, role, content, summary_seed)?;
        tx.execute(
            "INSERT INTO session_messages (session_id, role, content, thinking, timestamp)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![
                session_id,
                role,
                content,
                thinking
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(|s| truncate_chars(s, MAX_STORED_THINKING_CHARS)),
                timestamp_rfc3339
            ],
        )?;
        tx.execute(
            "UPDATE sessions SET
                message_count = (SELECT COUNT(*) FROM session_messages WHERE session_id = ?1),
                updated_at = ?2
             WHERE session_id = ?1",
            rusqlite::params![session_id, now],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// 追加一条**工具调用**记录（`role='tool'`）。
    ///
    /// 网关路径此前只在内存里累计工具的 token 开销，`session_messages` 里没有任何
    /// 工具行 —— 会话历史看不到「调了什么、多久、结果如何」，任务复盘也无从统计。
    /// 这里按行落盘；`input` / `output` 会按上限截断（见
    /// [`MAX_STORED_TOOL_FIELD_CHARS`]），完整输出仍以工具自身产物为准。
    // 7 个业务字段 + &self 超出门槛：参数都是独立的列值，再包一层结构体
    // 只会把同一组必填信息藏到别处，不提升可读性。
    #[allow(clippy::too_many_arguments)]
    pub fn append_tool_message(
        &self,
        session_id: &str,
        tool_name: &str,
        tool_input: &str,
        tool_output: &str,
        elapsed_ms: Option<u64>,
        ok: Option<bool>,
        timestamp_rfc3339: &str,
    ) -> Result<()> {
        let db = self.db()?;
        let now = Utc::now().to_rfc3339();
        let input = truncate_chars(tool_input.trim(), MAX_STORED_TOOL_FIELD_CHARS);
        let output = truncate_chars(tool_output.trim(), MAX_STORED_TOOL_FIELD_CHARS);
        let tx = db.unchecked_transaction()?;
        tx.execute(
            "INSERT OR IGNORE INTO sessions
                (session_id, summary, state, message_count, tokens_used, created_at, updated_at)
             VALUES (?1, 'Empty session', 'idle', 0, 0, ?2, ?2)",
            rusqlite::params![session_id, now],
        )?;
        tx.execute(
            "INSERT INTO session_messages
                (session_id, role, content, tool_name, tool_input, tool_output, elapsed_ms, tool_ok, timestamp)
             VALUES (?1, 'tool', ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            rusqlite::params![
                session_id,
                format!("[{tool_name}]"),
                tool_name,
                input,
                output,
                elapsed_ms.map(|ms| ms as i64),
                ok.map(|v| if v { 1i64 } else { 0i64 }),
                timestamp_rfc3339
            ],
        )?;
        tx.execute(
            "UPDATE sessions SET
                message_count = (SELECT COUNT(*) FROM session_messages WHERE session_id = ?1),
                updated_at = ?2
             WHERE session_id = ?1",
            rusqlite::params![session_id, now],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// 刷新会话元数据（message_count / updated_at），**不触碰消息行**。
    ///
    /// 供 `persist_session` 在不再整段重写消息后维持 `sessions.list` 的计数与排序。
    pub fn touch_session(&self, session_id: &str) -> Result<()> {
        let db = self.db()?;
        let now = Utc::now().to_rfc3339();
        db.execute(
            "INSERT OR IGNORE INTO sessions
                (session_id, summary, state, message_count, tokens_used, created_at, updated_at)
             VALUES (?1, 'Empty session', 'idle', 0, 0, ?2, ?2)",
            rusqlite::params![session_id, now],
        )?;
        db.execute(
            "UPDATE sessions SET
                message_count = (SELECT COUNT(*) FROM session_messages WHERE session_id = ?1),
                updated_at = ?2
             WHERE session_id = ?1",
            rusqlite::params![session_id, now],
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

    /// 按行序返回工具成败标记（非工具行 = `None`），与 [`Self::restore_session`]
    /// 的 `ORDER BY id` 同序。
    ///
    /// `tool_ok` 一直随工具行落盘（[`Self::append_tool_message`]），但读取链
    /// 此前没把它带上，于是 `chat.history` 的历史工具卡只能按「有没有输出」
    /// 推断成败 —— **刷新后失败的工具一律显示成成功**。NULL 是旧数据或回合
    /// 被中断（无信号），调用方必须按「未知」呈现，不得臆测。
    pub fn tool_outcome_flags(&self, session_id: &str) -> Result<Vec<Option<bool>>> {
        let db = self.db()?;
        let mut stmt = db.prepare(
            "SELECT role, tool_ok FROM session_messages WHERE session_id = ?1 ORDER BY id",
        )?;
        let flags = stmt
            .query_map(rusqlite::params![session_id], |row| {
                let role: String = row.get(0)?;
                let ok: Option<i64> = row.get(1)?;
                Ok(if role == "tool" { ok.map(|v| v != 0) } else { None })
            })?
            .filter_map(|r| r.ok())
            .collect();
        Ok(flags)
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

/// 落盘思考正文的字符上限：复盘要能读，但不能让一条思考把 DB 撑爆。
const MAX_STORED_THINKING_CHARS: usize = 8_000;

/// 工具入参/输出单字段落盘上限。完整输出以工具自身产物为准，这里只留可读证据。
const MAX_STORED_TOOL_FIELD_CHARS: usize = 4_000;

/// 单轮思考缓冲上限（字符）：超出即截断，只保留可复盘的主体。
const MAX_PENDING_THINKING_CHARS: usize = 16_000;

/// 进程级工具活动去重表上限（超出即整体清空；去重只防同一次调用被多条路径重复落盘）。
const MAX_TOOL_ACTIVITY_KEYS: usize = 4_096;

/// 单轮思考缓冲：思考增量/整段先攒着，assistant 行落盘时一起写 `thinking` 列。
///
/// 放在 persistence 层而不是网关层：桌面主路径（`agent::runtime` 非流式回路）
/// 不发 `Thinking` 事件，只有网关的流式桥能攒到；两条路径必须共用同一个缓冲，
/// 否则「智能体思路」在主流路径上永远是空的。
fn pending_thinking_store() -> &'static std::sync::Mutex<std::collections::HashMap<String, String>>
{
    static BUF: std::sync::OnceLock<
        std::sync::Mutex<std::collections::HashMap<String, String>>,
    > = std::sync::OnceLock::new();
    BUF.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()))
}

/// 记一段思考（增量或整段；按字符上限截断）。
pub fn note_pending_thinking(session: &str, text: &str) {
    if text.trim().is_empty() {
        return;
    }
    let mut guard = pending_thinking_store()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let buf = guard.entry(session.to_string()).or_default();
    if buf.chars().count() >= MAX_PENDING_THINKING_CHARS {
        return;
    }
    let room = MAX_PENDING_THINKING_CHARS.saturating_sub(buf.chars().count());
    buf.extend(text.chars().take(room));
}

/// 取走本轮的思考缓冲（assistant 落盘时调用）。
pub fn take_pending_thinking(session: &str) -> Option<String> {
    pending_thinking_store()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(session)
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// 读取本轮思考缓冲但**不消费**（回合收尾发事件时用；落盘仍需 `take_` 取走）。
pub fn peek_pending_thinking(session: &str) -> Option<String> {
    pending_thinking_store()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(session)
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// 丢弃残留思考（新一轮用户输入到达时调用，避免跨轮串味）。
pub fn clear_pending_thinking(session: &str) {
    pending_thinking_store()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(session);
}

/// 进程级 `sessions.db` 句柄（工具活动落盘共用一条连接，避免每条工具行都重新打开库）。
fn tool_activity_db() -> &'static std::sync::Mutex<SessionManager> {
    static MGR: std::sync::OnceLock<std::sync::Mutex<SessionManager>> =
        std::sync::OnceLock::new();
    MGR.get_or_init(|| {
        let path = crate::persistence::config::Config::config_dir().join("sessions.db");
        let mut manager = SessionManager::new(&path);
        if let Err(e) = manager.init() {
            tracing::warn!(err = %e, "工具活动落盘：sessions.db 初始化失败");
        }
        std::sync::Mutex::new(manager)
    })
}

fn tool_activity_seen() -> &'static std::sync::Mutex<std::collections::HashSet<String>> {
    static SEEN: std::sync::OnceLock<std::sync::Mutex<std::collections::HashSet<String>>> =
        std::sync::OnceLock::new();
    SEEN.get_or_init(|| std::sync::Mutex::new(std::collections::HashSet::new()))
}

/// 记一次工具活动（`role='tool'` 行），供**网关桥**与**计划路径**共用。
///
/// 幂等：`call_id` 存在时按 `(session, call_id)` 去重 —— 同一次调用可能被
/// 引擎侧与网关事件桥各看到一次，不能因此写两行。缺 `call_id` 时退化为
/// `(session, tool, 输出哈希)`，仍能拦住完全相同的重复事件。
///
/// 失败只记日志、绝不影响回合（复盘证据不该拖垮主链路）。
pub fn append_tool_activity(
    session: &str,
    call_id: Option<&str>,
    tool: &str,
    input: &str,
    output: &str,
    elapsed_ms: Option<u64>,
    ok: Option<bool>,
) {
    let tool = tool.trim();
    if session.trim().is_empty() || tool.is_empty() {
        return;
    }
    // 单测禁止写真实用户库：事件 tap 跑在独立线程，线程局部的 config_dir 覆盖
    // 传不过去，一旦落盘就会把测试会话混进用户的 `~/.marguerite/sessions.db`。
    // 集成测试（tests/ 目录）编译时不带 cfg(test)，仍按真实路径验证。
    if cfg!(test) {
        let _ = (call_id, input, output, elapsed_ms, ok);
        return;
    }
    use std::hash::{Hash, Hasher};
    let key = match call_id.map(str::trim).filter(|s| !s.is_empty()) {
        Some(id) => format!("{session}|id:{id}"),
        None => {
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            session.hash(&mut hasher);
            tool.hash(&mut hasher);
            output.hash(&mut hasher);
            format!("{session}|h:{:016x}", hasher.finish())
        }
    };
    {
        let mut seen = tool_activity_seen()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if !seen.insert(key) {
            return;
        }
        if seen.len() > MAX_TOOL_ACTIVITY_KEYS {
            seen.clear();
        }
    }
    let manager = tool_activity_db().lock().unwrap_or_else(|e| e.into_inner());
    if let Err(e) = manager.append_tool_message(
        session,
        tool,
        input,
        output,
        elapsed_ms,
        ok,
        &Utc::now().to_rfc3339(),
    ) {
        tracing::warn!(err = %e, session = %session, tool = %tool, "工具活动落盘失败（忽略）");
    }
}

/// 项目路径归一化：去空白、丢空串、去重（保持首次出现顺序）。
fn normalize_projects(projects: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::with_capacity(projects.len());
    for p in projects {
        let p = p.trim();
        if !p.is_empty() && !out.iter().any(|x| x == p) {
            out.push(p.to_string());
        }
    }
    out
}

/// 空会话占位标题（`append_message_with` / `touch_session` / 空快照 / 工具行共用）。
const PLACEHOLDER_SUMMARY: &str = "Empty session";

/// 把仍是占位标题的会话补成真实标题：用户消息用调用方给的种子（前 100 字），
/// 没有种子时只在首条真实消息是 assistant 的情况下用正文兜底（与
/// [`SessionManager::generate_summary`]「没有用户消息就取第一条」口径一致）。
/// 标题已被重命名过（不等于占位符）时**绝不覆盖**。
fn backfill_placeholder_summary(
    tx: &rusqlite::Transaction<'_>,
    session_id: &str,
    role: &str,
    content: &str,
    summary_seed: Option<&str>,
) -> Result<()> {
    let seed = summary_seed.map(str::trim).filter(|s| !s.is_empty());
    let title = match seed {
        Some(s) => s.chars().take(100).collect::<String>(),
        None if role == "assistant" && !content.trim().is_empty() => content
            .trim()
            .chars()
            .take(100)
            .collect::<String>(),
        None => return Ok(()),
    };
    tx.execute(
        "UPDATE sessions SET summary = ?1 WHERE session_id = ?2 AND summary = ?3",
        rusqlite::params![title, session_id, PLACEHOLDER_SUMMARY],
    )?;
    Ok(())
}

/// 按字符（不是字节）截断，避免切断多字节字符导致乱码。
fn truncate_chars(text: &str, max_chars: usize) -> String {    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    let mut out: String = text.chars().take(max_chars).collect();
    out.push('…');
    out
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

    /// 每个用例独占目录：固定 tag 目录会跨**运行**残留 —— Windows 下用例结束时
    /// manager 仍持有 sessions.db，收尾的 `remove_dir_all` 静默失败，下一轮就把
    /// 上一轮的数据当成自己的初始状态（实测 `touch_session` 之后读到旧标题）。
    fn setup_manager(name: &str) -> (SessionManager, PathBuf) {
        static SEQ: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let tmp_dir = std::env::temp_dir().join(format!(
            "marguerite_test_{}_{}_{}",
            name,
            std::process::id(),
            SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
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

    // 占位标题回填：行可能先由 touch_session / 空快照 / 工具行创建
    // （'Empty session'），真实消息到达时必须自愈，否则侧栏里全是
    // 带完整内容的空标题会话（真机 DB 实测存在）。
    #[test]
    fn test_append_message_backfills_placeholder_summary_with_seed() {
        let (manager, tmp_dir) = setup_manager("ss_backfill_seed");
        let now = Utc::now().to_rfc3339();

        manager.touch_session("session-1").unwrap();
        let info = manager.get_session("session-1").unwrap().unwrap();
        assert_eq!(info.summary, "Empty session");

        manager
            .append_message(
                "session-1",
                "user",
                "配置一下本地模型",
                &now,
                Some("配置一下本地模型"),
            )
            .unwrap();

        let info = manager.get_session("session-1").unwrap().unwrap();
        assert_eq!(info.summary, "配置一下本地模型");

        std::fs::remove_dir_all(&tmp_dir).unwrap_or(());
    }

    #[test]
    fn test_append_message_backfills_placeholder_summary_from_assistant() {
        let (manager, tmp_dir) = setup_manager("ss_backfill_assistant");
        let now = Utc::now().to_rfc3339();

        manager.touch_session("session-1").unwrap();
        manager
            .append_message(
                "session-1",
                "assistant",
                "已为你启用本地模型路由。",
                &now,
                None,
            )
            .unwrap();

        let info = manager.get_session("session-1").unwrap().unwrap();
        assert_eq!(info.summary, "已为你启用本地模型路由。");

        std::fs::remove_dir_all(&tmp_dir).unwrap_or(());
    }

    #[test]
    fn test_append_message_keeps_renamed_summary() {
        let (manager, tmp_dir) = setup_manager("ss_backfill_rename");
        let now = Utc::now().to_rfc3339();

        manager.touch_session("session-1").unwrap();
        manager
            .update_summary("session-1", Some("用户自定义标题"))
            .unwrap();
        manager
            .append_message("session-1", "user", "随便聊聊", &now, Some("随便聊聊"))
            .unwrap();

        let info = manager.get_session("session-1").unwrap().unwrap();
        assert_eq!(info.summary, "用户自定义标题");

        std::fs::remove_dir_all(&tmp_dir).unwrap_or(());
    }

    /// 存量修复：init 必须把「有内容但仍是占位标题」的旧数据补成真实标题，
    /// 且**只**动占位符（用户重命名过的标题不受影响）。
    #[test]
    fn init_repairs_legacy_placeholder_titles_but_not_renames() {
        let (mut manager, tmp_dir) = setup_manager("ss_title_repair");
        let now = Utc::now().to_rfc3339();

        manager
            .append_message("session-1", "user", "帮我看看部署日志", &now, Some("帮我看看部署日志"))
            .unwrap();
        manager
            .append_message("session-2", "assistant", "已检查完成", &now, None)
            .unwrap();
        manager.touch_session("session-3").unwrap();
        // `update_summary` 只 UPDATE 已存在的行（不建行），所以重命名必须发生在
        // 行创建之后；随后再追加一条消息，验证重命名不会被回填覆盖。
        manager
            .append_message("session-4", "user", "随便聊聊", &now, Some("随便聊聊"))
            .unwrap();
        manager
            .update_summary("session-4", Some("用户自定义标题"))
            .unwrap();
        manager
            .append_message("session-4", "user", "再聊一句", &now, Some("再聊一句"))
            .unwrap();
        manager
            .db()
            .unwrap()
            .execute(
                "UPDATE sessions SET summary = 'Empty session' WHERE session_id = 'session-1'",
                [],
            )
            .unwrap();

        // init 常驻在每次 sessions.list 打开 manager 时执行，必须幂等。
        manager.init().unwrap();
        manager.init().unwrap();

        assert_eq!(
            manager.get_session("session-1").unwrap().unwrap().summary,
            "帮我看看部署日志"
        );
        assert_eq!(
            manager.get_session("session-2").unwrap().unwrap().summary,
            "已检查完成",
            "没有用户消息时用首条真实消息兜底"
        );
        assert_eq!(
            manager.get_session("session-3").unwrap().unwrap().summary,
            "Empty session",
            "零消息会话保持占位符（不会凭空造标题）"
        );
        assert_eq!(
            manager.get_session("session-4").unwrap().unwrap().summary,
            "用户自定义标题",
            "用户重命名过的不受影响"
        );

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

    /// P2-9：迁移后多根双写/回读；只写旧列时回退单值。
    #[test]
    fn test_project_paths_multivalue_roundtrip_and_legacy_fallback() {
        let (mut manager, tmp_dir) = setup_manager("ss_multi_paths");

        // init 的幂等迁移补齐 `project_paths` 列后，写入 2 个路径 → 回读顺序一致
        manager
            .bind_projects(
                "multi-1",
                &["D:/proj/a".to_string(), "D:/proj/b".to_string()],
            )
            .unwrap();
        assert_eq!(
            manager.project_paths_of("multi-1").unwrap(),
            vec!["D:/proj/a".to_string(), "D:/proj/b".to_string()],
            "多根必须原序回读"
        );
        assert_eq!(
            manager
                .get_session("multi-1")
                .unwrap()
                .unwrap()
                .project_path
                .as_deref(),
            Some("D:/proj/a"),
            "主根（project_path）必须是第一个"
        );

        // 只写旧列（bind_project）→ 回退为 1 个
        manager.bind_project("legacy-1", "D:/proj/only").unwrap();
        assert_eq!(
            manager.project_paths_of("legacy-1").unwrap(),
            vec!["D:/proj/only".to_string()],
            "旧列必须回退为单值"
        );

        // 空列表为无操作：不得清空已有归属
        manager.bind_projects("legacy-1", &[]).unwrap();
        assert_eq!(
            manager.project_paths_of("legacy-1").unwrap(),
            vec!["D:/proj/only".to_string()]
        );

        std::fs::remove_dir_all(&tmp_dir).unwrap_or(());
    }
}

#[cfg(test)]
mod append_only_tests {
    use super::*;

    fn temp_db() -> (SessionManager, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("marg_append_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        (SessionManager::new(&dir.join("sessions.db")), dir)
    }

    /// R70 核心契约：**追加永不删除**。
    ///
    /// 历史缺陷：`save_snapshot` 是「DELETE 全部 + 重插」，台账压缩折叠后用它落盘，
    /// 用户翻回去的历史就少一截。`append_message` 必须逐条累加、顺序稳定。
    #[test]
    fn append_never_deletes_and_preserves_order() {
        let (mut mgr, dir) = temp_db();
        mgr.init().unwrap();
        mgr.append_message("s1", "user", "第一条用户消息", "2026-10-19T10:00:00Z", Some("第一条用户消息"))
            .unwrap();
        mgr.append_message("s1", "assistant", "第一条回复", "2026-10-19T10:00:05Z", None)
            .unwrap();
        mgr.append_message("s1", "user", "第二条用户消息", "2026-10-19T10:01:00Z", Some("第二条用户消息"))
            .unwrap();

        let snap = mgr.load_snapshot("s1").unwrap();
        assert_eq!(snap.messages.len(), 3, "三次追加必须都在，一条不能少");
        match &snap.messages[0] {
            Message::User { content, timestamp } => {
                assert_eq!(content, "第一条用户消息");
                assert_eq!(
                    timestamp.timestamp_millis(),
                    1_792_404_000_000,
                    "必须保留传入的真实时间戳（不得偷换成 now）"
                );
            }
            other => panic!("首条应为 user：{other:?}"),
        }
        match &snap.messages[1] {
            Message::Assistant { content, .. } => assert_eq!(content, "第一条回复"),
            other => panic!("第二条应为 assistant：{other:?}"),
        }
        // message_count 与实际行数一致（sessions.list 展示用）
        let info = mgr.get_session("s1").unwrap().expect("会话行应存在");
        assert_eq!(info.message_count, 3);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// 标题只认**首条**种子，且后续追加/重命名不被覆盖。
    #[test]
    fn summary_seed_applies_only_on_first_creation() {
        let (mut mgr, dir) = temp_db();
        mgr.init().unwrap();
        mgr.append_message("s2", "user", "开场的用户消息", "2026-10-19T10:00:00Z", Some("开场的用户消息"))
            .unwrap();
        let t1 = mgr.get_session("s2").unwrap().unwrap().summary;
        assert_eq!(t1, "开场的用户消息");
        // 后续追加即使带不同的种子，也不得改标题（用户可能已重命名）
        mgr.append_message("s2", "assistant", "回复内容", "2026-10-19T10:00:05Z", Some("回复内容"))
            .unwrap();
        let t2 = mgr.get_session("s2").unwrap().unwrap().summary;
        assert_eq!(t2, "开场的用户消息", "追加不得覆盖既有标题");
        let _ = std::fs::remove_dir_all(dir);
    }

    /// 项目归属绑定不得动消息行（拆自 save_snapshot_for_project）。
    #[test]
    fn bind_project_keeps_messages_and_count() {
        let (mut mgr, dir) = temp_db();
        mgr.init().unwrap();
        mgr.append_message("s3", "user", "msg-a", "2026-10-19T10:00:00Z", None)
            .unwrap();
        mgr.append_message("s3", "assistant", "msg-b", "2026-10-19T10:00:05Z", None)
            .unwrap();
        mgr.bind_project("s3", "D:/proj/demo").unwrap();
        let info = mgr.get_session("s3").unwrap().unwrap();
        assert_eq!(info.project_path.as_deref(), Some("D:/proj/demo"));
        assert_eq!(info.message_count, 2, "绑项目不得影响消息行");
        let snap = mgr.load_snapshot("s3").unwrap();
        assert_eq!(snap.messages.len(), 2);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// 复盘数据面：思考正文与工具行都必须真正落盘（此前网关路径两者全丢）。
    #[test]
    fn thinking_and_tool_rows_are_persisted() {
        let (mut mgr, dir) = temp_db();
        mgr.init().unwrap();
        mgr.append_message("s4", "user", "排查一下项目", "2026-10-24T10:00:00Z", Some("排查一下项目"))
            .unwrap();
        mgr.append_tool_message(
            "s4",
            "read_file",
            "{\"path\":\"Cargo.toml\"}",
            "package name = marguerite",
            Some(37),
            Some(true),
            "2026-10-24T10:00:01Z",
        )
        .unwrap();
        mgr.append_message_with(
            "s4",
            "assistant",
            "已定位问题",
            "2026-10-24T10:00:02Z",
            None,
            Some("先读清单，再看依赖"),
        )
        .unwrap();

        let snap = mgr.load_snapshot("s4").unwrap();
        assert_eq!(snap.messages.len(), 3, "user / tool / assistant 三行都在");
        match &snap.messages[1] {
            Message::Tool {
                name,
                input,
                output,
                elapsed_ms,
                ..
            } => {
                assert_eq!(name, "read_file");
                assert!(input.contains("Cargo.toml"), "{input}");
                assert!(output.contains("marguerite"), "{output}");
                assert_eq!(*elapsed_ms, Some(37));
            }
            other => panic!("第二行应为 tool：{other:?}"),
        }
        match &snap.messages[2] {
            Message::Assistant { thinking, .. } => {
                assert_eq!(thinking.as_deref(), Some("先读清单，再看依赖"))
            }
            other => panic!("第三行应为 assistant：{other:?}"),
        }
        let info = mgr.get_session("s4").unwrap().unwrap();
        assert_eq!(info.message_count, 3);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// 超长工具输出/思考按字符截断，不得把 DB 撑爆，也不得切碎多字节字符。
    #[test]
    fn oversized_tool_fields_are_truncated_by_chars() {
        let (mut mgr, dir) = temp_db();
        mgr.init().unwrap();
        let huge = "玛".repeat(MAX_STORED_TOOL_FIELD_CHARS + 500);
        mgr.append_tool_message("s5", "shell_exec", &huge, &huge, None, None, "2026-10-24T10:00:01Z")
            .unwrap();
        let snap = mgr.load_snapshot("s5").unwrap();
        match &snap.messages[0] {
            Message::Tool {
                input, output, ..
            } => {
                assert_eq!(input.chars().count(), MAX_STORED_TOOL_FIELD_CHARS + 1);
                assert!(input.ends_with('…'));
                assert_eq!(output.chars().count(), MAX_STORED_TOOL_FIELD_CHARS + 1);
            }
            other => panic!("应为 tool：{other:?}"),
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    /// `sessions.search` 的对话面：必须真的扫到消息正文，并把命中处标出来。
    #[test]
    fn search_messages_finds_content_and_marks_the_hit() {
        let (mut mgr, dir) = temp_db();
        mgr.init().unwrap();
        let now = Utc::now();
        mgr.save_snapshot(
            "s-search",
            "idle",
            &[
                Message::User {
                    content: "帮我排查部署失败的原因".into(),
                    timestamp: now,
                },
                Message::Assistant {
                    content: "日志里显示端口被占用".into(),
                    timestamp: now,
                    thinking: None,
                },
            ],
            0,
        )
        .unwrap();

        let hits = mgr.search_messages("端口", 10).unwrap();
        assert_eq!(hits.len(), 1, "应命中助手消息：{hits:?}");
        assert_eq!(hits[0].session_id, "s-search");
        assert_eq!(hits[0].role, "assistant");
        assert!(hits[0].snippet.contains(">>>端口<<<"), "命中处必须可高亮：{}", hits[0].snippet);
        assert!(hits[0].created_at_ms > 0);

        // LIKE 元字符按字面匹配：`%` 不能变成通配符扫出全部消息
        assert!(mgr.search_messages("%", 10).unwrap().is_empty());
        // 空结果不 panic
        assert!(mgr.search_messages("不存在的词", 10).unwrap().is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn search_messages_scans_tool_output_columns() {
        let (mut mgr, dir) = temp_db();
        mgr.init().unwrap();
        mgr.append_tool_message(
            "s-tool",
            "shell_exec",
            "ls",
            "ENOENT: no such file",
            None,
            None,
            "2026-10-24T10:00:01Z",
        )
        .unwrap();
        let hits = mgr.search_messages("ENOENT", 10).unwrap();
        assert_eq!(hits.len(), 1, "工具输出必须可被搜到：{hits:?}");
        assert_eq!(hits[0].role, "tool");
        assert!(hits[0].snippet.contains(">>>ENOENT<<<"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn search_snippet_is_char_safe_and_marks_the_match() {
        let long = format!("{}{}{}", "前".repeat(100), "命中的词", "后".repeat(100));
        let snippet = search_snippet(&long, "命中的词");
        assert!(snippet.contains(">>>命中的词<<<"));
        assert!(snippet.starts_with('…'), "左侧被截断应有省略号：{snippet}");
        assert!(snippet.ends_with('…'), "右侧被截断应有省略号：{snippet}");
        // 大小写不敏感，但标记区间必须来自原文
        assert_eq!(search_snippet("The Deployment Failed", "deployment"), "The >>>Deployment<<< Failed");
        // 未命中时退化为纯前缀，绝不 panic
        assert_eq!(search_snippet("短文本", "缺"), "短文本");
    }
}
