//! 自适应预算：把「轮数 / 单步超时 / 整轮超时 / 重试」从散落的硬编码常量，
//! 改为**按任务本身推导**的统一策略。
//!
//! 设计边界（必须诚实）：
//! - 不是把超时交给一个不受约束的模型随意设值。运行时的超时是安全边界，
//!   完全放开会让一个卡死的上游请求永远占着 worker。
//! - 这里的「智能判断」由三部分组成：
//!   1. **模型显式声明**：任务/规划文本里可以出现 `[[budget:simple|standard|complex|deep]]`，
//!      这是规划模型对任务体量的自我判断，优先级最高；
//!   2. **任务信号**：文本长度、多步/深链路关键词、规划出的成员数、工具面规模；
//!   3. **硬护栏**：任何结果都被钳制在安全区间内（轮数 ≤ 64，单步 ≤ 30 分钟，
//!      整轮 ≤ 2 小时，重试 ≤ 5 次），env 覆盖仍然优先。
//!
//! 这样既避免了「所有任务一律 8 轮 / 120s」把复杂任务切断，也避免了「无上限」
//! 导致的资源悬挂。所有推导都是确定性的、可单测的。

use std::time::Duration;

/// 任务体量档位。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskScale {
    /// 单点问答 / 一次工具调用即可完成
    Simple,
    /// 常规多步任务
    Standard,
    /// 跨文件、跨阶段、需要多次验证的任务
    Complex,
    /// 长程任务：研究→实现→验证→交付，或大规模并行
    Deep,
}

impl TaskScale {
    pub fn as_str(self) -> &'static str {
        match self {
            TaskScale::Simple => "simple",
            TaskScale::Standard => "standard",
            TaskScale::Complex => "complex",
            TaskScale::Deep => "deep",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "simple" | "light" | "quick" => Some(TaskScale::Simple),
            "standard" | "normal" | "default" => Some(TaskScale::Standard),
            "complex" | "hard" => Some(TaskScale::Complex),
            "deep" | "long" | "max" => Some(TaskScale::Deep),
            _ => None,
        }
    }

    fn upgraded(self) -> Self {
        match self {
            TaskScale::Simple => TaskScale::Standard,
            TaskScale::Standard => TaskScale::Complex,
            TaskScale::Complex | TaskScale::Deep => TaskScale::Deep,
        }
    }
}

/// 一次任务的预算（全部经过钳制）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BudgetPolicy {
    pub scale: TaskScale,
    /// 工具回路最大轮数
    pub tool_rounds: usize,
    /// 单次模型/工具步骤的超时
    pub step_timeout: Duration,
    /// 整轮（含多成员并行）的超时
    pub run_timeout: Duration,
    /// provider 读超时（连续多久没有字节才判死）
    pub llm_read_timeout_secs: u64,
    /// 可重试次数（网络/限流类错误）
    pub max_retries: u32,
}

/// 硬护栏：任何「智能判断」都不能越过这些值。
pub const MAX_TOOL_ROUNDS: usize = 64;
pub const MAX_STEP_SECS: u64 = 30 * 60;
pub const MAX_RUN_SECS: u64 = 2 * 60 * 60;
pub const MAX_RETRIES: u32 = 5;

/// 子代理并发闸门的默认值（同时运行的成员数）。
///
/// 这是**并发控制**，不是成员数量上限：更多成员会排队而不是被丢弃。
/// 取 5 的依据：provider 侧限流（主流 API 的并发配额多在个位数）+ 单机 CPU/网络
/// 同时承担 N 路流式响应；`MARGUERITE_SUBAGENT_MAX_CONCURRENT` 可覆盖。
pub const DEFAULT_SUBAGENT_MAX_CONCURRENT: usize = 5;
/// 并发闸门的硬护栏（防一次开出上百路流式连接把 provider 打挂）。
pub const MAX_SUBAGENT_MAX_CONCURRENT: usize = 64;

