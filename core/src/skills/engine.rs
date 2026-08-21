//! 技能执行引擎
//!
//! 负责加载、查找和执行技能，管理技能的完整生命周期。

use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use chrono::Utc;
use tracing::{info, warn};

use super::loader::SkillLoader;
use super::registry::{Skill, SkillRegistry};
use crate::agent::store as agent_store;
use crate::cache::PrefixCache;
use crate::intent::router::IntentContext;
use crate::mcp::registry::ToolRegistry;
use crate::model::llm::{LLMClient, LLMConfig};
use crate::persistence::config::Config;
use crate::types::Message;
use crate::usage;

/// 技能执行输出
#[derive(Debug, Clone)]
pub struct SkillOutput {
    /// 技能名称
    pub skill_name: String,
    /// 执行结果文本
    pub result: String,
    /// 是否成功
    pub success: bool,
    /// 执行耗时
    pub duration: Duration,
    /// 使用的 Token 数
    pub tokens_used: u32,
}

impl SkillOutput {
    /// 创建成功的输出
    pub fn success(
        skill_name: String,
        result: String,
        duration: Duration,
        tokens_used: u32,
    ) -> Self {
        Self {
            skill_name,
            result,
            success: true,
            duration,
            tokens_used,
        }
    }

    /// 创建失败的输出
    pub fn failure(skill_name: String, error: String, duration: Duration) -> Self {
        Self {
            skill_name,
            result: error,
            success: false,
            duration,
            tokens_used: 0,
        }
    }
}

/// 技能组合（多个技能顺序编排）的执行结果
///
/// 对应「自定义性格 + 技能组合」中"组合"这一层：把若干技能按序执行，
/// 共享同一条人设（persona），聚合每一步产出。
#[derive(Debug, Clone)]
pub struct CombinationOutput {
    /// 各技能步骤的执行结果（与输入顺序一致）
    pub steps: Vec<SkillOutput>,
    /// 是否全部步骤成功（无 Failure）
    pub success: bool,
    /// 合并后的可读结果（每步带序号标题拼接）
    pub combined_result: String,
}

impl CombinationOutput {
    /// 是否全部步骤成功
    pub fn success(&self) -> bool {
        self.success
    }
}

/// 技能执行引擎
///
/// 管理技能的执行生命周期，包括技能查找、Prompt 渲染和执行调度。
pub struct SkillEngine {
    /// 技能注册表
    registry: Arc<SkillRegistry>,
    /// 工具注册表
    _tool_registry: Arc<ToolRegistry>,
    /// 技能加载器
    loader: SkillLoader,
    /// 真实 LLM 客户端（按运行时配置构建）。
    /// 为 `None` 时技能退化为「渲染 Prompt」离线模式（不报错、不联网）——明确的降级语义，
    /// 而非被锁死的虚假实现。
    llm: Option<LLMClient>,
    /// 客户端前缀缓存落盘路径（W-7b）：`Some` 时每次技能执行后把冷上下文落盘，
    /// 使跨进程重启可复用 KV 前缀；`None` 表示不落盘（离线 / 测试）。
    cache_path: Option<std::path::PathBuf>,
}

impl SkillEngine {
    /// 创建新的技能引擎（离线模式，不携带 LLM 客户端）
    pub fn new(registry: Arc<SkillRegistry>, tool_registry: Arc<ToolRegistry>) -> Self {
        let skills_dir = dirs::home_dir()
            .unwrap_or_else(|| std::path::PathBuf::from("."))
            .join(".marguerite")
            .join("skills");

        Self {
            registry,
            _tool_registry: tool_registry,
            loader: SkillLoader::new(&skills_dir),
            llm: None,
            cache_path: None,
        }
    }

