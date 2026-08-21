//! CLI 调用链集成（A2.6）
//!
//! `marguerite -p "修复 xxx" -f src/main.rs` 走真实调用链：
//! 意图分类 → 模板解析 → 链执行（内置文件工具 + 内置技能）→ 结果摘要。

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use super::builtin::register_builtin_tools;
use super::executor::{ChainExecutor, StepStatus};
use super::schema::{ChainContext, ChainPlan};
use crate::agent::subagent::SubAgentManager;
use crate::intent::classifier::Classifier;
use crate::mcp::registry::ToolRegistry;
use crate::mode::{ModeKind, ModeManager};
use crate::model::router::ModelRouter;
use crate::persistence::config::Config;
use crate::skills::engine::SkillEngine;
use crate::skills::registry::{Skill, SkillRegistry, SkillSource};

/// 内置技能（本地 prompt 模板，无需 LLM）
fn builtin_skill(name: &str, description: &str) -> Skill {
    Skill {
        name: name.to_string(),
        description: description.to_string(),
        version: "1.0.0".to_string(),
        author: None,
        intent: vec![name.to_string()],
        tools: vec![],
        prompt_template: format!("【{}】请基于提供的输入生成专业内容。", name),
        dependencies: vec![],
        source: SkillSource::Builtin,
        rating: 0.0,
        uses: 0,
        rank: crate::skills::registry::ranks::SYSTEM,
    }
}

/// 单指令走调用链：有对应模板则执行并返回结果文本；否则返回 None（回退 Agent）
pub async fn run_direct_chain(
    config: &Config,
    prompt: &str,
    file: Option<&Path>,
    agent: Option<&str>,
) -> Option<String> {
    let intent = Classifier::new().classify(prompt);
    let template = ChainExecutor::template_for_intent(&intent.intent)?;

    // 打开用量台账（CLI 路径不经网关 init），让技能/链的真实调用被记录（F5 衔接）。
    crate::usage::init();

    let mut registry = ToolRegistry::new();
    register_builtin_tools(&mut registry);
    let tools = Arc::new(registry);

    let mut skill_registry = SkillRegistry::new();
    for (name, desc) in [
        ("explain", "代码解释"),
        ("docgen", "文档生成"),
        ("commit", "提交信息生成"),
    ] {
        let _ = skill_registry.register(builtin_skill(name, desc));
    }
    // 按运行时配置构建技能引擎：配置了模型密钥则真实调 LLM 执行技能（F1 修复），
    // 否则退化为离线渲染（不联网、不报假）。
    let skills = Arc::new(SkillEngine::with_config(
        Arc::new(skill_registry),
        Arc::clone(&tools),
        config,
    ));

    let router = Arc::new(ModelRouter::new(config));
    let agents = Arc::new(SubAgentManager::new(
        Default::default(),
        Arc::clone(&tools),
        router,
    ));

    let mut mode = ModeManager::new();
    // CLI 单指令 = 一次性授权语义 → bloom（写操作不再逐条审批）
    let _ = mode.switch(ModeKind::Bloom);
    let mut executor = ChainExecutor::new(tools, skills, agents, Arc::new(mode), None);

    // 生产人设接线（关闭 F7 C 类待办根）：CLI 显式 --agent <id> 时，从该智能体台账取人设，
    // 注入技能执行（自定义性格 + 技能组合在调用链中真实生效，而非仅聊天层）。
    if let Some(id) = agent {
        if let Some(persona) = crate::agent::store::personality_for(id) {
            if !persona.trim().is_empty() {
                executor.set_persona(Some(persona));
            }
        }
    }

    let mut vars: HashMap<String, serde_json::Value> = HashMap::new();
    if let Some(f) = file {
        vars.insert("file".into(), serde_json::json!(f.display().to_string()));
    }
    vars.insert("description".into(), serde_json::json!(prompt));
    vars.insert("query".into(), serde_json::json!(prompt));
    vars.insert("instruction".into(), serde_json::json!(prompt));

    let plan = ChainPlan::from_template("cli-chain", template, vars)?;
    let mut ctx = ChainContext::new(plan.context.clone());
    let outcome = executor.execute(plan, &mut ctx).await.ok()?;
    Some(format_outcome(&outcome))
}

