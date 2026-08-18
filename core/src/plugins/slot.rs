//! Plugin Slot：统一工具（Tool）与技能（Skill）注册槽
//!
//! 参考 opencode 的 Plugin Slot 概念：tool 和 skill 不再是两套独立系统，
//! 而是统一的「能力槽」——每个槽声明自己的类型、名称、描述、参数 schema，
//! 运行时按类型路由到对应的执行器（ToolRegistry / SkillLoader）。
//!
//! 优势：
//! - 统一发现：`list_slots()` 一次返回所有可用能力（工具 + 技能）
//! - 统一调用：`invoke(name, args)` 自动路由
//! - 插件可同时提供工具和技能
//! - 为 MCP/ACP 双协议接入预留统一入口

use std::collections::HashMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// 槽位类型
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum SlotKind {
    /// 可执行工具（有副作用，如 shell/fs/computer_use）
    Tool,
    /// 技能（prompt 模板 + 可选工具链，无直接副作用）
    Skill,
    /// MCP 外部工具
    McpTool,
    /// 内置命令（CLI/TUI 命令）
    Command,
}

/// 槽位描述符
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SlotDescriptor {
    /// 唯一名称（如 "shell"、"skill:fix"、"mcp:github.search"）
    pub name: String,
    /// 槽位类型
    pub kind: SlotKind,
    /// 人类可读描述
    pub description: String,
    /// JSON Schema 参数（工具）或 intent 关键词（技能）
    pub schema: Value,
    /// 来源插件名（None = 内置）
    pub source: Option<String>,
    /// 是否需要用户确认（PlanGate 集成）
    pub requires_approval: bool,
}

/// 槽位执行结果
#[derive(Debug, Clone)]
pub enum SlotResult {
    /// 文本输出
    Text(String),
    /// JSON 输出
    Json(Value),
    /// 流式输出（chunk）
    Stream(String),
}

/// 槽位执行器 trait：工具/技能各自实现
pub trait SlotExecutor: Send + Sync {
    /// 执行槽位
    fn execute(&self, name: &str, args: &HashMap<String, String>) -> Result<SlotResult, String>;
}

/// 统一插件槽注册表
pub struct PluginSlot {
    slots: HashMap<String, SlotDescriptor>,
    executors: HashMap<String, Arc<dyn SlotExecutor>>,
}

impl Default for PluginSlot {
    fn default() -> Self {
        Self::new()
    }
}

impl PluginSlot {
    pub fn new() -> Self {
        Self {
            slots: HashMap::new(),
            executors: HashMap::new(),
        }
    }

    /// 注册一个槽位
    pub fn register(&mut self, descriptor: SlotDescriptor, executor: Arc<dyn SlotExecutor>) {
        let name = descriptor.name.clone();
        self.slots.insert(name.clone(), descriptor);
        self.executors.insert(name, executor);
    }

    /// 注册工具槽（便捷方法）
    pub fn register_tool(
        &mut self,
        name: &str,
        description: &str,
        schema: Value,
        executor: Arc<dyn SlotExecutor>,
    ) {
        self.register(
            SlotDescriptor {
                name: name.to_string(),
                kind: SlotKind::Tool,
                description: description.to_string(),
                schema,
                source: None,
                requires_approval: false,
            },
            executor,
        );
    }

    /// 注册技能槽（便捷方法）
    pub fn register_skill(
        &mut self,
        name: &str,
        description: &str,
        intents: Vec<String>,
        executor: Arc<dyn SlotExecutor>,
    ) {
        self.register(
            SlotDescriptor {
                name: format!("skill:{name}"),
                kind: SlotKind::Skill,
                description: description.to_string(),
                schema: serde_json::json!({ "intents": intents }),
                source: None,
                requires_approval: false,
            },
            executor,
        );
    }

    /// 注销槽位
    pub fn unregister(&mut self, name: &str) -> bool {
        self.slots.remove(name).is_some() && self.executors.remove(name).is_some()
    }

    /// 获取槽位描述
    pub fn get(&self, name: &str) -> Option<&SlotDescriptor> {
        self.slots.get(name)
    }

