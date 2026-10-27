//! 记忆层 RPC（ExperienceLedger 直连）：语义检索 / 写入 / 校验 / 删除 / 管理面列表。
//!
//! 数据源：`marguerite_experience.db`（SQLite，由 `ExperienceLedger` 托管）。
//! 所有方法在台账不可用时静默降级返回空结果，不阻塞前端。
//!
//! 写入治理：`memory.save` 记录 `source`（user/agent/tool/import）、`actor`、
//! `confidence`、`status`（draft/active）；`memory.validate` / `memory.delete`
//! 提供可验证、可删除的管理闭环（未知 id 如实报错，不谎称成功）。

use serde_json::{json, Value};

use crate::memory::{Experience, ExperienceKind, ExperienceLedger, ExperienceMeta};
use crate::persistence::config::Config;
use crate::rbac::policy::Policy;
use crate::rbac::roles::Role;

fn open_ledger() -> Option<ExperienceLedger> {
    let mut ledger = ExperienceLedger::new(&Config::data_dir().join("marguerite_experience.db"));
    match ledger.init() {
        Ok(()) => Some(ledger),
        Err(e) => {
            tracing::warn!(err = %e, "记忆台账打开失败");
            None
        }
    }
}

/// 写入口 RBAC 校验（本地 owner 恒为 Admin → 恒通过，但校验真实执行）。
fn rbac_denied(method: &str) -> Option<Value> {
    match Policy.check(method, Role::Admin) {
        Ok(()) => None,
        Err(forbidden) => Some(json!({
            "ok": false,
            "code": forbidden.code,
            "method": method,
            "message": forbidden.message,
        })),
    }
}

fn experience_view(e: &Experience) -> Value {
    json!({
        "id": e.id,
        "kind": e.kind.as_str(),
        "title": e.title,
        "body": e.body,
        "tags": e.tags,
        "signature": e.signature,
        "validated": e.validated,
        "created_at": e.created_at,
        "source": e.source,
        "actor": e.actor,
        "confidence": e.confidence,
        "status": e.status,
        "session_key": e.session_key,
    })
}

/// 解析 id：接受 JSON 数字或数字字符串（前端 from 表单常传字符串）。
fn id_of(value: &Value) -> Option<i64> {
    value
        .as_i64()
        .or_else(|| value.as_str().and_then(|s| s.trim().parse::<i64>().ok()))
}

/// `memory.search`：按 query 做经验台账检索（经验 = 坑 / 胜利 / 模式）。
pub fn memory_search(params: &Value) -> Value {
    let query = params.get("query").and_then(Value::as_str).unwrap_or("");
    if query.is_empty() {
        return json!({ "items": [] });
    }
    let limit = params.get("limit").and_then(Value::as_u64).unwrap_or(20) as usize;
    let items = match open_ledger() {
        Some(l) => match l.search(query, limit) {
            Ok(list) => list.iter().map(experience_view).collect::<Vec<_>>(),
            Err(e) => {
                tracing::warn!(err = %e, "memory.search 失败");
                Vec::new()
            }
        },
        None => Vec::new(),
    };
    json!({ "items": items })
}

