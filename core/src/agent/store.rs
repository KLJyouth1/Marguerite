//! 智能体（Agent）台账：真实 CRUD + 持久化 `data_dir/agents.json`
//!
//! 解决「自定义性格与技能组合智能体是假的」问题：此前 `agents.list` 硬编码空数组，
//! 且 create/update/delete 无 Rust 实现。本模块提供进程单例台账，内置预置智能体
//! （builtin）与用户自定义智能体（custom）统一落盘，供 `agents.list/create/update/delete`
//! 网关方法与对话人设注入（`personality_hint`）消费。

use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::agent::profile::auto_profile;
use crate::domain::Message;
use crate::persistence::config::Config;

/// 智能体记录（字段对齐 WebUI `types/agents.ts` 契约，camelCase + snake_case 双写）
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AgentRecord {
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub tools: Vec<String>,
    /// 技能组合（自定义智能体可为空，内置智能体通常带默认技能）
    #[serde(default)]
    pub skills: Vec<String>,
    /// 性格/人设（system prompt）：对话时注入到模型系统消息
    #[serde(default)]
    pub system_prompt: String,
    #[serde(default)]
    pub workspace: String,
    #[serde(default)]
    pub agent_dir: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub is_builtin: bool,
    #[serde(default)]
    pub model: String,
}

fn default_true() -> bool {
    true
}

impl AgentRecord {
    /// 序列化为网关契约（camelCase + snake_case 双写，兼容两端读取）
    pub fn to_value(&self) -> Value {
        json!({
            "id": self.id,
            "name": self.name,
            "description": self.description,
            "tools": self.tools,
            "skills": self.skills,
            "system_prompt": self.system_prompt,
            "systemPrompt": self.system_prompt,
            "workspace": self.workspace,
            "agent_dir": self.agent_dir,
            "agentDir": self.agent_dir,
            "enabled": self.enabled,
            "is_builtin": self.is_builtin,
            "isBuiltin": self.is_builtin,
            "type": if self.is_builtin { "builtin" } else { "custom" },
            "model": self.model,
        })
    }
}

/// 智能体台账（单例 + 落盘 `data_dir/agents.json`）
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct AgentStore {
    pub agents: Vec<AgentRecord>,
}

impl AgentStore {
    fn path() -> PathBuf {
        Config::data_dir().join("agents.json")
    }

    fn load() -> Self {
        std::fs::read_to_string(Self::path())
            .ok()
            .and_then(|raw| serde_json::from_str(&raw).ok())
            .unwrap_or_default()
    }

    fn save(&self) {
        if let Some(dir) = Self::path().parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(raw) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(Self::path(), raw);
        }
    }

    fn seed_builtins(&mut self) {
        let builtins = builtin_agents();
        let mut changed = false;
        for b in builtins {
            if let Some(existing) = self.agents.iter_mut().find(|a| a.id == b.id) {
                // 更新已有内置智能体的定义（工具名、system_prompt 等可能已修正）
                if existing.is_builtin {
                    existing.name = b.name;
                    existing.description = b.description;
                    existing.model = b.model;
                    existing.tools = b.tools;
                    existing.system_prompt = b.system_prompt;
                    existing.skills = b.skills;
                    changed = true;
                }
            } else {
                self.agents.push(b);
                changed = true;
            }
        }
        if changed {
            self.save();
        }
    }

    fn find(&self, id: &str) -> Option<&AgentRecord> {
        self.agents.iter().find(|a| a.id == id)
    }

    fn find_mut(&mut self, id: &str) -> Option<&mut AgentRecord> {
        self.agents.iter_mut().find(|a| a.id == id)
    }
}

fn store() -> &'static Mutex<AgentStore> {
    static S: OnceLock<Mutex<AgentStore>> = OnceLock::new();
    S.get_or_init(|| {
        let mut s = AgentStore::load();
        s.seed_builtins();
        Mutex::new(s)
    })
}

