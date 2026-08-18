//! 面板注册表（Track D1，对译 `DSH-better-sidebar` 的 37 条 SCREEN_REGISTRY）。
//!
//! 工作台是「面板槽位」的集合。内置面板在 [`default_panels`] 预置 37 个；
//! ComingSoon 标记承载「占位兜底」（渲染层对 `coming_soon=true` 的面板渲染占位而非报错）；
//! 第三方 tab 经 [`PanelRegistry::register_third_party`] 注入，与内置面板隔离。

use std::collections::HashMap;

/// 面板类型。枚举覆盖核心工作台面，第三方扩展走 [`PanelKind::Custom`]。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PanelKind {
    /// 任务看板。
    Kanban,
    /// Git 提交图。
    GitGraph,
    /// 实时算力 / 资源监控。
    Monitor,
    /// 对话。
    Chat,
    /// 设置。
    Settings,
    /// 第三方自定义面板（承接外部 tab）。
    Custom(String),
}

/// 单条面板定义（图标交由 theme token 层，id/title 纯文字，禁 emoji）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PanelDef {
    /// 稳定标识。
    pub id: String,
    /// 展示标题（真实工作台语义，如「任务看板」「Git 提交图」）。
    pub title: String,
    /// 面板类型。
    pub kind: PanelKind,
    /// 是否启用。
    pub enabled: bool,
    /// 是否 ComingSoon 占位（渲染层渲染占位而非实体）。
    pub coming_soon: bool,
}

/// 面板注册表：内置面板 + 第三方 tab。
#[derive(Debug, Clone, Default)]
pub struct PanelRegistry {
    panels: HashMap<String, PanelDef>,
    third_party: Vec<PanelDef>,
}

/// 构造面板定义的简写（避免 37 条手写样板）。
fn mk(id: &str, title: &str, kind: PanelKind, enabled: bool, coming_soon: bool) -> PanelDef {
    PanelDef {
        id: id.to_string(),
        title: title.to_string(),
        kind,
        enabled,
        coming_soon,
    }
}

impl PanelRegistry {
    /// 空注册表。
    pub fn new() -> Self {
        Self::default()
    }

    /// 注册一个内置面板（按 id 去重覆盖）。
    pub fn register(&mut self, def: PanelDef) {
        self.panels.insert(def.id.clone(), def);
    }

    /// 注册第三方 tab（与内置面板隔离，不进 `panels`）。
    pub fn register_third_party(&mut self, def: PanelDef) {
        self.third_party.push(def);
    }

    /// 按 id 取面板（仅内置）。
    pub fn get(&self, id: &str) -> Option<&PanelDef> {
        self.panels.get(id)
    }

    /// 所有启用的内置面板。
    pub fn list_enabled(&self) -> Vec<&PanelDef> {
        self.panels.values().filter(|p| p.enabled).collect()
    }

    /// 所有 ComingSoon 占位面板（渲染层兜底）。
    pub fn list_coming_soon(&self) -> Vec<&PanelDef> {
        self.panels.values().filter(|p| p.coming_soon).collect()
    }

    /// 所有第三方 tab。
    pub fn list_third_party(&self) -> &[PanelDef] {
        &self.third_party
    }

    /// 内置面板总数。
    pub fn len(&self) -> usize {
        self.panels.len()
    }

    /// 是否空。
    pub fn is_empty(&self) -> bool {
        self.panels.is_empty()
    }
}

