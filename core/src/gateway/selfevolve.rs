//! 元智能体自迭代闭环的 RPC 门面（`selfevolve.*`）
//!
//! 与 [`crate::harness::meta`] 的分工：那里是五层闭环的**内核**（纯函数 + 可重放台账），
//! 这里只做网关门面 —— 参数解析、数据目录解析、错误映射，不含任何业务判断。
//!
//! | 方法 | 语义 | 写入 |
//! |------|------|------|
//! | `selfevolve.status` | 策略 / 生效版本 / 待审提案 | 否 |
//! | `selfevolve.signals` | 从行为台账抽取的失败信号 | 否 |
//! | `selfevolve.analyze` | 诊断 + 改进规划（预演，不登记） | 否 |
//! | `selfevolve.history` | 按版本折叠的演进史 | 否 |
//! | `selfevolve.sweep` | 监控→诊断→规划→登记提案（无候选评估） | 追加台账 |
//! | `selfevolve.cycle` | 完整一轮（需显式提供**真实回放**的基线/候选） | 追加台账 |
//! | `selfevolve.approve` / `reject` / `rollback` | 人工审核与回滚 | 追加台账 |
//!
//! 命名选用 `selfevolve.` 而非 `meta.`：后者是既有 Python 侧车方法族
//! （`meta.run` / `meta.runs.*`），语义不同，不得混淆。

use serde_json::{json, Value};

use crate::gateway::protocol::RpcError;
use crate::harness::meta::{
    self, EvalOutcome, MetaAgent, MetaLedger, MetaPolicy, PromotionVerdict, RolloutStage,
    VersionSnapshot,
};
use crate::persistence::config::Config;

/// 信号窗口默认条数（行为台账是环形的，取最近一段即可）
const DEFAULT_SIGNAL_WINDOW: usize = 200;

/// 元智能体 RPC 分派（仅处理 `selfevolve.` 前缀）
pub fn dispatch(method: &str, params: &Value) -> Result<Value, RpcError> {
    match method {
        "selfevolve.status" => Ok(agent().status()),
        "selfevolve.signals" => Ok(signals_json(params)),
        "selfevolve.analyze" => Ok(analyze_json(params)),
        "selfevolve.history" => Ok(history_json()),
        "selfevolve.sweep" => sweep(params),
        "selfevolve.cycle" => cycle(params),
        "selfevolve.approve" => approve(params),
        "selfevolve.reject" => reject(params),
        "selfevolve.promote" => promote(params),
        "selfevolve.rollback" => rollback(params),
        _ => Err(RpcError::new(
            "NOT_IMPLEMENTED",
            format!("未知的元智能体方法：{method}"),
        )),
    }
}

fn agent() -> MetaAgent {
    MetaAgent::new(Config::data_dir(), MetaPolicy::default())
}

fn window(params: &Value) -> usize {
    params
        .get("limit")
        .and_then(Value::as_u64)
        .map(|n| n as usize)
        .unwrap_or(DEFAULT_SIGNAL_WINDOW)
        .clamp(1, 2000)
}

/// `selfevolve.signals`：监控层输出（只读）
fn signals_json(params: &Value) -> Value {
    let items: Vec<Value> = meta::observe_recent(window(params))
        .into_iter()
        .map(|s| {
            json!({
                "kind": s.kind.as_str(),
                "session": s.session,
                "tool": s.tool,
                "detail": s.detail,
                "at_ms": s.at_ms,
            })
        })
        .collect();
    json!({ "window": window(params), "count": items.len(), "items": items })
}

/// `selfevolve.analyze`：诊断 + 改进规划预演（**不登记台账**，可安全定时轮询）
fn analyze_json(params: &Value) -> Value {
    let signals = meta::observe_recent(window(params));
    let policy = MetaPolicy::default();
    let Some(diag) = meta::diagnose(&signals) else {
        return json!({
            "signals": 0,
            "diagnosis": Value::Null,
            "plan": Value::Null,
            "message": "观察窗口内没有失败信号，无需改进",
        });
    };
    let plan = meta::plan_improvement(&diag, &policy);
    json!({
        "signals": diag.signals,
        "diagnosis": {
            "cause": diag.cause.as_str(),
            "confidence": diag.confidence,
            "evidence": diag.evidence,
        },
        "plan": plan.as_ref().map(|p| json!({
            "target": p.target.as_str(),
            "risk": p.risk.as_str(),
            "mutation": p.mutation,
            "rationale": p.rationale,
        })),
        "auto_apply": policy.auto_apply,
        "message": match &plan {
            Some(p) => format!("根因 {} → 建议改「{}」（风险 {}）", diag.cause.as_str(), p.target.as_str(), p.risk.as_str()),
            None => format!("根因 {} 未达提案条件（置信度 {:.2}）", diag.cause.as_str(), diag.confidence),
        },
    })
}

