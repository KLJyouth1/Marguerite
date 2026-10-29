//! 跨项目会话记忆桥（B3 升级：向量语义 + 渐进披露）
//!
//! 借鉴 GUI_Agent `cross_project_bridge.py`：直接检索本机各 AI 工具写入的
//! `~/.trae-cn/memory/projects/<project>/{project_memory.md, topics.md}` 记忆文件。
//! 语义层复用 [`LocalEmbedding`] 特征哈希向量（词 + 双词 + 中文单字粒度，零依赖），
//! 并对查询做中英同义扩展（[`expand_query`]）以提升跨语言语义命中；
//! 敏感字段（API key / token / password 等）自动 `[REDACTED]` 脱敏。
//!
//! **渐进披露**（对齐 claude-mem）：
//! - [`cross_project_search`] 只返回标题级摘要（title + 截断 excerpt + 行号），防上下文爆炸；
//! - 命中后由 [`cross_project_expand`] 按 `{project, file, line}` 展开该行 ±3 行上下文。
//!
//! 返回形如：
//! ```json
//! { "items": [{ "project", "file", "kind", "title", "line", "excerpt", "score", "updated_at" }] }
//! ```

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use crate::memory::compact::as_stable_prefix;
use crate::memory::embedding::EmbeddingProvider;
use crate::memory::long_term::LocalEmbedding;
use crate::memory::Observation;
use crate::types::Message;

