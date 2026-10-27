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

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::events::{Envelope, EventBus, EventType};
use crate::context::cache::SummaryCache;
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
/// 压缩跳过低水位：仅当条目数不多**且**用量占比低于此值时，才判定「无多少可回收」并跳过。
///
/// 旧实现只看条目数（`entries.len() <= KEEP_RECENT_TURNS * 2`）→ 少数几条巨大工具输出
/// （单条几十万 token）也永远不压缩，直到撞上 provider 上下文窗口。改为结合**真实 token
/// 压力**判断：低占比才是真正的「短/轻」；高占比即使条目很少也必须压缩。
pub const COMPACT_SKIP_RATIO: f64 = 0.5;
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
    /// 会话关联的全部项目目录（P2-9 多根；首项与 `project_path` 一致）。
    ///
    /// `#[serde(default)]` 兼容旧台账 JSON：缺字段 → 空 Vec，读取时回退
    /// `project_path` 单值（见 [`session_projects`]）。
    #[serde(default)]
    pub project_paths: Vec<String>,
    /// 流式生成中的「部分文本」（F1：turn 进行中增量持久化，
    /// 供前端切走再切回时恢复正在生成的消息占位，不丢流式文本）。
    #[serde(default)]
    pub partial_text: Option<String>,
    /// 非对话文本的 token 开销：**工具调用参数 + 工具输出**。
    ///
    /// 此前 `used_tokens()` 只统计 user/assistant 聊天文本，工具密集的长任务会被
    /// 严重低估 → `ratio()` 偏低 → 压缩触发过晚 → 撞上 provider 上下文窗口（HTTP 400）。
    /// 这里**只累计 token 数、不保存原文**，保证落盘有界；`#[serde(default)]` 兼容
    /// 既有磁盘 JSON（缺该字段时反序列化为 0）。
    ///
    /// 口径边界（2026-10-24 核实，避免注释再次漂移）：这里的账只有一个来源——
    /// **工具文本真正进入模型上下文**（工具调用入参 + `Message::Tool.output`）。
    /// 写入点：
    /// - `agent::stream_loop`（真流式回路，完整输出）与 `agent::runtime`
    ///   （非流式回路，完整入参 + 输出）—— 主路径（在 `Message::Tool` 入历史处同步记）；
    /// - `gateway::chat` 的事件桥与 `plan::runner` 的 `LedgerTap` —— 工具结果**兜底**
    ///   （引擎侧已按同一 `call_id` 记过则被幂等键跳过）；流式路径的**调用入参**
    ///   目前由事件桥记一次；
    /// - `agent::multi_strategy`（多策略工具回路，不发事件，只能自记）。
    ///
    /// **工具 schema 与系统提示不计**：它们是每轮恒定的基线，既不随对话增长、
    /// 也不由本台账的「累计」语义表达；若需要把它们纳入，应作为 `used_tokens()`
    /// 之外的独立基线项，而不是累加进这里。
    #[serde(default)]
    pub overhead_tokens: u64,
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
            project_paths: Vec::new(),
            partial_text: None,
            overhead_tokens: 0,
        }
    }

    /// 绑定/更新项目根目录
    fn set_project(&mut self, project: &str) {
        if self.project_path.as_deref() != Some(project) {
            self.project_path = Some(project.to_string());
        }
    }

    /// 绑定会话的全部项目目录（P2-9 多根）：去重保序，首个同步为 `project_path`。
    /// 空列表视为无操作——不得把已有归属清空。
    fn set_projects(&mut self, projects: &[String]) {
        let normalized = normalize_projects(projects);
        if normalized.is_empty() {
            return;
        }
        self.project_path = normalized.first().cloned();
        self.project_paths = normalized;
    }

    /// 读取项目目录集合：`project_paths` 优先（去重保序），空则回退旧字段
    /// `project_path` 单值；都空 → 空 Vec。
    fn projects(&self) -> Vec<String> {
        projects_of(&self.project_paths, self.project_path.as_deref())
    }

    /// 当前 token 用量（对话条目 + 非对话开销）
    pub fn used_tokens(&self) -> u64 {
        self.entries.iter().map(|e| e.tokens).sum::<u64>() + self.overhead_tokens
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
    /// 每会话最近记账过的工具文本幂等键（**仅内存**，不落盘）。
    ///
    /// 同一工具结果可能被两条路径看到：引擎侧（文本真正写进 `Message::Tool` 时）
    /// 与网关事件桥（`ToolResult` 事件回流兜底）。谁先记都行，但只能记一次 ——
    /// 这里按 `call_id`（缺 id 时按「工具名 + 内容 hash」）去重。
    recent_tool_keys: HashMap<String, RecentToolKeys>,
}

/// 每会话保留的幂等键上限（超出按 FIFO 淘汰，内存有界）。
const RECENT_TOOL_KEYS_MAX: usize = 512;

/// 有界的幂等键集合（FIFO 淘汰）。
#[derive(Default)]
struct RecentToolKeys {
    order: VecDeque<String>,
    set: HashSet<String>,
}

impl RecentToolKeys {
    fn contains(&self, key: &str) -> bool {
        self.set.contains(key)
    }

