//! 插件系统
//!
//! 借鉴 外部参考 plugins：轻量插件契约 + 沙箱执行。
//! 插件为原生 Rust trait 实现（编译时静态加载），
//! 支持生命周期钩子、能力注册与热插拔（通过 Arc 注册表）。

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Result;

pub mod di;
pub mod manifest;
pub mod patch;
pub mod slot;

pub use manifest::{Activation, ManifestDiagnostic, Permission};
pub use patch::{PatchDiagnostic, PatchLayer, PatchOp, PatchReport, PatchSummary};
pub use slot::{PluginSlot, SlotDescriptor, SlotExecutor, SlotKind, SlotResult};

/// 插件上下文：提供运行时可用的核心能力
#[derive(Debug, Clone, Default)]
pub struct PluginContext {
    /// 配置项
    pub config: HashMap<String, String>,
    /// 能力注入（如工具调用闭包）
    pub capabilities: HashMap<String, String>,
}

impl PluginContext {
    /// 读取配置
    pub fn get(&self, key: &str) -> Option<&str> {
        self.config.get(key).map(|s| s.as_str())
    }
}

/// 插件元信息
#[derive(Debug, Clone)]
pub struct PluginMeta {
    pub name: String,
    pub version: String,
    pub description: String,
    pub author: Option<String>,
}

/// 插件 trait：轻量契约
///
/// 所有插件实现 `meta()` 与 `on_load()`，可选钩子默认空实现。
/// `capabilities()` 声明插件提供的能力名（供注册表路由）。
pub trait Plugin: Send + Sync {
    /// 插件元信息
    fn meta(&self) -> PluginMeta;

    /// 插件初始化（加载时调用）
    fn on_load(&self, _ctx: &PluginContext) -> Result<()> {
        Ok(())
    }

    /// 插件卸载（可选钩子）
    fn on_unload(&self) {}

    /// 声明提供的能力名列表
    fn capabilities(&self) -> Vec<String> {
        Vec::new()
    }

    /// 执行插件动作（由能力名路由）
    fn execute(&self, _capability: &str, _input: &str, _ctx: &PluginContext) -> Result<String> {
        Err(anyhow::anyhow!("插件 `{}` 未实现该动作", self.meta().name))
    }
}

/// 插件注册表
///
/// 管理插件生命周期，支持按能力名路由调用。
#[derive(Default)]
pub struct PluginRegistry {
    plugins: Vec<Arc<dyn Plugin>>,
    /// 能力名 → 插件索引（构建时生成）
    index: HashMap<String, usize>,
}

