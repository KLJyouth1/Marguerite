//! 引导 / 服务商配置方法族（onboarding.*）
//!
//! 提供模型服务商目录、状态、连通性探测与凭据保存能力。
//! `onboarding.provider.configure` 为 Rust 离线回退实现；
//! `onboarding.llmProfile.*`（v3 新增）对齐 WebUI 设置页「多服务商档案」契约。

use anyhow::Result;
use serde_json::{json, Value};

use crate::gateway::methods::GatewayContext;
use crate::gateway::protocol::RpcError;
use crate::persistence::config::Config;

/// `onboarding.llmProfile.upsert`：新增/更新一个模型服务商档案。
///
/// WebUI 设置页在「添加新服务商 / 编辑非主档案」时调用。Rust 实现将档案
/// 持久化到 models.json（ModelRegistry），密钥经 [`Config::write_env_key`]
/// 转写 `.env`（E3/K1 凭证护栏），绝不落明文。
pub(crate) fn llm_profile_upsert(params: &Value) -> Result<Value, RpcError> {
    use crate::model::registry::{ModelProfile, ProfileKind};
    let provider_id = params
        .get("providerId")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| RpcError::new("INVALID_REQUEST", "缺少 providerId"))?
        .to_string();

    // 凭据：显式 apiKey > apiKeyEnv 引用 > keepCurrentSecret 保留现状
    let explicit_key = params
        .get("apiKey")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default()
        .to_string();
    let keep_current = params
        .get("keepCurrentSecret")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    let model = params
        .get("model")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default()
        .to_string();
    if model.is_empty() {
        return Err(RpcError::new("INVALID_REQUEST", "缺少 model"));
    }

    let base_url = {
        let from_req = params
            .get("baseUrl")
            .or_else(|| params.get("base_url"))
            .and_then(Value::as_str)
            .map(str::trim)
            .unwrap_or_default()
            .to_string();
        if !from_req.is_empty() {
            from_req
        } else {
            match provider_id.as_str() {
                "openai" => "https://api.openai.com/v1".to_string(),
                "deepseek" => "https://api.deepseek.com".to_string(),
                "anthropic" => "https://api.anthropic.com".to_string(),
                _ => String::new(),
            }
        }
    };
    if base_url.is_empty() {
        return Err(RpcError::new(
            "INVALID_REQUEST",
            "自定义服务商必须提供 baseUrl",
        ));
    }

    let label = params
        .get("label")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(&provider_id)
        .to_string();
    let kind = match params
        .get("kind")
        .and_then(Value::as_str)
        .unwrap_or("cloud")
    {
        "local" => ProfileKind::Local,
        "custom" => ProfileKind::Custom,
        _ => ProfileKind::Cloud,
    };

    let mut selector = crate::model::selector::ModelSelector::load()
        .map_err(|e| RpcError::new("PROFILE_LOAD_FAILED", e.to_string()))?;
    let reg = selector.registry_mut();
    let profile_id = format!("{provider_id}-primary");

    // 已存在档案且要求保留密钥 → 不动旧值；否则写入新 key 并转写 .env
    let existing_key = reg
        .get(&profile_id)
        .map(|p| p.api_key.clone())
        .unwrap_or_default();
    let api_key = if keep_current && !existing_key.is_empty() {
        existing_key
    } else {
        explicit_key.clone()
    };
    if !explicit_key.is_empty() {
        Config::write_env_key(&explicit_key);
    }

    let profile = ModelProfile {
        id: profile_id,
        label,
        provider: provider_id.clone(),
        base_url,
        api_key,
        model,
        kind,
        builtin: false,
        temperature: 0.7,
        max_tokens: 4096,
        tier: crate::model::tier::ModelTier::Balanced,
        tier_caps: crate::model::tier::TierCaps::default(),
        tier_caps_declared: false,
    };
    reg.upsert(profile);
    selector
        .save()
        .map_err(|e| RpcError::new("PROFILE_SAVE_FAILED", e.to_string()))?;

    Ok(json!({
        "ok": true,
        "saved": true,
        "providerId": provider_id,
    }))
}

/// `onboarding.llmProfile.upsert`（Value 版本）：供 dispatch_rust 同步分派。
pub(crate) fn llm_profile_upsert_value(params: &Value) -> Value {
    match llm_profile_upsert(params) {
        Ok(v) => v,
        Err(e) => json!({ "ok": false, "error": e.message, "code": e.code }),
    }
}

/// `onboarding.llmProfile.activate`：把指定档案设为当前生效主模型。
pub(crate) fn llm_profile_activate(ctx: &GatewayContext, params: &Value) -> Value {
    let provider_id = params
        .get("providerId")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or_default();
    let model_override = params
        .get("model")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or_default();

    let mut cfg = Config::load().unwrap_or_else(|_| ctx.config.clone());
    if !provider_id.is_empty() {
        cfg.model.provider = provider_id.to_string();
    }
    if !model_override.is_empty() {
        cfg.model.model_name = model_override.to_string();
    }
    match cfg.save() {
        Ok(()) => json!({
            "ok": true,
            "activated": true,
            "restartRequired": false,
            "providerId": cfg.model.provider,
            "model": cfg.model.model_name,
        }),
        Err(e) => json!({ "ok": false, "error": format!("激活失败: {e}") }),
    }
}

/// `onboarding.llmProfile.active.remove`：移除当前主档案（回退默认直连）。
///
/// **必须真正删掉 `.env` 里的密钥**：只清内存再 `save()` 是无效动作——
/// `api_key` 带 `skip_serializing`、YAML 结构性不落钥，而下一次 `Config::load()`
/// 的 `apply_env_overrides` 会从 `.env` 原样读回来。此前这里正是如此：返回
/// `removed: true`，可引擎下一轮仍然拿着旧钥鉴权（UI 说移除、实际没移除）。
pub(crate) fn llm_profile_active_remove(ctx: &GatewayContext, _params: &Value) -> Value {
    let mut cfg = Config::load().unwrap_or_else(|_| ctx.config.clone());
    cfg.model.api_key.clear();
    cfg.model.provider = "deepseek".to_string();
    cfg.model.model_name = "deepseek-v4-pro".to_string();
    let key_removed = Config::remove_env_key();
    match cfg.save() {
        Ok(()) => json!({ "ok": true, "removed": true, "keyRemoved": key_removed }),
        Err(e) => json!({ "ok": false, "error": format!("移除失败: {e}") }),
    }
}
