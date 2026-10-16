//! 上下文优化（KV-Cache 极简工程实践 §2.9）
//!
//! 三条最小杠杆，直接放大缓存命中率：
//! - **稳定前缀重组（P1）**：system/persona/KB 前置，volatile 后置 → 缓存键稳定部分对齐
//! - **滑动窗口 + 注意力 sink（StreamingLLM）**：保留首条 sink（通常就是 system）+ 最近 N 轮
//! - **可缓存比**：stable 前缀 token / 总 token，指导把易变数据移出前缀

use crate::model::llm::token_estimate;
use crate::types::Message;

/// 稳定前缀的**唯一权威切分**：开头连续的 System 消息（leading run）。
///
/// 「稳定前缀」在本工程有三处消费方——[`reorder_stable_prefix`]（重排边界）、
/// [`stable_prefix_text`]（指纹/token 口径）、`model::llm::warmup`（预热发送范围）。
/// 三者必须完全一致：只要有一处把中段 volatile System 也算进来，指纹就对不上，
/// 预热登记的前缀匹配不上真实请求、`prefix_saved_tokens` 也永远为 0。
/// 因此把切分收敛到这一个函数，谁都不许再自己数一遍。
pub fn leading_stable_prefix(messages: &[Message]) -> &[Message] {
    let leading = messages
        .iter()
        .take_while(|m| matches!(m, Message::System { .. }))
        .count();
    &messages[..leading]
}

