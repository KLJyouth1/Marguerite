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
/// 持久化到 models.json（ModelRegistry），密钥经 [`Config::write_env_key_slot`]
/// 转写该档案在 `.env` 的**专属槽**（E3/K1 凭证护栏），绝不落明文；
/// 也支持 `apiKeyEnv`：只存变量名，运行期由 BYOK 回退链按名解析。
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
    // 「使用环境变量」：webui 发送的是变量**名**（`OPENAI_API_KEY`），不是密钥本身。
    // 不识别它 → 用户选完保存，凭据会静默失效（apiKey 被清空、没有任何回退来源）。
    let api_key_env_param = params
        .get("apiKeyEnv")
        .or_else(|| params.get("api_key_env"))
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default()
        .to_string();

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
            default_base_url_for(&provider_id)
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
    // 显式 kind 优先；**未给定时不能一律当云**：本地/回环端点（Ollama / LM Studio）
    // 落到 `Cloud` 会让「本地模型」在路由与凭据判定里被当成需要密钥的云服务 ——
    // 用户真实档案 `ollama-primary | provider=ollama | kind=cloud` 就是这么产生的。
    // 判据与 `onboarding` 的 R72 本地豁免逐字一致（provider 名 或 回环地址）。
    let kind = match params.get("kind").and_then(Value::as_str) {
        Some("local") => ProfileKind::Local,
        Some("custom") => ProfileKind::Custom,
        Some(_) => ProfileKind::Cloud,
        None => infer_profile_kind(&provider_id, &base_url),
    };

    let mut selector = crate::model::selector::ModelSelector::load()
        .map_err(|e| RpcError::new("PROFILE_LOAD_FAILED", e.to_string()))?;
    let reg = selector.registry_mut();
    let profile_id = format!("{provider_id}-primary");

    // 凭据三选一（keepCurrentSecret 时不触碰磁盘）：
    // - 显式 key → 写该档案**分槽**（多 provider 并存），并清掉环境变量引用；
    // - apiKeyEnv → 只记变量名，删掉旧分槽（否则残留值会压过用户新选的来源）；
    // - 都没有 → 清空凭据。
    let existing = reg.get(&profile_id).cloned();
    // 先取出既有协议：`existing` 后面会被 move 进凭据分支，届时就读不到了。
    let existing_wire = existing.as_ref().map(|p| p.wire_api);
    // 同理先取出多模态声明：重配服务商不该把用户勾过的能力悄悄清空。
    let existing_caps = existing.as_ref().map(|p| p.capabilities.clone());
    let (api_key, api_key_env) = if keep_current {
        existing
            .map(|p| (p.api_key, p.api_key_env))
            .unwrap_or_default()
    } else if !explicit_key.is_empty() {
        Config::write_env_key_slot(&profile_id, &explicit_key);
        (explicit_key.clone(), String::new())
    } else if !api_key_env_param.is_empty() {
        Config::remove_env_key_slot(&profile_id);
        (String::new(), api_key_env_param)
    } else {
        (String::new(), String::new())
    };

    // 协议声明：显式值优先；给错必须**响亮拒绝**（拼错回落 chat 会变成 404 之谜）。
    // 未给时沿用该档案已有取值（重配服务商不该把协议悄悄重置），再退到默认 chat。
    // 注意：`existing` 在上面凭据分支里被 move 过，故这里只用 `existing_wire`。
    let wire_api = resolve_wire_api(params, &provider_id, existing_wire.unwrap_or_default())?;

    let profile = ModelProfile {
        id: profile_id,
        label,
        provider: provider_id.clone(),
        base_url,
        api_key,
        api_key_env,
        model,
        kind,
        builtin: false,
        temperature: 0.7,
        max_tokens: 4096,
        tier: crate::model::tier::ModelTier::Balanced,
        tier_caps: crate::model::tier::TierCaps::default(),
        tier_caps_declared: false,
        // 用户显式声明优先；未声明即 chat（存量档案与本地端点都只有 chat）。
        wire_api,
        // 传了 `capabilities` → 以用户勾选为准；没传 → 保留既有声明，
        // 重配服务商不该把用户勾过的能力悄悄清空（新档案则为未声明 = 自动识别）。
        capabilities: match params.get("capabilities") {
            Some(v) => crate::model::registry::ModelCapabilities::from_json(v),
            None => existing_caps.unwrap_or_default(),
        },
    };
