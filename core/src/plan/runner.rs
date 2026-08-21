//! 复杂任务计划驱动长执行（Bloom 长续航核心）
//!
//! 聊天主路径检测到复杂任务（复杂度 Complex）时，不再跑单轮，而是进入
//! 「自动拆解 → 计划驱动分步执行」闭环：
//!
//! 1. `plans.analyze` 自动拆解 → 发布 `session.event.plan_revision`（前端
//!    PlanRunRibbon 出现计划步骤）；
//! 2. `plans.run` 启动运行 → 发布 `session.event.plan_run`（running）；
//! 3. 注册长时任务（心跳 + 检查点，跨会话续跑/恢复）；
//! 4. 逐步执行：每步由 Orchestrator 子任务驱动，进度与完成事件逐步发布，
//!    步骤间做上下文渐进压缩（防长窗口爆掉）、行为捕获；
//! 5. 步骤失败：发布错误并终止运行（terminal reason=blocked），已完成结论仍回传；
//! 6. 全部完成：运行置 completed，聚合各步结论作为 lead 收据返回。

use std::sync::{Arc, OnceLock};

use serde_json::{json, Value};

use crate::automation::long_running::{self, Stage};
use crate::events::{Envelope, EventBus, EventType};
use crate::model::router::Complexity;
use crate::orchestration::{OrchestrationOutcome, Orchestrator};
use crate::persistence::config::Config;
use crate::plan;

/// 复杂任务判定与分析共享的分析器（纯规则、零 LLM、进程内缓存避免重复构造）。
fn analyzer() -> &'static Orchestrator {
    static R: OnceLock<Orchestrator> = OnceLock::new();
    R.get_or_init(|| Orchestrator::new(Config::default()))
}

/// 项目型多阶段信号检测：不依赖复杂度分类器。
///
/// 覆盖「帮我做一个完善的FPS…」这类被分类器判为 Simple/Chat、但本质是多阶段
/// 交付型诉求的文本——命中「构建动作词 + 产物名/强化词」即视为复杂任务。
fn has_project_signal(text: &str) -> bool {
    let t = text.to_lowercase();
    const BUILD: &[&str] = &[
        "做一个",
        "做个",
        "构建",
        "开发",
        "搭建",
        "创建",
        "实现一个",
        "实现",
        "设计一个",
        "写一个",
        "完成一个",
        "做出一个",
        "完成",
    ];
    if !BUILD.iter().any(|w| t.contains(w)) {
        return false;
    }
    const PRODUCT: &[&str] = &[
        "游戏",
        "系统",
        "平台",
        "项目",
        "应用",
        "app",
        "工具",
        "网站",
        "网页",
        "软件",
        "编译器",
        "框架",
        "引擎",
        "客户端",
        "服务端",
        "数据库",
        "编辑器",
        "机器人",
        "模块",
        "组件",
    ];
    const STRENGTHEN: &[&str] = &["完善", "完整", "全套", "高级", "从零"];
    PRODUCT.iter().any(|w| t.contains(w)) || STRENGTHEN.iter().any(|w| t.contains(w))
}

/// 统一判定：是否应走「计划驱动长执行」（主路径判定与 runner 兜底复核共用）。
fn qualifies(config: &Config, text: &str) -> bool {
    if !config.plan.auto_decompose {
        return false;
    }
    let text = text.trim();
    let len = text.chars().count();
    // 极短/中短输入（闲聊式响应）不拆解；防过度
    if text.is_empty() || len < 12 {
        return false;
    }
    let a = analyzer().analyze(text);
    a.complexity == Complexity::Complex || has_project_signal(text)
}

/// 聊天主路径判定：该输入是否应走「计划驱动长执行」。
///
/// 开启 `plan.auto_decompose` 且满足以下任一条件时返回 true：
/// 1. 复杂度分类为 Complex（编程/科研等长任务）；
/// 2. 命中项目型信号（做/构建/开发 + 产物名/强化词，如「做一个完善的FPS射击游戏」）；
///
/// 简单/中等任务保持原单轮路径，行为完全向后兼容。
///
/// 门槛只做「闲聊过滤」：小于 12 字符的输入不拆解（避免"帮我修个bug"/"好"等
/// 简短诉求被切成多步噪音）。
pub fn should_auto_decompose(config: &Config, text: &str) -> bool {
    qualifies(config, text)
}

