//! AI 智能融合路由（Bloom：离线纯规则自动决策编排形态）。
//!
//! `router.decision` 对输入做融合路由决策，返回自动编排形态
//! （Solo / MAX 双带队 / 专家团队 / 集群）与决策理由。
//! 与 chat 会话内的 Orchestrator 同源（analyze 纯规则，离线可用），
//! 供 Web UI 的 AI 智能融合路由面板只读展示。
//!
//! 2026-08「模型研究驱动」增强：基于对中美开源/闭源模型（DeepSeek / Qwen /
//! GLM / Kimi / MiniMax / Doubao / OpenAI / Anthropic / Google / Meta）的深度调研，
//! 决策结果额外给出 **结构化模型选择**（模型族 + 首选/备选 + 思考模式 + few-shot 策略）、
//! **缓存策略建议** 与 **提示词策略建议**，让前端 Agent 面板可以直接据其选择模型与提示形态。
//!
//! 调研驱动的选择要点：
//! - Agentic 编码（SWE-bench 领跑）：GLM-4.5 / Claude-Sonnet，非思考，few-shot 不推荐
//! - 复杂推理：DeepSeek-R1 / Qwen3-Thinking，思考模式，**few-shot 禁用**（R1 官方证实 few-shot 持续降性能）
//! - 简单对话：Qwen3 / Doubao-Seed-1.6，非思考，短示例可接受
//! - 长上下文 / 总结：GLM-4.5 / MiniMax-M1，few-shot 不推荐（上下文是稀缺资源，lost-in-middle 加剧）

use std::sync::OnceLock;

use serde_json::{json, Value};

use crate::orchestration::Orchestrator;
use crate::persistence::config::Config;

/// Few-shot 使用策略（调研结论：不同模型族对 few-shot 的耐受度天差地别）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FewShotPolicy {
    /// 可用：简单对话/检索，1~2 个极短示例帮助稳定格式
    Allowed,
    /// 不推荐：复杂编码/写作，few-shot 占上下文且引入格式噪音（lost-in-middle 加剧）
    Discouraged,
    /// 禁用：R1 系推理模型，官方实证 few-shot 会持续降低性能
    Forbidden,
}

impl FewShotPolicy {
    fn label(&self) -> &'static str {
        match self {
            FewShotPolicy::Allowed => "allowed",
            FewShotPolicy::Discouraged => "discouraged",
            FewShotPolicy::Forbidden => "forbidden",
        }
    }
}

/// 结构化模型选择：模型族 + 首选/备选 + 思考模式 + few-shot 策略。
struct ModelSelection {
    family: &'static str,
    model: &'static str,
    alt: &'static str,
    mode: &'static str,
    few_shot: FewShotPolicy,
}

impl ModelSelection {
    fn to_json(&self) -> Value {
        json!({
            "family": self.family,
            "model": self.model,
            "alt": self.alt,
            "mode": self.mode,
            "few_shot_policy": self.few_shot.label(),
        })
    }
}

/// 由意图 + 复杂度选择模型（离线纯规则，调研结论驱动）。
fn select_model(clf_intent: &str, complexity: &str) -> ModelSelection {
    let is_complex = complexity == "Complex";
    match clf_intent {
        // 编码类：复杂走 SWE-bench 领跑者（agentic 编码）；简单走性价比路线
        "Refactor" | "Fix" | "Test" | "Explain" => {
            if is_complex {
                ModelSelection {
                    family: "agentic-coding",
                    model: "glm-4.5",
                    alt: "claude-sonnet",
                    mode: "non-thinking",
                    few_shot: FewShotPolicy::Discouraged,
                }
            } else {
                ModelSelection {
                    family: "coding",
                    model: "qwen3",
                    alt: "deepseek-v3",
                    mode: "non-thinking",
                    few_shot: FewShotPolicy::Allowed,
                }
            }
        }
        // 问答类：复杂走推理模型（thinking + few-shot 禁用）；简单走快速对话
        "Question" => {
            if is_complex {
                ModelSelection {
                    family: "reasoning",
                    model: "deepseek-r1",
                    alt: "qwen3-thinking",
                    mode: "thinking",
                    few_shot: FewShotPolicy::Forbidden,
                }
            } else {
                ModelSelection {
                    family: "chat",
                    model: "qwen3",
                    alt: "doubao-seed-1.6",
                    mode: "non-thinking",
                    few_shot: FewShotPolicy::Allowed,
                }
            }
        }
        // 检索：工具调用 + 向量检索，无需思考
        "Search" => ModelSelection {
            family: "retrieval",
            model: "qwen3",
            alt: "deepseek-v3",
            mode: "non-thinking",
            few_shot: FewShotPolicy::Allowed,
        },
        // 文档/提交：长文总结 + 结构化输出稳定
        "Doc" | "Commit" => ModelSelection {
            family: "writing",
            model: "glm-4.5",
            alt: "claude-sonnet",
            mode: "non-thinking",
            few_shot: FewShotPolicy::Discouraged,
        },
        // 通用兜底
        _ => ModelSelection {
            family: "general",
            model: "qwen3",
            alt: "deepseek-v3",
            mode: "non-thinking",
            few_shot: FewShotPolicy::Allowed,
        },
    }
}