/// 记忆根目录（对齐 TRAE/WorkBuddy 记忆协议）
fn memory_root() -> PathBuf {
    std::env::var_os("USERPROFILE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("C:\\Users"))
        .join(".trae-cn")
        .join("memory")
        .join("projects")
}

/// 关注的记忆文件（project_memory.md 优先，其次 topics.md）
const MEMORY_FILES: &[&str] = &["project_memory.md", "topics.md"];
/// 脱敏关键词
const SECRET_KEYS: &[&str] = &[
    "api_key",
    "apikey",
    "token",
    "password",
    "passwd",
    "secret",
    "credential",
    "authorization",
    "bearer",
    "private_key",
    "BEGIN PRIVATE KEY",
];
/// 检索结果上限
const MAX_ITEMS: usize = 12;
/// 摘要级 excerpt 截断字符数
const SNIPPET_CHARS: usize = 60;
/// 召回注入片段长度：比搜索摘要更长 —— 60 字只够“知道有这件事”，
/// 不足以让主模型据此推理，等于召回了但用不上。
const RECALL_SNIPPET_CHARS: usize = 160;
/// 仅靠向量分的准入门槛。
///
/// 0.12 时代几乎不设防：哈希向量对长句的区分度弱，低分行大多是噪声，
/// 结果是“4 条命中”但没有一条真正相关。抬高门槛，并用关键词重叠兜底。
const RECALL_LINE_SCORE_MIN: f64 = 0.22;
/// 关键词重叠的最低 token 数（CJK 单字也计）。达到即视为字面强相关。
const RECALL_KEYWORD_MIN: usize = 2;
/// 渐进披露展开的上下文行数（前后各 N 行）
const EXPAND_RADIUS: usize = 3;

/// 中英同义扩展表（跨语言语义命中；key 为中文常用词，value 为对应英文/缩写）
const SYNONYM_MAP: &[(&str, &[&str])] = &[
    ("修复", &["fix", "bug", "bugfix"]),
    ("编译", &["build", "compile", "cargo"]),
    ("测试", &["test", "tests"]),
    ("部署", &["deploy", "release"]),
    ("性能", &["performance", "perf"]),
    ("依赖", &["dependency", "dependencies", "cargo"]),
    ("配置", &["config", "configuration"]),
    ("错误", &["error", "err"]),
    ("接口", &["api", "endpoint"]),
    ("数据库", &["database", "db", "sqlite"]),
    ("会话", &["session"]),
    ("上下文", &["context"]),
    ("压缩", &["compact", "compaction"]),
    ("记忆", &["memory", "mem"]),
    ("规划", &["plan", "planning"]),
    ("浏览器", &["browser", "chrome", "cdp"]),
    ("联网", &["web", "search", "fetch"]),
    ("网关", &["gateway", "ws"]),
    ("启动", &["start", "boot", "launch"]),
    ("崩溃", &["crash", "panic", "hang"]),
];

/// 一条跨项目记忆命中
#[derive(Debug, Clone)]
pub struct CrossProjectHit {
    pub project: String,
    pub file: String,
    pub kind: String,
    pub title: String,
    pub line: usize,
    pub excerpt: String,
    pub score: f64,
    pub updated_at: String,
}

/// 查询扩展：把查询中出现的中文关键词映射到对应英文（跨语言语义召回）
fn expand_query(query: &str) -> String {
    let mut extra: Vec<&str> = Vec::new();
    for (zh, en) in SYNONYM_MAP {
        if query.contains(zh) {
            extra.extend(*en);
        }
    }
    if extra.is_empty() {
        return query.to_string();
    }
    format!("{} {}", query, extra.join(" "))
}

/// 行内是否含有可复用的事实内容：空行、代码围栏、纯标题/分隔线都不算。
fn is_informative_line(line: &str) -> bool {
    let trimmed = line.trim();
    if trimmed.is_empty()
        || trimmed.starts_with("```")
        || trimmed.starts_with("|---")
        || trimmed.starts_with("| ---")
    {
        return false;
    }
    trimmed.chars().filter(|c| c.is_alphanumeric()).count() >= 6
}

/// 查询与候选行的分词重叠数（零依赖粗分词；CJK 单字也计）。
///
/// 用于兜住「向量分偏低但字面强相关」的行，并给低分噪声再加一道闸。
fn keyword_overlap(query: &str, line: &str) -> usize {
    let q: HashSet<String> = crate::e::nlp::tokenize_zh(&expand_query(query))
        .into_iter()
        .filter(|t| !t.is_empty())
        .collect();
    if q.is_empty() {
        return 0;
    }
    let l: HashSet<String> = crate::e::nlp::tokenize_zh(line).into_iter().collect();
    q.intersection(&l).count()
}

/// 大小写不敏感地在**原始串**中定位 ASCII 模式，返回原始串的字节偏移。
///
/// 直接扫描原串（不经过 `to_lowercase()` 副本），保证返回的偏移一定落在字符边界上，
/// 从而修复「用 lowercase 副本的偏移去切原串」在非 ASCII 文本前的错位/越界缺陷。
fn find_ci(haystack: &str, needle: &str) -> Option<usize> {
    let n = needle.len();
    if n == 0 || haystack.len() < n {
        return None;
    }
    let hbytes = haystack.as_bytes();
    let nbytes = needle.as_bytes();
    let last = haystack.len() - n;
    let mut i = 0;
    while i <= last {
        if haystack.is_char_boundary(i)
            && haystack.is_char_boundary(i + n)
            && hbytes[i..i + n].eq_ignore_ascii_case(nbytes)
        {
            return Some(i);
        }
        i += 1;
    }
    None
}

/// 脱敏：替换形如 key=value / key: value 的敏感值
///
/// 对每个敏感键替换**全部**出现（而非仅第一处），且大小写不敏感匹配基于原串偏移，
/// 避免非 ASCII 前缀下 `to_lowercase()` 变长导致的错位/越界。值仍替换为 `[REDACTED]`，
/// 并保留既有 `end <= 64` 长度守卫。
pub(crate) fn redact(text: &str) -> String {
    /// 单模式最多处理的匹配数（防病态输入下的死循环；正常日志远低于此）。
    const MAX_REDACT_MATCHES: usize = 4096;
    let mut out = text.to_string();
    for k in SECRET_KEYS {
        let lower = k.to_lowercase();
        // key=value / key:value / "key": "value"
        let patterns = [
            format!("{lower}="),
            format!("{lower}:"),
            format!("\"{lower}\":"),
        ];
        for p in patterns {
            // 游标推进：每次匹配后从 `key+分隔符` 之后继续。替换只改 `start` 之后的
            // 内容，`out[..start]` 不变，故偏移在替换后依旧稳定。
            let mut from = 0usize;
            for _ in 0..MAX_REDACT_MATCHES {
                let Some(rel) = find_ci(&out[from..], &p) else {
                    break;
                };
                let start = from + rel + p.len();
                let rest = &out[start..];
                let end = rest
                    .find(|c: char| !c.is_alphanumeric() && c != '-' && c != '_' && c != '.')
                    .unwrap_or(rest.len());
                if end > 0 && end <= 64 {
                    let replacement = format!("{}[REDACTED]", &out[..start]);
                    out = format!("{replacement}{}", &rest[end..]);
                }
                // 无论是否替换都前进：替换后 `out[..start]` 未变，偏移仍有效；
                // 未替换则跳过本处匹配继续找下一个，避免死循环。
                from = start;
            }
        }
    }
    out
}

/// 枚举项目记忆文件
fn collect_files(root: &Path) -> Vec<(String, PathBuf, &'static str)> {
    let mut out = Vec::new();
    let entries = match std::fs::read_dir(root) {
        Ok(e) => e,
        Err(_) => return out,
    };
    for entry in entries.flatten() {
        let project = entry.file_name().to_string_lossy().to_string();
        let dir = entry.path();
        if !dir.is_dir() {
            continue;
        }
        for name in MEMORY_FILES {
            let p = dir.join(name);
            if p.is_file() {
                let kind = if *name == "project_memory.md" {
                    "project_memory"
                } else {
                    "topics"
                };
                out.push((project.clone(), p, kind));
            }
        }
    }
    out
}

/// 从 Markdown 提取内容行（保留原始行号，1-based；去标题语法、限长）
fn content_lines(raw: &str) -> Vec<(usize, String)> {
    raw.lines()
        .enumerate()
        .filter(|(_, l)| !l.trim().is_empty())
        .map(|(idx, l)| {
            let cleaned = l
                .trim()
                .trim_start_matches('#')
                .trim()
                .trim_start_matches('>')
                .trim()
                .to_string();
            (idx + 1, cleaned)
        })
        .filter(|(_, l)| l.len() >= 4)
        .collect()
}

/// 文件首行标题（第一个内容行）
fn file_title(lines: &[(usize, String)]) -> String {
    lines
        .first()
        .map(|(_, l)| truncate_chars(l, 80))
        .unwrap_or_default()
}

/// 按字符数截断（保留中文字符完整性）
fn truncate_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let head: String = s.chars().take(max).collect();
    format!("{head}…")
}

