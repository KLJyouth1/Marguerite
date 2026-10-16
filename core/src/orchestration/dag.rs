//! 任务 DAG 编排内核：让子代理「复杂任务自己拆、自己干」。
//!
//! 对齐参考实现（Nexustar HierarchicalOrchestrator / GoalExecutionEngine）的
//! 三层结构，但**全部真实执行**：
//! 1. 模板分解：按任务形态生成带依赖的任务 DAG（research → plan → draft → review）；
//! 2. Kahn 分层：拓扑排序 + 稳定排序（priority 降序、id 升序）+ 环拍平兜底；
//! 3. 逐层执行：同一层内子代理并行真实运行（SubAgentManager::spawn_with_spec），
//!    层间依赖保证——下游任务注入上游全部产出后执行；
//! 4. 结果聚合：按层序拼接各子代理结论，供 lead 综合。
//!
//! 不依赖 LLM 做分解（离线可测、确定性），子代理执行本身走真实模型路由与工具链。

use std::collections::{HashMap, HashSet};

use serde_json::json;

use crate::agent::subagent::SubAgentManager;
use crate::events::{Envelope, EventBus, EventType};
use crate::model::router::{Complexity, IntentType};

/// 任务节点：一个待执行的角色子任务。
#[derive(Debug, Clone)]
pub struct TaskNode {
    /// 稳定节点 id（`t1`..`tN`），供依赖与事件追踪。
    pub id: String,
    /// 角色标签（人类可读，用于综合与事件）。
    pub role: &'static str,
    /// 该节点执行的目标描述。
    pub goal: String,
    /// 依赖的节点 id 列表（全部完成后本节点才可执行）。
    pub deps: Vec<String>,
    /// 层内调度优先级（数值大者优先）。
    pub priority: i32,
    /// 模型路由意图。
    pub intent: IntentType,
    /// 模型路由复杂度。
    pub complexity: Complexity,
}

impl TaskNode {
    pub fn new(
        id: &str,
        role: &'static str,
        goal: String,
        deps: Vec<String>,
        priority: i32,
        intent: IntentType,
        complexity: Complexity,
    ) -> Self {
        Self {
            id: id.to_string(),
            role,
            goal,
            deps,
            priority,
            intent,
            complexity,
        }
    }
}

/// 任务形态（决定模板分解形状）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DagKind {
    /// 完整交付：调研 → 规划 → 起草 → 审校（4 节点，经典构建闭环）。
    Build,
    /// 快速产出：规划 → 起草 → 审校（无调研，适合已有上下文）。
    Draft,
    /// 文档优先：规划 → 起草 → 审校，起草角色为文档专家。
    Document,
    /// 智能体分阶段（分阶段智能体 对齐）：调研 → 规划 → 实现 → 反思自检(s2_5) → 审校。
    /// 在「实现」与「审校」之间插入显式反思阶段：先对上游工作记忆做 go/no-go 自检，
    /// 再决定是否进入最终审校/交付。对齐 分阶段智能体 的 s1 感知 → s2 工作记忆 → s2_5 反思 → s3 行动。
    Reflect,
    /// 大规模 fan-out（「上千 subagent」）：编码实现节点按「枚举条目（文件/模块列表）」
    /// 或 `fanout` 分片数拆为 N 个并行子节点（多文件/多模块并行），上游调研/规划产出
    /// 注入每个实现节点，最后统一审校。
    FanOut { fanout: usize },
}

