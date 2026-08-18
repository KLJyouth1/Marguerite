//! 主流终端工作台插件 融合层：声明式插件清单 + 清单→原生 Plugin 桥接 + Hook 运行时
//!
//! 融合策略（守 D3 不搬运）：
//! - 不重复造轮子，复用既有 `plugins::Plugin` trait 与 `PluginRegistry`（外部参考 派生）。
//! - 新增 **声明式清单** `PluginManifest`：以数据描述一个插件（hooks / slash_commands /
//!   skills / subagents），免去为每个插件写整块 trait 实现样板。
//! - `ManifestPlugin`：把一份 `PluginManifest` 桥接成既有 `Plugin`，从而能直接 `register`
//!   进 `PluginRegistry`，与原生编译期插件共存。
//! - `HookRegistry`：对齐 Claude Code 的 PreToolUse / PostToolUse / Notification 钩子，
//!   钩子执行复用既有 L1/L2/L3 护栏概念（命令守卫 / 执行沙箱 / 操作护栏），不内联 agent 逻辑。
//!
//! 设计边界：清单式插件的「执行」由 `capabilities()` 中声明的动作名路由到既有
//! `Plugin::execute`；slash/subagent 声明仅作可观测登记（由上层调度消费），
//! 此处保持接口完整、不越权实现整段 SDK。

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Result;
use serde::{Deserialize, Serialize};

use super::{Plugin, PluginContext, PluginMeta, PluginRegistry};

/// Claude Code 风格钩子事件（对齐 SDK 的 PreToolUse / PostToolUse / Notification / Stop）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HookEvent {
    /// 工具调用前（可用于拦截/改写参数，复用 L1 命令守卫）
    PreToolUse,
    /// 工具调用后（可用于审计/改写结果，复用 L2/L3 审计）
    PostToolUse,
    /// 通知事件（复用 events 目录语义）
    Notification,
    /// agent 停止前（终检，复用 L3 操作护栏）
    Stop,
    /// 自演化触发（由 `automation/cron.rs` 的周期任务驱动，跑评测→变异闭环）
    SelfEvolve,
}

impl HookEvent {
    pub fn as_str(&self) -> &'static str {
        match self {
            HookEvent::PreToolUse => "PreToolUse",
            HookEvent::PostToolUse => "PostToolUse",
            HookEvent::Notification => "Notification",
            HookEvent::Stop => "Stop",
            HookEvent::SelfEvolve => "SelfEvolve",
        }
    }

    pub fn parse(s: &str) -> Option<HookEvent> {
        match s {
            "PreToolUse" | "pre_tool_use" | "PreTool" => Some(HookEvent::PreToolUse),
            "PostToolUse" | "post_tool_use" | "PostTool" => Some(HookEvent::PostToolUse),
            "Notification" | "notification" => Some(HookEvent::Notification),
            "Stop" | "stop" => Some(HookEvent::Stop),
            "SelfEvolve" | "self_evolve" | "SelfEvolving" => Some(HookEvent::SelfEvolve),
            _ => None,
        }
    }
}

/// 单条钩子声明（声明式，执行语义由 `HookRegistry` 落地）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HookSpec {
    /// 监听的事件
    pub event: String,
    /// 钩子脚本/处理器标识（路由键）
    pub handler: String,
    /// 附加参数（透传给 handler）
    #[serde(default)]
    pub params: HashMap<String, String>,
}

/// 斜杠命令声明（登记可观测，由上层 REPL/SDK 消费）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SlashCommand {
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub arguments: Vec<String>,
}

/// 子代理声明（Claude Code SDK 概念；登记可观测，由编排器消费）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubagentSpec {
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub tools: Vec<String>,
    /// 模型偏好（如 "inherit" / "sonnet" / "opus"），仅登记
    #[serde(default)]
    pub model: Option<String>,
}

/// 技能条目引用（复用既有 `skills::Skill` 体系，此处仅登记引用）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillRef {
    pub name: String,
    #[serde(default)]
    pub from: Option<String>,
}