/// `memory.cross_project.search`：跨项目记忆向量语义检索（标题级摘要，渐进披露第 1 步）
///
/// params: `{ query: string, limit?: number }`
pub fn cross_project_search(params: &Value) -> Value {
    let query = params.get("query").and_then(Value::as_str).unwrap_or("");
    let limit = params.get("limit").and_then(Value::as_u64).unwrap_or(6) as usize;
    if query.trim().is_empty() {
        return json!({ "items": [], "root": memory_root().to_string_lossy() });
    }
    let q_vec = EmbeddingProvider::global().embed(&expand_query(query));
    let root = memory_root();
    let mut hits: Vec<CrossProjectHit> = Vec::new();

    for (project, path, kind) in collect_files(&root) {
        let raw = match std::fs::read_to_string(&path) {
            Ok(r) => r,
            Err(_) => continue,
        };
        let meta = std::fs::metadata(&path);
        let updated_at = meta
            .ok()
            .and_then(|m| m.modified().ok())
            .map(|t| {
                let d = t.duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
                d.as_secs().to_string()
            })
            .unwrap_or_default();
        let lines = content_lines(&raw);
        // 整文件相似度低时跳过（快速过滤）
        let full_score =
            EmbeddingProvider::global().cosine(&q_vec, &EmbeddingProvider::global().embed(&raw));
        if full_score < 0.02 {
            continue;
        }
        for (line_no, line) in &lines {
            let line_score = EmbeddingProvider::global()
                .cosine(&q_vec, &EmbeddingProvider::global().embed(line));
            if line_score < 0.12 {
                continue;
            }
            let combined = line_score * 0.7 + full_score * 0.3;
            hits.push(CrossProjectHit {
                project: project.clone(),
                file: path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string(),
                kind: kind.to_string(),
                title: file_title(&lines),
                line: *line_no,
                excerpt: redact(&truncate_chars(line, SNIPPET_CHARS)),
                score: combined,
                updated_at: updated_at.clone(),
            });
        }
    }
    // 按分数降序，去重（同 project+excerpt）
    hits.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let mut seen: HashSet<String> = HashSet::new();
    let items: Vec<Value> = hits
        .into_iter()
        .filter(|h| seen.insert(format!("{}.{}.{}", h.project, h.file, h.line)))
        .take(limit.clamp(1, MAX_ITEMS))
        .map(|h| {
            json!({
                "project": h.project,
                "file": h.file,
                "kind": h.kind,
                "title": h.title,
                "line": h.line,
                "excerpt": h.excerpt,
                "score": (h.score * 1000.0).round() / 1000.0,
                "updated_at": h.updated_at,
            })
        })
        .collect();
    json!({ "items": items, "root": root.to_string_lossy() })
}

