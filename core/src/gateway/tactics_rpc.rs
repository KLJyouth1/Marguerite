//! 战术推演 RPC（`tactics.*`）
//!
//! 三条只读接口，全部走 Rust 原生分支（不代理 Python 侧车）：
//!
//! - `tactics.catalog`：内置战术目录（含「有闸门 / 无闸门」对照项）与缺省先验；
//! - `tactics.simulate`：给定任务 DAG 与先验，跑完整个目录并给出裁决 + 事前验尸；
//! - `tactics.replay`：把 harness 台账里的一次真实运行折算成场景，做反事实对照，
//!   并用 5-Why 链把「为什么是这样」拆到可核对的证据为止。
//!
//! 三条诚实约束：
//!
//! 1. 推演结果**不是预测**，是声明先验下的相对排序；`unknowns` 非空时前端必须显式
//!    展示「这些数字没有实测支撑」（与 `context::compact_state` 的未知 ≠ 0 同源）。
//! 2. 回放里台账没有的字段进 `gaps`，不补零、不猜：例如台账不记录真实依赖形状，
//!    回放按条目先后假设为串行链，并明确标注 `measured-sequential`。
//! 3. 参数非法一律 `INVALID_REQUEST`，且校验发生在任何计算/读库之前。
//!
//! 错误三段式（`ok:false + error + code`）：
//! - `INVALID_REQUEST`：参数缺失或结构非法（DAG 成环、依赖不存在、task 过量…）；
//! - `LEDGER_UNAVAILABLE`：台账被禁用或打开/查询失败；
//! - `LEDGER_RUN_NOT_FOUND`：`tactics.replay` 指定的 run 不存在。

use serde_json::{json, Value};

use crate::harness::ledger;
use crate::tactics;

fn invalid(msg: impl Into<String>) -> Value {
    json!({ "ok": false, "error": msg.into(), "code": "INVALID_REQUEST" })
}

fn unavailable(err: impl std::fmt::Display) -> Value {
    json!({
        "ok": false,
        "enabled": false,
        "error": err.to_string(),
        "code": "LEDGER_UNAVAILABLE",
    })
}

/// 把可序列化对象摊成 `{ ok: true, ...fields }`，与其余 RPC 的扁平形状一致。
fn ok_with<T: serde::Serialize>(value: &T) -> Value {
    match serde_json::to_value(value) {
        Ok(Value::Object(mut m)) => {
            m.insert("ok".into(), json!(true));
            Value::Object(m)
        }
        Ok(other) => json!({ "ok": true, "value": other }),
        Err(e) => json!({
            "ok": false,
            "error": format!("结果序列化失败: {e}"),
            "code": "SERIALIZE_FAILED",
        }),
    }
}

/// `tactics.catalog`：战术目录 + 缺省参数。
///
/// 目录里同时给「同一拓扑有/无闸门」两条，是为了让「验证本身值不值」成为可被
/// 数据裁决的问题；缺省先验一并返回（带 `n=0` 的诚实标记），前端不需要再抄一份。
pub fn tactics_catalog() -> Value {
    let tactics = tactics::catalog();
    json!({
        "ok": true,
        "count": tactics.len(),
        "tactics": tactics,
        "topologies": tactics::Topology::ALL.iter().map(|t| t.as_key()).collect::<Vec<_>>(),
        "defaults": {
            "trials": tactics::model::DEFAULT_TRIALS,
            "maxTrials": tactics::model::MAX_TRIALS,
            "maxFailProbability": 0.15,
            "model": tactics::Model::default(),
        },
    })
}

/// `tactics.simulate`：跑完整推演并裁决。
pub fn tactics_simulate(params: &Value) -> Value {
    let scenario = match tactics::Scenario::from_params(params) {
        Ok(s) => s,
        Err(e) => return invalid(e),
    };
    let criteria = tactics::Criteria::from_params(params, &scenario);
    ok_with(&tactics::plan(&scenario, &criteria))
}

