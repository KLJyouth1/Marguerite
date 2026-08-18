//! 会话上下文台账与渐进压缩（B2 升级）
//!
//! 对齐 Web UI `session.event.compaction` / `contextPressure` 契约：
//! - 每个会话维护 (role, text, token 估算) 台账，落盘 `data_dir/context/<session>.json`
//! - [`context_status`] 返回 token 用量 / 预算 / 压缩次数 / 压缩质量
//! - 压缩采用 **4 层渐进**（对齐 Claude Code 的 折叠→微压缩→自动压缩 思想）：
//!   1. 压缩前折叠（[`snip_long`]）：超长工具结果/代码块保留头尾、省略中段；
//!   2. 结构化语义摘要：优先 LLM（[`build_summary_llm`] 按 目标/进展/决策/下一步/文件 维度
//!      生成），无 LLM / 失败时回退确定性规则（[`build_summary`]，零依赖可用）；
//!   3. 滑动窗口保留：最近 [`KEEP_RECENT_TURNS`] 轮原文 verbatim（head/tail 保护）；
//!   4. 预压缩快照 + 可回滚（[`ContextLedger::rollback`]），保证可追溯。
//! - 质量评估：`compression_ratio`（1 - 压缩后/压缩前 token）+ 摘要模式记录。

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::events::{Envelope, EventBus, EventType};
use crate::model::llm::{LLMClient, LLMConfig, LLMProvider};
use crate::persistence::config::Config;
use crate::types::Message;

/// 压缩预算默认值（token）：128K 窗口、80% 告警、85% 触发压缩
pub const DEFAULT_BUDGET: u64 = 128_000;
/// 压缩触发阈值（相对预算的比例）
pub const COMPACT_RATIO: f64 = 0.85;
/// 告警阈值
pub const WARN_RATIO: f64 = 0.80;
/// 压缩后保留的最近轮次（原文 verbatim）
pub const KEEP_RECENT_TURNS: usize = 8;
/// 折叠保留头部字符数
const SNIP_HEAD: usize = 400;
/// 折叠保留尾部字符数
const SNIP_TAIL: usize = 200;

/// 一条上下文记录
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextEntry {
    pub role: String,
    pub text: String,
    pub tokens: u64,
    pub at_ms: u64,
}

/// 会话上下文（落盘结构）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionContext {
    pub session: String,
    pub entries: Vec<ContextEntry>,
    pub budget: u64,
    pub compaction_count: u32,
    pub last_compact_ms: Option<u64>,
    /// 最近一次压缩的结构化摘要（Markdown）
    pub summary: Option<String>,
    /// 最近一次压缩模式（"deterministic" | "llm"）
    pub last_mode: Option<String>,
    /// 最近一次压缩率（1 - 压缩后/压缩前 token）
    pub compression_ratio: Option<f64>,
    /// 可用预压缩快照数量（rollback 目标）
    pub snapshot_count: u32,
    /// 会话关联的项目根目录（跨重启持久化；None = 未绑定项目，属「最近」）
    pub project_path: Option<String>,
}

impl SessionContext {
    fn new(session: &str) -> Self {
        Self {
            session: session.to_string(),
            entries: Vec::new(),
            budget: DEFAULT_BUDGET,
            compaction_count: 0,
            last_compact_ms: None,
            summary: None,
            last_mode: None,
            compression_ratio: None,
            snapshot_count: 0,
            project_path: None,
        }
    }

    /// 绑定/更新项目根目录
    fn set_project(&mut self, project: &str) {
        if self.project_path.as_deref() != Some(project) {
            self.project_path = Some(project.to_string());
        }
    }

    /// 当前 token 用量
    pub fn used_tokens(&self) -> u64 {
        self.entries.iter().map(|e| e.tokens).sum()
    }

    /// 用量占比（0-1）
    pub fn ratio(&self) -> f64 {
        if self.budget == 0 {
            return 0.0;
        }
        self.used_tokens() as f64 / self.budget as f64
    }
}

/// 会话上下文台账（进程单例 + 落盘）
pub struct ContextLedger {
    dir: std::path::PathBuf,
    sessions: HashMap<String, SessionContext>,
}

impl ContextLedger {
    fn new(data_dir: &std::path::Path) -> Self {
        let dir = data_dir.join("context");
        let _ = std::fs::create_dir_all(&dir);
        let sessions = HashMap::new();
        Self { dir, sessions }
    }

    fn path(&self, session: &str) -> std::path::PathBuf {
        let safe: String = session
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        self.dir.join(format!("{safe}.json"))
    }

    fn snapshots_dir(&self, session: &str) -> std::path::PathBuf {
        let safe: String = session
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        self.dir.join("snapshots").join(safe)
    }