    fn insert(&mut self, key: String) {
        if self.set.insert(key.clone()) {
            self.order.push_back(key);
            while self.order.len() > RECENT_TOOL_KEYS_MAX {
                if let Some(old) = self.order.pop_front() {
                    self.set.remove(&old);
                }
            }
        }
    }
}

impl ContextLedger {
    fn new(data_dir: &std::path::Path) -> Self {
        let dir = data_dir.join("context");
        let _ = std::fs::create_dir_all(&dir);
        let sessions = HashMap::new();
        Self {
            dir,
            sessions,
            recent_tool_keys: HashMap::new(),
        }
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

    /// 落盘（原子写，走 `persistence::write_json_atomic`）。
    ///
    /// R13（2026-10-18）：此前 `std::fs::write` 直写——该 API 先截断再写。
    /// 这是**全仓最高频**的台账写入（每轮对话写两次：用户消息 + 助手回复），
    /// 一旦在写入途中被杀/断电，文件留半截 JSON，而 `load()` 的兜底是
    /// `SessionContext::new(session)`（**整份归零**）。用户可见后果是：
    /// 聊了几十轮后某次崩溃重开 → 上下文全丢，`memory_search` / 压缩 / 令牌
    /// 计量同时失去依据，且界面看不出「曾经有过历史」。
    /// 改原子写后，读者要么看到旧完整内容、要么看到新完整内容。
    fn persist(&self, ctx: &SessionContext) {
        if let Ok(raw) = serde_json::to_string_pretty(ctx) {
            let _ = crate::persistence::write_json_atomic(&self.path(&ctx.session), &raw);
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

    /// 累计非对话文本的 token 开销（工具输出 / schema / 系统提示），返回更新后的用量占比。
    ///
    /// 只保存 token 数（不保存原文），因此落盘与内存都有界；饱和加法防止溢出。
    fn record_overhead(&mut self, session: &str, tokens: u64) -> f64 {
        let mut ctx = self.load(session);
        ctx.overhead_tokens = ctx.overhead_tokens.saturating_add(tokens);
        let ratio = ctx.ratio();
        self.persist(&ctx);
        self.sessions.insert(session.to_string(), ctx);
        ratio
    }

    /// 带幂等键的工具文本记账。
    ///
    /// - `dedupe = true`（事件桥兜底）：键已记过 → 直接返回当前占比，不再累加；
    /// - `dedupe = false`（引擎侧主路径）：**必定**累加（同一工具产生相同文本的
    ///   多次真实调用各算一次），但同样登记键 —— 引擎在发送 `ToolResult` 事件
    ///   **之前**调用，故桥上后到的同一条结果一定能看到键并跳过。
    fn record_tool_overhead_keyed(
        &mut self,
        session: &str,
        key: Option<String>,
        tokens: u64,
        dedupe: bool,
    ) -> f64 {
        if let Some(k) = key {
            let already = {
                let seen = self
                    .recent_tool_keys
                    .entry(session.to_string())
                    .or_default();
                if dedupe && seen.contains(&k) {
                    true
                } else {
                    seen.insert(k);
                    false
                }
            };
            if already {
                return self.load(session).ratio();
            }
        }
        self.record_overhead(session, tokens)
    }

    /// 读取会话状态
    fn status(&mut self, session: &str) -> SessionContext {
        self.load(session)
    }

    /// 写入会话的全部项目目录（P2-9；原子落盘，语义同 [`Self::persist`]）。
    fn set_projects(&mut self, session: &str, projects: &[String]) {
        let mut ctx = self.load(session);
        ctx.set_projects(projects);
        self.persist(&ctx);
        self.sessions.insert(session.to_string(), ctx);
    }

    /// 读取会话的项目目录集合（多值优先，旧单值字段回退）。
    fn projects(&mut self, session: &str) -> Vec<String> {
        self.status(session).projects()
    }

    /// 列出目录下的快照文件，按**修改时间**升序（最旧在前）。
    ///
    /// ⚠️ 不能按文件名排序：快照名是 `cmp-<uuid>`，字典序与时间无关 —— 历史缺陷
    /// 让「清理最旧」与「回滚最新」都作用在**随机**一份上（`/compact undo` 可能
    /// 恢复到更早或更晚的状态）。按 mtime 排序才是真正的时间序。
    fn snapshot_files_by_mtime(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
        let mut files: Vec<(std::time::SystemTime, std::path::PathBuf)> =
            std::fs::read_dir(dir)
                .map(|rd| {
                    rd.flatten()
                        .filter(|e| {
                            e.path().extension().map(|x| x == "json").unwrap_or(false)
                        })
                        .map(|e| {
                            let t = e
                                .metadata()
                                .and_then(|m| m.modified())
                                .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
                            (t, e.path())
                        })
                        .collect()
                })
                .unwrap_or_default();
        files.sort_by_key(|a| a.0);
        files.into_iter().map(|(_, p)| p).collect()
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
        // 清理：只保留最近 5 份（按 mtime 判新旧，见 snapshot_files_by_mtime）
        let files = Self::snapshot_files_by_mtime(&dir);
        if files.len() > 5 {
            for oldest in files.iter().take(files.len() - 5) {
                let _ = std::fs::remove_file(oldest);
            }
        }
    }

    /// 恢复最近一次预压缩快照；成功返回 true
    fn rollback(&mut self, session: &str) -> bool {
        let mut ctx = self.load(session);
        let dir = self.snapshots_dir(session);
        let files = Self::snapshot_files_by_mtime(&dir);
        let Some(latest) = files.last() else {
            return false;
        };
        let Ok(raw) = std::fs::read_to_string(latest) else {
            return false;
        };
        let Ok(restored) = serde_json::from_str::<SessionContext>(&raw) else {
            return false;
        };
        let _ = std::fs::remove_file(latest); // 回滚一次即消费一份快照
        ctx.entries = restored.entries;
        ctx.summary = None; // 摘要回到压缩前状态
        ctx.last_mode = None;
        ctx.compression_ratio = None;
        // 让计数与**磁盘上真实剩余**的快照数一致，避免 status 报告不存在的快照。
        ctx.snapshot_count = Self::snapshot_files_by_mtime(&dir).len() as u32;
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
        // 跳过条件必须基于**真实 token 压力**，而非仅条目数：条目少但 token 压力高
        // （少数几条巨大工具输出）同样必须压缩，否则会撞上 provider 上下文窗口。
        if ctx.entries.len() <= KEEP_RECENT_TURNS * 2 && ctx.ratio() < COMPACT_SKIP_RATIO {
            return None; // 条目少且压力低 → 无多少可回收，跳过
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
        // 第 2 层：语义摘要（LLM 优先，确定性回退）——经内容 hash 缓存复用，
        // 相同内容第二次压缩不再重复计算（`SummaryCache` 生产接线）。
        let (summary, mode) = {
            let mut cache = summary_cache().lock().unwrap_or_else(|e| e.into_inner());
            select_summary(&mut cache, &ctx.entries, &folded, llm_summary)
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
        wire_api: m.wire_api,
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

/// 摘要缓存上限（条目数）与 TTL（秒）。
const SUMMARY_CACHE_MAX: usize = 256;
const SUMMARY_CACHE_TTL_SECS: u64 = 3600;

/// 进程级摘要缓存单例（内容 hash → 摘要）。
///
/// 压缩时按内容复用摘要，避免对同一内容重复调用 LLM 或重复跑确定性抽取。
fn summary_cache() -> &'static Mutex<SummaryCache> {
    static CACHE: OnceLock<Mutex<SummaryCache>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(SummaryCache::new(SUMMARY_CACHE_MAX, SUMMARY_CACHE_TTL_SECS)))
}

/// 摘要内容键：`mode`（llm / deterministic）+ 每条 (role, `snip_long(text)`) 的规范串 hash。
///
/// 键必须在**原始条目**上取（函数内部对文本做 `snip_long`）：[`compact`] 传入的 `folded`
/// 已是折叠结果，而 [`build_summary_llm`] 也以原始条目为输入（内部折叠）——对原始条目
/// 折叠后的规范串，两处天然一致，缓存方能真正命中。
fn summary_key(mode: &str, entries: &[ContextEntry]) -> u64 {
    let mut s = String::new();
    s.push_str(mode);
    s.push('\u{1f}');
    for e in entries {
        s.push_str(&e.role);
        s.push('\u{1e}');
        s.push_str(&snip_long(&e.text));
        s.push('\u{1d}');
    }
    SummaryCache::content_hash(&s)
}

/// 选择摘要（LLM 优先、确定性回退），经给定缓存按内容复用。返回 `(summary, mode)`。
///
/// `raw` 用于取内容键（见 [`summary_key`]），`folded` 供确定性摘要使用。
fn select_summary(
    cache: &mut SummaryCache,
    raw: &[ContextEntry],
    folded: &[ContextEntry],
    llm_summary: Option<String>,
) -> (String, &'static str) {
    let llm = llm_summary.filter(|s| !s.trim().is_empty());
    let mode: &'static str = if llm.is_some() { "llm" } else { "deterministic" };
    let key = summary_key(mode, raw);
    if let Some(hit) = cache.get(key) {
        return (hit, mode);
    }
    let summary = match llm {
        Some(s) => s,
        None => build_summary(folded),
    };
    cache.put(key, summary.clone());
    (summary, mode)
}

/// 带缓存的 LLM 摘要生成：先按内容 hash 查缓存，未命中才调用 LLM 并回填。
///
/// LLM 摘要在 [`compact`] 之外生成（异步调用点），故缓存查询必须放在这里才能真正跳过
/// 重复的 LLM 调用；[`compact`] 内部再用同一内容键复用（双保险，键一致）。
async fn cached_llm_summary(entries: &[ContextEntry]) -> Option<String> {
    let key = summary_key("llm", entries);
    let cached = {
        let mut c = summary_cache().lock().unwrap_or_else(|e| e.into_inner());
        c.get(key)
    };
    if let Some(hit) = cached {
        return Some(hit);
    }
    let summary = build_summary_llm(entries).await;
    if let Some(s) = &summary {
        let mut c = summary_cache().lock().unwrap_or_else(|e| e.into_inner());
        c.put(key, s.clone());
    }
    summary
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

/// `context.record_overhead`：计入非对话文本（工具输出 / schema / 系统提示）的 token 开销。
///
/// 返回更新后的用量占比；只累计 token 数，不保存原文。
pub fn record_overhead(session: &str, tokens: u64) -> f64 {
    ledger()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .record_overhead(session, tokens)
}

/// `context.record_tool_output`：把一次工具输出估算为 token 并计入 overhead。
///
/// 复用 [`estimate_tokens`]；**只累加 token 数、不保存原文**，故对超长输出天然有界。
/// 空/纯空白输出不计（返回当前占比）。
pub fn record_tool_output(session: &str, tool: &str, output: &str) -> f64 {
    if output.trim().is_empty() {
        return ledger()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .status(session)
            .ratio();
    }
    let tokens = estimate_tokens(output);
    tracing::debug!(session = %session, tool = %tool, tokens, "记录工具输出 token 开销");
    record_overhead(session, tokens)
}

/// 工具文本的幂等键（引擎侧与事件桥按同一键去重）。
///
/// `call_id` 优先：两边拿到的是同一个模型调用 id，与文本序列化格式无关；
/// 缺 id 时（部分本地模型的 `tool_calls` 不带 id）退化为「工具名 + 内容 hash」。
fn tool_record_key(call_id: Option<&str>, slot: &str, tool: &str, text: &str) -> String {
    match call_id.map(str::trim).filter(|s| !s.is_empty()) {
        Some(id) => format!("{slot}:id:{id}"),
        None => format!(
            "{slot}:txt:{tool}:{:016x}",
            SummaryCache::content_hash(text)
        ),
    }
}

/// 引擎侧：工具输出**真正写入模型上下文**（`Message::Tool.output`）时调用。
///
/// 与 [`record_tool_output`] 的区别是带 `call_id` 幂等键：**必定**累加本次文本
/// （同一工具产生相同文本的多次真实调用各算一次），同时登记键，使随后到达的
/// 网关 `ToolResult` 事件（见 [`record_tool_output_event_fallback`]）不会重复计数。
pub fn record_tool_output_at_context(
    session: &str,
    call_id: Option<&str>,
    tool: &str,
    output: &str,
) -> f64 {
    if output.trim().is_empty() {
        return ledger()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .status(session)
            .ratio();
    }
    let tokens = estimate_tokens(output);
    let key = tool_record_key(call_id, "out", tool, output);
    tracing::debug!(
        session = %session, tool = %tool, tokens, call_id = ?call_id,
        "工具输出进入模型上下文，计入台账"
    );
    ledger()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .record_tool_overhead_keyed(session, Some(key), tokens, false)
}

/// 引擎侧：工具**入参**进入模型上下文（与输出共用同一套幂等键，slot=`in`）。
///
/// 与 [`record_tool_output_at_context`] 的区别只在幂等槽位：入参与输出是同一
/// `call_id` 的两段不同文本，必须分别登记，否则「入参已记 → 输出被误跳过」。
pub fn record_tool_input_at_context(
    session: &str,
    call_id: Option<&str>,
    tool: &str,
    input: &str,
) -> f64 {
    if input.trim().is_empty() {
        return ledger()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .status(session)
            .ratio();
    }
    let tokens = estimate_tokens(input);
    let key = tool_record_key(call_id, "in", tool, input);
    ledger()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .record_tool_overhead_keyed(session, Some(key), tokens, false)
}

/// 事件桥：同一条**入参**事件若引擎侧已记过 → 跳过（与输出同一套去重语义）。
pub fn record_tool_input_event_fallback(
    session: &str,
    call_id: Option<&str>,
    tool: &str,
    input: &str,
) -> f64 {
    if input.trim().is_empty() {
        return ledger()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .status(session)
            .ratio();
    }
    let tokens = estimate_tokens(input);
    let key = tool_record_key(call_id, "in", tool, input);
    ledger()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .record_tool_overhead_keyed(session, Some(key), tokens, true)
}

/// 网关事件桥兜底：同一工具结果若已由引擎侧（或前一条事件）记过 → 跳过。
///
/// 只在**没有引擎记账**的路径（如 `Envelope::tool_result` 只带摘要的链路/工作台）
/// 真正生效；它不替代引擎侧记账，只是补漏。失败结果（`ok:false`）同样按实际
/// 事件文本计，不因失败跳过。
pub fn record_tool_output_event_fallback(
    session: &str,
    call_id: Option<&str>,
    tool: &str,
    text: &str,
) -> f64 {
    if text.trim().is_empty() {
        return ledger()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .status(session)
            .ratio();
    }
    let tokens = estimate_tokens(text);
    let key = tool_record_key(call_id, "out", tool, text);
    tracing::debug!(
        session = %session, tool = %tool, tokens, call_id = ?call_id,
        "事件桥兜底记录工具结果 token 开销"
    );
    ledger()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .record_tool_overhead_keyed(session, Some(key), tokens, true)
}

/// 增量持久化流式「部分文本」（F1）：turn 进行中调用，切走再切回可恢复。
pub fn record_partial(session: &str, text: &str) {
    let mut g = ledger().lock().unwrap_or_else(|e| e.into_inner());
    let mut ctx = g.load(session);
    ctx.partial_text = if text.is_empty() {
        None
    } else {
        Some(text.to_string())
    };
    g.persist(&ctx);
    g.sessions.insert(session.to_string(), ctx);
}

/// 读取会话的流式部分文本（无则 None）。
pub fn partial_text(session: &str) -> Option<String> {
    ledger()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .status(session)
        .partial_text
        .clone()
}

/// 清除部分文本（turn 完成后调用，避免残留占位）。
pub fn clear_partial(session: &str) {
    let mut g = ledger().lock().unwrap_or_else(|e| e.into_inner());
    let mut ctx = g.load(session);
    ctx.partial_text = None;
    g.persist(&ctx);
    g.sessions.insert(session.to_string(), ctx);
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

/// 最后一条台账条目的时间戳（R70：供逐条落盘时取与台账一致的毫秒时间）。
///
/// 与 `context_entries(session).last()` 等价，但**不复制整段条目** —— 每轮对话
/// 都会调用，长会话（数千条）下避免一次 O(n) 克隆。
pub fn last_entry_at_ms(session: &str) -> Option<u64> {
    ledger()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .status(session)
        .entries
        .last()
        .map(|e| e.at_ms)
}

/// 项目目录归一化：去空白、丢空串、去重（保持首次出现顺序）。
fn normalize_projects(projects: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::with_capacity(projects.len());
    for p in projects {
        let p = p.trim();
        if !p.is_empty() && !out.iter().any(|x| x == p) {
            out.push(p.to_string());
        }
    }
    out
}

/// 读取口径：`project_paths` 优先；为空则回退旧字段 `project_path` 单值。
fn projects_of(project_paths: &[String], project_path: Option<&str>) -> Vec<String> {
    let paths = normalize_projects(project_paths);
    if !paths.is_empty() {
        return paths;
    }
    project_path
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| vec![s.to_string()])
        .unwrap_or_default()
}

/// 绑定会话的全部项目目录（P2-9 多根；跨重启持久化到台账 JSON）。
///
/// 与 [`bind_session_project`] 的区别：这里是**集合**语义（去重保序），并把
/// `project_path` 同步为第一个，保证旧读取方（单值 API / 旧前端）行为不变。
/// 空列表为无操作，不会清空已有归属。
pub fn bind_session_projects(session: &str, projects: &[String]) {
    ledger()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .set_projects(session, projects);
}

/// 读取会话绑定的全部项目目录（P2-9）。
///
/// 优先返回 `project_paths`；为空时回退旧字段 `project_path`（`vec![p]`）；
/// 都空返回空 Vec。
pub fn session_projects(session: &str) -> Vec<String> {
    ledger()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .projects(session)
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
    /// 关联的全部项目目录（P2-9 多根；首项与 `project_path` 一致，空 = 未绑定）
    pub project_paths: Vec<String>,
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
        // 先按「多根优先、旧单值回退」取值，再把 `project_path` 移入结构体。
        let project_paths = ctx.projects();
        out.push(ContextSessionSummary {
            session: ctx.session,
            message_count: ctx.entries.len(),
            updated_ms,
            title: clip(&title, 200),
            project_path: ctx.project_path,
            project_paths,
        });
    }
    out.sort_by_key(|cs| std::cmp::Reverse(cs.updated_ms));
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
        "overhead_tokens": ctx.overhead_tokens,
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
        // F1：暴露流式 partial_text，供前端切回时恢复「正在生成」占位
        "partial_text": ctx.partial_text,
    })
}

/// 由模型上下文窗口推导台账预算：预留 25% 给输出与系统提示，
/// 并钳制在 `[32K, 1.5M]`（小窗口不能用不可用的低预算，异常配置不能抬到无穷）。
///
/// 历史缺陷：预算曾是固定 128K —— 1M 窗口的旗舰模型在 ~109K 就被判定
/// `compact_required`，长任务关键中间结论被有损摘要掉（表现即「越聊越差」）；
/// 小窗口模型又压得太晚，直接撞 provider 400。
pub fn budget_for_window(context_window: u64) -> u64 {
    if context_window == 0 {
        return DEFAULT_BUDGET;
    }
    ((context_window * 3) / 4).clamp(32_000, 1_500_000)
}

/// 按当前生效模型窗口设置本会话上下文预算（写台账并持久化）。
pub fn set_budget(session: &str, context_window: u64) -> u64 {
    let budget = budget_for_window(context_window);
    if context_window == 0 {
        return budget;
    }
    let mut l = ledger().lock().unwrap_or_else(|e| e.into_inner());
    let mut ctx = l.load(session);
    if ctx.budget != budget {
        ctx.budget = budget;
        l.persist(&ctx);
        l.sessions.insert(session.to_string(), ctx);
    }
    budget
}

/// `context.compact`：确定性压缩（快速路径，chat 自动压缩使用）。
pub fn context_compact(session: &str, bus: Option<&EventBus>) -> Value {    let compaction_id = format!("cmp-{}", uuid::Uuid::new_v4());
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
    // 第 2 层：LLM 语义摘要（失败/无 key → None → 确定性回退）。
    // 先按内容 hash 查缓存，命中则直接复用、跳过 LLM 调用。
    let llm_summary = {
        let entries = ledger()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .status(session)
            .entries;
        cached_llm_summary(&entries).await
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

    /// P2-9：多路径写入后按原序读出；只有旧字段时回退为单值。
    #[test]
    fn session_projects_multi_roundtrip_and_legacy_fallback() {
        let (mut l, dir) = tmp_ledger("projects");
        l.set_projects(
            "s-multi",
            &[
                "D:/proj/a".to_string(),
                "D:/proj/b".to_string(),
                "D:/proj/a".to_string(), // 重复 → 丢弃
            ],
        );
        assert_eq!(
            l.projects("s-multi"),
            vec!["D:/proj/a".to_string(), "D:/proj/b".to_string()],
            "多值必须去重且保持写入顺序"
        );
        assert_eq!(
            l.status("s-multi").project_path.as_deref(),
            Some("D:/proj/a"),
            "旧单值字段必须同步为第一个"
        );

        // 旧台账只有 project_path 字段 → 回退单值
        let mut legacy = SessionContext::new("s-legacy");
        legacy.project_path = Some("D:/proj/only".to_string());
        std::fs::write(
            l.path("s-legacy"),
            serde_json::to_string(&legacy).unwrap(),
        )
        .unwrap();
        assert_eq!(
            l.projects("s-legacy"),
            vec!["D:/proj/only".to_string()],
            "旧字段必须回退为单值"
        );

        // 两者都空 → 空 Vec
        assert!(l.projects("s-none").is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn budget_follows_model_window_with_output_reserve() {
        // 1M 窗口：预留 25% 输出空间，而不是固定 128K 提早丢历史
        assert_eq!(budget_for_window(1_000_000), 750_000);
        // 均衡档 64K：预算 48K
        assert_eq!(budget_for_window(65_536), 49_152);
        // Turbo 16K：不可压到不可用，钳到 32K 下限
        assert_eq!(budget_for_window(16_384), 32_000);
        // 未声明窗口：保持历史默认
        assert_eq!(budget_for_window(0), DEFAULT_BUDGET);
        // 异常超大窗口：上限防呆
        assert_eq!(budget_for_window(10_000_000), 1_500_000);
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

    /// 缺陷回归（HIGH）：条目数少但 token 压力极高（几条巨大工具输出）**必须**压缩。
    ///
    /// 旧实现只看 `entries.len() <= KEEP_RECENT_TURNS * 2`，少数巨型条目被永久跳过，
    /// 直到撞上 provider 上下文窗口。此处把「按真实 token 压力判定」钉死。
    #[test]
    fn test_compact_few_entries_high_ratio_compacts() {
        let (mut l, _d) = tmp_ledger("fewhuge");
        let mut ctx = l.status("fh1");
        ctx.budget = 1000; // 小预算使 ratio 越过 COMPACT_SKIP_RATIO
        ctx.entries = (0..4)
            .map(|i| ContextEntry {
                role: if i % 2 == 0 { "user" } else { "assistant" }.into(),
                text: format!("巨大工具输出 {i} {}", "x".repeat(200)),
                tokens: 400,
                at_ms: i as u64,
            })
            .collect();
        l.sessions.insert("fh1".into(), ctx);
        assert!(
            l.status("fh1").ratio() >= COMPACT_SKIP_RATIO,
            "构造的少数大条目应处于高 token 压力"
        );
        let report = l.compact("fh1", "cmp-fh", None);
        assert!(
            report.is_some(),
            "条目少但高 token 压力必须触发压缩（缺陷回归）"
        );
        assert!(l.status("fh1").compaction_count >= 1);
    }

    /// 反向守卫：条目不多且 token 压力低（小消息、低 ratio）→ 仍应跳过，不做无谓工作。
    #[test]
    fn test_compact_many_tiny_entries_low_ratio_skips() {
        let (mut l, _d) = tmp_ledger("manytiny");
        let mut ctx = l.status("mt1");
        ctx.budget = DEFAULT_BUDGET; // 128k，条目虽多但 token 极小
        ctx.entries = (0..KEEP_RECENT_TURNS * 2)
            .map(|i| ContextEntry {
                role: "user".into(),
                text: format!("hi {i}"),
                tokens: 2,
                at_ms: i as u64,
            })
            .collect();
        l.sessions.insert("mt1".into(), ctx);
        assert!(
            l.status("mt1").ratio() < COMPACT_SKIP_RATIO,
            "小消息应处于低 token 压力"
        );
        assert!(
            l.compact("mt1", "cmp-mt", None).is_none(),
            "低压力下不应压缩（无谓工作）"
        );
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
        // E13：压缩必须覆盖**可见历史的开头**，不能只压中间一段。
        // 第 0 步只存在于最早那条 user 消息里；摘要里出现它，说明压缩窗口
        // 是从会话起点算起的，而不是从某个保留锚点之后才开始的。
        assert!(
            report.summary.contains("第 0 步"),
            "摘要必须覆盖最早的可见历史：{}",
            report.summary
        );
        let ctx = l.status("s3");
        assert!(ctx.compaction_count >= 1);
        // 压缩后条目 = 摘要 1 + 保留最近 KEEP_RECENT_TURNS 条
        assert_eq!(ctx.entries.len(), 1 + KEEP_RECENT_TURNS);
        // 最近一轮原文必须原样保留；被压掉的早期条目不得再以原文出现。
        let tail: Vec<&str> = ctx.entries.iter().skip(1).map(|e| e.text.as_str()).collect();
        assert!(
            tail.iter().any(|t| t.contains("第 19 步")),
            "最近条目必须原样保留：{tail:?}"
        );
        assert!(
            !tail.iter().any(|t| t.contains("第 0 步")),
            "已压缩的早期条目不应再以原文出现：{tail:?}"
        );
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
            project_paths: Vec::new(),
            partial_text: None,
            overhead_tokens: 0,
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

    /// 缺陷回归（MEDIUM）：相同内容的第二次压缩必须复用 `SummaryCache` 缓存的摘要，
    /// 不再重复跑确定性/LLM 摘要计算（内容 hash 键）。
    #[test]
    fn test_summary_cache_reuses_second_compaction() {
        let raw = vec![
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
        ];
        let folded: Vec<ContextEntry> = raw
            .iter()
            .map(|e| ContextEntry {
                role: e.role.clone(),
                text: snip_long(&e.text),
                tokens: e.tokens,
                at_ms: e.at_ms,
            })
            .collect();
        let mut cache = SummaryCache::new(16, 60);
        let (s1, m1) = select_summary(&mut cache, &raw, &folded, None);
        assert_eq!(m1, "deterministic");
        assert_eq!(cache.stats.misses, 1, "首次应未命中并写入");
        let (s2, m2) = select_summary(&mut cache, &raw, &folded, None);
        assert_eq!(m2, "deterministic");
        assert_eq!(s2, s1, "相同内容的第二次压缩必须复用缓存摘要");
        assert_eq!(cache.stats.hits, 1, "第二次必须命中缓存");
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

    /// 删除全局台账中的临时会话文件，避免污染真实 data_dir（in-memory 条目无碍）。
    fn cleanup_ledger_file(session: &str) {
        let path = Config::data_dir()
            .join("context")
            .join(format!("{session}.json"));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn test_overhead_raises_used_and_ratio() {
        let (mut l, _d) = tmp_ledger("overhead");
        l.record("ov1", "user", "hello world");
        let before = l.status("ov1");
        let used_before = before.used_tokens();
        let ratio_before = before.ratio();
        let r = l.record_overhead("ov1", 500);
        let after = l.status("ov1");
        assert_eq!(after.overhead_tokens, 500, "overhead 应被累计");
        assert!(
            after.used_tokens() > used_before,
            "overhead 应抬升 used_tokens：{used_before} -> {}",
            after.used_tokens()
        );
        assert!(
            after.ratio() > ratio_before,
            "overhead 应抬升 ratio：{ratio_before} -> {}",
            after.ratio()
        );
        assert!(
            (r - after.ratio()).abs() < 1e-12,
            "record_overhead 返回值应等于最新 ratio"
        );
        // 饱和累加：再次叠加应继续增长
        l.record_overhead("ov1", 1000);
        assert_eq!(l.status("ov1").overhead_tokens, 1500);
    }

    #[test]
    fn test_record_tool_output_accounts_large_output() {
        // 走全局台账：record_tool_output 是唯一对外的写入入口，context_status 读取同一单例。
        let session = format!("mgt-ctx-tool-{}", uuid::Uuid::new_v4());
        let big = "x".repeat(40_000); // 拉丁字符 ≈ 10k token，远超普通对话
        let expected = estimate_tokens(&big);
        assert!(expected > 5_000, "大工具输出应估算出大量 token：{expected}");
        let ratio = record_tool_output(&session, "shell", &big);
        let status = context_status(&session);
        assert_eq!(
            status["overhead_tokens"].as_u64(),
            Some(expected),
            "大工具输出应完整计入 overhead"
        );
        assert_eq!(status["used_tokens"].as_u64(), Some(expected));
        assert!(ratio > 0.0);
        // 空/纯空白输出不计（返回当前占比，overhead 不变）
        let r0 = record_tool_output(&session, "shell", "   ");
        assert_eq!(
            context_status(&session)["overhead_tokens"].as_u64(),
            Some(expected),
            "空输出不应改变 overhead"
        );
        assert!((r0 - ratio).abs() < 1e-12);
        cleanup_ledger_file(&session);
    }

    /// E12 回归：工具输出**真正进入模型上下文**时按完整文本记账，事件桥随后收到
    /// 同一条结果（按同一 `call_id`）不得重复计数。
    #[test]
    fn test_record_tool_output_at_context_counts_full_once() {
        let session = format!("mgt-ctx-e12-full-{}", uuid::Uuid::new_v4());
        let big = "x".repeat(40_000); // 拉丁字符 ≈ 10k token
        let expected = estimate_tokens(&big);
        assert!(expected > 5_000, "40k 字符应估算出大量 token：{expected}");

        // 引擎侧：完整输出进上下文 → 计满
        record_tool_output_at_context(&session, Some("call-1"), "read_file", &big);
        assert_eq!(
            context_status(&session)["overhead_tokens"].as_u64(),
            Some(expected),
            "完整输出必须计满（而不是摘要的个位数）"
        );

        // 事件桥兜底：同一 call_id 的同一条结果 → 跳过（即便事件只带摘要）
        record_tool_output_event_fallback(&session, Some("call-1"), "read_file", "ok");
        assert_eq!(
            context_status(&session)["overhead_tokens"].as_u64(),
            Some(expected),
            "同一工具结果被引擎与事件桥各看到一次时只能记一次"
        );

        // 另一次真实调用（不同 call_id）即使输出完全相同，也必须各记一次
        record_tool_output_at_context(&session, Some("call-2"), "read_file", &big);
        assert_eq!(
            context_status(&session)["overhead_tokens"].as_u64(),
            Some(expected * 2),
            "不同调用 id 的相同输出是两次真实上下文开销"
        );

        let ratio = context_status(&session)["ratio"].as_f64().unwrap_or(0.0);
        assert!(ratio > 0.1, "40k×2 字符输出应显著抬升 ratio：{ratio}");
        cleanup_ledger_file(&session);
    }

    /// 入参与输出共用同一 `call_id` 但必须分别记账：槽位不同（`in` / `out`），
    /// 否则「入参已记 → 输出被误判重复而丢掉」。事件桥重复收到时两边都要跳过。
    #[test]
    fn test_tool_input_and_output_dedupe_independently() {
        let session = format!("mgt-ctx-inout-{}", uuid::Uuid::new_v4());
        let input = r#"{"path":"src/main.rs"}"#;
        let output = "file body";
        let expected = estimate_tokens(input) + estimate_tokens(output);

        record_tool_input_at_context(&session, Some("call-1"), "read_file", input);
        record_tool_output_at_context(&session, Some("call-1"), "read_file", output);
        assert_eq!(
            context_status(&session)["overhead_tokens"].as_u64(),
            Some(expected),
            "入参与输出必须各记一次，不能互相吞掉"
        );

        // 网关桥/计划 tap 随后看到同两条事件 → 都跳过
        record_tool_input_event_fallback(&session, Some("call-1"), "read_file", input);
        record_tool_output_event_fallback(&session, Some("call-1"), "read_file", output);
        assert_eq!(
            context_status(&session)["overhead_tokens"].as_u64(),
            Some(expected),
            "重复事件不得二次计入"
        );
        cleanup_ledger_file(&session);
    }

    /// 无 `call_id`（部分本地模型不带 id）时按「工具名 + 内容 hash」去重：
    /// 引擎侧仍对每次真实调用各记一次，事件桥只跳过已登记的那些。
    #[test]
    fn test_tool_output_dedupes_without_call_id_by_content() {
        let session = format!("mgt-ctx-e12-noid-{}", uuid::Uuid::new_v4());
        let text = "same tool output";
        let one = estimate_tokens(text);

        // 两次真实调用（都无 id）→ 各记一次
        record_tool_output_at_context(&session, None, "shell", text);
        record_tool_output_at_context(&session, None, "shell", text);
        assert_eq!(
            context_status(&session)["overhead_tokens"].as_u64(),
            Some(one * 2)
        );

        // 桥随后收到这两条事件 → 全部跳过（引擎已登记）
        record_tool_output_event_fallback(&session, None, "shell", text);
        record_tool_output_event_fallback(&session, None, "shell", text);
        assert_eq!(
            context_status(&session)["overhead_tokens"].as_u64(),
            Some(one * 2),
            "无 call_id 时也必须靠内容键去重"
        );
        cleanup_ledger_file(&session);
    }

    /// 失败结果按实际错误文本计；空输出（实际文本为空）不改变台账。
    #[test]
    fn test_tool_output_event_fallback_counts_failure_and_skips_empty() {
        let session = format!("mgt-ctx-e12-fail-{}", uuid::Uuid::new_v4());
        let err_text = "[工具错误 E_TIMEOUT] shell_exec 超时 120s";
        let expected = estimate_tokens(err_text);

        record_tool_output_event_fallback(&session, Some("call-f"), "shell_exec", err_text);
        assert_eq!(
            context_status(&session)["overhead_tokens"].as_u64(),
            Some(expected),
            "失败的工具结果不得因失败而跳过记账"
        );

        record_tool_output_event_fallback(&session, Some("call-e"), "shell_exec", "   ");
        assert_eq!(
            context_status(&session)["overhead_tokens"].as_u64(),
            Some(expected),
            "空输出不改变台账"
        );
        cleanup_ledger_file(&session);
    }

    #[test]
    fn test_context_status_surfaces_overhead() {
        let session = format!("mgt-ctx-status-{}", uuid::Uuid::new_v4());
        record_overhead(&session, 1234);
        let v = context_status(&session);
        assert!(
            v.get("overhead_tokens").is_some(),
            "context_status 必须暴露 overhead_tokens（使此前的漏记可观测）"
        );
        assert_eq!(v["overhead_tokens"].as_u64(), Some(1234));
        assert_eq!(v["used_tokens"].as_u64(), Some(1234));
        cleanup_ledger_file(&session);
    }

    #[test]
    fn test_legacy_json_without_overhead_deserializes() {
        // 旧版磁盘 JSON 无 overhead_tokens / partial_text 字段 → serde(default) 回退为 0/None
        let legacy = r#"{
            "session": "legacy1",
            "entries": [],
            "budget": 128000,
            "compaction_count": 0,
            "last_compact_ms": null,
            "summary": null,
            "last_mode": null,
            "compression_ratio": null,
            "snapshot_count": 0,
            "project_path": null
        }"#;
        let ctx: SessionContext =
            serde_json::from_str(legacy).expect("旧 JSON 必须仍可反序列化");
        assert_eq!(ctx.overhead_tokens, 0, "缺失字段应回退为 0");
        assert_eq!(ctx.partial_text, None);
        assert_eq!(ctx.used_tokens(), 0);
    }
}
