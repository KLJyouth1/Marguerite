//! 调用链数据结构（v2 工作台）
//!
//! 意图 → 有序/并行步骤链（tool/skill/expert/guard），
//! 每步支持超时、重试、fallback、审批标记。

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// 步骤类型
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChainStepType {
    /// 工具调用
    Tool,
    /// 技能执行
    Skill,
    /// 子代理（专家）
    Expert,
    /// 护栏检查
    Guard,
}

/// 执行模式
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ExecutionMode {
    /// 顺序执行
    #[default]
    Sequential,
    /// 并行执行
    Parallel,
}

fn default_retries() -> u32 {
    1
}

fn default_timeout() -> u64 {
    60
}

/// 单个调用链步骤
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChainStep {
    /// 步骤 ID（供 `{{sN.output}}` 引用）
    pub step_id: String,
    /// 步骤类型
    pub step_type: ChainStepType,
    /// 目标（工具名 / 技能名 / 专家 profile / guard 层）
    pub target: String,
    /// 参数（支持 `$var` 与 `{{sN.output}}` 插值）
    #[serde(default)]
    pub params: HashMap<String, serde_json::Value>,
    /// 最大重试次数
    #[serde(default = "default_retries")]
    pub max_retries: u32,
    /// 步骤超时（秒）
    #[serde(default = "default_timeout")]
    pub timeout_sec: u64,
    /// 失败时的回退步骤
    pub fallback: Option<Box<ChainStep>>,
    /// 是否需要审批
    #[serde(default)]
    pub requires_approval: bool,
    /// 输出是否需要过护栏（链后汇总验证）
    #[serde(default)]
    pub guard: bool,
}

/// 调用链计划
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChainPlan {
    /// 链 ID
    pub chain_id: String,
    /// 意图名（模板名）
    pub intent: String,
    /// 步骤列表
    pub steps: Vec<ChainStep>,
    /// 执行模式
    #[serde(default)]
    pub execution_mode: ExecutionMode,
    /// 链上下文初始变量（如 `{"file": "src/main.rs"}`）
    #[serde(default)]
    pub context: HashMap<String, serde_json::Value>,
}

impl ChainPlan {
    /// 创建空链（用于 question/chat 等无步骤意图）
    pub fn empty(chain_id: &str, intent: &str) -> Self {
        Self {
            chain_id: chain_id.to_string(),
            intent: intent.to_string(),
            steps: Vec::new(),
            execution_mode: ExecutionMode::Sequential,
            context: HashMap::new(),
        }
    }

    /// 从模板构建（模板名 + 初始变量）
    pub fn from_template(
        chain_id: &str,
        template: &str,
        vars: HashMap<String, serde_json::Value>,
    ) -> Option<Self> {
        let steps = crate::chain::templates::resolve(template)?;
        Some(Self {
            chain_id: chain_id.to_string(),
            intent: template.to_string(),
            steps,
            execution_mode: ExecutionMode::Sequential,
            context: vars,
        })
    }
}

/// 步骤输出
#[derive(Debug, Clone)]
pub struct StepOutput {
    /// 步骤返回值
    pub value: serde_json::Value,
    /// 错误信息（失败时）
    pub error: Option<String>,
}

impl StepOutput {
    /// 创建成功输出
    pub fn success(value: serde_json::Value) -> Self {
        Self { value, error: None }
    }

    /// 创建失败输出
    pub fn failure(error: impl Into<String>) -> Self {
        Self {
            value: serde_json::Value::Null,
            error: Some(error.into()),
        }
    }
}

/// 链上下文（变量 + 步骤输出，支持插值）
#[derive(Debug, Clone, Default)]
pub struct ChainContext {
    /// 初始变量（`$file` 等）
    pub vars: HashMap<String, serde_json::Value>,
    /// 步骤输出（`{{sN.output}}` / `{{sN.error}}`）
    pub outputs: HashMap<String, StepOutput>,
}