/// 计划驱动长执行入口。
///
/// 调用方保证已通过 [`should_auto_decompose`] 判定（本函数内部也会兜底复核）。
/// 事件经 `bus` 回流网关 → 前端；返回的 `OrchestrationOutcome` 由调用方包装
/// `session.event.router_decision` + `session.event.done` 富收据。
/// `experience_hint` 为可选的系统提示注入片段（如跨项目记忆），随每个子步骤
/// 编排器一并注入；空串时行为与旧版完全一致。
pub async fn run_autonomous(
    config: Config,
    prompt: String,
    session: String,
    bus: Arc<EventBus>,
    experience_hint: String,
) -> anyhow::Result<OrchestrationOutcome> {
    if !qualifies(&config, &prompt) {
        return Err(anyhow::anyhow!(
            "run_autonomous 仅用于复杂/项目型任务，当前输入不满足"
        ));
    }
    let max_steps = config.plan.max_steps.max(1);
    let ckpt_every = config.plan.checkpoint_interval_steps.max(1);
    let heartbeat_s = config.plan.heartbeat_interval_s.max(5);

    // 1) 自动拆解 + 落库修订
    let created = plan::plans_create(&json!({ "prompt": prompt }));
    let Some(rev) = created.get("revision") else {
        let why = created
            .get("error")
            .and_then(Value::as_str)
            .unwrap_or("未知");
        return Err(anyhow::anyhow!("自动拆解失败：{why}"));
    };
    publish_plan(
        &bus,
        &session,
        EventType::PlanRevision,
        json!({ "plan_revision": rev.clone() }),
    );

    // 2) 启动运行实例
    let revision_id = rev["revisionId"].as_str().unwrap_or("").to_string();
    let started = plan::plans_run(&json!({ "revisionId": revision_id }));
    let Some(run) = started.get("run") else {
        return Err(anyhow::anyhow!(
            "计划运行启动失败：{}",
            started
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("未知")
        ));
    };
    let run_id = run["runId"].as_str().unwrap_or("").to_string();
    publish_plan(
        &bus,
        &session,
        EventType::PlanRun,
        json!({ "plan_run": run.clone() }),
    );

    // 3) 长时任务注册（心跳 + 检查点，跨会话续跑）
    let lrt_id = {
        let mut g = long_running::global()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        g.register(&prompt, heartbeat_s)
    };

    let mut lead_replies: Vec<String> = Vec::new();
    let mut completed_steps = 0usize;
    let mut stopped_reason = "completed";
    let mut failed_step: Option<String> = None;

    // 4) 逐步执行
    loop {
        let Some((sid, title)) = current_step(&run_id) else {
            break; // 无进行中/待执行步骤 → 计划结束
        };
        completed_steps += 1;
        if completed_steps > max_steps {
            stopped_reason = "max_steps";
            tracing::warn!(session = %session, run_id = %run_id, steps = completed_steps,
                "计划执行达 max_steps 上限，自动停止");
            let _ = plan::plans_cancel(&json!({ "runId": run_id }));
            break;
        }

        // 步骤间上下文渐进压缩（防长窗口爆掉）
        compact_if_needed(&session, &bus);

        // 子步骤任务文本：步骤标题 + 整体目标上下文
        let step_task = format!(
            "【计划子步骤 {sid}｜{title}】\n整体目标：{prompt}\n\n请完成上述子步骤，输出有依据的结论；不要展开其他子步骤。"
        );

        let mut orch = Orchestrator::new(config.clone());
        orch.set_event_bus(bus.clone(), session.clone());
        // 跨项目记忆等提示随子步骤编排注入（空串自动跳过）
        if !experience_hint.is_empty() {
            orch.set_experience_hint(experience_hint.clone());
        }
        match orch.dispatch(&step_task).await {
            Ok(outcome) => {
                let reply = outcome.lead_reply;
                lead_replies.push(reply.clone());
                crate::context::session::record_turn(&session, "assistant", &reply);
                crate::memory::behavior::record_behavior(
                    &session,
                    "plan_step_done",
                    &format!("{sid} {title}"),
                );
                // 心跳 + 检查点（每 ckpt_every 步）
                let mut g = long_running::global()
                    .lock()
                    .unwrap_or_else(|e| e.into_inner());
                let _ = g.heartbeat(&lrt_id);
                if completed_steps % ckpt_every == 0 {
                    let _ = g.save_checkpoint(
                        &lrt_id,
                        Stage::Execute,
                        json!({
                            "runId": run_id,
                            "stepId": sid,
                            "completedSteps": completed_steps,
                            "maxSteps": max_steps,
                        }),
                    );
                }
                drop(g);
                // 推进运行：当前步骤 completed → 下一 pending
                let advanced =
                    plan::plans_run_advance(&json!({ "runId": run_id, "advance": true }));
                if let Some(r) = advanced.get("run") {
                    publish_plan(
                        &bus,
                        &session,
                        EventType::PlanRun,
                        json!({ "plan_run": r.clone() }),
                    );
                    if r["status"].as_str() == Some("completed") {
                        break;
                    }
                }
            }
            Err(e) => {
                failed_step = Some(format!("{sid} {title}"));
                stopped_reason = "failed";
                let msg = format!("计划子步骤 {sid} 执行失败：{e}");
                tracing::warn!(session = %session, run_id = %run_id, err = %e, "计划步骤失败");
                crate::memory::behavior::record_behavior(&session, "plan_step_failed", &msg);
                bus.publish(Envelope::error(&session, "plan_runner", &msg));
                // 失败即终止本轮运行（不静默重试），已完成步骤结论保留回传
                let _ = plan::plans_cancel(&json!({ "runId": run_id }));
                break;
            }
        }
    }

    // 5) 收尾：长时任务置完成 + 运行终态事件 + 聚合结论
    let mut g = long_running::global()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _ = g.heartbeat(&lrt_id);
    let _ = g.complete(&lrt_id);
    let latest = long_running::task_view(g.get(&lrt_id).expect("长时任务应存在"));
    drop(g);
    if stopped_reason != "failed" {
        if let Some(r) = plan::plans_checklist(&json!({ "runId": run_id })).get("run") {
            publish_plan(
                &bus,
                &session,
                EventType::PlanRun,
                json!({ "plan_run": r.clone() }),
            );
        }
    }
    let lead_reply = if lead_replies.is_empty() {
        format!("计划执行结束（{}）。", stopped_reason)
    } else if stopped_reason == "failed" {
        format!(
            "已完成 {} 个步骤，第 {} 步失败后停止。已完成结论如下：\n\n{}",
            completed_steps.saturating_sub(1),
            failed_step.unwrap_or_default(),
            lead_replies.join("\n\n")
        )
    } else {
        lead_replies.join("\n\n")
    };
    let _ = latest;
    let analysis = analyzer().analyze(&prompt);
    Ok(OrchestrationOutcome {
        strategy: analysis.strategy,
        lead_reply,
        subagent_summaries: lead_replies,
        analysis,
        completion_verdict: crate::governance::completion_gate::CompletionVerdict::allowed(),
        reflect_verdict: None,
    })
}