/// 团队整轮的硬护栏：比单 agent 的 [`MAX_RUN_SECS`] 宽，因为成员是**分波**跑的。
///
/// 推导：单成员最长 30 分钟 × 64 成员 / 5 并发 ≈ 6.4h，取 4h 作为「绝大多数真实
/// 团队任务不会超过」的边界；它仍是**有界**的（不会挂死），并且
/// `MARGUERITE_TEAM_RUN_TIMEOUT_SECS` 可在区间内覆盖。
pub const MAX_TEAM_RUN_SECS: u64 = 4 * 60 * 60;

/// 当前生效的子代理并发闸门（env 可覆盖，默认 [`DEFAULT_SUBAGENT_MAX_CONCURRENT`]）。
pub fn subagent_max_concurrent() -> usize {
    env_u64("MARGUERITE_SUBAGENT_MAX_CONCURRENT")
        .map(|value| value as usize)
        .unwrap_or(DEFAULT_SUBAGENT_MAX_CONCURRENT)
        .clamp(1, MAX_SUBAGENT_MAX_CONCURRENT)
}

/// 团队 / 蜂群整轮超时：按「成员数 ÷ 并发闸门」的分波数放大单轮预算。
///
/// 为什么必须按成员数放大：成员是分波 spawn 的，成员越多波数越多；
/// 旧实现用固定 1 小时，24 名成员的蜂群在最后几波里必然被外层看门狗误判超时。
/// 优先级：`MARGUERITE_TEAM_RUN_TIMEOUT_SECS` > 按规模放大 > 单轮基准；
/// 最终仍钳制在 `[30s, MAX_TEAM_RUN_SECS]`（有界，不会无界挂死）。
pub fn team_run_timeout(task: &str, members: usize) -> Duration {
    let base = resolve(task, members, 0).run_timeout;
    let concurrency = subagent_max_concurrent().max(1);
    let waves = members.max(1).div_ceil(concurrency);
    let scaled = base.as_secs().saturating_mul(waves as u64);
    let secs = env_u64("MARGUERITE_TEAM_RUN_TIMEOUT_SECS").unwrap_or(scaled);
    Duration::from_secs(secs.clamp(30, MAX_TEAM_RUN_SECS))
}

/// 解析任务文本里模型显式写下的预算声明：`[[budget:deep]]` / `[budget=complex]`。
///
/// 这是「让模型判断」的入口：规划模型可以在计划开头标注任务体量，运行时据此
/// 放宽/收紧预算，而不是由代码写死一个档位。
pub fn declared_scale(text: &str) -> Option<TaskScale> {
    let lower = text.to_ascii_lowercase();
    for marker in ["[[budget:", "[budget:", "[budget="] {
        if let Some(start) = lower.find(marker) {
            let rest = &lower[start + marker.len()..];
            let end = rest
                .find(|c: char| c == ']' || c == '}' || c.is_whitespace())
                .unwrap_or(rest.len());
            if let Some(scale) = TaskScale::parse(&rest[..end]) {
                return Some(scale);
            }
        }
    }
    None
}

/// 任务信号打分（确定性，可解释）。
fn score_task(text: &str, planned_members: usize, tool_count: usize) -> u32 {
    let lower = text.to_ascii_lowercase();
    let mut score = 0u32;
    // 文本越长，越可能不是一句话问答（每 400 字符 +1，最多 +3）。
    score += (text.chars().count() / 400).min(3) as u32;
    // 多步 / 深链路信号。
    const DEEP_SIGNALS: &[&str] = &[
        "重构", "架构", "迁移", "端到端", "审计", "研究", "调研", "实现", "部署", "验证",
        "多文件", "并行", "团队", "蜂群", "生成", "建模", "视频", "游戏", "报告", "测试",
        "修复", "优化", "refactor", "migrate", "end-to-end", "audit", "research",
        "implement", "deploy", "verify", "multi-file", "parallel", "generate",
    ];
    // 编码/文件类信号：这类任务即便描述很短也需要多轮工具回路（读→改→跑→验），
    // 至少按 Standard 起步，不能让「帮我改一下这个文件」被当成一句话问答。
    const CODE_SIGNALS: &[&str] = &[
        "代码", "文件", "目录", "命令", "运行", "编译", "构建", "安装", "脚本", "接口",
        "函数", "bug", "报错", "日志", "code", "file", "run", "build", "install",
        "script", "function", "error", "log",
    ];
    let deep_hits = DEEP_SIGNALS
        .iter()
        .filter(|signal| lower.contains(**signal))
        .count();
    let code_hits = CODE_SIGNALS
        .iter()
        .filter(|signal| lower.contains(**signal))
        .count();
    // 代码/文件类任务至少要进入 Standard：读→改→跑→验本身就是多轮回路。
    score += (deep_hits.min(6) + if code_hits > 0 { 2 } else { 0 }) as u32;
    // 规划出的成员数：>1 说明模型自己判断需要分工。
    if planned_members >= 2 {
        score += 1;
    }
    if planned_members >= 4 {
        score += 1;
    }
    // 工具面很大（大量 MCP/插件工具）通常意味着更长的发现-调用回路。
    if tool_count >= 40 {
        score += 1;
    }
    score
}

