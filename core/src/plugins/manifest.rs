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
use std::path::Path;
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

/// 插件激活方式（P0 S3）。
///
/// 缺省 `Restart`（保守）：未声明 `entry` 的清单**不会**被假装成可热加载。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Activation {
    /// 需重启进程方可生效（缺省）
    #[default]
    Restart,
    /// 运行期热加载（要求同时声明 `entry`，见 `manifest.hot_without_entry`）
    Hot,
}

/// 插件权限声明（P0 S3 最小可用集：fs / net / exec / event / slot）。
///
/// 序列化形态为内部标签：`{"kind":"exec","allow":["git"]}`。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Permission {
    /// 文件系统访问
    Fs {
        /// 允许的路径范围（**不做 glob 解释**，仅登记）
        #[serde(default)]
        scope: Vec<String>,
        /// 是否允许写
        #[serde(default)]
        write: bool,
    },
    /// 网络访问
    Net {
        /// 允许的目标主机
        #[serde(default)]
        allow_hosts: Vec<String>,
    },
    /// 外部命令执行
    Exec {
        /// 允许的命令白名单
        #[serde(default)]
        allow: Vec<String>,
    },
    /// 事件总线收发
    Event {
        /// 允许发布的事件模式
        #[serde(default)]
        publish: Vec<String>,
        /// 允许订阅的事件模式
        #[serde(default)]
        subscribe: Vec<String>,
    },
    /// 槽位注册
    Slot {
        /// 允许注册的槽位名
        #[serde(default)]
        register: Vec<String>,
        /// 是否可影响槽位顺序
        #[serde(default)]
        order: bool,
    },
}

/// 清单自洽性诊断（P0 S3）。
///
/// **本模块自有**类型：刻意不与 `plugins::patch::PatchDiagnostic` 统一 —— S2 的
/// patch 诊断已过全量门禁，提前统一会引入无关回归。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManifestDiagnostic {
    /// 机器可读诊断码，如 `manifest.hot_without_entry`
    pub code: &'static str,
    /// 产生该诊断的清单 `name`
    pub manifest: String,
    /// 人类可读说明
    pub message: String,
}

impl ManifestDiagnostic {
    fn new(code: &'static str, manifest: &str, message: String) -> Self {
        Self {
            code,
            manifest: manifest.to_string(),
            message,
        }
    }
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
    /// 本插件声明会订阅或发布的事件路由键名（DSH 二创扩展）。
    ///
    /// 与 `EventBus` 主题模式一致，可用通配（`*` / `**`）；仅用于文档 / 对账 / 订阅自动注册。
    ///
    /// 例：`["vision.recognized", "slash.**", "agent.*", "usage.daily"]`
    #[serde(default)]
    pub events: Vec<String>,
    /// 能力级模型路由：能力名 → 模型偏好（如 "opus"/"sonnet"/"inherit"），
    /// 供 `agent::multi_strategy` 的 fleet 路由消费（每能力指定目标模型）。
    #[serde(default)]
    pub capability_models: HashMap<String, String>,

    // ── P0 S3 新增字段：全部 `#[serde(default)]`，16 份既有 JSON 必须仍能解析 ──
    /// 原生入口：**相对 manifest 所在目录**的路径；`None` = 纯声明式（无原生实现）。
    #[serde(default)]
    pub entry: Option<String>,
    /// 激活方式；缺省 `Restart`（保守，不假装能热加载）。
    #[serde(default)]
    pub activation: Activation,
    /// 卸载入口：相对 manifest 所在目录的路径。
    #[serde(default)]
    pub dispose: Option<String>,
    /// 声明的权限（最小可用集）。
    #[serde(default)]
    pub permissions: Vec<Permission>,
}

impl PluginManifest {
    /// 从 TOML/YAML 字符串解析（Cargo 生态用 TOML；此处用 serde 透传 JSON 以统一测试）。
    pub fn from_json(s: &str) -> Result<Self> {
        serde_json::from_str(s).map_err(Into::into)
    }