reg.upsert(profile);
    selector
        .save()
        .map_err(|e| RpcError::new("PROFILE_SAVE_FAILED", e.to_string()))?;

    // P3 打通：模型档案新增/更新也广播 models.changed，让已打开的路由候选、
    // 聊天主模型指示等页面被动刷新（此前只有 local_deploy 两个发布点）。
    crate::events::product::emit(
        "models.changed",
        "",
        json!({ "reason": "llm_profile_upsert", "provider_id": provider_id }),
    );

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
///
/// # 接线点（改这段前先读完）
///
/// 本次修复前的实现**只写 `config.yaml` 的 `model.provider`**（`model_name` /
/// `base_url` 原样不动），而执行侧生效模型取自 `models.json` 的 active 档案
/// （`crate::model::selector::apply_effective_model`）。两个真相源各写一半，
/// 结果是「界面提示已激活、引擎仍跑旧模型」+「provider 换成 A 却还带着
/// B 的 model 名与 endpoint」。
///
/// 现在把激活统一为**一件事**：在模型档案（`models.json`）里选中该服务商的档案，
/// 再把同样的三要素镜像回 `config.yaml`，使档案与配置文件不再互相矛盾。
/// 档案里找不到对应服务商时**明确报错**，不做「只写 config.yaml」的静默降级 ——
/// 那种降级正是要根除的缺陷（写了却没人读）。
pub(crate) fn llm_profile_activate(
    ctx: &GatewayContext,
    params: &Value,
) -> Result<Value, RpcError> {
    // 两种寻址方式（优先级从高到低）：
    //   · `profileId` —— 档案 id 精确指名。同一 provider 下有多份档案（典型：本机
    //     Ollama 装了 5 个模型、都 provider=ollama）时，仅用 providerId 无法区分，
    //     `model` 也可能重名，只有 id 是无歧义的主键。
    //   · `providerId`（+ 可选 `model`）—— 兼容既有调用方与「按服务商切档」语义。
    let profile_id_param = params
        .get("profileId")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default()
        .to_string();
    let provider_id = params
        .get("providerId")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default()
        .to_lowercase();
    if profile_id_param.is_empty() && provider_id.is_empty() {
        return Err(RpcError::new("INVALID_REQUEST", "缺少 profileId 或 providerId"));
    }
    let model_override = params
        .get("model")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or_default()
        .to_string();

    let mut selector = crate::model::selector::ModelSelector::load()
        .map_err(|e| RpcError::new("PROFILE_LOAD_FAILED", e.to_string()))?;
    let target = if profile_id_param.is_empty() {
        resolve_activation_profile(selector.registry(), &provider_id, &model_override)
            .ok_or_else(|| {
                RpcError::new(
                    "onboarding.llmProfile.unknown_provider",
                    format!(
                        "模型档案中没有服务商 {provider_id}{} 的档案：请先在设置页「模型档案」\
                         添加该服务商（或 CLI `marguerite model add`），再激活",
                        if model_override.is_empty() {
                            String::new()
                        } else {
                            format!(" / 模型 {model_override}")
                        }
                    ),
                )
            })?
    } else {
        if !selector.registry().profiles.contains_key(&profile_id_param) {
            return Err(RpcError::new(
                "onboarding.llmProfile.unknown_profile",
                format!(
                    "模型档案中不存在 id={profile_id_param}：档案可能已被删除，\
                     请刷新模型选择器后重试"
                ),
            ));
        }
        profile_id_param
    };
    selector
        .use_active(&target)
        .map_err(|e| RpcError::new("PROFILE_SAVE_FAILED", e.to_string()))?;
    let profile = selector
        .registry()
        .get(&target)
        .cloned()
        .ok_or_else(|| RpcError::new("PROFILE_LOAD_FAILED", format!("档案消失: {target}")))?;

    let mirrored = mirror_active_into_config(ctx, &profile)
        .map_err(|e| RpcError::new("CONFIG_SAVE_FAILED", format!("激活失败: {e}")))?;
    // 「保存即生效」：镜像只改了**文件**，而运行中的引擎读的是
    // `publish_effective_config` 维护的进程级生效快照 —— 不发布的话，接口回
    // `activated:true`、`config.yaml` 也确实写对了，但引擎下一轮照旧拿旧的
    // provider + model 发请求，直到重启。这正是本项目反复出现的静默失效
    // （`onboarding_router_configure` 早已按同款处理，这里此前漏了）。
crate::gateway::chat::publish_effective_config(mirrored);

    // P3 打通：激活主模型广播 models.changed，路由候选 / 聊天主模型指示即时感知。
    crate::events::product::emit(
        "models.changed",
        "",
        json!({ "reason": "llm_profile_activate", "provider_id": profile.provider }),
    );

    Ok(json!({
        "ok": true,
        "activated": true,
        "restartRequired": false,
        "providerId": profile.provider,
        "model": profile.model,
        "profileId": target,
    }))
}

/// `llm_profile_activate` 的 `Value` 版本：供 `dispatch_rust` 同步分派（不抛异常）。
pub(crate) fn llm_profile_activate_value(ctx: &GatewayContext, params: &Value) -> Value {
    match llm_profile_activate(ctx, params) {
        Ok(v) => v,
        Err(e) => json!({ "ok": false, "error": e.message, "code": e.code }),
    }
}

/// 在模型档案里挑出「该服务商的哪个档案」应被激活。
///
/// 打分规则（数字越小越优先；同分按 id 升序，保证确定性）：
/// 0 = `{provider}-primary`（`llmProfile.upsert` 的建档约定）
/// 1 = 该 provider 的**用户自建**档案（非内置）
/// 2 = id 恰为 provider（如种子 `openrouter`）
/// 3 = 该 provider 的内置种子档案（如 `openai` → `openai-gpt-*`）
/// 4 = id 以 `{provider}-` 开头且用户自建
/// 5 = id 以 `{provider}-` 开头（如 `zhipu` → `zhipu-glm-5-3`，其 provider 字段是 `custom`）
///
/// `model_override` 非空时**只**在 model 完全相符的档案里挑 —— 否则会把用户点名的
/// 模型换成另一个，与 `config.yaml` 镜像当场矛盾。
fn resolve_activation_profile(
    reg: &crate::model::registry::ModelRegistry,
    provider_id: &str,
    model_override: &str,
) -> Option<String> {
    let primary_id = format!("{provider_id}-primary");
    let prefix = format!("{provider_id}-");
    let mut best: Option<(u8, String)> = None;
    for (id, p) in &reg.profiles {
        if !model_override.is_empty() && p.model != model_override {
            continue;
        }
        let rank = if id.as_str() == primary_id {
            0
        } else if p.provider.eq_ignore_ascii_case(provider_id) && !p.builtin {
            1
        } else if id.as_str() == provider_id {
            2
        } else if p.provider.eq_ignore_ascii_case(provider_id) {
            3
        } else if id.starts_with(&prefix) && !p.builtin {
            4
        } else if id.starts_with(&prefix) {
            5
        } else {
            continue;
        };
        let better = match &best {
            None => true,
            Some((br, bid)) => rank < *br || (rank == *br && id.as_str() < bid.as_str()),
        };
        if better {
            best = Some((rank, id.clone()));
        }
    }
    best.map(|(_, id)| id)
}

/// 把生效档案的三要素镜像回 `config.yaml`（模型段的唯一职责就是「与档案一致」），
/// 并**返回写盘后的配置**，供调用方发布进程级生效快照。
///
/// 返回值不是可有可无的便利：`Config::save()` 只落文件，而运行中的引擎读的是
/// `publish_effective_config` 维护的内存快照。只看文件不看快照，就会写出「盘上
/// 已激活、引擎照旧跑旧模型」的静默失效。故这里把「刚写盘的那一份」交回调用方，
/// 由调用方决定何时发布 —— 避免两边各自 `Config::load()` 出两份不一致的真相。
fn mirror_active_into_config(
    ctx: &GatewayContext,
    profile: &crate::model::registry::ModelProfile,
) -> Result<Config> {
    let mut cfg = Config::load().unwrap_or_else(|_| ctx.config.clone());
    cfg.model.provider = profile.provider.clone();
    cfg.model.model_name = profile.model.clone();
    cfg.model.base_url = profile.base_url.clone();
    // 协议也属于「三要素」：漏镜像时，任何走 config.yaml 的构造路径
    // （`LLMConfig::from_app_config`）都会拿到旧的 chat，而档案写的是
    // responses —— 同一份配置两个真相源，正是本文件反复在修的那类缺陷。
    cfg.model.wire_api = profile.wire_api;
    cfg.save()?;
    Ok(cfg)
}