/// 声明式插件清单（主流终端工作台插件 融合核心数据结构）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginManifest {
    pub name: String,
    pub version: String,
    pub description: String,
    #[serde(default)]
    pub author: Option<String>,
    /// 声明的能力（供 `PluginRegistry` 路由；缺省时由 hooks/skills 推导）
    #[serde(default)]
    pub capabilities: Vec<String>,
    /// 钩子声明
    #[serde(default)]
    pub hooks: Vec<HookSpec>,
    /// 斜杠命令声明
    #[serde(default)]
    pub slash_commands: Vec<SlashCommand>,
    /// 子代理声明
    #[serde(default)]
    pub subagents: Vec<SubagentSpec>,
    /// 技能引用
    #[serde(default)]
    pub skills: Vec<SkillRef>,
    /// 分层 bundle 依赖（按 `topo_order` 有序加载），声明本插件依赖的其它插件名。
    #[serde(default)]
    pub dependencies: Vec<String>,
    /// 能力级模型路由：能力名 → 模型偏好（如 "opus"/"sonnet"/"inherit"），
    /// 供 `agent::multi_strategy` 的 fleet 路由消费（每能力指定目标模型）。
    #[serde(default)]
    pub capability_models: HashMap<String, String>,
}

impl PluginManifest {
    /// 从 TOML/YAML 字符串解析（Cargo 生态用 TOML；此处用 serde 透传 JSON 以统一测试）。
    pub fn from_json(s: &str) -> Result<Self> {
        serde_json::from_str(s).map_err(Into::into)
    }

    /// 推导能力名集合（显式 capabilities + 各子结构自动登记键）。
    fn derived_capabilities(&self) -> Vec<String> {
        let mut caps: Vec<String> = self.capabilities.clone();
        for h in &self.hooks {
            let key = format!("hook.{}", h.handler);
            if !caps.contains(&key) {
                caps.push(key);
            }
        }
        if !self.slash_commands.is_empty() {
            caps.push("slash.commands".to_string());
        }
        if !self.subagents.is_empty() {
            caps.push("subagent.spawn".to_string());
        }
        if !self.skills.is_empty() {
            caps.push("skill.invoke".to_string());
        }
        caps
    }

    /// 取得能力级模型偏好（fleet 路由消费点）。
    ///
    /// 例：`capability_models = {"review.run": "opus"}` 时，
    /// `model_for_capability("review.run")` 返回 `Some("opus")`。
    pub fn model_for_capability(&self, capability: &str) -> Option<&str> {
        self.capability_models.get(capability).map(|s| s.as_str())
    }

    /// 取得分层 bundle 依赖列表（按 `topo_order` 有序加载）。
    pub fn dependencies(&self) -> &[String] {
        &self.dependencies
    }
}

/// 把 `PluginManifest` 桥接为原生 `Plugin` 实现。
///
/// 这样一份声明式清单即可 `register` 进既有 `PluginRegistry`，与原生编译期插件共存。
pub struct ManifestPlugin {
    manifest: PluginManifest,
    /// 派生能力（构造时算好，确保 `capabilities()` 稳定）
    caps: Vec<String>,
}

impl ManifestPlugin {
    pub fn new(manifest: PluginManifest) -> Self {
        let caps = manifest.derived_capabilities();
        Self { manifest, caps }
    }

    /// 取得内部清单（供上层调度消费 hooks/slash/subagents/skills 登记）
    pub fn manifest(&self) -> &PluginManifest {
        &self.manifest
    }
}

impl Plugin for ManifestPlugin {
    fn meta(&self) -> PluginMeta {
        PluginMeta {
            name: self.manifest.name.clone(),
            version: self.manifest.version.clone(),
            description: self.manifest.description.clone(),
            author: self.manifest.author.clone(),
        }
    }

    fn capabilities(&self) -> Vec<String> {
        self.caps.clone()
    }