/// 内置预置智能体（可被「自定义」复制为 custom，本身不可编辑）
fn builtin_agents() -> Vec<AgentRecord> {
    let s = |items: &[&str]| items.iter().map(|x| x.to_string()).collect::<Vec<String>>();
    let all_tools = s(&[
        "web_search",
        "web_fetch",
        "web_research",
        "browser",
        "system_control",
        "computer_use",
        "window_manage",
        "clipboard",
        "process_manage",
        "office_automation",
        "read_file",
        "write_file",
        "edit_file",
        "apply_patch",
        "list_dir",
        "find_files",
        "grep",
        "codebase_search",
        "shell_exec",
        "code_exec",
        "git",
        "memory_search",
        "memory_save",
        "vision",
        "research",
        "list_skills",
        "use_skill",
        "create_skill",
        "automation",
        "json_query",
        "kb_search",
        "kb_ingest",
    ]);
    vec![
        AgentRecord {
            id: "main".to_string(),
            name: "主智能体".to_string(),
            description: "通用助手，负责日常对话、任务编排与工具调用。".to_string(),
            tools: all_tools.clone(),
            skills: s(&["通用问答", "代码生成", "文件操作", "联网搜索", "浏览器操控"]),
            system_prompt: "你是 Marguerite 主智能体，一个能力全面的 AI 助手。你拥有文件操作、联网搜索、浏览器自动化、电脑操控、代码执行、知识库、记忆、技能等工具。\n\n核心原则：\n1. 绝不轻易说「无法」「不能」「建议您手动」——你有工具，先调用工具尝试。\n2. 一个工具失败时，换另一个工具或方法继续，不要因一次失败就放弃整个任务。\n3. 已获取的信息要充分利用来完成任务，不要因为缺少某一条信息就宣布失败。\n4. 需要实时信息时用 web_search/web_fetch；需要网页交互时用 browser；需要操控电脑软件时用 system_control/computer_use。\n5. 主动判断何时需要调用工具，不要凭空猜测可查询的信息。\n6. 用与用户相同的语言回复，语言简洁、逻辑清晰，给出可执行的结果而非建议。\n\n上下文连续性（最重要）：\n7. 你必须仔细阅读并理解本次对话中的所有历史消息（用户和助手的每一轮），后续指令通常基于前文内容。\n8. 当用户说「整理成网页」「继续」「把上面的内容做成文件」等指代性指令时，必须回溯前文找到对应内容并直接执行，绝不能要求用户重新提供。\n9. 每次回复前先回顾：用户之前说了什么？我之前做了什么？用户现在要我做什么？确保行为连贯。\n10. 跨会话记忆：调用 memory_search 检索历史会话中的相关信息，结合记忆理解用户意图和偏好。\n11. 如果上下文中已有足够信息完成任务，直接完成，不要反问或要求重复输入。".to_string(),
            is_builtin: true,
            ..Default::default()
        },
        AgentRecord {
            id: "code-architect".to_string(),
            name: "架构师".to_string(),
            description: "系统设计与架构决策，技术方案评审。".to_string(),
            tools: s(&["read_file", "write_file", "edit_file", "list_dir", "find_files", "grep", "codebase_search", "web_search", "web_fetch", "memory_search", "list_skills", "use_skill"]),
            skills: s(&["系统设计", "架构评审", "技术选型", "代码规划"]),
            system_prompt: "你是资深软件架构师 Bob。擅长系统设计、模块划分、技术选型与架构评审。输出结构化设计文档，包含类图/时序图/文件清单/任务列表。用与用户需求相同的语言回复。".to_string(),
            is_builtin: true,
            ..Default::default()
        },
        AgentRecord {
            id: "code-engineer".to_string(),
            name: "工程师".to_string(),
            description: "编写优雅、可读、可扩展、高效的代码。".to_string(),
            tools: s(&["read_file", "write_file", "edit_file", "apply_patch", "list_dir", "find_files", "grep", "codebase_search", "shell_exec", "code_exec", "git", "memory_search", "list_skills", "use_skill", "create_skill"]),
            skills: s(&["代码实现", "测试驱动", "调试修复", "Git 工作流"]),
            system_prompt: "你是软件工程师 Alex。负责编写优雅、可读、可扩展、高效的代码，遵循 Google 风格规范。先理解上下文再动手，小步提交，每步可验证。用与用户需求相同的语言回复。".to_string(),
            is_builtin: true,
            ..Default::default()
        },
        AgentRecord {
            id: "code-reviewer".to_string(),
            name: "代码审查专家".to_string(),
            description: "代码审查、安全审计、质量把关。".to_string(),
            tools: s(&["read_file", "grep", "codebase_search", "find_files", "list_dir", "memory_search", "list_skills", "use_skill"]),
            skills: s(&["代码审查", "安全审计", "性能优化", "规范检查"]),
            system_prompt: "你是资深代码审查专家。注重代码规范、安全性、可维护性与边界条件，给出可执行的改进建议。审查时逐项列出问题、严重级别和修复方案。".to_string(),
            is_builtin: true,
            ..Default::default()
        },
        AgentRecord {
            id: "qa-engineer".to_string(),
            name: "测试工程师".to_string(),
            description: "测试用例设计、质量保障、缺陷追踪。".to_string(),
            tools: s(&["read_file", "write_file", "edit_file", "shell_exec", "code_exec", "grep", "find_files", "list_skills", "use_skill"]),
            skills: s(&["测试设计", "自动化测试", "边界分析", "回归验证"]),
            system_prompt: "你是 QA 测试工程师。擅长设计测试用例、边界分析、自动化测试和缺陷报告。确保每个功能都有正面和负面测试覆盖。".to_string(),
            is_builtin: true,
            ..Default::default()
        },
        AgentRecord {
            id: "product-manager".to_string(),
            name: "产品经理".to_string(),
            description: "需求分析、PRD 撰写、优先级排序。".to_string(),
            tools: s(&["web_search", "web_fetch", "web_research", "memory_search", "memory_save", "list_skills", "use_skill"]),
            skills: s(&["需求分析", "PRD 撰写", "用户故事", "优先级排序"]),
            system_prompt: "你是产品经理 Alice。负责需求分析、PRD 撰写、用户故事拆解和优先级排序。输出结构化的产品文档，包含用户场景、功能列表和验收标准。".to_string(),
            is_builtin: true,
            ..Default::default()
        },
        AgentRecord {
            id: "researcher".to_string(),
            name: "研究员".to_string(),
            description: "深度联网研究、信息检索、报告撰写。".to_string(),
            tools: s(&["web_search", "web_fetch", "web_research", "browser", "research", "memory_search", "memory_save", "read_file", "write_file", "list_skills", "use_skill", "kb_search", "kb_ingest"]),
            skills: s(&["联网研究", "文献综述", "信息验证", "报告撰写"]),
            system_prompt: "你是深度研究员。擅长多源信息检索、交叉验证、综合分析。使用 web_research 进行检索+深读，用 browser 处理需要登录的页面，输出带来源引用的结构化研究报告。".to_string(),
            is_builtin: true,
            ..Default::default()
        },
        AgentRecord {
            id: "browser-operator".to_string(),
            name: "浏览器操控员".to_string(),
            description: "浏览器自动化、网页操作、数据采集。".to_string(),
            tools: s(&["browser", "web_search", "web_fetch", "system_control", "computer_use", "vision", "screenshot", "list_skills", "use_skill"]),
            skills: s(&["浏览器自动化", "网页数据采集", "表单填写", "视觉定位"]),
            system_prompt: "你是浏览器自动化专家。使用 browser 工具操控真实 Chromium（open/click/type/screenshot/a11y/vision_click），完成网页导航、数据采集、表单填写等任务。优先用 a11y 语义定位，视觉定位作为补充。".to_string(),
            is_builtin: true,
            ..Default::default()
        },
        AgentRecord {
            id: "computer-operator".to_string(),
            name: "电脑操控员".to_string(),
            description: "桌面操控、鼠标键盘控制、GUI 自动化。".to_string(),
            tools: s(&["system_control", "computer_use", "browser", "vision", "screenshot", "shell_exec", "window_manage", "clipboard", "process_manage", "office_automation", "list_skills", "use_skill"]),
            skills: s(&["桌面自动化", "GUI 定位", "键鼠控制", "OCR 识别", "办公软件操控", "窗口管理"]),
            system_prompt: "你是桌面自动化专家。使用 system_control/computer_use 操控鼠标键盘、截图、OCR；用 window_manage 管理窗口；用 process_manage 启动/关闭程序；用 office_automation 操控 WPS/Office；用 clipboard 读写剪贴板。操作前先截图观察，确认坐标后再执行，避免误操作。绝不轻易说无法完成，先用工具尝试。".to_string(),
            is_builtin: true,
            ..Default::default()
        },
        AgentRecord {
            id: "data-analyst".to_string(),
            name: "数据分析师".to_string(),
            description: "数据统计、分析、可视化。".to_string(),
            tools: s(&["read_file", "write_file", "shell_exec", "code_exec", "web_search", "web_fetch", "list_dir", "find_files", "json_query", "list_skills", "use_skill", "kb_search"]),
            skills: s(&["数据分析", "统计建模", "图表生成", "SQL 查询"]),
            system_prompt: "你是资深数据分析师。偏好用数据和事实说话，使用 code_exec 运行 Python 进行数据分析和可视化，输出结构化结论与图表建议。".to_string(),
            is_builtin: true,
            ..Default::default()
        },
        AgentRecord {
            id: "devops".to_string(),
            name: "运维工程师".to_string(),
            description: "部署、CI/CD、服务器管理、监控。".to_string(),
            tools: s(&["shell_exec", "read_file", "write_file", "edit_file", "git", "web_search", "web_fetch", "list_dir", "find_files", "list_skills", "use_skill"]),
            skills: s(&["部署运维", "CI/CD", "监控告警", "故障排查"]),
            system_prompt: "你是运维工程师。擅长服务器管理、部署流水线、容器编排、监控告警和故障排查。操作前确认影响范围，危险操作需说明后果。".to_string(),
            is_builtin: true,
            ..Default::default()
        },
        AgentRecord {
            id: "writer".to_string(),
            name: "文案写作".to_string(),
            description: "内容创作、文案撰写、翻译润色。".to_string(),
            tools: s(&["web_search", "web_fetch", "memory_search", "read_file", "write_file", "list_skills", "use_skill", "create_skill"]),
            skills: s(&["文案创作", "翻译润色", "内容策划", "SEO 优化"]),
            system_prompt: "你是专业文案写手。擅长各类文体创作、翻译润色、内容策划。根据场景调整语气和风格，输出有感染力的文字。".to_string(),
            is_builtin: true,
            ..Default::default()
        },
    ]
}