/// `memory.cross_project.expand`：渐进披露第 2 步——展开命中行的上下文
///
/// params: `{ project: string, file: string, line: number }`
/// 返回命中行 ±[`EXPAND_RADIUS`] 行的上下文（全部脱敏）。
pub fn cross_project_expand(params: &Value) -> Value {
    let project = params.get("project").and_then(Value::as_str).unwrap_or("");
    let file = params.get("file").and_then(Value::as_str).unwrap_or("");
    let line = params.get("line").and_then(Value::as_u64).unwrap_or(0) as usize;
    if project.is_empty() || file.is_empty() || line == 0 {
        return json!({ "expanded": false, "error": "需要 project/file/line" });
    }
    let root = memory_root();
    let path = root.join(project).join(file);
    let raw = match std::fs::read_to_string(&path) {
        Ok(r) => r,
        Err(_) => return json!({ "expanded": false, "error": "记忆文件不可读" }),
    };
    let all: Vec<String> = raw.lines().map(redact).collect();
    let lo = line.saturating_sub(EXPAND_RADIUS + 1);
    let hi = (line + EXPAND_RADIUS).min(all.len());
    let context: Vec<Value> = all[lo..hi]
        .iter()
        .enumerate()
        .map(|(i, text)| {
            json!({
                "line": lo + i + 1,
                "text": text,
            })
        })
        .collect();
    let hit = all.get(line.saturating_sub(1)).cloned().unwrap_or_default();
    json!({
        "expanded": true,
        "project": project,
        "file": file,
        "line": line,
        "hit": hit,
        "context": context,
    })
}

/// 跨项目记忆检索（纯召回，不含渲染）：返回去重后的 `(项目名, 脱敏片段)` 列表
/// （按相似度降序、按 `limit` 截断）。`cross_project_recall_hint` 与 W-6 的
/// `recall_stable_prefix` 共用同一检索面，保证两路召回结果一致。
fn gather_recall_snippets_for_session(
    query: &str,
    limit: usize,
    session: &str,
) -> Vec<(String, String)> {
    let mut rows = gather_recall_scored(query, limit);
    let _ = integrate_synapse(session, query, &mut rows);
    rows.into_iter().map(|r| (r.project, r.snippet)).collect()
}

/// 召回一条（内部）：含稳定 id 与分数，供联想记忆网络接入。
#[derive(Debug, Clone)]
struct RecallScored {
    /// 稳定条目 id（隐私红线：仅哈希，不含记忆正文）
    id: String,
    project: String,
    snippet: String,
    score: f64,
}

