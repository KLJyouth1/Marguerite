//! 协作战术推演工具（`tactics`）：把「要不要派子代理 / 派几个 / 加不加验收」
//! 从拍脑袋变成可核对的一次推演。
//!
//! ## 为什么主智能体需要它
//!
//! 编排层此前只会「按任务类型挑策略」，而策略选择没有任何可复算的依据：
//! 星型、专家团、蜂群、接力各自赌什么、失败会长什么样，全靠提示词里的形容词。
//! 结果就是同一类任务时而派 3 个子代理、时而单干，事后也没人说得清哪个更好。
//!
//! 本工具把 `tactics.*` RPC 的**同一批处理函数**接到工具面（与界面同源，不写第二套逻辑）：
//!
//! | action | 复用的处理函数 | 产出 |
//! |---|---|---|
//! | `catalog` | `gateway::tactics_rpc::tactics_catalog` | 7 条战术 + 缺省先验（`n=0` 如实标注） |
//! | `plan` | `gateway::tactics_rpc::tactics_simulate` | 裁决 + 落选原因 + pre-mortem + 未知先验 |
//! | `replay` | `gateway::tactics_rpc::tactics_replay` | 台账实测 vs 各战术的反事实差值 + 5-Why |
//!
//! ## 诚实边界
//!
//! 推演是**声明先验下的确定性蒙特卡洛**，不是预测、更不是证据：同一 seed 同一结果，
//! 换先验就换结论。工具文本必须原样带出 `unknowns`（没有实测样本的先验）与这句提醒，
//! 否则模型会把「推荐 star-gated」讲成「star-gated 一定更好」。

use std::collections::HashMap;

use anyhow::Result;
use serde_json::{json, Value};

use crate::agent::tool_cycle::ToolResult;
use crate::mcp::registry::ToolExecutor;

/// `tactics` 支持的动作（与 `tactics.*` RPC 一一对应，不新增契约）。
pub const TACTICS_ACTIONS: &[&str] = &["catalog", "plan", "replay"];

const DESCRIPTION: &str = "协作战术推演（solo / 星型子智能体 / 专家团 / 蜂群 / 接力）：\
    在把任务拆给子代理之前，用同一份任务 DAG 比较各战术的成功率、p90 时间与 tokens、\
    失败原因分布，并给出可推翻的裁决（kill 判据）、每个落选者的败因和 pre-mortem。\
    action=catalog 看目录与缺省先验；action=plan 传 scenario（tasks DAG，可选 budgetTokens/\
    deadlineMs/priors/trials/seed）得到推荐与事前验尸；action=replay 传 runId，把 harness \
    台账里一次真实运行折算成场景做反事实回放（含 5-Why 链）。\
    这是模型推演不是实测：没有实测样本的先验会被列为 unknown（n=0），别把推荐当成预测；\
    要对真实运行取证请用 action=replay。编排多智能体任务前先 plan 一次，再决定派几个、要不要验收。";

/// 工具入口：与界面共用 `tactics.*` 处理函数，纯计算、无副作用（Plan 模式可用）。
pub fn tactics_tool() -> Box<dyn ToolExecutor> {
    let run: crate::tools::builtin::ToolFn = std::sync::Arc::new(|args: &HashMap<String, String>| {
        let args = args.clone();
        Box::pin(async move { run_tactics_tool(args).await })
    });
    crate::tools::builtin::make_local_tool(
        "tactics",
        DESCRIPTION,
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": TACTICS_ACTIONS,
                    "description": "catalog=战术目录；plan=对新场景推演；replay=回放台账里的真实运行"
                },
                "scenario": {
                    "type": "object",
                    "description": "plan 必填：{\"tasks\":[{\"id\":\"a\",\"workK\":40,\"dependsOn\":[]}], \
                                    \"budgetTokens\":60000,\"deadlineMs\":120000,\"trials\":2000,\"seed\":42,\
                                    \"priors\":{\"failureRate\":0.08,\"pricePerMTokUsd\":2}}。\
                                    depend 形状必须无环、id 唯一；workK 单位是千 token。"
                },
                "run_id": {
                    "type": "string",
                    "description": "replay 必填：harness 台账里的 run_id（先 harness.status 查最近运行）"
                },
                "trials": {"type": "integer", "description": "可选：试跑次数，默认 2000，上限 20000"},
                "seed": {"type": "integer", "description": "可选：随机种子，默认 42；同 seed 结果逐字节可复现"},
                "max_fail_probability": {
                    "type": "number",
                    "description": "可选：kill 判据里的失败概率上限，默认 0.15"
                }
            },
            "required": ["action"]
        }),
        run,
    )
}

