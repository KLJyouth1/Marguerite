//! Mode 方法族（从 methods.rs 分治迁出）
//!
//! 覆盖 `mode.*` 方法：运行时模式查询 / 切换 / 规格快照 / 蜂群配置。
//! 与 `models.routing.*` 联动（omni 路由会联动切换到 omni-transcend 模式）。

use serde_json::{json, Value};

use crate::gateway::methods::GatewayContext;
use crate::gateway::protocol::RpcError;
use crate::persistence::config::Config;

/// `mode.get`：返回当前有效模式 + 3×3 用户模式视图。
pub(crate) fn mode_get() -> Value {
    use crate::domain::UserMode;

    let runtime = crate::mode::runtime_mode();
    let effective_kind: crate::mode::ModeKind = if runtime == crate::mode::ModeKind::Bloom {
        crate::mode::ModeKind::Bloom
    } else {
        let cfg = Config::load()
            .map(|c| c.bloom.parallel_enabled)
            .unwrap_or(false);
        if cfg {
            crate::mode::ModeKind::Bloom
        } else {
            runtime
        }
    };
    let available: Vec<&str> = crate::mode::ModeKind::all_names();
    let catalog: Vec<Value> = crate::mode::ModeKind::ALL
        .iter()
        .map(|m| {
            let spec = crate::mode::spec::builtin::for_kind(*m);
            json!({
                "kind": m.as_str(),
                "label": m.label(),
                "description": spec.description,
                "riskLevel": format!("{:?}", spec.risk_level),
                "hasBloomSpec": spec.bloom.is_some(),
            })
        })
        .collect();

    // 3×3 用户模式视图（唯一合法用户模式）：用户只见「极简/标准/极致 × 咨询/方案/执行」，
    // 旧 bloom/ensemble/omni 能力经 from_user_mode 收编进执行列，不再作为顶层 UI 出现。
    let current = crate::mode::default_user_mode_for(effective_kind);
    let user_modes: Vec<Value> = UserMode::all()
        .iter()
        .map(|m| {
            json!({
                "key": m.as_key(),
                "label": m.label(),
                "intensity": m.intensity().as_key(),
                "intensityLabel": m.intensity().label(),
                "working": m.working().as_key(),
                "workingLabel": m.working().label(),
                "agentMode": m.agent_mode().as_key(),
            })
        })
        .collect();

    json!({
        "kind": effective_kind.as_str(),
        "runtimeKind": runtime.as_str(),
        "available": available,
        "catalog": catalog,
        "userMode": json!({
            "key": current.as_key(),
            "label": current.label(),
            "intensity": current.intensity().as_key(),
            "intensityLabel": current.intensity().label(),
            "working": current.working().as_key(),
            "workingLabel": current.working().label(),
            "agentMode": current.agent_mode().as_key(),
        }),
        "userModes": user_modes,
    })
}

/// `mode.spec`：当前有效模式的规格快照。
pub(crate) fn mode_spec() -> Value {
    let runtime = crate::mode::runtime_mode();
    let spec = crate::mode::spec::builtin::for_kind(runtime);
    let label = runtime.label();
    let bloom = spec.bloom.as_ref().map(|x| {
        let plan = crate::model::ensemble_deep::DeepPlan::from_mode_spec(
            x.thinking_depth,
            x.verification_passes,
        );
        json!({
            "max_swarm_shards": x.max_swarm_shards,
            "max_sub_experts": x.max_sub_experts,
            "thinking_depth": x.thinking_depth,
            "verification_passes": x.verification_passes,
            "effective_refine_rounds": plan.refine_rounds,
            "effective_verify_rounds": plan.verify_rounds,
            "effective_max_llm_calls": plan.max_llm_calls(),
            "reasoning_effort": x.reasoning_effort,
            "effective_thinking_effort": format!(
                "{:?}",
                crate::model::thinking::ThinkingEffort::from_spec_str(&x.reasoning_effort)
            ),
            "temperature": x.temperature,
            "snapshot_enabled": x.snapshot_enabled,
            "auto_revert_max": x.auto_revert_max,
            "auto_retry_failures": x.auto_retry_failures,
            "budget_tokens": x.budget_tokens,
            "max_iterations": x.max_iterations,
            "max_fixes": x.max_fixes,
            "anchoring_enabled": x.anchoring.enabled,
            "anchoring_minimal_tools": x.anchoring.minimal_tools,
            "anchoring_ptc_tool": x.anchoring.ptc_tool,
            "anchoring_bootstrap_max_tokens": x.anchoring.bootstrap_max_tokens,
            "privileges_auto_approve": x.privileges.auto_approve,
            "privileges_max_concurrent_tools": x.privileges.max_concurrent_tools,
        })
    });
    json!({
        "kind": runtime.as_str(),
        "label": label,
        "risk_level": format!("{:?}", spec.risk_level),
        "approval_mutating": format!("{:?}", spec.approval_policy.mutating),
        "allowed_tools": spec.allowed_tools.len(),
        "denied_tools": spec.denied_tools.len(),
        "bloom": bloom,
    })
}