/// 按档案 **id** 激活：use_active + 镜像 config + 发布生效快照。
///
/// 供 `local_models` 的 Ollama 一键部署等内部调用方复用「激活即生效」完整路径
/// （与 [`llm_profile_activate`] 同源，只是选择键从 providerId 换成精确 id）。
/// 三步缺一不可：只写 models.json 不镜像 → config.yaml 与档案矛盾；
/// 只镜像不 `publish_effective_config` → 盘上对了、运行引擎下一轮仍跑旧模型。
pub(crate) fn activate_profile_by_id(
    ctx: &GatewayContext,
    profile_id: &str,
) -> Result<(), String> {
    let mut selector = crate::model::selector::ModelSelector::load()
        .map_err(|e| format!("模型档案加载失败: {e}"))?;
    let profile = selector
        .registry()
        .get(profile_id)
        .cloned()
        .ok_or_else(|| format!("模型档案中不存在 id={profile_id}"))?;
    selector
        .use_active(profile_id)
        .map_err(|e| format!("激活失败: {e}"))?;
    let mirrored = mirror_active_into_config(ctx, &profile)
        .map_err(|e| format!("配置镜像失败: {e}"))?;
    crate::gateway::chat::publish_effective_config(mirrored);
    Ok(())
}

/// 把 `ModelSelector` 注册表里**可用**的档案收敛为路由候选列表并落盘 + 发布生效快照。
///
/// 设计约束（与 onboarding/llm_profile 同源）：`Config::save()` 只落文件，运行引擎读
/// 的是 `publish_effective_config` 维护的内存快照。故本函数写完即发布，避免「盘上候选
/// 更新、UI/引擎下一轮仍读旧候选」。
///
/// 合并策略：候选 = 注册表全部档案 id ∪（仍存在于注册表的既有人工选择）。即部署/激活
/// 新模型会自动并入候选；用户经 patch 手动增删的、且仍指向有效档案的条目被保留；已从
/// 注册表移除的陈旧候选被丢弃。云端档案一律纳入候选（运行时凭密钥可用性再裁决）。
pub(crate) fn reconcile_routing_candidates(ctx: &GatewayContext) -> Result<Config, String> {
    let selector = crate::model::selector::ModelSelector::load()
        .map_err(|e| format!("模型档案加载失败: {e}"))?;
    let registry = selector.registry();
    let known: std::collections::HashSet<String> =
        registry.list().iter().map(|p| p.id.clone()).collect();
    // 候选基线 = 注册表全部档案 id（任一档案均可被激活进路由）。
    let mut next: std::collections::HashSet<String> = known.iter().cloned().collect();
    // 从**原始 YAML**读取（不经 Config::load 的 apply_env_overrides）：避免 .env 的
    // MARGUERITE_BASE_URL/PROVIDER/MODEL 覆盖内存 cfg 后整体 `save()` 把 model 段
    // 回写成云端值——否则本地激活刚被 mirror 写盘的回环端点，又被这里冲掉（静默失效）。
    let raw = std::fs::read_to_string(Config::config_path()).unwrap_or_default();
    let mut cfg: Config = serde_yaml::from_str(&raw).unwrap_or_else(|_| ctx.config.clone());
    // 保留仍指向有效档案的既有人工选择（用户经 patch 维护的子集）。
    for id in &cfg.model_routing.candidate_models {
        if known.contains(id) {
            next.insert(id.clone());
        }
    }
    let mut merged: Vec<String> = next.into_iter().collect();
    merged.sort();
    if merged != cfg.model_routing.candidate_models {
        cfg.model_routing.candidate_models = merged;
        cfg.save()
            .map_err(|e| format!("候选列表落盘失败: {e}"))?;
    }
    // 发布进程级生效快照：候选列表即时对 UI/引擎可见，无需重启。
    crate::gateway::chat::publish_effective_config(cfg.clone());
    Ok(cfg)
}

/// `onboarding.llmProfile.active.remove`：移除当前主档案（回退默认直连）。
///
/// **必须真正删掉 `.env` 里的密钥**：只清内存再 `save()` 是无效动作——
/// `api_key` 带 `skip_serializing`、YAML 结构性不落钥，而下一次 `Config::load()`
/// 的 `apply_env_overrides` 会从 `.env` 原样读回来。此前这里正是如此：返回
/// `removed: true`，可引擎下一轮仍然拿着旧钥鉴权（UI 说移除、实际没移除）。
pub(crate) fn llm_profile_active_remove(ctx: &GatewayContext, _params: &Value) -> Value {
    const FALLBACK_PROVIDER: &str = "deepseek";
    const FALLBACK_MODEL: &str = "deepseek-v4-pro";

    // 先记下当前 active 档案再回退：分槽名基于档案 id，切走之后就查不到了。
    // 旧实现只删全局槽，而 upsert 写入的是分槽 →「移除」后引擎仍拿旧钥鉴权。
    let active_id = Config::active_profile_identity().map(|(id, _, _)| id);
    let slot_removed = active_id
        .as_deref()
        .map(Config::remove_env_key_slot)
        .unwrap_or(false);
    let global_removed = Config::remove_env_key();
    let key_removed = slot_removed || global_removed;

    // **档案侧必须一并回落到内置默认档案**（不是可选的美化）：执行侧生效模型取自
    // `models.json` 的 active，只改 `config.yaml` 会让界面说「已移除」而引擎照旧。
    let fallback_profile = (|| -> Option<crate::model::registry::ModelProfile> {
        let mut sel = crate::model::selector::ModelSelector::load().ok()?;
        let id = resolve_activation_profile(sel.registry(), FALLBACK_PROVIDER, FALLBACK_MODEL)?;
        sel.use_active(&id).ok()?;
        sel.registry().get(&id).cloned()
    })();

    let mut cfg = Config::load().unwrap_or_else(|_| ctx.config.clone());
    cfg.model.api_key.clear();
    cfg.model.provider = FALLBACK_PROVIDER.to_string();
    cfg.model.model_name = FALLBACK_MODEL.to_string();
    if let Some(p) = &fallback_profile {
        cfg.model.base_url = p.base_url.clone();
    }
    match cfg.save() {
        Ok(()) => {
            // 与激活同款补发布：只落盘不发布时，引擎下一轮仍拿旧 provider / 旧密钥
            // （`.env` 分槽已删，但内存快照里那把钥还在）。
            crate::gateway::chat::publish_effective_config(cfg);
            json!({ "ok": true, "removed": true, "keyRemoved": key_removed })
        }
        Err(e) => json!({ "ok": false, "error": format!("移除失败: {e}") }),
    }
}