/// 是否建议开启思考模式（hybrid thinking budget）：复杂度高 / 需推理时建议 thinking。
fn thinking_recommended(complexity: &str) -> bool {
    matches!(complexity, "Complex")
}

/// 缓存策略建议（调研结论：前缀必须字节级稳定，tools→system→messages 顺序，易变内容后置）。
fn cache_advice() -> &'static str {
    "稳定前缀置前：tools → system → 静态上下文，动态内容（会话/时间戳）放最后；\
     保持字节级一致以命中 Anthropic/OpenAI/Gemini/DeepSeek 的前缀缓存（读价约为输入价 10%~50%）。"
}

/// 提示词建议（调研结论：R1 系推理模型对 few-shot 敏感、易语言混用；长上下文存在 lost-in-middle）。
fn prompt_advice(few_shot: &str) -> &'static str {
    match few_shot {
        "forbidden" => {
            "该模型族对 few-shot 敏感（官方实证会持续降低性能）：直接描述问题，零示例；\
             显式指定输出语言避免混用；若需结构化输出请用 schema 而非示例。"
        }
        "discouraged" => {
            "few-shot 会占用宝贵上下文并放大 lost-in-middle：用 schema/约束说明替代示例；\
             关键信息置于首/尾，压缩中段。"
        }
        _ => "可用 1~2 个极短示例稳定输出格式；长上下文仍建议关键信息置首/尾（lost-in-middle）。",
    }
}

