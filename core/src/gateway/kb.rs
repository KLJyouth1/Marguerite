//! 知识图谱 RPC：全量图谱 / 相关节点 / 项目级图谱。
//!
//! 为前端「知识图谱」与「项目图谱」面板提供只读视图。
//! 图谱在返回前会自动从经验台账增量索引进图（幂等，按 doc_id 去重）。

use serde_json::{json, Value};

use crate::gateway::_helpers::param_str;
use crate::memory::ExperienceLedger;
use crate::persistence::config::Config;

/// `kb.graph.view`：返回全量图谱（含节点度、边、统计）。
pub fn kb_graph_view() -> Value {
    // 关系图谱自动化：从经验台账（历史记忆）自动抽取实体-关系索引进图，
    // 使图谱反映历史记录/上下文/会话沉淀，而非仅手动灌入。
    auto_index_experience_graph();

    let graph = match crate::kb::graph::global_graph().lock() {
        Ok(g) => g,
        Err(_) => return json!({ "ok": false, "error": "图谱锁获取失败" }),
    };
    let mut degree: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for e in graph.edges() {
        *degree.entry(e.from.clone()).or_insert(0) += 1;
        *degree.entry(e.to.clone()).or_insert(0) += 1;
    }
    let nodes: Vec<Value> = graph
        .entities()
        .into_iter()
        .map(|id| json!({ "id": id, "degree": degree.get(id).copied().unwrap_or(0) }))
        .collect();
    let edges: Vec<Value> = graph
        .edges()
        .iter()
        .map(|e| json!({ "from": e.from, "to": e.to, "kind": e.kind.label(), "weight": e.weight }))
        .collect();
    json!({ "nodes": nodes, "edges": edges, "entity_count": graph.entity_count(), "edge_count": graph.edge_count() })
}

/// 把经验台账（历史记忆）的最近经验自动索引进全局图谱（幂等，按 doc_id 去重）。
pub fn auto_index_experience_graph() {
    let db_path = Config::data_dir().join("marguerite_experience.db");
    if !db_path.exists() {
        return;
    }
    let mut ledger = ExperienceLedger::new(&db_path);
    if ledger.init().is_err() {
        return;
    }
    let exps = match ledger.list_recent(100) {
        Ok(v) => v,
        Err(_) => return,
    };
    if let Ok(mut graph) = crate::kb::graph::global_graph().lock() {
        for e in &exps {
            let doc_id = format!("exp:{}", e.id);
            if graph.entities_in_document(&doc_id).is_empty() {
                let text = format!("{} {}", e.title, e.body);
                graph.index_document(&doc_id, &text);
            }
        }
    }
}

/// `kb.graph.related`：查询某实体的相关节点。
pub fn kb_graph_related(params: &Value) -> Value {
    let entity = param_str(params, "entity", "entity", "");
    let graph = match crate::kb::graph::global_graph().lock() {
        Ok(g) => g,
        Err(_) => return json!({ "ok": false, "error": "图谱锁获取失败" }),
    };
    json!({ "entity": entity, "related": graph.neighbors(entity) })
}

/// `graph.project.list`：列出所有项目级图谱。
pub fn graph_project_list() -> Value {
    let items: Vec<Value> = crate::kb::project_graph::list()
        .into_iter()
        .map(|(key, project, nodes, edges)| json!({ "key": key, "project": project, "nodes": nodes, "edges": edges }))
        .collect();
    json!({ "items": items, "total": items.len() })
}

/// `graph.project.view`：查看某项目的图谱。
pub fn graph_project_view(params: &Value) -> Value {
    let project = param_str(params, "project", "project", "");
    if project.is_empty() {
        return json!({ "ok": false, "error": "graph.project.view 需要 project（项目文件夹绝对路径）" });
    }
    match crate::kb::project_graph::load(project) {
        Ok(g) => serde_json::to_value(g).unwrap_or(json!({ "ok": false, "error": "序列化失败" })),
        Err(e) => json!({ "ok": false, "error": e }),
    }
}

/// `graph.project.save`：保存某项目的图谱（前端手工编辑后回写）。
pub fn graph_project_save(params: &Value) -> Value {
    let project = param_str(params, "project", "project", "").to_string();
    if project.is_empty() {
        return json!({ "ok": false, "error": "graph.project.save 需要 project" });
    }
    let mut g = crate::kb::project_graph::ProjectGraph {
        project: project.clone(),
        ..Default::default()
    };
    if let Some(arr) = params.get("nodes").and_then(Value::as_array) {
        for n in arr {
            let id = n.get("id").and_then(Value::as_str).unwrap_or("");
            let kind = n.get("kind").and_then(Value::as_str).unwrap_or("");
            if id.is_empty() || kind.is_empty() {
                continue;
            }
            g.nodes.push(crate::kb::project_graph::GraphNode {
                id: id.to_string(),
                kind: kind.to_string(),
                label: n.get("label").and_then(Value::as_str).map(String::from),
            });
        }
    }
    if let Some(arr) = params.get("edges").and_then(Value::as_array) {
        for e in arr {
            let from = e.get("from").and_then(Value::as_str).unwrap_or("");
            let to = e.get("to").and_then(Value::as_str).unwrap_or("");
            let kind = e.get("kind").and_then(Value::as_str).unwrap_or("");
            if from.is_empty() || to.is_empty() {
                continue;
            }
            g.edges.push(crate::kb::project_graph::GraphEdge {
                from: from.to_string(),
                to: to.to_string(),
                kind: kind.to_string(),
            });
        }
    }
    g.summary = format!(
        "手工保存：{} 节点 / {} 边（一项目一图）",
        g.nodes.len(),
        g.edges.len()
    );
    match crate::kb::project_graph::save(&g) {
        Ok(()) => json!({ "ok": true, "nodes": g.nodes.len(), "edges": g.edges.len() }),
        Err(e) => json!({ "ok": false, "error": e }),
    }
}

/// `graph.project.delete`：删除某项目的图谱记录。
pub fn graph_project_delete(params: &Value) -> Value {
    let project = param_str(params, "project", "project", "");
    if project.is_empty() {
        return json!({ "ok": false, "error": "缺少 project" });
    }
    match crate::kb::project_graph::delete(project) {
        Ok(()) => json!({ "ok": true }),
        Err(e) => json!({ "ok": false, "error": e }),
    }
}

/// `graph.project.analyze`：对项目目录做图分析并自动建图。
pub fn graph_project_analyze(params: &Value) -> Value {
    let project = param_str(params, "project", "project", "");
    if project.is_empty() {
        return json!({ "ok": false, "error": "缺少 project" });
    }
    let max_files =
        crate::gateway::_helpers::param_i64(params, "maxFiles", "max_files", 200) as usize;
    match crate::kb::project_graph::analyze(project, max_files) {
        Ok(g) => serde_json::to_value(g).unwrap_or(json!({ "ok": false, "error": "序列化失败" })),
        Err(e) => json!({ "ok": false, "error": e }),
    }
}