fn arg_str(args: &HashMap<String, String>, key: &str) -> String {
    args.get(key).map(|s| s.trim().to_string()).unwrap_or_default()
}

fn arg_u64(args: &HashMap<String, String>, key: &str) -> Option<u64> {
    arg_str(args, key).parse::<u64>().ok()
}

fn arg_f64(args: &HashMap<String, String>, key: &str) -> Option<f64> {
    arg_str(args, key)
        .parse::<f64>()
        .ok()
        .filter(|x| x.is_finite())
}

/// 处理函数返回 `ok:false` 时如实转成工具错误（R-10：绝不把失败渲染成成功）。
fn tool_failure(v: &Value, what: &str) -> ToolResult {
    let message = v
        .get("error")
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .unwrap_or("推演未执行，且没有给出原因");
    let code = v
        .get("code")
        .and_then(Value::as_str)
        .unwrap_or("TACTICS_FAILED");
    ToolResult::Error {
        message: format!("{what}未执行：{message}"),
        code: code.to_string(),
    }
}

fn num(v: &Value, key: &str) -> f64 {
    v.get(key).and_then(Value::as_f64).unwrap_or(0.0)
}

fn fmt_ms(ms: f64) -> String {
    if ms >= 60_000.0 {
        format!("{:.0}m{:02.0}s", (ms / 60_000.0).floor(), (ms % 60_000.0) / 1000.0)
    } else if ms >= 1000.0 {
        format!("{:.1}s", ms / 1000.0)
    } else {
        format!("{ms:.0}ms")
    }
}

fn fmt_tokens(t: f64) -> String {
    if t >= 1_000_000.0 {
        format!("{:.2}M", t / 1_000_000.0)
    } else if t >= 1000.0 {
        format!("{:.1}k", t / 1000.0)
    } else {
        format!("{t:.0}")
    }
}

fn fmt_usd(x: f64) -> String {
    if x <= 0.0 {
        "$0".to_string()
    } else if x < 0.01 {
        "<$0.01".to_string()
    } else {
        format!("${x:.2}")
    }
}

fn fmt_pct(x: f64) -> String {
    format!("{:.1}%", x * 100.0)
}

/// 缺省先验渲染：`failureRate=8.0% (n=0, declared)` —— `n` 与来源必须跟数字一起出现，
/// 否则「8%」会被读成实测值。
fn render_priors(model: &Value) -> String {
    let fields = [
        "failureRate",
        "throughputKPerMin",
        "pricePerMTokUsd",
        "selfDetect",
        "retryBackoff",
    ];
    fields
        .iter()
        .filter_map(|f| {
            let p = model.get(*f)?;
            let value = p.get("value").and_then(Value::as_f64)?;
            let n = p.get("n").and_then(Value::as_u64).unwrap_or(0);
            let source = p.get("source").and_then(Value::as_str).unwrap_or("unknown");
            let rendered = match *f {
                "failureRate" | "selfDetect" => fmt_pct(value),
                "pricePerMTokUsd" => fmt_usd(value),
                "throughputKPerMin" => format!("{value:.1}k tok/min"),
                _ => format!("{value:.2}"),
            };
            Some(format!("{f}={rendered} (n={n}, {source})"))
        })
        .collect::<Vec<_>>()
        .join(" · ")
}

fn render_catalog(v: &Value) -> String {
    let mut out = String::new();
    let count = v.get("count").and_then(Value::as_u64).unwrap_or(0);
    let defaults = v.get("defaults").cloned().unwrap_or_else(|| json!({}));
    out.push_str(&format!(
        "协作战术目录（{count} 条 · 缺省 trials={} · 失败概率上限 {}）\n",
        defaults.get("trials").and_then(Value::as_u64).unwrap_or(0),
        fmt_pct(num(&defaults, "maxFailProbability")),
    ));
    if let Some(list) = v.get("tactics").and_then(Value::as_array) {
        for t in list {
            let id = t.get("id").and_then(Value::as_str).unwrap_or("?");
            let topology = t.get("topology").and_then(Value::as_str).unwrap_or("?");
            let width = t.get("width").and_then(Value::as_u64).unwrap_or(1);
            let retries = t.get("retries").and_then(Value::as_u64).unwrap_or(0);
            let gate = match t.get("gate") {
                Some(g) if !g.is_null() => {
                    let detect = num(g, "detect");
                    let overhead = num(g, "tokenOverhead");
                    format!("闸门 检出{} · 开销{}", fmt_pct(detect), fmt_pct(overhead))
                }
                _ => "无闸门（缺陷会静默传播）".to_string(),
            };
            let bet = t.get("bet").and_then(Value::as_str).unwrap_or("");
            out.push_str(&format!(
                "- {id:<12} 拓扑 {topology:<6} 并发 {width} · 重试 {retries} · {gate} —— {bet}\n"
            ));
        }
    }
    out.push_str(&format!(
        "缺省先验：{}\n",
        render_priors(&defaults["model"])
    ));
    out.push_str(
        "用法：action=plan 传 scenario（tasks DAG + 可选预算/时限/先验）；\
         有/无闸门的对照项就是用来裁决「验收值不值」的。\n",
    );
    out
}

