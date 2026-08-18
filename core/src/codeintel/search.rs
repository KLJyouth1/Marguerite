//! 混合检索与代码上下文组装
//!
//! - 符号精确命中优先（tree-sitter 索引的 fn/struct/class 等定义）；
//! - 文本 BM25 命中补充（复用 `kb::store::bm25_score`，中文经 bigram 召回）；
//! - `build_context` 把命中文件的相关代码按 token 预算组装成可注入 prompt 的片段。

use std::collections::HashSet;

use crate::codeintel::index::ProjectIndex;
use crate::context::session::estimate_tokens;
use crate::kb::store::bm25_score;

/// 单条检索命中
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchHit {
    pub path: String,
    pub line: Option<usize>,
    pub snippet: String,
    pub score: f64,
    /// 来源：`symbol`（定义命中）| `text`（BM25 文本命中）
    pub source: String,
}

/// 截断片段到约 `max` 字符（按字符边界，绝不 panic）
fn truncate_snippet(text: &str, max: usize) -> String {
    let mut out: String = text.chars().take(max).collect();
    if text.chars().count() > max {
        out.push('…');
    }
    out
}

/// 混合检索：符号命中在前，文本命中在后（按 path 去重，符号优先）
pub fn search(index: &ProjectIndex, query: &str, limit: usize) -> anyhow::Result<Vec<SearchHit>> {
    let limit = limit.max(1);
    let mut seen: HashSet<String> = HashSet::new();
    let mut hits: Vec<SearchHit> = Vec::new();

    // 1) 符号定义命中（最高优先级）
    for s in index.symbols_named(query, limit)? {
        if seen.insert(s.path.clone()) {
            hits.push(SearchHit {
                path: s.path,
                line: Some(s.line),
                snippet: format!("{} {}", s.kind, s.name),
                score: 1.0,
                source: "symbol".to_string(),
            });
        }
    }

    // 2) 文本 BM25 命中（补充符号未覆盖的文件）
    let chunks = index.all_chunks()?;
    let docs: Vec<&str> = chunks.iter().map(|(_, t)| t.as_str()).collect();
    for (i, score) in bm25_score(&docs, query) {
        if hits.len() >= limit {
            break;
        }
        let (path, text) = &chunks[i];
        if seen.insert(path.clone()) {
            hits.push(SearchHit {
                path: path.clone(),
                line: None,
                snippet: truncate_snippet(text, 300),
                score,
                source: "text".to_string(),
            });
        }
    }

    hits.truncate(limit);
    Ok(hits)
}

/// 把命中文件的相关代码组装为可注入 prompt 的上下文，受 token 预算约束。
///
/// 对每个命中路径取其索引分块拼接（保留行结构），超出预算时按字符截断并终止。
pub fn build_context(index: &ProjectIndex, hits: &[SearchHit], token_budget: u64) -> String {
    let mut out = String::new();
    let mut used: u64 = 0;
    let mut seen: HashSet<String> = HashSet::new();

    for h in hits {
        if seen.contains(&h.path) {
            continue;
        }
        seen.insert(h.path.clone());

        let Ok(chunks) = index.chunks_of(&h.path) else {
            continue;
        };
        if chunks.is_empty() {
            continue;
        }
        let text = chunks.join("\n");
        let header = format!("// === {} ===\n", h.path);
        let header_tokens = estimate_tokens(&header);
        let text_tokens = estimate_tokens(&text);

        if used + header_tokens + text_tokens > token_budget {
            let remain = token_budget.saturating_sub(used + header_tokens);
            let budget_chars = (remain.saturating_mul(4)) as usize;
            out.push_str(&header);
            out.push_str(&truncate_snippet(&text, budget_chars));
            out.push_str("\n// ... (truncated)\n");
            break;
        }

        used += header_tokens + text_tokens;
        out.push_str(&header);
        out.push_str(&text);
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn tmp_dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "codeintel_srch_{tag}_{}_{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn write(ws: &Path, rel: &str, content: &str) {
        let p = ws.join(rel);
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(&p, content).unwrap();
    }

    #[test]
    fn symbol_query_returns_definition_first() {
        let ws = tmp_dir("sym");
        let dbd = tmp_dir("symdb");
        write(
            &ws,
            "auth.rs",
            "pub fn authenticate(token: &str) -> bool { true }\n",
        );
        write(&ws, "util.rs", "pub fn unrelated() {}\n");

        let idx = ProjectIndex::open(&ws, &dbd).unwrap();
        idx.refresh().unwrap();
        let hits = search(&idx, "authenticate", 10).unwrap();
        assert!(!hits.is_empty());
        assert_eq!(hits[0].source, "symbol");
        assert_eq!(hits[0].path, "auth.rs");
        assert_eq!(hits[0].line, Some(1));
        let _ = std::fs::remove_dir_all(&ws);
        let _ = std::fs::remove_dir_all(&dbd);
    }

    #[test]
    fn chinese_query_hits_text_via_bigram() {
        let ws = tmp_dir("zh");
        let dbd = tmp_dir("zhdb");
        write(
            &ws,
            "note.rs",
            "// 三缓冲渲染保证帧率稳定\npub fn render() {}\n",
        );
        let idx = ProjectIndex::open(&ws, &dbd).unwrap();
        idx.refresh().unwrap();
        let hits = search(&idx, "帧率", 10).unwrap();
        assert!(!hits.is_empty(), "中文 bigram 应命中文本");
        let _ = std::fs::remove_dir_all(&ws);
        let _ = std::fs::remove_dir_all(&dbd);
    }

    #[test]
    fn context_respects_token_budget() {
        let ws = tmp_dir("ctx");
        let dbd = tmp_dir("ctxdb");
        let big = (0..200)
            .map(|i| format!("big line {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        write(&ws, "big.rs", &big);
        let idx = ProjectIndex::open(&ws, &dbd).unwrap();
        idx.refresh().unwrap();
        let hits = search(&idx, "big", 5).unwrap();

        let small = build_context(&idx, &hits, 10);
        let large = build_context(&idx, &hits, 100_000);
        assert!(!small.is_empty());
        assert!(small.len() < large.len(), "预算小应截断");
        let _ = std::fs::remove_dir_all(&ws);
        let _ = std::fs::remove_dir_all(&dbd);
    }
}
