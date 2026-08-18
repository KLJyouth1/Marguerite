//! 双模型协作
//!
//! 规划模型（强推理）与执行模型（轻量）分工：
//! 1. 规划模型生成结构化计划（Plan）
//! 2. 执行模型逐步执行，异常回传重新规划
//! 3. 用户透明：一站式入口 `run`
//!
//! 借鉴 NexusStar dual_model 思路，但更轻量：
//! 规划结果用结构化文本协议（无需 JSON schema 依赖）。

use anyhow::{anyhow, Result};
use chrono::{DateTime, Utc};

use crate::model::llm::{LLMClient, LLMResponse};
use crate::types::Message;

/// 计划步骤状态
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StepStatus {
    Pending,
    Running,
    Done,
    Failed(String),
}

/// 计划步骤
#[derive(Debug, Clone)]
pub struct PlanStep {
    /// 步骤 ID
    pub id: usize,
    /// 步骤描述
    pub description: String,
    /// 状态
    pub status: StepStatus,
    /// 执行结果
    pub result: Option<String>,
}

/// 执行计划
#[derive(Debug, Clone)]
pub struct Plan {
    /// 计划标题
    pub title: String,
    /// 步骤列表
    pub steps: Vec<PlanStep>,
    /// 创建时间
    pub created_at: DateTime<Utc>,
    /// 计划来源模型
    pub planner_model: String,
}

impl Plan {
    /// 是否全部完成
    pub fn is_complete(&self) -> bool {
        self.steps.iter().all(|s| s.status == StepStatus::Done)
    }

    /// 是否有失败步骤
    pub fn has_failure(&self) -> bool {
        self.steps
            .iter()
            .any(|s| matches!(s.status, StepStatus::Failed(_)))
    }

    /// 未完成步骤数
    pub fn pending_count(&self) -> usize {
        self.steps
            .iter()
            .filter(|s| s.status == StepStatus::Pending || s.status == StepStatus::Running)
            .count()
    }
}

/// 双模型协调器
///
/// planner 负责推理与规划，executor 负责快速执行。
/// 执行失败时自动回传 planner 重新规划（有限次）。
pub struct DualModelCoordinator {
    /// 规划模型（强推理）
    planner: LLMClient,
    /// 执行模型（轻量）
    executor: LLMClient,
    /// 最大重规划次数
    max_replan: usize,
}

impl DualModelCoordinator {
    pub fn new(planner: LLMClient, executor: LLMClient) -> Self {
        Self {
            planner,
            executor,
            max_replan: 2,
        }
    }

    /// 规划模型客户端引用
    pub fn planner(&self) -> &LLMClient {
        &self.planner
    }

    /// 执行模型客户端引用
    pub fn executor(&self) -> &LLMClient {
        &self.executor
    }

    /// 一站式执行：规划 → 执行 → 异常回传
    pub async fn run(&self, task: &str) -> Result<ExecutionReport> {
        let mut plan = self.plan(task).await?;
        let mut replans = 0;

        loop {
            let result = self.execute(&mut plan).await?;
            if !result.failed || replans >= self.max_replan {
                return Ok(result);
            }
            // 异常回传：将失败信息反馈给规划模型重新规划
            plan = self.replan(&plan, &result).await?;
            replans += 1;
        }
    }

    /// 生成计划（强推理模型）
    pub async fn plan(&self, task: &str) -> Result<Plan> {
        let resp = self.planner.chat_complete(&self.plan_prompt(task)).await?;
        parse_plan(&resp, &self.planner_model_name())
    }

    /// 重新规划（基于失败反馈）
    async fn replan(&self, prev: &Plan, report: &ExecutionReport) -> Result<Plan> {
        let failures: Vec<String> = report
            .failures
            .iter()
            .map(|(id, msg)| format!("步骤 {id}: {msg}"))
            .collect();
        let prompt = format!(
            "原计划《{}》执行失败，请重新规划剩余步骤。\n失败原因：\n{}\n只输出新的步骤列表。",
            prev.title,
            failures.join("\n")
        );
        let resp = self
            .planner
            .chat_complete(&self.plan_prompt(&prompt))
            .await?;
        parse_plan(&resp, &self.planner_model_name())
    }

    /// 逐步执行（轻量模型）
    ///
    /// 顺序执行未完成步骤，失败即停止并回传。
    pub async fn execute(&self, plan: &mut Plan) -> Result<ExecutionReport> {
        let mut report = ExecutionReport::default();
        for step in plan.steps.iter_mut() {
            if step.status == StepStatus::Done {
                continue;
            }
            step.status = StepStatus::Running;
            let prompt = format!("执行以下任务步骤，直接给出结果：\n{}", step.description);
            match self
                .executor
                .chat_complete(&[Message::User {
                    content: prompt,
                    timestamp: Utc::now(),
                }])
                .await
            {
                Ok(resp) => {
                    step.result = Some(resp.content.clone());
                    step.status = StepStatus::Done;
                    report.successes += 1;
                }
                Err(e) => {
                    let msg = e.to_string();
                    step.status = StepStatus::Failed(msg.clone());
                    report.failures.push((step.id, msg));
                    report.failed = true;
                    break;
                }
            }
        }
        Ok(report)
    }

    /// 规划提示词
    fn plan_prompt(&self, task: &str) -> Vec<Message> {
        let now = Utc::now().to_rfc3339();
        vec![
            Message::System {
                content: "你是规划模型。将任务分解为 1-8 个可执行步骤，每步一行，格式：`N. 描述`。只输出步骤列表。".to_string(),
                timestamp: Utc::now(),
            },
            Message::User {
                content: format!("[{now}]\n{task}"),
                timestamp: Utc::now(),
            },
        ]
    }