    /// 按运行时配置创建技能引擎，附带真实 LLM 客户端。
    ///
    /// 仅当 `model.api_key` 非空时才构建客户端；否则 `llm` 为 `None`，
    /// 技能回到离线渲染降级模式——保证无密钥环境（测试 / 离线）依旧可用且不联网。
    pub fn with_config(
        registry: Arc<SkillRegistry>,
        tool_registry: Arc<ToolRegistry>,
        config: &Config,
    ) -> Self {
        // KV-cache 极简实践：每个引擎实例持有独立缓存（不污染其他实例 / 测试），
        // 已配密钥时挂载到 LLM 客户端 → 技能执行走精确前缀命中，省去重复预填充。
        // W-7b：缓存默认从 `~/.marguerite/prefix_cache.json` 加载冷上下文（落盘复用），
        // 加载失败静默回退内存缓存，不致命。
        let skills_dir = dirs::home_dir()
            .unwrap_or_else(|| std::path::PathBuf::from("."))
            .join(".marguerite")
            .join("skills");
        let cache_path = dirs::home_dir()
            .unwrap_or_else(|| std::path::PathBuf::from("."))
            .join(".marguerite")
            .join("prefix_cache.json");
        let llm = if config.model.api_key.trim().is_empty() {
            None
        } else {
            let cache = Arc::new(PrefixCache::open(&cache_path));
            Some(LLMClient::new(LLMConfig::from_model_config(config)).with_cache(cache))
        };

        Self {
            registry,
            _tool_registry: tool_registry,
            loader: SkillLoader::new(&skills_dir),
            llm,
            cache_path: if config.model.api_key.trim().is_empty() {
                None
            } else {
                Some(cache_path)
            },
        }
    }

    /// 初始化技能引擎
    ///
    /// 加载内置技能到注册表中。
    pub fn init(&mut self) -> Result<()> {
        let skills = self.loader.load_all()?;
        let count = skills.len();
        let registry =
            Arc::get_mut(&mut self.registry).expect("SkillEngine 必须独占注册表所有权才能初始化");
        for skill in skills {
            if let Err(e) = registry.register(skill) {
                warn!("注册技能失败: {}", e);
            }
        }
        info!("技能引擎初始化完成，加载了 {} 个技能", count);
        Ok(())
    }

    /// 执行技能（无额外人设）
    ///
    /// 等价于 [`SkillEngine::execute_with`]（`persona = None`）。保留此签名以维持
    /// 既有调用点（链执行器 Skill 分支、单测）不变。
    pub async fn execute(&self, skill_name: &str, context: &IntentContext) -> Result<SkillOutput> {
        self.execute_with(skill_name, context, None).await
    }

    /// 带人设（persona）执行单个技能
    ///
    /// `persona` 为 `Some` 且非空时，作为系统指令前缀注入 LLM 调用，使「自定义性格/智能体」
    /// 真正驱动技能产出（关闭此前"技能步不注入人设"的 C 类待办）。
    ///
    /// - 已配置模型密钥：真实执行，persona 拼接到技能系统指令之后。
    /// - 未配置模型密钥（离线降级）：返回「persona + 分隔线 + 渲染 Prompt」，
    ///   让用户直接看到 *本应发送* 的完整指令（明确降级，非静默假成功）。
    pub async fn execute_with(
        &self,
        skill_name: &str,
        context: &IntentContext,
        persona: Option<&str>,
    ) -> Result<SkillOutput> {
        let skill = self
            .registry
            .get(skill_name)
            .ok_or_else(|| anyhow::anyhow!("技能 '{}' 未找到", skill_name))?;
        self.run_one(&skill, context, persona).await
    }