/// `selfevolve.history`：折叠后的版本史 + 原始记录（审计用）
fn history_json() -> Value {
    let ledger = MetaLedger::new(Config::data_dir());
    let folded = ledger.folded();
    json!({
        "versions": folded.iter().map(snapshot_json).collect::<Vec<_>>(),
        "records": ledger.history().unwrap_or_default().iter().map(snapshot_json).collect::<Vec<_>>(),
        "active_version": ledger.active().map(|s| s.version),
        "rollback_target": ledger.rollback_target().map(|s| s.version),
    })
}

fn snapshot_json(s: &VersionSnapshot) -> Value {
    json!({
        "version": s.version,
        "at": s.at,
        "stage": s.stage.as_str(),
        "cause": s.cause.as_str(),
        "target": s.plan.target.as_str(),
        "risk": s.plan.risk.as_str(),
        "mutation": s.plan.mutation,
        "baseline": s.baseline.as_ref().map(outcome_json),
        "candidate": s.candidate.as_ref().map(outcome_json),
        "verdict": s.verdict.as_ref().map(verdict_json),
        "note": s.note,
    })
}

fn outcome_json(o: &EvalOutcome) -> Value {
    json!({
        "passed": o.passed,
        "total": o.total,
        "tokens": o.tokens,
        "ms": o.ms,
        "success_rate": o.success_rate(),
    })
}

fn verdict_json(v: &PromotionVerdict) -> Value {
    // 裁决本身已带 tag/content 结构（verdict + detail），这里只补一个布尔方便前端分流
    let mut j = serde_json::to_value(v).unwrap_or(Value::Null);
    if let Some(o) = j.as_object_mut() {
        o.insert("promoted".to_string(), json!(v.is_promote()));
    }
    j
}

/// `selfevolve.sweep`：监控 → 诊断 → 规划 → 登记提案（等价于 `run_cycle` 无候选）
fn sweep(params: &Value) -> Result<Value, RpcError> {
    let signals = meta::observe_recent(window(params));
    let mut agent = agent();
    // 无候选评估：本轮只做「识别 + 提案」，不改任何真实配置（路线 A）。
    // 基线如实传 None —— 尚未回放就是尚未回放，不伪造成一次测量。
    let report = agent.run_cycle(&signals, None, None);
    let mut v = serde_json::to_value(&report)
        .map_err(|e| RpcError::new("INTERNAL", format!("报告序列化失败：{e}")))?;
    v["observed_signals"] = json!(signals.len());
    v["status"] = agent.status();
    Ok(v)
}

/// `selfevolve.cycle`：带**真实回放结果**的完整一轮。
///
/// 基线/候选必须由调用方在沙箱里真跑出来（本函数不臆造评分）；缺基线 → 拒绝，
/// 而不是拿默认值凑一个「看起来通过了」的裁决。
fn cycle(params: &Value) -> Result<Value, RpcError> {
    let baseline = parse_outcome(params.get("baseline"))
        .ok_or_else(|| RpcError::new("INVALID_PARAMS", "baseline 缺失或字段不全（需 passed/total/tokens/ms）"))?;
    let candidate = match params.get("candidate") {
        None | Some(Value::Null) => None,
        Some(v) => Some(parse_outcome(Some(v)).ok_or_else(|| {
            RpcError::new("INVALID_PARAMS", "candidate 字段不全（需 passed/total/tokens/ms）")
        })?),
    };
    let signals = meta::observe_recent(window(params));
    let mut agent = agent();
    let report = agent.run_cycle(&signals, Some(baseline), candidate);
    let mut v = serde_json::to_value(&report)
        .map_err(|e| RpcError::new("INTERNAL", format!("报告序列化失败：{e}")))?;
    v["observed_signals"] = json!(signals.len());
    v["status"] = agent.status();
    Ok(v)
}

fn parse_outcome(v: Option<&Value>) -> Option<EvalOutcome> {
    let v = v?;
    let get = |k: &str| v.get(k).and_then(Value::as_u64);
    Some(EvalOutcome {
        passed: get("passed")? as usize,
        total: get("total")? as usize,
        tokens: get("tokens")?,
        ms: get("ms")?,
    })
}

/// `selfevolve.approve`：人工批准待审提案
fn approve(params: &Value) -> Result<Value, RpcError> {
    let version = required_version(params)?;
    let note = params.get("note").and_then(Value::as_str).map(str::to_string);
    let mut agent = agent();
    let snap = agent
        .approve(version, note)
        .map_err(|e| RpcError::new("INVALID_STATE", e.to_string()))?;
    Ok(json!({ "snapshot": snapshot_json(&snap), "status": agent.status() }))
}