/// 模板分解：把目标拆成带依赖的角色任务 DAG。
///
/// 分解是确定性的（不调 LLM），保证离线可测与结果稳定；
/// 「自己拆」的灵活性由角色目标注入目标文本体现。
pub fn decompose(goal: &str, kind: DagKind) -> Vec<TaskNode> {
    let mut nodes = Vec::new();
    match kind {
        DagKind::Build => {
            nodes.push(TaskNode::new(
                "t1",
                "调研",
                format!("调研主题要点与现状，输出有据可循的结论：{goal}"),
                vec![],
                7,
                IntentType::Chat,
                Complexity::Medium,
            ));
            nodes.push(TaskNode::new(
                "t2",
                "架构设计",
                format!("规划执行步骤、接口契约与依赖，输出清晰方案：{goal}"),
                vec![],
                8,
                IntentType::Reasoning,
                Complexity::Complex,
            ));
            nodes.push(TaskNode::new(
                "t3",
                "编码实现",
                format!("综合调研与规划产出，完成具体实现并交付可运行结果：{goal}"),
                vec!["t1".to_string(), "t2".to_string()],
                9,
                IntentType::Code,
                Complexity::Complex,
            ));
            nodes.push(TaskNode::new(
                "t4",
                "测试验证",
                format!("审校产出质量与边界覆盖，指出风险并给出修复建议：{goal}"),
                vec!["t3".to_string()],
                6,
                IntentType::Code,
                Complexity::Medium,
            ));
        }
        DagKind::Draft => {
            nodes.push(TaskNode::new(
                "t1",
                "架构设计",
                format!("规划执行步骤与验收标准：{goal}"),
                vec![],
                8,
                IntentType::Reasoning,
                Complexity::Medium,
            ));
            nodes.push(TaskNode::new(
                "t2",
                "编码实现",
                format!("依据规划完成实现并交付：{goal}"),
                vec!["t1".to_string()],
                9,
                IntentType::Code,
                Complexity::Medium,
            ));
            nodes.push(TaskNode::new(
                "t3",
                "测试验证",
                format!("审校实现并给出质量结论：{goal}"),
                vec!["t2".to_string()],
                6,
                IntentType::Code,
                Complexity::Simple,
            ));
        }
        DagKind::Document => {
            nodes.push(TaskNode::new(
                "t1",
                "架构设计",
                format!("梳理内容结构与要点：{goal}"),
                vec![],
                8,
                IntentType::Reasoning,
                Complexity::Medium,
            ));
            nodes.push(TaskNode::new(
                "t2",
                "文档",
                format!("依据结构产出完整文档：{goal}"),
                vec!["t1".to_string()],
                9,
                IntentType::Summarization,
                Complexity::Medium,
            ));
            nodes.push(TaskNode::new(
                "t3",
                "测试验证",
                format!("审校文档准确性与完整性：{goal}"),
                vec!["t2".to_string()],
                6,
                IntentType::Chat,
                Complexity::Simple,
            ));
        }
        DagKind::Reflect => {
            nodes.push(TaskNode::new(
                "t1",
                "调研",
                format!("调研主题要点与现状，输出有据可循的结论：{goal}"),
                vec![],
                7,
                IntentType::Chat,
                Complexity::Medium,
            ));
            nodes.push(TaskNode::new(
                "t2",
                "架构设计",
                format!("规划执行步骤、接口契约与依赖，输出清晰方案：{goal}"),
                vec![],
                8,
                IntentType::Reasoning,
                Complexity::Complex,
            ));
            nodes.push(TaskNode::new(
                "t3",
                "编码实现",
                format!("综合调研与规划产出，完成具体实现并交付可运行结果：{goal}"),
                vec!["t1".to_string(), "t2".to_string()],
                9,
                IntentType::Code,
                Complexity::Complex,
            ));
            nodes.push(TaskNode::new(
                "t_reflect",
                "反思自检",
                format!("阶段反思（s2_5）：复核调研/规划/实现的工作记忆是否自洽、有无缺失或冲突，给出 go/no-go 与待补项：{goal}"),
                vec!["t3".to_string()],
                7,
                IntentType::Reasoning,
                Complexity::Medium,
            ));
            nodes.push(TaskNode::new(
                "t4",
                "测试验证",
                format!("审校产出质量与边界覆盖，指出风险并给出修复建议：{goal}"),
                vec!["t_reflect".to_string()],
                6,
                IntentType::Code,
                Complexity::Medium,
            ));
        }
        DagKind::FanOut { fanout } => {
            let n = fanout.max(1);
            // 调研 + 架构设计 先行并行
            nodes.push(TaskNode::new(
                "t1",
                "调研",
                format!("调研主题要点与现状，输出有据可循的结论：{goal}"),
                vec![],
                7,
                IntentType::Chat,
                Complexity::Medium,
            ));
            nodes.push(TaskNode::new(
                "t2",
                "架构设计",
                format!("规划执行步骤、接口契约与依赖，明确模块/文件拆分：{goal}"),
                vec![],
                8,
                IntentType::Reasoning,
                Complexity::Complex,
            ));
            // 编码实现 fan-out：优先用目标里的枚举条目（文件/模块列表），否则按 fanout 分片。
            let items = crate::orchestration::extract_enumerated(goal);
            let shards: Vec<String> = if items.len() >= 2 {
                items
            } else {
                (1..=n).map(|i| format!("第 {i}/{n} 部分")).collect()
            };
            let impl_ids: Vec<String> =
                (0..shards.len()).map(|i| format!("t3_{}", i + 1)).collect();
            for (i, shard) in shards.iter().enumerate() {
                nodes.push(TaskNode::new(
                    &impl_ids[i],
                    "编码实现",
                    format!("综合调研与规划产出，实现「{shard}」并交付可运行结果：{goal}"),
                    vec!["t1".to_string(), "t2".to_string()],
                    9,
                    IntentType::Code,
                    Complexity::Complex,
                ));
            }
            // 测试验证：依赖全部编码实现节点
            nodes.push(TaskNode::new(
                "t4",
                "测试验证",
                format!("审校全部实现产出的质量与边界覆盖，指出风险并给出修复建议：{goal}"),
                impl_ids,
                6,
                IntentType::Code,
                Complexity::Medium,
            ));
        }
    }
    nodes
}