fn render_plan(v: &Value) -> String {
    let mut out = String::new();
    let rationale = &v["rationale"];
    let no_viable = rationale
        .get("noViableTactic")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    out.push_str(&format!(
        "协作战术推演（trials={} · seed={} · 总工作量 {} tok）\n",
        v.get("trials").and_then(Value::as_u64).unwrap_or(0),
        v.get("seed").and_then(Value::as_u64).unwrap_or(0),
        fmt_tokens(num(v, "totalWorkK") * 1000.0),
    ));

    let recommended = rationale.get("recommended").and_then(Value::as_str);
    let results = v.get("results").and_then(Value::as_array).cloned().unwrap_or_default();
    let winner = recommended.and_then(|id| {
        results
            .iter()
            .find(|r| r.get("tactic").and_then(Value::as_str) == Some(id))
    });

    if no_viable || recommended.is_none() {
        out.push_str(&format!(
            "裁决：没有任何战术同时满足判据 —— {}\n",
            rationale
                .get("reason")
                .and_then(Value::as_str)
                .unwrap_or("未给出原因")
        ));
    } else if let (Some(id), Some(w)) = (recommended, winner) {
        out.push_str(&format!(
            "推荐：{id}（{} · 并发 {} · {}）\n",
            w.get("topology").and_then(Value::as_str).unwrap_or("?"),
            w.get("width").and_then(Value::as_u64).unwrap_or(1),
            if w.get("gated").and_then(Value::as_bool).unwrap_or(false) {
                "有闸门"
            } else {
                "无闸门"
            },
        ));
        out.push_str(&format!(
            "  成功率 {} · p50 {} · p90 {} · p90 tokens {} · p90 成本 {} · 协调占比 {} · 瓶颈 {}\n",
            fmt_pct(num(w, "successRate")),
            fmt_ms(num(w, "p50Ms")),
            fmt_ms(num(w, "p90Ms")),
            fmt_tokens(num(w, "p90Tokens")),
            fmt_usd(num(w, "p90CostUsd")),
            fmt_pct(num(w, "coordinationShare")),
            w.get("bottleneck").and_then(Value::as_str).unwrap_or("-"),
        ));
        if let Some(verdict) = rationale
            .get("verdicts")
            .and_then(Value::as_array)
            .and_then(|vs| {
                vs.iter().find(|x| x.get("tactic").and_then(Value::as_str) == Some(id))
            })
        {
            let checks: Vec<String> = verdict
                .get("checks")
                .and_then(Value::as_array)
                .map(|cs| {
                    cs.iter()
                        .map(|c| {
                            let id = c.get("id").and_then(Value::as_str).unwrap_or("?");
                            let unit = match id {
                                "budget-tokens" => fmt_tokens(num(c, "actual")),
                                "deadline" => fmt_ms(num(c, "actual")),
                                _ => fmt_pct(num(c, "actual")),
                            };
                            let threshold = match id {
                                "budget-tokens" => fmt_tokens(num(c, "threshold")),
                                "deadline" => fmt_ms(num(c, "threshold")),
                                _ => fmt_pct(num(c, "threshold")),
                            };
                            let state = if c.get("violated").and_then(Value::as_bool).unwrap_or(false) {
                                "违反"
                            } else {
                                "通过"
                            };
                            format!("{id}: {unit} vs 上限 {threshold}（{state}）")
                        })
                        .collect()
                })
                .unwrap_or_default();
            if !checks.is_empty() {
                out.push_str(&format!("  判据：{}\n", checks.join(" · ")));
            }
        }
    }

    let why_not: HashMap<&str, &str> = rationale
        .get("whyNot")
        .and_then(Value::as_array)
        .map(|ws| {
            ws.iter()
                .filter_map(|w| {
                    Some((
                        w.get("tactic").and_then(Value::as_str)?,
                        w.get("detail").and_then(Value::as_str).unwrap_or(""),
                    ))
                })
                .collect()
        })
        .unwrap_or_default();

    if !results.is_empty() {
        out.push_str("各战术：\n");
        for r in &results {
            let id = r.get("tactic").and_then(Value::as_str).unwrap_or("?");
            let suffix = why_not
                .get(id)
                .map(|d| format!(" · 落选：{d}"))
                .unwrap_or_default();
            out.push_str(&format!(
                "- {id:<12} 成功率 {} · p90 {} · {} · {}{}\n",
                fmt_pct(num(r, "successRate")),
                fmt_ms(num(r, "p90Ms")),
                fmt_tokens(num(r, "p90Tokens")),
                fmt_pct(num(r, "coordinationShare")),
                suffix,
            ));
        }
    }

    if let Some(items) = v.get("premortem").and_then(Value::as_array) {
        if !items.is_empty() {
            out.push_str("pre-mortem（假设这次已经失败，最可能的死法）：\n");
            for it in items {
                let severity = match it.get("severity").and_then(Value::as_str).unwrap_or("") {
                    "high" => "高",
                    "medium" => "中",
                    "low" => "低",
                    other => other,
                };
                out.push_str(&format!(
                    "- [{}] {} 频率 {}：{} 缓解：{}\n",
                    severity,
                    it.get("id").and_then(Value::as_str).unwrap_or("?"),
                    fmt_pct(num(it, "frequency")),
                    it.get("evidence").and_then(Value::as_str).unwrap_or(""),
                    it.get("mitigation").and_then(Value::as_str).unwrap_or(""),
                ));
            }
        }
    }

    let unknowns: Vec<&str> = v
        .get("unknowns")
        .and_then(Value::as_array)
        .map(|us| {
            us.iter()
                .filter_map(|u| u.get("field").and_then(Value::as_str))
                .collect()
        })
        .unwrap_or_default();
    if unknowns.is_empty() {
        out.push_str("未知先验：无（全部先验都有实测样本）\n");
    } else {
        out.push_str(&format!(
            "未知先验（n=0 的声明值，不是实测）：{}\n",
            unknowns.join("、")
        ));
    }
    out.push_str(
        "这是确定性蒙特卡洛推演，不是实测预测；换先验就换结论。\
         要对真实运行取证请用 action=replay（run_id 从 harness.status 取）。\n",
    );
    out
}