/// 把稳定前缀（leading run）前置，volatile（user/assistant/tool 及其后所有消息）保持
/// 相对顺序后置。
///
/// KV-Cache 原则 P1：不变的东西放最前、每请求都变的东西放最后，否则前缀永远不命中。
///
/// M2 降耗引擎修正：**只提升 leading run**。出现在首个 User/Assistant/Tool 消息
/// 之后的 System 消息是装配层刻意放置的 volatile 段（记忆召回 / 经验提示 /
/// Track A 路由片段，见 `agent::runtime::build_model_messages` 动态区），必须保持
/// 原位——若无条件前置所有 System 消息，会把这些逐轮变化段挪回历史之前，从第 0
/// 字节摧毁服务端 KV 前缀复用。
pub fn reorder_stable_prefix(messages: &[Message]) -> Vec<Message> {
    // leading run 本身已在前缀区，无需移动；其余消息（含 mid-sequence System）
    // 按原顺序拼接，保持逐字节位置不变。
    let leading = leading_stable_prefix(messages).len();
    let mut out: Vec<Message> = Vec::with_capacity(messages.len());
    out.extend(messages[..leading].iter().cloned());
    out.extend(messages[leading..].iter().cloned());
    out
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

/// 稳定前缀（**开头连续的 System 消息**，即 leading run）的拼接文本（供哈希 / token 估算复用）。
///
/// **必须与 [`reorder_stable_prefix`] 的边界完全一致**：只取 leading run。
/// 首个 User/Assistant/Tool **之后**的 System 消息是装配层刻意放置的 volatile 段
/// （记忆召回 / 经验提示 / Track A 路由片段，见 `agent::runtime::build_model_messages`），
/// 它们逐轮变化。
///
/// 为什么这条边界是硬要求：`model::llm` 用本函数的结果作为「稳定前缀指纹」喂给
/// [`crate::cache::PrefixCache::note_prefix`]，而后者以「该指纹是否出现过」判定前缀复用
/// （命中即累加 `prefix_saved_tokens`）。若把 volatile 段并进来，指纹每轮都不同 →
/// `note_prefix` 永远判定「新前缀」→ **`prefix_saved_tokens` 结构性恒为 0**，
/// 前缀复用指标彻底失效（用户看到「缓存命中率永远是 0」）。
pub fn stable_prefix_text(messages: &[Message]) -> String {
    leading_stable_prefix(messages)
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
        // M2 降耗引擎新契约：只有 leading run（开头连续 System）参与前置语义；
        // 首条非 System 之后的消息（含 System）全部保持原位。
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
        let roles: Vec<&str> = out.iter().map(|m| m.role()).collect();
        assert_eq!(
            roles,
            vec!["user", "system", "user"],
            "无 leading System 时序列保持原样（mid-sequence System 不前置）"
        );
    }

    #[test]
    fn reorder_keeps_mid_sequence_system_in_place() {
        // M2 降耗引擎契约：出现在首个非 System 消息之后的 System 段是装配层
        // 刻意放置的 volatile 段（记忆召回 / 经验提示），必须保持原位，
        // 不得被前置回历史之前（否则摧毁服务端 KV 前缀复用）。
        let sys_front = Message::System {
            content: "静态规则".into(),
            timestamp: Utc::now(),
        };
        let user1 = Message::User {
            content: "q1".into(),
            timestamp: Utc::now(),
        };
        let sys_dynamic = Message::System {
            content: "逐轮记忆召回".into(),
            timestamp: Utc::now(),
        };
        let user2 = Message::User {
            content: "q2".into(),
            timestamp: Utc::now(),
        };
        let msgs = vec![sys_front.clone(), user1.clone(), sys_dynamic.clone(), user2.clone()];
        let out = reorder_stable_prefix(&msgs);
        assert_eq!(out.len(), 4);
        assert_eq!(out[0].role(), "system", "leading System 保持首位");
        assert_eq!(
            content(&out[1]), "q1",
            "首个非 System 之后的消息保持原顺序"
        );
        assert_eq!(
            content(&out[2]), "逐轮记忆召回",
            "mid-sequence System 不得被前置，必须保持原位"
        );
        assert_eq!(content(&out[3]), "q2");
        // 整体应与输入完全一致（leading run 本已在前，其余不动）
        assert_eq!(out.len(), msgs.len());
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
        // 无 leading System 时序列完全不动（mid-sequence System 保持原位）
        let roles: Vec<&str> = out.iter().map(|m| m.role()).collect();
        assert_eq!(roles, vec!["user", "system", "user", "assistant"]);
        let contents: Vec<String> = out
            .iter()
            .map(|m| match m {
                Message::User { content, .. } | Message::Assistant { content, .. } => {
                    content.clone()
                }
                _ => String::new(),
            })
            .collect();
        assert_eq!(contents, vec!["a".to_string(), String::new(), "b".to_string(), "c".to_string()]);
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

    fn sys(content: &str) -> Message {
        Message::System {
            content: content.into(),
            timestamp: Utc::now(),
        }
    }

    fn user(content: &str) -> Message {
        Message::User {
            content: content.into(),
            timestamp: Utc::now(),
        }
    }

    #[test]
    fn stable_prefix_text_takes_only_leading_run() {
        // 两个 leading System 都算，首个 User 之后的 System（volatile 记忆召回）不算
        let msgs = vec![
            sys("persona"),
            sys("tool-schema"),
            user("问题"),
            sys("记忆召回：本轮才有的片段"),
            user("继续"),
        ];
        assert_eq!(stable_prefix_text(&msgs), "persona\ntool-schema");
    }

    #[test]
    fn stable_prefix_text_is_empty_without_leading_system() {
        // 首条不是 System → 没有稳定前缀（不能把中间那条 System 误当成前缀）
        let msgs = vec![user("hi"), sys("mid-sequence recall")];
        assert_eq!(stable_prefix_text(&msgs), "");
        assert_eq!(stable_prefix_tokens(&msgs, 1.0), 0);
    }

    /// 回归守卫：volatile 段逐轮变化**不得**改变稳定前缀指纹。
    ///
    /// 此前 `stable_prefix_text` 把**所有** System 消息拼在一起，于是每轮变化的中段
    /// 记忆召回污染了指纹 → `PrefixCache::note_prefix` 永远判定「新前缀」→
    /// `prefix_saved_tokens` 结构性恒为 0。这里直接把「跨轮指纹一致」钉死。
    #[test]
    fn stable_prefix_fingerprint_survives_volatile_recall_changes() {
        let turn = |recall: &str| {
            vec![
                sys("persona"),
                sys("tool-schema"),
                user("第一问"),
                sys(recall),
                user("第二问"),
            ]
        };
        let t1 = stable_prefix_text(&turn("召回片段 A"));
        let t2 = stable_prefix_text(&turn("召回片段 B —— 完全不同"));
        assert_eq!(t1, t2, "volatile 记忆召回变化不得影响稳定前缀文本");
        assert_eq!(crate::cache::hash_text(&t1), crate::cache::hash_text(&t2));

        // 而真正的稳定前缀变化必须被检出（否则守卫失效）
        let t3 = stable_prefix_text(&[sys("persona-changed"), sys("tool-schema"), user("第一问")]);
        assert_ne!(t1, t3);
    }

    #[test]
    fn stable_prefix_reuse_is_observable_through_note_prefix() {
        let turn = |recall: &str| {
            vec![sys("persona"), sys("tool-schema"), user("q"), sys(recall)]
        };
        let cache = crate::cache::PrefixCache::new(8);
        let first = crate::cache::hash_text(&stable_prefix_text(&turn("召回 A")));
        let second = crate::cache::hash_text(&stable_prefix_text(&turn("召回 B")));
        let tokens = stable_prefix_tokens(&turn("召回 A"), 1.0) as u32;

        assert!(!cache.note_prefix(first, tokens), "首次出现应为「新前缀」");
        assert!(
            cache.note_prefix(second, tokens),
            "同一稳定前缀的第二次调用必须判定为复用（否则 prefix_saved_tokens 恒为 0）"
        );
        assert!(
            cache.stats().prefix_saved_tokens > 0,
            "复用必须如实计入 prefix_saved_tokens"
        );
    }

    /// 三处消费方必须共用同一条边界（leading run）：重排 / 指纹 / 预热发送范围。
    /// 任何一处自己再数一遍，都会让指纹与预热登记对不上。
    #[test]
    fn leading_run_boundary_is_shared_by_all_consumers() {
        let msgs = vec![
            sys("persona"),
            sys("kb"),
            user("q"),
            sys("volatile recall"),
            user("q2"),
        ];
        // 重排不得移动中段 System（边界一致）
        let ordered = reorder_stable_prefix(&msgs);
        assert!(matches!(ordered[3], Message::System { .. }), "中段 System 应保持原位");
        assert_eq!(ordered.len(), msgs.len());
        // 指纹只覆盖 leading run，与重排边界一致
        assert_eq!(stable_prefix_text(&msgs), "persona\nkb");
        assert_eq!(leading_stable_prefix(&msgs).len(), 2);
        // 预热发送范围也必须是同一段（用 leading_stable_prefix 取，天然一致）
        let warm: Vec<String> = leading_stable_prefix(&ordered)
            .iter()
            .map(|m| match m {
                Message::System { content, .. } => content.clone(),
                _ => unreachable!(),
            })
            .collect();
        assert_eq!(warm, vec!["persona".to_string(), "kb".to_string()]);
        // 且预热指纹 == 真实请求指纹（预热收益才能计上）
        assert_eq!(
            crate::cache::hash_text(&stable_prefix_text(&ordered)),
            crate::cache::hash_text(&warm.join("\n"))
        );
    }
}