/// 分阶段智能体的 s2_5 反思自检结果（对齐 分阶段智能体 的「先反思工作记忆再行动」）。
///
/// 纯确定性、离线可测：不调 LLM，仅依据各阶段产出文本判定工作记忆是否可信。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reflection {
    /// 是否可继续交付（工作记忆无失败/空阶段）。
    pub go: bool,
    /// 失败或空阶段标签（按出现顺序）。
    pub failed_stages: Vec<String>,
    /// 人类可读的逐阶段自检笔记。
    pub notes: Vec<String>,
}

/// 对各阶段产出（工作记忆）做反思自检。
///
/// 判定规则（确定性）：
/// - 产出为空或纯空白 → 视为缺失；
/// - 产出是执行器写下的**失败占位**（见 [`is_failed_stage_output`]）→ 视为失败；
/// - 其余视为有效产出。
/// - `go = failed_stages.is_empty()`。
///
/// 对应 分阶段智能体 的 s2_5：在推进入行动（审校/交付）前，先对积累的工作记忆做
/// go/no-go 自检，避免基于残缺记忆给出不可信结论。
pub fn reflect_pass(summaries: &[String]) -> Reflection {
    let mut failed_stages: Vec<String> = Vec::new();
    let mut notes: Vec<String> = Vec::new();
    for (i, s) in summaries.iter().enumerate() {
        let label = format!("阶段 {}", i + 1);
        let trimmed = s.trim();
        if is_failed_stage_output(trimmed) {
            failed_stages.push(label.clone());
            notes.push(format!("{label}: 缺失/失败（工作记忆不可信）"));
        } else {
            notes.push(format!(
                "{label}: 有效产出（{} 字符）",
                trimmed.chars().count()
            ));
        }
    }
    let go = failed_stages.is_empty();
    Reflection {
        go,
        failed_stages,
        notes,
    }
}

