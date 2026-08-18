//! 内置链模板（v2 工作台）
//!
//! 每个模板把一类用户目标拆解为有序/并行步骤链。
//! 参数支持 `$var`（链初始变量）与 `{{sN.output}}`（前序步骤输出）插值。

use std::collections::HashMap;

use super::schema::{ChainStep, ChainStepType};

/// 构造宏：HashMap<String, serde_json::Value> 字面量
macro_rules! mk_params {
    ($($k:expr => $v:expr),* $(,)?) => {{
        let mut m: HashMap<String, serde_json::Value> = HashMap::new();
        $( m.insert($k.to_string(), serde_json::json!($v)); )*
        m
    }};
}

/// 内置模板名（与意图分类器一一对应）
pub const TEMPLATE_NAMES: &[&str] = &[
    "explain", "refactor", "fix", "test", "doc", "commit", "search",
];

/// 解析模板名 → 步骤链
pub fn resolve(name: &str) -> Option<Vec<ChainStep>> {
    match name {
        "explain" => Some(explain()),
        "refactor" => Some(refactor()),
        "fix" => Some(fix()),
        "test" => Some(test()),
        "doc" => Some(doc()),
        "commit" => Some(commit()),
        "search" => Some(search()),
        _ => None,
    }
}

/// 是否支持该模板
pub fn is_supported(name: &str) -> bool {
    TEMPLATE_NAMES.contains(&name)
}

/// 列出所有模板名
pub fn template_names() -> Vec<&'static str> {
    TEMPLATE_NAMES.to_vec()
}

/// explain：读文件 → 解释技能
fn explain() -> Vec<ChainStep> {
    vec![
        step(
            "s1",
            ChainStepType::Tool,
            "read_file",
            mk_params! {"path" => "$file"},
        ),
        step(
            "s2",
            ChainStepType::Skill,
            "explain",
            mk_params! {"code" => "{{s1.output}}", "lang" => "$lang"},
        ),
    ]
}

/// refactor：读文件 → 重构专家（输出过护栏）→ 写回（审批）→ 语法护栏 → 运行测试（失败回退调试）
fn refactor() -> Vec<ChainStep> {
    vec![
        step(
            "s1",
            ChainStepType::Tool,
            "read_file",
            mk_params! {"path" => "$file"},
        ),
        step_guard(
            "s2",
            ChainStepType::Expert,
            "refactorer",
            mk_params! {"code" => "{{s1.output}}", "instruction" => "$instruction"},
        ),
        step_approval(
            "s3",
            ChainStepType::Tool,
            "edit_file",
            mk_params! {"path" => "$file", "content" => "{{s2.output}}"},
        ),
        step(
            "s4",
            ChainStepType::Guard,
            "syntax",
            mk_params! {"code" => "{{s3.output}}", "lang" => "$lang"},
        ),
        step_fallback(
            "s5",
            ChainStepType::Tool,
            "run_tests",
            mk_params! {"file" => "$file"},
            step(
                "s6",
                ChainStepType::Tool,
                "debug",
                mk_params! {"file" => "$file"},
            ),
        ),
    ]
}

/// fix：读文件 → 语义检查 → 修复专家 → 写回（审批）→ 语法护栏 → 运行测试（失败回退调试）
fn fix() -> Vec<ChainStep> {
    vec![
        step(
            "s1",
            ChainStepType::Tool,
            "read_file",
            mk_params! {"path" => "$file"},
        ),
        step(
            "s2",
            ChainStepType::Guard,
            "semantic",
            mk_params! {"code" => "{{s1.output}}", "lang" => "$lang"},
        ),
        step_guard(
            "s3",
            ChainStepType::Expert,
            "fixer",
            mk_params! {"code" => "{{s1.output}}", "bug" => "$description"},
        ),
        step_approval(
            "s4",
            ChainStepType::Tool,
            "edit_file",
            mk_params! {"path" => "$file", "content" => "{{s3.output}}"},
        ),
        step(
            "s5",
            ChainStepType::Guard,
            "syntax",
            mk_params! {"code" => "{{s4.output}}", "lang" => "$lang"},
        ),
        step_fallback(
            "s6",
            ChainStepType::Tool,
            "run_tests",
            mk_params! {"file" => "$file"},
            step(
                "s7",
                ChainStepType::Tool,
                "debug",
                mk_params! {"file" => "$file"},
            ),
        ),
    ]
}

