//! 模型注册表（Phase H）— 多命名模型档案 + 自定义 API 接入点
//!
//! 解决「没有模型选择及自定义接入」痛点：
//! - 多命名档案（ModelProfile），支持云 / 本地 / 自定义三类接入点
//! - 本地模型仅作为 custom 接入点，不预内置（用户 2026-08-05 决议 D-03）
//! - 选区：active（主模型）+ copilot（Bloom 副模型）
//! - 持久化到 `models.json`（与 `config.yaml` 同目录）

use std::collections::HashMap;
use std::path::Path;

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};

use crate::model::llm::{LLMConfig, LLMProvider};

/// 档案类别
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ProfileKind {
    /// 云端 API（OpenAI / DeepSeek 等）
    #[default]
    Cloud,
    /// 本地模型（llama.cpp / Ollama 等），仅经 custom 接入点
    Local,
    /// 用户自定义（任意 OpenAI 兼容端点）
    Custom,
}

fn default_temperature() -> f32 {
    0.7
}
fn default_max_tokens() -> u32 {
    4096
}

/// 模型档案（一个命名的接入点）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelProfile {
    /// 唯一 ID（如 "deepseek-chat" / "my-local-llama"）
    pub id: String,
    /// 展示名
    pub label: String,
    /// 提供商标识（openai / deepseek / custom / ollama ...）
    pub provider: String,
    /// API 基址（custom / 本地必填）
    pub base_url: String,
    /// API 密钥（BYOK；本地可空）
    #[serde(default)]
    pub api_key: String,
    /// 实际模型名
    pub model: String,
    /// 类别
    #[serde(default)]
    pub kind: ProfileKind,
    /// 是否内置便利模板（不可被误删至空）
    #[serde(default)]
    pub builtin: bool,
    /// 采样温度
    #[serde(default = "default_temperature")]
    pub temperature: f32,
    /// 最大生成 token
    #[serde(default = "default_max_tokens")]
    pub max_tokens: u32,
}

impl ModelProfile {
    /// 转 LLMConfig（供 LLMClient 使用）
    pub fn to_llm_config(&self) -> LLMConfig {
        let provider = match self.provider.as_str() {
            "openai" => LLMProvider::OpenAI,
            "deepseek" => LLMProvider::DeepSeek,
            _ => LLMProvider::Custom {
                base_url: self.base_url.clone(),
            },
        };
        LLMConfig {
            provider,
            model: self.model.clone(),
            api_key: self.api_key.clone(),
            base_url: self.base_url.clone(),
            max_tokens: self.max_tokens,
            temperature: self.temperature,
            timeout_secs: 60,
            thinking: crate::model::thinking::ThinkingEffort::Off,
            context_window: crate::model::thinking::CONTEXT_WINDOW_DEFAULT,
        }
    }
}

/// 模型注册表
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ModelRegistry {
    /// 命名档案表
    #[serde(default)]
    pub profiles: HashMap<String, ModelProfile>,
    /// 当前主模型 ID
    #[serde(default)]
    pub active: String,
    /// 当前 Bloom 副模型 ID（可选）
    #[serde(default)]
    pub copilot: Option<String>,
}

impl ModelRegistry {
    /// 种子：仅云端便利模板，不内置任何本地模型（D-03 决议）
    pub fn with_seeds() -> Self {
        let mut profiles = HashMap::new();
        profiles.insert(
            "deepseek-v4-pro".to_string(),
            ModelProfile {
                id: "deepseek-v4-pro".into(),
                label: "DeepSeek V4 Pro".into(),
                provider: "deepseek".into(),
                base_url: "https://api.deepseek.com".into(),
                api_key: String::new(),
                model: "deepseek-v4-pro".into(),
                kind: ProfileKind::Cloud,
                builtin: true,
                temperature: 0.7,
                max_tokens: 4096,
            },
        );
        profiles.insert(
            "openai-compatible".to_string(),
            ModelProfile {
                id: "openai-compatible".into(),
                label: "OpenAI 兼容（自定义）".into(),
                provider: "custom".into(),
                base_url: "https://api.openai.com/v1".into(),
                api_key: String::new(),
                model: "gpt-4o-mini".into(),
                kind: ProfileKind::Custom,
                builtin: true,
                temperature: 0.7,
                max_tokens: 4096,
            },
        );
        Self {
            profiles,
            active: "deepseek-v4-pro".into(),
            copilot: None,
        }
    }

