//! 声明式工作流编排（对齐 DSH `dsh-tool-workflow` 的 pipeline/parallel 语义）。
//!
//! Rust 声明式形态：工作流以 JSON 规格描述（[`WorkflowSpec`]），由 [`WorkflowEngine`]
//! 驱动 [`SubAgentManager`] 逐阶段执行——**阶段内 fan-out 并行、阶段间屏障**。
//! 对齐上游语义：
//! - `pipeline`（无阶段屏障）：本模块以「阶段内全量扇出 + 阶段间 `wait_all` 屏障」实现，
//!   每阶段等价于一次并行 fan-out，全部 settle 后才进下一阶段（对应上游 phase 进度）。
//! - 单条 item 失败（spawn 失败或结果缺失）→ 该条落为 `None`，**不拖垮整轮**，
//!   对应上游「普通 stage throw 只 drop 该 item」。
//!
//! 不依赖 LLM 做分解（离线可测、确定性）：子代理执行本身走真实模型路由与工具链，
//! 无模型时子代理失败 → 报告仍正常收尾返回。

pub mod ralph;

use serde::{Deserialize, Serialize};

use crate::agent::subagent::SubAgentManager;
use crate::model::router::{Complexity, IntentType};

/// 工作流阶段：一个阶段内扇出若干子任务提示词（`items`）并行执行。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkflowPhase {
    /// 阶段标题（人类可读，用于报告与事件追踪）。
    pub title: String,
    /// 可选阶段说明（注入每条子任务的上文）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// 该阶段扇出的子任务提示词列表。
    pub items: Vec<String>,
}

/// 工作流规格（声明式 JSON 形态）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkflowSpec {
    /// 工作流名（非空）。
    pub name: String,
    /// 一句话描述（可选）。
    #[serde(default)]
    pub description: String,
    /// 阶段列表（非空）。
    pub phases: Vec<WorkflowPhase>,
}

impl WorkflowSpec {
    /// 从 JSON 解析并做结构校验。
    ///
    /// 校验规则（对齐上游 JSON 校验）：`name` 非空、`phases` 非空、每阶段 `title` 非空。
    pub fn from_json(s: &str) -> Result<Self, String> {
        let spec: WorkflowSpec =
            serde_json::from_str(s).map_err(|e| format!("工作流 JSON 解析失败: {e}"))?;
        spec.validate()?;
        Ok(spec)
    }

    /// 结构校验：`name` 非空、`phases` 非空、每阶段 `title` 非空。
    pub fn validate(&self) -> Result<(), String> {
        if self.name.trim().is_empty() {
            return Err("工作流 name 不能为空".to_string());
        }
        if self.phases.is_empty() {
            return Err("工作流 phases 不能为空".to_string());
        }
        for (i, phase) in self.phases.iter().enumerate() {
            if phase.title.trim().is_empty() {
                return Err(format!("工作流第 {} 个阶段的 title 不能为空", i + 1));
            }
        }
        Ok(())
    }
}

/// 单阶段执行结果。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PhaseResult {
    /// 阶段标题（与 [`WorkflowPhase::title`] 对齐）。
    pub title: String,
    /// 每条 item 的结果：`Some(产出)` 或 `None`（该条失败/无结果）。
    /// 长度恒等于对应 [`WorkflowPhase::items`] 的长度。
    pub item_results: Vec<Option<String>>,
}

/// 工作流整体报告。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkflowReport {
    /// 工作流名（与 [`WorkflowSpec::name`] 对齐）。
    pub name: String,
    /// 各阶段结果（与 `phases` 一一对应）。
    pub phase_results: Vec<PhaseResult>,
}

/// 纯函数：把工作流规格展开为 `(phase_index, item_index)` 的扇出计划。
///
/// 阶段内按 items 顺序、阶段间按 phases 顺序，产出稳定有序的全量扇出点。
/// 用于离线可测地验证「阶段内 fan-out、阶段间屏障」的调度形状。
pub fn fanout_plan(spec: &WorkflowSpec) -> Vec<(usize, usize)> {
    let mut plan = Vec::new();
    for (pi, phase) in spec.phases.iter().enumerate() {
        for (ii, _item) in phase.items.iter().enumerate() {
            plan.push((pi, ii));
        }
    }
    plan
}

/// 工作流引擎：声明式规格 → 逐阶段真实执行。
#[derive(Debug, Clone, Default)]
pub struct WorkflowEngine;

impl WorkflowEngine {
    /// 构造引擎（无内部状态，纯调度器）。
    pub fn new() -> Self {
        Self
    }