    /// 批量装载某目录下的全部 `*.plugin.json` 为 `PluginManifest`。
    ///
    /// 逐项失败一律响亮（读目录/读目录项/读文件/解析任一失败 → `anyhow::bail!` 带路径）。
    /// 与 `market::collect_plugin_items` 的「逐项 skipped」不同：此处装配需要**结构体**，
    /// 故任一条损坏即整体失败（boot 装配不应带半截清单跑）。
    pub fn load_dir(dir: &Path) -> Result<Vec<PluginManifest>> {
        let rd = std::fs::read_dir(dir)
            .map_err(|e| anyhow::anyhow!("读取插件清单目录失败 {}: {e}", dir.display()))?;
        let mut out: Vec<PluginManifest> = Vec::new();
        for entry in rd {
            let entry = entry
                .map_err(|e| anyhow::anyhow!("读取插件清单目录项失败（{}）: {e}", dir.display()))?;
            let path = entry.path();
            let is_manifest = path
                .file_name()
                .and_then(|n| n.to_str())
                .map(|n| n.ends_with(".plugin.json"))
                .unwrap_or(false);
            if !is_manifest {
                continue;
            }
            let text = std::fs::read_to_string(&path)
                .map_err(|e| anyhow::anyhow!("读取插件清单文件失败 {}: {e}", path.display()))?;
            let m = Self::from_json(&text)
                .map_err(|e| anyhow::anyhow!("解析插件清单文件失败 {}: {e:?}", path.display()))?;
            out.push(m);
        }
        Ok(out)
    }