    fn planner_model_name(&self) -> String {
        "planner".to_string()
    }
}

/// 执行报告
#[derive(Debug, Clone, Default)]
pub struct ExecutionReport {
    /// 成功步骤数
    pub successes: usize,
    /// 失败步骤（id, 原因）
    pub failures: Vec<(usize, String)>,
    /// 是否有失败
    pub failed: bool,
}

/// 解析模型输出的计划
///
/// 兼容格式：
/// - `1. 描述` / `1. 描述`
/// - 带 markdown 列表 `- 1. xxx`
pub fn parse_plan(resp: &LLMResponse, model: &str) -> Result<Plan> {
    let mut steps = Vec::new();
    for (idx, line) in resp.content.lines().enumerate() {
        let line = line
            .trim()
            .trim_start_matches('-')
            .trim()
            .trim_start_matches('*')
            .trim();
        if line.is_empty() {
            continue;
        }
        // 匹配 `数字. 描述`
        let desc = if let Some(pos) = line.find(". ") {
            let head = &line[..pos];
            if head.chars().all(|c| c.is_ascii_digit()) {
                line[pos + 2..].trim().to_string()
            } else {
                continue;
            }
        } else {
            continue;
        };
        if desc.is_empty() {
            continue;
        }
        steps.push(PlanStep {
            id: idx,
            description: desc,
            status: StepStatus::Pending,
            result: None,
        });
    }
    if steps.is_empty() {
        return Err(anyhow!("规划模型未返回有效步骤"));
    }
    Ok(Plan {
        title: format!("任务计划（{} 步）", steps.len()),
        steps,
        created_at: Utc::now(),
        planner_model: model.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::llm::TokenUsage;

    fn fake_response(content: &str) -> LLMResponse {
        LLMResponse {
            content: content.to_string(),
            thinking: None,
            model: "planner".to_string(),
            usage: TokenUsage::default(),
            tool_calls: Vec::new(),
        }
    }

    #[test]
    fn test_parse_plan() {
        let resp = fake_response("1. 分析需求\n2. 编写代码\n3. 测试验证\n");
        let plan = parse_plan(&resp, "planner").unwrap();
        assert_eq!(plan.steps.len(), 3);
        assert_eq!(plan.steps[0].description, "分析需求");
        assert!(plan.pending_count() == 3);
        assert!(!plan.is_complete());
    }

    #[test]
    fn test_parse_plan_markdown() {
        let resp = fake_response("- 1. 步骤一\n- 2. 步骤二\n");
        let plan = parse_plan(&resp, "planner").unwrap();
        assert_eq!(plan.steps.len(), 2);
    }

    #[test]
    fn test_parse_plan_empty_fails() {
        let resp = fake_response("无法规划");
        assert!(parse_plan(&resp, "planner").is_err());
    }

    #[test]
    fn test_plan_status_transitions() {
        let mut plan = Plan {
            title: "t".to_string(),
            steps: vec![PlanStep {
                id: 0,
                description: "s".to_string(),
                status: StepStatus::Pending,
                result: None,
            }],
            created_at: Utc::now(),
            planner_model: "m".to_string(),
        };
        assert!(!plan.is_complete());
        plan.steps[0].status = StepStatus::Done;
        assert!(plan.is_complete());
        plan.steps[0].status = StepStatus::Failed("x".to_string());
        assert!(plan.has_failure());
    }

    #[test]
    fn test_plan_pending_count() {
        let plan = Plan {
            title: "t".to_string(),
            steps: vec![
                PlanStep {
                    id: 0,
                    description: "a".to_string(),
                    status: StepStatus::Done,
                    result: None,
                },
                PlanStep {
                    id: 1,
                    description: "b".to_string(),
                    status: StepStatus::Running,
                    result: None,
                },
                PlanStep {
                    id: 2,
                    description: "c".to_string(),
                    status: StepStatus::Pending,
                    result: None,
                },
                PlanStep {
                    id: 3,
                    description: "d".to_string(),
                    status: StepStatus::Failed("x".to_string()),
                    result: None,
                },
            ],
            created_at: Utc::now(),
            planner_model: "m".to_string(),
        };
        assert_eq!(plan.pending_count(), 2);
    }

    #[test]
    fn test_parse_plan_skips_non_step_lines() {
        let resp = fake_response("开始分析任务\n1. 步骤一\n（中间说明）\n2. 步骤二\n* 3. 步骤三\n");
        let plan = parse_plan(&resp, "planner").unwrap();
        assert_eq!(plan.steps.len(), 3);
        assert_eq!(plan.steps[0].description, "步骤一");
        assert_eq!(plan.planner_model, "planner");
        assert!(!plan.title.is_empty());
    }

    #[test]
    fn test_parse_plan_empty_description_skipped() {
        let resp = fake_response("1. \n2. 有效步骤\n");
        let plan = parse_plan(&resp, "planner").unwrap();
        assert_eq!(plan.steps.len(), 1);
        assert_eq!(plan.steps[0].description, "有效步骤");
    }

    #[test]
    fn test_plan_prompt_shape() {
        let coordinator = DualModelCoordinator::new(
            LLMClient::new(Default::default()),
            LLMClient::new(Default::default()),
        );
        let messages = coordinator.plan_prompt("写一个排序函数");
        assert_eq!(messages.len(), 2);
        assert!(matches!(messages[0], Message::System { .. }));
        assert!(matches!(messages[1], Message::User { .. }));
        assert_eq!(coordinator.planner_model_name(), "planner");
    }
}
