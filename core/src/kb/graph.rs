//! 知识图谱（KB 质量层 · Exclusive 能力 `kb.graph`）
//!
//! 在 BM25 关键词检索之外，构建实体-关系结构，支撑跨文档推理。
//! 离线启发式抽取（零 NLP 依赖、确定性、可单测）：
//! - 实体：大写开头的词序列（1–3 词）与引号短语；
//! - 关系：同块共现 → `CoOccurs` 加权边；句式 "X is a Y" / "X uses Y" 等 → 定型边。

use std::collections::{HashMap, HashSet};

/// 关系类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RelationKind {
    CoOccurs,
    IsA,
    Uses,
    Implements,
    Provides,
}

impl RelationKind {
    pub fn label(&self) -> &'static str {
        match self {
            RelationKind::CoOccurs => "co_occurs",
            RelationKind::IsA => "is_a",
            RelationKind::Uses => "uses",
            RelationKind::Implements => "implements",
            RelationKind::Provides => "provides",
        }
    }
}

/// 一条有向加权边。
#[derive(Debug, Clone)]
pub struct Edge {
    pub from: String,
    pub to: String,
    pub kind: RelationKind,
    pub weight: f64,
}

/// 内存知识图谱。
#[derive(Debug, Clone, Default)]
pub struct KnowledgeGraph {
    entities: HashSet<String>,
    edges: Vec<Edge>,
    doc_entities: HashMap<String, HashSet<String>>,
}

const IGNORE_SUBJECTS: &[&str] = &[
    "The", "A", "An", "It", "This", "That", "These", "Those", "When", "Then", "If", "For", "In",
    "On", "At", "To", "Of", "As", "But", "Or", "And", "We", "They", "You", "He", "She", "I",
];

const OBJECT_STOPWORDS: &[&str] = &[
    "the", "a", "an", "and", "or", "but", "of", "to", "in", "on", "for", "with", "is", "are",
    "was", "were", "be", "been", "being", "it", "this", "that", "as", "at", "by", "from", "not",
    "no",
];

fn is_capitalized_word(w: &str) -> bool {
    match w.chars().next() {
        Some(c) => c.is_uppercase(),
        None => false,
    }
}

fn strip_trailing_nonword(s: &str) -> String {
    s.trim_end_matches(|c: char| !c.is_alphanumeric() && c != ' ')
        .trim()
        .to_string()
}

/// 抽取文本中的实体（大写词序列 / 引号短语），确定性、保序去重。
pub fn extract_entities(text: &str) -> Vec<String> {
    let mut entities: Vec<String> = Vec::new();
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'"' {
            let mut j = i + 1;
            let mut s = String::new();
            while j < bytes.len() && bytes[j] != b'"' {
                s.push(bytes[j] as char);
                j += 1;
            }
            let s = s.trim().to_string();
            if s.len() >= 3 && s.chars().any(|c| c.is_alphabetic()) {
                entities.push(s);
            }
            i = j + 1;
        } else {
            i += 1;
        }
    }

    let words: Vec<&str> = text.split_whitespace().collect();
    let mut k = 0;
    while k < words.len() {
        if is_capitalized_word(words[k]) {
            let mut run: Vec<&str> = Vec::new();
            let mut m = k;
            while m < words.len() && is_capitalized_word(words[m]) && run.len() < 3 {
                run.push(words[m]);
                m += 1;
            }
            if run.len() == 1 && IGNORE_SUBJECTS.contains(&run[0]) {
                k = m;
                continue;
            }
            let ent = strip_trailing_nonword(&run.join(" "));
            if !ent.is_empty() && ent.chars().any(|c| c.is_alphabetic()) {
                entities.push(ent);
            }
            k = m;
        } else {
            k += 1;
        }
    }

    let mut seen = HashSet::new();
    entities.retain(|e| seen.insert(e.clone()));
    entities
}