/// 37 个内置面板槽位（对译 `DSH-better-sidebar` 的 SCREEN_REGISTRY）。
///
/// 标题按真实工作台语义命名；图标交由 theme token 层，此处不出现 emoji。
/// 部分面板标记为 ComingSoon（渲染占位兜底）。
pub fn default_panels() -> Vec<PanelDef> {
    vec![
        mk("chat", "对话", PanelKind::Chat, true, false),
        mk("tasks", "任务看板", PanelKind::Kanban, true, false),
        mk("git-graph", "Git 提交图", PanelKind::GitGraph, true, false),
        mk("monitor-compute", "实时算力监控", PanelKind::Monitor, true, false),
        mk("memory-layer", "记忆层", PanelKind::Custom("memory".into()), true, false),
        mk("knowledge-graph", "知识图谱", PanelKind::Custom("kg".into()), true, false),
        mk("file-explorer", "文件浏览器", PanelKind::Custom("fs".into()), true, false),
        mk("terminal", "终端", PanelKind::Custom("term".into()), true, false),
        mk("settings", "设置", PanelKind::Settings, true, false),
        mk("agent-catalog", "Agent 目录", PanelKind::Custom("agent".into()), true, false),
        mk("usage-dashboard", "用量仪表盘", PanelKind::Monitor, true, false),
        mk("session-history", "会话历史", PanelKind::Custom("session".into()), true, false),
        mk("prompt-library", "提示词库", PanelKind::Custom("prompt".into()), true, false),
        mk("plugin-market", "插件市场", PanelKind::Custom("plugin".into()), true, true),
        mk("workflow-builder", "工作流编排", PanelKind::Custom("wf".into()), true, true),
        mk("rag-explorer", "检索增强浏览", PanelKind::Custom("rag".into()), true, true),
        mk("eval-board", "评测看板", PanelKind::Custom("eval".into()), true, true),
        mk("collab-board", "实时协同白板", PanelKind::Custom("collab".into()), true, true),
        mk("notebook", "笔记", PanelKind::Custom("note".into()), true, false),
        mk("canvas", "画布", PanelKind::Custom("canvas".into()), true, false),
        mk("diff-view", "差异视图", PanelKind::Custom("diff".into()), true, false),
        mk("log-stream", "日志流", PanelKind::Monitor, true, false),
        mk("trace-view", "调用链追踪", PanelKind::Monitor, true, false),
        mk("model-router", "模型路由", PanelKind::Custom("router".into()), true, false),
        mk("tool-registry", "工具注册表", PanelKind::Custom("tool".into()), true, false),
        mk("sandbox-console", "沙箱控制台", PanelKind::Custom("sandbox".into()), true, false),
        mk("security-audit", "安全审计", PanelKind::Custom("sec".into()), true, false),
        mk("billing", "额度账单", PanelKind::Monitor, true, false),
        mk("notifications", "通知中心", PanelKind::Custom("notify".into()), true, false),
        mk("profile", "个人资料", PanelKind::Settings, true, false),
        mk("theme-panel", "主题面板", PanelKind::Settings, true, false),
        mk("shortcut-map", "快捷键映射", PanelKind::Settings, true, false),
        mk("extension-list", "扩展列表", PanelKind::Custom("ext".into()), true, false),
        mk("system-status", "系统状态", PanelKind::Monitor, true, false),
        mk("data-pipeline", "数据管线", PanelKind::Custom("pipe".into()), true, true),
        mk("api-explorer", "API 调试台", PanelKind::Custom("api".into()), true, true),
        mk("help-center", "帮助中心", PanelKind::Custom("help".into()), true, false),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_builtins_enabled() {
        let mut r = PanelRegistry::new();
        for p in default_panels() {
            r.register(p);
        }
        assert_eq!(r.len(), 37, "内置面板必须预置 37 个槽位");
        assert_eq!(r.list_enabled().len(), 37, "全部启用");
        let chat = r.get("chat").expect("chat 应存在");
        assert_eq!(chat.title, "对话");
        assert_eq!(chat.kind, PanelKind::Chat);
        assert!(r.get("__missing__").is_none(), "未命中返回 None");
    }

    #[test]
    fn coming_soon_listed() {
        let mut r = PanelRegistry::new();
        for p in default_panels() {
            r.register(p);
        }
        let cs = r.list_coming_soon();
        assert!(!cs.is_empty(), "应有 ComingSoon 占位面板");
        assert!(cs.iter().any(|p| p.id == "plugin-market"));
        assert!(cs.iter().all(|p| p.coming_soon));
    }

    #[test]
    fn third_party_isolated() {
        let mut r = PanelRegistry::new();
        r.register_third_party(mk(
            "ext-1",
            "第三方扩展",
            PanelKind::Custom("ext".into()),
            true,
            false,
        ));
        assert_eq!(r.list_third_party().len(), 1);
        assert!(r.get("ext-1").is_none(), "第三方 tab 不进内置表");
    }
}