/// `tactics.replay`：用台账里一次真实运行做反事实回放。
pub fn tactics_replay(params: &Value) -> Value {
    let Some(run_id) = params
        .get("runId")
        .or_else(|| params.get("run_id"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
    else {
        return invalid("runId 不可为空");
    };
    let trials = params
        .get("trials")
        .and_then(Value::as_u64)
        .map(|n| (n as usize).clamp(1, tactics::model::MAX_TRIALS))
        .unwrap_or(tactics::model::DEFAULT_TRIALS);
    let seed = params.get("seed").and_then(Value::as_u64).unwrap_or(42);
    let criteria = tactics::Criteria::from_params_standalone(params);

    let Some(l) = ledger::global() else {
        return unavailable("harness 台账不可用（未启用或打开失败），无法回放真实运行");
    };
    let (runs, _) = match l.query_filtered(None, Some(run_id), None, 1) {
        Ok(v) => v,
        Err(e) => return unavailable(format!("查询运行台账失败: {e}")),
    };
    let Some(run) = runs.into_iter().next() else {
        return json!({
            "ok": false,
            "error": format!("运行记录不存在: {run_id}"),
            "code": "LEDGER_RUN_NOT_FOUND",
        });
    };

    // 只做投影，不改台账语义：run 级 token 优先，条目级 token 作为回退；
    // 缺 name 的条目退到 item_type，绝不补一个看似真实的假名字。
    let observed = tactics::ObservedRun {
        run_id: run.run_id.clone(),
        target: run.target.clone(),
        status: run.status.clone(),
        elapsed_ms: run.elapsed_ms.unwrap_or(0).max(0) as f64,
        tokens: (run.prompt_tokens + run.output_tokens).max(0) as f64,
        ok: run.status == "ok",
        items: run
            .items
            .iter()
            .map(|i| tactics::MeasuredItem {
                seq: i.seq,
                name: if i.name.trim().is_empty() {
                    i.item_type.clone()
                } else {
                    i.name.clone()
                },
                elapsed_ms: i.elapsed_ms.max(0) as f64,
                tokens: (i.tokens_in + i.tokens_out).max(0) as f64,
                ok: i.ok,
                error_code: i.error_code.clone(),
            })
            .collect(),
    };

    let report = tactics::replay(&observed, &tactics::catalog(), trials, seed, &criteria);
    ok_with(&report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_returns_the_gate_contrast_and_defaults() {
        let v = tactics_catalog();
        assert_eq!(v["ok"], true);
        let list = v["tactics"].as_array().expect("tactics 数组");
        assert!(!list.is_empty());
        assert!(
            list.iter().any(|t| t["gate"].is_null()),
            "目录必须包含无闸门战术（否则无法裁决验证本身的价值）"
        );
        assert!(
            list.iter().any(|t| !t["gate"].is_null()),
            "目录必须包含有闸门战术"
        );
        assert_eq!(
            v["defaults"]["model"]["failureRate"]["n"], 0,
            "缺省先验没有实测样本，必须如实报 n=0"
        );
    }

    #[test]
    fn simulate_rejects_bad_scenarios_before_any_work() {
        let v = tactics_simulate(&json!({}));
        assert_eq!(v["code"], "INVALID_REQUEST");
        let v = tactics_simulate(&json!({ "tasks": [] }));
        assert_eq!(v["code"], "INVALID_REQUEST");
        let v = tactics_simulate(&json!({
            "tasks": [
                {"id": "a", "dependsOn": ["b"]},
                {"id": "b", "dependsOn": ["a"]}
            ]
        }));
        assert_eq!(v["code"], "INVALID_REQUEST");
        assert!(
            v["error"].as_str().unwrap_or_default().contains('a'),
            "成环报错必须点名节点，否则用户无从修起"
        );
    }

    #[test]
    fn simulate_returns_a_flat_contract_the_console_can_render() {
        let v = tactics_simulate(&json!({
            "tasks": [
                {"id": "a", "workK": 20},
                {"id": "b", "workK": 20, "dependsOn": ["a"]}
            ],
            "trials": 100,
            "seed": 3
        }));
        assert_eq!(v["ok"], true);
        assert!(v["rationale"].is_object(), "必须有裁决");
        assert!(
            !v["unknowns"].as_array().expect("unknowns 数组").is_empty(),
            "声明先验必须被标成未知"
        );
        let results = v["results"].as_array().expect("results 数组");
        assert!(results.len() >= 5, "目录里每条战术都要有结果");
        assert!(
            results.iter().all(|r| r["successRate"].is_number()),
            "结果字段必须用 camelCase（前端契约）"
        );
    }

    #[test]
    fn replay_requires_a_run_id() {
        let v = tactics_replay(&json!({ "runId": "  " }));
        assert_eq!(v["code"], "INVALID_REQUEST");
        let v = tactics_replay(&json!({}));
        assert_eq!(v["code"], "INVALID_REQUEST");
    }
}
