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
//!
//! ## 装配语义（P0 S4）
//!
//! - **append-only**：`register()` 遇重名**拒绝**（返回 `Err`），不再静默覆盖 ——
//!   静默覆盖会让「后来的定义悄悄顶掉先前定义」表现为一切正常，是最隐蔽的失效。
//! - **`order`**：槽位带排序权重，`list()` 按 `(order, name)` 排序，因此 patch
//!   声明插入的条目能落在确定的相对位置。
//! - **`disabled`**：被停用的槽位不再出现在 `list()`，且 `invoke()` 显式报错，
//!   绝不静默可用。

use std::collections::{BTreeSet, HashMap};
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
    /// 展示/装配排序权重（小者先；缺省 0）
    #[serde(default)]
    pub order: i32,
    /// 声明该槽位来源的逻辑槽（patch 的 `Insert.slot`）；内置槽为 `None`
    #[serde(default)]
    pub declared_slot: Option<String>,
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
    /// 已停用槽位名（停用项保留在 `slots` 中，仅从 `list()` 隐藏且禁止 `invoke`）
    disabled: BTreeSet<String>,
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
            disabled: BTreeSet::new(),
        }
    }

    /// 注册一个槽位（**append-only**）：重名返回 `Err`，绝不覆盖已有定义。
    pub fn register(
        &mut self,
        descriptor: SlotDescriptor,
        executor: Arc<dyn SlotExecutor>,
    ) -> Result<(), String> {
        let name = descriptor.name.clone();
        if self.slots.contains_key(&name) {
            return Err(format!("槽位 `{name}` 已注册，拒绝覆盖（append-only）"));
        }
        self.slots.insert(name.clone(), descriptor);
        self.executors.insert(name, executor);
        Ok(())
    }

    /// 注册工具槽（便捷方法）
    pub fn register_tool(
        &mut self,
        name: &str,
        description: &str,
        schema: Value,
        executor: Arc<dyn SlotExecutor>,
    ) -> Result<(), String> {
        self.register(
            SlotDescriptor {
                name: name.to_string(),
                kind: SlotKind::Tool,
                description: description.to_string(),
                schema,
                source: None,
                requires_approval: false,
                order: 0,
                declared_slot: None,
            },
            executor,
        )
    }

    /// 注册技能槽（便捷方法）
    pub fn register_skill(
        &mut self,
        name: &str,
        description: &str,
        intents: Vec<String>,
        executor: Arc<dyn SlotExecutor>,
    ) -> Result<(), String> {
        self.register(
            SlotDescriptor {
                name: format!("skill:{name}"),
                kind: SlotKind::Skill,
                description: description.to_string(),
                schema: serde_json::json!({ "intents": intents }),
                source: None,
                requires_approval: false,
                order: 0,
                declared_slot: None,
            },
            executor,
        )
    }

    /// 注销槽位
    pub fn unregister(&mut self, name: &str) -> bool {
        let removed = self.slots.remove(name).is_some() && self.executors.remove(name).is_some();
        // 同步清理停用标记，避免「注销后同名重注册仍是停用态」的残留。
        self.disabled.remove(name);
        removed
    }

    /// 获取槽位描述
    pub fn get(&self, name: &str) -> Option<&SlotDescriptor> {
        self.slots.get(name)
    }

    /// 列出所有**可用**（未停用）槽位，按 `(order, name)` 排序
    pub fn list(&self) -> Vec<&SlotDescriptor> {
        self.list_all()
            .into_iter()
            .filter(|s| !self.disabled.contains(&s.name))
            .collect()
    }

    /// 列出全部槽位（**含被停用项**），按 `(order, name)` 排序
    pub fn list_all(&self) -> Vec<&SlotDescriptor> {
        let mut slots: Vec<&SlotDescriptor> = self.slots.values().collect();
        slots.sort_by(|a, b| a.order.cmp(&b.order).then_with(|| a.name.cmp(&b.name)));
        slots
    }

    /// 该槽位是否已被停用（不存在亦返回 `false`）
    pub fn is_disabled(&self, name: &str) -> bool {
        self.disabled.contains(name)
    }

    /// 停用槽位：槽位存在则置为停用并返回 `true`；**不存在返回 `false`**。
    pub fn disable(&mut self, name: &str) -> bool {
        if self.slots.contains_key(name) {
            self.disabled.insert(name.to_string());
            true
        } else {
            false
        }
    }

    /// 按类型列出**可用**（未停用）槽位（与 `list()` 同口径：停用项不出现）
    pub fn list_by_kind(&self, kind: &SlotKind) -> Vec<&SlotDescriptor> {
        self.slots
            .values()
            .filter(|s| &s.kind == kind && !self.disabled.contains(&s.name))
            .collect()
    }

    /// 按名称/描述搜索**可用**（未停用）槽位（与 `list()` 同口径：停用项不出现）
    pub fn search(&self, query: &str) -> Vec<&SlotDescriptor> {
        let q = query.to_lowercase();
        self.slots
            .values()
            .filter(|s| {
                !self.disabled.contains(&s.name)
                    && (s.name.to_lowercase().contains(&q)
                        || s.description.to_lowercase().contains(&q))
            })
            .collect()
    }

    /// 执行槽位（被停用的槽位**显式报错**，不静默可用）
    pub fn invoke(&self, name: &str, args: &HashMap<String, String>) -> Result<SlotResult, String> {
        if self.disabled.contains(name) {
            return Err(format!("槽位 `{name}` 已被停用，不可调用"));
        }
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

    /// 构造带 `order` 的工具槽描述符（测试用）。
    fn descriptor(name: &str, order: i32) -> SlotDescriptor {
        SlotDescriptor {
            name: name.to_string(),
            kind: SlotKind::Tool,
            description: name.to_string(),
            schema: Value::Null,
            source: None,
            requires_approval: false,
            order,
            declared_slot: None,
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
        )
        .unwrap();
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
        )
        .unwrap();
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
        slot.register_tool("shell", "执行命令", Value::Null, Arc::new(EchoExecutor))
            .unwrap();
        slot.register_tool("fs_read", "读取文件", Value::Null, Arc::new(EchoExecutor))
            .unwrap();
        slot.register_skill("fix", "修复", vec![], Arc::new(EchoExecutor))
            .unwrap();

        let results = slot.search("文件");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "fs_read");

        let results = slot.search("fix");
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn test_unregister() {
        let mut slot = PluginSlot::new();
        slot.register_tool("temp", "临时", Value::Null, Arc::new(EchoExecutor))
            .unwrap();
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

    #[test]
    fn duplicate_register_is_rejected_and_keeps_original() {
        let mut slot = PluginSlot::new();
        slot.register(descriptor("echo", 0), Arc::new(EchoExecutor))
            .unwrap();
        // 换一个描述再注册同名槽：必须被拒，且**原项不被覆盖**。
        let mut replacement = descriptor("echo", 99);
        replacement.description = "被拒绝的替代品".to_string();
        let err = slot
            .register(replacement, Arc::new(EchoExecutor))
            .expect_err("重名注册必须返回 Err");
        assert!(err.contains("echo"), "错误信息须含槽位名，实际: {err}");
        assert_eq!(slot.len(), 1, "重名被拒后槽位数不得增加");
        assert_eq!(
            slot.get("echo").unwrap().description,
            "echo",
            "重名被拒后原描述必须保持"
        );
        assert_eq!(slot.get("echo").unwrap().order, 0, "原 order 必须保持");
    }

    #[test]
    fn list_sorts_by_order_then_name() {
        let mut slot = PluginSlot::new();
        slot.register(descriptor("zzz", 5), Arc::new(EchoExecutor))
            .unwrap();
        slot.register(descriptor("beta", 1), Arc::new(EchoExecutor))
            .unwrap();
        slot.register(descriptor("alpha", 1), Arc::new(EchoExecutor))
            .unwrap();

        let names: Vec<&str> = slot.list().iter().map(|s| s.name.as_str()).collect();
        // order 升序为主序；同 order 按 name 字典序
        assert_eq!(names, vec!["alpha", "beta", "zzz"]);
    }

    #[test]
    fn disable_hides_from_list_but_not_list_all() {
        let mut slot = PluginSlot::new();
        slot.register(descriptor("keep", 0), Arc::new(EchoExecutor))
            .unwrap();
        slot.register(descriptor("gone", 0), Arc::new(EchoExecutor))
            .unwrap();

        assert!(!slot.is_disabled("gone"));
        assert!(slot.disable("gone"), "停用存在的槽位应返回 true");
        assert!(slot.is_disabled("gone"));
        assert!(!slot.disable("nope"), "停用不存在的槽位应返回 false");

        let listed: Vec<&str> = slot.list().iter().map(|s| s.name.as_str()).collect();
        assert_eq!(listed, vec!["keep"], "list() 不应含被停用项");

        let all: Vec<&str> = slot.list_all().iter().map(|s| s.name.as_str()).collect();
        assert_eq!(all, vec!["gone", "keep"], "list_all() 应含被停用项");
    }

    #[test]
    fn disable_hides_from_list_by_kind_and_search() {
        let mut slot = PluginSlot::new();
        // 两个同类型技能，描述都含同一检索关键词；外加一个工具槽。
        slot.register_skill("keep", "保留技能", vec![], Arc::new(EchoExecutor))
            .unwrap();
        slot.register_skill("gone", "停用技能", vec![], Arc::new(EchoExecutor))
            .unwrap();
        slot.register(descriptor("plain", 0), Arc::new(EchoExecutor))
            .unwrap();

        assert!(slot.disable("skill:gone"));

        // 类型枚举：停用项不出现
        let skills: Vec<&str> = slot
            .list_by_kind(&SlotKind::Skill)
            .iter()
            .map(|s| s.name.as_str())
            .collect();
        assert_eq!(skills, vec!["skill:keep"], "list_by_kind() 不应含停用项");

        // 搜索（描述命中）：停用项不出现
        let by_desc: Vec<&str> = slot
            .search("技能")
            .iter()
            .map(|s| s.name.as_str())
            .collect();
        assert_eq!(by_desc, vec!["skill:keep"], "search() 不应含停用项");

        // 搜索（名称命中）：停用项不出现
        let by_name: Vec<&str> = slot
            .search("skill:")
            .iter()
            .map(|s| s.name.as_str())
            .collect();
        assert_eq!(by_name, vec!["skill:keep"], "search() 不应含停用项");

        // list_all() 仍含停用项
        let all: Vec<&str> = slot.list_all().iter().map(|s| s.name.as_str()).collect();
        assert!(all.contains(&"skill:gone"), "list_all() 应仍含停用项: {all:?}");
        assert_eq!(all.len(), 3, "list_all() 应含全部 3 项: {all:?}");
    }

    #[test]
    fn invoke_disabled_slot_errors() {
        let mut slot = PluginSlot::new();
        slot.register_tool("echo", "回声", Value::Null, Arc::new(EchoExecutor))
            .unwrap();
        // 停用前可正常调用
        assert!(slot.invoke("echo", &HashMap::new()).is_ok());
        assert!(slot.disable("echo"));

        let err = slot
            .invoke("echo", &HashMap::new())
            .expect_err("被停用槽位必须报错，不得静默可用");
        assert!(err.contains("echo"), "错误信息须含槽位名，实际: {err}");
        assert!(err.contains("停用"), "错误信息须说明原因，实际: {err}");
    }
}
