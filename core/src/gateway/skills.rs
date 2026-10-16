//! 技能管理方法族（skills.*）
//!
//! 负责技能的列出、获取、搜索、重载、卸载、创建与远程安装。
//! 统一委托 [`SkillLoader`] 加载 YAML 定义，保持 CLI/TUI/网关一致。

use std::path::PathBuf;
use std::sync::OnceLock;

use serde_json::{json, Value};

use crate::gateway::methods::GatewayContext;
use crate::gateway::protocol::RpcError;
use crate::skills::loader::SkillLoader;

/// 技能目录解析：统一委托 [`crate::tools::skill_tools::resolve_skills_dir`]，
/// 保持 CLI/TUI/网关三处一致；修复「打包后双击 exe 技能页为空」：cwd 不再决定成败。
fn skills_dir(ctx: &GatewayContext) -> PathBuf {
    crate::tools::skill_tools::resolve_skills_dir(&ctx.config.skills.skills_dir)
}

/// 把内核 Skill 映射为 WebUI `types/skills.ts` 的 Skill 视图
fn skill_view(s: &crate::skills::registry::Skill) -> Value {
    json!({
        "name": s.name,
        "description": s.description,
        "description_zh": s.description,
        "version": s.version,
        "author": s.author,
        "status": "ready",
        "layer": "bundled",
        "kind": "skill",
        "triggers": s.intent,
        "tools": s.tools,
        "dependencies": s.dependencies,
        "rating": s.rating,
        "uses": s.uses,
    })
}

pub fn skills_list(ctx: &GatewayContext) -> Value {
    let loader = SkillLoader::new(&skills_dir(ctx));
    let skills = match loader.load_all() {
        Ok(list) => list.iter().map(skill_view).collect::<Vec<_>>(),
        Err(e) => {
            tracing::warn!(err = %e, "skills.list 加载失败");
            Vec::new()
        }
    };
    json!({ "skills": skills })
}

pub fn skills_get(ctx: &GatewayContext, params: &Value) -> Value {
    let name = params
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if name.is_empty() {
        return json!({ "error": "name 不可为空" });
    }
    let loader = SkillLoader::new(&skills_dir(ctx));
    match loader.load(name) {
        Ok(s) => skill_view(&s),
        Err(e) => {
            tracing::warn!(err = %e, "skills.get 未找到 {name}");
            json!({ "error": format!("技能 {name} 不存在") })
        }
    }
}

/// `skills.reload`：真重扫技能目录，返回变化摘要（added/removed）与最新 generation。
///
/// WebUI 手动刷新会据此展示「新增/移除/无变化」toast，随后重新拉取 `skills.list`。
pub fn skills_reload(ctx: &GatewayContext) -> Value {
    static GENERATION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    static SNAPSHOT: OnceLock<std::sync::Mutex<Vec<String>>> = OnceLock::new();
    let gen = GENERATION.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let loader = SkillLoader::new(&skills_dir(ctx));
    match loader.load_all() {
        Ok(list) => {
            let names: Vec<String> = list.iter().map(|s| s.name.clone()).collect();
            let snapshot = SNAPSHOT.get_or_init(|| std::sync::Mutex::new(Vec::new()));
            let mut prev = match snapshot.lock() {
                Ok(g) => g,
                Err(e) => e.into_inner(),
            };
            let added: Vec<String> = names
                .iter()
                .filter(|n| !prev.contains(n))
                .cloned()
                .collect();
            let removed: Vec<String> = prev
                .iter()
                .filter(|n| !names.contains(n))
                .cloned()
                .collect();
            *prev = names;
            json!({
                "success": true,
                "changed": !added.is_empty() || !removed.is_empty(),
                "partial": false,
                "generation": gen,
                "added": added,
                "removed": removed,
                "modified": [],
                "errors": [],
            })
        }
        Err(e) => {
            tracing::warn!(err = %e, "skills.reload 重扫失败");
            json!({
                "success": false,
                "changed": false,
                "partial": false,
                "generation": gen,
                "errors": [{ "message": format!("{e}") }],
            })
        }
    }
}