    /// 从文件加载；文件缺失或为空则返回种子默认值
    pub fn load(path: &Path) -> Result<Self> {
        if path.exists() {
            let s = std::fs::read_to_string(path)?;
            let reg: ModelRegistry = serde_json::from_str(&s)?;
            if reg.profiles.is_empty() {
                return Ok(Self::with_seeds());
            }
            Ok(reg)
        } else {
            Ok(Self::with_seeds())
        }
    }

    /// 保存到文件（确保父目录存在）
    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(p) = path.parent() {
            std::fs::create_dir_all(p)?;
        }
        std::fs::write(path, serde_json::to_string_pretty(self)?)?;
        Ok(())
    }

    /// 取档案
    pub fn get(&self, id: &str) -> Option<&ModelProfile> {
        self.profiles.get(id)
    }

    /// 列表（按 ID 排序）
    pub fn list(&self) -> Vec<&ModelProfile> {
        let mut v: Vec<&ModelProfile> = self.profiles.values().collect();
        v.sort_by(|a, b| a.id.cmp(&b.id));
        v
    }

    /// 新增档案（ID 不可重复）
    pub fn add(&mut self, p: ModelProfile) -> Result<()> {
        if self.profiles.contains_key(&p.id) {
            return Err(anyhow!("档案已存在: {}", p.id));
        }
        self.profiles.insert(p.id.clone(), p);
        Ok(())
    }

    /// 新增或覆盖档案
    pub fn upsert(&mut self, p: ModelProfile) {
        self.profiles.insert(p.id.clone(), p);
    }

    /// 删除档案（至少保留一个；active 不可删空）
    pub fn remove(&mut self, id: &str) -> Result<ModelProfile> {
        if !self.profiles.contains_key(id) {
            return Err(anyhow!("档案不存在: {}", id));
        }
        if self.profiles.len() <= 1 {
            return Err(anyhow!("至少保留一个模型档案"));
        }
        let removed = self.profiles.remove(id).unwrap();
        if id == self.active {
            self.active = self.profiles.keys().next().cloned().unwrap_or_default();
        }
        if self.copilot.as_deref() == Some(id) {
            self.copilot = None;
        }
        Ok(removed)
    }

    /// 设置主模型
    pub fn set_active(&mut self, id: &str) -> Result<()> {
        if !self.profiles.contains_key(id) {
            return Err(anyhow!("档案不存在: {}", id));
        }
        self.active = id.to_string();
        Ok(())
    }

    /// 设置 Bloom 副模型（None 表示关闭双模型）
    pub fn set_copilot(&mut self, id: Option<String>) -> Result<()> {
        if let Some(i) = &id {
            if !self.profiles.contains_key(i) {
                return Err(anyhow!("档案不存在: {}", i));
            }
        }
        self.copilot = id;
        Ok(())
    }

    /// 当前主模型档案
    pub fn active_profile(&self) -> Option<&ModelProfile> {
        self.profiles.get(&self.active)
    }

    /// 当前 Bloom 副模型档案
    pub fn copilot_profile(&self) -> Option<&ModelProfile> {
        self.copilot.as_ref().and_then(|i| self.profiles.get(i))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn custom_profile(id: &str, base: &str) -> ModelProfile {
        ModelProfile {
            id: id.into(),
            label: id.into(),
            provider: "custom".into(),
            base_url: base.into(),
            api_key: "k".into(),
            model: "m".into(),
            kind: ProfileKind::Local,
            builtin: false,
            temperature: 0.5,
            max_tokens: 2048,
        }
    }

    #[test]
    fn test_seeds_only_cloud_no_local() {
        let reg = ModelRegistry::with_seeds();
        assert!(reg.profiles.contains_key("deepseek-v4-pro"));
        assert!(reg.profiles.contains_key("openai-compatible"));
        assert!(reg.active_profile().is_some());
        // 无任何 kind==Local 的内置档案
        assert!(!reg
            .list()
            .iter()
            .any(|p| p.kind == ProfileKind::Local && p.builtin));
    }

    #[test]
    fn test_add_and_set_active() {
        let mut reg = ModelRegistry::with_seeds();
        reg.add(custom_profile("local-llama", "http://localhost:11434/v1"))
            .unwrap();
        assert!(reg.get("local-llama").is_some());
        reg.set_active("local-llama").unwrap();
        assert_eq!(reg.active, "local-llama");
        assert_eq!(
            reg.active_profile().unwrap().base_url,
            "http://localhost:11434/v1"
        );
    }

    #[test]
    fn test_add_duplicate_fails() {
        let mut reg = ModelRegistry::with_seeds();
        let p = reg.get("deepseek-v4-pro").unwrap().clone();
        assert!(reg.add(p).is_err());
    }

    #[test]
    fn test_copilot_set_and_clear() {
        let mut reg = ModelRegistry::with_seeds();
        reg.set_copilot(Some("openai-compatible".into())).unwrap();
        assert!(reg.copilot_profile().is_some());
        reg.set_copilot(None).unwrap();
        assert!(reg.copilot_profile().is_none());
    }

    #[test]
    fn test_remove_protects_minimum() {
        let mut reg = ModelRegistry::with_seeds();
        // 删到只剩一个应失败
        reg.remove("openai-compatible").unwrap();
        assert!(reg.remove("deepseek-v4-pro").is_err());
        // active 在删除后自动迁移
        assert_eq!(reg.active, "deepseek-v4-pro");
    }

    #[test]
    fn test_remove_clears_copilot() {
        let mut reg = ModelRegistry::with_seeds();
        reg.add(custom_profile("x", "http://x/v1")).unwrap();
        reg.set_copilot(Some("x".into())).unwrap();
        reg.remove("x").unwrap();
        assert!(reg.copilot_profile().is_none());
    }

    #[test]
    fn test_to_llm_config_mapping() {
        let reg = ModelRegistry::with_seeds();
        let ds = reg.get("deepseek-v4-pro").unwrap();
        let cfg = ds.to_llm_config();
        assert!(matches!(cfg.provider, LLMProvider::DeepSeek));
        let oc = reg.get("openai-compatible").unwrap();
        let cfg2 = oc.to_llm_config();
        assert!(matches!(cfg2.provider, LLMProvider::Custom { .. }));
    }

    #[test]
    fn test_save_and_load_roundtrip() {
        let tmp = std::env::temp_dir().join("marguerite_reg_test/models.json");
        let mut reg = ModelRegistry::with_seeds();
        reg.add(custom_profile("ll", "http://ll/v1")).unwrap();
        reg.set_copilot(Some("ll".into())).unwrap();
        reg.save(&tmp).unwrap();
        let loaded = ModelRegistry::load(&tmp).unwrap();
        assert_eq!(loaded.profiles.len(), reg.profiles.len());
        assert_eq!(loaded.copilot.as_deref(), Some("ll"));
        assert!(loaded.get("ll").is_some());
        let _ = std::fs::remove_dir_all(tmp.parent().unwrap());
    }

    #[test]
    fn test_load_missing_returns_seeds() {
        let missing = std::env::temp_dir().join("marguerite_reg_missing/models.json");
        let reg = ModelRegistry::load(&missing).unwrap();
        assert!(reg.profiles.contains_key("deepseek-v4-pro"));
    }
}