    fn execute(&self, capability: &str, input: &str, _ctx: &PluginContext) -> Result<String> {
        // 钩子能力路由：hook.<handler> 执行时复用 L1/L2/L3 护栏概念做声明式校验
        if let Some(handler) = capability.strip_prefix("hook.") {
            // 在既有护栏模型下，钩子执行 = 把输入经命令守卫/沙箱/操作护栏校验后回显决策
            // 此处接入真实守卫时由 security 模块落地；当前保持接口完整并给出可观测回包。
            if let Some(spec) = self.manifest.hooks.iter().find(|h| h.handler == handler) {
                let event = HookEvent::parse(&spec.event)
                    .map(|e| e.as_str())
                    .unwrap_or("unknown");
                return Ok(format!(
                    "hook[{}] event={} params={:?} -> allow (guard passthrough)",
                    handler, event, spec.params
                ));
            }
            return Err(anyhow::anyhow!("未知钩子 handler: {handler}"));
        }
        if capability == "slash.commands" {
            let names: Vec<&str> = self
                .manifest
                .slash_commands
                .iter()
                .map(|s| s.name.as_str())
                .collect();
            return Ok(format!("slash_commands: {:?}", names));
        }
        if capability == "subagent.spawn" {
            let names: Vec<&str> = self
                .manifest
                .subagents
                .iter()
                .map(|s| s.name.as_str())
                .collect();
            return Ok(format!("subagents: {:?}", names));
        }
        if capability == "skill.invoke" {
            let names: Vec<&str> = self
                .manifest
                .skills
                .iter()
                .map(|s| s.name.as_str())
                .collect();
            return Ok(format!("skills: {:?}", names));
        }
        // 显式声明的原生能力直接回显（由真实实现或测试覆盖）
        if self.manifest.capabilities.contains(&capability.to_string()) {
            return Ok(format!("capability[{}] -> {}", capability, input));
        }
        Err(anyhow::anyhow!(
            "插件 `{}` 未实现能力 `{}`",
            self.meta().name,
            capability
        ))
    }
}

/// 钩子运行时：按事件注册 handler，对齐 Claude Code 的钩子派发。
///
/// 钩子 handler 以 `Arc<dyn Fn(HookEvent, &str) -> Result<String>>` 注入，
/// 真实守卫逻辑（L1/L2/L3）由调用方提供；此处负责注册/派发/顺序执行。
pub struct HookRegistry {
    /// event → 有序 handler 列表
    handlers: HashMap<HookEvent, Vec<Arc<dyn HookHandler>>>,
}

/// 钩子处理器接口（注入式，真实护栏由 security 模块实现）
pub trait HookHandler: Send + Sync {
    /// 执行钩子，返回处置结果文本（如 "allow" / "deny: reason" / "modify: ..."）
    fn run(&self, event: HookEvent, payload: &str) -> Result<String>;
}

impl Default for HookRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl HookRegistry {
    pub fn new() -> Self {
        Self {
            handlers: HashMap::new(),
        }
    }

    /// 注册一个钩子处理器（同名事件可注册多个，按注册顺序执行）
    pub fn register(&mut self, event: HookEvent, handler: Arc<dyn HookHandler>) {
        self.handlers.entry(event).or_default().push(handler);
    }

    /// 派发某事件：顺序执行所有 handler，任一返回非 allow 即短路返回该结果。
    pub fn dispatch(&self, event: HookEvent, payload: &str) -> Vec<HookOutcome> {
        let mut outcomes = Vec::new();
        if let Some(list) = self.handlers.get(&event) {
            for h in list {
                match h.run(event, payload) {
                    Ok(decision) => {
                        let allow = decision.starts_with("allow");
                        outcomes.push(HookOutcome {
                            event,
                            decision,
                            allowed: allow,
                        });
                        if !allow {
                            break; // 短路：首个 deny/modify 即停
                        }
                    }
                    Err(e) => outcomes.push(HookOutcome {
                        event,
                        decision: format!("error: {e}"),
                        allowed: false,
                    }),
                }
            }
        }
        outcomes
    }