    /// 执行单个技能（persona 可空），真实执行 / 离线降级逻辑统一在此。
    async fn run_one(
        &self,
        skill: &Skill,
        context: &IntentContext,
        persona: Option<&str>,
    ) -> Result<SkillOutput> {
        let start = Instant::now();

        // 渲染 Prompt
        let rendered_prompt = self
            .render_prompt(skill, context)
            .context("渲染 Prompt 失败")?;

        let (result, tokens_used) = match &self.llm {
            Some(client) => {
                // 真实执行：技能元信息作为系统指令，渲染后的 Prompt 作为用户指令。
                let mut system = format!(
                    "你正在执行名为「{}」的技能。该技能的定位：{}。\n\
                     请根据用户给出的指令与上下文，生成符合该技能要求的、可直接交付的专业内容；\
                     不要复述指令本身，不要输出元说明。",
                    skill.name, skill.description
                );
                if let Some(p) = persona {
                    if !p.trim().is_empty() {
                        system = format!("{}\n\n【当前人设 / 智能体要求】\n{}", system, p.trim());
                    }
                }
                let messages = vec![
                    Message::System {
                        content: system,
                        timestamp: Utc::now(),
                    },
                    Message::User {
                        content: rendered_prompt.clone(),
                        timestamp: Utc::now(),
                    },
                ];
                match client.chat_complete_cached(&messages).await {
                    Ok(resp) => (resp.content, resp.usage.total_tokens),
                    Err(e) => {
                        return Ok(SkillOutput::failure(
                            skill.name.clone(),
                            format!("技能 LLM 执行失败: {e:#}"),
                            start.elapsed(),
                        ));
                    }
                }
            }
            None => {
                // 离线降级：未配置模型密钥，返回「persona（若有）+ 渲染 Prompt」，
                // 不联网、不报错，但明示本应发送的完整指令（非虚假实现）。
                let out = match persona {
                    Some(p) if !p.trim().is_empty() => {
                        format!("{}\n\n---\n{}", p.trim(), rendered_prompt)
                    }
                    _ => rendered_prompt.clone(),
                };
                (out, 0)
            }
        };

        let duration = start.elapsed();
        info!(
            "技能 '{}' 执行完成（persona={}），耗时: {:?}, tokens_used: {}",
            skill.name,
            persona.map(|p| !p.trim().is_empty()).unwrap_or(false),
            duration,
            tokens_used
        );

        // 确保用量台账已打开（CLI / 测试可能尚未由网关触发 init），让技能调用被真实记录。
        usage::init();
        // W-7b：技能执行后把冷上下文前缀缓存落盘（跨重启复用）；失败静默忽略，不致命。
        self.persist_cache();

        Ok(SkillOutput::success(
            skill.name.clone(),
            result,
            duration,
            tokens_used,
        ))
    }

    /// 落盘客户端前缀缓存（W-7b）：仅当 `cache_path` 与 `llm` 缓存均存在时写盘；
    /// 失败（磁盘只读 / 目录不存在）静默忽略——技能执行不应因落盘问题而失败。
    fn persist_cache(&self) {
        if let (Some(path), Some(client)) = (&self.cache_path, &self.llm) {
            if let Err(e) = client.save_cache(path) {
                warn!("前缀缓存落盘失败（已忽略）: {e}");
            }
        }
    }

    /// 按序执行一组技能（「技能组合」），共享同一 persona
    ///
    /// 未注册的技能名不会静默跳过，而是作为 [`SkillOutput::failure`] 步骤如实记录，
    /// 让组合结果能反映"某技能缺失"，而非假装全部成功。
    pub async fn execute_combination(
        &self,
        skills: &[&str],
        context: &IntentContext,
    ) -> Result<CombinationOutput> {
        self.execute_combination_with(skills, context, None).await
    }

    /// 带人设按序执行技能组合（见 [`SkillEngine::execute_combination`]，persona 注入每一步）
    pub async fn execute_combination_with(
        &self,
        skills: &[&str],
        context: &IntentContext,
        persona: Option<&str>,
    ) -> Result<CombinationOutput> {
        let mut steps = Vec::with_capacity(skills.len());
        for name in skills {
            let out = match self.registry.get(name) {
                Some(skill) => self.run_one(&skill, context, persona).await?,
                None => SkillOutput::failure(
                    (*name).to_string(),
                    format!("技能 '{}' 未注册（组合中跳过该步）", name),
                    Duration::ZERO,
                ),
            };
            steps.push(out);
        }
        let success = steps.iter().all(|s| s.success);
        let combined_result = steps
            .iter()
            .enumerate()
            .map(|(i, s)| format!("[{}] {}\n{}", i + 1, s.skill_name, s.result))
            .collect::<Vec<_>>()
            .join("\n\n");
        Ok(CombinationOutput {
            steps,
            success,
            combined_result,
        })
    }

