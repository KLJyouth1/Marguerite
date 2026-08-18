//! 技能工具：让 LLM 在工具循环中列出 / 调用 / 创建技能
//!
//! - `list_skills`：枚举技能目录下所有可用技能（名称 + 描述 + 触发词）
//! - `use_skill`：加载技能 YAML，用传入变量渲染 prompt_template，返回渲染结果
//!   （LLM 拿到渲染后的专业提示后按其指引执行——技能本质是结构化专家 prompt）
//! - `create_skill`：把 name/description/prompt_template/triggers 写成新 YAML 文件

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use serde_json::json;

use crate::agent::tool_cycle::ToolResult;
use crate::mcp::registry::ToolRegistry;
use crate::security::skill_scan::{log_skill_block, SkillInstallScanner};
use crate::skills::loader::SkillLoader;

/// 解析技能目录：相对路径从 cwd 逐级上溯，与网关 `skills_dir()` 同构。
pub fn resolve_skills_dir(configured: &str) -> PathBuf {
    let p = PathBuf::from(configured);
    if p.is_absolute() {
        if p.is_dir() {
            return p;
        }
    } else if let Ok(cwd) = std::env::current_dir() {
        let direct = cwd.join(&p);
        if direct.is_dir() {
            return direct;
        }
        let mut base = cwd.as_path();
        for _ in 0..6 {
            match base.parent() {
                Some(parent) => {
                    let candidate = parent.join(&p);
                    if candidate.is_dir() {
                        return candidate;
                    }
                    base = parent;
                }
                None => break,
            }
        }
    }
    let fallback = crate::persistence::config::Config::data_dir().join("skills");
    let _ = std::fs::create_dir_all(&fallback);
    fallback
}

fn ok(data: String) -> Result<ToolResult> {
    Ok(ToolResult::Success {
        data,
        metadata: HashMap::new(),
    })
}

fn err(message: String, code: &str) -> Result<ToolResult> {
    Ok(ToolResult::Error {
        message,
        code: code.to_string(),
    })
}