/// test：读文件 → 测试生成专家
fn test() -> Vec<ChainStep> {
    vec![
        step(
            "s1",
            ChainStepType::Tool,
            "read_file",
            mk_params! {"path" => "$file"},
        ),
        step(
            "s2",
            ChainStepType::Expert,
            "testgen",
            mk_params! {"code" => "{{s1.output}}", "coverage" => "$coverage"},
        ),
    ]
}

/// doc：读文件 → 文档生成技能
fn doc() -> Vec<ChainStep> {
    vec![
        step(
            "s1",
            ChainStepType::Tool,
            "read_file",
            mk_params! {"path" => "$file"},
        ),
        step(
            "s2",
            ChainStepType::Skill,
            "docgen",
            mk_params! {"code" => "{{s1.output}}"},
        ),
    ]
}

/// commit：读差异 → 提交信息技能
fn commit() -> Vec<ChainStep> {
    vec![
        step(
            "s1",
            ChainStepType::Tool,
            "read_file",
            mk_params! {"path" => "$file"},
        ),
        step(
            "s2",
            ChainStepType::Skill,
            "commit",
            mk_params! {"diff" => "{{s1.output}}"},
        ),
    ]
}

/// search：代码搜索（单工具链）
fn search() -> Vec<ChainStep> {
    vec![step(
        "s1",
        ChainStepType::Tool,
        "search_code",
        mk_params! {"query" => "$query", "path" => "$file"},
    )]
}

fn step(
    id: &str,
    step_type: ChainStepType,
    target: &str,
    params: HashMap<String, serde_json::Value>,
) -> ChainStep {
    ChainStep {
        step_id: id.to_string(),
        step_type,
        target: target.to_string(),
        params,
        max_retries: 1,
        timeout_sec: 60,
        fallback: None,
        requires_approval: false,
        guard: false,
    }
}

fn step_approval(
    id: &str,
    step_type: ChainStepType,
    target: &str,
    params: HashMap<String, serde_json::Value>,
) -> ChainStep {
    let mut s = step(id, step_type, target, params);
    s.requires_approval = true;
    s
}

fn step_guard(
    id: &str,
    step_type: ChainStepType,
    target: &str,
    params: HashMap<String, serde_json::Value>,
) -> ChainStep {
    let mut s = step(id, step_type, target, params);
    s.guard = true;
    s
}

fn step_fallback(
    id: &str,
    step_type: ChainStepType,
    target: &str,
    params: HashMap<String, serde_json::Value>,
    fallback: ChainStep,
) -> ChainStep {
    let mut s = step(id, step_type, target, params);
    s.fallback = Some(Box::new(fallback));
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_all_templates_resolvable() {
        for name in TEMPLATE_NAMES {
            let steps = resolve(name);
            assert!(steps.is_some(), "模板 {} 应可解析", name);
            let steps = steps.unwrap();
            assert!(!steps.is_empty(), "模板 {} 不应为空", name);
            // 步骤 ID 唯一
            let mut ids: Vec<&str> = steps.iter().map(|s| s.step_id.as_str()).collect();
            ids.sort_unstable();
            ids.dedup();
            assert_eq!(ids.len(), steps.len(), "模板 {} 步骤 ID 重复", name);
        }
    }

    #[test]
    fn test_unknown_template_none() {
        assert!(resolve("nonexistent").is_none());
        assert!(!is_supported("nonexistent"));
        assert_eq!(template_names().len(), TEMPLATE_NAMES.len());
    }

    #[test]
    fn test_explain_chain_shape() {
        let steps = resolve("explain").unwrap();
        assert_eq!(steps.len(), 2);
        assert_eq!(steps[0].step_type, ChainStepType::Tool);
        assert_eq!(steps[0].target, "read_file");
        assert_eq!(steps[1].step_type, ChainStepType::Skill);
        assert!(!steps[1].guard);
        assert!(!steps[1].requires_approval);
    }

    #[test]
    fn test_fix_chain_approval_and_fallback() {
        let steps = resolve("fix").unwrap();
        // 编辑步骤需审批
        let edit = steps.iter().find(|s| s.target == "edit_file").unwrap();
        assert!(edit.requires_approval);
        // 测试步骤带 fallback
        let tests = steps.iter().find(|s| s.target == "run_tests").unwrap();
        let fb = tests.fallback.as_deref().expect("run_tests 应有 fallback");
        assert_eq!(fb.target, "debug");
        // 含语法护栏步骤
        assert!(steps.iter().any(|s| s.step_type == ChainStepType::Guard));
    }

    #[test]
    fn test_refactor_uses_expert() {
        let steps = resolve("refactor").unwrap();
        assert!(steps.iter().any(|s| s.step_type == ChainStepType::Expert));
    }
}