fn apply_patch(record: &mut AgentRecord, params: &Value) {
    if let Some(v) = params.get("name").and_then(Value::as_str) {
        record.name = v.to_string();
    }
    if let Some(v) = params.get("description").and_then(Value::as_str) {
        record.description = v.to_string();
    }
    if let Some(v) = params.get("system_prompt").and_then(Value::as_str) {
        record.system_prompt = v.to_string();
    }
    if let Some(v) = params.get("systemPrompt").and_then(Value::as_str) {
        record.system_prompt = v.to_string();
    }
    if let Some(v) = params.get("workspace").and_then(Value::as_str) {
        record.workspace = v.to_string();
    }
    if let Some(v) = params.get("agent_dir").and_then(Value::as_str) {
        record.agent_dir = v.to_string();
    }
    if let Some(v) = params.get("agentDir").and_then(Value::as_str) {
        record.agent_dir = v.to_string();
    }
    if let Some(v) = params.get("model").and_then(Value::as_str) {
        record.model = v.to_string();
    }
    if let Some(v) = params.get("enabled").and_then(Value::as_bool) {
        record.enabled = v;
    }
    if let Some(v) = params.get("tools").and_then(Value::as_array) {
        record.tools = v
            .iter()
            .filter_map(|x| {
                x.as_str()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_string)
            })
            .collect();
    }
    if let Some(v) = params.get("skills").and_then(Value::as_array) {
        record.skills = v
            .iter()
            .filter_map(|x| {
                x.as_str()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_string)
            })
            .collect();
    }
}

