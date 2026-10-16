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
use crate::model::tier::{ModelTier, TierCaps};

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
    /// API 密钥（BYOK；本地可空；serde 序列化跳过 —— 密钥绝不落 models.json）
    #[serde(default, skip_serializing)]
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
    /// 所属模型档位（三档帕累托分层，5c）。serde default=Balanced，
    /// 旧 models.json 与内置种子零漂移加载（未标注档的档案落到均衡档）。
    #[serde(default)]
    pub tier: ModelTier,
    /// 实测能力向量（custom 端点可自填；种子档缺省=按所属档准入线）。
    /// serde default 保证旧文件无损加载。
    #[serde(default)]
    pub tier_caps: TierCaps,
    /// `tier_caps` 是否**由作者显式声明**（而非 serde 缺省填充）。
    ///
    /// 这个布尔字段是必需的，不能靠「caps == TierCaps::default()」反推：
    /// 一个恰好把能力填成均衡档数值的档案，与"从未填写"在数值上完全无法区分。
    /// 路由端的档位准入（`tier.rs::filter_chain`）必须据此区分两种情况——
    /// false → 信任其档位声明但标记为"未验证"；true → 用 caps 严格卡准入线。
    /// 若不分这两种情况，当前种子数据（只标 tier、未写 caps）会让旗舰档候选链变空。
    #[serde(default)]
    pub tier_caps_declared: bool,
}