    fn load(&mut self, session: &str) -> SessionContext {
        if let Some(s) = self.sessions.get(session) {
            return s.clone();
        }
        let loaded = std::fs::read_to_string(self.path(session))
            .ok()
            .and_then(|raw| serde_json::from_str::<SessionContext>(&raw).ok())
            .unwrap_or_else(|| SessionContext::new(session));
        self.sessions.insert(session.to_string(), loaded.clone());
        loaded
    }

    fn persist(&self, ctx: &SessionContext) {
        if let Ok(raw) = serde_json::to_string_pretty(ctx) {
            let _ = std::fs::write(self.path(&ctx.session), raw);
        }
    }

    /// 记录一轮对话（role: user / assistant），返回更新后的用量占比
    fn record(&mut self, session: &str, role: &str, text: &str) -> f64 {
        let mut ctx = self.load(session);
        ctx.entries.push(ContextEntry {
            role: role.to_string(),
            text: text.to_string(),
            tokens: estimate_tokens(text),
            at_ms: now_ms(),
        });
        // 有界：防止无界增长（每侧最多 4096 条，超出丢弃最旧）
        if ctx.entries.len() > 8192 {
            let drop = ctx.entries.len() - 8192;
            ctx.entries.drain(0..drop);
        }
        let ratio = ctx.ratio();
        self.persist(&ctx);
        self.sessions.insert(session.to_string(), ctx);
        ratio
    }

    /// 读取会话状态
    fn status(&mut self, session: &str) -> SessionContext {
        self.load(session)
    }

    /// 保存预压缩快照（供 `/compact undo` 回滚；超过 5 份清理最旧）
    fn save_snapshot(&self, ctx: &SessionContext, compaction_id: &str) {
        let dir = self.snapshots_dir(&ctx.session);
        let _ = std::fs::create_dir_all(&dir);
        let mut snap = ctx.clone();
        snap.snapshot_count = 0; // 快照内不递归计数
        if let Ok(raw) = serde_json::to_string(&snap) {
            let _ = std::fs::write(dir.join(format!("{compaction_id}.json")), raw);
        }
        // 清理：只保留最近 5 份
        let mut files: Vec<_> = std::fs::read_dir(&dir)
            .map(|rd| rd.flatten().map(|e| e.path()).collect())
            .unwrap_or_default();
        files.sort();
        while files.len() > 5 {
            if let Some(oldest) = files.first() {
                let _ = std::fs::remove_file(oldest);
            }
            files.remove(0);
        }
    }

    /// 恢复最近一次预压缩快照；成功返回 true
    fn rollback(&mut self, session: &str) -> bool {
        let mut ctx = self.load(session);
        let dir = self.snapshots_dir(session);
        let mut files: Vec<_> = std::fs::read_dir(&dir)
            .map(|rd| rd.flatten().map(|e| e.path()).collect())
            .unwrap_or_default();
        files.sort();
        let latest = files.last();
        let Some(latest) = latest else {
            return false;
        };
        let Ok(raw) = std::fs::read_to_string(latest) else {
            return false;
        };
        let Ok(restored) = serde_json::from_str::<SessionContext>(&raw) else {
            return false;
        };
        let _ = std::fs::remove_file(latest); // 回滚一次即消费一份快照
        let restored_count = ctx.snapshot_count.saturating_sub(1);
        ctx.entries = restored.entries;
        ctx.summary = None; // 摘要回到压缩前状态
        ctx.last_mode = None;
        ctx.compression_ratio = None;
        ctx.snapshot_count = restored_count;
        self.persist(&ctx);
        self.sessions.insert(session.to_string(), ctx);
        true
    }

