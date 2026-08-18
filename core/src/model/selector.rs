//! 模型选择器（Phase H2）— 注册表 + 配置目录路径 + 即时切换 + 密钥回退
//!
//! 三形态（CLI / TUI / GUI）共用同一份 `models.json`（与 `config.yaml` 同目录）。
//! 选择器负责加载、切换、持久化，并在解析最终 LLMConfig 时套用 BYOK 回退链。

use std::path::PathBuf;

use anyhow::Result;

use crate::model::byok;
use crate::model::llm::LLMConfig;
use crate::model::registry::{ModelProfile, ModelRegistry, ProfileKind};
use crate::persistence::config::Config;

/// `models.json` 路径（`~/.marguerite/models.json`）
pub fn models_path() -> PathBuf {
    Config::config_dir().join("models.json")
}

/// 模型选择器：包裹 `ModelRegistry` + 持久化路径
pub struct ModelSelector {
    registry: ModelRegistry,
    path: PathBuf,
}

impl ModelSelector {
    /// 从默认路径加载（缺失则种子默认值）
    pub fn load() -> Result<Self> {
        let path = models_path();
        let registry = ModelRegistry::load(&path)?;
        Ok(Self { registry, path })
    }

    /// 从指定路径加载（测试用）
    pub fn with_path(path: PathBuf) -> Result<Self> {
        let registry = ModelRegistry::load(&path)?;
        Ok(Self { registry, path })
    }

    /// 持久化（写后收紧 0600）
    pub fn save(&self) -> Result<()> {
        self.registry.save(&self.path)?;
        byok::chmod_0600(&self.path)?;
        Ok(())
    }

    pub fn registry(&self) -> &ModelRegistry {
        &self.registry
    }

    /// 切换主模型并保存
    pub fn use_active(&mut self, id: &str) -> Result<()> {
        self.registry.set_active(id)?;
        self.save()
    }

    /// 设置 / 关闭 Bloom 副模型并保存
    pub fn use_copilot(&mut self, id: Option<String>) -> Result<()> {
        self.registry.set_copilot(id)?;
        self.save()
    }

    /// 新增档案并保存
    pub fn add(&mut self, p: ModelProfile) -> Result<()> {
        self.registry.add(p)?;
        self.save()
    }

    /// 删除档案并保存
    pub fn remove(&mut self, id: &str) -> Result<ModelProfile> {
        let removed = self.registry.remove(id)?;
        self.save()?;
        Ok(removed)
    }

    /// 解析主模型的最终 LLMConfig（api_key 经 BYOK 回退链）
    pub fn active_config(&self) -> Option<LLMConfig> {
        self.registry.active_profile().map(|p| {
            let mut cfg = p.to_llm_config();
            cfg.api_key = byok::resolve_api_key(&p.id, &p.api_key, None);
            cfg
        })
    }

    /// 解析 Bloom 副模型的最终 LLMConfig（若有）
    pub fn copilot_config(&self) -> Option<LLMConfig> {
        self.registry.copilot_profile().map(|p| {
            let mut cfg = p.to_llm_config();
            cfg.api_key = byok::resolve_api_key(&p.id, &p.api_key, None);
            cfg
        })
    }
}

/// 由命令行 flag 构造档案（供 `model add` 使用）
pub fn profile_from_flags(
    flags: &std::collections::HashMap<String, String>,
) -> Result<ModelProfile> {
    let id = flags
        .get("id")
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("缺少 --id"))?;
    let base_url = flags
        .get("base-url")
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("缺少 --base-url"))?;
    let model = flags
        .get("model")
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("缺少 --model"))?;
    let provider = flags
        .get("provider")
        .cloned()
        .unwrap_or_else(|| "custom".into());
    let label = flags.get("label").cloned().unwrap_or_else(|| id.clone());
    let api_key = flags.get("api-key").cloned().unwrap_or_default();
    let kind = match flags.get("kind").map(String::as_str) {
        Some("local") => ProfileKind::Local,
        Some("custom") => ProfileKind::Custom,
        _ => ProfileKind::Cloud,
    };
    Ok(ModelProfile {
        id,
        label,
        provider,
        base_url,
        api_key,
        model,
        kind,
        builtin: false,
        temperature: 0.7,
        max_tokens: 4096,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("marguerite_sel_{name}"))
    }

    #[test]
    fn test_selector_switch_and_persist() {
        let p = tmp_path("switch/models.json");
        let _ = std::fs::remove_dir_all(p.parent().unwrap());
        let mut sel = ModelSelector::with_path(p.clone()).unwrap();
        let prof = ModelProfile {
            id: "local-llama".into(),
            label: "Local Llama".into(),
            provider: "ollama".into(),
            base_url: "http://localhost:11434/v1".into(),
            api_key: String::new(),
            model: "llama3".into(),
            kind: ProfileKind::Local,
            builtin: false,
            temperature: 0.7,
            max_tokens: 4096,
        };
        sel.add(prof).unwrap();
        sel.use_active("local-llama").unwrap();
        // 重新加载确认持久化
        let reloaded = ModelSelector::with_path(p).unwrap();
        assert_eq!(reloaded.registry().active, "local-llama");
        assert!(reloaded.registry().get("local-llama").is_some());
    }

    #[test]
    fn test_selector_copilot_cycle() {
        let p = tmp_path("copilot/models.json");
        let _ = std::fs::remove_dir_all(p.parent().unwrap());
        let mut sel = ModelSelector::with_path(p).unwrap();
        sel.add(ModelProfile {
            id: "cop".into(),
            label: "Cop".into(),
            provider: "custom".into(),
            base_url: "https://x/v1".into(),
            api_key: String::new(),
            model: "m".into(),
            kind: ProfileKind::Custom,
            builtin: false,
            temperature: 0.7,
            max_tokens: 4096,
        })
        .unwrap();
        sel.use_copilot(Some("cop".into())).unwrap();
        assert_eq!(sel.registry().copilot.as_deref(), Some("cop"));
        sel.use_copilot(None).unwrap();
        assert!(sel.registry().copilot.is_none());
    }

    #[test]
    fn test_profile_from_flags_missing_id() {
        let flags = std::collections::HashMap::new();
        assert!(profile_from_flags(&flags).is_err());
    }

    #[test]
    fn test_profile_from_flags_ok() {
        let mut flags = std::collections::HashMap::new();
        flags.insert("id".into(), "my-openai".into());
        flags.insert("base-url".into(), "https://api.openai.com/v1".into());
        flags.insert("model".into(), "gpt-4o".into());
        flags.insert("provider".into(), "openai".into());
        flags.insert("kind".into(), "cloud".into());
        let p = profile_from_flags(&flags).unwrap();
        assert_eq!(p.id, "my-openai");
        assert_eq!(p.kind, ProfileKind::Cloud);
        assert_eq!(p.api_key, "");
    }
}