    /// 真实执行某个「智能体（性格 + 技能组合）」
    ///
    /// 从 `agent::store` 取该智能体的人设（`personality_for`）与技能清单（`skills`），
    /// 把人设作为 persona、把技能清单作为组合，依次真实执行（配置模型密钥则调 LLM，否则离线降级）。
    ///
    /// 这是「自定义性格与技能组合可对话」在技能执行层的端到端落地：此前 `AgentRecord` 虽存了
    /// `skills` + `system_prompt`，却没有任何代码把它们 *作为整体执行*——本方法补全该缺口。
    ///
    /// 注意：内置/自定义智能体的 `skills` 多为人类可读标签（如"数据分析"），未必与注册表技能 id
    /// 一一对应；未匹配的技能名会如实记为失败步骤（不假装执行成功）。
    pub async fn run_agent_combination(
        &self,
        agent_id: &str,
        context: &IntentContext,
    ) -> Result<CombinationOutput> {
        let record = agent_store::get(agent_id)
            .ok_or_else(|| anyhow::anyhow!("智能体 '{}' 不存在", agent_id))?;
        let skills: Vec<String> = record
            .get("skills")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|x| x.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();
        let persona = agent_store::personality_for(agent_id);
        let skill_refs: Vec<&str> = skills.iter().map(|s| s.as_str()).collect();
        self.execute_combination_with(&skill_refs, context, persona.as_deref())
            .await
    }

    /// 根据意图查找技能
    ///
    /// 在所有技能中查找匹配意图关键词的技能。
    pub fn find_skill_for_intent(&self, intent: &str) -> Option<Arc<Skill>> {
        let skills = self.registry.find_by_intent(intent);
        // 按评分降序排列，取最高分
        skills
            .into_iter()
            .max_by(|a, b| a.rating.partial_cmp(&b.rating).unwrap())
    }

    /// 渲染 Prompt
    ///
    /// 将技能模板中的变量替换为意图上下文中的实际内容。
    /// 支持以下变量：
    /// - `{{text}}` — 用户输入文本
    /// - `{{files}}` — 相关文件列表
    /// - `{{language}}` — 检测到的语言
    /// - `{{context}}` — 额外上下文
    /// - `{{#files}}...{{/files}}` — 文件列表存在时的条件块
    pub fn render_prompt(&self, skill: &Skill, context: &IntentContext) -> Result<String> {
        let mut prompt = skill.prompt_template.clone();

        // 替换 {{text}}
        prompt = prompt.replace("{{text}}", &context.text);

        // 替换 {{language}}（从文件扩展名推断）
        let language = self.detect_language(context);
        prompt = prompt.replace("{{language}}", &language);

        // 替换 {{context}}（当前为文本的摘要）
        let context_str = format!("用户输入: {}", context.text);
        prompt = prompt.replace("{{context}}", &context_str);

        // 处理 {{#files}}...{{/files}} 条件块
        if context.files.is_empty() {
            // 移除整个条件块
            prompt = self.remove_section(&prompt, "files");
        } else {
            // 替换条件块内容中的 {{files}} 变量
            prompt = self.render_files_section(&prompt, context);
        }

        // 替换剩余的 {{files}}（如果未在条件块中处理）
        if prompt.contains("{{files}}") {
            let files_str = context
                .files
                .iter()
                .map(|f| f.display().to_string())
                .collect::<Vec<_>>()
                .join("\n");
            prompt = prompt.replace("{{files}}", &files_str);
        }

        Ok(prompt)
    }

    /// 记录使用情况
    pub fn record_usage(&self, skill_name: &str, _success: bool) {
        // 由于 registry 是 Arc 共享，需要内部可变性
        // 当前简单实现使用 tracing 日志
        info!("技能 '{}' 使用记录: success={}", skill_name, _success);
    }

    /// 检测语言（从文件扩展名推断）
    fn detect_language(&self, context: &IntentContext) -> String {
        if let Some(file) = context.files.first() {
            match file.extension().and_then(|e| e.to_str()) {
                Some("rs") => "Rust".to_string(),
                Some("py") => "Python".to_string(),
                Some("js") => "JavaScript".to_string(),
                Some("ts") => "TypeScript".to_string(),
                Some("tsx") => "TypeScript React".to_string(),
                Some("jsx") => "JavaScript React".to_string(),
                Some("go") => "Go".to_string(),
                Some("java") => "Java".to_string(),
                Some("rb") => "Ruby".to_string(),
                Some("c") => "C".to_string(),
                Some("cpp") | Some("cc") | Some("cxx") => "C++".to_string(),
                Some("cs") => "C#".to_string(),
                Some("php") => "PHP".to_string(),
                Some("swift") => "Swift".to_string(),
                Some("kt") | Some("kts") => "Kotlin".to_string(),
                Some("scala") => "Scala".to_string(),
                Some("toml") => "TOML".to_string(),
                Some("yaml") | Some("yml") => "YAML".to_string(),
                Some("json") => "JSON".to_string(),
                Some("md") => "Markdown".to_string(),
                Some("html") => "HTML".to_string(),
                Some("css") => "CSS".to_string(),
                Some("sql") => "SQL".to_string(),
                Some("sh") | Some("bash") => "Shell".to_string(),
                Some("ps1") => "PowerShell".to_string(),
                _ => "Unknown".to_string(),
            }
        } else {
            "Unknown".to_string()
        }
    }