/// 取第一个非空字符串参数（兼容 camelCase / snake_case 两套命名）。
fn str_param(params: &Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|k| params.get(*k).and_then(Value::as_str))
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// 解析并校验请求协议（`wire_api`）。
///
/// 两件事在这里一次做对，缺一不可：
/// 1. **两套命名都认**：控制台的 `buildProviderPayload` 会把字段名转成 camelCase
///    （`wire_api` → `wireApi`），而此前只读 snake_case —— 用户在协议下拉框里
///    选了 responses，保存照样成功，落盘的仍是 chat，请求打到老协议上。
/// 2. **真正执行 [`crate::model::wire::validate`]**：该函数早已存在（含单测），
///    但此前**没有任何调用点**，于是 `anthropic + responses` 这类不可能成功的
///    组合能一路存进 models.json，最后表现为 404 而错误信息指向「模型名不对」。
///
/// `fallback` 也要过校验：历史档案可能已经带着坏组合，保存时顺手拦住。
fn resolve_wire_api(
    params: &Value,
    provider_id: &str,
    fallback: crate::model::wire::WireApi,
) -> Result<crate::model::wire::WireApi, RpcError> {
    let wire = match str_param(params, &["wire_api", "wireApi"]) {
        Some(raw) => crate::model::wire::WireApi::from_key(&raw).ok_or_else(|| {
            RpcError::new(
                "INVALID_WIRE_API",
                format!(
                    "未知 wire_api: {raw}（可选：chat / responses）。\
                     若你的端点只提供 /chat/completions 请用 chat。"
                ),
            )
        })?,
        None => fallback,
    };
    crate::model::wire::validate(
        wire,
        crate::model::provider_protocol::is_anthropic_family(provider_id),
    )
    .map_err(|e| RpcError::new("INVALID_WIRE_API", e))?;
    Ok(wire)
}

/// 内置服务商的默认端点（自定义服务商返回空串，由调用方拒绝）。
///
/// id 口径以 `pyengine/provider/registry.py` 为准（`dashscope` 即为其中之一）。
/// `qwen` 保留为**历史别名**：旧版离线目录曾用它落过档案，这些档案多数没写
/// base_url，去掉兜底会让它们保存时报「必须提供 baseUrl」。
fn default_base_url_for(provider: &str) -> String {
    match provider {
        "openai" => "https://api.openai.com/v1".to_string(),
        "deepseek" => "https://api.deepseek.com".to_string(),
        "anthropic" => "https://api.anthropic.com".to_string(),
        "ollama" => "http://localhost:11434".to_string(),
        "dashscope" | "qwen" => "https://dashscope.aliyuncs.com/compatible-mode/v1".to_string(),
        _ => String::new(),
    }
}

/// 由「provider 名 + 端点」推断档案种类。
///
/// 判据必须与 `onboarding` 的本地豁免逐字一致：**回环端点一律 Local**。
/// 判错的后果不是显示问题，而是本地模型在路由/凭据判定里被当成「需要密钥的云服务」
/// → 明明就绪却发不出消息（历史缺陷 `ollama-primary | kind=cloud` 就是这么来的）。
fn infer_profile_kind(provider: &str, base_url: &str) -> crate::model::registry::ProfileKind {
    use crate::model::registry::ProfileKind;
    // 判据与凭据门禁 / doctor / onboarding 同源，避免「建档算云、发消息算本地」的分裂。
    if crate::model::byok::is_keyless_endpoint(provider, base_url) {
        ProfileKind::Local
    } else {
        ProfileKind::Cloud
    }
}

