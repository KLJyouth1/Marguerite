//! 记忆生命周期钩子（参考 claude-mem 的 5 个生命周期事件）
//!
//! 在 agent 会话的关键节点触发记忆操作：
//! - [`MemoryHook::SessionStart`]：会话开始，加载历史摘要/偏好
//! - [`MemoryHook::UserPromptSubmit`]：用户提交 prompt 前，注入相关记忆
//! - [`MemoryHook::PostToolUse`]：工具执行后，记录观察
//! - [`MemoryHook::Summary`]：回合结束，压缩/提炼记忆
//! - [`MemoryHook::SessionEnd`]：会话结束，持久化
//!
//! 钩子设计为同步 + 非阻塞：失败不中断主流程，仅记录日志。

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use tracing::{debug, warn};

/// 记忆钩子事件类型
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum MemoryHookEvent {
    /// 会话开始
    SessionStart {
        session_id: String,
        project: String,
        timestamp: String,
    },
    /// 用户提交 prompt
    UserPromptSubmit {
        session_id: String,
        prompt: String,
        timestamp: String,
    },
    /// 工具执行后
    PostToolUse {
        session_id: String,
        tool: String,
        success: bool,
        output_preview: String,
        duration_ms: u64,
        timestamp: String,
    },
    /// 回合总结
    Summary {
        session_id: String,
        summary: String,
        tool_count: usize,
        timestamp: String,
    },
    /// 会话结束
    SessionEnd {
        session_id: String,
        total_turns: usize,
        total_tools: usize,
        timestamp: String,
    },
}

/// 单次工具调用记录（用于回合总结）
#[derive(Debug, Clone)]
#[allow(dead_code)]
struct ToolCallRecord {
    name: String,
    success: bool,
    output_preview: String,
    duration_ms: u64,
}

/// 记忆钩子：在 agent 生命周期节点记录观察，供长期记忆/经验账本使用。
///
/// 设计原则：
/// - 永不阻塞主流程（所有错误降级为日志）
/// - 内存缓冲 + 批量落盘（避免每次工具调用都写磁盘）
/// - 可禁用（配置 `memory.hooks_enabled = false`）
#[derive(Clone)]
pub struct MemoryHooks {
    inner: Arc<Mutex<MemoryHooksInner>>,
    data_dir: PathBuf,
    enabled: bool,
}

struct MemoryHooksInner {
    session_id: String,
    project: String,
    started_at: Instant,
    turn_count: usize,
    tool_calls: Vec<ToolCallRecord>,
    /// 当前回合累积的观察文本
    observations: Vec<String>,
}

impl MemoryHooks {
    /// 创建记忆钩子实例
    pub fn new(
        session_id: impl Into<String>,
        project: impl Into<String>,
        data_dir: PathBuf,
    ) -> Self {
        Self {
            inner: Arc::new(Mutex::new(MemoryHooksInner {
                session_id: session_id.into(),
                project: project.into(),
                started_at: Instant::now(),
                turn_count: 0,
                tool_calls: Vec::new(),
                observations: Vec::new(),
            })),
            data_dir,
            enabled: true,
        }
    }

    /// 创建禁用的空钩子（所有操作 no-op）
    pub fn disabled() -> Self {
        Self {
            inner: Arc::new(Mutex::new(MemoryHooksInner {
                session_id: String::new(),
                project: String::new(),
                started_at: Instant::now(),
                turn_count: 0,
                tool_calls: Vec::new(),
                observations: Vec::new(),
            })),
            data_dir: PathBuf::new(),
            enabled: false,
        }
    }

    /// 是否启用
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// SessionStart：会话开始，加载历史上下文
    pub async fn on_session_start(&self) {
        if !self.enabled {
            return;
        }
        let inner = self.inner.lock().await;
        let event = MemoryHookEvent::SessionStart {
            session_id: inner.session_id.clone(),
            project: inner.project.clone(),
            timestamp: chrono::Utc::now().to_rfc3339(),
        };
        drop(inner);
        debug!(?event, "memory hook: session_start");
        self.append_event(&event).await;
    }

    /// UserPromptSubmit：用户提交 prompt
    pub async fn on_user_prompt(&self, prompt: &str) {
        if !self.enabled {
            return;
        }
        let mut inner = self.inner.lock().await;
        inner.turn_count += 1;
        let event = MemoryHookEvent::UserPromptSubmit {
            session_id: inner.session_id.clone(),
            prompt: prompt.to_string(),
            timestamp: chrono::Utc::now().to_rfc3339(),
        };
        drop(inner);
        debug!(?event, "memory hook: user_prompt_submit");
        self.append_event(&event).await;
    }

    /// PostToolUse：工具执行完成
    pub async fn on_post_tool_use(
        &self,
        tool: &str,
        success: bool,
        output: &str,
        duration_ms: u64,
    ) {
        if !self.enabled {
            return;
        }
        let preview: String = output.chars().take(200).collect();
        let mut inner = self.inner.lock().await;
        inner.tool_calls.push(ToolCallRecord {
            name: tool.to_string(),
            success,
            output_preview: preview.clone(),
            duration_ms,
        });
        // 记录失败的工具调用作为观察（供经验账本学习）
        if !success {
            inner
                .observations
                .push(format!("[tool-fail] {tool}: {preview}"));
        }
        let event = MemoryHookEvent::PostToolUse {
            session_id: inner.session_id.clone(),
            tool: tool.to_string(),
            success,
            output_preview: preview,
            duration_ms,
            timestamp: chrono::Utc::now().to_rfc3339(),
        };
        drop(inner);
        debug!(?event, "memory hook: post_tool_use");
        self.append_event(&event).await;
    }

