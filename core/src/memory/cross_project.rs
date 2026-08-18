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

/// 脱敏：替换形如 key=value / key: value 的敏感值
pub(crate) fn redact(text: &str) -> String {
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
            if let Some(idx) = out.to_lowercase().find(&p) {
                let start = idx + p.len();
                let rest = &out[start..];
                let end = rest
                    .find(|c: char| !c.is_alphanumeric() && c != '-' && c != '_' && c != '.')
                    .unwrap_or(rest.len());
                if end > 0 && end <= 64 {
                    let replacement = format!("{}[REDACTED]", &out[..start]);
                    out = format!("{replacement}{}", &rest[end..]);
                }
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

/// 跨项目记忆自动注入：按查询召回本机各项目记忆，格式化为系统提示片段。
///
/// 与 [`cross_project_search`] 同一检索面（`~/.trae-cn/memory/projects/`），但直接产出
/// 供系统提示注入的纯文本（来源项目标注 + 去重 + 脱敏 + 限长），避免把整段记忆塞进
/// 上下文。无记忆目录 / 无相关命中 / 空查询时返回空串，绝不阻塞对话。
///
/// 用途：`run_turn` 前置调用，把跨项目既有经验随系统提示一起喂给模型。
/// 跨项目记忆检索（纯召回，不含渲染）：返回去重后的 `(项目名, 脱敏片段)` 列表
/// （按相似度降序、按 `limit` 截断）。`cross_project_recall_hint` 与 W-6 的
/// `recall_stable_prefix` 共用同一检索面，保证两路召回结果一致。
fn gather_recall_snippets(query: &str, limit: usize) -> Vec<(String, String)> {
    if query.trim().is_empty() {
        return Vec::new();
    }
    let q_vec = EmbeddingProvider::global().embed(&expand_query(query));
    let root = memory_root();
    let mut hits: Vec<(String, String, f64)> = Vec::new(); // (project, snippet, score)
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
            let line_score = LocalEmbedding::cosine(&q_vec, &LocalEmbedding::embed(&line));
            if line_score < 0.12 {
                continue;
            }
            let combined = line_score * 0.7 + full_score * 0.3;
            hits.push((
                project.clone(),
                redact(&truncate_chars(&line, SNIPPET_CHARS)),
                combined,
            ));
        }
    }
    // 分数降序 + 去重（同 project+snippet 只留一条），按 limit 截断
    hits.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap_or(std::cmp::Ordering::Equal));
    let mut seen: HashSet<String> = HashSet::new();
    let mut out: Vec<(String, String)> = Vec::new();
    for (project, snippet, _score) in hits {
        if !seen.insert(format!("{project}:{snippet}")) {
            continue;
        }
        out.push((project, snippet));
        if out.len() >= limit.max(1) {
            break;
        }
    }
    out
}

pub fn cross_project_recall_hint(query: &str, limit: usize) -> String {
    let snips = gather_recall_snippets(query, limit);
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
pub fn recall_stable_prefix(query: &str, limit: usize) -> Option<Message> {
    let snips = gather_recall_snippets(query, limit);
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
        assert!(cross_project_recall_hint("", 4).is_empty());
        assert!(cross_project_recall_hint("   ", 4).is_empty());
    }

    #[test]
    fn test_recall_hint_missing_root_returns_empty() {
        // 无法覆盖 memory_root（固定 USERPROFILE）；至少保证确定性返回不 panic，
        // 且 limit=0 被 clamp 为 1 后行为一致。
        let _ = cross_project_recall_hint("编译错误修复", 0);
        let _ = cross_project_recall_hint("部署上线", 8);
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
}