/// `memory.save`：写入一条经验（支持自动 signature 防重复）。
///
/// 可选治理入参：`source`（user|agent|tool|import，默认 agent）、`actor`、
/// `sessionKey`、`confidence`（0..1，默认 0.6）、`status`（draft|active，默认 active）。
/// 显式写入**不受** `memory.auto_capture_enabled` 限制（显式意图优先）。
pub fn memory_save(params: &Value) -> Value {
    if let Some(denied) = rbac_denied("memory.save") {
        return denied;
    }
    let kind = ExperienceKind::from_kind_str(
        params
            .get("kind")
            .and_then(Value::as_str)
            .unwrap_or("pitfall"),
    );
    let title = params
        .get("title")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let body = params
        .get("body")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if title.is_empty() || body.is_empty() {
        return json!({
            "saved": false,
            "ok": false,
            "code": "MEMORY_INVALID_PARAMS",
            "message": "title 与 body 不可为空",
        });
    }
    let tags: Vec<String> = params
        .get("tags")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    let signature = params
        .get("signature")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| {
            use std::collections::hash_map::DefaultHasher;
            use std::hash::{Hash, Hasher};
            let mut h = DefaultHasher::new();
            format!("{title}{body}").hash(&mut h);
            format!("auto:{:x}", h.finish())
        });
    let meta = ExperienceMeta {
        source: params
            .get("source")
            .and_then(Value::as_str)
            .unwrap_or("agent")
            .to_string(),
        actor: params
            .get("actor")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        confidence: params
            .get("confidence")
            .and_then(Value::as_f64)
            .unwrap_or(0.6),
        status: params
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or("active")
            .to_string(),
        session_key: params
            .get("sessionKey")
            .or_else(|| params.get("session_key"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
    }
    .normalized();
    match open_ledger() {
        Some(mut l) => match l.record_meta(kind, title, body, &tags, &signature, &meta) {
            Ok(id) => json!({
                "saved": true,
                "ok": true,
                "id": id,
                "kind": kind.as_str(),
                "source": meta.source,
                "actor": meta.actor,
                "confidence": meta.confidence,
                "status": meta.status,
                "session_key": meta.session_key,
            }),
            Err(e) => json!({
                "saved": false,
                "ok": false,
                "code": "MEMORY_WRITE_FAILED",
                "message": format!("保存失败: {e}"),
            }),
        },
        None => json!({
            "saved": false,
            "ok": false,
            "code": "MEMORY_UNAVAILABLE",
            "message": "记忆台账不可用",
        }),
    }
}

/// `memory.list`：列出经验台账（按 kind 过滤，默认最近 50 条）。
pub fn memory_list(params: &Value) -> Value {
    let limit = params.get("limit").and_then(Value::as_u64).unwrap_or(50) as usize;
    let kind = params.get("kind").and_then(Value::as_str);
    let items = match open_ledger() {
        Some(l) => {
            let res = if let Some(k) = kind {
                l.list_by_kind(ExperienceKind::from_kind_str(k), limit)
            } else {
                l.list_recent(limit)
            };
            match res {
                Ok(list) => list.iter().map(experience_view).collect::<Vec<_>>(),
                Err(e) => {
                    tracing::warn!(err = %e, "memory.list 失败");
                    Vec::new()
                }
            }
        }
        None => Vec::new(),
    };
    json!({ "items": items })
}

/// `memory.entries`：管理面只读分页（`kind?` / `status?` / `limit=50` / `offset=0`）。
pub fn memory_entries(params: &Value) -> Value {
    let limit = params.get("limit").and_then(Value::as_u64).unwrap_or(50) as usize;
    let offset = params.get("offset").and_then(Value::as_u64).unwrap_or(0) as usize;
    let kind = params
        .get("kind")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty());
    let status = params
        .get("status")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty());
    match open_ledger() {
        Some(l) => match l.list_entries(kind, status, limit, offset) {
            Ok(list) => {
                let items: Vec<Value> = list.iter().map(experience_view).collect();
                json!({
                    "ok": true,
                    "items": items,
                    "total": items.len(),
                    "limit": limit,
                    "offset": offset,
                })
            }
            Err(e) => json!({
                "ok": false,
                "code": "MEMORY_UNAVAILABLE",
                "message": format!("读取记忆条目失败: {e}"),
                "items": [],
            }),
        },
        None => json!({
            "ok": false,
            "code": "MEMORY_UNAVAILABLE",
            "message": "记忆台账不可用",
            "items": [],
        }),
    }
}

/// `memory.validate`：设置条目验证状态（`{id, validated}`）。
///
/// 未知 id → `{ok:false, code:"MEMORY_NOT_FOUND"}`；成功返回更新后的条目。
pub fn memory_validate(params: &Value) -> Value {
    if let Some(denied) = rbac_denied("memory.validate") {
        return denied;
    }
    let Some(id) = params.get("id").and_then(id_of) else {
        return json!({
            "ok": false,
            "code": "MEMORY_INVALID_PARAMS",
            "message": "memory.validate 需要数字 id",
        });
    };
    let validated = params
        .get("validated")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    match open_ledger() {
        Some(mut l) => match l.set_validated(id, validated) {
            Ok(true) => match l.get(id) {
                Ok(Some(item)) => json!({ "ok": true, "item": experience_view(&item) }),
                Ok(None) => json!({
                    "ok": false,
                    "code": "MEMORY_NOT_FOUND",
                    "message": format!("记忆条目 {id} 不存在"),
                }),
                Err(e) => json!({
                    "ok": false,
                    "code": "MEMORY_UNAVAILABLE",
                    "message": format!("回读条目失败: {e}"),
                }),
            },
            Ok(false) => json!({
                "ok": false,
                "code": "MEMORY_NOT_FOUND",
                "message": format!("记忆条目 {id} 不存在"),
            }),
            Err(e) => json!({
                "ok": false,
                "code": "MEMORY_UNAVAILABLE",
                "message": format!("更新验证状态失败: {e}"),
            }),
        },
        None => json!({
            "ok": false,
            "code": "MEMORY_UNAVAILABLE",
            "message": "记忆台账不可用",
        }),
    }
}