    /// 执行渐进压缩（4 层：折叠 → 语义摘要 → 滑动窗口 → 快照），返回压缩报告
    ///
    /// `llm_summary`：LLM 语义摘要（调用方已尝试生成；None 时回退确定性规则）。
    fn compact(
        &mut self,
        session: &str,
        compaction_id: &str,
        llm_summary: Option<String>,
    ) -> Option<CompactReport> {
        let mut ctx = self.load(session);
        if ctx.entries.len() <= KEEP_RECENT_TURNS * 2 {
            return None; // 太短，跳过
        }
        // 第 4 层：保存预压缩快照（回滚目标）
        self.save_snapshot(&ctx, compaction_id);
        let tokens_before = ctx.used_tokens();
        // 第 1 层：压缩前折叠（超长条目保留头尾，供摘要与保留窗口使用）
        let folded: Vec<ContextEntry> = ctx
            .entries
            .iter()
            .map(|e| ContextEntry {
                role: e.role.clone(),
                text: snip_long(&e.text),
                tokens: e.tokens,
                at_ms: e.at_ms,
            })
            .collect();
        // 第 2 层：语义摘要（LLM 优先，确定性回退）
        let (summary, mode) = match llm_summary {
            Some(s) if !s.trim().is_empty() => (s, "llm"),
            _ => (build_summary(&folded), "deterministic"),
        };
        // 第 3 层：保留最近 N 轮原文，其余替换为摘要（摘要前置）
        let keep = ctx.entries.len().saturating_sub(KEEP_RECENT_TURNS);
        let tail = ctx.entries.split_off(keep);
        ctx.entries.clear();
        if !summary.is_empty() {
            ctx.entries.push(ContextEntry {
                role: "summary".to_string(),
                text: summary.clone(),
                tokens: estimate_tokens(&summary),
                at_ms: now_ms(),
            });
        }
        ctx.entries.extend(tail);
        let tokens_after = ctx.used_tokens();
        ctx.compaction_count += 1;
        ctx.last_compact_ms = Some(now_ms());
        ctx.summary = Some(summary.clone());
        ctx.last_mode = Some(mode.to_string());
        ctx.compression_ratio = if tokens_before == 0 {
            Some(0.0)
        } else {
            Some(1.0 - tokens_after as f64 / tokens_before as f64)
        };
        ctx.snapshot_count = ctx.snapshot_count.saturating_add(1);
        self.persist(&ctx);
        self.sessions.insert(session.to_string(), ctx);
        Some(CompactReport {
            summary,
            mode: mode.to_string(),
            tokens_before,
            tokens_after,
            compression_ratio: 1.0 - tokens_after as f64 / tokens_before.max(1) as f64,
        })
    }
}