fn render_replay(v: &Value) -> String {
    let mut out = String::new();
    let source = v
        .get("scenarioSource")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    if v.get("insufficientEvidence").and_then(Value::as_bool).unwrap_or(false) {
        out.push_str(&format!(
            "反事实回放：这次运行没有可折算的条目（场景来源 {source}），未做任何模拟。\n"
        ));
        return out;
    }
    let observed = &v["observed"];
    out.push_str(&format!(
        "反事实回放：run={} · 实测 {} {} · {} tok · 场景来源 {source}\n",
        observed.get("runId").and_then(Value::as_str).unwrap_or("?"),
        observed.get("status").and_then(Value::as_str).unwrap_or("?"),
        fmt_ms(num(observed, "elapsedMs")),
        fmt_tokens(num(observed, "tokens")),
    ));
    if let Some(deltas) = v.get("deltas").and_then(Value::as_array) {
        if !deltas.is_empty() {
            out.push_str("各战术 vs 实测（负值=更好）：\n");
            for d in deltas {
                out.push_str(&format!(
                    "- {:<12} 成功率 {} · Δ时间 {} · Δtokens {} · Δ成功率 {}\n",
                    d.get("tactic").and_then(Value::as_str).unwrap_or("?"),
                    fmt_pct(num(d, "successRate")),
                    fmt_pct(num(d, "timeDeltaPct")),
                    fmt_pct(num(d, "tokenDeltaPct")),
                    fmt_pct(num(d, "successDelta")),
                ));
            }
        }
    }
    let rationale = &v["rationale"];
    if let Some(id) = rationale.get("recommended").and_then(Value::as_str) {
        out.push_str(&format!("裁决：推荐 {id}\n"));
    } else if rationale.get("noViableTactic").and_then(Value::as_bool).unwrap_or(false) {
        out.push_str("裁决：没有任何战术满足判据\n");
    }
    if let Some(why) = v.get("whyChain").and_then(Value::as_array) {
        if !why.is_empty() {
            out.push_str("5-Why：\n");
            for link in why {
                out.push_str(&format!(
                    "- 为什么？{} → {} [{}]\n",
                    link.get("question").and_then(Value::as_str).unwrap_or(""),
                    link.get("answer").and_then(Value::as_str).unwrap_or(""),
                    link.get("kind").and_then(Value::as_str).unwrap_or("?"),
                ));
            }
        }
    }
    if let Some(gaps) = v.get("gaps").and_then(Value::as_array) {
        if !gaps.is_empty() {
            let fields: Vec<String> = gaps
                .iter()
                .map(|g| {
                    format!(
                        "{}（{}）",
                        g.get("field").and_then(Value::as_str).unwrap_or("?"),
                        g.get("reason").and_then(Value::as_str).unwrap_or("")
                    )
                })
                .collect();
            out.push_str(&format!("台账没记录、因此未采信的字段：{}\n", fields.join("；")));
        }
    }
    out.push_str("回放用实测运行做对照，但依赖形状等未入账字段是显式假设，见上。\n");
    out
}

