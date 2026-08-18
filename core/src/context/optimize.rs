//! 上下文优化（KV-Cache 极简工程实践 §2.9）
//!
//! 三条最小杠杆，直接放大缓存命中率：
//! - **稳定前缀重组（P1）**：system/persona/KB 前置，volatile 后置 → 缓存键稳定部分对齐
//! - **滑动窗口 + 注意力 sink（StreamingLLM）**：保留首条 sink（通常就是 system）+ 最近 N 轮
//! - **可缓存比**：stable 前缀 token / 总 token，指导把易变数据移出前缀

use crate::model::llm::token_estimate;
use crate::types::Message;

/// 把稳定前缀（所有 System 消息）前置，volatile（user/assistant/tool）保持相对顺序后置。
///
/// KV-Cache 原则 P1：不变的东西放最前、每请求都变的东西放最后，否则前缀永远不命中。
pub fn reorder_stable_prefix(messages: &[Message]) -> Vec<Message> {
    let mut stable: Vec<Message> = Vec::new();
    let mut rest: Vec<Message> = Vec::new();
    for m in messages {
        match m {
            Message::System { .. } => stable.push(m.clone()),
            _ => rest.push(m.clone()),
        }
    }
    stable.extend(rest);
    stable
}

/// StreamingLLM 式滑动窗口：保留前 `sink` 条（注意力 sink），丢弃中间，保留最后 `recent` 条。
///
/// 长对话不再无限膨胀；首条 sink 通常就是 system 前缀，被服务端 KV cache 持续复用。
pub fn compact_window(messages: &[Message], sink: usize, recent: usize) -> Vec<Message> {
    if messages.len() <= sink + recent {
        return messages.to_vec();
    }
    let mut out = messages[..sink].to_vec();
    let tail = &messages[messages.len() - recent..];
    out.extend_from_slice(tail);
    out
}

/// 稳定前缀（System 消息）的拼接文本（供哈希 / token 估算复用）
pub fn stable_prefix_text(messages: &[Message]) -> String {
    messages
        .iter()
        .filter_map(|m| match m {
            Message::System { content, .. } => Some(content.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// 稳定前缀（System 消息）的 token 估算（family 微调系数）
pub fn stable_prefix_tokens(messages: &[Message], family: f64) -> usize {
    token_estimate(&stable_prefix_text(messages), family)
}

/// 可缓存比 ∈ [0,1]：stable 前缀 token / 总 token。越接近 1，缓存命中率潜力越高。
pub fn prefix_cacheable_ratio(messages: &[Message], family: f64) -> f64 {
    let total: usize = messages
        .iter()
        .map(|m| match m {
            Message::User { content, .. }
            | Message::Assistant { content, .. }
            | Message::System { content, .. } => token_estimate(content, family),
            Message::Tool { input, output, .. } => {
                token_estimate(input, family) + token_estimate(output, family)
            }
        })
        .sum();
    if total == 0 {
        return 0.0;
    }
    stable_prefix_tokens(messages, family) as f64 / total as f64
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn content(m: &Message) -> &str {
        match m {
            Message::User { content, .. }
            | Message::Assistant { content, .. }
            | Message::System { content, .. } => content,
            Message::Tool { .. } => "",
        }
    }

    #[test]
    fn reorder_moves_system_to_front() {
        let msgs = vec![
            Message::User {
                content: "q".into(),
                timestamp: Utc::now(),
            },
            Message::System {
                content: "sys".into(),
                timestamp: Utc::now(),
            },
            Message::User {
                content: "q2".into(),
                timestamp: Utc::now(),
            },
        ];
        let out = reorder_stable_prefix(&msgs);
        assert_eq!(out[0].role(), "system", "System 应前置");
    }

    #[test]
    fn reorder_preserves_volatile_relative_order() {
        let msgs = vec![
            Message::User {
                content: "a".into(),
                timestamp: Utc::now(),
            },
            Message::System {
                content: "s".into(),
                timestamp: Utc::now(),
            },
            Message::User {
                content: "b".into(),
                timestamp: Utc::now(),
            },
            Message::Assistant {
                content: "c".into(),
                timestamp: Utc::now(),
                thinking: None,
            },
        ];
        let out = reorder_stable_prefix(&msgs);
        assert_eq!(out[0].role(), "system");
        // volatile 部分保持 a, b, c 的相对顺序
        let volatile: Vec<&str> = out[1..]
            .iter()
            .map(|m| match m {
                Message::User { content, .. } | Message::Assistant { content, .. } => {
                    content.as_str()
                }
                _ => "",
            })
            .collect();
        assert_eq!(volatile, vec!["a", "b", "c"]);
    }

    #[test]
    fn compact_window_keeps_sink_and_recent() {
        let msgs: Vec<Message> = (0..10)
            .map(|i| Message::User {
                content: format!("m{i}"),
                timestamp: Utc::now(),
            })
            .collect();
        let out = compact_window(&msgs, 1, 2);
        assert_eq!(out.len(), 3, "sink(1)+recent(2)=3");
        assert_eq!(content(&out[0]), "m0", "sink 保留首条");
        assert_eq!(content(&out[2]), "m9", "recent 保留末条");
    }

    #[test]
    fn compact_window_noop_when_short() {
        let msgs: Vec<Message> = (0..3)
            .map(|i| Message::User {
                content: format!("m{i}"),
                timestamp: Utc::now(),
            })
            .collect();
        let out = compact_window(&msgs, 1, 2);
        assert_eq!(out.len(), 3);
    }

    #[test]
    fn prefix_cacheable_ratio_all_system_is_one() {
        let msgs = vec![Message::System {
            content: "long stable system prompt".into(),
            timestamp: Utc::now(),
        }];
        let r = prefix_cacheable_ratio(&msgs, 1.0);
        assert!((r - 1.0).abs() < 1e-9, "全 System 可缓存比应为 1, got {r}");
    }

    #[test]
    fn prefix_cacheable_ratio_no_system_is_zero() {
        let msgs = vec![Message::User {
            content: "volatile user input".into(),
            timestamp: Utc::now(),
        }];
        let r = prefix_cacheable_ratio(&msgs, 1.0);
        assert!((r - 0.0).abs() < 1e-9, "无 System 可缓存比应为 0, got {r}");
    }

    #[test]
    fn stable_prefix_tokens_counts_only_system() {
        let msgs = vec![
            Message::System {
                content: "stable".into(),
                timestamp: Utc::now(),
            },
            Message::User {
                content: "volatile".into(),
                timestamp: Utc::now(),
            },
        ];
        // "stable" = 6 ASCII → 6*0.25 = 1.5 → ceil 2
        assert_eq!(stable_prefix_tokens(&msgs, 1.0), 2);
    }
}