/// 压缩报告（模式 + 质量指标）
#[derive(Debug, Clone)]
pub struct CompactReport {
    pub summary: String,
    pub mode: String,
    pub tokens_before: u64,
    pub tokens_after: u64,
    pub compression_ratio: f64,
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Token 估算（兼容旧调用点，转发到高精度实现）。
pub fn estimate_tokens(text: &str) -> u64 {
    estimate_tokens_precise(text)
}

/// 高精度 Token 估算：区分 CJK 与拉丁字符，避免「字节/3」对中文严重低估。
///
/// - CJK / 全角字符：约 1.6 token/字（中文 token 化后通常 1~2 token）；
/// - 拉丁字母 / 数字 / 符号：约 4 字符/token；
/// - 空白：约 0.3 token。
///
/// 最终向上取整且至少 1，保证 `estimate_tokens("")=1` 等不变式成立。
///
/// 注：家族级精确计数需接入各模型 tokenizer（tiktoken/cl100k 等）；当前以
/// 跨家族稳健的多语种启发式替代，**不伪造**精度。
pub fn estimate_tokens_precise(text: &str) -> u64 {
    let mut tokens: f64 = 0.0;
    for ch in text.chars() {
        if is_cjk_char(ch) {
            tokens += 1.6;
        } else if ch.is_whitespace() {
            tokens += 0.3;
        } else {
            tokens += 0.25;
        }
    }
    tokens.ceil().max(1.0) as u64
}

/// 判断是否为 CJK（含日文假名、韩文、全角标点）字符。
fn is_cjk_char(ch: char) -> bool {
    ('\u{3000}'..='\u{303F}').contains(&ch) // CJK 符号和标点
        || ('\u{3040}'..='\u{309F}').contains(&ch) // 平假名
        || ('\u{30A0}'..='\u{30FF}').contains(&ch) // 片假名
        || ('\u{3400}'..='\u{4DBF}').contains(&ch) // 扩展A
        || ('\u{4E00}'..='\u{9FFF}').contains(&ch) // 基本汉字
        || ('\u{AC00}'..='\u{D7AF}').contains(&ch) // 谚文音节
        || ('\u{FF00}'..='\u{FFEF}').contains(&ch) // 全角字符
}

/// 第 1 层折叠：超长文本保留头尾、省略中段（压缩前预处理，降低摘要输入体积）
pub fn snip_long(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= SNIP_HEAD + SNIP_TAIL {
        return text.to_string();
    }
    let head: String = chars[..SNIP_HEAD].iter().collect();
    let tail: String = chars[chars.len() - SNIP_TAIL..].iter().collect();
    format!(
        "{head}\n…[省略 {} 字符]…\n{tail}",
        chars.len() - SNIP_HEAD - SNIP_TAIL
    )
}

/// 从配置加载 LLM 摘要用的 LLM 配置（环境变量 / .env 覆盖；无 key 返回 None）
fn llm_config() -> Option<LLMConfig> {
    let cfg = Config::load().ok()?;
    let m = &cfg.model;
    if m.api_key.trim().is_empty() {
        return None;
    }
    Some(LLMConfig {
        provider: LLMProvider::Custom {
            base_url: m.base_url.clone(),
        },
        model: m.model_name.clone(),
        api_key: m.api_key.clone(),
        base_url: m.base_url.clone(),
        max_tokens: 1024,
        temperature: 0.3,
        timeout_secs: 30,
        thinking: crate::model::thinking::ThinkingEffort::Off,
        context_window: crate::model::thinking::CONTEXT_WINDOW_DEFAULT,
    })
}

/// 第 2 层语义摘要：LLM 按「目标/进展/关键决策/下一步/涉及文件」维度生成 Markdown 摘要。
///
/// 无可用 LLM / 调用失败 / 返回空 → None（调用方回退确定性摘要），绝不 panic。
pub async fn build_summary_llm(entries: &[ContextEntry]) -> Option<String> {
    let llm = llm_config()?;
    let sys = "你是一个会话上下文摘要器。请把对话历史压缩为结构化 Markdown 摘要，\
        必须包含以下五个维度（缺数据写「（无）」）：\
        ## 会话摘要\n- **目标**: 用户最初要完成什么\n- **进展**: 已完成到什么程度\n- **关键决策**: 已确定的技术方案/决定\n- **下一步**: 尚未完成的待办\n- **涉及文件**: 出现的文件路径\n\
        要求：保留关键决策与文件路径原文，删除客套与重复，控制在 400 字以内。";
    let body: String = entries
        .iter()
        .map(|e| format!("[{}]\n{}", e.role, snip_long(&e.text)))
        .collect::<Vec<_>>()
        .join("\n\n");
    let messages = [
        Message::System {
            content: sys.to_string(),
            timestamp: chrono::Utc::now(),
        },
        Message::User {
            content: body,
            timestamp: chrono::Utc::now(),
        },
    ];
    let client = LLMClient::new(llm);
    match client.chat_complete(&messages).await {
        Ok(r) if !r.content.trim().is_empty() => Some(r.content),
        _ => None,
    }
}

/// 结构化摘要（确定性规则抽取，零 LLM 调用）
fn build_summary(entries: &[ContextEntry]) -> String {
    let mut goal = String::new();
    let mut progress = String::new();
    let mut decisions: Vec<String> = Vec::new();
    let mut next_steps: Vec<String> = Vec::new();
    let mut files: Vec<String> = Vec::new();

    for e in entries {
        let text = e.text.trim();
        if text.is_empty() {
            continue;
        }
        let is_user = e.role == "user";
        if goal.is_empty() && is_user {
            goal = clip(text, 120);
        }
        if decisions.len() < 3
            && contains_any(
                text,
                &[
                    "决定",
                    "方案",
                    "选择",
                    "采用",
                    "修改为",
                    "decided",
                    "choose",
                ],
            )
        {
            decisions.push(clip(text, 100));
        }
        if next_steps.len() < 3
            && contains_any(text, &["接下来", "下一步", "待办", "需要", "Next", "TODO"])
        {
            next_steps.push(clip(text, 100));
        }
        if is_user && progress.is_empty() {
            // 保留最近用户输入作为进度锚点（末尾覆盖）
            progress = clip(text, 100);
        }
        // 提取疑似文件路径
        extract_files(text, &mut files);
    }
    // decisions 去重保序
    decisions.dedup();
    next_steps.dedup();
    files.dedup();
    let mut files_list: Vec<String> = files.into_iter().take(5).collect();

    let mut out = String::from("## 会话摘要\n");
    out.push_str(&format!(
        "- **目标**: {}\n",
        if goal.is_empty() { "（无）" } else { &goal }
    ));
    out.push_str(&format!(
        "- **进展**: {}\n",
        if progress.is_empty() {
            "（无）"
        } else {
            &progress
        }
    ));
    out.push_str("- **关键决策**:\n");
    if decisions.is_empty() {
        out.push_str("  - （无）\n");
    } else {
        for d in &decisions {
            out.push_str(&format!("  - {d}\n"));
        }
    }
    out.push_str("- **下一步**:\n");
    if next_steps.is_empty() {
        out.push_str("  - （无）\n");
    } else {
        for n in &next_steps {
            out.push_str(&format!("  - {n}\n"));
        }
    }
    out.push_str("- **涉及文件**:\n");
    if files_list.is_empty() {
        out.push_str("  - （无）\n");
    } else {
        files_list.truncate(5);
        for f in files_list.iter().rev() {
            out.push_str(&format!("  - {f}\n"));
        }
    }
    out
}

fn contains_any(text: &str, keys: &[&str]) -> bool {
    keys.iter().any(|k| text.contains(k))
}

fn clip(text: &str, max: usize) -> String {
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= max {
        return text.to_string();
    }
    let cut: String = chars[..max].iter().collect();
    format!("{cut}…")
}

/// 提取疑似文件路径（带常见扩展名或含 / \ 的片段）
fn extract_files(text: &str, out: &mut Vec<String>) {
    const EXTS: &[&str] = &[
        ".rs", ".py", ".ts", ".tsx", ".js", ".jsx", ".json", ".toml", ".yaml", ".yml", ".md",
        ".txt", ".vue", ".css", ".html", ".go", ".java", ".c", ".cpp", ".h",
    ];
    for token in text.split_whitespace() {
        let t = token.trim_matches(|c: char| "(),。；;\"'`【】[]".contains(c));
        if (t.contains('/') || t.contains('\\') || EXTS.iter().any(|e| t.contains(e)))
            && t.len() > 2
            && t.len() <= 220
            && !out.contains(&t.to_string())
        {
            out.push(t.to_string());
            if out.len() >= 8 {
                return;
            }
        }
    }
}

fn ledger() -> &'static Mutex<ContextLedger> {
    static LEDGER: OnceLock<Mutex<ContextLedger>> = OnceLock::new();
    LEDGER.get_or_init(|| Mutex::new(ContextLedger::new(&Config::data_dir())))
}