    /// 列出所有槽位
    pub fn list(&self) -> Vec<&SlotDescriptor> {
        let mut slots: Vec<&SlotDescriptor> = self.slots.values().collect();
        slots.sort_by(|a, b| a.name.cmp(&b.name));
        slots
    }

    /// 按类型列出槽位
    pub fn list_by_kind(&self, kind: &SlotKind) -> Vec<&SlotDescriptor> {
        self.slots.values().filter(|s| &s.kind == kind).collect()
    }

    /// 按名称前缀搜索槽位
    pub fn search(&self, query: &str) -> Vec<&SlotDescriptor> {
        let q = query.to_lowercase();
        self.slots
            .values()
            .filter(|s| {
                s.name.to_lowercase().contains(&q) || s.description.to_lowercase().contains(&q)
            })
            .collect()
    }

    /// 执行槽位
    pub fn invoke(&self, name: &str, args: &HashMap<String, String>) -> Result<SlotResult, String> {
        let executor = self
            .executors
            .get(name)
            .ok_or_else(|| format!("槽位 `{name}` 不存在"))?;
        executor.execute(name, args)
    }

    /// 槽位总数
    pub fn len(&self) -> usize {
        self.slots.len()
    }

    /// 是否为空
    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }

    /// 检查槽位是否存在
    pub fn contains(&self, name: &str) -> bool {
        self.slots.contains_key(name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct EchoExecutor;

    impl SlotExecutor for EchoExecutor {
        fn execute(
            &self,
            name: &str,
            args: &HashMap<String, String>,
        ) -> Result<SlotResult, String> {
            let text = args.get("text").cloned().unwrap_or_default();
            Ok(SlotResult::Text(format!("{name}: {text}")))
        }
    }

    #[test]
    fn test_register_and_invoke_tool() {
        let mut slot = PluginSlot::new();
        slot.register_tool(
            "echo",
            "回声工具",
            serde_json::json!({"type": "object", "properties": {"text": {"type": "string"}}}),
            Arc::new(EchoExecutor),
        );
        assert_eq!(slot.len(), 1);
        assert!(slot.contains("echo"));

        let mut args = HashMap::new();
        args.insert("text".to_string(), "hello".to_string());
        let result = slot.invoke("echo", &args).unwrap();
        match result {
            SlotResult::Text(s) => assert_eq!(s, "echo: hello"),
            _ => panic!("expected text result"),
        }
    }

    #[test]
    fn test_register_and_invoke_skill() {
        let mut slot = PluginSlot::new();
        slot.register_skill(
            "fix",
            "修复代码",
            vec!["fix".to_string(), "修复".to_string()],
            Arc::new(EchoExecutor),
        );
        assert!(slot.contains("skill:fix"));
        let skills = slot.list_by_kind(&SlotKind::Skill);
        assert_eq!(skills.len(), 1);

        let mut args = HashMap::new();
        args.insert("text".to_string(), "bug".to_string());
        let result = slot.invoke("skill:fix", &args).unwrap();
        match result {
            SlotResult::Text(s) => assert_eq!(s, "skill:fix: bug"),
            _ => panic!("expected text result"),
        }
    }

    #[test]
    fn test_search_slots() {
        let mut slot = PluginSlot::new();
        slot.register_tool("shell", "执行命令", Value::Null, Arc::new(EchoExecutor));
        slot.register_tool("fs_read", "读取文件", Value::Null, Arc::new(EchoExecutor));
        slot.register_skill("fix", "修复", vec![], Arc::new(EchoExecutor));

        let results = slot.search("文件");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "fs_read");

        let results = slot.search("fix");
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn test_unregister() {
        let mut slot = PluginSlot::new();
        slot.register_tool("temp", "临时", Value::Null, Arc::new(EchoExecutor));
        assert!(slot.unregister("temp"));
        assert!(!slot.contains("temp"));
        assert!(!slot.unregister("temp"));
    }

    #[test]
    fn test_invoke_unknown() {
        let slot = PluginSlot::new();
        let result = slot.invoke("nope", &HashMap::new());
        assert!(result.is_err());
    }
}
