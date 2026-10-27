//! Harness 运行台账 RPC（`harness.ledger.*`）
//!
//! 只读观测面：状态总览 / 运行查询 / 确定性回放摘要。
//! 数据源是 [`crate::harness::ledger`]（SQLite `harness.db`），不做任何内存假数据。
//!
//! 错误三段式（`ok:false + error + code`）：
//! - `LEDGER_UNAVAILABLE`：台账被禁用（`MARGUERITE_HARNESS_LEDGER=0`）或打开/查询失败；
//! - `LEDGER_RUN_NOT_FOUND`：`harness.ledger.replay` 指定的 run 不存在。

use serde_json::{json, Value};

use crate::harness::ledger;

/// 默认查询条数与上限（与 `harness.ledger.query` 约定一致）
const DEFAULT_LIMIT: usize = 20;
const MAX_LIMIT: usize = 100;

fn unavailable(err: impl std::fmt::Display) -> Value {
    json!({
        "ok": false,
        "enabled": false,
        "error": err.to_string(),
        "code": "LEDGER_UNAVAILABLE",
    })
}

/// `harness.status`：原生 harness 的一次性可观测快照。
/// 为什么需要它：此前 harness 对外只有 `harness.ledger.*` 三条台账接口 —— 页面能看
/// 「跑了什么」，却看不到「harness 本身长什么样」（钩子有没有注册、能力成熟度、
/// 路由策略、子代理预算、工作区/数据目录是否可用）。这里把**真实组件的真实状态**
/// 摊开，并给出 `problems[]`，避免用户只能靠猜。
///
/// 所有字段都来自真实构造（`HarnessConfig::default()` / `HookRunner` /
/// `CapabilityRegistry` / `HarnessLedger`），缺什么就报什么，不写占位值。
pub fn harness_status() -> Value {
    let defaults = crate::harness::HarnessConfig::default();
    // 工作区：与 `Harness` 默认值同源（进程 cwd）；存在性单独标注，不假装可用。
    let workspace = defaults.workspace.clone();
    let workspace_exists = workspace.is_dir();
    let data_dir = crate::persistence::config::Config::data_dir();
    let data_dir_exists = data_dir.is_dir();

    // 钩子：真实注册一遍工作区守卫，再按阶段读出名字与数量。
    let hooks = crate::harness::HookRunner::new();
    hooks.register_workspace_guard(workspace.clone());
    let phases = [
        crate::harness::HookPhase::PreToolCall,
        crate::harness::HookPhase::PostToolCall,
        crate::harness::HookPhase::PreSubagent,
        crate::harness::HookPhase::PostSubagent,
        crate::harness::HookPhase::OnError,
    ];
    let hook_table: Vec<Value> = phases
        .iter()
        .map(|p| {
            json!({
                "phase": p.as_str(),
                "count": hooks.count(*p),
                "names": hooks.hook_names(*p),
            })
        })
        .collect();

    // 能力注册表：成熟度分布来自 `Maturity`，不四舍五入成"全稳定"。
    let registry = crate::harness::CapabilityRegistry::new();
    let capabilities: Vec<Value> = registry
        .all()
        .iter()
        .map(|c| {
            json!({
                "id": c.capability.as_str(),
                "summary": c.summary,
                "distilledFrom": c.distilled_from,
                "maturity": c.maturity.as_str(),
                "modulePath": c.module_path,
            })
        })
        .collect();
    let mut counts = serde_json::Map::new();
    for m in [
        crate::harness::Maturity::Stable,
        crate::harness::Maturity::Beta,
        crate::harness::Maturity::Experimental,
    ] {
        counts.insert(m.as_str().to_string(), json!(registry.by_maturity(m).len()));
    }

    let mut problems: Vec<String> = Vec::new();
    if !workspace_exists {
        problems.push(format!("工作区不存在：{}", workspace.display()));
    }
    if !data_dir_exists {
        problems.push(format!(
            "数据目录不存在：{}（sidechain 转录与台账会创建失败）",
            data_dir.display()
        ));
    }

    // 台账：能打开就报真实计数，打不开如实报错并计入 problems。
    let ledger_json = match ledger::global() {
        Some(l) => match l.status() {
            Ok(st) => json!({
                "available": true,
                "enabled": true,
                "path": l.path().display().to_string(),
                "schemaVersion": ledger::SCHEMA_VERSION,
                "runs": st.runs,
                "items": st.items,
                "bytes": st.bytes,
            }),
            Err(e) => {
                problems.push(format!("台账总览读取失败：{e}"));
                json!({ "available": false, "enabled": true, "error": e.to_string() })
            }
        },
        None => {
            let note = "harness 台账未启用（MARGUERITE_HARNESS_LEDGER=0 或打开失败）";
            problems.push(note.to_string());
            json!({ "available": false, "enabled": false, "error": note })
        }
    };

    json!({
        "ok": true,
        "workspace": workspace.display().to_string(),
        "workspaceExists": workspace_exists,
        "dataDir": data_dir.display().to_string(),
        "dataDirExists": data_dir_exists,
        "subagents": {
            "maxConcurrent": defaults.max_concurrent,
            "budgetPerSubagent": defaults.budget_per_subagent,
            "timeoutSecs": defaults.subagent_timeout.as_secs(),
        },
        "routing": {
            "policy": defaults.policy.as_str(),
            "policies": crate::harness::RoutingPolicy::ALL.iter().map(|p| p.as_str()).collect::<Vec<_>>(),
        },
        "hooks": hook_table,
        "capabilities": capabilities,
        "capabilityCounts": counts,
        "ledger": ledger_json,
        "problems": problems,
    })
}