/// 记忆条目稳定 id：`project + 片段` 的 FNV-1a 哈希（复用既有 [`crate::cache::hash_text`]）。
/// 仅暴露哈希 id，绝不泄露记忆正文（隐私红线）。
fn recall_id(project: &str, snippet: &str) -> String {
    format!(
        "{:016x}",
        crate::cache::hash_text(&format!("{project}\u{1}{snippet}"))
    )
}

/// 联想记忆网络对激活分的权重（基础相似度分 + 该权重 × 激活分）。
const SYNAPSE_ACT_WEIGHT: f64 = 0.15;
/// 事件 `ids` 字段上限。
const RECALL_IDS_MAX: usize = 8;

/// 与 [`gather_recall_snippets_for_session`] 同一打分逻辑，但保留稳定 id 与分数（synapse 接入所需）。
fn gather_recall_scored(query: &str, limit: usize) -> Vec<RecallScored> {
    if query.trim().is_empty() {
        return Vec::new();
    }
    let q_vec = EmbeddingProvider::global().embed(&expand_query(query));
    let root = memory_root();
    let mut hits: Vec<RecallScored> = Vec::new();
    for (project, path, _kind) in collect_files(&root) {
        let raw = match std::fs::read_to_string(&path) {
            Ok(r) => r,
            Err(_) => continue,
        };
        // 整文件相似度低时快速过滤（与 cross_project_search 阈值一致）
        let full_score =
            EmbeddingProvider::global().cosine(&q_vec, &EmbeddingProvider::global().embed(&raw));
        if full_score < 0.02 {
            continue;
        }
        for (_line_no, line) in content_lines(&raw) {
            // 结构性噪声（空行、代码围栏、只有标题）没有可复用的事实内容。
            if !is_informative_line(&line) {
                continue;
            }
            let line_score = LocalEmbedding::cosine(&q_vec, &LocalEmbedding::embed(&line));
            let overlap = keyword_overlap(query, &line);
            if line_score < RECALL_LINE_SCORE_MIN && overlap < RECALL_KEYWORD_MIN {
                continue;
            }
            // 字面命中优先于哈希向量的偶然相似：把分数抬到门槛之上。
            let line_score = if overlap >= RECALL_KEYWORD_MIN {
                line_score.max(0.25)
            } else {
                line_score
            };
            let combined = line_score * 0.7 + full_score * 0.3;
            let snippet = redact(&truncate_chars(&line, RECALL_SNIPPET_CHARS));
            hits.push(RecallScored {
                id: recall_id(&project, &snippet),
                project: project.clone(),
                snippet,
                score: combined,
            });
        }
    }
    hits.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.id.cmp(&b.id))
    });
    let mut seen: HashSet<String> = HashSet::new();
    hits.retain(|h| seen.insert(h.id.clone()));
    hits.truncate(limit.max(1));
    hits
}