    /// 按规格逐阶段执行工作流。
    ///
    /// - 阶段内 fan-out：逐条 [`SubAgentManager::spawn_with_spec`] 并行发起（真实执行由
    ///   manager 的并发闸门 `max_concurrent` 限制），spawn 立即返回、执行在后台并发进行；
    /// - 阶段间屏障：本阶段全部 settle（[`SubAgentManager::wait_all`]）后才进下一阶段；
    /// - 收拢：按 items 顺序对齐结果，单条失败/无结果 → `None`，不中断整轮。
    ///
    /// 离线/无模型时子代理会失败，但引擎仍正常收尾并返回结构完整的 [`WorkflowReport`]。
    pub async fn run(
        &self,
        spec: &WorkflowSpec,
        manager: &SubAgentManager,
        workspace_hint: &str,
    ) -> WorkflowReport {
        let mut phase_results = Vec::with_capacity(spec.phases.len());

        for phase in &spec.phases {
            // 阶段内 fan-out：并行发起全部 items。
            // `(agent_id, spawned)`：spawn 失败记为 `(空, false)`，收拢时落 `None`。
            let mut ids: Vec<(String, bool)> = Vec::with_capacity(phase.items.len());
            for item in &phase.items {
                let task = match &phase.detail {
                    Some(d) => format!("{}\n\n【阶段说明】{}\n【工作区】{}", item, d, workspace_hint),
                    None => format!("{}\n\n【工作区】{}", item, workspace_hint),
                };
                let role = format!("工作流「{}」阶段执行子代理", phase.title);
                match manager
                    .spawn_with_spec(
                        &task,
                        workspace_hint,
                        &role,
                        IntentType::General,
                        Complexity::Medium,
                    )
                    .await
                {
                    Ok(id) => ids.push((id, true)),
                    Err(_) => ids.push((String::new(), false)),
                }
            }

            // 阶段间屏障：等待本阶段全部 settle。
            let _ = manager.wait_all().await;

            // 收拢结果（按 items 顺序对齐）。
            let mut item_results = Vec::with_capacity(phase.items.len());
            for (id, spawned) in ids {
                if !spawned {
                    item_results.push(None);
                    continue;
                }
                match manager.get_result(&id).await {
                    Some(r) => item_results.push(Some(r)),
                    None => item_results.push(None),
                }
            }

            phase_results.push(PhaseResult {
                title: phase.title.clone(),
                item_results,
            });
        }

        WorkflowReport {
            name: spec.name.clone(),
            phase_results,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use crate::agent::subagent::SubAgentConfig;
    use crate::mcp::registry::ToolRegistry;
    use crate::model::router::ModelRouter;
    use crate::persistence::config::Config;

    /// 离线子 Agent 管理器：模型不可达（base_url 指向拒绝连接的本地端口），
    /// spawn/wait/收集链路仍真实跑通——证明工作流引擎不依赖 LLM。
    fn offline_subagents() -> SubAgentManager {
        let cfg = Config {
            model: crate::persistence::config::ModelConfig {
                provider: "custom".to_string(),
                model_name: "offline-model".to_string(),
                api_key: "offline".to_string(),
                base_url: "http://127.0.0.1:1".to_string(),
                copilot_model: None,
            },
            ..Default::default()
        };
        let router = Arc::new(ModelRouter::new(&cfg));
        let reg = Arc::new(ToolRegistry::new());
        SubAgentManager::new(SubAgentConfig::default(), reg, router)
    }

    fn sample_spec() -> WorkflowSpec {
        WorkflowSpec {
            name: "demo".to_string(),
            description: "示例工作流".to_string(),
            phases: vec![
                WorkflowPhase {
                    title: "调研".to_string(),
                    detail: None,
                    items: vec!["调研要点 a".to_string(), "调研要点 b".to_string()],
                },
                WorkflowPhase {
                    title: "实现".to_string(),
                    detail: Some("编码实现".to_string()),
                    items: vec!["实现 c".to_string()],
                },
            ],
        }
    }

    #[test]
    fn spec_from_json_parses_and_rejects_bad() {
        // 合法解析（description 缺省、detail 缺省）。
        let ok = r#"{"name":"demo","phases":[{"title":"调研","items":["a","b"]}]}"#;
        let spec = WorkflowSpec::from_json(ok).expect("合法 JSON 应解析");
        assert_eq!(spec.name, "demo");
        assert_eq!(spec.phases.len(), 1);
        assert_eq!(spec.phases[0].items.len(), 2);
        assert_eq!(spec.phases[0].detail, None);

        // 缺 name。
        assert!(WorkflowSpec::from_json(r#"{"phases":[{"title":"t","items":[]}]}"#).is_err());
        // 缺 phases。
        assert!(WorkflowSpec::from_json(r#"{"name":"n"}"#).is_err());
        // 空 title。
        assert!(
            WorkflowSpec::from_json(r#"{"name":"n","phases":[{"title":"","items":["x"]}]}"#)
                .is_err()
        );
        // 非法 JSON。
        assert!(WorkflowSpec::from_json("not-json").is_err());
    }

    #[test]
    fn fanout_plan_aligns_with_spec() {
        let plan = fanout_plan(&sample_spec());
        // 阶段 0 两条 + 阶段 1 一条，按 (phase_index, item_index) 稳定展开。
        assert_eq!(plan, vec![(0, 0), (0, 1), (1, 0)]);
    }

    #[test]
    fn fanout_plan_empty_items_yields_no_entries() {
        let spec = WorkflowSpec {
            name: "empty".to_string(),
            description: String::new(),
            phases: vec![WorkflowPhase {
                title: "无 items".to_string(),
                detail: None,
                items: vec![],
            }],
        };
        assert!(fanout_plan(&spec).is_empty());
    }

    #[tokio::test]
    async fn engine_run_offline_reports_shape() {
        // 离线下子代理执行会失败，但引擎必须正常收尾：phase_results 与 phases 一一对应、
        // item_results 长度与 items 一致。
        let spec = sample_spec();
        let manager = offline_subagents();
        let report = WorkflowEngine::new()
            .run(&spec, &manager, "ws-offline")
            .await;

        assert_eq!(report.name, "demo");
        assert_eq!(
            report.phase_results.len(),
            2,
            "phase_results 应与 phases 一一对应"
        );
        assert_eq!(
            report.phase_results[0].item_results.len(),
            2,
            "item_results 长度应与 items 一致"
        );
        assert_eq!(report.phase_results[1].item_results.len(), 1);
        assert_eq!(report.phase_results[0].title, "调研");
        assert_eq!(report.phase_results[1].title, "实现");
    }
}