/// `context.record_turn`（内部：chat 链路调用）
pub fn record_turn(session: &str, role: &str, text: &str) -> f64 {
    ledger()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .record(session, role, text)
}

/// 读取会话的全部上下文条目（供网关把会话历史落盘到 sessions.db / 水合 chat.history）。
/// 无该会话时返回空 Vec。
pub fn context_entries(session: &str) -> Vec<ContextEntry> {
    ledger()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .status(session)
        .entries
}

/// 绑定会话到项目根目录（跨重启持久化到台账 JSON）。
/// `chat.send` 携带 workspaceId 时调用，使台账层面的会话归属不丢。
pub fn bind_session_project(session: &str, project: &str) {
    let mut g = ledger().lock().unwrap_or_else(|e| e.into_inner());
    let mut ctx = g.load(session);
    ctx.set_project(project);
    g.persist(&ctx);
    g.sessions.insert(session.to_string(), ctx);
}

/// 读取会话绑定的项目根目录（台账优先；无则 None）。
pub fn session_project(session: &str) -> Option<String> {
    ledger()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .status(session)
        .project_path
        .clone()
}

/// 一个会话的上下文台账摘要（供 `sessions.list` 合并，防「数据在、列表丢」）。
#[derive(Debug, Clone)]
pub struct ContextSessionSummary {
    /// 真实 session id（来自 JSON 内容，不受文件名 safe 化影响）
    pub session: String,
    pub message_count: usize,
    /// 最近一次记录的毫秒时间戳
    pub updated_ms: u64,
    /// 展示标题（摘要优先，其次最后一条原文，截断 200 字）
    pub title: String,
    /// 关联的项目根目录（None = 未绑定项目，归「最近」）
    pub project_path: Option<String>,
}

/// 枚举全部有上下文台账的会话（扫描 `data_dir/context/*.json`，跳过快照目录）。
///
/// 会话在 `chat.send` 记录用户消息时即落盘台账 JSON（`record_turn`），但
/// `sessions.db` 只在整轮完整结束时才写入。中途关闭/引擎失败会造成
/// 「context JSON 在、sessions.list 无记录」的可见性丢失；本函数让列表
/// 数据源合并台账，避免会话重启后从侧边栏消失。
pub fn context_sessions() -> Vec<ContextSessionSummary> {
    let dir = Config::data_dir().join("context");
    let mut out: Vec<ContextSessionSummary> = Vec::new();
    let Ok(rd) = std::fs::read_dir(&dir) else {
        return out;
    };
    for entry in rd.flatten() {
        let path = entry.path();
        if !path.is_file() || path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let Ok(raw) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Ok(ctx) = serde_json::from_str::<SessionContext>(&raw) else {
            continue;
        };
        if ctx.entries.is_empty() {
            continue;
        }
        let last = ctx.entries.last();
        let updated_ms = last.map(|e| e.at_ms).unwrap_or(0);
        let title = ctx
            .summary
            .clone()
            .unwrap_or_else(|| last.map(|e| e.text.clone()).unwrap_or_default());
        out.push(ContextSessionSummary {
            session: ctx.session,
            message_count: ctx.entries.len(),
            updated_ms,
            title: clip(&title, 200),
            project_path: ctx.project_path,
        });
    }
    out.sort_by(|a, b| b.updated_ms.cmp(&a.updated_ms));
    out
}