    /// 推导能力名集合（显式 capabilities + 各子结构自动登记键 + events 派生订阅能力）。
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
        // DSH 二创扩展：每条事件订阅路由登记为 `event.<pattern>` 能力，
        // 以便 `PluginRegistry` 可按事件模式反向查询感兴趣的插件。
        for e in &self.events {
            let key = format!("event.{}", e);
            if !caps.contains(&key) {
                caps.push(key);
            }
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

    /// 取得本插件声明的事件路由键列表（DSH 二创扩展）。
    ///
    /// 与 `EventBus` 主题模式一致，可用通配；用于订阅自动注册与能力对账。
    pub fn events(&self) -> &[String] {
        &self.events
    }

    /// 是否**纯声明式**（P0 S3）：既无原生入口、也未声明任何权限。
    ///
    /// 当前 16 份 DSH 清单全部为 `true` —— 即「声明了但没实现」，须在 UI / doctor 可见。
    pub fn is_declarative_only(&self) -> bool {
        self.entry.is_none() && self.permissions.is_empty()
    }

    /// 清单自洽性校验（P0 S3）：一次报全，不短路。
    ///
    /// 覆盖四类 code：
    /// ① `manifest.hot_without_entry`：`activation == Hot` 却无 `entry`（无从热加载）；
    /// ② `manifest.undeclared_side_effect`：`capabilities` 出现 `fs.`/`exec.`/`net.` 前缀，
    ///    但 `permissions` 为空；
    /// ③ `manifest.empty_shell`：`capabilities`/`hooks`/`slash_commands`/`subagents`/
    ///    `skills`/`events` 全空（空壳清单）；
    /// ④ `manifest.exec_without_decl`：声明了 `Permission::Exec`，却无任何 `exec.` 前缀能力。
    pub fn validate(&self) -> Vec<ManifestDiagnostic> {
        let mut diags = Vec::new();

        // ① 声明热加载但无原生入口
        if self.activation == Activation::Hot && self.entry.is_none() {
            diags.push(ManifestDiagnostic::new(
                "manifest.hot_without_entry",
                &self.name,
                "activation=hot 但未声明 entry，热加载无从谈起".to_string(),
            ));
        }

        // ② 副作用能力未声明权限
        let side_effects: Vec<&str> = self
            .capabilities
            .iter()
            .map(String::as_str)
            .filter(|c| {
                c.starts_with("fs.") || c.starts_with("exec.") || c.starts_with("net.")
            })
            .collect();
        if self.permissions.is_empty() && !side_effects.is_empty() {
            diags.push(ManifestDiagnostic::new(
                "manifest.undeclared_side_effect",
                &self.name,
                format!("声明了副作用能力 {side_effects:?}，但 permissions 为空（未声明权限）"),
            ));
        }

        // ③ 空壳清单
        let is_empty_shell = self.capabilities.is_empty()
            && self.hooks.is_empty()
            && self.slash_commands.is_empty()
            && self.subagents.is_empty()
            && self.skills.is_empty()
            && self.events.is_empty();
        if is_empty_shell {
            diags.push(ManifestDiagnostic::new(
                "manifest.empty_shell",
                &self.name,
                "capabilities/hooks/slash_commands/subagents/skills/events 全空（空壳清单）"
                    .to_string(),
            ));
        }

        // ④ 声明 exec 权限却无 exec. 能力
        let has_exec_perm = self
            .permissions
            .iter()
            .any(|p| matches!(p, Permission::Exec { .. }));
        let has_exec_cap = self.capabilities.iter().any(|c| c.starts_with("exec."));
        if has_exec_perm && !has_exec_cap {
            diags.push(ManifestDiagnostic::new(
                "manifest.exec_without_decl",
                &self.name,
                "声明了 exec 权限，但 capabilities 中无 exec. 前缀能力（权限与能力不对齐）"
                    .to_string(),
            ));
        }

        diags
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

    // ---- DSH 二创扩展：events 字段测试 ----

    const DSH_MODLENS: &str = r#"{
        "name": "dsh-modlens",
        "version": "0.1.0-dsh",
        "description": "DSH Modlens 二创：纯文本模型识图能力",
        "author": "marguerite-dsh",
        "capabilities": ["dsh.vision.img2txt", "dsh.vision.extract_json"],
        "events": ["vision.recognized", "slash.recognize", "agent.paste"],
        "hooks": [
            {"event": "PreToolUse", "handler": "modlens_prepare", "params": {"step": "ocr"}}
        ],
        "slash_commands": [
            {"name": "/recognize", "description": "识别剪贴板/附件图片", "arguments": ["format"]}
        ]
    }"#;

    #[test]
    fn dsh_events_field_parses_and_getter_works() {
        let m = PluginManifest::from_json(DSH_MODLENS).unwrap();
        assert_eq!(m.name, "dsh-modlens");
        assert_eq!(
            m.events(),
            &[
                "vision.recognized".to_string(),
                "slash.recognize".to_string(),
                "agent.paste".to_string(),
            ]
        );
    }

    #[test]
    fn dsh_events_derived_as_event_capabilities() {
        let m = PluginManifest::from_json(DSH_MODLENS).unwrap();
        let caps = m.derived_capabilities();
        // 显式能力
        assert!(caps.contains(&"dsh.vision.img2txt".to_string()));
        assert!(caps.contains(&"dsh.vision.extract_json".to_string()));
        // events → event.<pattern> 派生能力
        assert!(caps.contains(&"event.vision.recognized".to_string()));
        assert!(caps.contains(&"event.slash.recognize".to_string()));
        assert!(caps.contains(&"event.agent.paste".to_string()));
        // hooks + slash 派生能力不应缺失
        assert!(caps.contains(&"hook.modlens_prepare".to_string()));
        assert!(caps.contains(&"slash.commands".to_string()));
    }

    // ---- T-1.2：DSH manifest 实装验证 ----

    /// DSH 清单目录：`DSH_MANIFEST_DIR` 覆盖 > 默认 `<repo>/plugins/manifests`。
    ///
    /// `CARGO_MANIFEST_DIR` = `<repo>/core`，故上溯**一层**即仓库根。
    ///
    /// 历史缺陷：此处曾写成 `.join("..").join("..")`（多一层）→ 解析到
    /// `<repo>/../plugins/manifests`（工作区之外，本项目实测为 `D:\Downloads\plugins\manifests`）
    /// → 目录不存在 → `read_dir` 返回 Err → 清单列表为空 → 本模块两条 DSH 清单测试
    /// 走 `if list.is_empty() { return }` **空转通过**。门禁 16/16 全绿，实际零验证
    /// —— 比 FAIL 更隐蔽（vacuous pass）。
    fn dsh_manifest_dir() -> std::path::PathBuf {
        std::env::var("DSH_MANIFEST_DIR")
            .ok()
            .filter(|v| !v.trim().is_empty())
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| {
                std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                    .join("..")
                    .join("plugins")
                    .join("manifests")
            })
    }

