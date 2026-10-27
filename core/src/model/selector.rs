//! 模型选择器（Phase H2）— 注册表 + 配置目录路径 + 即时切换 + 密钥回退
//!
//! 三形态（CLI / TUI / GUI）共用同一份 `models.json`（与 `config.yaml` 同目录）。
//! 选择器负责加载、切换、持久化，并在解析最终 LLMConfig 时套用 BYOK 回退链。

use std::path::PathBuf;

use anyhow::{anyhow, Result};

use crate::model::byok;
use crate::model::llm::LLMConfig;
use crate::model::registry::{ModelProfile, ModelRegistry, ProfileKind};
use crate::model::tier::{ModelTier, TierCaps};
use crate::persistence::config::Config;

/// `models.json` 路径（`~/.marguerite/models.json`）
pub fn models_path() -> PathBuf {
    Config::config_dir().join("models.json")
}

/// **生效模型三要素**（provider / model_name / base_url）：以 `models.json` 的 active 档案为准。
///
/// # 为什么以 models.json 为准（实测缺陷）
///
/// 执行侧只认 `config.yaml`：`ModelRouter` 通篇读 `config.model.*`，从不看 `models.json`
/// （`ModelSelector::active_config()` 甚至零调用点）。而 `model use` / 控制台激活
/// （`llmProfile.activate`）只写 `models.json` —— 于是「切换了模型，回答与报错仍来自旧
/// provider」是必然结果：两个真相源，执行只认其一。
///
/// # 为什么是「读侧覆盖」而不是「写回 config.yaml」
///
/// 写回会在**每次切换**时把用户手写的 YAML 整份重排（丢注释、字段顺序变化、文件抖动），
/// 且一旦某处调用方传错路径就会误改真实配置。读侧覆盖只影响「用哪个模型」这一件事，
/// 一行解析、零副作用，且 `config.yaml` 继续承载其余全部配置。
///
/// 档案缺失 / `models.json` 损坏时**回退 config.yaml**（不假装切换成功，也不清空模型）。
pub fn effective_model_fields(cfg: &Config) -> (String, String, String) {
    effective_model_fields_at(cfg, &models_path())
}

/// 路径可注入的 [`effective_model_fields`]（单测用，绝不触碰真实 `~/.marguerite`）。
pub fn effective_model_fields_at(
    cfg: &Config,
    models_path: &std::path::Path,
) -> (String, String, String) {
    let fallback = || {
        (
            cfg.model.provider.clone(),
            cfg.model.model_name.clone(),
            cfg.model.base_url.clone(),
        )
    };
    // `models.json` **不存在**时必须回退 config.yaml：`ModelRegistry::load` 在文件缺失时
    // 返回**种子默认值**（active = 某个内置档案），若拿它当生效值，就等于把「用户从没在
    // 档案库里选过模型」的机器强行切到种子默认模型 —— 那正是「静默改用户模型」。
    if !models_path.exists() {
        return fallback();
    }
    let Ok(reg) = ModelRegistry::load(models_path) else {
        return fallback();
    };
    match reg.profiles.get(&reg.active) {
        Some(p) => (p.provider.clone(), p.model.clone(), p.base_url.clone()),
        None => fallback(),
    }
}

/// 供**会话入口**调用：返回把「用户选择的模型」叠加进 `model` 段之后的配置。
///
/// 叠加来源是 `models.json` 的 active 档案（`model use` / 控制台 `llmProfile.activate`
/// 写的就是它）。三要素与配置一致时**返回原配置的克隆前先比较**、不一致才克隆 —— 未选过
/// 档案的机器（`models.json` 缺失）原样返回，行为与旧版完全一致。
///
/// 调用点应是「用户会话」的边界（网关 chat / CLI run / TUI 工作台），**不是**通用构造器：
/// 让通用构造器读全局 `models.json` 会让单测依赖开发者本机的档案库。
pub fn apply_effective_model(cfg: &Config) -> Config {
    apply_effective_model_at(cfg, &models_path())
}