/// `context.status`：会话上下文状态视图
pub fn context_status(session: &str) -> Value {
    let ctx = ledger()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .status(session);
    let ratio = ctx.ratio();
    json!({
        "session": ctx.session,
        "used_tokens": ctx.used_tokens(),
        "budget": ctx.budget,
        "ratio": ratio,
        "alert": if ratio >= COMPACT_RATIO { "compact_required" }
                 else if ratio >= WARN_RATIO { "warning" } else { "normal" },
        "compaction_count": ctx.compaction_count,
        "last_compact_ms": ctx.last_compact_ms,
        "last_mode": ctx.last_mode,
        "compression_ratio": ctx.compression_ratio,
        "snapshot_count": ctx.snapshot_count,
        "entries": ctx.entries.len(),
        "summary": ctx.summary,
    })
}

/// `context.compact`：确定性压缩（快速路径，chat 自动压缩使用）。
pub fn context_compact(session: &str, bus: Option<&EventBus>) -> Value {
    let compaction_id = format!("cmp-{}", uuid::Uuid::new_v4());
    if let Some(b) = bus {
        b.publish(Envelope::new(
            EventType::Compaction,
            json!({
                "status": "started",
                "phase": "summarize",
                "compaction_id": compaction_id,
                "compactionId": compaction_id,
                "session_key": session,
                "key": session,
            }),
            session,
        ));
    }
    compact_and_publish(session, bus, &compaction_id, None)
}

/// `context.compact`：LLM 语义压缩（手动/网关路径，LLM 优先、确定性回退）。
pub async fn context_compact_async(session: &str, bus: Option<&EventBus>) -> Value {
    let compaction_id = format!("cmp-{}", uuid::Uuid::new_v4());
    if let Some(b) = bus {
        b.publish(Envelope::new(
            EventType::Compaction,
            json!({
                "status": "started",
                "phase": "summarize",
                "compaction_id": compaction_id,
                "compactionId": compaction_id,
                "session_key": session,
                "key": session,
            }),
            session,
        ));
    }
    // 第 2 层：LLM 语义摘要（失败/无 key → None → 确定性回退）
    let llm_summary = {
        let entries = ledger()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .status(session)
            .entries;
        build_summary_llm(&entries).await
    };
    compact_and_publish(session, bus, &compaction_id, llm_summary)
}

/// 核心：执行渐进压缩并发布生命周期事件（LLM 摘要已由调用方预生成）。
fn compact_and_publish(
    session: &str,
    bus: Option<&EventBus>,
    compaction_id: &str,
    llm_summary: Option<String>,
) -> Value {
    let report = ledger().lock().unwrap_or_else(|e| e.into_inner()).compact(
        session,
        compaction_id,
        llm_summary,
    );
    match report {
        Some(r) => {
            if let Some(b) = bus {
                b.publish(Envelope::new(
                    EventType::Compaction,
                    json!({
                        "status": "completed",
                        "compacted": true,
                        "phase": "done",
                        "mode": r.mode,
                        "compression_ratio": r.compression_ratio,
                        "tokens_before": r.tokens_before,
                        "tokens_after": r.tokens_after,
                        "compaction_id": compaction_id,
                        "compactionId": compaction_id,
                        "summary": r.summary,
                        "session_key": session,
                        "key": session,
                        "safe_to_send": true,
                    }),
                    session,
                ));
            }
            json!({
                "compaction_id": compaction_id,
                "compactionId": compaction_id,
                "compacted": true,
                "mode": r.mode,
                "compression_ratio": r.compression_ratio,
                "tokens_before": r.tokens_before,
                "tokens_after": r.tokens_after,
                "summary": r.summary,
                "session_key": session,
            })
        }
        None => {
            if let Some(b) = bus {
                b.publish(Envelope::new(
                    EventType::Compaction,
                    json!({
                        "status": "skipped",
                        "compacted": false,
                        "phase": "done",
                        "compaction_id": compaction_id,
                        "compactionId": compaction_id,
                        "session_key": session,
                        "key": session,
                        "safe_to_send": true,
                    }),
                    session,
                ));
            }
            json!({
                "compaction_id": compaction_id,
                "compactionId": compaction_id,
                "compacted": false,
                "summary": Value::Null,
                "session_key": session,
            })
        }
    }
}

