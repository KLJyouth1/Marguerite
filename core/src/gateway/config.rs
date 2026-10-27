//! Config 方法族（从 methods.rs 分治迁出）
//!
//! 覆盖 `config.*` 方法：读取配置 / 补丁更新。侧车在线时代理 Python，离线时由本模块
//! 提供最小可用实现，保证设置页「写→读」闭环一致，不出现开关保存后回弹。

use std::cell::Cell;
use std::sync::Mutex;

use serde_json::{json, Value};

use crate::gateway::methods::GatewayContext;
use crate::persistence::config::Config;

/// 配置写入**进程级串行锁**。
///
/// 所有写路径都是「`Config::load()` → 改内存 → `save()`」的读改写序列。并发两次写各自基于
/// 同一份快照落盘 → 后写整份覆盖先写，而两边都回 `ok: true`（真机实测：两条并行
/// `config.patch.safe` 均返回 ok，但只有一条落地 = **静默丢更新**）。加锁后写请求串行，
/// 后者能看到前者的结果，不再互相覆盖。
static CONFIG_WRITE_LOCK: Mutex<()> = Mutex::new(());

/// `config.get`（Rust 离线回退）：返回当前配置快照。
///
/// 每次读取都从磁盘重载，保证 `config.patch.safe` 落盘后立即可见。
pub(crate) fn config_get(ctx: &GatewayContext) -> Value {
    let cfg = Config::load().unwrap_or_else(|_| ctx.config.clone());
    // 生效模型以 models.json 的 active 档案为准（与 `ModelRouter` 同一口径）：
    // 否则控制台显示的是 config.yaml 里的旧模型，用户切完看到「没变化」。
    let (eff_provider, eff_model, eff_base_url) =
        crate::model::selector::effective_model_fields(&cfg);
    let llm_profile = json!({
        "model": eff_model,
        "base_url": eff_base_url,
    });
    let (llm_profiles, llm_profile_rows) = {
        // v3.1：合并 models.json 用户档案 + 当前主 provider。
        // webui 的 storedProfileIds 依赖该映射判定「已保存的服务商」；
        // 此前只有单一主 provider 条目，保存的档案在设置页不可见（空状态）。
        let mut m = serde_json::Map::new();
        let mut rows: Vec<Value> = Vec::new();
        if let Ok(sel) = crate::model::selector::ModelSelector::load() {
            let active_id = sel.registry().active.clone();
            for p in sel.registry().list() {
                // 内置无凭据模板（且非当前激活）不视为「已保存的服务商」
                let skip = p.builtin && p.api_key.is_empty() && p.id != active_id;
                if skip {
                    continue;
                }
                m.insert(
                    p.provider.clone(),
                    json!({
                        "model": p.model,
                        "base_url": p.base_url,
                        "profileId": p.id,
                        "label": p.label,
                        // 档案级协议：同一个人可能既有 chat 的本地端点，
                        // 也有 responses 的云端服务商，列表必须能分辨。
                        "wire_api": p.wire_api.as_key(),
                    }),
                );
                // 追加「按档案 id 列的数组」——与上面的 provider→档案 映射**并存**。
                //
                // 为什么必须另开一个字段（而不是复用 `llm_profiles`）：那个映射以
                // provider 为 key，同一 provider 的多份档案会**互相覆盖**。本机实测：
                // 用户有 5 份 Ollama 档案（qwen3:4b / llama3.2:1b / omnipilot…），
                // 前端只能看到最后一份，于是聊天输入框根本无法提供「云端厂商 /
                // 自定义 / 本地模型」三档选择。`kind` 是分档的唯一依据，
                // 此前也从未回传前端（序列化为 cloud/local/custom）。
                rows.push(json!({
                    "id": p.id,
                    "provider": p.provider,
                    "model": p.model,
                    "base_url": p.base_url,
                    "label": p.label,
                    "kind": p.kind,
                    "wire_api": p.wire_api.as_key(),
                    "builtin": p.builtin,
                    "active": p.id == active_id,
                }));
            }
        }
        // 主 provider 始终可见（即使无档案行）
        m.entry(eff_provider.clone())
            .or_insert_with(|| llm_profile.clone());
        (m, rows)
    };
    json!({
        "model": {
            "provider": eff_provider,
            "model_name": eff_model,
            "base_url": eff_base_url,
            "api_key_configured": !cfg.model.api_key.is_empty(),
        },
        "llm": {
            "provider": eff_provider,
            "model": eff_model,
            "base_url": eff_base_url,
            // 密钥绝不回传前端：仅暴露是否已配置（E3/K1 凭证护栏）
            "api_key": "",
            "api_key_configured": !cfg.model.api_key.is_empty(),
            // 当前请求协议（`chat` / `responses`）。前端与 doctor 据此解释
            // "为什么请求打到了 /v1/responses"；不暴露它就只能靠翻配置文件。
            "wire_api": cfg.model.wire_api.as_key(),
        },
        "llm_profiles": llm_profiles,
        // 附加（非破坏性）：按档案 id 列的完整清单，供聊天输入框的模型选择器分档。
        "llm_profile_rows": llm_profile_rows,
        // 实际生效的请求超时（而非写死的 60）：本地模型在 CPU 上处理 7k+ 提示词
        // 会显著超过 60s（实测 67s），前端/doctor 拿这个值才能解释"为什么等待很久"。
        "llm_request_timeout_seconds": if cfg.model.timeout_secs > 0 {
            cfg.model.timeout_secs
        } else {
            crate::model::llm::default_timeout_secs()
        },
        "mcp": {
            "servers": cfg.mcp.servers.iter().map(|s| json!({
                "name": s.name,
                "command": s.command,
                "args": s.args,
                "url": s.url,
                "enabled": s.enabled,
            })).collect::<Vec<_>>(),
        },
        "memory": {
            "sensory_path": cfg.memory.sensory_path,
            "short_term_path": cfg.memory.short_term_path,
            "auto_capture_enabled": cfg.memory.auto_capture_enabled,
            "dream": {
                "enabled": cfg.dream.enabled,
                "auto_schedule": cfg.dream.auto_schedule,
            },
        },
        "squilla_router": {
            "self_learning": {
                "enabled": cfg.squilla_router.enabled,
            },
            "enabled": cfg.squilla_router.enabled,
            "preset_binding": "follow_primary",
            "default_tier": "",
            "visual_mode": "",
            "cross_provider_tiers": false,
            "tiers": {},
        },
        "llm_ensemble": {
            "enabled": false,
            "model_options": [],
            "candidates": [],
        },
        "skills": {
            "skills_dir": cfg.skills.skills_dir,
        },
        "log": {
            "level": cfg.log.level,
            "file": cfg.log.file,
        },
        "naming": {
            "enabled": cfg.naming.enabled,
        },
        "control_ui": {
            // 首屏语言（与 pyengine `[control_ui] default_locale` 同语义）：
            // webui 设置页写入，网关托管 index.html 时据此渲染 data-locale / <html lang>
            "default_locale": cfg.control_ui.default_locale,
        },
        "privacy": {
            "disable_network_observability": cfg.privacy.disable_network_observability,
            "network_observability_disabled_effective": cfg.privacy.disable_network_observability,
        },
        "guard": {
            "syntax": cfg.guard.syntax,
            "semantic": cfg.guard.semantic,
            "fact": cfg.guard.fact,
            "contract": cfg.guard.contract,
        },
        "bloom": {
            "parallel_enabled": cfg.bloom.parallel_enabled,
        },
        // 模型路由模式（direct/router/ensemble/bloom）。侧车在线时由
        // `overlay_engine_owned_sections` 覆盖回响应；但侧车**离线**时 `config.get`
        // 走本 Rust 回退实现，旧实现漏掉本段 → 设置页「模型路由」永远空白
        // （用户口径「模型路由没有显示」）。这里补齐离线回退的同源输出，
        // 两种形态（snake/camel）都给，前端无论读哪个键都看得到。
        "model_routing": {
            "mode": cfg.model_routing.mode,
            "fusionModel": cfg.model_routing.fusion_model,
            "fusion_model": cfg.model_routing.fusion_model,
            "fixedModel": cfg.model_routing.fixed_model,
            "fixed_model": cfg.model_routing.fixed_model,
            "candidateModels": cfg.model_routing.candidate_models,
            "candidate_models": cfg.model_routing.candidate_models,
            "crossProviderTiers": cfg.model_routing.cross_provider_tiers,
            "cross_provider_tiers": cfg.model_routing.cross_provider_tiers,
        },
        "modelRouting": {
            "mode": cfg.model_routing.mode,
            "fusionModel": cfg.model_routing.fusion_model,
            "fixedModel": cfg.model_routing.fixed_model,
            "candidateModels": cfg.model_routing.candidate_models,
            "crossProviderTiers": cfg.model_routing.cross_provider_tiers,
        },
        // 计划执行配置（WebUI 设置页「计划与审批」据此渲染；`require_approval`
        // 是计划执行前的硬审批闸门开关，见 `core/src/plan/runner.rs`）
        "plan": plan_json(&cfg),
    })
}

