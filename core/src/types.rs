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