/// `harness.ledger.status`：真实计数 + 体积 + 成功率 / 耗时分布 / token 汇总
pub fn harness_ledger_status() -> Value {
    let Some(l) = ledger::global() else {
        return unavailable("harness 台账不可用（未启用或打开失败），可通过 MARGUERITE_HARNESS_LEDGER=0 显式关闭");
    };
    match l.status() {
        Ok(st) => json!({
            "ok": true,
            "enabled": true,
            "path": l.path().display().to_string(),
            "schemaVersion": ledger::SCHEMA_VERSION,
            "runs": st.runs,
            "items": st.items,
            "bytes": st.bytes,
            "oldestTsMs": st.oldest_ts_ms,
            "newestTsMs": st.newest_ts_ms,
            "successRate": (st.success_rate * 100.0).round() / 100.0,
            "avgElapsedMs": st.avg_elapsed_ms.round() as u64,
            "p95ElapsedMs": st.p95_elapsed_ms.round() as u64,
            "tokensIn": st.tokens_in,
            "tokensOut": st.tokens_out,
            "lastError": st.last_error,
            // 验收结论分布（四个取值都在，缺的报 0；不把 ok 冒充成 passed）
            "verdicts": st.verdicts,
        }),
        Err(e) => unavailable(format!("读取台账总览失败: {e}")),
    }
}