fn scale_from_score(score: u32) -> TaskScale {
    match score {
        0..=1 => TaskScale::Simple,
        2..=3 => TaskScale::Standard,
        4..=6 => TaskScale::Complex,
        _ => TaskScale::Deep,
    }
}

fn base_policy(scale: TaskScale) -> BudgetPolicy {
    // (轮数, 单步秒, 整轮秒, provider 读超时秒, 请求尝试次数)
    // 尝试次数随任务体量递增：一句话问答不值得为瞬态错误反复等退避，
    // 长程任务则宁可多试几次也不要让整轮工作白费（历史值 3 保留在 Standard）。
    let (rounds, step, run, read, retries) = match scale {
        TaskScale::Simple => (12usize, 120u64, 900u64, 120u64, 2u32),
        TaskScale::Standard => (24, 300, 1_800, 240, 3),
        TaskScale::Complex => (40, 600, 3_600, 360, 4),
        TaskScale::Deep => (64, 900, 7_200, 480, 5),
    };
    BudgetPolicy {
        scale,
        tool_rounds: rounds,
        step_timeout: Duration::from_secs(step),
        run_timeout: Duration::from_secs(run),
        llm_read_timeout_secs: read,
        max_retries: retries,
    }
}

fn env_u64(key: &str) -> Option<u64> {
    std::env::var(key)
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
}

fn clamp_policy(mut policy: BudgetPolicy) -> BudgetPolicy {
    policy.tool_rounds = policy.tool_rounds.clamp(1, MAX_TOOL_ROUNDS);
    let step = policy.step_timeout.as_secs().clamp(30, MAX_STEP_SECS);
    let run = policy.run_timeout.as_secs().clamp(step, MAX_RUN_SECS);
    policy.step_timeout = Duration::from_secs(step);
    policy.run_timeout = Duration::from_secs(run);
    policy.llm_read_timeout_secs = policy
        .llm_read_timeout_secs
        .clamp(30, MAX_STEP_SECS);
    policy.max_retries = policy.max_retries.min(MAX_RETRIES);
    policy
}

/// 解析一次任务的预算。
///
/// 优先级：env 强制档位 → 模型声明 `[[budget:...]]` → 任务信号推导。
/// 调用方仍可用单项 env（`MARGUERITE_SUBAGENT_TOOL_ROUNDS` /
/// `MARGUERITE_SUBAGENT_TIMEOUT_SECS` / `MARGUERITE_ORCH_RUN_TIMEOUT_SECS`）覆盖。
pub fn resolve(task: &str, planned_members: usize, tool_count: usize) -> BudgetPolicy {
    let forced = std::env::var("MARGUERITE_BUDGET_SCALE")
        .ok()
        .and_then(|value| TaskScale::parse(&value));
    let mut scale = forced
        .or_else(|| declared_scale(task))
        .unwrap_or_else(|| scale_from_score(score_task(task, planned_members, tool_count)));
    // 多个成员并行本身就是「体量更大」的强信号：至少升一档。
    if planned_members >= 4 {
        scale = scale.upgraded();
    }
    let mut policy = base_policy(scale);
    // 工具面越大，发现-调用回路越长，轮数适度上浮。
    if tool_count >= 40 {
        policy.tool_rounds += 8;
    }
    // 单项 env 覆盖（历史变量保持兼容）。
    if let Some(rounds) = env_u64("MARGUERITE_SUBAGENT_TOOL_ROUNDS") {
        policy.tool_rounds = rounds as usize;
    }
    if let Some(secs) = env_u64("MARGUERITE_SUBAGENT_TIMEOUT_SECS") {
        policy.run_timeout = Duration::from_secs(secs);
    }
    if let Some(secs) = env_u64("MARGUERITE_ORCH_RUN_TIMEOUT_SECS") {
        policy.run_timeout = Duration::from_secs(secs);
    }
    clamp_policy(policy)
}