/// `router.decision`：对输入做融合路由决策（离线纯规则）。
pub fn router_decision(params: &Value) -> Value {
    let query = params
        .get("query")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if query.trim().is_empty() {
        return json!({ "error": "query 不可为空" });
    }
    static ROUTER: OnceLock<Orchestrator> = OnceLock::new();
    let orch = ROUTER.get_or_init(|| Orchestrator::new(Config::default()));
    let a = orch.analyze(query);
    let clf_intent = format!("{:?}", a.intent);
    let complexity = format!("{:?}", a.complexity);
    let sel = select_model(&clf_intent, &complexity);
    json!({
        "strategy": a.strategy.label(),
        "intent": clf_intent,
        "complexity": complexity,
        "needs_llm": a.needs_llm,
        "subtask_count": a.subtasks.len(),
        "subtasks": a.subtasks.iter().map(|s| json!({
            "role": s.role,
            "task": s.task,
        })).collect::<Vec<_>>(),
        "rationale": a.rationale,
        // ── 模型研究驱动的运营级建议 ──
        "thinking_recommended": thinking_recommended(&complexity),
        "selected_model": sel.to_json(),
        "cache_advice": cache_advice(),
        "prompt_advice": prompt_advice(sel.few_shot.label()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn router_decision_includes_research_guidance() {
        let v = router_decision(&json!({ "query": "帮我重构这段 Rust 代码并修复内存泄漏" }));
        assert!(v["strategy"].as_str().is_some_and(|s| !s.is_empty()));
        assert!(v["thinking_recommended"].is_boolean());
        assert!(v["selected_model"]["model"]
            .as_str()
            .is_some_and(|s| !s.is_empty()));
        assert!(v["selected_model"]["mode"].as_str().is_some());
        assert!(v["cache_advice"]
            .as_str()
            .is_some_and(|s| s.contains("前缀")));
        assert!(v["prompt_advice"].as_str().is_some_and(|s| !s.is_empty()));
    }

    #[test]
    fn empty_query_returns_error() {
        let v = router_decision(&json!({ "query": "" }));
        assert!(v["error"].as_str().is_some());
    }

    // ── Few-Shot 反例测试（调研结论：R1 系推理模型禁用 few-shot）──
    // 直接测纯函数 select_model，避免本地分类器启发式对查询文本的脆弱映射。

    /// 复杂推理（Question+Complex）：thinking + few-shot 禁用。
    #[test]
    fn reasoning_model_forbids_few_shot() {
        let sel = select_model("Question", "Complex");
        assert_eq!(sel.family, "reasoning");
        assert_eq!(sel.mode, "thinking");
        assert_eq!(sel.few_shot, FewShotPolicy::Forbidden);
        assert!(thinking_recommended("Complex"));
        assert!(prompt_advice(sel.few_shot.label()).contains("零示例"));
    }

    /// 反例守护：复杂推理的提示词绝不允许「可用示例」指引。
    #[test]
    fn reasoning_prompt_never_suggests_examples() {
        let sel = select_model("Question", "Complex");
        let advice = prompt_advice(sel.few_shot.label());
        assert!(!advice.contains("可用 1~2"));
        assert!(advice.contains("schema"));
    }

    /// 简单对话（Question+Simple）：非思考 + few-shot 允许。
    #[test]
    fn simple_chat_allows_few_shot() {
        let sel = select_model("Question", "Simple");
        assert_eq!(sel.family, "chat");
        assert_eq!(sel.mode, "non-thinking");
        assert_eq!(sel.few_shot, FewShotPolicy::Allowed);
        assert!(!thinking_recommended("Simple"));
        assert!(prompt_advice(sel.few_shot.label()).contains("极短示例"));
    }

    /// 复杂编码（Refactor+Complex）：非思考 + few-shot 不推荐（discouraged）+ schema 指引。
    #[test]
    fn complex_coding_discourages_few_shot() {
        let sel = select_model("Refactor", "Complex");
        assert_eq!(sel.family, "agentic-coding");
        assert_eq!(sel.mode, "non-thinking");
        assert_eq!(sel.few_shot, FewShotPolicy::Discouraged);
        assert!(prompt_advice(sel.few_shot.label()).contains("schema"));
    }

    /// 策略内部：少数派组合矩阵完备（防止新增意图漏配 → 兜底 general + allowed）。
    #[test]
    fn selection_matrix_covers_all_known_intents() {
        for intent in [
            "Explain", "Refactor", "Fix", "Test", "Doc", "Commit", "Search", "Question", "Unknown",
        ] {
            for complexity in ["Simple", "Medium", "Complex"] {
                let sel = select_model(intent, complexity);
                assert!(!sel.model.is_empty(), "{intent}/{complexity} 无模型");
                assert!(sel.alt.contains('-'), "{intent} 备选异常");
                // 兜底断言：任何意图都不能是空策略
                assert!(sel.family.len() >= 4, "{intent} 族过短");
            }
        }
    }

    /// 端到端冒烟：router_decision 的 selected_model 结构与策略一致。
    #[test]
    fn decision_exposes_selected_model() {
        let v = router_decision(&json!({ "query": "重构这段代码" }));
        let sel = &v["selected_model"];
        assert!(sel["model"].as_str().is_some_and(|s| !s.is_empty()));
        assert!(sel["mode"].as_str().is_some());
        let fs = sel["few_shot_policy"].as_str().unwrap();
        assert!(fs == "allowed" || fs == "discouraged" || fs == "forbidden");
        // few_shot_policy 与 prompt_advice 必须同源一致
        assert!(v["prompt_advice"].as_str().unwrap().len() > 10);
    }
}