/// 取某位置之前的实体主语（大写词序列，逆序扫描，剥离尾随忽略词）。
fn entity_at_left(text: &str, pos: usize) -> Option<String> {
    let left = &text[..pos];
    let words: Vec<&str> = left.split_whitespace().rev().collect();
    let mut run: Vec<&str> = Vec::new();
    for w in words {
        if is_capitalized_word(w) && run.len() < 3 {
            run.push(w);
        } else {
            break;
        }
    }
    while let Some(last) = run.last() {
        if IGNORE_SUBJECTS.contains(last) {
            run.pop();
        } else {
            break;
        }
    }
    if run.is_empty() {
        return None;
    }
    run.reverse();
    let ent = strip_trailing_nonword(&run.join(" "));
    if ent.chars().any(|c| c.is_alphabetic()) {
        Some(ent)
    } else {
        None
    }
}

/// 取某位置之后的宾语名词短语（1–3 词，允许小写，剥离尾随停用词）。
fn phrase_at_right(text: &str, pos: usize) -> Option<String> {
    let right = &text[pos..];
    let words: Vec<&str> = right.split_whitespace().collect();
    let mut run: Vec<String> = Vec::new();
    for w in words {
        let bare = w.trim_end_matches(|c: char| !c.is_alphanumeric());
        if bare.is_empty() || run.len() >= 3 {
            break;
        }
        run.push(bare.to_string());
    }
    while let Some(last) = run.last() {
        if OBJECT_STOPWORDS.contains(&last.to_lowercase().as_str()) || last.is_empty() {
            run.pop();
        } else {
            break;
        }
    }
    if run.is_empty() {
        return None;
    }
    let phrase = run.join(" ");
    if phrase.chars().any(|c| c.is_alphabetic()) {
        Some(phrase)
    } else {
        None
    }
}

/// 抽取文本中的定型关系（is_a / uses / implements / provides）。
pub fn extract_relations(text: &str) -> Vec<(String, String, RelationKind)> {
    let mut out = Vec::new();
    let connectors: &[(&str, RelationKind)] = &[
        (" is a ", RelationKind::IsA),
        (" is an ", RelationKind::IsA),
        (" uses ", RelationKind::Uses),
        (" implements ", RelationKind::Implements),
        (" provides ", RelationKind::Provides),
    ];
    for (pat, kind) in connectors {
        let mut search_from = 0;
        while let Some(rel) = text[search_from..].find(pat) {
            let abs = search_from + rel;
            let right_pos = abs + pat.len();
            if let (Some(from), Some(to)) =
                (entity_at_left(text, abs), phrase_at_right(text, right_pos))
            {
                if !from.is_empty() && !to.is_empty() && from != to {
                    out.push((from, to, *kind));
                }
            }
            search_from = right_pos;
        }
    }
    out
}

impl KnowledgeGraph {
    pub fn new() -> Self {
        Self::default()
    }

    /// 索引一篇文档，抽取实体与关系。
    pub fn index_document(&mut self, doc_id: &str, text: &str) {
        let entities = extract_entities(text);
        self.doc_entities
            .entry(doc_id.to_string())
            .or_default()
            .extend(entities.iter().cloned());
        for e in &entities {
            self.entities.insert(e.clone());
        }
        // 共现边（同块内两两配对）
        for i in 0..entities.len() {
            for j in (i + 1)..entities.len() {
                let (a, b) = if entities[i] <= entities[j] {
                    (entities[i].clone(), entities[j].clone())
                } else {
                    (entities[j].clone(), entities[i].clone())
                };
                self.add_or_update_edge(&a, &b, RelationKind::CoOccurs, 1.0);
            }
        }
        // 定型关系边
        for (from, to, kind) in extract_relations(text) {
            self.entities.insert(from.clone());
            self.entities.insert(to.clone());
            self.add_or_update_edge(&from, &to, kind, 2.0);
        }
    }