async fn run_tactics_tool(args: HashMap<String, String>) -> Result<ToolResult> {
    use crate::tools::builtin::{err, ok};
    let action = arg_str(&args, "action").to_ascii_lowercase();
    if action.is_empty() {
        return err(
            format!("tactics 需要 action（{}）", TACTICS_ACTIONS.join("|")),
            "INVALID_PARAMS",
        );
    }
    match action.as_str() {
        "catalog" => {
            let v = crate::gateway::tactics_rpc::tactics_catalog();
            ok(render_catalog(&v))
        }
        "plan" => {
            let raw = arg_str(&args, "scenario");
            if raw.is_empty() {
                return err(
                    "tactics plan 需要 scenario（{\"tasks\":[{\"id\":\"a\",\"workK\":40}]}）；\
                     先确认任务 DAG 再推演"
                        .to_string(),
                    "INVALID_PARAMS",
                );
            }
            let mut params = match serde_json::from_str::<Value>(&raw) {
                Ok(Value::Object(m)) => Value::Object(m),
                Ok(_) => {
                    return err("scenario 必须是 JSON 对象".to_string(), "INVALID_PARAMS")
                }
                Err(e) => {
                    return err(
                        format!("scenario 不是合法 JSON：{e}"),
                        "INVALID_PARAMS",
                    )
                }
            };
            if let Some(n) = arg_u64(&args, "trials") {
                params["trials"] = json!(n);
            }
            if let Some(n) = arg_u64(&args, "seed") {
                params["seed"] = json!(n);
            }
            if let Some(x) = arg_f64(&args, "max_fail_probability") {
                params["maxFailProbability"] = json!(x);
            }
            let v = crate::gateway::tactics_rpc::tactics_simulate(&params);
            if v.get("ok").and_then(Value::as_bool) == Some(true) {
                ok(render_plan(&v))
            } else {
                Ok(tool_failure(&v, "战术推演"))
            }
        }
        "replay" => {
            let run_id = arg_str(&args, "run_id");
            if run_id.is_empty() {
                return err(
                    "tactics replay 需要 run_id（先用 harness.status 查最近一次运行）".to_string(),
                    "INVALID_PARAMS",
                );
            }
            let mut params = json!({ "runId": run_id });
            if let Some(n) = arg_u64(&args, "trials") {
                params["trials"] = json!(n);
            }
            if let Some(n) = arg_u64(&args, "seed") {
                params["seed"] = json!(n);
            }
            if let Some(x) = arg_f64(&args, "max_fail_probability") {
                params["maxFailProbability"] = json!(x);
            }
            let v = crate::gateway::tactics_rpc::tactics_replay(&params);
            if v.get("ok").and_then(Value::as_bool) == Some(true) {
                ok(render_replay(&v))
            } else {
                Ok(tool_failure(&v, "反事实回放"))
            }
        }
        other => err(
            format!(
                "tactics 不支持 action={other}；支持：{}",
                TACTICS_ACTIONS.join("|")
            ),
            "UNSUPPORTED_ACTION",
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect()
    }

    /// 工具面与界面面同源的自证：动作必须命中真实存在的 `tactics.*` 处理函数，
    /// 且这些方法名都在网关派发表里（不得私开第二套推演语义）。
    #[test]
    fn every_action_maps_to_an_existing_rpc_handler() {
        let dispatch = include_str!("../gateway/methods.rs");
        for method in ["tactics.catalog", "tactics.simulate", "tactics.replay"] {
            assert!(
                dispatch.contains(&format!("\"{method}\"")),
                "网关派发表里没有 {method}"
            );
        }
    }

    #[tokio::test]
    async fn plan_returns_a_verdict_with_honest_priors() {
        let tool = tactics_tool();
        let scenario = json!({
            "tasks": [
                {"id": "plan", "workK": 20},
                {"id": "impl-a", "workK": 40, "dependsOn": ["plan"]},
                {"id": "impl-b", "workK": 40, "dependsOn": ["plan"]},
                {"id": "verify", "workK": 30, "dependsOn": ["impl-a", "impl-b"]}
            ],
            "trials": 200,
            "seed": 7
        })
        .to_string();
        let out = tool
            .execute(&args(&[("action", "plan"), ("scenario", scenario.as_str())]))
            .await
            .unwrap();
        match out {
            ToolResult::Success { data, .. } => {
                assert!(data.contains("推荐："), "{data}");
                assert!(data.contains("各战术："), "{data}");
                assert!(data.contains("pre-mortem"), "{data}");
                assert!(
                    data.contains("未知先验（n=0 的声明值，不是实测）"),
                    "缺省先验没有样本，必须如实标注：{data}"
                );
                assert!(
                    data.contains("不是实测预测"),
                    "推演结论必须带诚实边界：{data}"
                );
            }
            other => panic!("plan 应以成功返回，实际 {other:?}"),
        }
    }

    #[tokio::test]
    async fn bad_scenarios_fail_with_stable_codes() {
        let tool = tactics_tool();
        let out = tool
            .execute(&args(&[("action", "plan")]))
            .await
            .unwrap();
        match out {
            ToolResult::Error { code, .. } => assert_eq!(code, "INVALID_PARAMS"),
            other => panic!("缺 scenario 应报 INVALID_PARAMS，实际 {other:?}"),
        }
        let cyclic = json!({
            "tasks": [
                {"id": "a", "dependsOn": ["b"]},
                {"id": "b", "dependsOn": ["a"]}
            ]
        })
        .to_string();
        let out = tool
            .execute(&args(&[("action", "plan"), ("scenario", cyclic.as_str())]))
            .await
            .unwrap();
        match out {
            ToolResult::Error { code, message } => {
                assert_eq!(code, "INVALID_REQUEST");
                assert!(message.contains('a'), "成环报错要点名节点：{message}");
            }
            other => panic!("成环应报 INVALID_REQUEST，实际 {other:?}"),
        }
    }

    #[tokio::test]
    async fn catalog_exposes_the_gate_contrast() {
        let tool = tactics_tool();
        let out = tool.execute(&args(&[("action", "catalog")])).await.unwrap();
        match out {
            ToolResult::Success { data, .. } => {
                assert!(data.contains("无闸门"), "{data}");
                assert!(data.contains("闸门 检出"), "{data}");
                assert!(
                    data.contains("n=0"),
                    "缺省先验必须带样本数，避免读成实测：{data}"
                );
            }
            other => panic!("catalog 应以成功返回，实际 {other:?}"),
        }
    }

    /// 注册守卫：提示里能读到名字、调用却「工具不存在」，是本仓库踩过的坑
    /// （`terminal` 当年就是这样：目录里写了、注册表里没有）。
    #[test]
    fn tactics_is_registered_in_the_builtin_registry() {
        let mut registry = crate::mcp::registry::ToolRegistry::new();
        crate::tools::builtin::register_builtin_tools(
            &mut registry,
            std::path::Path::new("."),
            None,
            None,
            None,
        )
        .unwrap();
        let names = registry.list_tools();
        assert!(
            names.iter().any(|n| n == "tactics"),
            "内置注册表缺少 tactics；当前 {} 个：{names:?}",
            names.len()
        );
    }

    #[tokio::test]
    async fn unknown_action_fails_loudly() {
        let tool = tactics_tool();
        let out = tool
            .execute(&args(&[("action", "teleport")]))
            .await
            .unwrap();
        match out {
            ToolResult::Error { code, message } => {
                assert_eq!(code, "UNSUPPORTED_ACTION");
                assert!(message.contains("teleport"), "{message}");
            }
            other => panic!("未知动作应报错，实际 {other:?}"),
        }
    }
}
