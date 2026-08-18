//! KB 质量层 · 去噪（rag_denoiser）
//!
//! 在索引前过滤低质量文档，避免噪声污染 BM25 检索与知识图谱。
//! 全部为确定性启发式，零外部依赖、可单测。

use std::collections::{HashMap, HashSet};

/// 去噪报告。
#[derive(Debug, Clone, Default)]
pub struct NoiseReport {
    pub kept: usize,
    pub dropped: usize,
    pub reasons: HashMap<String, usize>,
}

impl NoiseReport {
    fn drop(&mut self, reason: &'static str) {
        self.dropped += 1;
        *self.reasons.entry(reason.to_string()).or_insert(0) += 1;
    }
}

const STOPWORDS: &[&str] = &[
    "the", "a", "an", "and", "or", "but", "of", "to", "in", "on", "for", "with", "is", "are",
    "was", "were", "be", "been", "being", "it", "this", "that", "these", "those", "as", "at", "by",
    "from", "we", "you", "they", "he", "she", "i", "not", "no", "yes", "if", "then", "so",
];

fn tokenize_lower(s: &str) -> Vec<String> {
    s.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| t.len() >= 2)
        .map(|t| t.to_string())
        .collect()
}

/// 判定单段文本是否为噪声，返回原因（非噪声返回 None）。
pub fn noise_reason(text: &str) -> Option<&'static str> {
    let trimmed = text.trim();
    if trimmed.chars().count() < 20 {
        return Some("too_short");
    }
    let lower = trimmed.to_lowercase();
    if lower.contains("lorem ipsum") {
        return Some("boilerplate");
    }
    let alnum = trimmed.chars().filter(|c| c.is_alphanumeric()).count();
    let punct = trimmed.chars().filter(|c| c.is_ascii_punctuation()).count();
    if alnum == 0 || (punct as f64) / (alnum as f64) > 0.5 {
        return Some("punctuation_spam");
    }
    let toks = tokenize_lower(trimmed);
    if !toks.is_empty() {
        let sw = toks
            .iter()
            .filter(|t| STOPWORDS.contains(&t.as_str()))
            .count();
        if (sw as f64) / (toks.len() as f64) > 0.85 {
            return Some("low_information");
        }
    }
    None
}

fn jaccard(a: &[String], b: &[String]) -> f64 {
    let sa: HashSet<&String> = a.iter().collect();
    let sb: HashSet<&String> = b.iter().collect();
    let inter = sa.intersection(&sb).count();
    let union = sa.union(&sb).count();
    if union == 0 {
        0.0
    } else {
        inter as f64 / union as f64
    }
}

/// 过滤文档列表：丢弃噪声文档，并对近重复文档去重（Jaccard > 0.9）。
pub fn filter_documents(docs: &[(String, String)]) -> (Vec<(String, String)>, NoiseReport) {
    let mut report = NoiseReport::default();
    let mut kept: Vec<(String, String)> = Vec::new();
    let mut kept_tok: Vec<Vec<String>> = Vec::new();
    for (title, text) in docs {
        if let Some(reason) = noise_reason(text) {
            report.drop(reason);
            continue;
        }
        let toks = tokenize_lower(text);
        let mut dup = false;
        for kt in &kept_tok {
            if jaccard(&toks, kt) > 0.9 {
                dup = true;
                break;
            }
        }
        if dup {
            report.drop("near_duplicate");
            continue;
        }
        kept.push((title.clone(), text.clone()));
        kept_tok.push(toks);
        report.kept += 1;
    }
    (kept, report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drops_too_short() {
        assert_eq!(noise_reason("hi"), Some("too_short"));
        assert_eq!(noise_reason(""), Some("too_short"));
    }

    #[test]
    fn drops_boilerplate() {
        assert_eq!(
            noise_reason("Lorem ipsum dolor sit amet consectetur adipiscing."),
            Some("boilerplate")
        );
    }

    #[test]
    fn drops_punctuation_spam() {
        assert_eq!(
            noise_reason("????????????????????????!!!!!!!!!!!!!!"),
            Some("punctuation_spam")
        );
    }

    #[test]
    fn drops_low_information() {
        let t = "the of and to in on for with is are the of and to in on for with is are";
        assert_eq!(noise_reason(t), Some("low_information"));
    }

    #[test]
    fn keeps_real_content() {
        assert_eq!(
            noise_reason(
                "Rust is a systems programming language focused on safety and concurrency."
            ),
            None
        );
    }

    #[test]
    fn filter_removes_noise_and_duplicates() {
        let docs = vec![
            (
                "good1".to_string(),
                "Rust emphasizes memory safety without a garbage collector.".to_string(),
            ),
            (
                "good2".to_string(),
                "Go is simple and built for concurrent programming.".to_string(),
            ),
            (
                "junk".to_string(),
                "the of and to in on for with is are".to_string(),
            ),
            (
                "dup".to_string(),
                "Rust emphasizes memory safety without a garbage collector.".to_string(),
            ),
        ];
        let (clean, report) = filter_documents(&docs);
        assert_eq!(report.dropped, 2, "junk + dup dropped, {:?}", report);
        assert_eq!(clean.len(), 2, "only good1+good2 kept");
        assert!(report.reasons.contains_key("low_information"));
        assert!(report.reasons.contains_key("near_duplicate"));
    }
}