/// 子代理工具轮数（env 覆盖在 [`resolve`] 内已处理）。
pub fn subagent_rounds(task: &str, tool_count: usize) -> usize {
    resolve(task, 1, tool_count).tool_rounds
}

/// 单次 LLM 请求的重试次数。
///
/// 这一层拿不到任务文本（provider 客户端只认配置），所以按 Standard 档取基准；
/// `MARGUERITE_LLM_MAX_RETRIES` 可按环境覆盖，仍受 [`MAX_RETRIES`] 硬护栏约束。
pub fn default_max_retries() -> u32 {
    env_u64("MARGUERITE_LLM_MAX_RETRIES")
        .map(|value| value as u32)
        .unwrap_or_else(|| resolve("[[budget:standard]]", 1, 0).max_retries)
        .clamp(1, MAX_RETRIES)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_question_gets_simple_budget() {
        let policy = resolve("你好", 0, 5);
        assert_eq!(policy.scale, TaskScale::Simple);
        assert_eq!(policy.tool_rounds, 12);
        assert!(policy.run_timeout.as_secs() >= 600);
    }

    #[test]
    fn code_task_is_at_least_standard() {
        // 描述很短，但涉及代码/文件 → 必须是 Standard 起步（回归「一句话任务只给 8 轮」）。
        let policy = resolve("帮我改一下这个文件", 0, 10);
        assert_ne!(policy.scale, TaskScale::Simple);
        assert!(policy.tool_rounds >= 24);
    }

    #[test]
    fn multi_step_task_gets_more_rounds_and_time() {
        let simple = resolve("你好", 0, 5);
        let complex = resolve(
            "重构支付模块并迁移数据库，补测试，端到端验证后部署，输出报告",
            3,
            50,
        );
        assert!(complex.tool_rounds > simple.tool_rounds, "{complex:?}");
        assert!(complex.run_timeout > simple.run_timeout, "{complex:?}");
        assert!(complex.max_retries >= simple.max_retries);
    }

    #[test]
    fn model_declared_scale_wins_over_text_signals() {
        let policy = resolve("[[budget:deep]] 做一个登录页", 0, 5);
        assert_eq!(policy.scale, TaskScale::Deep);
        assert_eq!(policy.tool_rounds, 64);
    }

    #[test]
    fn policy_is_always_clamped() {
        // 极端声明也不能突破硬护栏（直接对钳制函数断言，避免测试间共享 env）。
        let clamped = clamp_policy(BudgetPolicy {
            scale: TaskScale::Deep,
            tool_rounds: 9_999,
            step_timeout: Duration::from_secs(999_999),
            run_timeout: Duration::from_secs(999_999),
            llm_read_timeout_secs: 999_999,
            max_retries: 99,
        });
        assert!(clamped.tool_rounds <= MAX_TOOL_ROUNDS);
        assert!(clamped.step_timeout.as_secs() <= MAX_STEP_SECS);
        assert!(clamped.run_timeout.as_secs() <= MAX_RUN_SECS);
        assert!(clamped.max_retries <= MAX_RETRIES);
    }

    #[test]
    fn many_planned_members_upgrade_scale() {
        let policy = resolve("做一个页面", 6, 5);
        assert_ne!(policy.scale, TaskScale::Simple);
        assert!(policy.tool_rounds >= 16);
    }

    #[test]
    fn llm_retry_budget_is_bounded() {
        // 不假设进程 env 干净，只断言落在硬护栏区间内。
        let retries = default_max_retries();
        assert!((1..=MAX_RETRIES).contains(&retries), "{retries}");
    }
}