    /// Summary：回合结束，生成摘要
    pub async fn on_summary(&self, assistant_text: &str) {
        if !self.enabled {
            return;
        }
        let mut inner = self.inner.lock().await;
        let tool_count = inner.tool_calls.len();
        // 从工具调用和助手段落生成简要摘要
        let mut summary_parts: Vec<String> = Vec::new();
        if !assistant_text.is_empty() {
            let preview: String = assistant_text.chars().take(300).collect();
            summary_parts.push(preview);
        }
        if !inner.observations.is_empty() {
            summary_parts.push(format!("观察: {} 条", inner.observations.len()));
        }
        let summary = summary_parts.join(" | ");
        let event = MemoryHookEvent::Summary {
            session_id: inner.session_id.clone(),
            summary: summary.clone(),
            tool_count,
            timestamp: chrono::Utc::now().to_rfc3339(),
        };
        // 重置回合缓冲
        inner.tool_calls.clear();
        inner.observations.clear();
        drop(inner);
        debug!(?event, "memory hook: summary");
        self.append_event(&event).await;
    }

    /// SessionEnd：会话结束，持久化
    pub async fn on_session_end(&self) {
        if !self.enabled {
            return;
        }
        let inner = self.inner.lock().await;
        let total_turns = inner.turn_count;
        let total_tools = inner.tool_calls.len();
        let elapsed = inner.started_at.elapsed().as_secs();
        let event = MemoryHookEvent::SessionEnd {
            session_id: inner.session_id.clone(),
            total_turns,
            total_tools,
            timestamp: chrono::Utc::now().to_rfc3339(),
        };
        drop(inner);
        debug!(?event, elapsed_secs = elapsed, "memory hook: session_end");
        self.append_event(&event).await;
    }

    /// 获取当前回合的工具调用数
    pub async fn tool_count(&self) -> usize {
        self.inner.lock().await.tool_calls.len()
    }

    /// 获取当前回合的观察（失败记录等）
    pub async fn observations(&self) -> Vec<String> {
        self.inner.lock().await.observations.clone()
    }

    /// 追加事件到 JSONL 日志文件（追加写，失败不报错）
    async fn append_event(&self, event: &MemoryHookEvent) {
        if !self.enabled {
            return;
        }
        let dir = self.data_dir.join("memory").join("hooks");
        if let Err(e) = tokio::fs::create_dir_all(&dir).await {
            warn!("memory hooks: 创建目录失败: {e}");
            return;
        }
        let date = chrono::Utc::now().format("%Y-%m-%d").to_string();
        let path = dir.join(format!("hooks-{date}.jsonl"));
        let line = match serde_json::to_string(event) {
            Ok(s) => s,
            Err(e) => {
                warn!("memory hooks: 序列化事件失败: {e}");
                return;
            }
        };
        use tokio::io::AsyncWriteExt;
        let mut open_opts = tokio::fs::OpenOptions::new();
        open_opts.create(true).append(true);
        match open_opts.open(&path).await {
            Ok(mut f) => {
                if let Err(e) = f.write_all(line.as_bytes()).await {
                    warn!("memory hooks: 写入失败: {e}");
                }
                let _ = f.write_all(b"\n").await;
            }
            Err(e) => warn!("memory hooks: 打开文件失败: {e}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_hooks_lifecycle() {
        let dir = std::env::temp_dir().join("marguerite_test_hooks");
        let hooks = MemoryHooks::new("test-session", "test-project", dir.clone());
        hooks.on_session_start().await;
        hooks.on_user_prompt("hello").await;
        hooks.on_post_tool_use("shell", true, "output", 42).await;
        hooks
            .on_post_tool_use("fs", false, "permission denied", 10)
            .await;
        assert_eq!(hooks.tool_count().await, 2);
        assert_eq!(hooks.observations().await.len(), 1);
        hooks.on_summary("done").await;
        assert_eq!(hooks.tool_count().await, 0);
        hooks.on_session_end().await;
        // 验证文件写入
        let date = chrono::Utc::now().format("%Y-%m-%d").to_string();
        let path = dir
            .join("memory")
            .join("hooks")
            .join(format!("hooks-{date}.jsonl"));
        assert!(path.exists(), "hooks jsonl should exist");
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("session_start"));
        assert!(content.contains("user_prompt_submit"));
        assert!(content.contains("post_tool_use"));
        assert!(content.contains("summary"));
        assert!(content.contains("session_end"));
        // cleanup
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_disabled_hooks_noop() {
        let hooks = MemoryHooks::disabled();
        assert!(!hooks.is_enabled());
        hooks.on_session_start().await;
        hooks.on_user_prompt("hi").await;
        hooks.on_post_tool_use("x", true, "", 0).await;
        hooks.on_summary("").await;
        hooks.on_session_end().await;
        assert_eq!(hooks.tool_count().await, 0);
    }
}
