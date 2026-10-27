//! 技能管理方法族（skills.*）
//!
//! 负责技能的列出、获取、搜索、重载、卸载、创建与远程安装。
//! 统一委托 [`SkillLoader`] 加载 YAML 定义，保持 CLI/TUI/网关一致。

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::OnceLock;

use serde_json::{json, Value};

use crate::gateway::methods::GatewayContext;
use crate::gateway::protocol::RpcError;
use crate::skills::loader::SkillLoader;
use crate::skills::plugin_bridge::{self, BridgeReport};

/// 技能目录解析：统一委托 [`crate::tools::skill_tools::resolve_skills_dir`]，
/// 保持 CLI/TUI/网关三处一致；修复「打包后双击 exe 技能页为空」：cwd 不再决定成败。
fn skills_dir(ctx: &GatewayContext) -> PathBuf {
    crate::tools::skill_tools::resolve_skills_dir(&ctx.config.skills.skills_dir)
}

/// 把内核 Skill 映射为 WebUI `types/skills.ts` 的 Skill 视图
fn skill_view(s: &crate::skills::registry::Skill) -> Value {
    skill_view_with_origin(s, "directory", None, false)
}

/// 带来源的视图：`origin = "plugin"` 表示由插件清单声明（可能只是引用）。
///
/// `include_content = true` 仅用于 `skills.get`（详情/编辑器）；列表绝不携带完整
/// 正文——技能目录有数百条，逐条 4k 正文会把列表响应撑爆。
fn skill_view_with_origin(
    s: &crate::skills::registry::Skill,
    origin: &str,
    plugin: Option<&str>,
    include_content: bool,
) -> Value {
    let file_path = match &s.source {
        crate::skills::registry::SkillSource::Local { path } => {
            Some(path.display().to_string())
        }
        _ => None,
    };
    let kind = file_path
        .as_deref()
        .map(|p| if p.ends_with("SKILL.md") { "skill_md" } else { "yaml" })
        .unwrap_or("plugin_ref");
    let mut view = json!({
        "name": s.name,
        "description": s.description,
        "description_zh": s.description,
        "version": s.version,
        "author": s.author,
        "status": "ready",
        "layer": "bundled",
        "kind": kind,
        // 来源归因：目录技能 vs 插件声明。插件引用型条目 prompt 为空，
        // 前端据此显示「引用」而不是假装可直接执行。
        "origin": origin,
        "plugin": plugin,
        "executable": !s.prompt_template.is_empty(),
        "triggers": s.intent,
        "tools": s.tools,
        "dependencies": s.dependencies,
        "rating": s.rating,
        "uses": s.uses,
        "file_path": file_path,
        "enabled": true,
    });
    if include_content {
        if let Some(obj) = view.as_object_mut() {
            obj.insert("content".to_string(), json!(s.prompt_template));
        }
    }
    view
}

/// 技能目录 + 插件清单声明的技能（目录优先，插件不覆盖同名真实实现）。
///
/// 返回 `(合并后的技能, 技能名 → 插件名, 桥接报告)`。
fn merged_skills(ctx: &GatewayContext) -> (Vec<crate::skills::registry::Skill>, HashMap<String, String>, BridgeReport) {
    let loader = SkillLoader::new(&skills_dir(ctx));
    // 索引扫描：列表/选择只需要元数据 + 正文摘要，完整正文留给 `use_skill` 按需加载。
    let base = match loader.scan_index() {
        Ok(scan) => scan.skills,
        Err(e) => {
            tracing::warn!(err = %e, "技能目录加载失败（插件技能仍会尝试桥接）");
            Vec::new()
        }
    };
    let mut seen: HashSet<String> = base.iter().map(|s| s.name.clone()).collect();
    let manifests = crate::gateway::market::plugin_manifest_dir();
    let (bridged, report) = plugin_bridge::collect(manifests.as_deref());
    let mut plugins: HashMap<String, String> = HashMap::new();
    let mut out = base;
    for entry in bridged {
        if !seen.insert(entry.skill.name.clone()) {
            continue;
        }
        plugins.insert(entry.skill.name.clone(), entry.plugin.clone());
        out.push(entry.skill);
    }
    (out, plugins, report)
}