/// 联想记忆网络接入点：① 共召回强化；② 扩散激活分参与重排；③ 落盘；④ 发
/// `session.event.memory_recall` 事件。
///
/// 可关闭：`memory.synapse_enabled=false` 时**不**强化、**不**重排、**不**发事件，
/// 结果与引入该功能前逐位一致。返回本次强化的共现对数。
fn integrate_synapse(session: &str, query: &str, rows: &mut [RecallScored]) -> usize {
    if rows.is_empty() {
        return 0;
    }
    // 关闭开关：完全等价于既有行为（顺序/内容零变化，且不发事件）。
    if !crate::memory::synapse::synapse_enabled() {
        return 0;
    }
    let ids: Vec<String> = rows.iter().map(|r| r.id.clone()).collect();
    let graph = crate::memory::synapse::global_graph();
    let reinforced = {
        let mut g = graph.lock().unwrap_or_else(|e| e.into_inner());
        let now = crate::cache::now_ms();
        // 时间驱动的遗忘（生产接线）：权重不再只增不减，老边按间隔衰减直至被剪枝。
        // 首次调用仅建立基准（不衰减）；间隔不足一个周期时是 no-op。
        let _ = g.maybe_decay_by_elapsed(now);
        let reinforced = g.reinforce(&ids, now);
        // 扩散激活：以本轮命中的 id 为种子，取邻居激活分（确定性）
        let act: std::collections::HashMap<String, f64> =
            g.activate(&ids, rows.len().max(1)).into_iter().collect();
        // 落盘（失败静默：联想记忆是尽力而为的增强，绝不阻塞召回）
        let _ = g.save_to(&crate::memory::synapse::synapse_path());
        drop(g);
        // 重排：基础相似度 + 权重 × 激活分（同分按 id 稳定排序）
        for r in rows.iter_mut() {
            r.score += SYNAPSE_ACT_WEIGHT * act.get(&r.id).copied().unwrap_or(0.0);
        }
        rows.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.id.cmp(&b.id))
        });
        reinforced
    };

    // 事件（诚实可视化契约）：仅在有命中时发布；hits==0 不发。
    let top_score = rows.first().map(|r| r.score.clamp(0.0, 1.0)).unwrap_or(0.0);
    let ids_top: Vec<String> = rows
        .iter()
        .take(RECALL_IDS_MAX)
        .map(|r| r.id.clone())
        .collect();
    if let Some(bus) = crate::events::global_bus() {
        bus.publish(crate::events::Envelope::new(
            crate::events::EventType::MemoryRecall,
            serde_json::json!({
                "sessionKey": session,
                "query": query,
                "hits": rows.len(),
                "topScore": (top_score * 1000.0).round() / 1000.0,
                "ids": ids_top,
                "reinforced": reinforced,
            }),
            session,
        ));
    }
    reinforced
}

pub fn cross_project_recall_hint(query: &str, limit: usize, session: &str) -> String {
    let snips = gather_recall_snippets_for_session(query, limit, session);
    if snips.is_empty() {
        return String::new();
    }
    let mut s =
        String::from("以下是其他项目记忆中与该请求相关的既有经验，来源项目已标注，仅作参考：\n");
    for (project, snippet) in &snips {
        s.push_str(&format!("- [{project}] {snippet}\n"));
    }
    s
}

/// 把召回片段渲染成**稳定前缀**（`Message::System`），直接落进 P-7
/// `reorder_stable_prefix` 的 stable 半区 —— 接 W-6「记忆召回主路径默认注入 `as_stable_prefix`」。
///
/// 与 [`cross_project_recall_hint`] 同源检索，但产出固定格式（`## Recalled memory (compacted)`）
/// 的 System 消息：相同召回结果逐字节稳定，可被 `PrefixCache` / 服务端 prompt caching 复用。
/// 无命中 / 空查询时返回 `None`，绝不阻塞对话。
pub fn recall_stable_prefix(query: &str, limit: usize, session: &str) -> Option<Message> {
    let snips = gather_recall_snippets_for_session(query, limit, session);
    snippets_to_stable_prefix(&snips)
}