/// `onboarding.provider.configure`（legacy「编辑当前服务商」保存）的**档案优先**实现。
///
/// # 为什么这条路径必须改写（根因，2026-10-21 实测）
///
/// 旧实现只写三样东西：`config.yaml` 的 `model` 段、`.env` 全局槽
/// `MARGUERITE_API_KEY`、进程内生效快照 —— **从不碰 `models.json`**。
/// 而请求侧真正的取值链是
/// `chat.send → apply_effective_model() → models.json.active`，
/// 只要 `models.json` 存在，active 档案的 provider / model / base_url 就会
/// **覆盖** `config.yaml` 的同名字段（`selector::effective_model_fields_at`）。
///
/// 于是「设置 → 模型服务 → 编辑当前服务商 → 保存 → 提示成功」之后，引擎照旧用
/// 旧档案发请求。实测证据：
/// · `.env` 只有全局槽 `MARGUERITE_API_KEY`（legacy 写的），
///   **没有** `MARGUERITE_API_KEY_<PROFILE>`（upsert 才写分槽）→ 说明走的是老路；
/// · `config.yaml.model.provider = deepseek`，而 `models.json.active = omnipilot16c-local`
///   → 两个真相源互相矛盾，引擎信后者。
///
/// 修法（不是打补丁）：把这条路径收敛成与 `llmProfile.activate` 同一套动作 ——
/// 更新/创建档案 → 设为 active → 镜像回 config.yaml → 发布生效快照 → 重算路由候选。
/// 无论前端走新路还是老路、CLI 走哪条，**真相源只有 models.json 一份**。
pub(crate) fn configure_primary_provider(
    ctx: &GatewayContext,
    params: &Value,
) -> Result<Value, RpcError> {
    use crate::model::registry::ModelProfile;

    let mut selector = crate::model::selector::ModelSelector::load()
        .map_err(|e| RpcError::new("PROFILE_LOAD_FAILED", e.to_string()))?;

    let active_id = selector.registry().active.clone();
    let active = selector.registry().get(&active_id).cloned();

    let provider = str_param(params, &["provider", "providerId"])
        .or_else(|| active.as_ref().map(|p| p.provider.clone()))
        .ok_or_else(|| RpcError::new("INVALID_REQUEST", "缺少 provider"))?;

    // 同名服务商的既有档案：只有 provider 一致时才允许「沿用它的 model / base_url」。
    // 这条限制是**正确性**要求而非洁癖：active 若是 ollama，而用户改配 openai，
    // 沿用 active 的 base_url 会把请求发到 `localhost:11434` 却带着 openai 的 key
    // —— 比「报错」危险得多。跨服务商时只认该服务商自己的档案或内置默认端点。
    let same_provider_active = active
        .as_ref()
        .filter(|p| p.provider.eq_ignore_ascii_case(&provider))
        .cloned();
    let primary_id = format!("{provider}-primary");
    let primary_profile = selector.registry().get(&primary_id).cloned();
    let known = same_provider_active.as_ref().or(primary_profile.as_ref());

    let model = str_param(params, &["model", "modelName", "model_name"])
        .or_else(|| known.map(|p| p.model.clone()))
        .filter(|m| !m.is_empty())
        .ok_or_else(|| {
            RpcError::new(
                "INVALID_REQUEST",
                format!("缺少 model：服务商 {provider} 没有可沿用的档案，请显式提供模型名"),
            )
        })?;
    let base_url = str_param(params, &["baseUrl", "base_url"])
        .or_else(|| known.map(|p| p.base_url.clone()))
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| default_base_url_for(&provider));
    if base_url.is_empty() {
        return Err(RpcError::new(
            "INVALID_REQUEST",
            "自定义服务商必须提供 baseUrl",
        ));
    }

    // 目标档案：active 同 provider 就地更新（保住 id / label / tier / 温度等）
    // → `{provider}-primary` → 该 provider 的任一档案 → 新建。
    // 就地更新是关键：否则每次保存都会把用户的 tier / label 重置成默认值。
    let target_id = if same_provider_active.is_some() {
        active_id.clone()
    } else if primary_profile.is_some() {
        primary_id.clone()
    } else {
        resolve_activation_profile(selector.registry(), &provider, "")
            .unwrap_or_else(|| primary_id.clone())
    };

    // 协议校验（在闭包外，才能用 `?` 把错误抛给调用方）。
    // 只解析**显式**传入的值：未传时保持既有声明（就地更新路径 clone 已有档案）。
    let explicit_wire = match str_param(params, &["wire_api", "wireApi"]) {
        Some(_) => Some(resolve_wire_api(
            params,
            &provider,
            crate::model::wire::WireApi::default(),
        )?),
        None => None,
    };

    let mut profile = selector
        .registry()
        .get(&target_id)
        .cloned()
        .unwrap_or_else(|| ModelProfile {
            id: target_id.clone(),
            label: provider.clone(),
            provider: provider.clone(),
            base_url: base_url.clone(),
            api_key: String::new(),
            api_key_env: String::new(),
            model: model.clone(),
            kind: infer_profile_kind(&provider, &base_url),
            builtin: false,
            temperature: 0.7,
            max_tokens: 4096,
            tier: crate::model::tier::ModelTier::Balanced,
            tier_caps: crate::model::tier::TierCaps::default(),
            tier_caps_declared: false,
            // 新建档案时的协议：显式传入才用，否则默认 chat。
            wire_api: explicit_wire.unwrap_or_default(),
            // 新建档案走自动识别；就地更新路径 clone 已有档案，保住用户勾过的能力。
            capabilities: Default::default(),
        });

    profile.provider = provider.clone();
    profile.model = model.clone();
    profile.base_url = base_url.clone();
    // 就地更新也必须吃下显式协议：旧实现只在「新建档案」分支使用解析结果，
    // 于是用户在同一个服务商上从 chat 切到 responses —— 下拉框选了、提示保存成功，
    // 落盘的却仍是 clone 来的旧值。协议是请求形状本身，静默丢失等于功能不存在。
    if let Some(wire) = explicit_wire {
        profile.wire_api = wire;
    }
    // 多模态能力：显式传入则覆盖（就地更新与新建都适用）；未传保留原声明。
    if let Some(v) = params.get("capabilities") {
        profile.capabilities = crate::model::registry::ModelCapabilities::from_json(v);
    }
    // 端点变了 kind 必须跟着变（云 ↔ 回环），否则本地档案被当成云服务要密钥。
    if !profile.builtin {
        profile.kind = infer_profile_kind(&provider, &base_url);
    }

    // 凭据三态：显式 key > apiKeyEnv > 都不给 = **保持现状**
    // （前端 `preserveApiKey` 就是「不要动我的密钥」，此处不写即保留）。
    let explicit_key = str_param(params, &["apiKey", "api_key"]).unwrap_or_default();
    let api_key_env = str_param(params, &["apiKeyEnv", "api_key_env"]).unwrap_or_default();
    if !explicit_key.is_empty() {
        Config::write_env_key_slot(&target_id, &explicit_key);
        profile.api_key = explicit_key;
        profile.api_key_env = String::new();
    } else if !api_key_env.is_empty() {
        Config::remove_env_key_slot(&target_id);
        profile.api_key_env = api_key_env;
        profile.api_key = String::new();
    }

    selector.registry_mut().upsert(profile.clone());
    selector
        .save()
        .map_err(|e| RpcError::new("PROFILE_SAVE_FAILED", e.to_string()))?;
    selector
        .use_active(&target_id)
        .map_err(|e| RpcError::new("PROFILE_SAVE_FAILED", e.to_string()))?;

    let mirrored = mirror_active_into_config(ctx, &profile)
        .map_err(|e| RpcError::new("CONFIG_SAVE_FAILED", format!("镜像配置失败: {e}")))?;
    // 保存即生效：引擎读的是进程级快照，不发布就要重启才换模型。