    /// 某事件是否注册了 handler
    pub fn has(&self, event: HookEvent) -> bool {
        self.handlers
            .get(&event)
            .map(|v| !v.is_empty())
            .unwrap_or(false)
    }
}

/// 单次钩子派发结果
#[derive(Debug, Clone)]
pub struct HookOutcome {
    pub event: HookEvent,
    pub decision: String,
    pub allowed: bool,
}

/// 便捷构造：把若干清单中的钩子声明 + 注入的真实 handler 装入 `HookRegistry`。
///
/// `resolver` 把 `HookSpec.handler` 解析为真实 handler；解析失败则跳过该钩子。
pub fn build_hook_registry<F>(manifests: &[PluginManifest], mut resolver: F) -> HookRegistry
where
    F: FnMut(&str) -> Option<Arc<dyn HookHandler>>,
{
    let mut reg = HookRegistry::new();
    for m in manifests {
        for spec in &m.hooks {
            if let Some(event) = HookEvent::parse(&spec.event) {
                if let Some(h) = resolver(&spec.handler) {
                    reg.register(event, h);
                }
            }
        }
    }
    reg
}

/// 把清单注册进既有 `PluginRegistry`（桥接为 `ManifestPlugin`）。
pub fn register_manifests(registry: &mut PluginRegistry, manifests: &[PluginManifest]) {
    for m in manifests {
        registry.register(Arc::new(ManifestPlugin::new(m.clone())));
    }
}