/// `mode.stage`：Bloom 三段锚定的权威阶段（V2.2-Bloom RPC）。
///
/// 数据优先级（诚实标注来源，绝不冒充）：
/// 1. 会话锚定台账有真实推进 → `runtime: true, source: "session"`（权威值）；
/// 2. 否则按当前有效模式规格推导 → `runtime: false, source: "spec"`（与前端旧规则一致）。
///
/// 阶段字符串与前端 STAGES 枚举一致：disabled / anchored / ptc / full。
pub(crate) fn mode_stage(params: &Value) -> Value {
    use crate::mode::anchoring::AnchorStage;

    let runtime = crate::mode::runtime_mode();
    let session = params.get("session").and_then(Value::as_str).unwrap_or("");

    // 1) 会话锚定台账（权威值）
    if let Some(stage) = crate::gateway::chat::session_anchor_stage(session) {
        let s = match stage {
            AnchorStage::Anchored => "anchored",
            AnchorStage::Ptc => "ptc",
            AnchorStage::Full => "full",
        };
        return json!({
            "ok": true,
            "runtime": true,
            "stage": s,
            "source": "session",
            "session": if session.is_empty() { Value::Null } else { json!(session) },
        });
    }

    // 2) 无台账 → 按规格推导（与前端旧推导规则保持同构；诚实标注非运行时）
    let spec = crate::mode::spec::builtin::for_kind(runtime);
    let b = spec.bloom.as_ref();
    let anchored_enabled = b.map(|x| x.anchoring.enabled).unwrap_or(false);
    let stage = if !anchored_enabled {
        "disabled"
    } else {
        let depth = b.map(|x| x.thinking_depth).unwrap_or(0);
        let conc = b.map(|x| x.privileges.max_concurrent_tools).unwrap_or(0);
        let vpass = b.map(|x| x.verification_passes).unwrap_or(0);
        let experts = b.map(|x| x.max_sub_experts).unwrap_or(0);
        if depth >= 3 && conc >= 2 && vpass >= 2 && experts >= 3 {
            "full"
        } else if depth >= 3 && conc >= 2 {
            "ptc"
        } else {
            "anchored"
        }
    };
    json!({
        "ok": true,
        "runtime": false,
        "stage": stage,
        "source": "spec",
        "session": Value::Null,
    })
}

/// `mode.swarm.get`：蜂群规模设置（用户所选即生效，不再有硬上限）。
pub(crate) fn mode_swarm_get() -> Value {
    let cfg = crate::mode::swarm::load();
    let runtime = crate::mode::runtime_mode();
    let default_shards = if runtime == crate::mode::ModeKind::OmniTranscend {
        13
    } else {
        11
    };
    json!({
        "configuredShards": if cfg.max_swarm_shards == 0 { serde_json::Value::Null } else { json!(cfg.max_swarm_shards) },
        "configuredSubExperts": if cfg.max_sub_experts == 0 { serde_json::Value::Null } else { json!(cfg.max_sub_experts) },
        "effectiveShards": crate::mode::swarm::effective_shards(default_shards),
        "effectiveSubExperts": crate::mode::swarm::effective_sub_experts(5),
        // 显式声明「无硬上限」：此前这里回 200 / 50，前端据此把输入框钉死在旧上限。
        "maxShardsCap": serde_json::Value::Null,
        "maxSubExpertsCap": serde_json::Value::Null,
    })
}

/// `mode.swarm.set`：设置蜂群规模（0 = 未配置；其余按用户所选原样保存）。
pub(crate) fn mode_swarm_set(params: &Value) -> Value {
    let mut cfg = crate::mode::swarm::load();
    if let Some(v) = params.get("maxSwarmShards").and_then(Value::as_u64) {
        cfg.max_swarm_shards = v as usize;
    } else if let Some(v) = params.get("max_swarm_shards").and_then(Value::as_u64) {
        cfg.max_swarm_shards = v as usize;
    }
    if let Some(v) = params.get("maxSubExperts").and_then(Value::as_u64) {
        cfg.max_sub_experts = v as usize;
    } else if let Some(v) = params.get("max_sub_experts").and_then(Value::as_u64) {
        cfg.max_sub_experts = v as usize;
    }
    match crate::mode::swarm::save(&cfg) {
        Ok(()) => mode_swarm_get(),
        Err(e) => json!({ "ok": false, "error": e }),
    }
}