/// 格式化链执行结果
fn format_outcome(outcome: &crate::chain::ChainOutcome) -> String {
    let mut lines = vec![format!(
        "调用链 [{}] 执行{}",
        outcome.chain_id,
        if outcome.succeeded() {
            "成功"
        } else {
            "完成（含失败步骤）"
        }
    )];
    for s in &outcome.steps {
        let mark = match s.status {
            StepStatus::Succeeded => "✓",
            StepStatus::Failed => "✗",
            StepStatus::Skipped => "↪",
        };
        let summary = if s.summary.is_empty() {
            String::new()
        } else {
            format!("（{}）", truncate(&s.summary, 100))
        };
        let error = s
            .error
            .as_deref()
            .map(|e| format!("：{e}"))
            .unwrap_or_default();
        lines.push(format!("  {mark} {}{}{}", s.step_id, summary, error));
    }
    if let Some(report) = &outcome.guard_report {
        let issues = if report.issues.is_empty() {
            "无问题".to_string()
        } else {
            report
                .issues
                .iter()
                .map(|i| format!("{:?}: {}", i.severity, i.message))
                .collect::<Vec<_>>()
                .join("；")
        };
        lines.push(format!(
            "  护栏：{}（{}）",
            if outcome.repaired {
                "已自动修复"
            } else {
                "检查通过"
            },
            issues
        ));
    }
    lines.join("\n")
}

fn truncate(s: &str, max: usize) -> String {
    let mut out: String = s.chars().take(max).collect();
    if s.chars().count() > max {
        out.push('…');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_file(name: &str, content: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(name);
        std::fs::write(&path, content).unwrap();
        path
    }

    #[tokio::test]
    async fn test_run_direct_chain_explain_succeeds() {
        let tmp = temp_file(
            "marguerite_chain_cli_test.rs",
            "fn main() { println!(\"hi\"); }\n",
        );
        let config = Config::default();
        let out = run_direct_chain(&config, "解释这段代码的含义", Some(&tmp), None)
            .await
            .expect("explain 意图应有链输出");
        assert!(out.contains("调用链"));
        assert!(out.contains("s1"));
        assert!(out.contains("s2"));
        assert!(!out.contains("✗"), "explain 链应全部成功: {out}");
        let _ = std::fs::remove_file(&tmp);
    }

    #[tokio::test]
    async fn test_run_direct_chain_unknown_intent_falls_back() {
        let config = Config::default();
        // 无对应模板 → 返回 None（回退 AgentRuntime）
        assert!(run_direct_chain(&config, "今天天气怎么样", None, None)
            .await
            .is_none());
    }

    #[tokio::test]
    async fn test_run_direct_chain_search() {
        let tmp = temp_file("marguerite_chain_cli_search.rs", "fn find_target() {}\n");
        let config = Config::default();
        let out = run_direct_chain(&config, "搜索 find_target 在哪里", Some(&tmp), None)
            .await
            .expect("search 意图应有链输出");
        assert!(out.contains("调用链"));
        let _ = std::fs::remove_file(&tmp);
    }

    #[tokio::test]
    async fn test_run_direct_chain_injects_builtin_agent_persona() {
        // 生产接线点回归：--agent <id> 必须把该智能体人设注入技能执行输出（关闭 F7 C 类待办根）。
        // 用自定义智能体做密封测试，避免依赖全局台账里内置智能体是否被 seed（机器相关、非密封）。
        use crate::agent::store as agent_store;
        let probe_id = "f7b-persona-probe";
        let _ = agent_store::delete(probe_id); // 清理可能残留，保证幂等
        let _ = agent_store::create(&serde_json::json!({
            "id": probe_id,
            "name": "F7B探针",
            "system_prompt": "F7B_PROBE_PERSONA_你以量子视角审视代码",
        }));

        let tmp = temp_file("marguerite_chain_cli_persona.rs", "fn find_target() {}\n");
        let config = Config::default();
        let out = run_direct_chain(&config, "解释这段代码", Some(&tmp), Some(probe_id))
            .await
            .expect("explain 意图应有链输出");
        let _ = agent_store::delete(probe_id);
        assert!(
            out.contains("F7B_PROBE_PERSONA_"),
            "--agent 人设未注入技能执行输出: {out}"
        );
        let _ = std::fs::remove_file(&tmp);
    }
}