    fn add_or_update_edge(&mut self, from: &str, to: &str, kind: RelationKind, delta: f64) {
        if let Some(e) = self
            .edges
            .iter_mut()
            .find(|e| e.from == from && e.to == to && e.kind == kind)
        {
            e.weight += delta;
            return;
        }
        self.edges.push(Edge {
            from: from.to_string(),
            to: to.to_string(),
            kind,
            weight: delta,
        });
    }

    pub fn entity_count(&self) -> usize {
        self.entities.len()
    }

    pub fn edge_count(&self) -> usize {
        self.edges.len()
    }

    pub fn entities(&self) -> Vec<&str> {
        self.entities.iter().map(|s| s.as_str()).collect()
    }

    pub fn edges(&self) -> &[Edge] {
        &self.edges
    }

    /// 与某实体直接相连的所有邻居（出/入边均计，去重）。
    pub fn neighbors(&self, entity: &str) -> Vec<String> {
        let mut out = Vec::new();
        let mut seen = HashSet::new();
        for e in &self.edges {
            if e.from == entity && seen.insert(e.to.clone()) {
                out.push(e.to.clone());
            } else if e.to == entity && seen.insert(e.from.clone()) {
                out.push(e.from.clone());
            }
        }
        out
    }

    /// 语义化别名。
    pub fn related(&self, entity: &str) -> Vec<String> {
        self.neighbors(entity)
    }

    pub fn entities_in_document(&self, doc_id: &str) -> Vec<String> {
        self.doc_entities
            .get(doc_id)
            .map(|s| s.iter().cloned().collect())
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_capitalized_entities() {
        let ents = extract_entities("Rust and Kubernetes power cloud systems.");
        assert!(ents.contains(&"Rust".to_string()));
        assert!(ents.contains(&"Kubernetes".to_string()));
    }

    #[test]
    fn extracts_quoted_phrase_entity() {
        let ents = extract_entities("The \"Type System\" prevents data races.");
        assert!(ents.contains(&"Type System".to_string()));
    }

    #[test]
    fn co_occurrence_links_entities_in_chunk() {
        let mut g = KnowledgeGraph::new();
        g.index_document(
            "d1",
            "Rust and Kubernetes are both used for cloud native systems.",
        );
        assert!(g.entity_count() >= 2);
        let n = g.related("Rust");
        assert!(
            n.contains(&"Kubernetes".to_string()),
            "Rust↔Kubernetes co_occur, got {:?}",
            n
        );
    }

    #[test]
    fn typed_is_a_relation_extracted() {
        let mut g = KnowledgeGraph::new();
        g.index_document(
            "d2",
            "Python is a dynamic language loved by data scientists.",
        );
        let has_isa = g
            .edges()
            .iter()
            .any(|e| e.kind == RelationKind::IsA && e.from == "Python");
        assert!(
            has_isa,
            "Python is_a should be extracted, edges={:?}",
            g.edges()
        );
    }

    #[test]
    fn uses_relation_extracted() {
        let mut g = KnowledgeGraph::new();
        g.index_document("d3", "The compiler uses LLVM as its backend.");
        let has_uses = g
            .edges()
            .iter()
            .any(|e| e.kind == RelationKind::Uses && e.from == "LLVM");
        // 主语应是 The compiler 中大写的 "LLVM"? "compiler" 小写，主语抽取为空；
        // 用更明确的句子验证。
        let mut g2 = KnowledgeGraph::new();
        g2.index_document("d4", "Rust uses ownership to guarantee memory safety.");
        let u = g2
            .edges()
            .iter()
            .any(|e| e.kind == RelationKind::Uses && e.from == "Rust");
        assert!(u, "Rust uses ownership, edges={:?}", g2.edges());
        let _ = has_uses;
    }

    #[test]
    fn entities_in_document_tracked() {
        let mut g = KnowledgeGraph::new();
        g.index_document("docX", "Go and Rust are systems languages.");
        let ents = g.entities_in_document("docX");
        assert!(ents.contains(&"Go".to_string()));
        assert!(ents.contains(&"Rust".to_string()));
    }
}