impl ChainContext {
    /// 创建新上下文
    pub fn new(vars: HashMap<String, serde_json::Value>) -> Self {
        Self {
            vars,
            outputs: HashMap::new(),
        }
    }

    /// 字符串插值：`$var` → vars；`{{sN.output}}` / `{{sN.error}}` → 步骤结果
    pub fn interpolate_str(&self, s: &str) -> String {
        let mut out = s.to_string();
        for (id, output) in &self.outputs {
            let key_out = format!("{{{{{}.output}}}}", id);
            let key_err = format!("{{{{{}.error}}}}", id);
            match (&output.error, &output.value) {
                (Some(e), _) => {
                    out = out.replace(&key_out, e);
                    out = out.replace(&key_err, e);
                }
                (None, v) => {
                    let repl = json_to_plain(v);
                    out = out.replace(&key_out, &repl);
                    out = out.replace(&key_err, "");
                }
            }
        }
        for (k, v) in &self.vars {
            let key = format!("${k}");
            out = out.replace(&key, &json_to_plain(v));
        }
        out
    }

    /// JSON 值插值（递归替换字符串字段）
    pub fn interpolate_value(&self, v: &serde_json::Value) -> serde_json::Value {
        match v {
            serde_json::Value::String(s) => serde_json::Value::String(self.interpolate_str(s)),
            serde_json::Value::Array(arr) => {
                serde_json::Value::Array(arr.iter().map(|x| self.interpolate_value(x)).collect())
            }
            serde_json::Value::Object(map) => serde_json::Value::Object(
                map.iter()
                    .map(|(k, val)| (k.clone(), self.interpolate_value(val)))
                    .collect(),
            ),
            other => other.clone(),
        }
    }
}

/// JSON 值转可插值文本：字符串取原值，其余取紧凑 JSON
fn json_to_plain(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_interpolate_vars_and_outputs() {
        let mut ctx = ChainContext::new(HashMap::from([(
            "file".into(),
            serde_json::json!("src/main.rs"),
        )]));
        ctx.outputs.insert(
            "s1".into(),
            StepOutput::success(serde_json::json!({"text": "hello"})),
        );

        assert_eq!(ctx.interpolate_str("$file"), "src/main.rs");
        assert_eq!(ctx.interpolate_str("{{s1.output}}"), r#"{"text":"hello"}"#);
        assert_eq!(ctx.interpolate_str("{{s1.error}}"), "");
    }

    #[test]
    fn test_interpolate_failure_output() {
        let mut ctx = ChainContext::new(HashMap::new());
        ctx.outputs.insert("s1".into(), StepOutput::failure("boom"));
        assert_eq!(ctx.interpolate_str("{{s1.output}}"), "boom");
        assert_eq!(ctx.interpolate_str("{{s1.error}}"), "boom");
    }

    #[test]
    fn test_interpolate_value_recursive() {
        let mut ctx =
            ChainContext::new(HashMap::from([("file".into(), serde_json::json!("a.rs"))]));
        ctx.outputs
            .insert("s1".into(), StepOutput::success(serde_json::json!("X")));
        let v = serde_json::json!({
            "path": "$file",
            "list": ["{{s1.output}}", "fixed"],
            "nested": {"code": "{{s1.output}}"}
        });
        let out = ctx.interpolate_value(&v);
        assert_eq!(out["path"], "a.rs");
        assert_eq!(out["list"][0], "X");
        assert_eq!(out["nested"]["code"], "X");
    }

    #[test]
    fn test_plan_from_template_and_empty() {
        let plan = ChainPlan::from_template(
            "c1",
            "explain",
            HashMap::from([("file".into(), serde_json::json!("main.rs"))]),
        )
        .unwrap();
        assert_eq!(plan.intent, "explain");
        assert_eq!(plan.steps.len(), 2);
        assert_eq!(plan.context["file"], "main.rs");

        let empty = ChainPlan::empty("c0", "chat");
        assert!(empty.steps.is_empty());
    }
}