crate::gateway::chat::publish_effective_config(mirrored);

    // P3 打通：onboarding.provider.configure 委托到本函数后才真正落盘生效；
    // 在这里（而非 onboarding.rs）发事件，避免与 llmProfile.* 路径重复广播。
    crate::events::product::emit(
        "models.changed",
        "",
        json!({ "reason": "provider_configure", "provider_id": provider }),
    );
    let candidate_models = reconcile_routing_candidates(ctx)
        .map(|c| c.model_routing.candidate_models.len())
        .unwrap_or(0);

    Ok(json!({
        "ok": true,
        "saved": true,
        // 兼容 legacy 返回形状（前端读这两个键决定提示文案）
        "restart_required": false,
        "restartRequired": false,
        "providerId": profile.provider,
        "model": profile.model,
        "baseUrl": profile.base_url,
        "profileId": target_id,
        "candidateModels": candidate_models,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    use crate::events::EventBus;
    use crate::model::registry::{ModelRegistry, ProfileKind};
    use crate::model::selector::ModelSelector;

    /// 隔离配置目录的 `GatewayContext`（**必须**：否则用例会读写用户真实的
    /// `~/.marguerite/config.yaml` 与 `models.json`）。
    fn isolated_ctx() -> GatewayContext {
        static SEQ: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "mgt-llmprofile-{}-{}",
            std::process::id(),
            SEQ.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::create_dir_all(&dir);
        Config::set_config_dir_override(dir);
        GatewayContext {
            config: Config::default(),
            token: "t".into(),
            started_at: std::time::Instant::now(),
            bus: Arc::new(EventBus::default()),
        }
    }

    /// 在隔离目录里建一个 `{provider}-primary` 档案并把 active 先指到别处。
    fn seed_primary_profile(id: &str, provider: &str, model: &str, base_url: &str) {
        let path = crate::model::selector::models_path();
        let mut sel = ModelSelector::with_path(path).unwrap();
        let mut p = sel.registry().profiles.values().next().cloned().unwrap();
        p.id = id.to_string();
        p.label = id.to_string();
        p.provider = provider.to_string();
        p.model = model.to_string();
        p.base_url = base_url.to_string();
        p.api_key = String::new();
        p.kind = ProfileKind::Cloud;
        p.builtin = false;
        sel.add(p).unwrap();
        sel.use_active("deepseek-chat").unwrap();
    }

    fn write_config_mirror(provider: &str, model: &str, base_url: &str) {
        let mut cfg = Config::load().unwrap();
        cfg.model.provider = provider.to_string();
        cfg.model.model_name = model.to_string();
        cfg.model.base_url = base_url.to_string();
        cfg.save().unwrap();
    }

    /// 直接读 **磁盘上的 YAML 原文**（不过 `Config::load`）。
    ///
    /// 为什么不能断言 `Config::load()`：它每次都跑 `apply_env_overrides`，而仓库里
    /// 的 `core/.env` 与根 `.env` 都写着 `MARGUERITE_PROVIDER/MODEL/BASE_URL`，
    /// cargo 的 cwd 正是它们所在的目录 —— 断言会被 `.env` 静默改写（本用例首版
    /// 就是这样失败的）。镜像写的是文件，就该从文件读回来验。
    fn read_yaml_mirror() -> Config {
        let raw = std::fs::read_to_string(Config::config_path()).expect("config.yaml 应存在");
        serde_yaml::from_str(&raw).expect("config.yaml 应可解析")
    }

    /// **激活服务商必须落到 `models.json`，并把三要素镜像进 `config.yaml`**。
    ///
    /// 旧实现只写 `config.yaml` 的 `provider`（连 model / base_url 都不动），
    /// 于是「已激活」在引擎上是零动作、且 provider 与 model 名互相矛盾。
    #[test]
    fn activate_selects_registry_profile_and_mirrors_config() {
        let ctx = isolated_ctx();
        seed_primary_profile(
            "openai-primary",
            "openai",
            "gpt-5.5",
            "https://api.openai.com/v1",
        );
        write_config_mirror("deepseek", "deepseek-v4-flash", "https://api.deepseek.com");

        let res = llm_profile_activate(&ctx, &json!({ "providerId": "openai" }))
            .expect("激活应成功");
        assert_eq!(res["ok"], true);
        assert_eq!(res["profileId"], "openai-primary");
        assert_eq!(res["model"], "gpt-5.5");

        // 执行侧真相源：models.json 的 active
        let after = ModelSelector::with_path(crate::model::selector::models_path()).unwrap();
        assert_eq!(
            after.registry().active, "openai-primary",
            "激活必须落到 models.json（否则执行侧仍跑旧模型）"
        );
        // 镜像：三要素一致（旧实现会让 model / base_url 停在 deepseek）
        let mirrored = read_yaml_mirror();
        assert_eq!(mirrored.model.provider, "openai");
        assert_eq!(mirrored.model.model_name, "gpt-5.5");
        assert_eq!(mirrored.model.base_url, "https://api.openai.com/v1");
    }

    /// 同一 provider 下多份档案（本机 5 个 Ollama 模型）时，`profileId` 必须
    /// 精确命中；仅靠 providerId 会把用户点名的模型换成"打分最高"的那份。
    #[test]
    fn activate_by_profile_id_disambiguates_same_provider_models() {
        let ctx = isolated_ctx();
        seed_primary_profile(
            "ollama-llama",
            "ollama",
            "llama3.2:1b",
            "http://127.0.0.1:11434",
        );
        seed_primary_profile(
            "ollama-qwen",
            "ollama",
            "qwen3:4b",
            "http://127.0.0.1:11434",
        );
        write_config_mirror("ollama", "llama3.2:1b", "http://127.0.0.1:11434");

        let res = llm_profile_activate(&ctx, &json!({ "profileId": "ollama-qwen" }))
            .expect("按档案 id 激活应成功");
        assert_eq!(res["profileId"], "ollama-qwen");
        assert_eq!(res["model"], "qwen3:4b");

        let after = ModelSelector::with_path(crate::model::selector::models_path()).unwrap();
        assert_eq!(after.registry().active, "ollama-qwen");
        let mirrored = read_yaml_mirror();
        assert_eq!(mirrored.model.model_name, "qwen3:4b");
    }

    /// 档案 id 不存在时明确报错（不静默降级到同 provider 的其它档案）。
    #[test]
    fn activate_refuses_unknown_profile_id() {
        let ctx = isolated_ctx();
        seed_primary_profile(
            "ollama-llama",
            "ollama",
            "llama3.2:1b",
            "http://127.0.0.1:11434",
        );
        write_config_mirror("ollama", "llama3.2:1b", "http://127.0.0.1:11434");

        let err = llm_profile_activate(&ctx, &json!({ "profileId": "ollama-ghost" }))
            .expect_err("未知档案 id 必须报错");
        assert_eq!(err.code, "onboarding.llmProfile.unknown_profile");

        let after = ModelSelector::with_path(crate::model::selector::models_path()).unwrap();
        assert_eq!(after.registry().active, "deepseek-chat", "报错时不得切换 active");
    }

    /// 档案里没有该服务商时**明确报错且不写任何东西**：只写 config.yaml 的
    /// 静默降级正是本次要根除的缺陷（写了却没人读）。
    #[test]
    fn activate_refuses_unknown_provider_and_writes_nothing() {
        let ctx = isolated_ctx();
        let path = crate::model::selector::models_path();
        let mut sel = ModelSelector::with_path(path.clone()).unwrap();
        sel.use_active("deepseek-chat").unwrap();
        write_config_mirror("deepseek", "deepseek-v4-flash", "https://api.deepseek.com");

        let err = llm_profile_activate(&ctx, &json!({ "providerId": "no-such-provider" }))
            .expect_err("未知服务商必须报错");
        assert_eq!(err.code, "onboarding.llmProfile.unknown_provider");

        let mirrored = read_yaml_mirror();
        assert_eq!(mirrored.model.provider, "deepseek", "报错时不得改动 config.yaml");
        let after = ModelSelector::with_path(path).unwrap();
        assert_eq!(after.registry().active, "deepseek-chat", "报错时不得改动档案选区");
    }

    /// 打分规则与 `model` 精确约束（纯函数，不碰文件系统）。
    #[test]
    fn resolve_activation_profile_ranking_and_model_override() {
        let reg = ModelRegistry::with_seeds();

        // provider 直配（种子 provider=openai 的档案）
        let id = resolve_activation_profile(&reg, "openai", "").unwrap();
        assert_eq!(reg.profiles[&id].provider, "openai");
        // id 恰为 provider（种子 openrouter 的 provider 字段是 custom）
        assert_eq!(
            resolve_activation_profile(&reg, "openrouter", "").unwrap(),
            "openrouter"
        );
        // 前缀匹配（zhipu 的 provider 字段是 custom，只能靠 id 前缀认出）
        let zhipu = resolve_activation_profile(&reg, "zhipu", "").unwrap();
        assert!(zhipu.starts_with("zhipu-"), "实得 {zhipu}");
        // model 精确约束：点名的模型必须原样选中
        assert_eq!(
            resolve_activation_profile(&reg, "deepseek", "deepseek-v4-flash").unwrap(),
            "deepseek-v4-flash"
        );
        // 点名的模型不存在 → None（绝不拿别的模型充数，否则与镜像当场矛盾）
        assert!(resolve_activation_profile(&reg, "deepseek", "no-such-model").is_none());
        // 未知 provider → None
        assert!(resolve_activation_profile(&reg, "nope", "").is_none());
    }
    /// 根因回归（2026-10-21）：legacy `onboarding.provider.configure` 必须**写进档案库**。
    ///
    /// 修复前它只写 config.yaml + .env 全局槽，而 `apply_effective_model` 用
    /// `models.json.active` 覆盖 config.yaml → 用户看到「保存成功」但引擎照旧跑旧档案。
    /// 本用例直接断言「引擎实际会用的三要素」已经变成新服务商，而不是只看返回值。
    #[test]
    fn provider_configure_writes_models_json_and_engine_effective_model() {
        let ctx = isolated_ctx();
        // 先制造一个「本地档案是 active」的现场（正是用户机器上的真实状态）。
        let mut sel = ModelSelector::load().expect("load");
        let seed_id = sel.registry().profiles.keys().next().cloned().expect("seed profile");
        sel.use_active(&seed_id).expect("seed active");
        drop(sel);

        let res = crate::gateway::onboarding::onboarding_provider_configure(
            &ctx,
            &json!({
                "provider": "deepseek",
                "model": "deepseek-v4-flash",
                "baseUrl": "https://api.deepseek.com",
                "apiKey": "sk-test-legacy",
            }),
        );
        assert_eq!(res["ok"], true, "{res}");
        assert_eq!(res["restartRequired"], false, "{res}");

        // ① 档案库：active 已指向新服务商
        let reg = ModelRegistry::load(&crate::model::selector::models_path()).expect("registry");
        let active = reg.profiles.get(&reg.active).expect("active profile");
        assert_eq!(active.provider, "deepseek", "active 档案 provider 未更新");
        assert_eq!(active.model, "deepseek-v4-flash");

        // ② config.yaml：镜像同步（两处不再互相矛盾）。
        //    这里读**文件原文**而不是 `Config::load()`：后者会叠加 `.env` / 进程环境变量
        //    的覆盖（`MARGUERITE_MODEL` 等），那是另一层真相源，不该混进本用例的断言。
        let yaml = std::fs::read_to_string(Config::config_dir().join("config.yaml"))
            .expect("config.yaml 应已落盘");
        assert!(yaml.contains("provider: deepseek"), "config.yaml 未镜像 provider://n{yaml}");
        assert!(
            yaml.contains("model_name: deepseek-v4-flash"),
            "config.yaml 未镜像 model://n{yaml}"
        );

        // ③ **最关键**：引擎真正取的值（models.json 优先）必须是新服务商
        let cfg = Config::load().expect("config");
        let (provider, model, base_url) = crate::model::selector::effective_model_fields_at(
            &cfg,
            &crate::model::selector::models_path(),
        );
        assert_eq!(provider, "deepseek", "引擎仍会用旧档案 —— 保存等于没生效");
        assert_eq!(model, "deepseek-v4-flash");
        assert_eq!(base_url, "https://api.deepseek.com");
    }

    /// 未提供 apiKey 时**不得**清空已有密钥（前端 `preserveApiKey` 的语义）。
    /// 历史坑：每次改模型名就把密钥抹掉，用户表现为「保存后突然 401」。
    #[test]
    fn provider_configure_without_key_preserves_existing_credential() {
        let ctx = isolated_ctx();
        let first = crate::gateway::onboarding::onboarding_provider_configure(
            &ctx,
            &json!({ "provider": "deepseek", "model": "deepseek-v4-flash", "baseUrl": "https://api.deepseek.com", "apiKey": "sk-keep-me" }),
        );
        assert_eq!(first["ok"], true, "{first}");

        let second = crate::gateway::onboarding::onboarding_provider_configure(
            &ctx,
            &json!({ "provider": "deepseek", "model": "deepseek-v4-pro", "preserveApiKey": true }),
        );
        assert_eq!(second["ok"], true, "{second}");

        let reg = ModelRegistry::load(&crate::model::selector::models_path()).expect("registry");
        let active = reg.profiles.get(&reg.active).expect("active");
        assert_eq!(active.model, "deepseek-v4-pro", "模型应更新");
        // 密钥**按设计不落 models.json**（`#[serde(skip_serializing)]`），只在 `.env` 分槽里；
        // 所以「是否被清空」必须去分槽里查，而不是看档案字段。
        let slot = Config::env_slot_name(&reg.active);
        let env = std::fs::read_to_string(Config::config_dir().join(".env")).unwrap_or_default();
        assert!(
            env.contains(&format!("{slot}=sk-keep-me")),
            "未显式给 key 时分槽 {slot} 不得被清空，实际 .env://n{env}"
        );
    }

    /// 回环端点必须判成 Local（历史缺陷：`ollama-primary | kind=cloud` 导致
    /// 本地模型被当成「需要密钥的云服务」，明明就绪却发不出消息）。
    #[test]
    fn provider_configure_infers_local_kind_for_loopback_endpoints() {
        let ctx = isolated_ctx();
        let res = crate::gateway::onboarding::onboarding_provider_configure(
            &ctx,
            &json!({ "provider": "ollama", "model": "qwen3:8b", "baseUrl": "http://localhost:11434/v1" }),
        );
        assert_eq!(res["ok"], true, "{res}");
        let reg = ModelRegistry::load(&crate::model::selector::models_path()).expect("registry");
        let active = reg.profiles.get(&reg.active).expect("active");
        assert_eq!(active.kind, ProfileKind::Local, "回环端点必须是 Local: {active:?}");
    }

    /// 控制台发的是 **camelCase**（`buildProviderPayload` 把 `wire_api` 转成
    /// `wireApi`）。只认 snake_case 的旧实现会静默丢弃协议：用户在下拉框选了
    /// responses、保存提示成功，落盘的仍是 chat，请求继续打 /chat/completions。
    #[test]
    fn upsert_accepts_camel_case_wire_api_and_persists_it() {
        isolated_ctx();
        let res = llm_profile_upsert(&json!({
            "providerId": "custom",
            "baseUrl": "https://gateway.example.com/v1",
            "model": "gpt-4o",
            "apiKey": "sk-x",
            "wireApi": "responses",
        }))
        .expect("camelCase wireApi 必须被接受");
        assert_eq!(res["ok"], true);

        let reg = ModelRegistry::load(&crate::model::selector::models_path()).expect("registry");
        let p = reg.profiles.get("custom-primary").expect("档案应已落盘");
        assert_eq!(
            p.wire_api,
            crate::model::wire::WireApi::Responses,
            "wireApi 必须真正写进 models.json，而不是被静默忽略"
        );
    }

    /// `anthropic + responses` 是不可能成功的组合（Anthropic 只有 /messages），
    /// 必须在保存时就**响亮拒绝**，而不是存进去等运行时 404。
    #[test]
    fn upsert_rejects_anthropic_family_with_responses_wire() {
        isolated_ctx();
        for provider in ["anthropic", "custom_anthropic"] {
            let err = llm_profile_upsert(&json!({
                "providerId": provider,
                "baseUrl": "https://api.example.com",
                "model": "claude-sonnet-4-20250514",
                "apiKey": "sk-x",
                "wireApi": "responses",
            }))
            .expect_err("anthropic 家族不能声明 responses 协议");
            assert_eq!(err.code, "INVALID_WIRE_API", "{provider}: {}", err.message);
            assert!(
                err.message.contains("wire_api") && err.message.contains("anthropic"),
                "{provider}: {}",
                err.message
            );
        }
        // 反例守卫：anthropic + chat 必须放行（chat 只表示"非 responses"）
        let ok = llm_profile_upsert(&json!({
            "providerId": "custom_anthropic",
            "baseUrl": "https://api.example.com",
            "model": "claude-sonnet-4-20250514",
            "apiKey": "sk-x",
            "wireApi": "chat",
        }))
        .expect("anthropic 家族 + chat 应放行");
        assert_eq!(ok["ok"], true);
    }

    /// 重配服务商时**不能**把已声明的协议悄悄重置回 chat。
    #[test]
    fn upsert_without_wire_keeps_the_stored_protocol() {
        isolated_ctx();
        llm_profile_upsert(&json!({
            "providerId": "custom",
            "baseUrl": "https://gateway.example.com/v1",
            "model": "gpt-4o",
            "apiKey": "sk-x",
            "wireApi": "responses",
        }))
        .expect("首存");

        llm_profile_upsert(&json!({
            "providerId": "custom",
            "baseUrl": "https://gateway.example.com/v1",
            "model": "gpt-4.1",
            "keepCurrentSecret": true,
        }))
        .expect("重配");

        let reg = ModelRegistry::load(&crate::model::selector::models_path()).expect("registry");
        let p = reg.profiles.get("custom-primary").expect("档案");
        assert_eq!(p.model, "gpt-4.1", "模型应更新");
        assert_eq!(
            p.wire_api,
            crate::model::wire::WireApi::Responses,
            "未显式给协议时必须保留既有声明"
        );
    }

    /// 旧实现在「就地更新已存在档案」时丢弃显式协议（只 clone 旧值），
    /// 于是同一个服务商上切换协议永远不生效 —— 选了、存了、还是老协议。
    #[test]
    fn provider_configure_applies_explicit_wire_on_in_place_update() {
        let ctx = isolated_ctx();
        let first = crate::gateway::onboarding::onboarding_provider_configure(
            &ctx,
            &json!({
                "provider": "custom",
                "model": "gpt-4o",
                "baseUrl": "https://gateway.example.com/v1",
                "apiKey": "sk-x",
            }),
        );
        assert_eq!(first["ok"], true, "{first}");
        let before = ModelRegistry::load(&crate::model::selector::models_path()).expect("registry");
        let before_active = before.profiles.get(&before.active).expect("active").clone();
        assert_eq!(
            before_active.wire_api,
            crate::model::wire::WireApi::Chat,
            "首次保存缺省应为 chat"
        );

        // 同一 provider 就地更新，显式切到 responses
        let second = crate::gateway::onboarding::onboarding_provider_configure(
            &ctx,
            &json!({
                "provider": "custom",
                "model": "gpt-4o",
                "baseUrl": "https://gateway.example.com/v1",
                "preserveApiKey": true,
                "wireApi": "responses",
            }),
        );
        assert_eq!(second["ok"], true, "{second}");
        let after = ModelRegistry::load(&crate::model::selector::models_path()).expect("registry");
        let active = after.profiles.get(&after.active).expect("active");
        assert_eq!(
            active.wire_api,
            crate::model::wire::WireApi::Responses,
            "就地更新必须吃下显式协议，而不是 clone 旧值"
        );
        assert_eq!(after.active, before.active, "就地更新不应换档案 id");
        // 镜像也必须带上协议：否则走 config.yaml 的构造路径（from_app_config）
        // 仍会拿到旧的 chat，与档案矛盾。
        assert_eq!(
            read_yaml_mirror().model.wire_api,
            crate::model::wire::WireApi::Responses,
            "config.yaml 镜像必须与档案的协议一致"
        );
    }
}