/// 片段 → 稳定前缀（纯函数，便于单测；空片段返回 `None`）。
/// 每条片段渲染成一条 `Observation`（title=项目名，facts=[片段]），再经
/// [`as_stable_prefix`] 渲染为固定格式的 System 消息。
pub fn snippets_to_stable_prefix(snips: &[(String, String)]) -> Option<Message> {
    if snips.is_empty() {
        return None;
    }
    let observations: Vec<Observation> = snips
        .iter()
        .map(|(project, snippet)| Observation {
            kind: "cross_project".to_string(),
            title: project.clone(),
            facts: vec![snippet.clone()],
            concepts: Vec::new(),
            discovery_tokens: 0,
            created_at_ms: 0,
        })
        .collect();
    as_stable_prefix(&observations)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cosine_similarity_shared_embedding() {
        let a = LocalEmbedding::embed("hello world");
        let b = LocalEmbedding::embed("hello world");
        let c = LocalEmbedding::embed("completely different topic");
        assert!((LocalEmbedding::cosine(&a, &b) - 1.0).abs() < 1e-9);
        assert!(LocalEmbedding::cosine(&a, &c) < 0.9);
    }

    #[test]
    fn recall_gate_rejects_noise_and_accepts_keyword_hits() {
        // 字面强相关：即使哈希向量分一般，也应通过关键词兜底
        assert!(keyword_overlap("修复编译错误", "修复编译错误：补上 use 语句") >= 2);
        // 完全无关：重叠为 0，只能靠向量分过门槛
        assert_eq!(keyword_overlap("部署发布", "今天天气不错，适合散步"), 0);
    }

    #[test]
    fn informative_line_filter_skips_structural_noise() {
        assert!(!is_informative_line(""));
        assert!(!is_informative_line("## 进展"));
        assert!(!is_informative_line("```rust"));
        assert!(!is_informative_line("| --- | --- |"));
        // 有事实内容的行必须保留
        assert!(is_informative_line("- 修复了上下文压缩过早触发的问题"));
    }

    #[test]
    fn test_expand_query_appends_synonyms() {
        let q = expand_query("修复编译错误");
        assert!(q.contains("fix"));
        assert!(q.contains("build"));
        assert!(q.contains("error"));
        let plain = expand_query("普通无关键词查询");
        assert_eq!(plain, "普通无关键词查询");
    }

    #[test]
    fn test_redact_secrets() {
        let s = redact("api_key=sk-abc123def456 连接成功");
        assert!(!s.contains("sk-abc123def456"));
        assert!(s.contains("[REDACTED]"));
        let s2 = redact("正常文本没有敏感信息");
        assert_eq!(s2, "正常文本没有敏感信息");
    }

    #[test]
    fn test_redact_all_occurrences() {
        // B1 回归：同一敏感键多次出现时，旧实现只替换第一处 → 后续值泄漏。
        let s = redact("token=AAA 中间 token=BBB 末尾 token=CCC");
        assert!(!s.contains("AAA"), "第一处应脱敏: {s}");
        assert!(!s.contains("BBB"), "第二处不得泄漏: {s}");
        assert!(!s.contains("CCC"), "第三处不得泄漏: {s}");
        assert_eq!(s.matches("[REDACTED]").count(), 3, "三处都应脱敏: {s}");
    }

    #[test]
    fn test_redact_non_ascii_prefix_no_panic() {
        // 'İ' 的 to_lowercase() 会变长（2→3 字节）：旧实现用 lowercase 副本的偏移切原串，
        // 会错位漏脱敏甚至切在非字符边界上 panic。
        let s = redact("İİ token=SECRET123 结束");
        assert!(!s.contains("SECRET123"), "非 ASCII 前缀不得漏脱敏: {s}");
        assert!(s.contains("[REDACTED]"));
        // 纯中文（多字节）前缀同样不得 panic
        let s2 = redact("中文前缀 password=hunter2 后续");
        assert!(!s2.contains("hunter2"), "中文前缀不得漏脱敏: {s2}");
        assert!(s2.contains("[REDACTED]"));
    }

    #[test]
    fn test_cross_project_search_empty_query() {
        let v = cross_project_search(&json!({}));
        assert!(v["items"].is_array());
        assert!(v["items"].as_array().unwrap().is_empty());
    }

    #[test]
    fn test_collect_files_missing_root() {
        let hits = collect_files(&PathBuf::from("Z:\\definitely-missing"));
        assert!(hits.is_empty());
    }

    #[test]
    fn test_cross_project_expand_missing_args() {
        let v = cross_project_expand(&json!({}));
        assert_eq!(v["expanded"], false);
        let v2 = cross_project_expand(&json!({ "project": "p", "file": "f", "line": 1 }));
        // 项目不存在：应返回 expanded=false（不可读）
        assert_eq!(v2["expanded"], false);
    }

    #[test]
    fn test_truncate_chars_keeps_cjk() {
        let s = "一二三四五六七八九十";
        assert_eq!(truncate_chars(s, 5), "一二三四五…");
        assert_eq!(truncate_chars(s, 100), s);
    }

    #[test]
    fn test_recall_hint_empty_query_returns_empty() {
        assert!(cross_project_recall_hint("", 4, "s").is_empty());
        assert!(cross_project_recall_hint("   ", 4, "s").is_empty());
    }

    #[test]
    fn test_recall_hint_missing_root_returns_empty() {
        // 无法覆盖 memory_root（固定 USERPROFILE）；至少保证确定性返回不 panic，
        // 且 limit=0 被 clamp 为 1 后行为一致。
        let _ = cross_project_recall_hint("编译错误修复", 0, "s");
        let _ = cross_project_recall_hint("部署上线", 8, "s");
    }

    // ── W-6：记忆召回主路径默认注入 as_stable_prefix（稳定前缀）──
    #[test]
    fn snippets_to_stable_prefix_renders_system_message() {
        let m = snippets_to_stable_prefix(&[
            ("proj-a".to_string(), "use redact() for secrets".to_string()),
            (
                "proj-b".to_string(),
                "prefer tokio spawn for IO".to_string(),
            ),
        ])
        .expect("应产出稳定前缀");
        match m {
            Message::System { content, .. } => {
                assert!(
                    content.starts_with("## Recalled memory (compacted)"),
                    "固定格式表头缺失：{content}"
                );
                assert!(content.contains("proj-a"), "应含项目名：{content}");
                assert!(
                    content.contains("use redact() for secrets"),
                    "应含片段：{content}"
                );
                assert!(content.contains("proj-b"), "应含第二条项目名：{content}");
            }
            _ => panic!("应为 System 消息"),
        }
    }

    #[test]
    fn snippets_to_stable_prefix_empty_is_none() {
        assert!(snippets_to_stable_prefix(&[]).is_none());
    }

    /// 召回耗时基准（**默认不跑**：依赖本机真实记忆语料）。
    ///
    /// 为什么要它：真机审计量到首 token 11–14s，而「回合前置准备」里最可疑的就是
    /// 跨项目召回 —— 它要为**每个**记忆文件读全文、算整文件向量，再逐行算向量。
    /// 没有这个数字，任何"并行化/加缓存"的改动都只是猜。
    ///
    /// 跑法：`cargo test -p marguerite-core --lib bench_recall -- --ignored --nocapture`
    #[test]
    #[ignore = "基准：依赖本机 ~/.trae-cn/memory/projects 的真实语料"]
    fn bench_recall_cost() {
        use std::time::Instant;
        let root = memory_root();
        let files = collect_files(&root);
        let total_bytes: u64 = files
            .iter()
            .filter_map(|(_, p, _)| std::fs::metadata(p).ok())
            .map(|m| m.len())
            .sum();
        println!(
            "[bench] memory_root={} files={} bytes={}",
            root.display(),
            files.len(),
            total_bytes
        );
        // 冷跑一次（把向量/文件缓存预热），再计时 5 次取均值。
        let _ = cross_project_recall_hint("网关 侧车 记忆", 4, "bench");
        let mut samples = Vec::new();
        for _ in 0..5 {
            let t = Instant::now();
            let hint = cross_project_recall_hint("网关 侧车 记忆", 4, "bench");
            samples.push((t.elapsed(), hint.chars().count()));
        }
        for (d, chars) in &samples {
            println!("[bench] {:>8.1}ms  hint_chars={chars}", d.as_secs_f64() * 1000.0);
        }
        let avg = samples.iter().map(|(d, _)| d.as_secs_f64()).sum::<f64>() / samples.len() as f64;
        println!("[bench] avg={:.1}ms over {} samples", avg * 1000.0, samples.len());
    }
}