    /// 移除模板中的条件块
    fn remove_section(&self, template: &str, section_name: &str) -> String {
        let start_tag = format!("{{{{#{}}}}}", section_name);
        let end_tag = format!("{{{{/{}}}}}", section_name);

        let mut result = String::new();
        let mut in_section = false;
        let mut remaining = template;

        loop {
            if in_section {
                if let Some(end_pos) = remaining.find(&end_tag) {
                    result.push_str(&remaining[end_pos + end_tag.len()..]);
                    break;
                }
                // 如果找不到结束标签，停止处理
                break;
            }
            match remaining.find(&start_tag) {
                Some(start_pos) => {
                    result.push_str(&remaining[..start_pos]);
                    remaining = &remaining[start_pos + start_tag.len()..];
                    in_section = true;
                }
                None => {
                    result.push_str(remaining);
                    break;
                }
            }
        }

        result
    }

    /// 渲染文件列表条件块
    fn render_files_section(&self, template: &str, context: &IntentContext) -> String {
        let start_tag = "{{#files}}";
        let end_tag = "{{/files}}";

        let mut result = String::new();
        let mut remaining = template;

        loop {
            match remaining.find(start_tag) {
                Some(start_pos) => {
                    result.push_str(&remaining[..start_pos]);
                    remaining = &remaining[start_pos + start_tag.len()..];

                    match remaining.find(end_tag) {
                        Some(end_pos) => {
                            let section_content = &remaining[..end_pos];
                            // 替换 section 内的 {{files}} 为文件列表
                            let files_str = context
                                .files
                                .iter()
                                .map(|f| f.display().to_string())
                                .collect::<Vec<_>>()
                                .join("\n");
                            let rendered_section = section_content.replace("{{files}}", &files_str);
                            result.push_str(&rendered_section);
                            remaining = &remaining[end_pos + end_tag.len()..];
                        }
                        None => {
                            result.push_str(remaining);
                            break;
                        }
                    }
                }
                None => {
                    result.push_str(remaining);
                    break;
                }
            }
        }

        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::argv::Args;
    use crate::persistence::config::Config;
    use crate::skills::registry::SkillSource;

    fn create_test_context(text: &str, files: Vec<&str>) -> IntentContext {
        let mut ctx = IntentContext::new(text.to_string(), Args::default(), Config::default());
        ctx.files = files.into_iter().map(std::path::PathBuf::from).collect();
        ctx
    }

    fn create_test_skill(name: &str, template: &str) -> Skill {
        Skill {
            name: name.to_string(),
            description: "test".to_string(),
            version: "1.0.0".to_string(),
            author: None,
            intent: vec!["test".to_string()],
            tools: vec![],
            prompt_template: template.to_string(),
            dependencies: vec![],
            source: SkillSource::Builtin,
            rating: 0.0,
            uses: 0,
            rank: 300,
        }
    }

    #[test]
    fn test_render_prompt_text() {
        let registry = Arc::new(SkillRegistry::new());
        let tool_registry = Arc::new(ToolRegistry::new());
        let engine = SkillEngine::new(registry, tool_registry);

        let skill = create_test_skill("test", "用户说: {{text}}");
        let ctx = create_test_context("hello world", vec![]);
        let result = engine.render_prompt(&skill, &ctx).unwrap();
        assert_eq!(result, "用户说: hello world");
    }

    #[test]
    fn test_render_prompt_files() {
        let registry = Arc::new(SkillRegistry::new());
        let tool_registry = Arc::new(ToolRegistry::new());
        let engine = SkillEngine::new(registry, tool_registry);

        let skill = create_test_skill("test", "文件: {{files}}");
        let ctx = create_test_context("test", vec!["src/main.rs", "src/lib.rs"]);
        let result = engine.render_prompt(&skill, &ctx).unwrap();
        assert!(result.contains("src/main.rs"));
        assert!(result.contains("src/lib.rs"));
    }

    #[test]
    fn test_render_prompt_files_section_with_files() {
        let registry = Arc::new(SkillRegistry::new());
        let tool_registry = Arc::new(ToolRegistry::new());
        let engine = SkillEngine::new(registry, tool_registry);

        let template = "开始\n{{#files}}相关文件：{{files}}{{/files}}\n结束";
        let skill = create_test_skill("test", template);
        let ctx = create_test_context("test", vec!["main.rs", "lib.rs"]);
        let result = engine.render_prompt(&skill, &ctx).unwrap();
        assert_eq!(result, "开始\n相关文件：main.rs\nlib.rs\n结束");
    }

    #[test]
    fn test_render_prompt_files_section_without_files() {
        let registry = Arc::new(SkillRegistry::new());
        let tool_registry = Arc::new(ToolRegistry::new());
        let engine = SkillEngine::new(registry, tool_registry);

        let template = "开始\n{{#files}}相关文件：{{files}}{{/files}}\n结束";
        let skill = create_test_skill("test", template);
        let ctx = create_test_context("test", vec![]);
        let result = engine.render_prompt(&skill, &ctx).unwrap();
        assert_eq!(result, "开始\n\n结束");
    }

    #[test]
    fn test_render_prompt_language() {
        let registry = Arc::new(SkillRegistry::new());
        let tool_registry = Arc::new(ToolRegistry::new());
        let engine = SkillEngine::new(registry, tool_registry);

        let skill = create_test_skill("test", "语言: {{language}}");
        let ctx = create_test_context("test", vec!["main.rs"]);
        let result = engine.render_prompt(&skill, &ctx).unwrap();
        assert_eq!(result, "语言: Rust");
    }

    #[test]
    fn test_render_prompt_no_files_unknown_language() {
        let registry = Arc::new(SkillRegistry::new());
        let tool_registry = Arc::new(ToolRegistry::new());
        let engine = SkillEngine::new(registry, tool_registry);

        let skill = create_test_skill("test", "语言: {{language}}");
        let ctx = create_test_context("test", vec![]);
        let result = engine.render_prompt(&skill, &ctx).unwrap();
        assert_eq!(result, "语言: Unknown");
    }

    #[test]
    fn test_render_prompt_context() {
        let registry = Arc::new(SkillRegistry::new());
        let tool_registry = Arc::new(ToolRegistry::new());
        let engine = SkillEngine::new(registry, tool_registry);

        let skill = create_test_skill("test", "上下文: {{context}}");
        let ctx = create_test_context("你好世界", vec![]);
        let result = engine.render_prompt(&skill, &ctx).unwrap();
        assert!(result.contains("你好世界"));
    }

    #[test]
    fn test_find_skill_for_intent() {
        let mut registry = SkillRegistry::new();
        let skill = Skill {
            name: "explain".to_string(),
            description: "解释代码".to_string(),
            version: "1.0.0".to_string(),
            author: None,
            intent: vec!["explain".to_string(), "解释".to_string()],
            tools: vec![],
            prompt_template: "解释: {{text}}".to_string(),
            dependencies: vec![],
            source: SkillSource::Builtin,
            rating: 4.5,
            uses: 0,
            rank: 300,
        };
        registry.register(skill).unwrap();

        let registry = Arc::new(registry);
        let tool_registry = Arc::new(ToolRegistry::new());
        let engine = SkillEngine::new(registry, tool_registry);

        let found = engine.find_skill_for_intent("explain");
        assert!(found.is_some());
        assert_eq!(found.unwrap().name, "explain");

        let not_found = engine.find_skill_for_intent("nonexistent");
        assert!(not_found.is_none());
    }

    #[tokio::test]
    async fn test_execute_skill() {
        let mut registry = SkillRegistry::new();
        let skill = create_test_skill("greet", "你好, {{text}}!");
        registry.register(skill).unwrap();

        let registry = Arc::new(registry);
        let tool_registry = Arc::new(ToolRegistry::new());
        // 离线引擎（无 LLM 客户端）→ 降级返回渲染后的 Prompt，不联网、不报假。
        let engine = SkillEngine::new(registry, tool_registry);

        let ctx = create_test_context("世界", vec![]);
        let output = engine.execute("greet", &ctx).await.unwrap();
        assert!(output.success);
        assert_eq!(output.skill_name, "greet");
        assert!(output.result.contains("你好, 世界!"));
        assert_eq!(output.tokens_used, 0, "离线降级模式不计真实用量");
    }

    #[tokio::test]
    async fn test_execute_skill_with_llm_calls_real_backend() {
        // 构造带假密钥的配置 → with_config 应建立真实 LLM 客户端，execute 真调后端。
        use crate::persistence::config::ModelConfig;
        let config = Config {
            model: ModelConfig {
                api_key: "sk-test-fake".to_string(),
                base_url: "http://127.0.0.1:1".to_string(), // 必败端点：验证离线/失败语义
                model_name: "test-model".to_string(),
                ..Default::default()
            },
            ..Default::default()
        };
        let mut registry = SkillRegistry::new();
        let skill = create_test_skill("greet", "你好, {{text}}!");
        registry.register(skill).unwrap();
        let engine =
            SkillEngine::with_config(Arc::new(registry), Arc::new(ToolRegistry::new()), &config);
        let ctx = create_test_context("世界", vec![]);
        // 后端不可达 → 应返回 failure（真实错误），而非静默回显假成功。
        let output = engine.execute("greet", &ctx).await.unwrap();
        assert!(!output.success, "模型不可达时应真实失败，而非虚假回显");
        assert!(
            !output.result.contains("你好, 世界!"),
            "失败路径不应回显 prompt"
        );
    }

    #[tokio::test]
    async fn test_execute_nonexistent_skill() {
        let registry = Arc::new(SkillRegistry::new());
        let tool_registry = Arc::new(ToolRegistry::new());
        let engine = SkillEngine::new(registry, tool_registry);

        let ctx = create_test_context("test", vec![]);
        let result = engine.execute("nonexistent", &ctx).await;
        assert!(result.is_err());
    }

    #[test]
    fn test_detect_language() {
        let registry = Arc::new(SkillRegistry::new());
        let tool_registry = Arc::new(ToolRegistry::new());
        let engine = SkillEngine::new(registry, tool_registry);

        let test_cases = vec![
            (vec!["main.rs"], "Rust"),
            (vec!["app.py"], "Python"),
            (vec!["index.js"], "JavaScript"),
            (vec!["component.tsx"], "TypeScript React"),
            (vec!["style.css"], "CSS"),
            (vec!["data.json"], "JSON"),
            (vec!["doc.md"], "Markdown"),
            (vec!["script.sh"], "Shell"),
            (vec!["unknown.xyz"], "Unknown"),
            (vec![], "Unknown"),
        ];

        for (files, expected) in test_cases {
            let ctx = create_test_context("test", files.clone());
            let lang = engine.detect_language(&ctx);
            assert_eq!(lang, expected, "文件 {:?} 应检测为 {}", files, expected);
        }
    }

    #[tokio::test]
    async fn test_execute_with_injects_persona_offline() {
        let mut registry = SkillRegistry::new();
        registry
            .register(create_test_skill("greet", "你好, {{text}}!"))
            .unwrap();
        let tool_registry = Arc::new(ToolRegistry::new());
        let engine = SkillEngine::new(Arc::new(registry), tool_registry);

        let ctx = create_test_context("世界", vec![]);
        // 无 persona：仅渲染 Prompt（保持既有契约）
        let plain = engine.execute("greet", &ctx).await.unwrap();
        assert!(plain.result.contains("你好, 世界!"));
        assert!(
            !plain.result.contains("数据分析师"),
            "无 persona 不应含人设"
        );

        // 带 persona（离线降级）：返回「人设 + 分隔线 + 渲染 Prompt」
        let with = engine
            .execute_with("greet", &ctx, Some("你是礼貌的数据分析师"))
            .await
            .unwrap();
        assert!(
            with.result.contains("你是礼貌的数据分析师"),
            "persona 应注入离线输出"
        );
        assert!(with.result.contains("你好, 世界!"));
        assert_eq!(with.tokens_used, 0, "离线降级不计真实用量");
    }

    #[tokio::test]
    async fn test_execute_combination_runs_all_skills() {
        let mut registry = SkillRegistry::new();
        registry
            .register(create_test_skill("greet", "你好, {{text}}!"))
            .unwrap();
        registry
            .register(create_test_skill("explain", "解释: {{text}}"))
            .unwrap();
        let engine = SkillEngine::new(Arc::new(registry), Arc::new(ToolRegistry::new()));

        let ctx = create_test_context("世界", vec![]);
        let out = engine
            .execute_combination(&["greet", "explain"], &ctx)
            .await
            .unwrap();
        assert_eq!(out.steps.len(), 2, "应分别对两个技能执行");
        assert!(out.success(), "离线模式下两步均应成功");
        assert!(out.combined_result.contains("你好, 世界!"));
        assert!(out.combined_result.contains("解释: 世界"));
        assert!(out.combined_result.contains("[1] greet"));
        assert!(out.combined_result.contains("[2] explain"));
    }

    #[tokio::test]
    async fn test_execute_combination_unregistered_skill_fails_step() {
        let mut registry = SkillRegistry::new();
        registry
            .register(create_test_skill("greet", "你好, {{text}}!"))
            .unwrap();
        let engine = SkillEngine::new(Arc::new(registry), Arc::new(ToolRegistry::new()));

        let ctx = create_test_context("世界", vec![]);
        let out = engine
            .execute_combination(&["greet", "ghost"], &ctx)
            .await
            .unwrap();
        assert_eq!(out.steps.len(), 2);
        assert!(out.steps[0].success, "已注册技能应成功");
        assert!(
            !out.steps[1].success,
            "未注册技能应记为失败步骤（不静默跳过）"
        );
        assert!(out.steps[1].result.contains("未注册"));
        assert!(!out.success(), "含失败步骤则组合整体不成功");
    }

    #[tokio::test]
    async fn test_run_agent_combination_glue_with_builtin_agent() {
        // 用始终存在的 builtin "main" 验证组合胶水：取人设 + 技能清单，未匹配技能如实标失败。
        // 不创建/删除自定义智能体，避免与 agent::store 单例测试并发污染。
        let mut registry = SkillRegistry::new();
        registry
            .register(create_test_skill("greet", "你好, {{text}}!"))
            .unwrap();
        let engine = SkillEngine::new(Arc::new(registry), Arc::new(ToolRegistry::new()));

        let ctx = create_test_context("世界", vec![]);
        let out = engine.run_agent_combination("main", &ctx).await.unwrap();
        // "main" 内置技能为可读标签（通用问答/代码生成/文件操作），与注册表 id 不对应 → 全部失败步骤
        assert!(!out.steps.is_empty(), "应至少取到一个技能步骤");
        assert!(
            out.combined_result.contains("未注册"),
            "未匹配技能应诚实标失败"
        );
    }

    #[tokio::test]
    async fn test_execute_with_llm_persona_still_fails_on_bad_key() {
        use crate::persistence::config::ModelConfig;
        let config = Config {
            model: ModelConfig {
                api_key: "sk-test-fake".to_string(),
                base_url: "http://127.0.0.1:1".to_string(),
                model_name: "test-model".to_string(),
                ..Default::default()
            },
            ..Default::default()
        };
        let mut registry = SkillRegistry::new();
        registry
            .register(create_test_skill("greet", "你好, {{text}}!"))
            .unwrap();
        let engine =
            SkillEngine::with_config(Arc::new(registry), Arc::new(ToolRegistry::new()), &config);
        let ctx = create_test_context("世界", vec![]);
        // 即使带 persona，后端不可达也应真实失败（persona 不能把假密钥救活）
        let output = engine
            .execute_with("greet", &ctx, Some("你是礼貌助手"))
            .await
            .unwrap();
        assert!(!output.success, "模型不可达时应真实失败，而非虚假回显");
        assert!(
            !output.result.contains("你好, 世界!"),
            "失败路径不应回显 prompt"
        );
    }
}