/// `context.rollback`：回滚到最近一次预压缩快照（`/compact undo`）。
pub fn context_rollback(session: &str) -> Value {
    let ok = ledger()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .rollback(session);
    json!({
        "rolled_back": ok,
        "session_key": session,
        "message": if ok { "已回滚到压缩前上下文" } else { "无可用快照（尚未压缩或快照已清空）" },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::persistence::config::Config;

    fn tmp_ledger(tag: &str) -> (ContextLedger, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("mgt-ctx-{tag}-{}", uuid::Uuid::new_v4()));
        (ContextLedger::new(&dir), dir)
    }

    #[test]
    fn test_record_and_status_ratio() {
        let (mut l, _d) = tmp_ledger("rec");
        l.record("s1", "user", "hello world");
        l.record("s1", "assistant", "hi there");
        let ctx = l.status("s1");
        assert_eq!(ctx.entries.len(), 2);
        assert!(ctx.used_tokens() >= 2);
        assert_eq!(ctx.session, "s1");
        // 未知会话返回空台账
        assert_eq!(l.status("none").entries.len(), 0);
    }

    #[test]
    fn test_compact_short_session_skips() {
        let (mut l, _d) = tmp_ledger("short");
        for i in 0..4 {
            l.record("s2", "user", &format!("msg {i}"));
        }
        assert!(l.compact("s2", "id", None).is_none(), "过短会话应跳过压缩");
        assert_eq!(l.status("s2").compaction_count, 0);
    }

    #[test]
    fn test_compact_keeps_tail_and_summary() {
        let (mut l, _d) = tmp_ledger("tail");
        // 20 轮 user/assistant 交替
        for i in 0..20 {
            l.record("s3", "user", &format!("关键任务：实现第 {i} 步功能"));
            l.record("s3", "assistant", &format!("下一步，继续第 {} 项", i + 1));
        }
        let report = l.compact("s3", "cmp-1", None).expect("长会话应压缩");
        assert!(report.summary.contains("## 会话摘要"));
        assert!(report.summary.contains("关键决策"));
        assert_eq!(report.mode, "deterministic");
        assert!(report.tokens_before > report.tokens_after);
        assert!(report.compression_ratio > 0.0);
        let ctx = l.status("s3");
        assert!(ctx.compaction_count >= 1);
        // 压缩后条目 = 摘要 1 + 保留最近 KEEP_RECENT_TURNS 条
        assert_eq!(ctx.entries.len(), 1 + KEEP_RECENT_TURNS);
        assert!(ctx.summary.is_some());
        assert_eq!(ctx.last_mode.as_deref(), Some("deterministic"));
        assert!(ctx.compression_ratio.unwrap_or(0.0) > 0.0);
        assert!(ctx.snapshot_count >= 1, "压缩应保存预压缩快照");
    }

    #[test]
    fn test_snip_long_folds_head_tail() {
        let long = "头".repeat(100) + &"中".repeat(900) + &"尾".repeat(100);
        let s = snip_long(&long);
        assert!(s.contains("头"), "应保留头部");
        assert!(s.contains("尾"), "应保留尾部");
        assert!(s.contains("省略"), "应标注省略");
        assert!(s.len() < long.len(), "折叠后应显著变短");
        // 短文本不折叠
        assert_eq!(snip_long("short"), "short");
    }

    #[test]
    fn test_context_sessions_enumerates_ledger_json() {
        // context_sessions 扫描全局 data_dir/context；写一个临时台账 JSON 验证枚举，
        // 断言后立即删除，不污染真实台账。
        let dir = Config::data_dir().join("context");
        let _ = std::fs::create_dir_all(&dir);
        let probe = format!("mgt-ctx-enum-{}", uuid::Uuid::new_v4());
        let path = dir.join(format!("{probe}.json"));
        let ctx = SessionContext {
            session: probe.clone(),
            entries: vec![
                ContextEntry {
                    role: "user".into(),
                    text: "帮我做一个 fps 射击游戏".into(),
                    tokens: 12,
                    at_ms: 1_700_000_000_000,
                },
                ContextEntry {
                    role: "assistant".into(),
                    text: "好的，开始构建".into(),
                    tokens: 8,
                    at_ms: 1_700_000_000_500,
                },
            ],
            budget: DEFAULT_BUDGET,
            compaction_count: 0,
            last_compact_ms: None,
            summary: None,
            last_mode: None,
            compression_ratio: None,
            snapshot_count: 0,
            project_path: None,
        };
        let write = std::fs::write(&path, serde_json::to_string(&ctx).unwrap());
        assert!(write.is_ok(), "临时台账写入失败");
        let summaries = context_sessions();
        let hit = summaries.iter().find(|s| s.session == probe);
        assert!(hit.is_some(), "context_sessions 应枚举到临时台账");
        let hit = hit.unwrap();
        assert_eq!(hit.message_count, 2);
        assert_eq!(hit.updated_ms, 1_700_000_000_500);
        assert!(hit.title.contains("好的"), "标题应取最后一条原文");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_rollback_restores_pre_compact() {
        let (mut l, _d) = tmp_ledger("rollback");
        for i in 0..20 {
            l.record("rb1", "user", &format!("任务 {i}"));
            l.record("rb1", "assistant", &format!("回复 {i}"));
        }
        let before = l.status("rb1").entries.len();
        let r = l.compact("rb1", "cmp-1", None).expect("应压缩");
        assert!(l.status("rb1").entries.len() < before);
        assert!(r.tokens_before > 0);
        // 回滚恢复
        assert!(l.rollback("rb1"), "应能回滚");
        let ctx = l.status("rb1");
        assert_eq!(ctx.entries.len(), before, "回滚后条目数应恢复");
        assert!(ctx.summary.is_none(), "回滚后摘要应清空");
        // 无快照时回滚失败
        let (mut l2, _d2) = tmp_ledger("noroll");
        assert!(!l2.rollback("none"), "无快照应返回 false");
    }

    #[test]
    fn test_compact_llm_summary_used_when_provided() {
        let (mut l, _d) = tmp_ledger("llmsum");
        for i in 0..20 {
            l.record("llm1", "user", &format!("任务 {i}"));
            l.record("llm1", "assistant", &format!("回复 {i}"));
        }
        let report = l
            .compact("llm1", "cmp-9", Some("## 会话摘要\nLLM 生成的摘要".into()))
            .expect("应压缩");
        assert_eq!(report.mode, "llm");
        assert!(report.summary.contains("LLM 生成的摘要"));
        assert_eq!(l.status("llm1").last_mode.as_deref(), Some("llm"));
    }

    #[test]
    fn test_estimate_tokens_never_zero() {
        assert_eq!(estimate_tokens(""), 1);
        assert_eq!(estimate_tokens("abc"), 1);
        assert!(estimate_tokens("abcdefghij") >= 3);
    }

    #[test]
    fn test_auto_compact_trigger_on_over_budget() {
        // 模拟 chat 链路：usage 越触发线（alert == compact_required）→ 自动压缩，压缩率可度量
        let (mut l, _d) = tmp_ledger("autocompact");
        // 注入小预算 + 超量 entries，使 ratio 越过 COMPACT_RATIO（0.85）
        let mut ctx = l.status("ac1");
        ctx.budget = 100;
        ctx.entries = (0..30)
            .map(|i| ContextEntry {
                role: if i % 2 == 0 { "user" } else { "assistant" }.into(),
                text: format!("关键任务 msg {i} {}", "x".repeat(20)),
                tokens: 40,
                at_ms: i as u64,
            })
            .collect();
        l.sessions.insert("ac1".into(), ctx);
        // 达到触发线：context_status 应判 alert == compact_required
        assert!(
            l.status("ac1").ratio() >= COMPACT_RATIO,
            "使用率应越过压缩触发线"
        );
        // 触发压缩（chat 链路 `if alert==compact_required { context_compact }`）
        let report = l.compact("ac1", "auto-1", None).expect("超预算应自动压缩");
        assert!(
            (0.0..=1.0).contains(&report.compression_ratio),
            "压缩率应在 [0,1]：{}",
            report.compression_ratio
        );
        let after = l.status("ac1");
        assert!(after.compaction_count >= 1);
        assert!(after.compression_ratio.is_some(), "压缩率应可度量");
        assert_eq!(after.last_mode.as_deref(), Some("deterministic"));
    }

    #[test]
    fn test_build_summary_extracts_sections() {
        let entries = vec![
            ContextEntry {
                role: "user".into(),
                text: "帮我重构登录模块并写测试".into(),
                tokens: 10,
                at_ms: 1,
            },
            ContextEntry {
                role: "assistant".into(),
                text: "决定采用 tokens.rs 方案".into(),
                tokens: 10,
                at_ms: 2,
            },
            ContextEntry {
                role: "user".into(),
                text: "下一步需要处理边界情况".into(),
                tokens: 10,
                at_ms: 3,
            },
            ContextEntry {
                role: "assistant".into(),
                text: "改动 src/auth.rs 完成".into(),
                tokens: 10,
                at_ms: 4,
            },
        ];
        let s = build_summary(&entries);
        assert!(s.contains("目标"));
        assert!(s.contains("采用 tokens.rs 方案"));
        assert!(s.contains("下一步需要处理边界情况"));
        assert!(s.contains("src/auth.rs"));
    }

    #[test]
    fn test_persist_roundtrip() {
        let (mut l, dir) = tmp_ledger("persist");
        l.record("p1", "user", "持久化测试");
        drop(l);
        let mut l2 = ContextLedger::new(&dir);
        let ctx = l2.status("p1");
        assert_eq!(ctx.entries.len(), 1);
        assert_eq!(ctx.entries[0].text, "持久化测试");
        let _ = Config::data_dir();
    }
}