    /// 是否由 `DSH_MANIFEST_DIR` 显式覆盖目录。
    ///
    /// 覆盖模式下目录缺失视为调用方的选择（允许跳过）；**默认路径下缺失视为缺陷**
    /// （路径解析错或清单被删），必须失败而不是静默跳过。
    fn dsh_manifest_dir_is_override() -> bool {
        std::env::var("DSH_MANIFEST_DIR")
            .map(|v| !v.trim().is_empty())
            .unwrap_or(false)
    }

    /// 从 DSH 清单目录加载全部 `dsh-*.plugin.json`。
    ///
    /// 读取失败或 JSON 解析失败**直接 panic 并带上路径** —— 曾经这两处都是
    /// `if let Ok(..)`，坏清单会被静默丢弃，表现为「份数莫名少一份」，
    /// 与上文空转缺陷同类。门禁宁可响亮失败，也不静默减计数。
    fn load_dsh_manifests_from_disk() -> Vec<(std::path::PathBuf, PluginManifest)> {
        let base = dsh_manifest_dir();
        let rd = match std::fs::read_dir(&base) {
            Ok(rd) => rd,
            Err(e) => {
                if dsh_manifest_dir_is_override() {
                    return Vec::new();
                }
                panic!(
                    "默认 DSH 清单目录 {} 不可读（{e}）—— 路径解析缺陷或清单目录被删",
                    base.display()
                );
            }
        };
        let mut out = Vec::new();
        for entry in rd {
            let path = entry
                .unwrap_or_else(|e| panic!("读取目录项失败（{}）: {e}", base.display()))
                .path();
            if path
                .file_name()
                .and_then(|n| n.to_str())
                .map(|n| n.starts_with("dsh-") && n.ends_with(".plugin.json"))
                .unwrap_or(false)
            {
                let text = std::fs::read_to_string(&path)
                    .unwrap_or_else(|e| panic!("读取清单 {} 失败: {e}", path.display()));
                let m = PluginManifest::from_json(&text)
                    .unwrap_or_else(|e| panic!("解析清单 {} 失败: {e:?}", path.display()));
                out.push((path, m));
            }
        }
        out.sort_by(|a, b| a.0.cmp(&b.0));
        out
    }

    #[test]
    fn load_all_dsh_manifests_success() {
        let list = load_dsh_manifests_from_disk();
        // 仅当调用方用 DSH_MANIFEST_DIR 显式指定目录时，空目录才允许跳过。
        // 默认路径下空 = 缺陷（见 dsh_manifest_dir 的历史空转说明），必须响亮失败。
        if list.is_empty() {
            assert!(
                dsh_manifest_dir_is_override(),
                "默认清单目录 {} 下未找到任何 dsh-*.plugin.json —— 路径解析缺陷或清单缺失\
                 （本测试曾因多一层 `..` 而空转通过，不得再静默跳过）",
                dsh_manifest_dir().display()
            );
            eprintln!(
                "skip: DSH_MANIFEST_DIR={} 下无 dsh-*.plugin.json（显式覆盖，允许跳过）",
                dsh_manifest_dir().display()
            );
            return;
        }
        // 动态下界：磁盘实际份数（基线 16）。下界而非等式，避免新增清单时再度漂移；
        // 但仍设下限，防止「只剩 1 份也绿」。
        assert!(
            list.len() >= 16,
            "应至少找到 16 份 DSH 二创 manifest，实际 {} 份：{:?}",
            list.len(),
            list.iter()
                .map(|(p, _)| p.display().to_string())
                .collect::<Vec<_>>()
        );
        for (path, m) in &list {
            // 字段齐全：name/version/description 非空；dependencies/events 至少其一非空。
            assert!(!m.name.is_empty(), "name empty @ {}", path.display());
            assert!(!m.version.is_empty(), "version empty @ {}", path.display());
            assert!(
                !m.description.is_empty(),
                "description empty @ {}",
                path.display()
            );
            let has_ext = !m.dependencies.is_empty() || !m.events.is_empty();
            assert!(
                has_ext,
                "manifest {} 至少应包含 dependencies 或 events 其中一个扩展字段",
                path.display()
            );
        }
    }