/// 注册全部技能相关工具到工具注册表
pub fn register_skill_tools(registry: &mut ToolRegistry, skills_dir: PathBuf) {
    let dir = Arc::new(skills_dir);

    // list_skills
    {
        let dir = dir.clone();
        registry.register_local(crate::tools::make_local_tool(
            "list_skills",
            "列出所有可用技能（专业 prompt 模板）。返回名称、描述、触发词。无参数。",
            json!({"type":"object","properties":{}}),
            Arc::new(move |_args| {
                let dir = dir.clone();
                Box::pin(async move {
                    let loader = SkillLoader::new(dir.as_path());
                    match loader.load_all() {
                        Ok(skills) => {
                            if skills.is_empty() {
                                return ok("当前无可用技能。可用 create_skill 创建。".into());
                            }
                            let lines: Vec<String> = skills
                                .iter()
                                .map(|s| {
                                    format!(
                                        "- {} (v{}): {}\n  触发词: {}",
                                        s.name,
                                        s.version,
                                        s.description,
                                        if s.intent.is_empty() {
                                            "（无）".to_string()
                                        } else {
                                            s.intent.join(", ")
                                        }
                                    )
                                })
                                .collect();
                            ok(format!(
                                "可用技能（{}个）:\n{}",
                                skills.len(),
                                lines.join("\n")
                            ))
                        }
                        Err(e) => err(format!("加载技能列表失败: {e}"), "SKILL_LOAD_ERROR"),
                    }
                })
            }),
        ));
    }

    // use_skill
    {
        let dir = dir.clone();
        registry.register_local(crate::tools::make_local_tool(
            "use_skill",
            "调用指定技能：加载技能的专家 prompt 模板并用变量渲染，返回专业指引供你遵循执行。参数: name(必填), 以及模板中用到的变量(如 text/query/language 等，作为额外字符串参数传入)。",
            json!({"type":"object","properties":{
                "name":{"type":"string","description":"技能名称，如 commit/fix/explain/refactor"},
                "text":{"type":"string","description":"技能要处理的文本/内容（多数技能的主输入）"},
                "query":{"type":"string","description":"查询/问题（部分技能使用）"},
                "language":{"type":"string","description":"语言（可选）"}
            },"required":["name"]}),
            Arc::new(move |args| {
                let dir = dir.clone();
                Box::pin(async move {
                    let name = match args.get("name") {
                        Some(n) if !n.trim().is_empty() => n.trim().to_string(),
                        _ => return err("use_skill 需要非空 name 参数".into(), "INVALID_PARAMS"),
                    };
                    let loader = SkillLoader::new(dir.as_path());
                    let skill = match loader.load(&name) {
                        Ok(s) => s,
                        Err(e) => return err(format!("技能 '{name}' 不存在或加载失败: {e}"), "SKILL_NOT_FOUND"),
                    };
                    // 渲染 prompt_template：{{var}} 替换为 args 中的值
                    let mut rendered = skill.prompt_template.clone();
                    for (k, v) in args {
                        let placeholder = format!("{{{{{k}}}}}");
                        rendered = rendered.replace(&placeholder, v);
                    }
                    ok(format!(
                        "【技能: {} v{}】\n{}\n\n请严格遵循以上技能指引完成任务。",
                        skill.name, skill.version, rendered
                    ))
                })
            }),
        ));
    }

    // create_skill
    {
        let dir = dir.clone();
        registry.register_local(crate::tools::make_local_tool(
            "create_skill",
            "创建新技能（YAML prompt 模板文件）。参数: name(必填,英文标识), description(必填), prompt_template(必填,可用 {{text}} 等变量), triggers(可选,逗号分隔触发词)。",
            json!({"type":"object","properties":{
                "name":{"type":"string","description":"技能英文标识，如 my-skill"},
                "description":{"type":"string","description":"技能描述"},
                "prompt_template":{"type":"string","description":"技能 prompt 模板，可用 {{text}} 等占位符"},
                "triggers":{"type":"string","description":"逗号分隔的触发词"}
            },"required":["name","description","prompt_template"]}),
            Arc::new(move |args| {
                let dir = dir.clone();
                Box::pin(async move {
                    let name = match args.get("name") {
                        Some(n) if !n.trim().is_empty() => sanitize_name(n.trim()),
                        _ => return err("create_skill 需要非空 name".into(), "INVALID_PARAMS"),
                    };
                    let description = args.get("description").cloned().unwrap_or_default();
                    let template = args.get("prompt_template").cloned().unwrap_or_default();
                    if description.is_empty() || template.is_empty() {
                        return err("description 和 prompt_template 不可为空".into(), "INVALID_PARAMS");
                    }
                    let triggers = args.get("triggers").cloned().unwrap_or_default();
                    let trigger_list: Vec<String> = triggers
                        .split(',')
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .collect();

                    if let Err(e) = std::fs::create_dir_all(dir.as_path()) {
                        return err(format!("创建技能目录失败: {e}"), "IO_ERROR");
                    }
                    let path = dir.as_path().join(format!("{name}.yaml"));
                    if path.exists() {
                        return err(format!("技能 '{name}' 已存在: {}", path.display()), "SKILL_EXISTS");
                    }
                    let content = render_create_skill_yaml(&name, &description, &template, &trigger_list);
                    // 落盘前 L2 恶意扫描（与 SkillLoader::install_yaml 同源同标准）
                    let verdict = SkillInstallScanner::new().scan(&content);
                    if verdict.blocked {
                        log_skill_block("tool_create", &name, Some(path.as_path()), &verdict);
                        return err(
                            format!(
                                "技能内容被安全层拒绝（{}）：{}",
                                verdict.code(),
                                verdict.reason
                            ),
                            crate::security::skill_scan::SKILL_MALWARE_BLOCKED,
                        );
                    }
                    match std::fs::write(&path, content) {
                        Ok(()) => ok(format!("技能 '{name}' 已创建: {}", path.display())),
                        Err(e) => err(format!("写入技能文件失败: {e}"), "IO_ERROR"),
                    }
                })
            }),
        ));
    }

    // kb_search：检索本地知识库
    {
        registry.register_local(crate::tools::make_local_tool(
            "kb_search",
            "检索本地知识库（BM25全文检索）。当用户询问之前对话内容、已灌入的文档、项目资料时使用。参数: query(必填), top_k(可选,默认5)。",
            json!({"type":"object","properties":{
                "query":{"type":"string","description":"检索查询词"},
                "top_k":{"type":"integer","description":"返回结果数，默认5"}
            },"required":["query"]}),
            Arc::new(move |args| {
                Box::pin(async move {
                    let query = match args.get("query") {
                        Some(q) if !q.trim().is_empty() => q.clone(),
                        _ => return err("kb_search 需要非空 query".into(), "INVALID_PARAMS"),
                    };
                    let top_k = args.get("top_k").and_then(|v| v.parse::<u64>().ok()).unwrap_or(5) as usize;
                    let result = crate::kb::gateway::search(&json!({"query": query, "topK": top_k}));
                    match result.get("items").and_then(|v| v.as_array()) {
                        Some(items) if !items.is_empty() => {
                            let lines: Vec<String> = items.iter().enumerate().map(|(i, item)| {
                                let title = item.get("title").and_then(|v| v.as_str()).unwrap_or("");
                                let chunk = item.get("chunk").and_then(|v| v.as_str()).unwrap_or("");
                                let score = item.get("score").and_then(|v| v.as_f64()).unwrap_or(0.0);
                                format!("{}. [{}] (相关度:{:.0}) {}", i+1, title, score*100.0, chunk)
                            }).collect();
                            ok(format!("知识库检索结果（{}条）:\n{}", items.len(), lines.join("\n---\n")))
                        }
                        _ => ok("知识库中未找到相关内容。可用 kb_ingest 灌入文档。".into()),
                    }
                })
            }),
        ));
    }

    // kb_ingest：灌入文档到知识库
    {
        registry.register_local(crate::tools::make_local_tool(
            "kb_ingest",
            "向本地知识库灌入文档（标题+正文），之后可用 kb_search 检索。参数: title(必填), text(必填)。",
            json!({"type":"object","properties":{
                "title":{"type":"string","description":"文档标题"},
                "text":{"type":"string","description":"文档正文"}
            },"required":["title","text"]}),
            Arc::new(move |args| {
                Box::pin(async move {
                    let title = match args.get("title") {
                        Some(t) if !t.trim().is_empty() => t.clone(),
                        _ => return err("kb_ingest 需要非空 title".into(), "INVALID_PARAMS"),
                    };
                    let text = match args.get("text") {
                        Some(t) if !t.trim().is_empty() => t.clone(),
                        _ => return err("kb_ingest 需要非空 text".into(), "INVALID_PARAMS"),
                    };
                    let result = crate::kb::gateway::ingest(&json!({"title": title, "text": text}));
                    if result.get("ok").and_then(|v| v.as_bool()).unwrap_or(false) {
                        ok(format!("文档已灌入知识库（总计{}篇，{}个分块）",
                            result.get("documents").and_then(|v| v.as_u64()).unwrap_or(0),
                            result.get("chunks").and_then(|v| v.as_u64()).unwrap_or(0)))
                    } else {
                        err(result.get("error").and_then(|v| v.as_str()).unwrap_or("灌入失败").to_string(), "KB_INGEST_ERROR")
                    }
                })
            }),
        ));
    }
}