/// `mode.switch`（Rust 异步分支）：切换运行时模式并持久化到配置。
pub(crate) async fn mode_switch(ctx: &GatewayContext, params: &Value) -> Result<Value, RpcError> {
    // 3×3 优先：`userMode`（如 "standard_doit"）经 from_user_mode 派生运行时执行档；
    // 兼容旧路径：`kind`/`mode` 直接给 ModeKind 名。
    use crate::domain::UserMode;
    let target = if let Some(um_key) = params
        .get("userMode")
        .and_then(Value::as_str)
        .or_else(|| params.get("user_mode").and_then(Value::as_str))
    {
        let um = UserMode::from_key(um_key)
            .ok_or_else(|| RpcError::new("INVALID_REQUEST", format!("无效用户模式: {um_key}")))?;
        crate::mode::from_user_mode(um)
    } else {
        let kind = params
            .get("kind")
            .and_then(Value::as_str)
            .or_else(|| params.get("mode").and_then(Value::as_str))
            .unwrap_or("agent");
        crate::mode::ModeKind::from_name(kind)
            .ok_or_else(|| RpcError::new("INVALID_REQUEST", format!("无效模式: {kind}")))?
    };

    // 记录切换前的模式：事件与审计日志的 `from` 必须反映**真实**来源。
    // 历史缺陷：两者都硬编码 `"from": "agent"`，于是从 plan/bloom 切到别的模式时，
    // 前端与审计流看到的都是「agent → X」，模式切换历史被系统性记错。
    let previous = crate::mode::runtime_mode();
    crate::mode::set_runtime_mode(target);

    let mut cfg = Config::load().unwrap_or_else(|_| ctx.config.clone());
    cfg.bloom.parallel_enabled = target == crate::mode::ModeKind::Bloom;
    if let Err(e) = cfg.save() {
        tracing::warn!("mode.switch：配置持久化失败（运行时模式已生效，重启后可能回退）: {e}");
    }

    ctx.bus.publish(crate::events::Envelope::mode_changed(
        "core",
        previous.as_str(),
        target.as_str(),
    ));

    // M3-D1 产品事件：切换 Mode（总线已由上方发布，这里仅落审计日志）
    crate::events::product::audit(
        crate::events::product::MODE_CHANGED,
        "core",
        serde_json::json!({ "from": previous.as_str(), "to": target.as_str() }),
    );

    Ok(json!({
        "ok": true,
        "kind": target.as_str(),
        "runtimeKind": crate::mode::runtime_mode().as_str(),
        "userMode": crate::mode::default_user_mode_for(target).as_key(),
    }))
}

/// `models.routing.set`：设置模型路由模式并持久化到 config.yaml。
pub(crate) async fn models_routing_set(
    ctx: &GatewayContext,
    params: &Value,
) -> Result<Value, RpcError> {
    let mode = params
        .get("mode")
        .and_then(Value::as_str)
        .unwrap_or("direct")
        .trim()
        .to_lowercase();
    if !matches!(
        mode.as_str(),
        "direct" | "router" | "ensemble" | "bloom" | "omni"
    ) {
        return Err(RpcError::new(
            "INVALID_REQUEST",
            format!("无效路由模式: {mode}（应为 direct/router/ensemble/bloom/omni）"),
        ));
    }
    let mut cfg = Config::load().unwrap_or_else(|_| ctx.config.clone());
    cfg.model_routing.mode = mode.clone();
    if let Err(e) = cfg.save() {
        tracing::warn!("models.routing.set：配置持久化失败（运行时已生效，重启后可能回退）: {e}");
    }
    // R72+：保存即生效 —— 发布进程级生效快照，运行引擎下一轮即用新路由模式，无需重启
    // （与 `onboarding_router_configure` 一致；此前此处只落盘不发布，模式切换可能不即时生效）。
crate::gateway::chat::publish_effective_config(cfg.clone());
    // P3 打通：路由模式保存广播 models.changed，前端 useChatFeatureToggles 改订
    // models.changed（此前订阅的 models.routing.changed 在后端从未发布，是死订阅）。
    crate::events::product::emit(
        "models.changed",
        "",
        json!({ "reason": "routing_set", "mode": mode }),
    );
    if mode == "omni" {
        let _ = mode_switch(ctx, &json!({ "kind": "omni-transcend" })).await;
    } else if crate::mode::runtime_mode() == crate::mode::ModeKind::OmniTranscend {
        let _ = mode_switch(ctx, &json!({ "kind": "agent" })).await;
    }
    Ok(json!({
        "ok": true,
        "mode": mode,
    }))
}