/// 路径可注入的 [`apply_effective_model`]。
pub fn apply_effective_model_at(cfg: &Config, models_path: &std::path::Path) -> Config {
    let (provider, model, base_url) = effective_model_fields_at(cfg, models_path);
    let provider_changed = cfg.model.provider.trim() != provider.trim();
    let mut overlaid = cfg.clone();
    let mut changed = provider_changed;
    if cfg.model.provider != provider {
        overlaid.model.provider = provider;
        changed = true;
    }
    if cfg.model.model_name != model {
        overlaid.model.model_name = model;
        changed = true;
    }
    if cfg.model.base_url != base_url {
        overlaid.model.base_url = base_url;
        changed = true;
    }
    // 密钥必须**跟随 provider**，规则分两种（这一点极易写错，改动前务必读完）：
    //
    // - **provider 变了**：必须把密钥换成新档案解析出的那把，**允许结果为空**。
    //   沿用旧 provider 的密钥有两个后果：① 必然 401；② 更糟的是把 A 家的密钥
    //   发给了 B 家的端点（跨域泄漏）。所以「解析不出」时宁可留空让请求以
    //   「未配置密钥」失败，也不能留着上一家的钥匙。
    // - **provider 没变**（仅换同家模型）：密钥不动。`resolve_api_key` 的解析顺序是
    //   档案分槽 → 受归属标记约束的全局槽 → 档案密文/明文；同 provider 下解析结果
    //   为空时（例如密钥只存在于被叠加前的 `cfg.model.api_key` 里）覆盖会清空可用密钥，
    //   因此仅在解析出非空值时才替换。
    if changed {
        if let Ok(reg) = ModelRegistry::load(models_path) {
            if let Some(p) = reg.profiles.get(&reg.active) {
                let key =
                    byok::resolve_api_key(&p.id, &p.provider, &p.api_key, &p.api_key_env, None);
                if provider_changed || !key.trim().is_empty() {
                    overlaid.model.api_key = key;
                }
            }
        }
    }
    overlaid
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

    /// 可变注册表访问（配合 [`Self::save`] 持久化）
    pub fn registry_mut(&mut self) -> &mut ModelRegistry {
        &mut self.registry
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

    /// 恢复一个被删掉的**内置种子档案**（`model remove` 的逆操作）。
    ///
    /// 为什么需要：`ModelRegistry::load` 只在 `seed_revision` 落后时补齐种子，
    /// 补齐后版本号持平 → 删掉的内置档案**永远不会自己回来**（用户「删了就再也找不到」）。
    /// 这里显式从内嵌种子取回该档案：已存在则报错（不覆盖用户可能已改过的版本），
    /// 不是内置 id 则报错（绝不凭空造档案）。
    pub fn restore_builtin(&mut self, id: &str) -> Result<ModelProfile> {
        if self.registry.profiles.contains_key(id) {
            return Err(anyhow!(
                "档案已存在，无需恢复: {id}（若要覆盖请先 model remove 再 restore）"
            ));
        }
        let seed = ModelRegistry::with_seeds();
        let Some(p) = seed.profiles.get(id).cloned() else {
            return Err(anyhow!(
                "没有名为 {id} 的内置档案可恢复（只支持恢复内置种子档案）"
            ));
        };
        self.registry.profiles.insert(id.to_string(), p.clone());
        self.save()?;
        Ok(p)
    }

    /// 恢复**全部**缺失的内置种子档案，返回恢复的 id 列表。
    pub fn restore_all_builtin(&mut self) -> Result<Vec<String>> {
        let seed = ModelRegistry::with_seeds();
        let mut restored: Vec<String> = Vec::new();
        for (id, p) in seed.profiles {
            if !self.registry.profiles.contains_key(&id) {
                self.registry.profiles.insert(id.clone(), p);
                restored.push(id);
            }
        }
        restored.sort();
        if !restored.is_empty() {
            self.save()?;
        }
        Ok(restored)
    }

    /// 运行时热更新内置种子模型（无需重编译）。
    ///
    /// 读取 `models.json` 同目录下的 `models.seed.json` 覆盖层，把其中缺失的档案
    /// 合并进当前注册表（**不覆盖**用户已编辑/自定义的档案，也不改动 active/copilot），
    /// 写回 `models.json` 后返回新补充的档案数。
    ///
    /// 用于「模型家族持续换代」场景：发布新模型家族时，只需随更新下发一个
    /// `models.seed.json`（含新档案 + 更高的 `seed_revision`），运行本命令即可生效，
    /// 无需重新编译二进制——彻底杜绝「硬编码旧版模型」问题。
    ///
    /// 覆盖层不存在时返回 `Ok(0)`（无操作）。
    pub fn reload_seeds(&mut self) -> Result<usize> {
        let overlay = self.path.with_file_name("models.seed.json");
        if !overlay.exists() {
            return Ok(0);
        }
        let s = std::fs::read_to_string(&overlay)?;
        let extra: ModelRegistry = serde_json::from_str(&s)
            .map_err(|e| anyhow::anyhow!("models.seed.json 解析失败: {e}"))?;
        let before = self.registry.profiles.len();
        self.registry.merge_seeds(&extra);
        let added = self.registry.profiles.len() - before;
        self.save()?;
        Ok(added)
    }

    /// 解析主模型的最终 LLMConfig（api_key 经 BYOK 回退链）
    pub fn active_config(&self) -> Option<LLMConfig> {
        self.registry.active_profile().map(|p| {
            let mut cfg = p.to_llm_config();
            cfg.api_key = byok::resolve_api_key(&p.id, &p.provider, &p.api_key, &p.api_key_env, None);
            cfg
        })
    }

    /// 解析 Bloom 副模型的最终 LLMConfig（若有）
    pub fn copilot_config(&self) -> Option<LLMConfig> {
        self.registry.copilot_profile().map(|p| {
            let mut cfg = p.to_llm_config();
            cfg.api_key = byok::resolve_api_key(&p.id, &p.provider, &p.api_key, &p.api_key_env, None);
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
        api_key_env: String::new(),
        model,
        kind,
        builtin: false,
        temperature: 0.7,
        max_tokens: 4096,
        tier: ModelTier::Balanced,
        tier_caps: TierCaps::default(),
        tier_caps_declared: false,
        // CLI `--wire-api` 可显式声明；缺省 chat（与全部内置/本地端点一致）。
        wire_api: flags
            .get("wire-api")
            .and_then(|v| crate::model::wire::WireApi::from_key(v))
            .unwrap_or_default(),
        // 未声明 → 按 provider/model 名保守识别（见 registry::recognize）。
        capabilities: Default::default(),
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
            api_key_env: String::new(),
            model: "llama3".into(),
            kind: ProfileKind::Local,
            builtin: false,
            temperature: 0.7,
            max_tokens: 4096,
            tier: ModelTier::Balanced,
            tier_caps: TierCaps::default(),
            tier_caps_declared: false,
            wire_api: crate::model::wire::WireApi::Chat,
            capabilities: Default::default(),
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
            api_key_env: String::new(),
            model: "m".into(),
            kind: ProfileKind::Custom,
            builtin: false,
            temperature: 0.7,
            max_tokens: 4096,
            tier: ModelTier::Balanced,
            tier_caps: TierCaps::default(),
            tier_caps_declared: false,
            wire_api: crate::model::wire::WireApi::Chat,
            capabilities: Default::default(),
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

    /// **生效模型以 models.json 为准**：这是「切换了模型却不生效」的根因修复守卫。
    #[test]
    fn effective_model_fields_prefer_models_json_active() {
        let base = std::env::temp_dir().join(format!("marguerite_sel_eff_{}", std::process::id()));
        let path = base.join("models.json");
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();

        let mut cfg = Config::default();
        cfg.model.provider = "config-provider".into();
        cfg.model.model_name = "config-model".into();
        cfg.model.base_url = "https://config.example".into();

        // 1) models.json 不存在 → 必须回退 config（不得假装切换、不得清空模型）
        assert_eq!(
            effective_model_fields_at(&cfg, &path),
            (
                "config-provider".to_string(),
                "config-model".to_string(),
                "https://config.example".to_string()
            ),
            "无档案时必须回退 config.yaml"
        );

        // 2) 有 models.json 且 active 指向某档案 → 执行侧取该档案（否则 `model use` 永不生效）
        let mut sel = ModelSelector::with_path(path.clone()).unwrap();
        let mut mine = sel
            .registry()
            .profiles
            .values()
            .next()
            .cloned()
            .expect("种子应有档案");
        mine.id = "preferred".into();
        mine.provider = "profile-provider".into();
        mine.model = "profile-model".into();
        mine.base_url = "https://profile.example".into();
        sel.add(mine).unwrap();
        sel.use_active("preferred").unwrap();

        assert_eq!(
            effective_model_fields_at(&cfg, &path),
            (
                "profile-provider".to_string(),
                "profile-model".to_string(),
                "https://profile.example".to_string()
            ),
            "models.json 的 active 必须是执行侧生效值"
        );

        // 3) active 指向不存在的档案（文件被手改坏）→ 回退 config，而不是给出空模型
        let raw = std::fs::read_to_string(&path).unwrap();
        let mut j: serde_json::Value = serde_json::from_str(&raw).unwrap();
        j["active"] = serde_json::Value::String("ghost".into());
        std::fs::write(&path, j.to_string()).unwrap();
        assert_eq!(
            effective_model_fields_at(&cfg, &path),
            (
                "config-provider".to_string(),
                "config-model".to_string(),
                "https://config.example".to_string()
            ),
            "active 悬空时必须回退 config，绝不能给出空模型"
        );
    }

    /// `apply_effective_model`：会话入口把用户选择叠加进 config，且**密钥跟随 provider**。
    #[test]
    fn apply_effective_model_overlays_model_and_key() {
        let base = std::env::temp_dir().join(format!("marguerite_sel_apply_{}", std::process::id()));
        let path = base.join("models.json");
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();

        let mut cfg = Config::default();
        cfg.model.provider = "cfg-provider".into();
        cfg.model.model_name = "cfg-model".into();
        cfg.model.base_url = "https://cfg.example".into();
        cfg.model.api_key = "sk-cfg".into();

        // 1) 无 models.json → 原样返回（未选过档案的机器行为不变）
        let same = apply_effective_model_at(&cfg, &path);
        assert_eq!(same.model.provider, "cfg-provider");
        assert_eq!(same.model.model_name, "cfg-model");
        assert_eq!(same.model.api_key, "sk-cfg");

        // 2) 有 active 档案 → 三要素覆盖为该档案，且密钥不再沿用旧 provider 的密钥
        let mut sel = ModelSelector::with_path(path.clone()).unwrap();
        let mut mine = sel.registry().profiles.values().next().cloned().unwrap();
        mine.id = "preferred".into();
        mine.provider = "prof-provider".into();
        mine.model = "prof-model".into();
        mine.base_url = "https://prof.example".into();
        mine.api_key = "sk-prof".into();
        sel.add(mine).unwrap();
        sel.use_active("preferred").unwrap();

        let out = apply_effective_model_at(&cfg, &path);
        assert_eq!(out.model.provider, "prof-provider");
        assert_eq!(out.model.model_name, "prof-model");
        assert_eq!(out.model.base_url, "https://prof.example");
        assert_ne!(
            out.model.api_key, "sk-cfg",
            "切换 provider 后不得继续沿用旧 provider 的密钥（否则必然 401 + 跨域泄露）"
        );
        // 注意：`ModelProfile.api_key` 带 `skip_serializing`，档案里那把 key 本来就
        // **不会落盘**（密钥只走 `.env`/环境变量），所以重新加载后解析结果是空。
        // 「解析不出就得留空」正是本函数的安全规则（宁可明确报未配置，也不留上一家的
        // 钥匙）；要验证「解析得出时必须带上」，需要给进程注入 `MARGUERITE_API_KEY[_<ID>]`
        // —— 那是全局可变量，会让并行用例互相污染，故不在这里做，改由
        // `provider_switch_never_keeps_the_previous_provider_key` 覆盖同家密钥不被清空。
    }

    /// **切换 provider 时绝不能沿用旧 provider 的密钥**：那既是必然 401，更是把
    /// A 家的密钥发给了 B 家的端点（跨域泄漏）。这条规则一旦被改成「解析不出来就
    /// 保留原样」，泄漏会静默复活且没有任何测试会红——故在此单独钉死。
    #[test]
    fn provider_switch_never_keeps_the_previous_provider_key() {
        let base =
            std::env::temp_dir().join(format!("marguerite_sel_keyswitch_{}", std::process::id()));
        let path = base.join("models.json");
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();

        // cfg 是「deepseek + 有一把 deepseek 的钥匙」
        let mut cfg = Config::default();
        cfg.model.provider = "deepseek".into();
        cfg.model.model_name = "deepseek-v4-flash".into();
        cfg.model.base_url = "https://api.deepseek.com".into();
        cfg.model.api_key = "sk-cfg".into();

        let mut sel = ModelSelector::with_path(path.clone()).unwrap();
        // 1) 同 provider 只换模型（deepseek-v4-flash → 种子 deepseek-v4-pro）：
        //    `.env` 的单槽密钥本就属于这一家，不得被清空。
        sel.use_active("deepseek-v4-pro").unwrap();
        let same_provider = apply_effective_model_at(&cfg, &path);
        assert_eq!(same_provider.model.provider, "deepseek");
        assert_eq!(same_provider.model.model_name, "deepseek-v4-pro");
        assert!(
            !same_provider.model.api_key.trim().is_empty(),
            "同 provider 换模型不得把 `.env` 单槽密钥清空（旧实现会直接不可用）"
        );

        // 2) 换成另一个 provider 的档案（该档案解析不出密钥）：
        //    必须**丢掉** deepseek 的钥匙，宁可留空走「未配置密钥」失败。
        let mut other = sel.registry().profiles.values().next().cloned().unwrap();
        other.id = "other-provider-primary".into();
        other.provider = "other-provider".into();
        other.model = "other-model".into();
        other.base_url = "https://other.example".into();
        other.api_key = String::new();
        sel.add(other).unwrap();
        sel.use_active("other-provider-primary").unwrap();

        let switched = apply_effective_model_at(&cfg, &path);
        assert_eq!(switched.model.provider, "other-provider");
        assert_eq!(switched.model.model_name, "other-model");
        assert_ne!(
            switched.model.api_key, "sk-cfg",
            "provider 变了还带着上一家的钥匙＝跨域泄露（绝不能回退成「保留原密钥」）"
        );
    }

    /// 删掉的内置档案必须能**恢复**（它在 `load` 里永远不会自己回来）。
    #[test]
    fn restore_builtin_revives_a_removed_seed() {
        let base =
            std::env::temp_dir().join(format!("marguerite_sel_restore_{}", std::process::id()));
        let path = base.join("models.json");
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        let mut sel = ModelSelector::with_path(path.clone()).unwrap();

        let victim = "openai-gpt-4o-mini".to_string();
        assert!(
            sel.registry().profiles.contains_key(&victim),
            "前置条件：种子里应有该档案"
        );
        sel.remove(&victim).unwrap();
        assert!(!sel.registry().profiles.contains_key(&victim), "删除后应消失");

        let restored = sel.restore_builtin(&victim).unwrap();
        assert_eq!(restored.id, victim);
        assert!(
            sel.registry().profiles.contains_key(&victim),
            "恢复后应重新在位"
        );
        // 落盘也要真的恢复（不能只改内存）
        let reloaded = ModelSelector::with_path(path).unwrap();
        assert!(
            reloaded.registry().profiles.contains_key(&victim),
            "恢复必须已持久化"
        );

        // 幂等语义：已存在 → 明确报错，而不是静默覆盖用户可能改过的版本
        let err = sel.restore_builtin(&victim).unwrap_err();
        assert!(err.to_string().contains("已存在"), "错误必须可诊断: {err}");
    }

    /// 恢复只覆盖内置档案，且提供 `--all` 的批量口径。
    #[test]
    fn restore_refuses_unknown_and_supports_restore_all() {
        let base =
            std::env::temp_dir().join(format!("marguerite_sel_restore2_{}", std::process::id()));
        let path = base.join("models.json");
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        let mut sel = ModelSelector::with_path(path).unwrap();

        let err = sel.restore_builtin("no-such-builtin-xyz").unwrap_err();
        assert!(
            err.to_string().contains("内置档案"),
            "未知 id 必须明确拒绝（不得凭空造档案）: {err}"
        );

        // 删两个内置档案 → `--all` 恰好恢复这两个
        for id in ["openai-gpt-4o-mini", "deepseek-v4-flash"] {
            if sel.registry().profiles.contains_key(id) {
                sel.remove(id).unwrap();
            }
        }
        let restored = sel.restore_all_builtin().unwrap();
        assert_eq!(restored.len(), 2, "只应恢复缺失的两个: {restored:?}");
        let mut sorted = restored.clone();
        sorted.sort();
        assert_eq!(restored, sorted, "输出必须确定性排序");
        for id in restored {
            assert!(sel.registry().profiles.contains_key(&id));
        }
        assert!(
            sel.restore_all_builtin().unwrap().is_empty(),
            "全部在位时不得谎报恢复"
        );
    }

    #[test]
    fn test_reload_seeds_adds_overlay_without_clobber() {
        // reload_seeds 应只补充缺失档案、不覆盖用户自定义档案、不改主模型、升级版本号。
        let base =
            std::env::temp_dir().join(format!("marguerite_sel_reload_{}", std::process::id()));
        let path = base.join("models.json");
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        let mut sel = ModelSelector::with_path(path.clone()).unwrap();
        sel.add(ModelProfile {
            id: "mine".into(),
            label: "Mine".into(),
            provider: "ollama".into(),
            base_url: "http://localhost:11434/v1".into(),
            api_key: String::new(),
            api_key_env: String::new(),
            model: "llama3".into(),
            kind: ProfileKind::Local,
            builtin: false,
            temperature: 0.7,
            max_tokens: 4096,
            tier: ModelTier::Balanced,
            tier_caps: TierCaps::default(),
            tier_caps_declared: false,
            wire_api: crate::model::wire::WireApi::Chat,
            capabilities: Default::default(),
        })
        .unwrap();
        sel.use_active("mine").unwrap();

        // 覆盖层：一个新模型家族 + 更高 seed_revision
        let overlay = base.join("models.seed.json");
        let overlay_json = r#"{
  "profiles": {
    "glm-5-pro": {
      "id": "glm-5-pro",
      "label": "智谱 GLM-5-Pro（热更新）",
      "provider": "custom",
      "base_url": "https://open.bigmodel.cn/api/paas/v4",
      "model": "glm-5-pro",
      "kind": "cloud",
      "builtin": true,
      "temperature": 0.7,
      "max_tokens": 8192
    }
  },
  "active": "deepseek-v4-pro",
  "copilot": null,
  "seed_revision": 9
}"#;
        std::fs::write(&overlay, overlay_json).unwrap();

        let added = sel.reload_seeds().unwrap();
        assert_eq!(added, 1, "应仅补充 1 个新家族");
        // 新家族已出现
        assert!(sel.registry().get("glm-5-pro").is_some());
        // 用户档案保留、主模型不变
        assert!(sel.registry().get("mine").is_some());
        assert_eq!(sel.registry().active, "mine");
        // 版本号升到覆盖层版本
        assert_eq!(sel.registry().seed_revision, 9);

        // 重新加载确认持久化
        let reloaded = ModelSelector::with_path(path).unwrap();
        assert!(reloaded.registry().get("glm-5-pro").is_some());
        assert_eq!(reloaded.registry().active, "mine");

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn test_reload_seeds_no_overlay_is_noop() {
        // 无覆盖层时安全返回 0（不报错、不改变注册表）。
        let base =
            std::env::temp_dir().join(format!("marguerite_sel_reload_noop_{}", std::process::id()));
        let path = base.join("models.json");
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        let mut sel = ModelSelector::with_path(path).unwrap();
        let before = sel.registry().profiles.len();
        let added = sel.reload_seeds().unwrap();
        assert_eq!(added, 0);
        assert_eq!(sel.registry().profiles.len(), before);
        let _ = std::fs::remove_dir_all(&base);
    }
}