pub fn skills_search(ctx: &GatewayContext, params: &Value) -> Value {
    let query = params
        .get("query")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_lowercase();
    if query.is_empty() {
        return json!({ "results": [] });
    }
    let loader = SkillLoader::new(&skills_dir(ctx));
    let results = match loader.load_all() {
        Ok(list) => list
            .iter()
            .filter(|s| {
                s.name.to_lowercase().contains(&query)
                    || s.description.to_lowercase().contains(&query)
                    || s.intent.iter().any(|i| i.to_lowercase().contains(&query))
            })
            .map(|s| {
                json!({
                    "name": s.name,
                    "description": s.description,
                    "identifier": s.name,
                    "source": "local",
                    "trust_level": "self",
                    "installed": true,
                })
            })
            .collect::<Vec<_>>(),
        Err(_) => Vec::new(),
    };
    json!({ "results": results })
}

pub fn skills_uninstall(ctx: &GatewayContext, params: &Value) -> Value {
    let name = params
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if name.is_empty() {
        return json!({ "success": false, "message": "name 不可为空" });
    }
    let loader = SkillLoader::new(&skills_dir(ctx));
    let path = loader.get_skill_path(name);
    if path.exists() {
        match std::fs::remove_file(&path) {
            Ok(()) => {
                json!({ "success": true, "name": name, "message": format!("已卸载技能 {name}") })
            }
            Err(e) => {
                json!({ "success": false, "name": name, "message": format!("卸载失败: {e}") })
            }
        }
    } else {
        json!({ "success": false, "name": name, "message": format!("技能 {name} 不存在") })
    }
}

/// 创建新技能（写入 YAML 文件到技能目录）
pub fn skills_create(ctx: &GatewayContext, params: &Value) -> Value {
    let name = params
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim();
    let description = params
        .get("description")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim();
    let prompt_template = params
        .get("prompt_template")
        .and_then(Value::as_str)
        .or_else(|| params.get("promptTemplate").and_then(Value::as_str))
        .unwrap_or_default()
        .trim();
    if name.is_empty() || description.is_empty() || prompt_template.is_empty() {
        return json!({ "success": false, "message": "name / description / prompt_template 均不可为空" });
    }
    // 名称安全化
    let safe_name: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect();
    let dir = skills_dir(ctx);
    if let Err(e) = std::fs::create_dir_all(&dir) {
        return json!({ "success": false, "message": format!("创建技能目录失败: {e}") });
    }
    let path = dir.join(format!("{safe_name}.yaml"));
    if path.exists() {
        return json!({ "success": false, "message": format!("技能 '{safe_name}' 已存在") });
    }
    let triggers = params
        .get("triggers")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str())
                .map(|s| format!("  - {s}"))
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default();
    let indented: String = prompt_template
        .lines()
        .map(|l| format!("  {l}"))
        .collect::<Vec<_>>()
        .join("\n");
    let content = format!(
        "name: {safe_name}\ndescription: {description}\nversion: \"1.0.0\"\nauthor: user\nintent:\n{triggers}\ntools: []\nprompt_template: |\n{indented}\n"
    );
    match std::fs::write(&path, content) {
        Ok(()) => {
            json!({ "success": true, "name": safe_name, "message": format!("技能 '{safe_name}' 已创建"), "path": path.display().to_string() })
        }
        Err(e) => json!({ "success": false, "message": format!("写入失败: {e}") }),
    }
}

/// 远程安装（async：真实下载 YAML 并注册到本地技能目录）
pub async fn skills_install(ctx: &GatewayContext, params: &Value) -> Result<Value, RpcError> {
    let identifier = params
        .get("identifier")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let source = params
        .get("source")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if identifier.is_empty() {
        return Ok(json!({ "success": false, "message": "identifier 不可为空" }));
    }
    let is_url = identifier.starts_with("http://") || identifier.starts_with("https://");
    if is_url && (source.is_empty() || source == "github") {
        let loader = SkillLoader::new(&skills_dir(ctx));
        return match loader.download_and_register(identifier).await {
            Ok(s) => Ok(json!({
                "success": true,
                "name": s.name,
                "message": format!("已安装技能 {}", s.name),
            })),
            Err(e) => Ok(json!({ "success": false, "message": format!("安装失败: {e}") })),
        };
    }
    Ok(json!({
        "success": false,
        "message": "Rust 网关当前仅支持 http(s) URL 安装（registry 标识符离线无法拉取）",
    }))
}