/// `harness.ledger.query`：按 sessionKey / runId 查询 run 及其 items
pub fn harness_ledger_query(params: &Value) -> Value {
    let Some(l) = ledger::global() else {
        return unavailable("harness 台账不可用（未启用或打开失败）");
    };
    let session_key = params
        .get("sessionKey")
        .or_else(|| params.get("session_key"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let run_id = params
        .get("runId")
        .or_else(|| params.get("run_id"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let limit = params
        .get("limit")
        .and_then(Value::as_u64)
        .map(|v| (v as usize).clamp(1, MAX_LIMIT))
        .unwrap_or(DEFAULT_LIMIT);
    // 验收结论过滤：非法取值直接拒绝（不静默退化成"不过滤"）。
    let verdict = params
        .get("verdict")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty());
    if let Some(v) = verdict {
        if !ledger::is_valid_verdict(v) {
            return json!({
                "ok": false,
                "error": format!("非法 verdict `{v}`，仅支持 {:?}", ledger::VERDICTS),
                "code": "INVALID_VERDICT",
            });
        }
    }

    match l.query_filtered(session_key, run_id, verdict, limit) {
        Ok((runs, truncated)) => {
            let runs: Vec<Value> = runs
                .into_iter()
                .map(|r| {
                    let items: Vec<Value> = r
                        .items
                        .iter()
                        .map(|i| {
                            json!({
                                "seq": i.seq,
                                "type": i.item_type,
                                "name": i.name,
                                "status": i.status,
                                "elapsedMs": i.elapsed_ms,
                                "tokensIn": i.tokens_in,
                                "tokensOut": i.tokens_out,
                                "ok": i.ok,
                                "errorCode": i.error_code,
                            })
                        })
                        .collect();
                    json!({
                        "runId": r.run_id,
                        "sessionKey": r.session_id,
                        "traceId": r.trace_id,
                        "target": r.target,
                        "status": r.status,
                        "startedAtMs": r.started_at_ms,
                        "endedAtMs": r.ended_at_ms,
                        "elapsedMs": r.elapsed_ms,
                        "promptTokens": r.prompt_tokens,
                        "outputTokens": r.output_tokens,
                        "model": r.model,
                        "policy": r.policy,
                        "verdict": r.verdict,
                        "itemCount": r.item_count,
                        "items": items,
                    })
                })
                .collect();
            json!({ "ok": true, "runs": runs, "truncated": truncated })
        }
        Err(e) => unavailable(format!("查询运行台账失败: {e}")),
    }
}

/// `harness.ledger.replay`：同一 run 的 items 必须得到同一摘要（时间戳不入摘要）
pub fn harness_ledger_replay(params: &Value) -> Value {
    // 参数校验先于台账访问：非法请求不该去碰数据库。
    let Some(run_id) = params
        .get("runId")
        .or_else(|| params.get("run_id"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
    else {
        return json!({
            "ok": false,
            "error": "runId 不可为空",
            "code": "INVALID_REQUEST",
        });
    };
    let Some(l) = ledger::global() else {
        return unavailable("harness 台账不可用（未启用或打开失败）");
    };
    match l.run_exists(run_id) {
        Ok(true) => match l.replay_digest(run_id) {
            Ok(d) => json!({
                "ok": true,
                "runId": run_id,
                "digest": d.digest,
                "itemCount": d.item_count,
                "deterministic": true,
                "note": "SHA-256 over seq|type|name|status|tokens_in|tokens_out|elapsed_ms 按 id 升序串接；时间戳不入摘要，同一输入序列必须得到同一摘要",
            }),
            Err(e) => unavailable(format!("计算回放摘要失败: {e}")),
        },
        Ok(false) => json!({
            "ok": false,
            "error": format!("运行记录不存在: {run_id}"),
            "code": "LEDGER_RUN_NOT_FOUND",
        }),
        Err(e) => unavailable(format!("查询运行记录失败: {e}")),
    }
}

/// `harness.ledger.verdict`：显式写入 run 的验收结论（完成门禁 / 人工复核）。
///
/// 取值只有 `passed | failed | unverified | unknown`；非法取值与不存在的 run 都必须
/// 明确报错（不静默改写成 `unverified`，也不补写幽灵行）。
pub fn harness_ledger_verdict(params: &Value) -> Value {
    let Some(run_id) = params
        .get("runId")
        .or_else(|| params.get("run_id"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
    else {
        return json!({
            "ok": false,
            "error": "runId 不可为空",
            "code": "INVALID_REQUEST",
        });
    };
    let Some(verdict) = params
        .get("verdict")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
    else {
        return json!({
            "ok": false,
            "error": "verdict 不可为空",
            "code": "INVALID_REQUEST",
        });
    };
    if !ledger::is_valid_verdict(verdict) {
        return json!({
            "ok": false,
            "error": format!("非法 verdict `{verdict}`，仅支持 {:?}", ledger::VERDICTS),
            "code": "INVALID_VERDICT",
        });
    }
    let Some(l) = ledger::global() else {
        return unavailable("harness 台账不可用（未启用或打开失败）");
    };
    match l.set_verdict(run_id, verdict) {
        Ok(true) => json!({ "ok": true, "runId": run_id, "verdict": verdict }),
        Ok(false) => json!({
            "ok": false,
            "error": format!("运行记录不存在: {run_id}"),
            "code": "LEDGER_RUN_NOT_FOUND",
        }),
        Err(e) => unavailable(format!("写入验收结论失败: {e}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    /// 非法请求必须在触碰台账之前拒绝（与全局台账是否可用无关）。
    #[test]
    fn replay_with_empty_run_id_is_invalid_request() {
        let v = harness_ledger_replay(&json!({ "runId": "  " }));
        assert_eq!(v["ok"], false);
        assert_eq!(v["code"], "INVALID_REQUEST");
    }

    /// `harness.status` 必须自洽：能力数与成熟度分布一致、五阶段钩子齐全、
    /// 工作区守卫真的注册上了、problems 只在真有问题时给出字符串。
    #[test]
    fn harness_status_reports_real_components() {
        let v = harness_status();
        assert_eq!(v["ok"], true, "只读快照永远返回 ok（问题走 problems[]）");
        let caps = v["capabilities"].as_array().expect("capabilities 数组");
        assert!(!caps.is_empty(), "能力注册表不得为空");
        let counted: usize = v["capabilityCounts"]
            .as_object()
            .expect("capabilityCounts 对象")
            .values()
            .map(|n| n.as_u64().unwrap_or(0) as usize)
            .sum();
        assert_eq!(counted, caps.len(), "成熟度分布之和必须等于能力条数");

        let hooks = v["hooks"].as_array().expect("hooks 数组");
        assert_eq!(hooks.len(), 5, "五阶段钩子都要出现");
        let pre = hooks
            .iter()
            .find(|h| h["phase"] == "pre_tool_call")
            .expect("pre_tool_call 阶段");
        assert!(
            pre["count"].as_u64().unwrap_or(0) >= 1,
            "工作区守卫必须真的注册在 PreToolCall（否则越界写不会被拦）"
        );
        assert!(v["subagents"]["timeoutSecs"].as_u64().unwrap_or(0) > 0);
        assert!(v["problems"]
            .as_array()
            .expect("problems 数组")
            .iter()
            .all(Value::is_string));
    }

    /// 写结论接口的参数校验必须发生在触碰台账之前。
    #[test]
    fn verdict_rpc_validates_before_touching_ledger() {
        let v = harness_ledger_verdict(&json!({ "verdict": "passed" }));
        assert_eq!(v["code"], "INVALID_REQUEST", "缺 runId");
        let v = harness_ledger_verdict(&json!({ "runId": "r1" }));
        assert_eq!(v["code"], "INVALID_REQUEST", "缺 verdict");
        let v = harness_ledger_verdict(&json!({ "runId": "r1", "verdict": "ok" }));
        assert_eq!(v["code"], "INVALID_VERDICT", "status 的 ok 不是合法结论");
    }

    /// 查询过滤同样先校验：非法结论不能被静默当成"不过滤"。
    #[test]
    fn query_rejects_invalid_verdict_before_ledger() {
        let v = harness_ledger_query(&json!({ "verdict": "maybe" }));
        assert_eq!(v["ok"], false);
        assert_eq!(v["code"], "INVALID_VERDICT");
    }
}