/// 方案 markdown → 步骤执行的「整体目标」上下文。
///
/// 剔除 `**编排形态**` / `**决策理由**` 等元信息行：这些行里的策略理由文本
/// （如「单 Agent 循环即可覆盖」）会被宪法前置裁决的破坏性关键词扫描误判为
/// C1 违规（「覆盖」命中），导致已批准实施的方案步骤被治理门拦截。
fn plan_step_context(markdown: &str) -> String {
    markdown
        .lines()
        .filter(|line| {
            let t = line.trim();
            !(t.starts_with("**编排形态**") || t.starts_with("**决策理由**"))
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// 按既有修订逐步执行（`plans.implement` 的后台执行体）。
///
/// 与 [`run_autonomous`] 的区别：不重新拆解，直接复用修订的标题与 markdown
/// 作为整体目标，按修订步骤逐条经 Orchestrator 真实执行；进度事件经 `bus`
/// 实时回流，`run_id` 由调用方先行创建（立即返回运行中卡片）。
pub async fn run_plan_revision_steps(
    config: Config,
    revision_id: String,
    run_id: String,
    session: String,
    bus: Arc<EventBus>,
    experience_hint: String,
) -> anyhow::Result<String> {
    let Some(rev_view) = plan::get_revision_view(&revision_id) else {
        anyhow::bail!("Plan revision not found: {revision_id}");
    };
    // 步骤「整体目标」用剔除元信息后的方案正文，避免决策理由文本（如
    // 「单 Agent 循环即可覆盖」）被宪法 C1 破坏性关键词扫描误判拦截。
    let prompt = plan_step_context(rev_view["markdown"].as_str().unwrap_or(""));
    let title = rev_view["title"].as_str().unwrap_or("").to_string();
    let max_steps = config.plan.max_steps.max(1);
    let ckpt_every = config.plan.checkpoint_interval_steps.max(1);
    let heartbeat_s = config.plan.heartbeat_interval_s.max(5);

    let lrt_id = {
        let mut g = long_running::global()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        g.register(&prompt, heartbeat_s)
    };

    let mut lead_replies: Vec<String> = Vec::new();
    let mut completed_steps = 0usize;
    let mut stopped_reason = "completed";
    let mut failed_step: Option<String> = None;

    loop {
        let Some((sid, step_title)) = current_step(&run_id) else {
            break; // 无进行中/待执行步骤 → 计划结束
        };
        completed_steps += 1;
        if completed_steps > max_steps {
            stopped_reason = "max_steps";
            tracing::warn!(session = %session, run_id = %run_id, steps = completed_steps,
                "计划执行达 max_steps 上限，自动停止");
            let _ = plan::plans_cancel(&json!({ "runId": run_id }));
            break;
        }

        // 步骤间上下文渐进压缩（防长窗口爆掉）
        compact_if_needed(&session, &bus);

        // 子步骤任务文本：步骤标题 + 修订全文作整体目标
        let step_task = format!(
            "【计划子步骤 {sid}｜{step_title}】\n整体目标：{title}\n\n{prompt}\n\n请完成上述子步骤，输出有依据的结论；不要展开其他子步骤。"
        );

        let mut orch = Orchestrator::new(config.clone());
        orch.set_event_bus(bus.clone(), session.clone());
        // 跨项目记忆等提示随子步骤编排注入（空串自动跳过）
        if !experience_hint.is_empty() {
            orch.set_experience_hint(experience_hint.clone());
        }
        match orch.dispatch(&step_task).await {
            Ok(outcome) => {
                let reply = outcome.lead_reply;
                lead_replies.push(reply.clone());
                crate::context::session::record_turn(&session, "assistant", &reply);
                crate::memory::behavior::record_behavior(
                    &session,
                    "plan_step_done",
                    &format!("{sid} {step_title}"),
                );
                // 心跳 + 检查点（每 ckpt_every 步）
                let mut g = long_running::global()
                    .lock()
                    .unwrap_or_else(|e| e.into_inner());
                let _ = g.heartbeat(&lrt_id);
                if completed_steps % ckpt_every == 0 {
                    let _ = g.save_checkpoint(
                        &lrt_id,
                        Stage::Execute,
                        json!({
                            "runId": run_id,
                            "stepId": sid,
                            "completedSteps": completed_steps,
                            "maxSteps": max_steps,
                        }),
                    );
                }
                drop(g);
                // 推进运行：当前步骤 completed → 下一 pending
                let advanced =
                    plan::plans_run_advance(&json!({ "runId": run_id, "advance": true }));
                if let Some(r) = advanced.get("run") {
                    publish_plan(
                        &bus,
                        &session,
                        EventType::PlanRun,
                        json!({ "plan_run": r.clone() }),
                    );
                    if r["status"].as_str() == Some("completed") {
                        break;
                    }
                }
            }
            Err(e) => {
                failed_step = Some(format!("{sid} {step_title}"));
                stopped_reason = "failed";
                let msg = format!("计划子步骤 {sid} 执行失败：{e}");
                tracing::warn!(session = %session, run_id = %run_id, err = %e, "计划步骤失败");
                crate::memory::behavior::record_behavior(&session, "plan_step_failed", &msg);
                bus.publish(Envelope::error(&session, "plan_runner", &msg));
                // 失败即终止本轮运行（不静默重试），已完成步骤结论保留回传
                let _ = plan::plans_cancel(&json!({ "runId": run_id }));
                break;
            }
        }
    }

    // 收尾：长时任务置完成 + 运行终态事件 + 聚合结论
    let mut g = long_running::global()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _ = g.heartbeat(&lrt_id);
    let _ = g.complete(&lrt_id);
    drop(g);
    if stopped_reason != "failed" {
        if let Some(r) = plan::plans_checklist(&json!({ "runId": run_id })).get("run") {
            publish_plan(
                &bus,
                &session,
                EventType::PlanRun,
                json!({ "plan_run": r.clone() }),
            );
        }
    }
    let lead_reply = if lead_replies.is_empty() {
        format!("计划执行结束（{}）。", stopped_reason)
    } else if stopped_reason == "failed" {
        format!(
            "已完成 {} 个步骤，第 {} 步失败后停止。已完成结论如下：\n\n{}",
            completed_steps.saturating_sub(1),
            failed_step.unwrap_or_default(),
            lead_replies.join("\n\n")
        )
    } else {
        lead_replies.join("\n\n")
    };
    // 把本会话上下文落盘（新任务实施时新建会话，sessions.list / chat.history 可见）
    crate::gateway::chat::persist_session(&session);
    Ok(lead_reply)
}

/// 取当前进行中（或下一个待执行）步骤的 (stepId, title)。
fn current_step(run_id: &str) -> Option<(String, String)> {
    let view = plan::plans_checklist(&json!({ "runId": run_id }));
    let run = view.get("run")?;
    let steps = run.get("steps")?.as_array()?;
    for st in steps {
        let status = st["status"].as_str().unwrap_or("");
        if status == "in_progress" {
            return Some((
                st["stepId"].as_str()?.to_string(),
                st["title"].as_str().unwrap_or("").to_string(),
            ));
        }
    }
    // 无 in_progress 时取第一个 pending（守护：状态机异常时仍可推进）
    for st in steps {
        if st["status"].as_str() == Some("pending") {
            return Some((
                st["stepId"].as_str()?.to_string(),
                st["title"].as_str().unwrap_or("").to_string(),
            ));
        }
    }
    None
}

/// 上下文压缩：assistant 回复已记录；告警「需压缩」时触发 4 层渐进压缩并广播事件。
fn compact_if_needed(session: &str, bus: &Arc<EventBus>) {
    if crate::context::session::context_status(session)["alert"] == "compact_required" {
        crate::context::session::context_compact(session, Some(bus));
    }
}

/// 发布 `session.event.plan_revision` / `session.event.plan_run`（payload 含
/// session_key/key，对齐 Web UI `payloadBelongsToSession` 归属判定）。
pub(crate) fn publish_plan(bus: &Arc<EventBus>, session: &str, ty: EventType, payload: Value) {
    let mut p = payload;
    if p.get("session_key").is_none() {
        p["session_key"] = json!(session);
        p["key"] = json!(session);
    }
    bus.publish(Envelope::new(ty, p, session));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::EventBus;

    #[test]
    fn plan_step_context_strips_strategy_meta_lines() {
        // 用户实测触发治理门 C1 误判的方案 markdown：决策理由含「覆盖」。
        let md = "# 计划：帮我做一个fps射击游戏\n\n**编排形态**: Solo Agent\n\n**决策理由**: 单一意图（Unknown），单 Agent 循环即可覆盖（标准 AI 助手 基线等价）\n\n1. 理解需求与约束\n2. 实施与产出\n3. 验证与收尾\n";
        let ctx = plan_step_context(md);
        // 元信息行被剔除，且不再携带会命中宪法 C1 破坏性关键词的「覆盖」
        assert!(!ctx.contains("**编排形态**"));
        assert!(!ctx.contains("**决策理由**"));
        assert!(!ctx.contains("覆盖"));
        // 方案正文保留：标题 + 步骤列表
        assert!(ctx.contains("# 计划：帮我做一个fps射击游戏"));
        assert!(ctx.contains("1. 理解需求与约束"));
        assert!(ctx.contains("3. 验证与收尾"));
        // 破坏性关键词信号全部消失（C1 前置裁决据此放行）
        for keyword in [
            "删除",
            "覆盖",
            "force",
            "强制",
            "格式化",
            "format",
            "shutdown",
            "重启",
        ] {
            assert!(
                !ctx.to_lowercase().contains(keyword),
                "误命中破坏性关键词: {keyword}"
            );
        }
    }

    #[test]
    fn plan_step_context_keeps_plain_markdown_unchanged() {
        let md = "# 计划：测试\n\n1. 写代码\n2. 跑测试\n";
        let ctx = plan_step_context(md);
        assert_eq!(ctx.trim(), "# 计划：测试\n\n1. 写代码\n2. 跑测试");
    }

    #[test]
    fn short_input_never_decomposes() {
        let cfg = Config::default();
        assert!(!should_auto_decompose(&cfg, "你好"));
    }

    #[test]
    fn auto_decompose_off_disables() {
        let mut cfg = Config::default();
        cfg.plan.auto_decompose = false;
        assert!(!should_auto_decompose(&cfg, "请用 Rust 实现一个完整的 HTTP 服务器，包含路由、中间件、静态文件、TLS 与压力测试，并撰写架构文档与性能报告。"));
    }

    #[test]
    fn complex_coding_task_triggers_decompose() {
        let cfg = Config::default();
        // 编程类多阶段复杂任务（命中 Complex 分类）
        let text = "请从零构建一个 Rust 命令行编译器项目：先做词法分析、再做语法分析、语义检查、中间代码生成、目标代码优化，最后用基准测试验证正确性并撰写完整文档。";
        assert!(
            should_auto_decompose(&cfg, text),
            "复杂编程任务应触发自动拆解"
        );
    }

    #[test]
    fn short_project_request_triggers_decompose() {
        let cfg = Config::default();
        // 用户实测场景：17 字符的项目型诉求（此前被 20 字符门槛误拦）
        let text = "帮我做一个完善的FPS第一人称射击游戏";
        assert!(
            should_auto_decompose(&cfg, text),
            "短项目诉求（≥12 字符）应触发自动拆解"
        );
    }

    #[test]
    fn terse_fix_request_skips_decompose() {
        let cfg = Config::default();
        // 简短修复诉求（<12 字符）：不拆解，避免多步噪音
        let text = "帮我修个bug";
        assert!(
            !should_auto_decompose(&cfg, text),
            "简短修复诉求不应触发拆解"
        );
    }

    #[test]
    fn publish_plan_attaches_session_identity() {
        let bus = Arc::new(EventBus::default());
        let session = "ws.plan-test";
        let mut rx = bus.subscribe(&format!("{session}.*"));
        publish_plan(
            &bus,
            session,
            EventType::PlanRevision,
            json!({ "plan_revision": { "revisionId": "rev-1", "planId": "plan-1", "title": "t", "markdown": "m", "steps": [], "current": true } }),
        );
        let env = rx.try_recv().expect("应收到事件");
        assert_eq!(env.r#type, EventType::PlanRevision);
        assert_eq!(env.payload["session_key"], "ws.plan-test");
        assert_eq!(env.payload["key"], "ws.plan-test");
    }

    #[tokio::test]
    async fn run_autonomous_rejects_simple_task() {
        let cfg = Config::default();
        let bus = Arc::new(EventBus::default());
        let err = run_autonomous(
            cfg,
            "简单问题".to_string(),
            "ws.x".to_string(),
            bus,
            String::new(),
        )
        .await
        .expect_err("简单任务应被拒绝");
        assert!(err.to_string().contains("仅用于复杂/项目型任务"));
    }
}