/// 判定一段阶段产出是否为「失败占位」（工作记忆不可信）。
///
/// 覆盖所有真实执行器产出的失败标记（见 `agent::team` / `agent::subagent`）：
/// `[子任务 … 失败/无结果]`、`[子专家执行失败]`、`[子专家执行超时]`、
/// `[子专家未完成]`、`[子专家无结果返回]`、`[成员 spawn 失败…]`、`[依赖环…]`。
///
/// ⚠️ 历史缺陷：原实现只认 `[子任务` 一种前缀，而真实执行器发的是
/// `[子专家执行失败]` / `[成员 spawn 失败]` / `[依赖环…]`，于是反思门对失败的
/// 阶段照样判 `go=true` —— 上游注释里写着的「fail-closed 交付门」形同虚设，
/// 失败产出被当成有效结论交付。此函数与执行器的标记集合必须保持同步。
fn is_failed_stage_output(text: &str) -> bool {
    if text.is_empty() {
        return true;
    }
    const MARKERS: &[&str] = &[
        "[子专家执行失败]",
        "[子专家执行超时]",
        "[子专家未完成]",
        "[子专家无结果返回]",
        "[成员 spawn 失败",
        "[依赖环",
    ];
    if MARKERS.iter().any(|m| text.contains(m)) {
        return true;
    }
    // 通用兜底：`[子任务 …]` / `[子专家 …]` 里含「失败 / 无结果 / 超时」。
    (text.starts_with("[子任务") || text.starts_with("[子专家"))
        && (text.contains("失败") || text.contains("无结果") || text.contains("超时"))
}

/// Kahn 拓扑分层：返回按依赖约束可分层的节点 id 批次（同批可并行）。
///
/// - 稳定排序：每层按 priority 降序、id 升序（消除集合迭代不确定性）；
/// - 环检测：无入度 0 节点时把剩余节点强制拍平到当前层（不死锁兜底）。
pub fn topo_layers(nodes: &[TaskNode]) -> Vec<Vec<String>> {
    let mut deps: HashMap<String, Vec<String>> = HashMap::new();
    let mut dependents: HashMap<String, Vec<String>> = HashMap::new();
    let mut remaining: HashSet<String> = HashSet::new();
    for n in nodes {
        deps.insert(n.id.clone(), n.deps.clone());
        remaining.insert(n.id.clone());
        for d in &n.deps {
            dependents.entry(d.clone()).or_default().push(n.id.clone());
        }
    }

    let mut layers: Vec<Vec<String>> = Vec::new();
    let mut done: HashSet<String> = HashSet::new();

    while !remaining.is_empty() {
        // 本层可执行：所有依赖已完成（或无依赖）
        let mut ready: Vec<&TaskNode> = nodes
            .iter()
            .filter(|n| remaining.contains(&n.id) && n.deps.iter().all(|d| done.contains(d)))
            .collect();
        if ready.is_empty() {
            // 环兜底：剩余节点强制拍平
            ready = nodes.iter().filter(|n| remaining.contains(&n.id)).collect();
        }
        ready.sort_by(|a, b| b.priority.cmp(&a.priority).then(a.id.cmp(&b.id)));

        let layer_ids: Vec<String> = ready.iter().map(|n| n.id.clone()).collect();
        for id in &layer_ids {
            remaining.remove(id);
            done.insert(id.clone());
        }
        layers.push(layer_ids);
    }
    layers
}