/// `models.routing.get`：读取当前模型路由模式。
pub(crate) fn models_routing_get(ctx: &GatewayContext) -> Value {
    let cfg = Config::load().unwrap_or_else(|_| ctx.config.clone());
    json!({
        "ok": true,
        "mode": cfg.model_routing.mode,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const GHOST: &str = "mode-stage-ghost-nonexistent";

    #[test]
    fn mode_get_emits_three_by_three_user_modes() {
        let _g = crate::mode::test_runtime_mode_guard();
        let v = mode_get();
        let ums = v["userModes"].as_array().expect("userModes 应为数组");
        assert_eq!(ums.len(), 9, "3×3 应有 9 档");
        let keys: Vec<&str> = ums.iter().map(|u| u["key"].as_str().unwrap()).collect();
        for expect in [
            "minimal_ask", "minimal_plan", "minimal_doit", "standard_ask", "standard_plan",
            "standard_doit", "extreme_ask", "extreme_plan", "extreme_doit",
        ] {
            assert!(keys.contains(&expect), "缺少用户模式键 {expect}");
            assert!(v["userMode"]["key"].is_string() && v["userMode"]["intensity"].is_string());
        }
        assert!(v["userMode"]["working"].is_string());
    }

    #[tokio::test]
    // 该 guard 串行化的正是「清空生效快照 → 异步 set → 断言快照」这段过程，
    // 必须跨 await 持有。`std::sync::MutexGuard` 是 `!Send`，持有它的 future
    // 无法迁移线程，不存在跨线程死锁路径；`#[tokio::test]` 每用例独占运行时。
    #[allow(clippy::await_holding_lock)]
    async fn models_routing_set_publishes_effective() {
        // 隔离配置目录，避免污染真实 ~/.marguerite/config.yaml
        let tmp = std::env::temp_dir().join(format!("marg-routing-test-{}", std::process::id()));
        let _ = std::fs::create_dir_all(tmp.join("config"));
        crate::persistence::safe_mode::apply_env(
            &crate::persistence::safe_mode::prepare(tmp.clone()).unwrap(),
        );
        let ctx = crate::gateway::methods::GatewayContext {
            config: crate::persistence::config::Config::default(),
            token: "t".into(),
            started_at: std::time::Instant::now(),
            bus: std::sync::Arc::new(crate::events::EventBus::default()),
        };
        // 生效快照是进程级全局：与其它断言快照的用例串行化，避免互相污染。
        let _g = crate::gateway::chat::effective_config_test_lock();
        crate::gateway::chat::clear_effective_config();
        let res = models_routing_set(&ctx, &json!({ "mode": "router" })).await;
        assert!(res.is_ok(), "models.routing.set 应成功: {res:?}");
        // 真源（文件）：路由模式已落盘
        let saved = crate::persistence::config::Config::load().unwrap();
        assert_eq!(saved.model_routing.mode, "router", "路由模式应已保存");
        // 生效快照已发布（与 onboarding_router_configure 一致）：运行引擎下一轮即用新路由
        assert!(
            crate::gateway::chat::effective_config().is_some(),
            "应已发布进程级生效快照"
        );
    }

    #[test]
    fn mode_stage_derives_from_spec_when_no_session_anchor() {
        let _g = crate::mode::test_runtime_mode_guard();
        let prev = crate::mode::runtime_mode();
        crate::mode::set_runtime_mode(crate::mode::ModeKind::Bloom);
        // 显式指定不存在的会话 → 走规格推导路径（不依赖全局 recent 顺序）
        let v = mode_stage(&json!({ "session": GHOST }));
        assert_eq!(v["ok"], true);
        assert_eq!(v["runtime"], false);
        assert_eq!(v["source"], "spec");
        assert!(
            ["disabled", "anchored", "ptc", "full"].contains(&v["stage"].as_str().unwrap()),
            "spec 推导阶段应为合法枚举，got {:?}",
            v["stage"]
        );
        crate::mode::set_runtime_mode(prev);
    }

    #[test]
    fn mode_stage_returns_authoritative_session_anchor() {
        let _g = crate::mode::test_runtime_mode_guard();
        let prev = crate::mode::runtime_mode();
        crate::mode::set_runtime_mode(crate::mode::ModeKind::Bloom);
        let session = "mode-stage-test-session";
        crate::gateway::chat::test_seed_anchor_stage(
            session,
            crate::mode::anchoring::AnchorStage::Full,
        );
        let v = mode_stage(&json!({ "session": session }));
        assert_eq!(v["ok"], true);
        assert_eq!(v["runtime"], true);
        assert_eq!(v["stage"], "full");
        assert_eq!(v["source"], "session");
        crate::mode::set_runtime_mode(prev);
    }

    #[test]
    fn mode_stage_disabled_for_non_bloom_mode() {
        let _g = crate::mode::test_runtime_mode_guard();
        let prev = crate::mode::runtime_mode();
        crate::mode::set_runtime_mode(crate::mode::ModeKind::Agent);
        let v = mode_stage(&json!({ "session": GHOST }));
        assert_eq!(v["ok"], true);
        assert_eq!(v["runtime"], false);
        assert_eq!(v["stage"], "disabled");
        crate::mode::set_runtime_mode(prev);
    }
}
