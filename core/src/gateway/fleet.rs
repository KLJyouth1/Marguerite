//! Fleet 路由 / 自进化 / 插件槽位 / 知识页 方法族
//!
//! Fleet 多角色模型路由、自进化历史、插件槽位浏览、内置知识页 CRUD。

use serde_json::{json, Value};

use crate::persistence::config::{Config, ModelConfig};

/// ModelConfig → LLMConfig 转换（与 GUI Tauri 命令同源）。
fn model_config_to_llm(m: &ModelConfig) -> crate::model::llm::LLMConfig {
    use crate::model::llm::{LLMConfig, LLMProvider};
    use crate::model::thinking::{ThinkingEffort, CONTEXT_WINDOW_DEFAULT};
    let provider = match m.provider.to_lowercase().as_str() {
        "openai" => LLMProvider::OpenAI,
        "deepseek" => LLMProvider::DeepSeek,
        "anthropic" => LLMProvider::Anthropic,
        _ => LLMProvider::Custom {
            base_url: m.base_url.clone(),
        },
    };
    LLMConfig {
        provider,
        model: m.model_name.clone(),
        api_key: m.api_key.clone(),
        base_url: m.base_url.clone(),
        max_tokens: 4096,
        temperature: 0.7,
        timeout_secs: if m.timeout_secs > 0 {
            m.timeout_secs
        } else {
            120
        },
        thinking: ThinkingEffort::Off,
        context_window: CONTEXT_WINDOW_DEFAULT,
        // 舰队成员沿用主配置声明的协议：这里映射的就是 `config.model`，
        // 若在此处填默认值，用户配了 responses 但舰队仍打 chat。
        wire_api: m.wire_api,
    }
}

/// `fleet.list`：返回 Fleet 多角色模型路由配置。
pub fn fleet_list() -> Value {
    let config = match Config::load() {
        Ok(c) => c,
        Err(_) => return json!({ "roles": [], "error": "配置加载失败" }),
    };
    let llm = model_config_to_llm(&config.model);
    let fleet = crate::model::fleet::FleetConfig::with_defaults(&llm);
    let roles: Vec<Value> = fleet
        .roles
        .iter()
        .map(|(id, b)| {
            json!({
                "role_id": id,
                "label": b.label,
                "kind": format!("{:?}", b.kind),
                "provider": format!("{:?}", b.config.provider),
                "model": b.config.model,
                "reasoning": format!("{:?}", b.reasoning),
                "max_concurrency": b.max_concurrency,
                "system_prompt": b.system_prompt,
            })
        })
        .collect();
    json!({
        "roles": roles,
        "default_role": fleet.default_role,
        "failover_chain": fleet.failover_chain,
        "enabled": fleet.enabled,
    })
}

/// `fleet.route`：按任务描述推荐角色。
pub fn fleet_route(params: &Value) -> Value {
    let task = params.get("task").and_then(Value::as_str).unwrap_or("");
    if task.is_empty() {
        return json!({ "error": "task 参数不可为空" });
    }
    let config = match Config::load() {
        Ok(c) => c,
        Err(_) => return json!({ "error": "配置加载失败" }),
    };
    let llm = model_config_to_llm(&config.model);
    let fleet = crate::model::fleet::FleetConfig::with_defaults(&llm);
    let router = crate::model::fleet::FleetRouter::new(fleet);
    match router.route_by_task(task) {
        Some(d) => json!({
            "role_id": d.role_id,
            "source": format!("{:?}", d.source),
            "provider": format!("{:?}", d.binding.config.provider),
            "model": d.binding.config.model,
            "reasoning": format!("{:?}", d.binding.reasoning),
            "label": d.binding.label,
        }),
        None => json!({ "error": "无可用路由" }),
    }
}

/// `evolve.history`：自进化历史记录。
pub fn evolve_history() -> Value {
    let data_dir = Config::data_dir();
    let runner = crate::harness::evolve::EvolutionRunner::new(&data_dir, "");
    match runner.load_history() {
        Ok(history) => {
            let records: Vec<Value> = history
                .iter()
                .map(|r| {
                    json!({
                        "round": r.round,
                        "timestamp": r.timestamp,
                        "diagnosis": r.diagnosis,
                        "score_before": r.score_before,
                        "score_after": r.score_after,
                        "accepted": r.accepted,
                        "rejection_reason": r.rejection_reason,
                    })
                })
                .collect();
            json!({ "records": records, "total": records.len() })
        }
        Err(e) => json!({ "records": [], "error": e.to_string() }),
    }
}

/// `plugins.list`：已注册插件/工具槽位。
pub fn plugins_list() -> Value {
    let caps = crate::domain::capabilities::registry();
    let items: Vec<Value> = caps
        .iter()
        .filter(|c| c.maturity.is_implemented())
        .map(|c| {
            json!({
                "name": c.id.0,
                "description": c.description,
                "group": format!("{:?}", c.group),
                "tier": c.tier.label(),
                "maturity": format!("{:?}", c.maturity),
            })
        })
        .collect();
    json!({ "plugins": items, "total": items.len() })
}

/// `pages.list`：内置知识页列表。
pub fn pages_list(_params: &Value) -> Value {
    match crate::memory::PageStore::open_default() {
        Ok(store) => {
            let items: Vec<Value> = store
                .list()
                .iter()
                .map(|p| {
                    json!({
                        "id": p.id, "title": p.title, "tags": p.tags,
                        "useCount": p.use_count, "updatedAt": p.updated_at,
                    })
                })
                .collect();
            json!({ "pages": items, "total": items.len() })
        }
        Err(e) => json!({ "pages": [], "error": e.to_string() }),
    }
}

/// `pages.add`：新建知识页。
pub fn pages_add(params: &Value) -> Value {
    let title = params.get("title").and_then(Value::as_str).unwrap_or("");
    if title.is_empty() {
        return json!({ "success": false, "message": "pages.add 需要 title" });
    }
    let content = params.get("content").and_then(Value::as_str).unwrap_or("");
    let tags: Vec<&str> = params
        .get("tags")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(|v| v.as_str()).collect())
        .unwrap_or_default();
    match crate::memory::PageStore::open_default() {
        Ok(mut store) => match store.create(title, content, &tags) {
            Ok(id) => {
                json!({ "success": true, "id": id, "message": format!("已建知识页 [{id}]：{title}") })
            }
            Err(e) => json!({ "success": false, "message": e.to_string() }),
        },
        Err(e) => json!({ "success": false, "message": e.to_string() }),
    }
}

/// `pages.search`：知识页混合检索。
pub fn pages_search(params: &Value) -> Value {
    let q = params.get("q").and_then(Value::as_str).unwrap_or("");
    match crate::memory::PageStore::open_default() {
        Ok(store) => {
            let top_k = params.get("topK").and_then(Value::as_u64).unwrap_or(5) as usize;
            let items: Vec<Value> = store
                .search(q, top_k)
                .into_iter()
                .map(|(p, s)| json!({ "id": p.id, "title": p.title, "content": p.content, "score": s, "tags": p.tags }))
                .collect();
            json!({ "pages": items, "total": items.len() })
        }
        Err(e) => json!({ "pages": [], "error": e.to_string() }),
    }
}