pub fn skills_list(ctx: &GatewayContext) -> Value {
    let (skills, plugins, _) = merged_skills(ctx);
    let items: Vec<Value> = skills
        .iter()
        .map(|s| match plugins.get(&s.name) {
            Some(plugin) => skill_view_with_origin(s, "plugin", Some(plugin), false),
            None => skill_view(s),
        })
        .collect();
    // 停用技能不参与扫描，必须单独回传，否则 UI 无法把它们重新启用。
    let disabled: Vec<Value> = SkillLoader::new(&skills_dir(ctx))
        .disabled_skills()
        .into_iter()
        .map(|(name, path)| json!({ "name": name, "file_path": path, "enabled": false }))
        .collect();
    json!({ "skills": items, "disabledSkills": disabled })
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
        Ok(s) => skill_view_with_origin(&s, "directory", None, true),
        Err(e) => {
            // 目录里没有 → 回退到插件声明（引用型也可能被 `skill.get` 查到）
            let (skills, plugins, _) = merged_skills(ctx);
            match skills.iter().find(|s| s.name == name) {
                Some(s) => {
                    let plugin = plugins.get(name).map(String::as_str);
                    skill_view_with_origin(s, "plugin", plugin, true)
                }
                None => {
                    tracing::warn!(err = %e, "skills.get 未找到 {name}");
                    json!({ "error": format!("技能 {name} 不存在") })
                }
            }
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
    // 与 `skills.list` 同源：目录技能 + 插件声明技能，避免刷新后插件技能"消失"。
    let (list, _, _) = merged_skills(ctx);
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
    // 能力目录（系统提示里的技能索引）走进程级缓存：技能目录刚刷新过就必须失效，
    // 否则模型看到的还是旧索引（"我改了技能，智能体却不知道"）。
    crate::agent::tool_catalog::invalidate_skill_cache();
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

/// `skills.coverage`：递归扫描覆盖报告。
///
/// 此前 `SkillLoader` 只扫顶层 YAML，`skills/trae/**`、`skills/codebuddy/**` 等
/// 目录型技能对产品完全不可见。本方法如实回报「扫了多少 / 载入多少 / 跳过哪些及原因」，
/// `skipped` 最多展示 50 条，`skippedTotal` 是真实总数。
pub fn skills_coverage(ctx: &GatewayContext) -> Value {
    let dir = skills_dir(ctx);
    let loader = SkillLoader::new(&dir);
    match loader.scan_index() {
        Ok(scan) => {
            let report = &scan.report;
            let skipped: Vec<Value> = report
                .skipped
                .iter()
                .take(50)
                .map(|s| json!({ "path": s.path, "reason": s.reason }))
                .collect();
            // 插件技能桥接报告：目录技能之外的另一条通道，必须单独可见，
            // 否则"插件声明了技能但页面没有"依然无法诊断。
            let manifests = crate::gateway::market::plugin_manifest_dir();
            let (bridged, bridge) = plugin_bridge::collect(manifests.as_deref());
            let merged_count = scan.skills.len() + bridged.len();
            json!({
                "ok": true,
                "root": dir.display().to_string(),
                "directories": report.directories,
                "filesScanned": report.files_scanned,
                "loaded": scan.skills.len(),
                "yaml": report.yaml,
                "skillMd": report.skill_md,
                "skipped": skipped,
                "skippedTotal": report.skipped_total,
                "plugins": plugin_bridge::report_json(&bridge),
                "mergedTotal": merged_count,
                "manifestsDir": manifests.map(|p| p.display().to_string()),
            })
        }
        Err(e) => json!({
            "ok": false,
            "error": format!("技能扫描失败: {e}"),
            "code": "SKILL_SCAN_FAILED",
        }),
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
    let (merged, _, _) = merged_skills(ctx);
    let results = merged
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
        .collect::<Vec<_>>();
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
    // remove_skill 覆盖两类落盘形态：顶层 YAML 文件、SKILL.md 目录；
    // 若技能处于停用状态，也会按索引找回真实路径后再删。路径越界一律拒绝。
    match loader.remove_skill(name) {
        Ok(path) => json!({
            "success": true,
            "name": name,
            "message": format!("已卸载技能 {name}"),
            "path": path.display().to_string(),
        }),
        Err(e) => json!({ "success": false, "name": name, "message": format!("卸载失败: {e}") }),
    }
}

/// `skills.setEnabled`：停用 / 启用一个技能（真实改名 + 持久索引）。
pub fn skills_set_enabled(ctx: &GatewayContext, params: &Value) -> Value {
    let name = params
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim();
    if name.is_empty() {
        return json!({ "ok": false, "message": "name 不可为空", "code": "INVALID_REQUEST" });
    }
    let Some(enabled) = params.get("enabled").and_then(Value::as_bool) else {
        return json!({ "ok": false, "message": "enabled 必须是布尔值", "code": "INVALID_REQUEST" });
    };
    let loader = SkillLoader::new(&skills_dir(ctx));
    match loader.set_enabled(name, enabled) {
        Ok(outcome) => json!({
            "ok": true,
            "name": outcome.name,
            "enabled": outcome.enabled,
            "changed": outcome.changed,
            "path": outcome.path.map(|p| p.display().to_string()),
            "appliesTo": "next_load",
        }),
        Err(e) => json!({ "ok": false, "name": name, "message": format!("{e}"), "code": "SKILL_TOGGLE_FAILED" }),
    }
}

/// `skills.update`：编辑一个 YAML 技能的描述 / 触发词 / 正文。
///
/// 只支持用户技能目录里的顶层 YAML（SKILL.md 目录型技能由第三方包管理，
/// 直接改写会破坏其包结构）；内容仍走 `install_yaml` 的 L2 恶意扫描与原子落盘。
pub fn skills_update(ctx: &GatewayContext, params: &Value) -> Value {
    let name = params
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim();
    if name.is_empty() {
        return json!({ "success": false, "message": "name 不可为空" });
    }
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
    if description.is_empty() || prompt_template.is_empty() {
        return json!({ "success": false, "message": "description / prompt_template 均不可为空" });
    }
    let safe_name = match crate::skills::loader::sanitize_skill_name(name) {
        Ok(n) => n,
        Err(e) => return json!({ "success": false, "message": format!("技能名不合法: {e}") }),
    };
    let loader = SkillLoader::new(&skills_dir(ctx));
    let path = match loader.try_skill_path(&safe_name) {
        Ok(p) => p,
        Err(e) => return json!({ "success": false, "message": format!("非法技能名: {e}") }),
    };
    if !path.exists() {
        return json!({
            "success": false,
            "message": format!("技能 {safe_name} 不是可编辑的 YAML 技能（SKILL.md 目录型技能请在外部编辑）"),
        });
    }
    let triggers: Vec<String> = params
        .get("triggers")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    let content = crate::tools::skill_tools::render_create_skill_yaml(
        &safe_name,
        description,
        prompt_template,
        &triggers,
    );
    // install_yaml 会原子覆盖同名文件，并再次执行 L2 恶意扫描。
    match loader.install_yaml(&safe_name, &content) {
        Ok(_) => json!({
            "success": true,
            "name": safe_name,
            "message": format!("技能 {safe_name} 已更新"),
            "path": path.display().to_string(),
        }),
        Err(e) => json!({ "success": false, "message": format!("保存失败: {e}") }),
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
    // 名称安全化：用户可用中文/空格展示名，落盘 stem 折成 `[A-Za-z0-9_-]`
    let safe_name = match crate::skills::loader::slugify_skill_name(name) {
        Ok(n) => n,
        Err(e) => {
            return json!({ "success": false, "message": format!("技能名不合法: {e}") })
        }
    };
    let loader = SkillLoader::new(&skills_dir(ctx));
    let path = match loader.try_skill_path(&safe_name) {
        Ok(p) => p,
        Err(e) => {
            return json!({ "success": false, "message": format!("非法技能名: {e}") })
        }
    };
    if path.exists() {
        return json!({ "success": false, "message": format!("技能 '{safe_name}' 已存在") });
    }
    let triggers: Vec<String> = params
        .get("triggers")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    // 复用 create_skill 工具的渲染器（含 YAML 转义），再走 install_yaml 的
    // L2 落盘扫描 —— 直接 `fs::write` 会绕过「安装前恶意扫描」这道闸门。
    let content = crate::tools::skill_tools::render_create_skill_yaml(
        &safe_name,
        description,
        prompt_template,
        &triggers,
    );
    match loader.install_yaml(&safe_name, &content) {
        Ok(_) => {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::EventBus;
    use crate::persistence::config::Config;
    use std::sync::Arc;
    use std::time::Instant;

    fn ctx_with_skills_dir(dir: &std::path::Path) -> GatewayContext {
        let mut config = Config::default();
        config.skills.skills_dir = dir.display().to_string();
        GatewayContext {
            config,
            token: "test-token".to_string(),
            started_at: Instant::now(),
            bus: Arc::new(EventBus::default()),
        }
    }

    /// `skills.uninstall` 的技能名直接参与路径拼装：必须拒绝越界名，
    /// 且绝不能删掉技能目录外的文件。
    #[test]
    fn uninstall_rejects_traversal_and_keeps_outside_file() {
        let base = std::env::temp_dir().join(format!(
            "marguerite-gw-uninstall-traversal-{}",
            std::process::id()
        ));
        let dir = base.join("skills");
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&dir).unwrap();
        // 技能目录外放一个「受害者」文件
        let victim = base.join("victim.yaml");
        std::fs::write(&victim, "name: victim\n").unwrap();

        let ctx = ctx_with_skills_dir(&dir);
        let v = skills_uninstall(&ctx, &json!({ "name": "../victim" }));
        assert_eq!(v["success"], false, "穿越型名字必须失败: {v}");
        assert!(victim.exists(), "技能目录外的文件绝不能被删除");

        let _ = std::fs::remove_dir_all(&base);
    }

    /// `skills.create` 必须走 L2 落盘扫描：恶意模板拒绝且不落盘。
    #[test]
    fn create_rejects_malicious_template_and_writes_nothing() {
        let base = std::env::temp_dir().join(format!(
            "marguerite-gw-create-malicious-{}",
            std::process::id()
        ));
        let dir = base.join("skills");
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&dir).unwrap();
        let ctx = ctx_with_skills_dir(&dir);

        let v = skills_create(
            &ctx,
            &json!({
                "name": "evil",
                "description": "x",
                "prompt_template": "result = eval(user_input)\nreturn result",
            }),
        );
        assert_eq!(v["success"], false, "含 eval 的技能必须被拒绝: {v}");
        assert!(!dir.join("evil.yaml").exists(), "被拦截的技能绝不落盘");

        let _ = std::fs::remove_dir_all(&base);
    }

    /// 描述里的换行/冒号不能改变 YAML 结构（字段注入）。
    #[test]
    fn create_escapes_description_into_single_scalar() {
        let base = std::env::temp_dir().join(format!(
            "marguerite-gw-create-escape-{}",
            std::process::id()
        ));
        let dir = base.join("skills");
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&dir).unwrap();
        let ctx = ctx_with_skills_dir(&dir);

        let v = skills_create(
            &ctx,
            &json!({
                "name": "My Skill",
                "description": "无害描述\nversion: \"9.9.9\"\nrank: 999",
                "prompt_template": "do the thing",
                "triggers": ["a: b"],
            }),
        );
        assert_eq!(v["success"], true, "创建应成功: {v}");
        let content = std::fs::read_to_string(dir.join("My-Skill.yaml")).unwrap();
        let parsed: serde_yaml::Value = serde_yaml::from_str(&content).unwrap();
        assert_eq!(parsed["version"].as_str().unwrap(), "1.0.0", "不能注入 version");
        assert_eq!(parsed["intent"][0].as_str().unwrap(), "a: b");
        assert!(parsed["description"]
            .as_str()
            .unwrap()
            .contains("无害描述"));

        let _ = std::fs::remove_dir_all(&base);
    }
}