/// 计划配置的 JSON 投影（`config.get` 与 `config.plan.get` 共用同一份形状，避免两处漂移）。
pub(crate) fn plan_json(cfg: &Config) -> Value {
    json!({
        "auto_decompose": cfg.plan.auto_decompose,
        "require_approval": cfg.plan.require_approval,
        "max_steps": cfg.plan.max_steps,
        "checkpoint_interval_steps": cfg.plan.checkpoint_interval_steps,
        "heartbeat_interval_s": cfg.plan.heartbeat_interval_s,
        "step_timeout_secs": cfg.plan.step_timeout_secs,
    })
}

/// `config.plan.get`：把计划执行配置单独暴露成 **Rust-first** 方法。
///
/// 为什么不复用 `config.get`：`config.get` 是 PY_FIRST —— 侧车在线时它返回的是
/// Python 侧 `config.toml` 的内容，而**计划执行器（`plan/runner.rs`）读的是
/// `~/.marguerite/config.yaml`**。两者不是同一个文件，于是设置页会「读到 A、改到 B」，
/// 开关看起来生效、引擎毫无变化。这里给出一对**同源**的 Rust 读写口。
pub(crate) fn config_plan_get(_ctx: &GatewayContext) -> Value {
    let cfg = Config::load().unwrap_or_default();
    json!({ "ok": true, "plan": plan_json(&cfg), "source": "config.yaml" })
}