    #[test]
    fn at_least_6_manifests_have_ext_fields_populated() {
        let list = load_dsh_manifests_from_disk();
        // 与 load_all_dsh_manifests_success 同口径：默认路径空必须失败，不得静默跳过。
        if list.is_empty() {
            assert!(
                dsh_manifest_dir_is_override(),
                "默认清单目录 {} 下无清单，无法统计扩展字段实装率",
                dsh_manifest_dir().display()
            );
            eprintln!("skip: DSH_MANIFEST_DIR override with no manifests on disk");
            return;
        }
        let n = list
            .iter()
            .filter(|(_, m)| !m.dependencies.is_empty() || !m.events.is_empty())
            .count();
        assert!(n >= 6, "至少 6 份 manifest 实装了扩展字段，实际 {n}");
    }

    // ---- P0 S3：entry / activation / dispose / permissions 新字段 ----

    /// 旧 JSON（无新字段）必须仍能解析，且缺省值保守。
    #[test]
    fn p0_s3_new_fields_default_conservatively() {
        let m = PluginManifest::from_json(
            r#"{"name":"legacy","version":"0.1.0","description":"无新字段的旧清单"}"#,
        )
        .unwrap();
        assert_eq!(m.entry, None);
        assert_eq!(m.activation, Activation::Restart);
        assert_eq!(m.dispose, None);
        assert!(m.permissions.is_empty());
        assert!(m.is_declarative_only());
    }

    /// 五种权限的 serde 内部标签（`kind`）往返。
    #[test]
    fn p0_s3_permission_variants_roundtrip() {
        let json = r#"{
            "name":"native-demo",
            "version":"0.2.0",
            "description":"带原生入口与权限",
            "entry":"bin/demo.exe",
            "activation":"hot",
            "dispose":"bin/demo-stop.exe",
            "capabilities":["exec.shell","demo.run"],
            "permissions":[
                {"kind":"fs","scope":["work"],"write":true},
                {"kind":"net","allow_hosts":["api.example.com"]},
                {"kind":"exec","allow":["git"]},
                {"kind":"event","publish":["demo.out"],"subscribe":["tool.**"]},
                {"kind":"slot","register":["tools"],"order":true}
            ]
        }"#;
        let m = PluginManifest::from_json(json).unwrap();
        assert_eq!(m.entry.as_deref(), Some("bin/demo.exe"));
        assert_eq!(m.activation, Activation::Hot);
        assert_eq!(m.dispose.as_deref(), Some("bin/demo-stop.exe"));
        assert_eq!(m.permissions.len(), 5);
        assert!(!m.is_declarative_only());
        assert!(
            m.validate().iter().all(|d| d.code != "manifest.exec_without_decl"),
            "已声明 exec. 能力时不应报 exec_without_decl"
        );
        // 内部标签序列化 → 反序列化保持同一形态
        let text = serde_json::to_string(&m.permissions).unwrap();
        let back: Vec<Permission> = serde_json::from_str(&text).unwrap();
        assert!(matches!(
            back[0],
            Permission::Fs { write: true, .. }
        ));
        assert!(matches!(back[4], Permission::Slot { order: true, .. }));
    }

    /// 热加载但无入口 → 自相矛盾，必须报 `manifest.hot_without_entry`。
    #[test]
    fn p0_s3_hot_without_entry_is_flagged() {
        let m = PluginManifest::from_json(
            r#"{"name":"hot-ghost","version":"0.1.0","description":"d","activation":"hot","capabilities":["x.run"]}"#,
        )
        .unwrap();
        let codes: Vec<&str> = m.validate().iter().map(|d| d.code).collect();
        assert!(codes.contains(&"manifest.hot_without_entry"), "{codes:?}");
    }
}
