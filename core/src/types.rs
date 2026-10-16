//! 共享类型定义
//!
//! 定义核心系统中使用的共享类型，避免模块间的循环依赖。

use chrono::{DateTime, Utc};

/// 消息枚举
#[derive(Debug, Clone)]
pub enum Message {
    /// 用户消息
    User {
        /// 消息内容
        content: String,
        /// 时间戳
        timestamp: DateTime<Utc>,
    },
    /// 助手消息
    Assistant {
        /// 消息内容
        content: String,
        /// 时间戳
        timestamp: DateTime<Utc>,
        /// 思考过程
        thinking: Option<String>,
    },
    /// 系统消息
    System {
        /// 消息内容
        content: String,
        /// 时间戳
        timestamp: DateTime<Utc>,
    },
    /// 工具调用消息
    Tool {
        /// 工具名称
        name: String,
        /// 输入参数
        input: String,
        /// 输出结果
        output: String,
        /// 时间戳
        timestamp: DateTime<Utc>,
        /// P2-10/T14：该次工具执行的**实测耗时**（毫秒）。仅在可精确归因单次调用时
        /// 记录（单轮单工具）；批量调用或多路并行无法拆分时保持 `None`（诚实：宁可
        /// 不显示，也不给一个错的耗时）。旧数据 `None` → 历史卡不显示耗时。
        elapsed_ms: Option<u64>,
    },
}

impl Message {
    /// 获取消息时间戳
    pub fn timestamp(&self) -> &DateTime<Utc> {
        match self {
            Message::User { timestamp, .. }
            | Message::Assistant { timestamp, .. }
            | Message::System { timestamp, .. }
            | Message::Tool { timestamp, .. } => timestamp,
        }
    }

    /// 获取消息角色
    pub fn role(&self) -> &'static str {
        match self {
            Message::User { .. } => "user",
            Message::Assistant { .. } => "assistant",
            Message::System { .. } => "system",
            Message::Tool { .. } => "tool",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_message_roles() {
        let now = Utc::now();

        let user_msg = Message::User {
            content: "hello".to_string(),
            timestamp: now,
        };
        assert_eq!(user_msg.role(), "user");

        let assistant_msg = Message::Assistant {
            content: "hi".to_string(),
            timestamp: now,
            thinking: None,
        };
        assert_eq!(assistant_msg.role(), "assistant");

        let system_msg = Message::System {
            content: "be helpful".to_string(),
            timestamp: now,
        };
        assert_eq!(system_msg.role(), "system");

        let tool_msg = Message::Tool {
            name: "test".to_string(),
            input: "input".to_string(),
            output: "output".to_string(),
            timestamp: now,
            elapsed_ms: None,
        };
        assert_eq!(tool_msg.role(), "tool");
    }

    #[test]
    fn test_message_timestamp() {
        let now = Utc::now();
        let msg = Message::User {
            content: "hello".to_string(),
            timestamp: now,
        };
        assert_eq!(msg.timestamp(), &now);
    }
}