/// `selfevolve.reject`：人工否决尚未生效的提案
fn reject(params: &Value) -> Result<Value, RpcError> {
    let version = required_version(params)?;
    let reason = params
        .get("reason")
        .and_then(Value::as_str)
        .unwrap_or("人工否决");
    let mut agent = agent();
    let snap = agent
        .reject(version, reason)
        .map_err(|e| RpcError::new("INVALID_STATE", e.to_string()))?;
    Ok(json!({ "snapshot": snapshot_json(&snap), "status": agent.status() }))
}

/// `selfevolve.promote`：阶段推进（approved → canary → live）。
///
/// 与 [`approve`] 分开：批准是「人工同意这个改动」，灰度是「分批放量」——
/// 两件事，不能一步跳过（`Proposed` 直接 `Live` 会被台账状态机拒绝）。
fn promote(params: &Value) -> Result<Value, RpcError> {
    let version = required_version(params)?;
    let stage = match params.get("stage").and_then(Value::as_str) {
        Some("canary") => RolloutStage::Canary,
        Some("live") => RolloutStage::Live,
        _ => return Err(RpcError::new("INVALID_PARAMS", "stage 需为 canary 或 live")),
    };
    let mut agent = agent();
    let snap = match stage {
        RolloutStage::Canary => agent.promote_canary(version),
        _ => agent.promote_live(version),
    }
    .map_err(|e| RpcError::new("INVALID_STATE", e.to_string()))?;
    Ok(json!({ "snapshot": snapshot_json(&snap), "status": agent.status() }))
}

/// `selfevolve.rollback`：把当前生效版本标记为已回滚
fn rollback(params: &Value) -> Result<Value, RpcError> {
    let reason = params
        .get("reason")
        .and_then(Value::as_str)
        .unwrap_or("人工回滚");
    let mut agent = agent();
    let snap = agent
        .rollback(reason)
        .map_err(|e| RpcError::new("INVALID_STATE", e.to_string()))?;
    Ok(json!({ "snapshot": snapshot_json(&snap), "status": agent.status() }))
}

fn required_version(params: &Value) -> Result<u32, RpcError> {
    params
        .get("version")
        .and_then(Value::as_u64)
        .filter(|v| *v > 0 && *v <= u32::MAX as u64)
        .map(|v| v as u32)
        .ok_or_else(|| RpcError::new("INVALID_PARAMS", "version 缺失或非法（需为正整数）"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_method_is_rejected() {
        let e = dispatch("selfevolve.nope", &json!({})).unwrap_err();
        assert_eq!(e.code, "NOT_IMPLEMENTED");
    }

    #[test]
    fn outcome_parsing_requires_all_fields() {
        assert!(parse_outcome(Some(&json!({ "passed": 1, "total": 2 }))).is_none());
        let ok = parse_outcome(Some(&json!({ "passed": 1, "total": 2, "tokens": 3, "ms": 4 })))
            .expect("四字段齐全应可解析");
        assert_eq!((ok.passed, ok.total, ok.tokens, ok.ms), (1, 2, 3, 4));
        assert!(parse_outcome(None).is_none());
    }

    #[test]
    fn cycle_refuses_to_invent_a_baseline() {
        // 没有真实回放数据时不得凑一份「看起来通过」的裁决
        let e = dispatch("selfevolve.cycle", &json!({})).unwrap_err();
        assert_eq!(e.code, "INVALID_PARAMS");
    }

    #[test]
    fn approve_requires_a_positive_version() {
        assert_eq!(
            dispatch("selfevolve.approve", &json!({})).unwrap_err().code,
            "INVALID_PARAMS"
        );
        assert_eq!(
            dispatch("selfevolve.approve", &json!({ "version": 0 }))
                .unwrap_err()
                .code,
            "INVALID_PARAMS"
        );
    }

    #[test]
    fn promote_requires_an_explicit_stage() {
        // 缺 stage / 非法 stage → 拒绝；不允许「默认推到最远」这种危险猜测
        for bad in [json!({ "version": 1 }), json!({ "version": 1, "stage": "live!" })] {
            let e = dispatch("selfevolve.promote", &bad).unwrap_err();
            assert_eq!(e.code, "INVALID_PARAMS", "入参 {bad} 应被拒绝");
        }
    }

    #[test]
    fn version_above_u32_is_rejected() {
        let e = dispatch("selfevolve.approve", &json!({ "version": u64::MAX })).unwrap_err();
        assert_eq!(e.code, "INVALID_PARAMS");
    }
}