/// 逐层执行 DAG：同一层子代理并行真实运行，下游节点注入上游产出。
///
/// `role_system` 提供角色系统人设（默认使用内置角色表）。
/// 返回 (节点 id → 产出文本)，层序稳定。
///
/// **legacy fallback（回滚锚）**：自 C6 起，`dispatch_team_dag` 已改走与 `dispatch_team`
/// 同源的统一依赖执行体 `build_team` + [`crate::agent::team::Team::run_p2p`]，本函数不再被
/// 调用，仅作为可一键回滚的参考实现保留。
pub async fn run_dag(
    subagents: &SubAgentManager,
    bus: Option<&Arc<EventBus>>,
    nodes: &[TaskNode],
    context: &str,
    role_system: &(dyn Fn(&str) -> &'static str + Sync),
) -> HashMap<String, String> {
    let layers = topo_layers(nodes);
    let mut outputs: HashMap<String, String> = HashMap::new();

    for (li, layer) in layers.iter().enumerate() {
        emit(
            bus,
            format!(
                "编排层级 {}/{}：{} 个子代理并行执行",
                li + 1,
                layers.len(),
                layer.len()
            ),
        );
        for id in layer {
            let Some(node) = nodes.iter().find(|n| n.id == *id) else {
                continue;
            };
            let deps_out = node
                .deps
                .iter()
                .filter_map(|d| outputs.get(d))
                .cloned()
                .collect::<Vec<_>>()
                .join("\n\n");
            let goal = if deps_out.is_empty() {
                node.goal.clone()
            } else {
                format!("{}\n\n【上游产出】\n{}", node.goal, deps_out)
            };
            emit(bus, format!("子代理「{}」开始：{}", node.role, node.goal));
            match subagents
                .spawn_with_spec(
                    &goal,
                    context,
                    role_system(node.role),
                    node.intent.clone(),
                    node.complexity.clone(),
                )
                .await
            {
                Ok(_) => {}
                Err(e) => {
                    emit(bus, format!("子代理「{}」启动失败：{e}", node.role));
                    outputs.insert(node.id.clone(), format!("[子任务 {} 失败: {e}]", node.role));
                }
            }
        }
        // 等待本层全部完成
        let _ = subagents.wait_all().await;
        // 收拢本层结果（collect_results 按完成顺序返回，需按 id 对齐）
        for id in layer {
            if outputs.contains_key(id) {
                continue;
            }
            let node = nodes.iter().find(|n| n.id == *id).unwrap();
            match subagents.get_result(id).await {
                Some(r) => {
                    outputs.insert(id.clone(), r);
                    emit(bus, format!("子代理「{}」完成", node.role));
                }
                None => {
                    outputs.insert(id.clone(), format!("[子任务 {} 无结果]", node.role));
                    emit(bus, format!("子代理「{}」无结果", node.role));
                }
            }
        }
    }
    outputs
}

/// 把节点产出聚合为按层序排列的综合上下文。
pub fn aggregate(nodes: &[TaskNode], outputs: &HashMap<String, String>) -> Vec<String> {
    let layers = topo_layers(nodes);
    let mut parts = Vec::new();
    for layer in layers {
        for id in layer {
            if let Some(node) = nodes.iter().find(|n| n.id == id) {
                let text = outputs.get(&id).cloned().unwrap_or_default();
                parts.push(format!("## {} · {}\n{}", node.role, node.id, text));
            }
        }
    }
    parts
}

fn emit(bus: Option<&Arc<EventBus>>, text: String) {
    if let Some(b) = bus {
        b.publish(Envelope::new(
            EventType::Thinking,
            json!({ "text": format!("> {text}") }),
            "orchestration.dag",
        ));
    }
}

use std::sync::Arc;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_decompose_build_has_dependency_chain() {
        let nodes = decompose("实现登录功能", DagKind::Build);
        assert_eq!(nodes.len(), 4);
        let t3 = nodes.iter().find(|n| n.id == "t3").unwrap();
        assert_eq!(t3.deps, vec!["t1", "t2"]);
        let t4 = nodes.iter().find(|n| n.id == "t4").unwrap();
        assert_eq!(t4.deps, vec!["t3"]);
    }

    #[test]
    fn test_topo_layers_respect_deps() {
        let nodes = decompose("写文档", DagKind::Document);
        let layers = topo_layers(&nodes);
        // 3 层：t1 → t2 → t3
        assert_eq!(layers.len(), 3);
        assert_eq!(layers[0], vec!["t1"]);
        assert_eq!(layers[1], vec!["t2"]);
        assert_eq!(layers[2], vec!["t3"]);
    }

    #[test]
    fn test_topo_layers_parallel_same_layer() {
        // 两个无依赖节点应同层并行
        let nodes = vec![
            TaskNode::new(
                "a",
                "调研",
                "A".into(),
                vec![],
                7,
                IntentType::Chat,
                Complexity::Simple,
            ),
            TaskNode::new(
                "b",
                "编码实现",
                "B".into(),
                vec![],
                9,
                IntentType::Code,
                Complexity::Simple,
            ),
        ];
        let layers = topo_layers(&nodes);
        assert_eq!(layers.len(), 1);
        // 稳定排序：priority 降序 → b 在前
        assert_eq!(layers[0], vec!["b", "a"]);
    }

    #[test]
    fn test_topo_layers_cycle_flattened() {
        // a→b→a 环：不应死锁，应拍平
        let nodes = vec![
            TaskNode::new(
                "a",
                "调研",
                "A".into(),
                vec!["b".into()],
                1,
                IntentType::Chat,
                Complexity::Simple,
            ),
            TaskNode::new(
                "b",
                "编码实现",
                "B".into(),
                vec!["a".into()],
                1,
                IntentType::Code,
                Complexity::Simple,
            ),
        ];
        let layers = topo_layers(&nodes);
        let flat: Vec<&String> = layers.iter().flatten().collect();
        assert_eq!(flat.len(), 2, "环节点应全部被拍平执行，不死锁");
    }

    #[test]
    fn test_aggregate_layer_order_stable() {
        let nodes = decompose("实现功能", DagKind::Draft);
        let mut outputs = HashMap::new();
        outputs.insert("t1".to_string(), "规划产出".to_string());
        outputs.insert("t2".to_string(), "实现产出".to_string());
        outputs.insert("t3".to_string(), "审校产出".to_string());
        let parts = aggregate(&nodes, &outputs);
        assert_eq!(parts.len(), 3);
        assert!(parts[0].contains("架构设计"));
        assert!(parts[1].contains("编码实现"));
        assert!(parts[2].contains("测试验证"));
    }

    #[test]
    fn test_decompose_reflect_shape() {
        let nodes = decompose("实现登录", DagKind::Reflect);
        assert_eq!(nodes.len(), 5);
        let t3 = nodes.iter().find(|n| n.id == "t3").unwrap();
        assert_eq!(t3.deps, vec!["t1", "t2"]);
        let tr = nodes.iter().find(|n| n.id == "t_reflect").unwrap();
        assert_eq!(tr.deps, vec!["t3".to_string()]);
        assert_eq!(tr.role, "反思自检");
        let t4 = nodes.iter().find(|n| n.id == "t4").unwrap();
        assert_eq!(t4.deps, vec!["t_reflect".to_string()]);
    }

    #[test]
    fn test_decompose_fanout_splits_enumeration() {
        let nodes = decompose(
            "实现以下模块：\n1. auth\n2. db\n3. api",
            DagKind::FanOut { fanout: 2 },
        );
        // t1 调研 + t2 架构设计 + 3 编码实现 + t4 测试验证 = 6 节点
        assert_eq!(nodes.len(), 6);
        let impls: Vec<_> = nodes.iter().filter(|n| n.role == "编码实现").collect();
        assert_eq!(
            impls.len(),
            3,
            "枚举 3 项应 fan-out 为 3 个实现节点，而非 fanout=2 分片"
        );
        // 每个实现节点依赖调研 + 架构设计，且相互并行（无相互依赖）
        for n in &impls {
            assert_eq!(n.deps, vec!["t1", "t2"]);
        }
        // 测试验证依赖全部实现节点
        let t4 = nodes.iter().find(|n| n.id == "t4").unwrap();
        assert_eq!(t4.deps.len(), 3);
    }

    #[test]
    fn test_decompose_fanout_shards_when_no_enumeration() {
        let nodes = decompose("重构整个仓库", DagKind::FanOut { fanout: 6 });
        let impls: Vec<_> = nodes.iter().filter(|n| n.role == "编码实现").collect();
        assert_eq!(
            impls.len(),
            6,
            "无枚举时按 fanout=6 分片为 6 个并行实现节点"
        );
        let t4 = nodes.iter().find(|n| n.id == "t4").unwrap();
        assert_eq!(t4.deps.len(), 6, "测试验证应依赖全部 6 个实现节点");
    }

    #[test]
    fn test_topo_layers_fanout_impls_parallel() {
        let nodes = decompose("实现模块", DagKind::FanOut { fanout: 3 });
        let layers = topo_layers(&nodes);
        // 层 0：t1+t2；层 1：3 个实现节点并行；层 2：t4
        assert_eq!(layers.len(), 3);
        assert_eq!(layers[0].len(), 2);
        assert_eq!(layers[1].len(), 3, "3 个实现节点应同层并行");
        assert_eq!(layers[2], vec!["t4"]);
    }

    #[test]
    fn test_topo_layers_reflect() {
        let nodes = decompose("实现功能", DagKind::Reflect);
        let layers = topo_layers(&nodes);
        assert_eq!(layers.len(), 4);
        // 第一层为无依赖的 t1(调研, prio 7) 与 t2(架构设计, prio 8)，
        // 层内顺序遵循 topo_layers 契约：priority 降序 → t2 先于 t1。
        assert_eq!(layers[0], vec!["t2", "t1"]);
        assert_eq!(layers[1], vec!["t3"]);
        // s2_5 反思自检独占一层，且严格位于实现之后、审校之前。
        assert_eq!(layers[2], vec!["t_reflect"]);
        assert_eq!(layers[3], vec!["t4"]);
    }

    #[test]
    fn test_reflect_pass_all_ok() {
        let summaries = vec![
            "调研结论".into(),
            "方案".into(),
            "代码".into(),
            "反思".into(),
            "审校".into(),
        ];
        let r = reflect_pass(&summaries);
        assert!(r.go);
        assert!(r.failed_stages.is_empty());
        assert_eq!(r.notes.len(), 5);
    }

    #[test]
    fn test_reflect_pass_detects_failure() {
        let summaries = vec![
            "调研结论".into(),
            "方案".into(),
            "[子任务 编码实现 失败: network error]".into(),
            "反思".into(),
            "审校".into(),
        ];
        let r = reflect_pass(&summaries);
        assert!(!r.go);
        assert_eq!(r.failed_stages, vec!["阶段 3".to_string()]);
    }

    /// 回归守卫：反思门必须识别**真实执行器**写下的失败占位，
    /// 而不只是 `[子任务 …]` 这一种历史前缀。
    #[test]
    fn test_reflect_pass_detects_real_executor_markers() {
        for marker in [
            "[子专家执行失败] network error",
            "[子专家执行超时]",
            "[子专家未完成]",
            "[子专家无结果返回]",
            "[成员 spawn 失败: boom]",
            "[依赖环：成员 b 的前置未达成，无法调度]",
        ] {
            let r = reflect_pass(&[marker.to_string()]);
            assert!(
                !r.go,
                "反思门应把 `{marker}` 判为失败阶段，否则失败产出会被当成有效结论交付"
            );
            assert_eq!(r.failed_stages, vec!["阶段 1".to_string()]);
        }
        // 真实产出不得被误杀。
        let ok = reflect_pass(&["[子专家 编码] 已交付 foo.rs，测试通过".to_string()]);
        assert!(ok.go, "含 `[子专家 ` 前缀的真实产出不应被判失败");
    }

    #[test]
    fn test_reflect_pass_empty_stage() {
        let summaries = vec!["a".into(), "".into(), "c".into()];
        let r = reflect_pass(&summaries);
        assert!(!r.go);
        assert_eq!(r.failed_stages, vec!["阶段 2".to_string()]);
    }
}