impl PluginRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// 注册插件（构建能力索引）
    pub fn register(&mut self, plugin: Arc<dyn Plugin>) {
        self.index.clear();
        self.plugins.push(plugin);
        self.reindex();
    }

    /// 重建能力索引
    fn reindex(&mut self) {
        for (idx, plugin) in self.plugins.iter().enumerate() {
            for cap in plugin.capabilities() {
                self.index.insert(cap, idx);
            }
        }
    }

    /// 加载全部插件（调用 on_load）
    pub fn load_all(&self, ctx: &PluginContext) -> Vec<(String, Result<()>)> {
        self.plugins
            .iter()
            .map(|p| {
                let name = p.meta().name;
                (name.clone(), p.on_load(ctx))
            })
            .collect()
    }

    /// 卸载全部插件
    pub fn unload_all(&self) {
        for plugin in &self.plugins {
            plugin.on_unload();
        }
    }

    /// 按能力名路由调用插件
    pub fn invoke(&self, capability: &str, input: &str, ctx: &PluginContext) -> Result<String> {
        let idx = self
            .index
            .get(capability)
            .ok_or_else(|| anyhow::anyhow!("无插件提供能力 `{capability}`"))?;
        self.plugins[*idx].execute(capability, input, ctx)
    }

    /// 已注册插件数
    pub fn len(&self) -> usize {
        self.plugins.len()
    }

    /// 是否为空
    pub fn is_empty(&self) -> bool {
        self.plugins.is_empty()
    }

    /// 列出所有插件元信息
    pub fn list_meta(&self) -> Vec<PluginMeta> {
        self.plugins.iter().map(|p| p.meta()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct EchoPlugin;

    impl Plugin for EchoPlugin {
        fn meta(&self) -> PluginMeta {
            PluginMeta {
                name: "echo".to_string(),
                version: "1.0.0".to_string(),
                description: "回声插件".to_string(),
                author: None,
            }
        }

        fn on_load(&self, _ctx: &PluginContext) -> Result<()> {
            Ok(())
        }

        fn capabilities(&self) -> Vec<String> {
            vec!["echo".to_string()]
        }

        fn execute(&self, _capability: &str, input: &str, _ctx: &PluginContext) -> Result<String> {
            Ok(format!("echo: {input}"))
        }
    }

    #[test]
    fn test_register_and_invoke() {
        let mut registry = PluginRegistry::new();
        registry.register(Arc::new(EchoPlugin));
        assert_eq!(registry.len(), 1);

        let ctx = PluginContext::default();
        let result = registry.invoke("echo", "hello", &ctx).unwrap();
        assert_eq!(result, "echo: hello");
    }

    #[test]
    fn test_invoke_unknown_capability() {
        let mut registry = PluginRegistry::new();
        registry.register(Arc::new(EchoPlugin));
        let ctx = PluginContext::default();
        assert!(registry.invoke("nope", "x", &ctx).is_err());
    }

    #[test]
    fn test_load_all() {
        let mut registry = PluginRegistry::new();
        registry.register(Arc::new(EchoPlugin));
        let results = registry.load_all(&PluginContext::default());
        assert_eq!(results.len(), 1);
        assert!(results[0].1.is_ok());
    }

    #[test]
    fn test_context_config() {
        let mut ctx = PluginContext::default();
        ctx.config.insert("key".to_string(), "value".to_string());
        assert_eq!(ctx.get("key"), Some("value"));
        assert_eq!(ctx.get("missing"), None);
    }

    #[test]
    fn test_list_meta() {
        let mut registry = PluginRegistry::new();
        registry.register(Arc::new(EchoPlugin));
        let metas = registry.list_meta();
        assert_eq!(metas[0].name, "echo");
        assert_eq!(metas[0].version, "1.0.0");
    }

    /// 声明能力但未实现 execute（走默认错误实现）
    struct CapOnlyPlugin;

    impl Plugin for CapOnlyPlugin {
        fn meta(&self) -> PluginMeta {
            PluginMeta {
                name: "caponly".to_string(),
                version: "1.0.0".to_string(),
                description: "声明能力但未实现动作".to_string(),
                author: None,
            }
        }

        fn capabilities(&self) -> Vec<String> {
            vec!["frobnicate".to_string()]
        }
    }

    #[test]
    fn test_invoke_unimplemented_action() {
        let mut registry = PluginRegistry::new();
        registry.register(Arc::new(CapOnlyPlugin));
        let ctx = PluginContext::default();
        let result = registry.invoke("frobnicate", "x", &ctx);
        assert!(result.is_err());
    }

    #[test]
    fn test_invoke_after_reindex() {
        // 注册第二个插件后，索引应仍能路由到已注册能力
        let mut registry = PluginRegistry::new();
        registry.register(Arc::new(EchoPlugin));
        registry.register(Arc::new(CapOnlyPlugin));
        let ctx = PluginContext::default();
        let result = registry.invoke("echo", "after", &ctx).unwrap();
        assert_eq!(result, "echo: after");
        assert!(registry.invoke("frobnicate", "x", &ctx).is_err());
    }

    #[test]
    fn test_unload_all() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static UNLOADS: AtomicUsize = AtomicUsize::new(0);

        struct UnloadPlugin;

        impl Plugin for UnloadPlugin {
            fn meta(&self) -> PluginMeta {
                PluginMeta {
                    name: "unload".to_string(),
                    version: "1.0.0".to_string(),
                    description: "卸载钩子".to_string(),
                    author: None,
                }
            }

            fn on_unload(&self) {
                UNLOADS.fetch_add(1, Ordering::SeqCst);
            }
        }

        let mut registry = PluginRegistry::new();
        registry.register(Arc::new(UnloadPlugin));
        registry.register(Arc::new(UnloadPlugin));
        registry.unload_all();
        assert_eq!(UNLOADS.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn test_empty_registry() {
        let registry = PluginRegistry::new();
        assert!(registry.is_empty());
        assert_eq!(registry.len(), 0);
        assert!(registry.list_meta().is_empty());
        assert!(registry.load_all(&PluginContext::default()).is_empty());
        registry.unload_all(); // 空注册表卸载不应 panic
        let ctx = PluginContext::default();
        assert!(registry.invoke("echo", "x", &ctx).is_err());
    }

    #[test]
    fn test_context_capabilities() {
        let mut ctx = PluginContext::default();
        ctx.capabilities
            .insert("http".to_string(), "allow".to_string());
        assert_eq!(
            ctx.capabilities.get("http").map(String::as_str),
            Some("allow")
        );
    }
}