/// `agents.list`：返回全部智能体（内置 + 自定义），按是否内置排序、内置在前。
pub fn list() -> Value {
    let guard = store().lock().unwrap_or_else(|e| e.into_inner());
    let mut items: Vec<Value> = guard.agents.iter().map(AgentRecord::to_value).collect();
    items.sort_by_key(|v| v["is_builtin"].as_bool() != Some(true)); // 内置在前
    json!({ "agents": items, "items": items })
}

/// `agents.get`：按 id 查单个智能体；不存在返回 None。
pub fn get(id: &str) -> Option<Value> {
    let guard = store().lock().unwrap_or_else(|e| e.into_inner());
    guard.find(id).map(AgentRecord::to_value)
}

/// 按 id 求系统人设片段（对话注入用；业务无关，供 chat 层调用）。
///
/// DSH persona 组合语义（v6）：core 段文本含独立 `<!--persona-complete-->`
/// 标记行时，标记剥除且该段 `complete=true`——其后技能/工具段被阻断不追加
/// （对齐上游 persona「complete 阻断后续 prompt 追加」）。
pub fn personality_for(id: &str) -> Option<String> {
    let guard = store().lock().unwrap_or_else(|e| e.into_inner());
    let record = guard.find(id)?;
    if !record.enabled {
        return None;
    }
    let mut sections: Vec<crate::persona::PersonaSection> = Vec::new();
    if !record.system_prompt.trim().is_empty() {
        let (content, complete) = crate::persona::strip_complete_marker(record.system_prompt.trim());
        sections.push(crate::persona::PersonaSection {
            id: "core".into(),
            content,
            complete,
        });
    }
    if !record.skills.is_empty() {
        sections.push(crate::persona::PersonaSection::new(
            "skills",
            &format!("技能组合：{}。", record.skills.join("、")),
        ));
    }
    if !record.tools.is_empty() {
        sections.push(crate::persona::PersonaSection::new(
            "tools",
            &format!("可用工具：{}。", record.tools.join("、")),
        ));
    }
    let rendered = crate::persona::render_sections(&sections, &std::collections::BTreeMap::new());
    (!rendered.trim().is_empty()).then_some(rendered)
}