fn sanitize_name(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect()
}

/// 渲染 `create_skill` 产出的技能 YAML 内容。
///
/// 抽成纯函数以便落盘前安全扫描与单测复用——`create_skill` 与
/// `SkillLoader::install_yaml` 共用同一套 L2 恶意扫描标准。
fn render_create_skill_yaml(
    name: &str,
    description: &str,
    prompt_template: &str,
    triggers: &[String],
) -> String {
    let triggers_yaml = triggers
        .iter()
        .map(|t| format!("  - {t}"))
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "name: {name}\ndescription: {description}\nversion: \"1.0.0\"\nauthor: marguerite-agent\nintent:\n{triggers_yaml}\ntools: []\nprompt_template: |\n{}\n",
        prompt_template
            .lines()
            .map(|l| format!("  {l}"))
            .collect::<Vec<_>>()
            .join("\n")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 良性技能 YAML 渲染正确且通过安全扫描。
    #[test]
    fn render_create_skill_yaml_benign_and_passes_scan() {
        let content =
            render_create_skill_yaml("my-skill", "求和技能", "请对给定数字求和并返回结果。", &["sum".into()]);
        assert!(content.contains("name: my-skill"));
        assert!(content.contains("prompt_template:"));
        assert!(content.contains("  - sum"));
        let v = crate::security::skill_scan::SkillInstallScanner::new().scan(&content);
        assert!(v.is_allowed(), "良性技能内容应放行: {:?}", v.reason);
    }

    /// create_skill 路径：prompt_template 内嵌 eval → 落盘前必须被安全层拒绝。
    #[test]
    fn create_skill_malicious_template_blocked_by_scan() {
        let evil_tpl = "result = eval(user_input)\nreturn result";
        let content = render_create_skill_yaml("evil", "x", evil_tpl, &[]);
        let v = crate::security::skill_scan::SkillInstallScanner::new().scan(&content);
        assert!(v.blocked, "含 eval 的 create_skill 内容应被拒绝");
        assert_eq!(v.code(), crate::security::skill_scan::SKILL_MALWARE_BLOCKED);
    }
}