impl ModelProfile {
    /// 转 LLMConfig（供 LLMClient 使用）
    pub fn to_llm_config(&self) -> LLMConfig {
        let provider = match self.provider.as_str() {
            "openai" => LLMProvider::OpenAI,
            "deepseek" => LLMProvider::DeepSeek,
            "anthropic" => LLMProvider::Anthropic,
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

impl ModelRegistry {
    /// 解析某档位的可用候选链（M2：把档位准入真正接进选路）。
    ///
    /// 链序来自内嵌 `tier_rules.json`（数据驱动：改数据文件即可调整档内候选顺序）。
    /// 每个 id 经 [`crate::model::tier::filter_chain`] 得到四类判定之一：
    /// 准入 / 拒绝（附逐项原因，供控制中心回显）/ 能力未声明（信任档位声明但标记未验证）
    /// / 档案缺失（数据漂移，不得当作可用）。
    ///
    /// 调用方（路由器、控制中心、doctor）应据 `rejected` 与 `missing` 给出可解释的
    /// "为何没选到它"，对齐设计稿 §3.1 的 `RouteDecision.why` 可观测要求。
    pub fn tier_chain(&self, tier: ModelTier) -> crate::model::tier::TierChain {
        let rules = crate::model::tier::TierRules::from_embedded();
        crate::model::tier::filter_chain(rules.chain_for(tier), tier, |id| {
            self.profiles
                .get(id)
                .map(|p| (p.tier_caps, p.tier_caps_declared))
        })
    }

    /// 档位能力声明的审计结果（供 doctor / 启动自检 / 控制中心使用）。
    ///
    /// 为什么需要它：档位准入（`tier.rs::filter_chain`）只在档案**显式声明**了
    /// `tier_caps` 时才真正过滤；未声明的档案按"信任档位声明"放行。
    /// 于是"有多少档案其实是未经校验的"就成了必须**可见**的运维事实——
    /// 否则用户以为自己在用经过能力校验的旗舰档，实际全是未声明的空壳。
    ///
    /// 另外，`tier_caps` 里的单价会用于预算预扣（设计稿 §6），
    /// 所以"声明了但与档位不符"（例如标旗舰却只支持 Off 推理）必须被指出，
    /// 而不是等到账单或降级行为异常时才发现。
    pub fn audit_tier_caps(&self) -> TierCapsAudit {
        let mut ids: Vec<&String> = self.profiles.keys().collect();
        ids.sort(); // 确定性输出，便于 diff 与测试
        let mut out = TierCapsAudit::default();
        for id in ids {
            let Some(p) = self.profiles.get(id) else { continue };
            if !p.tier_caps_declared {
                out.unverified.push(id.clone());
                continue;
            }
            let reasons = p.tier_caps.reject_reasons(p.tier);
            if reasons.is_empty() {
                out.ok.push(id.clone());
            } else {
                out.mismatched.push((id.clone(), p.tier, reasons));
            }
        }
        out
    }
}

/// 档位能力声明审计结果。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TierCapsAudit {
    /// 已声明且与所属档位一致
    pub ok: Vec<String>,
    /// **未声明能力**（按档位声明放行，但未经校验）—— 需要补数据的清单
    pub unverified: Vec<String>,
    /// 已声明但与所属档位矛盾（含逐项原因），必须修正
    pub mismatched: Vec<(String, ModelTier, Vec<String>)>,
}

impl TierCapsAudit {
    /// 是否存在需要人工处理的问题（未声明与声明矛盾都算）。
    pub fn has_issues(&self) -> bool {
        !self.unverified.is_empty() || !self.mismatched.is_empty()
    }

    /// 一行摘要，供 doctor 直接打印。
    pub fn summary(&self) -> String {
        format!(
            "档位能力声明：已校验 {} / 未声明 {} / 声明矛盾 {}",
            self.ok.len(),
            self.unverified.len(),
            self.mismatched.len()
        )
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
    /// 内置种子版本；load 据此把旧版 models.json 增量补齐缺失的内置模板
    /// （不复活用户已删除的档案：补齐后版本号立即升到当前值）。
    #[serde(default)]
    pub seed_revision: u32,
}

/// 当前内置种子版本。每次新增/变更内置云端模板时 +1，旧文件自动迁移。
/// v4：+1 → 腾讯混元 Hunyuan（Hy3 Preview / 2.0 Instruct）静态档案
/// v5：+1 → 三档模型分层（5c）：给全部种子标注 tier（旗舰/均衡/极速），
///        ModelProfile 增加 tier/tier_caps（serde default，旧文件无损加载）。
const SEED_REVISION: u32 = 5;

/// 内嵌种子模型数据（数据驱动，不再硬编码版本号到 Rust 源码）。
///
/// 由 `seed_models.json` 经 `include_str!` 在编译期嵌入；新增/换代模型家族只需
/// 改该数据文件（或运行时经 `models.seed.json` 覆盖层热更新，见
/// [`ModelSelector::reload_seeds`](crate::model::selector::ModelSelector::reload_seeds)）。
const SEED_MODELS_JSON: &str = include_str!("seed_models.json");

impl ModelRegistry {
    /// 种子：仅云端便利模板，不内置任何本地模型（D-03 决议）。
    ///
    /// 种子档案已外置为 `seed_models.json`（编译期经 `include_str!` 内嵌），
    /// 不再把具体模型版本号硬编码进 Rust 源码——新增/换代模型家族（如 DeepSeek V4 /
    /// Qwen3.8 / GLM-5.x / GPT-5.6）只需更新数据文件，或运行时经 `models.seed.json`
    /// 覆盖层热更新（见 [`ModelSelector::reload_seeds`]），无需重编译。
    pub fn with_seeds() -> Self {
        serde_json::from_str(SEED_MODELS_JSON)
            .expect("内嵌种子模型 JSON 损坏，无法启动（请检查 core/src/model/seed_models.json）")
    }

    /// 合并另一注册表的「内置种子」档案。
    ///
    /// 语义（与 `load` 的版本迁移一致）：
    /// - 仅补齐当前缺失的 ID（`entry().or_insert`，绝不覆盖用户已编辑/自定义的档案）；
    /// - 不改动用户的 `active` / `copilot` 选区；
    /// - `seed_revision` 取两者较大值（用于运行时热更新升级版本号）。
    ///
    /// 用于旧版 `models.json` 增量迁移（`load`）与运行时覆盖层热更新（`reload_seeds`）。
    pub fn merge_seeds(&mut self, other: &ModelRegistry) {
        for (id, profile) in &other.profiles {
            self.profiles
                .entry(id.clone())
                .or_insert_with(|| profile.clone());
        }
        if self.seed_revision < other.seed_revision {
            self.seed_revision = other.seed_revision;
        }
    }

    /// 从文件加载；文件缺失或为空则返回种子默认值。
    ///
    /// 旧版 models.json（无 seed_revision 或版本更低）会增量补齐缺失的内置模板，
    /// 并把版本升到当前值；用户已删除的档案不会被复活（补齐后一旦保存，下次不再合并）。
    pub fn load(path: &Path) -> Result<Self> {
        if path.exists() {
            let s = std::fs::read_to_string(path)?;
            let mut reg: ModelRegistry = serde_json::from_str(&s)?;
            if reg.profiles.is_empty() {
                return Ok(Self::with_seeds());
            }
            if reg.seed_revision < SEED_REVISION {
                let seeds = Self::with_seeds();
                for (id, profile) in seeds.profiles {
                    reg.profiles.entry(id).or_insert(profile);
                }
                reg.seed_revision = SEED_REVISION;
                // 防旧文件 active 指向已删档案：迁移后回退到第一个存在档案
                if !reg.profiles.contains_key(&reg.active) {
                    reg.active = reg.profiles.keys().next().cloned().unwrap_or_default();
                }
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
            tier: ModelTier::Balanced,
            tier_caps: TierCaps::default(),
            tier_caps_declared: false,
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
        // 删到只剩一个应失败（种子已扩至多云家族：deepseek + openai + custom/openrouter）
        let all: Vec<String> = reg.profiles.keys().cloned().collect();
        for id in all {
            if id != "deepseek-v4-pro" {
                reg.remove(&id).unwrap();
            }
        }
        assert!(reg.remove("deepseek-v4-pro").is_err());
        // active 不会被误删；删除其余种子后主模型仍为 pro
        assert_eq!(reg.active, "deepseek-v4-pro");
    }

    #[test]
    fn test_seeds_include_deepseek_family() {
        let reg = ModelRegistry::with_seeds();
        for id in [
            "deepseek-v4-pro",
            "deepseek-v4-flash",
            "deepseek-v4-flash-vision-exp",
        ] {
            let p = reg.get(id).expect(id);
            assert_eq!(p.kind, ProfileKind::Cloud);
            assert!(p.builtin, "{id} 应为内置便利模板");
            assert_eq!(p.provider, "deepseek");
        }
        assert!(reg.get("openai-compatible").is_some());
    }

    #[test]
    fn test_seeds_include_multi_provider() {
        let reg = ModelRegistry::with_seeds();
        // DeepSeek 家族
        for id in [
            "deepseek-v4-pro",
            "deepseek-v4-flash",
            "deepseek-v4-flash-vision-exp",
            "deepseek-chat",
        ] {
            let p = reg.get(id).expect(id);
            assert_eq!(p.provider, "deepseek");
            assert!(p.builtin, "{id} 应为内置便利模板");
        }
        // OpenAI 官方
        for id in ["openai-gpt-5-5", "openai-gpt-4o-mini"] {
            let p = reg.get(id).expect(id);
            assert_eq!(p.provider, "openai");
            assert_eq!(p.base_url, "https://api.openai.com/v1");
            assert!(p.builtin, "{id} 应为内置便利模板");
        }
        // Anthropic 原生
        for id in ["anthropic-claude-sonnet-4-6", "anthropic-claude-opus-4-7"] {
            let p = reg.get(id).expect(id);
            assert_eq!(p.provider, "anthropic");
            assert!(p.builtin, "{id} 应为内置便利模板");
        }
        // 聚合/自定义入口
        assert!(reg.get("openrouter").is_some());
        assert!(reg.get("openai-compatible").is_some());
        // Hunyuan static profiles are discoverable in the same registry.
        assert!(reg.get("hunyuan-hy3-preview").is_some());
        assert!(reg.get("hunyuan-2-0-instruct").is_some());
        // provider 面应覆盖 deepseek + openai + anthropic + custom
        let providers: std::collections::BTreeSet<&str> =
            reg.list().iter().map(|p| p.provider.as_str()).collect();
        assert!(providers.contains("deepseek"));
        assert!(providers.contains("openai"));
        assert!(providers.contains("anthropic"));
        assert!(providers.contains("custom"));
    }

    #[test]
    fn test_load_merges_missing_builtin_seed_profiles() {
        let base =
            std::env::temp_dir().join(format!("marguerite_reg_migrate_{}", std::process::id()));
        let path = base.join("models.json");
        // 模拟旧版文件：少了新增内置模板、seed_revision=0（旧文件无该字段）
        let mut old = ModelRegistry::with_seeds();
        old.seed_revision = 0;
        old.profiles.remove("openai-gpt-5-5").unwrap();
        old.profiles.remove("deepseek-chat").unwrap();
        old.profiles.remove("openrouter").unwrap();
        old.save(&path).unwrap();

        let loaded = ModelRegistry::load(&path).unwrap();
        assert!(
            loaded.get("openai-gpt-5-5").is_some(),
            "旧文件应补齐新增 OpenAI 种子"
        );
        assert!(
            loaded.get("deepseek-chat").is_some(),
            "旧文件应补齐 DeepSeek Chat"
        );
        assert!(
            loaded.get("openrouter").is_some(),
            "旧文件应补齐 OpenRouter 聚合入口"
        );
        assert_eq!(loaded.seed_revision, SEED_REVISION);
        // 用户已有档案保留
        assert!(loaded.profiles.contains_key("deepseek-v4-pro"));
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn test_load_does_not_resurrect_removed_builtins_after_revision() {
        let base =
            std::env::temp_dir().join(format!("marguerite_reg_removed_{}", std::process::id()));
        let path = base.join("models.json");
        let mut reg = ModelRegistry::with_seeds();
        // 当前版本下用户主动删除内置模板 → 保存后不应再被 load 复活
        reg.seed_revision = SEED_REVISION;
        reg.profiles.remove("openai-gpt-5-5").unwrap();
        reg.save(&path).unwrap();

        let loaded = ModelRegistry::load(&path).unwrap();
        assert!(
            loaded.get("openai-gpt-5-5").is_none(),
            "当前版本删除的内置档案不应被复活"
        );
        assert_eq!(loaded.seed_revision, SEED_REVISION);
        let _ = std::fs::remove_dir_all(&base);
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
        let an = reg.get("anthropic-claude-sonnet-4-6").unwrap();
        let cfg3 = an.to_llm_config();
        assert!(matches!(cfg3.provider, LLMProvider::Anthropic));
        assert_eq!(cfg3.base_url, "https://api.anthropic.com");
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

    #[test]
    fn test_merge_seeds_keeps_user_profile_and_active() {
        // 用户自定义档案 + 自定义主模型，merge 内置种子后：
        // - 用户档案不被覆盖；- active 不被改动；- seed_revision 升到内置版本。
        let mut reg = ModelRegistry::default();
        reg.profiles.insert(
            "my-custom".to_string(),
            ModelProfile {
                id: "my-custom".into(),
                label: "My Custom".into(),
                provider: "custom".into(),
                base_url: "http://localhost:11434/v1".into(),
                api_key: String::new(),
                model: "llama3".into(),
                kind: ProfileKind::Local,
                builtin: false,
                temperature: 0.5,
                max_tokens: 2048,
                tier: ModelTier::Balanced,
                tier_caps: TierCaps::default(),
                tier_caps_declared: false,
            },
        );
        reg.active = "my-custom".into();
        reg.seed_revision = 0;

        reg.merge_seeds(&ModelRegistry::with_seeds());

        // 用户档案保留
        assert!(reg.get("my-custom").is_some());
        // active 不变
        assert_eq!(reg.active, "my-custom");
        // 内置种子补齐
        assert!(reg.get("deepseek-v4-pro").is_some());
        assert!(reg.get("openai-gpt-5-5").is_some());
        // 版本号升到内置
        assert_eq!(reg.seed_revision, SEED_REVISION);
    }

    #[test]
    fn test_seeds_include_hunyuan_profile() {
        let reg = ModelRegistry::with_seeds();
        let profile = reg
            .get("hunyuan-2-0-instruct")
            .expect("Hunyuan seed profile should be discoverable");
        assert_eq!(profile.provider, "hunyuan");
        assert_eq!(profile.model, "hunyuan-2.0-instruct-20251111");
        assert!(profile.builtin);
    }

    #[test]
    fn test_with_seeds_loads_from_embedded_json() {
        // 种子模型已从 Rust 字面量外置为 seed_models.json：解析可得全部内置档案，
        // 且 active 默认指向 deepseek-v4-pro。
        let reg = ModelRegistry::with_seeds();
        assert!(reg.get("deepseek-v4-pro").is_some());
        assert!(reg.get("anthropic-claude-opus-4-7").is_some());
        assert!(reg.get("gemini-2-5-pro").is_some());
        assert_eq!(reg.active, "deepseek-v4-pro");
        assert_eq!(reg.seed_revision, SEED_REVISION);
        // 任何种子都不含本地模型（D-03 决议）
        assert!(!reg
            .list()
            .iter()
            .any(|p| p.kind == ProfileKind::Local && p.builtin));
    }

    #[test]
    fn test_seed_profiles_carry_tier_labels() {
        // M1 门禁：22 个内置档全部标了档位；旗舰/极速示例与设计稿一致。
        use crate::model::tier::ModelTier;
        let reg = ModelRegistry::with_seeds();
        assert_eq!(reg.seed_revision, 5, "seed_revision 必须升到 5");
        // 旗舰示例
        for id in ["deepseek-v4-pro", "anthropic-claude-opus-4-7", "gemini-2-5-pro"] {
            assert_eq!(
                reg.get(id).unwrap().tier,
                ModelTier::Flagship,
                "{id} 应为旗舰档"
            );
        }
        // 极速示例
        for id in ["anthropic-claude-haiku-4-5", "openai-gpt-4o-mini"] {
            assert_eq!(
                reg.get(id).unwrap().tier,
                ModelTier::Turbo,
                "{id} 应为极速档"
            );
        }
        // 均衡示例
        for id in ["anthropic-claude-sonnet-4-6", "zhipu-glm-5-3", "dashscope-qwen-max"] {
            assert_eq!(
                reg.get(id).unwrap().tier,
                ModelTier::Balanced,
                "{id} 应为均衡档"
            );
        }
        // 每个档都有 tier_caps（default 即均衡准入线），不panic
        for p in reg.list() {
            let _ = p.tier_caps.context_window;
        }
    }

    #[test]
    fn test_old_models_json_without_tier_field_loads_losslessly() {
        // M1 门禁：没有 tier/tier_caps 字段的旧 models.json 必须零漂移加载
        // （serde default 落到 Balanced），且不丢原有 profiles。
        let base =
            std::env::temp_dir().join(format!("marguerite_reg_oldtier_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        let path = base.join("models.json");
        // 旧格式：完全没有 tier/tier_caps/seed_revision 字段
        let old = r#"{
  "profiles": {
    "custom-legacy": {
      "id": "custom-legacy",
      "label": "Legacy",
      "provider": "custom",
      "base_url": "http://localhost:11434/v1",
      "model": "llama3",
      "kind": "custom",
      "builtin": false,
      "temperature": 0.5,
      "max_tokens": 2048
    }
  },
  "active": "custom-legacy",
  "copilot": null
}"#;
        std::fs::write(&path, old).unwrap();
        let loaded = ModelRegistry::load(&path).unwrap();
        let p = loaded.get("custom-legacy").expect("旧档案必须保留");
        assert_eq!(p.tier, ModelTier::Balanced, "缺省档应落到均衡档");
        assert_eq!(p.model, "llama3", "原有字段不得丢失");
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn audit_tier_caps_splits_three_buckets() {
        // 审计器是"档位准入是否真的生效"的可见出口：未声明能力必须与
        // 「已声明且达标」「已声明但矛盾」分开报，否则用户无从知道自己的
        // 旗舰档到底有没有经过能力校验（design §3.1 的可解释要求）。
        fn prof(id: &str, tier: ModelTier, caps: TierCaps, declared: bool) -> ModelProfile {
            ModelProfile {
                id: id.into(),
                label: id.into(),
                provider: "custom".into(),
                base_url: "https://example.invalid".into(),
                api_key: String::new(),
                model: id.into(),
                kind: ProfileKind::Custom,
                builtin: false,
                temperature: 0.7,
                max_tokens: 4096,
                tier,
                tier_caps: caps,
                tier_caps_declared: declared,
            }
        }
        let mut reg = ModelRegistry::default();
        reg.profiles.insert(
            "good".into(),
            prof("good", ModelTier::Flagship, ModelTier::Flagship.caps(), true),
        );
        reg.profiles.insert(
            "bad".into(),
            prof(
                "bad",
                ModelTier::Flagship,
                TierCaps {
                    context_window: 16_384,
                    ..ModelTier::Flagship.caps()
                },
                true,
            ),
        );
        reg.profiles.insert(
            "unknown".into(),
            prof("unknown", ModelTier::Balanced, TierCaps::default(), false),
        );

        let audit = reg.audit_tier_caps();
        assert_eq!(audit.ok, vec!["good".to_string()]);
        assert_eq!(audit.unverified, vec!["unknown".to_string()]);
        assert_eq!(audit.mismatched.len(), 1);
        assert_eq!(audit.mismatched[0].0, "bad");
        assert!(
            audit.mismatched[0].2.iter().any(|r| r.contains("上下文")),
            "矛盾原因须逐项可读：{:?}",
            audit.mismatched[0].2
        );
        assert!(audit.has_issues());
        assert!(audit.summary().contains("已校验 1"), "{}", audit.summary());
    }
}