/// `config.plan.set`：写计划执行配置到 `~/.marguerite/config.yaml`（Rust-first）。
///
/// 复用 `config_patch_local`（写锁 + 落盘 + `changed`/`restartRequired` 语义全在那一处），
/// 只是把载荷包装成它认识的 `{ patches: { plan: … } }` 形态 —— 避免两条写路径各自
/// 演化出不同的「改没改」判定。
pub(crate) fn config_plan_set(ctx: &GatewayContext, params: &Value) -> Value {
    let mut out = config_patch_local(ctx, &json!({ "patches": { "plan": params } }));
    if let Some(obj) = out.as_object_mut() {
        obj.insert("plan".to_string(), plan_json(&Config::load().unwrap_or_default()));
    }
    out
}

/// `config.patch` / `config.patch.safe` 的 Rust 本地实现。
///
/// 将 dot-path 补丁落到内存 Config 并持久化到 `config.yaml`。
pub(crate) fn config_patch_local(ctx: &GatewayContext, params: &Value) -> Value {
    // 写锁必须先于 `Config::load()`：整段读改写（load → 改 → save）串行化后，
    // 后到的写能看到前一个写的落盘结果，不再互相整份覆盖。
    let _guard = CONFIG_WRITE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut cfg = Config::load().unwrap_or_else(|_| ctx.config.clone());
    let changed = Cell::new(false);
    let mut linked: Vec<String> = Vec::new();
    let mut restart_required = false;

    let apply_bool = |patches: &serde_json::Map<String, Value>, key: &str, slot: &mut bool| {
        if let Some(v) = patches.get(key).and_then(Value::as_bool) {
            if *slot != v {
                *slot = v;
                changed.set(true);
            }
        }
    };
    let apply_str = |patches: &serde_json::Map<String, Value>, key: &str, slot: &mut String| {
        if let Some(v) = patches.get(key).and_then(Value::as_str) {
            if *slot != v {
                *slot = v.to_string();
                changed.set(true);
            }
        }
    };
    // 数值型补丁（plan.* 用；布尔/字符串之外的第三个类型）
    let apply_usize = |patches: &serde_json::Map<String, Value>, key: &str, slot: &mut usize| {
        if let Some(v) = patches.get(key).and_then(Value::as_u64) {
            let v = v as usize;
            if *slot != v {
                *slot = v;
                changed.set(true);
            }
        }
    };
    let apply_u64 = |patches: &serde_json::Map<String, Value>, key: &str, slot: &mut u64| {
        if let Some(v) = patches.get(key).and_then(Value::as_u64) {
            if *slot != v {
                *slot = v;
                changed.set(true);
            }
        }
    };
    // plan.heartbeat_interval_s 是 i64（其余数值字段是 usize/u64），单独一个闭包
    let apply_i64 = |patches: &serde_json::Map<String, Value>, key: &str, slot: &mut i64| {
        if let Some(v) = patches.get(key).and_then(Value::as_i64) {
            if *slot != v {
                *slot = v;
                changed.set(true);
            }
        }
    };

    let patches = params
        .get("patches")
        .or_else(|| params.get("patch"))
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();

    if let Some(naming) = patches.get("naming").and_then(Value::as_object) {
        apply_bool(naming, "enabled", &mut cfg.naming.enabled);
    }
    if let Some(privacy) = patches.get("privacy").and_then(Value::as_object) {
        apply_bool(
            privacy,
            "disable_network_observability",
            &mut cfg.privacy.disable_network_observability,
        );
    }
    if let Some(memory) = patches.get("memory").and_then(Value::as_object) {
        apply_bool(
            memory,
            "auto_capture_enabled",
            &mut cfg.memory.auto_capture_enabled,
        );
        if let Some(dream) = memory.get("dream").and_then(Value::as_object) {
            apply_bool(dream, "enabled", &mut cfg.dream.enabled);
            apply_bool(dream, "auto_schedule", &mut cfg.dream.auto_schedule);
        }
    }
    if let Some(router) = patches.get("squilla_router").and_then(Value::as_object) {
        if let Some(sl) = router.get("self_learning").and_then(Value::as_object) {
            let prev = cfg.squilla_router.enabled;
            apply_bool(sl, "enabled", &mut cfg.squilla_router.enabled);
            if !prev && cfg.squilla_router.enabled {
                if !cfg.dream.enabled {
                    cfg.dream.enabled = true;
                    changed.set(true);
                    linked.push("memory.dream.enabled".to_string());
                }
                if !cfg.dream.auto_schedule {
                    cfg.dream.auto_schedule = true;
                    changed.set(true);
                    linked.push("memory.dream.auto_schedule".to_string());
                }
                restart_required = true;
            }
        }
    }
    if let Some(model) = patches.get("model").and_then(Value::as_object) {
        // ⚠️ 注意（2026-10-21 实测）：这里写的是 `config.yaml` 的 model 段，
        // 而请求侧 `apply_effective_model()` 会用 `models.json.active` 档案的
        // provider/model/base_url **覆盖**它。也就是说：从本入口改「服务商/模型」
        // 在 models.json 存在时**不会生效**（只改了镜像文件）。
        // 需要真正切换模型请走 `onboarding.provider.configure`（现已档案优先）
        // 或 `onboarding.llmProfile.activate`。本段仅保留给「改 timeout / copilot
        // 等非三要素字段」以及无 models.json 的极简部署使用。
        apply_str(model, "provider", &mut cfg.model.provider);
        apply_str(model, "model_name", &mut cfg.model.model_name);
        apply_str(model, "base_url", &mut cfg.model.base_url);
        // api_key 特殊处理：转写 ~/.marguerite/.env（E3/K1 护栏），YAML 不落钥。
        // 用 write_env_key_for_active：有 active 档案时写该档案**分槽**，
        // 否则写带归属标记的全局槽；单槽写入会让第二家 provider 覆盖第一家。
        if let Some(v) = model.get("api_key").and_then(Value::as_str) {
            if cfg.model.api_key != v {
                cfg.model.api_key = v.to_string();
                changed.set(true);
                Config::write_env_key_for_active(&cfg.model.provider, v);
            }
        }
    }
    if let Some(guard) = patches.get("guard").and_then(Value::as_object) {
        apply_bool(guard, "syntax", &mut cfg.guard.syntax);
        apply_bool(guard, "semantic", &mut cfg.guard.semantic);
        apply_bool(guard, "fact", &mut cfg.guard.fact);
        apply_bool(guard, "contract", &mut cfg.guard.contract);
    }
    if let Some(bloom) = patches.get("bloom").and_then(Value::as_object) {
        apply_bool(bloom, "parallel_enabled", &mut cfg.bloom.parallel_enabled);
    }
    // 计划执行配置（`plan.*`）：其中 `require_approval` 就是 Phase 1 接上的
    // 「计划执行前硬审批闸门」开关。此前这一段**没有任何键接收**，所以计划参数
    // 只能手改 YAML / CLI，WebUI 完全不可见也不可改。
    //
    // 注意：`plan/runner.rs` 的 `cached_plan_config()` 是**进程级 OnceLock**，
    // 首步执行后即冻结 —— 运行时改这些值不会影响已缓存的执行器，因此必须
    // 标 `restart_required`，否则用户会看到「改了没生效」这种假承诺。
    if let Some(plan) = patches.get("plan").and_then(Value::as_object) {
        let before = changed.get();
        apply_bool(plan, "auto_decompose", &mut cfg.plan.auto_decompose);
        apply_bool(plan, "require_approval", &mut cfg.plan.require_approval);
        apply_usize(plan, "max_steps", &mut cfg.plan.max_steps);
        apply_usize(
            plan,
            "checkpoint_interval_steps",
            &mut cfg.plan.checkpoint_interval_steps,
        );
        apply_i64(
            plan,
            "heartbeat_interval_s",
            &mut cfg.plan.heartbeat_interval_s,
        );
        apply_u64(plan, "step_timeout_secs", &mut cfg.plan.step_timeout_secs);
        if changed.get() != before {
            restart_required = true;
        }
    }
    if let Some(routing) = patches.get("model_routing").and_then(Value::as_object) {
        apply_str(routing, "mode", &mut cfg.model_routing.mode);
        apply_str(routing, "fusionModel", &mut cfg.model_routing.fusion_model);
        apply_str(routing, "fusion_model", &mut cfg.model_routing.fusion_model);
        apply_str(routing, "fixedModel", &mut cfg.model_routing.fixed_model);
        apply_str(routing, "fixed_model", &mut cfg.model_routing.fixed_model);
        apply_bool(
            routing,
            "crossProviderTiers",
            &mut cfg.model_routing.cross_provider_tiers,
        );
        apply_bool(
            routing,
            "cross_provider_tiers",
            &mut cfg.model_routing.cross_provider_tiers,
        );
        for key in ["candidateModels", "candidate_models"] {
            if let Some(candidates) = routing.get(key).and_then(Value::as_array) {
                let v: Vec<String> = candidates
                    .iter()
                    .filter_map(|x| x.as_str().map(|s| s.to_string()))
                    .collect();
                if cfg.model_routing.candidate_models != v {
                    cfg.model_routing.candidate_models = v;
                    changed.set(true);
                }
            }
        }
    }
    if let Some(log) = patches.get("log").and_then(Value::as_object) {
        apply_str(log, "level", &mut cfg.log.level);
    }
    // 控制台首屏语言：webui 设置页切换语言时同步到这里（`control_ui.default_locale`）。
    // 此前该键**没有任何分支接收** → 切语言只落 localStorage、网关级设置写而不存，
    // 换浏览器/清缓存后首屏语言仍由硬编码值决定（与 pyengine 同键契约不一致）。
    if let Some(ui) = patches.get("control_ui").and_then(Value::as_object) {
        apply_str(ui, "default_locale", &mut cfg.control_ui.default_locale);
    }

    // dot-path 形态
    apply_bool(&patches, "naming.enabled", &mut cfg.naming.enabled);
    apply_bool(
        &patches,
        "privacy.disable_network_observability",
        &mut cfg.privacy.disable_network_observability,
    );
    apply_bool(
        &patches,
        "memory.auto_capture_enabled",
        &mut cfg.memory.auto_capture_enabled,
    );
    apply_bool(&patches, "memory.dream.enabled", &mut cfg.dream.enabled);
    apply_bool(
        &patches,
        "memory.dream.auto_schedule",
        &mut cfg.dream.auto_schedule,
    );
    apply_str(&patches, "model.provider", &mut cfg.model.provider);
    apply_str(&patches, "model.model_name", &mut cfg.model.model_name);
    apply_str(&patches, "model.base_url", &mut cfg.model.base_url);
    // dot-path 形态的 api_key 同样转写 .env（E3/K1 护栏），YAML 不落钥。
    // provider 已在上面先行应用，这里能正确判定 active 档案归属并写分槽。
    if let Some(v) = patches.get("model.api_key").and_then(Value::as_str) {
        if cfg.model.api_key != v {
            cfg.model.api_key = v.to_string();
            changed.set(true);
            Config::write_env_key_for_active(&cfg.model.provider, v);
        }
    }
    apply_bool(&patches, "guard.syntax", &mut cfg.guard.syntax);
    apply_bool(&patches, "guard.semantic", &mut cfg.guard.semantic);
    apply_bool(&patches, "guard.fact", &mut cfg.guard.fact);
    apply_bool(&patches, "guard.contract", &mut cfg.guard.contract);
    apply_bool(
        &patches,
        "bloom.parallel_enabled",
        &mut cfg.bloom.parallel_enabled,
    );
    apply_str(&patches, "model_routing.mode", &mut cfg.model_routing.mode);
    apply_str(
        &patches,
        "model_routing.fusionModel",
        &mut cfg.model_routing.fusion_model,
    );
    apply_str(
        &patches,
        "model_routing.fixedModel",
        &mut cfg.model_routing.fixed_model,
    );
    apply_bool(
        &patches,
        "model_routing.crossProviderTiers",
        &mut cfg.model_routing.cross_provider_tiers,
    );
    apply_str(&patches, "log.level", &mut cfg.log.level);
    apply_str(
        &patches,
        "control_ui.default_locale",
        &mut cfg.control_ui.default_locale,
    );

    // plan.* 的 dot-path 形态（前端既可能发嵌套对象，也可能发扁平点路径）
    {
        let before = changed.get();
        apply_bool(&patches, "plan.auto_decompose", &mut cfg.plan.auto_decompose);
        apply_bool(
            &patches,
            "plan.require_approval",
            &mut cfg.plan.require_approval,
        );
        apply_usize(&patches, "plan.max_steps", &mut cfg.plan.max_steps);
        apply_usize(
            &patches,
            "plan.checkpoint_interval_steps",
            &mut cfg.plan.checkpoint_interval_steps,
        );
        apply_i64(
            &patches,
            "plan.heartbeat_interval_s",
            &mut cfg.plan.heartbeat_interval_s,
        );
        apply_u64(
            &patches,
            "plan.step_timeout_secs",
            &mut cfg.plan.step_timeout_secs,
        );
        if changed.get() != before {
            restart_required = true;
        }
    }

    {
        let prev = cfg.squilla_router.enabled;
        apply_bool(
            &patches,
            "squilla_router.self_learning.enabled",
            &mut cfg.squilla_router.enabled,
        );
        if !prev && cfg.squilla_router.enabled {
            if !cfg.dream.enabled {
                cfg.dream.enabled = true;
                changed.set(true);
                linked.push("memory.dream.enabled".to_string());
            }
            if !cfg.dream.auto_schedule {
                cfg.dream.auto_schedule = true;
                changed.set(true);
                linked.push("memory.dream.auto_schedule".to_string());
            }
            restart_required = true;
        }
    }

    if !changed.get() {
        return json!({ "ok": true, "restartRequired": false, "changed": false, "linked": [] });
    }

    match cfg.save() {
        Ok(()) => {
            // 保存即生效（2026-10-21 修）：此前这里只 `cfg.save()` 落盘，**从不发布**
            // 进程级生效快照 —— 而 `chat.send` 读的是 `effective_config()`（未播种时
            // 回退启动快照 `ctx.config`），于是「行为 / 隐私 / 计划 / 命名 / 记忆学习」
            // 这些走 `config.patch.safe` 的设置全部表现为「保存成功、重启才生效」。
            // 与 `onboarding.*` / `models.routing.set` 保持同一纪律：写盘 + 发布。
            crate::gateway::chat::publish_effective_config(cfg.clone());
            let mut res = json!({
                "ok": true,
                "restartRequired": restart_required,
                "changed": true,
            });
            if !linked.is_empty() {
                res["linked"] = json!(linked);
                res["linkedLive"] = json!(!restart_required);
            }
            res
        }
        Err(e) => json!({ "ok": false, "error": format!("配置保存失败: {e}") }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::EventBus;
    use crate::gateway::methods::GatewayContext;
    use std::sync::Arc;
    use std::time::Instant;

    fn test_ctx() -> GatewayContext {
        GatewayContext {
            config: Config::default(),
            token: "test-token".to_string(),
            started_at: Instant::now(),
            bus: Arc::new(EventBus::default()),
        }
    }

    /// E3/K1 凭证护栏回归（P0-2）：config.get 响应**任何位置**都不得出现
    /// 明文 API Key；只能暴露 `api_key_configured` 布尔。
    #[test]
    fn test_config_get_never_returns_api_key() {
        // 配置目录隔离：config_get 先 `Config::load()` 读盘，本机已有真实
        // ~/.marguerite/config.yaml（Ollama 无 key）时会盖掉 ctx.config，
        // 使 api_key_configured 恒 false。同文件其它用例统一用 set_config_dir_override。
        let tmp_dir =
            std::env::temp_dir().join(format!("mgt_get_sec_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&tmp_dir).unwrap();
        Config::set_config_dir_override(tmp_dir.clone());

        let mut ctx = test_ctx();
        ctx.config.model.api_key = "sk-secret-p0test-abcdef123456".to_string();

        let v = config_get(&ctx);
        let s = serde_json::to_string(&v).unwrap();
        assert!(
            !s.contains("sk-secret-p0test"),
            "config.get 响应泄露明文密钥: {s}"
        );
        // llm 对象必须保留「是否已配置」布尔（前端设置页依赖）
        assert_eq!(v["llm"]["api_key"], "");
        assert_eq!(v["llm"]["api_key_configured"], true);
        assert_eq!(v["model"]["api_key_configured"], true);

        let _ = std::fs::remove_dir_all(&tmp_dir);
    }

    /// P0-1 配套回归：config.patch.safe 收到 model.api_key 时，
    /// ①内存生效（api_key_configured=true）②响应不含明文 ③YAML 不落钥。
    #[test]
    fn test_config_patch_api_key_goes_to_env_not_yaml() {
        let tmp_dir = std::env::temp_dir().join(format!("mgt_patch_sec_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&tmp_dir).unwrap();
        Config::set_config_dir_override(tmp_dir.clone());

        let ctx = test_ctx();
        let params = json!({
            "patches": { "model": { "api_key": "sk-patch-test-key-9876" } }
        });
        let res = config_patch_local(&ctx, &params);
        assert_eq!(res["ok"], true, "{res}");

        // YAML 落盘后不含密钥
        let yaml_path = tmp_dir.join("config.yaml");
        if yaml_path.exists() {
            let content = std::fs::read_to_string(&yaml_path).unwrap();
            assert!(
                !content.contains("sk-patch-test-key"),
                "patch 后 config.yaml 不得含明文密钥"
            );
        }

        // config.get 回读：配置生效但密钥不回传
        let v = config_get(&ctx);
        let s = serde_json::to_string(&v).unwrap();
        assert!(!s.contains("sk-patch-test-key"), "回读泄露: {s}");
        assert_eq!(v["model"]["api_key_configured"], true);

        let _ = std::fs::remove_dir_all(&tmp_dir);
    }

    /// `control_ui.default_locale` 必须**真落盘并被读回**。
    ///
    /// 此前该键没有任何接收分支：webui 切语言只写 localStorage，网关级设置写而不存
    /// （换浏览器/清缓存后首屏语言与用户所选不一致），与 pyengine 同键契约断裂。
    #[test]
    fn test_config_patch_persists_control_ui_default_locale() {
        let tmp_dir = std::env::temp_dir().join(format!("mgt_patch_loc_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&tmp_dir).unwrap();
        Config::set_config_dir_override(tmp_dir.clone());

        let ctx = test_ctx();
        // webui 的写法：dot-path
        let res = config_patch_local(
            &ctx,
            &json!({ "patches": { "control_ui.default_locale": "zh-Hans" } }),
        );
        assert_eq!(res["ok"], true, "{res}");
        assert_eq!(res["changed"], true, "首写应视为变更: {res}");

        // 落盘 + 回读（config_get 每次从磁盘重载）
        let yaml = std::fs::read_to_string(tmp_dir.join("config.yaml")).unwrap();
        assert!(yaml.contains("default_locale: zh-Hans"), "YAML 未落盘: {yaml}");
        let v = config_get(&ctx);
        assert_eq!(v["control_ui"]["default_locale"], "zh-Hans", "{v}");

        // 嵌套写法同样接受（设置页两种载荷形态都可能出现）
        let res = config_patch_local(
            &ctx,
            &json!({ "patches": { "control_ui": { "default_locale": "ja" } } }),
        );
        assert_eq!(res["changed"], true, "{res}");
        assert_eq!(config_get(&ctx)["control_ui"]["default_locale"], "ja");

        // 同值重复写 → 无变更（不产生无意义的落盘）
        let res = config_patch_local(
            &ctx,
            &json!({ "patches": { "control_ui.default_locale": "ja" } }),
        );
        assert_eq!(res["changed"], false, "同值不应报变更: {res}");

        let _ = std::fs::remove_dir_all(&tmp_dir);
    }

    /// `plan.*` 补丁必须能落盘、能回读，并且**如实标记 restartRequired**。
    ///
    /// 为什么必须标 restart：`plan/runner.rs` 的 `cached_plan_config()` 是进程级
    /// OnceLock（首步执行后即冻结），运行时改这些值不会影响已缓存的执行器 ——
    /// 不标 restart 就是「改了没生效」的假承诺。此前 `plan.*` 段**没有任何接收分支**，
    /// 计划参数（含 Phase 1 新增的 `require_approval` 审批闸门）只能手改 YAML/CLI。
    #[test]
    fn test_config_patch_persists_plan_section() {
        let tmp_dir =
            std::env::temp_dir().join(format!("mgt_patch_plan_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&tmp_dir).unwrap();
        Config::set_config_dir_override(tmp_dir.clone());

        let ctx = test_ctx();
        // 计划审批闸门开关（嵌套对象形态）
        let res = config_patch_local(
            &ctx,
            &json!({ "patches": { "plan": { "require_approval": true } } }),
        );
        assert_eq!(res["ok"], true, "{res}");
        assert_eq!(res["changed"], true, "首写应视为变更: {res}");
        assert_eq!(
            res["restartRequired"], true,
            "执行器配置是进程级缓存，必须要求重启: {res}"
        );
        assert_eq!(
            config_get(&ctx)["plan"]["require_approval"], true,
            "config_get 必须暴露 plan 段，否则设置页读不到"
        );
        let yaml = std::fs::read_to_string(tmp_dir.join("config.yaml")).unwrap();
        assert!(yaml.contains("require_approval: true"), "YAML 未落盘: {yaml}");

        // dot-path 形态同样接受 + 数值字段
        let res = config_patch_local(
            &ctx,
            &json!({ "patches": { "plan.max_steps": 24, "plan.step_timeout_secs": 180 } }),
        );
        assert_eq!(res["changed"], true, "{res}");
        assert_eq!(config_get(&ctx)["plan"]["max_steps"], 24);
        assert_eq!(config_get(&ctx)["plan"]["step_timeout_secs"], 180);

        // 同值重复写 → 无变更（不得因「部分字段没变」而误报变更）
        let res = config_patch_local(
            &ctx,
            &json!({ "patches": { "plan": { "require_approval": true } } }),
        );
        assert_eq!(res["changed"], false, "同值不应报变更: {res}");

        let _ = std::fs::remove_dir_all(&tmp_dir);
    }

    /// Rust-first 的 `config.plan.get` / `config.plan.set` 必须**与 config.yaml 同源**。
    ///
    /// 存在理由：`config.get / config.patch` 是 PY_FIRST —— 侧车在线时它们读写 Python 的
    /// `config.toml`，而**计划执行器（`plan/runner.rs`）读的是 `~/.marguerite/config.yaml`**。
    /// 若设置页用 `config.patch` 写 plan，就会出现「开关已开启、执行时没有闸门」的静默失效
    /// （正是本次 Phase 1 要根除的那类缺陷）。这一对方法必须自证只碰 config.yaml。
    #[test]
    fn test_config_plan_get_set_roundtrip_is_rust_first() {
        let tmp_dir = std::env::temp_dir().join(format!("mgt_plan_rpc_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&tmp_dir).unwrap();
        Config::set_config_dir_override(tmp_dir.clone());

        let ctx = test_ctx();
        // 初始读取：默认不开审批闸门（与 PlanConfig::default 一致）
        let v = config_plan_get(&ctx);
        assert_eq!(v["ok"], true, "{v}");
        assert_eq!(v["source"], "config.yaml", "{v}");
        assert_eq!(v["plan"]["require_approval"], false, "{v}");

        // 打开审批闸门 → 落盘 + 回读 true + 如实标 restartRequired
        let res = config_plan_set(&ctx, &json!({ "require_approval": true }));
        assert_eq!(res["ok"], true, "{res}");
        assert_eq!(res["changed"], true, "{res}");
        assert_eq!(
            res["restartRequired"], true,
            "执行器配置是进程级缓存，必须要求重启: {res}"
        );
        assert_eq!(res["plan"]["require_approval"], true, "{res}");
        assert_eq!(
            config_plan_get(&ctx)["plan"]["require_approval"], true,
            "回读必须反映刚落盘的值"
        );
        let yaml = std::fs::read_to_string(tmp_dir.join("config.yaml")).unwrap();
        assert!(yaml.contains("require_approval: true"), "YAML 未落盘: {yaml}");
        // 同一份投影也出现在 config.get 的 plan 段（形状不允许两处漂移）
        assert_eq!(config_get(&ctx)["plan"]["require_approval"], true);

        // 非法载荷（非对象）不得改坏配置
        let res = config_plan_set(&ctx, &json!("not-an-object"));
        assert_eq!(res["changed"], false, "非法载荷应视为无变更: {res}");
        assert_eq!(config_plan_get(&ctx)["plan"]["require_approval"], true);

        let _ = std::fs::remove_dir_all(&tmp_dir);
    }

    /// 并发写配置**不得静默丢更新**（R52 回归）。
    ///
    /// 每个写路径都是「`Config::load()` → 改内存 → `save()`」的读改写序列：无写锁时两条
    /// 并行写各自基于同一份快照落盘，后者整份覆盖前者，而两边都回 `ok: true`（真机实测
    /// 两条并行 `config.patch.safe` 只有一条落地）。此测试用栅栏把两线程同时推入读改写
    /// 窗口，重复多轮并断言**两边的键都落盘**。
    #[test]
    fn test_concurrent_config_patch_keeps_both_writes() {
        use std::sync::{Arc, Barrier};

        // 线程内的配置目录覆盖是 thread-local，故每个线程各自注入同一临时目录。
        fn in_dir<T>(dir: &std::path::Path, f: impl FnOnce() -> T) -> T {
            Config::set_config_dir_override(dir.to_path_buf());
            f()
        }

        let tmp_dir = std::env::temp_dir().join(format!("mgt_patch_race_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&tmp_dir).unwrap();
        let dir = tmp_dir.clone();

        let ctx = Arc::new(test_ctx());
        let rounds = 20usize;

        for round in 0..rounds {
            // 每轮回到「两键皆 false」的已知初态（顺序写，无竞争）
            in_dir(&dir, || {
                let reset = json!({
                    "patches": {
                        "naming.enabled": false,
                        "privacy.disable_network_observability": false,
                    }
                });
                assert_eq!(config_patch_local(&ctx, &reset)["ok"], true, "初态复位失败");
            });

            let barrier = Arc::new(Barrier::new(2));
            let (d1, d2) = (dir.clone(), dir.clone());
            let (c1, c2) = (ctx.clone(), ctx.clone());
            let (b1, b2) = (barrier.clone(), barrier.clone());

            let h1 = std::thread::spawn(move || {
                b1.wait();
                in_dir(&d1, || {
                    config_patch_local(
                        &c1,
                        &json!({ "patches": { "naming.enabled": true } }),
                    )
                })
            });
            let h2 = std::thread::spawn(move || {
                b2.wait();
                in_dir(&d2, || {
                    config_patch_local(
                        &c2,
                        &json!({ "patches": { "privacy.disable_network_observability": true } }),
                    )
                })
            });
            let r1 = h1.join().expect("线程 1 panic");
            let r2 = h2.join().expect("线程 2 panic");
            assert_eq!(r1["ok"], true, "第 {round} 轮写 1 未成功: {r1}");
            assert_eq!(r2["ok"], true, "第 {round} 轮写 2 未成功: {r2}");

            let snapshot = in_dir(&dir, || config_get(&ctx));
            assert_eq!(
                snapshot["naming"]["enabled"], true,
                "第 {round} 轮：写 1（naming.enabled）被并行写覆盖（静默丢更新）: {r1} / {r2}"
            );
            assert_eq!(
                snapshot["privacy"]["disable_network_observability"], true,
                "第 {round} 轮：写 2（privacy.disable_network_observability）被并行写覆盖（静默丢更新）: {r1} / {r2}"
            );
        }

        let _ = std::fs::remove_dir_all(&tmp_dir);
    }
    /// 「保存即生效」契约：`config.patch.safe`（行为 / 隐私 / 计划 / 命名 / 记忆学习
    /// 等所有设置页）写盘后**必须**发布进程级生效快照。
    ///
    /// 缺陷背景：此前该函数只 `cfg.save()`，而 `chat.send` 读的是
    /// `effective_config()`（未播种时回退启动快照 `ctx.config`）—— 于是这些设置
    /// 全部表现为「保存成功、重启才生效」，与用户口径「配置了但没变」完全一致。
    #[test]
    fn config_patch_publishes_effective_snapshot_so_settings_apply_without_restart() {
        let _g = crate::gateway::chat::effective_config_test_lock();
        crate::gateway::chat::clear_effective_config();
        let dir = std::env::temp_dir().join(format!(
            "mgt-config-publish-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        let _ = std::fs::create_dir_all(&dir);
        crate::persistence::config::Config::set_config_dir_override(dir.clone());
        let ctx = GatewayContext {
            config: crate::persistence::config::Config::default(),
            token: "t".into(),
            started_at: std::time::Instant::now(),
            bus: std::sync::Arc::new(crate::events::EventBus::default()),
        };

        assert!(
            crate::gateway::chat::effective_config().is_none(),
            "前置：快照应为空（否则本用例证明不了「发布」这一步）"
        );

        let res = config_patch_local(&ctx, &json!({ "patches": { "naming": { "enabled": false } } }));
        assert_eq!(res["ok"], true, "{res}");
        assert_eq!(res["changed"], true, "{res}");

        // ① 落盘
        let saved = crate::persistence::config::Config::load().expect("config");
        assert!(!saved.naming.enabled, "naming.enabled 应已写盘");
        // ② 发布（关键）：引擎下一轮就能看到，无需重启。
        //
        // 快照已按配置目录分键（`chat::effective_config_key`），本用例用的是自己
        // 的临时目录，因此可以安全断言**值**——这才真正证明「发布的是刚保存的那份」。
        let published = crate::gateway::chat::effective_config()
            .expect("config.patch.safe 必须发布生效快照，否则设置要重启才生效");
        assert!(!published.naming.enabled, "生效快照未反映本次修改");

        crate::gateway::chat::clear_effective_config();
        let _ = std::fs::remove_dir_all(&dir);
    }

}
