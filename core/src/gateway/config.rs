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
    let llm_profile = json!({
        "model": cfg.model.model_name,
        "base_url": cfg.model.base_url,
    });
    let llm_profiles = {
        // v3.1：合并 models.json 用户档案 + 当前主 provider。
        // webui 的 storedProfileIds 依赖该映射判定「已保存的服务商」；
        // 此前只有单一主 provider 条目，保存的档案在设置页不可见（空状态）。
        let mut m = serde_json::Map::new();
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
                    }),
                );
            }
        }
        // 主 provider 始终可见（即使无档案行）
        m.entry(cfg.model.provider.clone())
            .or_insert_with(|| llm_profile.clone());
        m
    };
    json!({
        "model": {
            "provider": cfg.model.provider,
            "model_name": cfg.model.model_name,
            "base_url": cfg.model.base_url,
            "api_key_configured": !cfg.model.api_key.is_empty(),
        },
        "llm": {
            "provider": cfg.model.provider,
            "model": cfg.model.model_name,
            "base_url": cfg.model.base_url,
            // 密钥绝不回传前端：仅暴露是否已配置（E3/K1 凭证护栏）
            "api_key": "",
            "api_key_configured": !cfg.model.api_key.is_empty(),
        },
        "llm_profiles": llm_profiles,
        "llm_request_timeout_seconds": 60,
        "mcp": {
            "servers": cfg.mcp.servers.iter().map(|s| json!({
                "name": s.name,
                "command": s.command,
                "args": s.args,
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
    })
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
        apply_str(model, "provider", &mut cfg.model.provider);
        apply_str(model, "model_name", &mut cfg.model.model_name);
        apply_str(model, "base_url", &mut cfg.model.base_url);
        // api_key 特殊处理：转写 ~/.marguerite/.env（E3/K1 护栏），YAML 不落钥
        if let Some(v) = model.get("api_key").and_then(Value::as_str) {
            if cfg.model.api_key != v {
                cfg.model.api_key = v.to_string();
                changed.set(true);
                Config::write_env_key(v);
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
    // dot-path 形态的 api_key 同样转写 .env（E3/K1 护栏），YAML 不落钥
    if let Some(v) = patches.get("model.api_key").and_then(Value::as_str) {
        if cfg.model.api_key != v {
            cfg.model.api_key = v.to_string();
            changed.set(true);
            Config::write_env_key(v);
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
}