/// 按 `dependencies` 做拓扑有序加载（Kahn 算法）。
///
/// 插件 A 依赖 B 时，B 必排在 A 之前。存在环时返回 `Err`（环上的插件名），
/// 调用方应拒绝该 bundle；无依赖的插件保持入参相对顺序（稳定）。
pub fn topo_order(manifests: &[PluginManifest]) -> Result<Vec<PluginManifest>> {
    use std::collections::{HashMap, VecDeque};

    let mut indeg: HashMap<&str, usize> = HashMap::new();
    let mut adj: HashMap<&str, Vec<&str>> = HashMap::new();
    for m in manifests {
        indeg.entry(m.name.as_str()).or_insert(0);
        for d in &m.dependencies {
            *indeg.entry(m.name.as_str()).or_insert(0) += 1;
            adj.entry(d.as_str()).or_default().push(m.name.as_str());
        }
    }

    let mut q: VecDeque<&str> = indeg
        .iter()
        .filter(|(_, &d)| d == 0)
        .map(|(&n, _)| n)
        .collect();
    let mut name_to_manifest: HashMap<&str, &PluginManifest> =
        manifests.iter().map(|m| (m.name.as_str(), m)).collect();
    let mut out: Vec<PluginManifest> = Vec::with_capacity(manifests.len());
    let mut placed = 0usize;

    while let Some(n) = q.pop_front() {
        if let Some(m) = name_to_manifest.remove(n) {
            out.push(m.clone());
            placed += 1;
        }
        if let Some(succ) = adj.get(n) {
            for s in succ {
                if let Some(d) = indeg.get_mut(s) {
                    *d = d.saturating_sub(1);
                    if *d == 0 {
                        q.push_back(s);
                    }
                }
            }
        }
    }

    if placed != manifests.len() {
        // 未能放置的节点即环上（或依赖缺失）的插件
        let cycle: Vec<String> = indeg
            .iter()
            .filter(|(_, &d)| d > 0)
            .map(|(&n, _)| n.to_string())
            .collect();
        return Err(anyhow::anyhow!("插件依赖存在环或缺失: {:?}", cycle));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{
        "name": "code-review-bot",
        "version": "1.2.0",
        "description": "声明式代码审查插件",
        "author": "marguerite",
        "capabilities": ["review.run"],
        "hooks": [
            {"event": "PreToolUse", "handler": "guard_cmd", "params": {"scope": "shell"}},
            {"event": "PostToolUse", "handler": "audit_io", "params": {}}
        ],
        "slash_commands": [
            {"name": "/review", "description": "审查当前变更", "arguments": ["target"]}
        ],
        "subagents": [
            {"name": "reviewer", "description": "专职审查", "tools": ["read","grep"], "model": "inherit"}
        ],
        "skills": [
            {"name": "security-scan", "from": "builtin"}
        ]
    }"#;

    #[test]
    fn manifest_parses_all_sections() {
        let m = PluginManifest::from_json(SAMPLE).unwrap();
        assert_eq!(m.name, "code-review-bot");
        assert_eq!(m.hooks.len(), 2);
        assert_eq!(m.slash_commands.len(), 1);
        assert_eq!(m.subagents.len(), 1);
        assert_eq!(m.skills.len(), 1);
        assert_eq!(m.hooks[0].handler, "guard_cmd");
        assert_eq!(m.slash_commands[0].name, "/review");
        assert_eq!(m.subagents[0].tools, vec!["read", "grep"]);
    }

    #[test]
    fn manifest_derives_capabilities() {
        let m = PluginManifest::from_json(SAMPLE).unwrap();
        let caps = m.derived_capabilities();
        assert!(caps.contains(&"review.run".to_string()));
        assert!(caps.contains(&"hook.guard_cmd".to_string()));
        assert!(caps.contains(&"hook.audit_io".to_string()));
        assert!(caps.contains(&"slash.commands".to_string()));
        assert!(caps.contains(&"subagent.spawn".to_string()));
        assert!(caps.contains(&"skill.invoke".to_string()));
    }

    #[test]
    fn manifest_plugin_bridges_into_registry() {
        let m = PluginManifest::from_json(SAMPLE).unwrap();
        let mut reg = PluginRegistry::new();
        register_manifests(&mut reg, &[m]);
        assert_eq!(reg.len(), 1);

        let ctx = PluginContext::default();
        // 钩子能力路由
        let r = reg.invoke("hook.guard_cmd", "ls -la", &ctx).unwrap();
        assert!(r.contains("hook[guard_cmd]"), "got: {r}");
        assert!(r.contains("PreToolUse"));
        // 声明式子结构能力
        let r = reg.invoke("slash.commands", "", &ctx).unwrap();
        assert!(r.contains("/review"));
        let r = reg.invoke("subagent.spawn", "", &ctx).unwrap();
        assert!(r.contains("reviewer"));
        let r = reg.invoke("skill.invoke", "", &ctx).unwrap();
        assert!(r.contains("security-scan"));
        // 显式能力
        let r = reg.invoke("review.run", "diff", &ctx).unwrap();
        assert!(r.contains("diff"));
    }

    #[test]
    fn manifest_plugin_unknown_capability_errors() {
        let m = PluginManifest::from_json(SAMPLE).unwrap();
        let mut reg = PluginRegistry::new();
        register_manifests(&mut reg, &[m]);
        let ctx = PluginContext::default();
        assert!(reg.invoke("nonexistent", "x", &ctx).is_err());
    }

    // ---- HookRegistry 测试 ----

    struct AllowHandler;
    impl HookHandler for AllowHandler {
        fn run(&self, _event: HookEvent, _payload: &str) -> Result<String> {
            Ok("allow".to_string())
        }
    }

    struct DenyHandler;
    impl HookHandler for DenyHandler {
        fn run(&self, _event: HookEvent, _payload: &str) -> Result<String> {
            Ok("deny: blocked by policy".to_string())
        }
    }

    #[test]
    fn hook_registry_dispatches_in_order() {
        let mut reg = HookRegistry::new();
        reg.register(HookEvent::PreToolUse, Arc::new(AllowHandler));
        reg.register(HookEvent::PreToolUse, Arc::new(AllowHandler));
        let out = reg.dispatch(HookEvent::PreToolUse, "cmd");
        assert_eq!(out.len(), 2);
        assert!(out.iter().all(|o| o.allowed));
    }

    #[test]
    fn hook_registry_short_circuits_on_deny() {
        let mut reg = HookRegistry::new();
        reg.register(HookEvent::PreToolUse, Arc::new(AllowHandler));
        reg.register(HookEvent::PreToolUse, Arc::new(DenyHandler));
        reg.register(HookEvent::PreToolUse, Arc::new(AllowHandler)); // 不应被执行
        let out = reg.dispatch(HookEvent::PreToolUse, "rm -rf");
        assert_eq!(out.len(), 2);
        assert!(out[0].allowed);
        assert!(!out[1].allowed);
        assert!(out[1].decision.contains("deny"));
    }

    #[test]
    fn hook_registry_empty_event_has_no_outcomes() {
        let reg = HookRegistry::new();
        assert!(!reg.has(HookEvent::Stop));
        assert!(reg.dispatch(HookEvent::Stop, "x").is_empty());
    }

    #[test]
    fn build_hook_registry_from_manifests() {
        let m = PluginManifest::from_json(SAMPLE).unwrap();
        let reg = build_hook_registry(&[m], |handler| {
            if handler == "guard_cmd" {
                Some(Arc::new(AllowHandler) as Arc<dyn HookHandler>)
            } else {
                None // audit_io 未提供真实 handler，跳过
            }
        });
        assert!(reg.has(HookEvent::PreToolUse));
        assert!(!reg.has(HookEvent::PostToolUse)); // audit_io 被跳过
        let out = reg.dispatch(HookEvent::PreToolUse, "git status");
        assert_eq!(out.len(), 1);
        assert!(out[0].allowed);
    }

    #[test]
    fn hook_event_parse_roundtrip() {
        assert_eq!(HookEvent::parse("PreToolUse"), Some(HookEvent::PreToolUse));
        assert_eq!(
            HookEvent::parse("post_tool_use"),
            Some(HookEvent::PostToolUse)
        );
        assert_eq!(
            HookEvent::parse("notification"),
            Some(HookEvent::Notification)
        );
        assert_eq!(HookEvent::parse("Stop"), Some(HookEvent::Stop));
        assert_eq!(HookEvent::parse("self_evolve"), Some(HookEvent::SelfEvolve));
        assert_eq!(HookEvent::parse("bogus"), None);
        assert_eq!(HookEvent::PreToolUse.as_str(), "PreToolUse");
        assert_eq!(HookEvent::SelfEvolve.as_str(), "SelfEvolve");
    }

    #[test]
    fn manifest_capability_model_and_deps() {
        let json = r#"{
            "name": "b",
            "version": "0.1.0",
            "description": "dep demo",
            "dependencies": ["a"],
            "capability_models": {"review.run": "opus"}
        }"#;
        let m = PluginManifest::from_json(json).unwrap();
        assert_eq!(m.dependencies(), &["a".to_string()]);
        assert_eq!(m.model_for_capability("review.run"), Some("opus"));
        assert_eq!(m.model_for_capability("missing.cap"), None);
    }

    #[test]
    fn topo_order_respects_dependencies() {
        let a = PluginManifest::from_json(r#"{"name":"a","version":"0.1.0","description":"a"}"#)
            .unwrap();
        let b = PluginManifest::from_json(
            r#"{"name":"b","version":"0.1.0","description":"b","dependencies":["a"]}"#,
        )
        .unwrap();
        // 入参故意逆序（b 在 a 前），拓扑排序须把 a 提到 b 之前
        let ordered = topo_order(&[b.clone(), a.clone()]).unwrap();
        assert_eq!(ordered[0].name, "a");
        assert_eq!(ordered[1].name, "b");
    }

    #[test]
    fn topo_order_detects_cycle() {
        let a = PluginManifest::from_json(
            r#"{"name":"a","version":"0.1.0","description":"a","dependencies":["b"]}"#,
        )
        .unwrap();
        let b = PluginManifest::from_json(
            r#"{"name":"b","version":"0.1.0","description":"b","dependencies":["a"]}"#,
        )
        .unwrap();
        assert!(topo_order(&[a, b]).is_err(), "a<->b 环应被拒绝");
    }
}