/// `memory.delete`：软删（`{id}` 或 `{ids:[...]}`）。
///
/// 单条：→ `{ok:true, id, deletedAt}`；不存在/已删除 → `{ok:false, code:"NOT_FOUND"}`。
/// 批量（兼容旧调用）：→ `{ok:true, deleted:n, missing:[未知 id]}`。
/// 软删后 `memory.search` / `memory.list` / `memory.entries` 自动排除。
pub fn memory_delete(params: &Value) -> Value {
    if let Some(denied) = rbac_denied("memory.delete") {
        return denied;
    }
    if let Some(id) = params.get("id").and_then(id_of) {
        if params.get("ids").is_none() {
            return match open_ledger() {
                Some(mut l) => match l.soft_delete(id) {
                    Ok(Some(deleted_at)) => json!({
                        "ok": true,
                        "id": id,
                        "deletedAt": deleted_at,
                    }),
                    Ok(None) => json!({
                        "ok": false,
                        "code": "NOT_FOUND",
                        "id": id,
                        "message": format!("记忆条目 {id} 不存在或已删除"),
                    }),
                    Err(e) => json!({
                        "ok": false,
                        "code": "MEMORY_UNAVAILABLE",
                        "message": format!("删除记忆条目失败: {e}"),
                    }),
                },
                None => json!({
                    "ok": false,
                    "code": "MEMORY_UNAVAILABLE",
                    "message": "记忆台账不可用",
                }),
            };
        }
    }
    let mut ids: Vec<i64> = Vec::new();
    if let Some(id) = params.get("id").and_then(id_of) {
        ids.push(id);
    }
    if let Some(arr) = params.get("ids").and_then(Value::as_array) {
        for v in arr {
            if let Some(id) = id_of(v) {
                ids.push(id);
            }
        }
    }
    if ids.is_empty() {
        return json!({
            "ok": false,
            "code": "MEMORY_INVALID_PARAMS",
            "message": "memory.delete 需要 id 或非空 ids 数组",
        });
    }
    ids.sort_unstable();
    ids.dedup();
    match open_ledger() {
        Some(mut l) => match l.delete_ids(&ids) {
            Ok((deleted, missing)) => json!({
                "ok": true,
                "deleted": deleted,
                "missing": missing,
                "deletedAt": chrono::Utc::now().to_rfc3339(),
            }),
            Err(e) => json!({
                "ok": false,
                "code": "MEMORY_UNAVAILABLE",
                "message": format!("删除记忆条目失败: {e}"),
            }),
        },
        None => json!({
            "ok": false,
            "code": "MEMORY_UNAVAILABLE",
            "message": "记忆台账不可用",
        }),
    }
}

/// `memory.stats`：记忆管控面板概览（各层真实计数 + 待验证 + 已软删）。
///
/// 计数全部来自真实数据源（SQLite / 各层存储），读不到时如实给 `null`/0，
/// 不编造「看起来健康」的数字。
pub fn memory_stats(_params: &Value) -> Value {
    let auto_capture_enabled = Config::load()
        .map(|c| c.memory.auto_capture_enabled)
        .unwrap_or(true);
    let Some(l) = open_ledger() else {
        return json!({
            "ok": false,
            "code": "MEMORY_UNAVAILABLE",
            "message": "记忆台账不可用",
            "autoCaptureEnabled": auto_capture_enabled,
        });
    };
    let total = l.count().unwrap_or(0);
    let pending = l.count_pending_validation().unwrap_or(0);
    let deleted = l.count_deleted().unwrap_or(0);
    let validated = total.saturating_sub(pending);
    let by_kind = |kind: ExperienceKind| l.count_by_kind(kind).unwrap_or(0);
    // 其余楼层：复用 layers_status 的真实计数（读失败时为 null）。
    let layer_counts = crate::memory::gateway::layers_status(&Value::Null);
    let mut layers = serde_json::Map::new();
    if let Some(items) = layer_counts.get("layers").and_then(Value::as_array) {
        for item in items {
            if let (Some(id), Some(count)) = (
                item.get("id").and_then(Value::as_str),
                item.get("count").and_then(Value::as_u64),
            ) {
                layers.insert(id.to_string(), json!(count));
            }
        }
    }
    layers.insert("experience".to_string(), json!(total));
    json!({
        "ok": true,
        "total": total,
        "validated": validated,
        "pendingValidation": pending,
        "deleted": deleted,
        "byKind": {
            "pitfall": by_kind(ExperienceKind::Pitfall),
            "decision": by_kind(ExperienceKind::Decision),
            "pattern": by_kind(ExperienceKind::Pattern),
        },
        "layers": layers,
        "autoCaptureEnabled": auto_capture_enabled,
    })
}