/// 解析 create 参数中的可选 `history`，转换为抽取式画像所需的 `Vec<Message>`。
/// 解析失败（字段缺失/类型异常）或缺失时返回空，交由上游决定（诚实边界：不编造）。
fn parse_history(params: &Value) -> Vec<Message> {
    params
        .get("history")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|v| serde_json::from_value::<Message>(v.clone()).ok())
                .collect()
        })
        .unwrap_or_default()
}

/// `agents.create`：新建自定义智能体。id 已存在（含内置）返回 `agent.exists`。
pub fn create(params: &Value) -> Result<Value, String> {
    let id = params
        .get("id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "id 不可为空".to_string())?;
    let mut guard = store().lock().unwrap_or_else(|e| e.into_inner());
    if guard.find(id).is_some() {
        return Err("agent.exists".to_string());
    }
    let mut record = AgentRecord {
        id: id.to_string(),
        enabled: true,
        is_builtin: false,
        ..Default::default()
    };
    apply_patch(&mut record, params);
    // W-5：未显式给定 system_prompt 时，据可选 history 以确定性抽取式自动生成人设
    // （接 P-11 auto_profile；不烧 token、不编造）。
    // 注意：就地写入 record.system_prompt，**绝不**二次加锁调 update —— 否则与上方
    // guard 重入死锁（std::sync::Mutex 非重入）。
    if record.system_prompt.trim().is_empty() {
        let history = parse_history(params);
        if !history.is_empty() {
            record.system_prompt = auto_profile(&history).system_prompt();
        }
    }
    guard.agents.push(record.clone());
    guard.save();
    Ok(record.to_value())
}

/// `agents.update`：更新自定义智能体。内置智能体不可改（`agent.builtin_immutable`）；
/// 不存在返回 `agent.not_found`。
pub fn update(params: &Value) -> Result<Value, String> {
    let id = params
        .get("id")
        .and_then(Value::as_str)
        .ok_or_else(|| "id 不可为空".to_string())?;
    let mut guard = store().lock().unwrap_or_else(|e| e.into_inner());
    let record = guard
        .find_mut(id)
        .ok_or_else(|| "agent.not_found".to_string())?;
    if record.is_builtin {
        return Err("agent.builtin_immutable".to_string());
    }
    apply_patch(record, params);
    let cloned = record.clone();
    guard.save();
    Ok(cloned.to_value())
}

/// `agents.delete`：删除自定义智能体。内置不可删（`agent.builtin_immutable`）。
pub fn delete(id: &str) -> Result<Value, String> {
    let mut guard = store().lock().unwrap_or_else(|e| e.into_inner());
    let idx = guard
        .agents
        .iter()
        .position(|a| a.id == id)
        .ok_or_else(|| "agent.not_found".to_string())?;
    if guard.agents[idx].is_builtin {
        return Err("agent.builtin_immutable".to_string());
    }
    guard.agents.remove(idx);
    guard.save();
    Ok(json!({ "deleted": true, "id": id }))
}

/// 从会话 key 解析智能体 id（`agent:{agentId}:...` 前缀），非智能体会话返回 None。
pub fn agent_id_from_session(session: &str) -> Option<&str> {
    session
        .strip_prefix("agent:")?
        .split(':')
        .next()
        .filter(|s| !s.is_empty())
}

/// 测试专用：清空自定义智能体并重建内置预置，保证单例台账在测试间隔离。
#[cfg(test)]
pub fn reset_for_test() {
    let s = store();
    let mut g = s.lock().unwrap_or_else(|e| e.into_inner());
    g.agents.retain(|a| a.is_builtin);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seed_builtins_once_and_list_contract() {
        // 直接构造 store 验证 seed 幂等 + 契约键
        let mut s = AgentStore::default();
        s.seed_builtins();
        assert!(s.agents.iter().any(|a| a.id == "main" && a.is_builtin));
        let n = s.agents.len();
        s.seed_builtins(); // 幂等：不重复 seed
        assert_eq!(s.agents.len(), n);
        let v = s.agents[0].to_value();
        assert_eq!(v["isBuiltin"], v["is_builtin"]);
        assert_eq!(v["systemPrompt"], v["system_prompt"]);
        assert_eq!(v["type"], "builtin");
        assert!(v["skills"].is_array());
    }

    #[test]
    fn create_update_delete_roundtrip() {
        // 不再调用 reset_for_test()：它会清空全局台账里所有自定义智能体，
        // 与并发运行的 W-5 测试（创建 auto-profile-agent-*）互相擦除导致隔离性假失败。
        // 改用唯一 id + 首尾 delete，与其他测试严格隔离。
        let _ = delete("roundtrip-polite-assistant");
        let _prev = AgentStore::path();
        // create
        let created = create(&json!({
            "id": "roundtrip-polite-assistant",
            "name": "礼貌助手",
            "system_prompt": "你说话礼貌客气。",
            "skills": ["礼貌", "幽默"],
            "tools": ["web_search"],
        }))
        .expect("create 应成功");
        assert_eq!(created["id"], "roundtrip-polite-assistant");
        assert_eq!(created["type"], "custom");
        assert_eq!(created["skills"][0], "礼貌");

        // 重复创建 → agent.exists
        let dup = create(&json!({ "id": "roundtrip-polite-assistant" })).unwrap_err();
        assert_eq!(dup, "agent.exists");

        // update
        let updated = update(&json!({ "id": "roundtrip-polite-assistant", "name": "超礼貌助手" }))
            .expect("update 应成功");
        assert_eq!(updated["name"], "超礼貌助手");

        // personality 注入片段
        let p = personality_for("roundtrip-polite-assistant").expect("应有人设");
        assert!(p.contains("礼貌客气"));
        assert!(p.contains("技能组合"));

        // delete
        let del = delete("roundtrip-polite-assistant").expect("delete 应成功");
        assert_eq!(del["deleted"], true);
        assert!(personality_for("roundtrip-polite-assistant").is_none());
    }

    #[test]
    fn persona_complete_marker_blocks_skill_and_tool_sections() {
        // DSH persona complete 语义：core 段含标记 → 技能/工具段被阻断
        let _ = delete("persona-complete-agent");
        let _prev = AgentStore::path();
        create(&json!({
            "id": "persona-complete-agent",
            "name": "阻断测试",
            "system_prompt": "只读我这一句。\n<!--persona-complete-->",
            "skills": ["礼貌"],
            "tools": ["web_search"],
        }))
        .expect("create 应成功");
        let p = personality_for("persona-complete-agent").expect("应有人设");
        assert!(p.contains("只读我这一句"), "core 段应保留: {p}");
        assert!(!p.contains("<!--persona-complete-->"), "标记应剥除: {p}");
        assert!(!p.contains("技能组合"), "complete 后技能段应被阻断: {p}");
        assert!(!p.contains("可用工具"), "complete 后工具段应被阻断: {p}");
        let _ = delete("persona-complete-agent");
    }

    #[test]
    fn builtin_immutable_update_and_delete_rejected() {
        // 仅操作内置智能体，无需 reset_for_test()（避免清空并发测试创建的自定义智能体）
        let err_u = update(&json!({ "id": "main", "name": "x" })).unwrap_err();
        assert_eq!(err_u, "agent.builtin_immutable");
        let err_d = delete("main").unwrap_err();
        assert_eq!(err_d, "agent.builtin_immutable");
    }

    #[test]
    fn session_agent_id_parsing() {
        assert_eq!(
            agent_id_from_session("agent:data-analyst:webchat:abc123"),
            Some("data-analyst")
        );
        assert_eq!(
            agent_id_from_session("agent:main:webchat:xyz"),
            Some("main")
        );
        assert_eq!(agent_id_from_session("ws.plain"), None);
        assert_eq!(agent_id_from_session("agent::webchat:x"), None);
    }

    // ── W-5：新 agent 创建主路径默认自动生成人设（接 P-11 auto_profile）──
    #[test]
    fn create_auto_profiles_from_history_when_no_explicit_prompt() {
        let _ = delete("auto-profile-agent-1");
        let created = create(&json!({
            "id": "auto-profile-agent-1",
            "name": "画像自动生成",
            "history": [
                {"role": "user", "content": "写个 rust 函数部署到后端"},
                {"role": "assistant", "content": "好的，我用 Python 和 Rust 写后端代码。"}
            ]
        }))
        .expect("create 应成功");
        // 未显式给 system_prompt → 默认据 history 自动注入人设
        let sp = personality_for("auto-profile-agent-1").expect("应有人设");
        assert!(sp.contains("你是一个工程师。"), "实际：{sp}");
        assert!(sp.contains("Rust"), "实际：{sp}");
        // create 返回值也应含该人设（双写契约一致）
        assert!(created["systemPrompt"]
            .as_str()
            .unwrap_or("")
            .contains("你是一个工程师。"));
        let _ = delete("auto-profile-agent-1");
    }

    #[test]
    fn create_explicit_system_prompt_takes_precedence_over_history() {
        let _ = delete("auto-profile-agent-2");
        let _ = create(&json!({
            "id": "auto-profile-agent-2",
            "name": "显式人设优先",
            "system_prompt": "你是一个自定义翻译官。",
            "history": [
                {"role": "user", "content": "写个 rust 函数部署到后端"},
                {"role": "assistant", "content": "我用 Python 和 Rust 写后端代码。"}
            ]
        }))
        .expect("create 应成功");
        // 显式 system_prompt 不被 auto_profile 覆盖
        let sp = personality_for("auto-profile-agent-2").expect("应有人设");
        assert_eq!(sp, "你是一个自定义翻译官。");
        let _ = delete("auto-profile-agent-2");
    }

    #[test]
    fn create_without_history_leaves_prompt_empty() {
        let _ = delete("auto-profile-agent-3");
        let _ = create(&json!({
            "id": "auto-profile-agent-3",
            "name": "无历史无画像"
        }))
        .expect("create 应成功");
        // 既无 history 也无 system_prompt → 不伪造人设（保持默认空，personality_for 返回 None）
        assert!(
            personality_for("auto-profile-agent-3").is_none(),
            "无种子历史不应编造人设"
        );
        let _ = delete("auto-profile-agent-3");
    }
}
