//! Track E — 原生 NLP 原语（不引 jieba / ONNX，够用即可、零依赖）。
//!
//! 提供中文粗分词、Jaccard 集合相似度、余弦向量相似度。供 `recall` 等模块复用，
//! 也为后续接真实 bge 嵌入召回时的"关键词召回"对照基线。

/// 把单字符缓冲区推入 token 列表（若非空）。
fn flush_token(buf: &mut String, tokens: &mut Vec<String>) {
    if !buf.is_empty() {
        tokens.push(std::mem::take(buf));
    }
}

/// 粗分词：ASCII/字母数字连续段聚为一个词 token；每个非 ASCII 字母（CJK、谚文等）
/// 单独成 token；空白与标点（ASCII/Unicode）断词。统一小写。不依赖任何外部 crate。
#[must_use]
pub fn tokenize_zh(text: &str) -> Vec<String> {
    let mut tokens: Vec<String> = Vec::new();
    let mut buf = String::new();

    for ch in text.chars() {
        if ch.is_whitespace() {
            flush_token(&mut buf, &mut tokens);
            continue;
        }
        if ch.is_alphanumeric() {
            if ch.is_ascii_alphanumeric() {
                buf.push(ch.to_ascii_lowercase());
            } else {
                // 非 ASCII 字母（CJK 等）单独成 token，提升中文重叠召回粒度。
                flush_token(&mut buf, &mut tokens);
                tokens.push(ch.to_lowercase().to_string());
            }
        } else {
            // 标点 / 符号（ASCII 或 Unicode）→ 断词
            flush_token(&mut buf, &mut tokens);
        }
    }
    flush_token(&mut buf, &mut tokens);
    tokens
}

/// Jaccard 集合相似度 = |A∩B| / |A∪B|。两集合皆空时返回 0.0（避免除零）。
#[must_use]
pub fn jaccard(a: &[String], b: &[String]) -> f64 {
    if a.is_empty() && b.is_empty() {
        return 0.0;
    }
    let sa: std::collections::HashSet<&String> = a.iter().collect();
    let sb: std::collections::HashSet<&String> = b.iter().collect();
    let inter = sa.intersection(&sb).count();
    let union = sa.union(&sb).count();
    if union == 0 {
        return 0.0;
    }
    inter as f64 / union as f64
}

/// 余弦相似度。长度不等或任一为零向量时返回 0.0（不 panic、不产生 NaN）。
/// 结果夹紧到 [-1, 1] 以吸收浮点噪声。
#[must_use]
pub fn cosine(a: &[f64], b: &[f64]) -> f64 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let dot: f64 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let na: f64 = a.iter().map(|x| x * x).sum::<f64>().sqrt();
    let nb: f64 = b.iter().map(|x| x * x).sum::<f64>().sqrt();
    if na == 0.0 || nb == 0.0 {
        return 0.0;
    }
    let c = dot / (na * nb);
    if c > 1.0 {
        1.0
    } else if c < -1.0 {
        -1.0
    } else {
        c
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokenize_basic() {
        assert_eq!(tokenize_zh("hello world"), vec!["hello", "world"]);
        // 中文逐字成 token
        assert_eq!(tokenize_zh("机器学习"), vec!["机", "器", "学", "习"]);
        // 混合 + 标点断词
        assert_eq!(
            tokenize_zh("Hello, 世界!"),
            vec!["hello", "世", "界"]
        );
        // 大小写归一
        assert_eq!(tokenize_zh("AbC"), vec!["abc"]);
    }

    #[test]
    fn jaccard_empty_full_partial() {
        assert_eq!(jaccard(&[], &[]), 0.0); // 双空
        let a = vec!["x".into(), "y".into()];
        let b = vec!["x".into(), "y".into()];
        assert_eq!(jaccard(&a, &b), 1.0); // 全等
        let c = vec!["y".into(), "z".into()];
        let expected = 1.0 / 3.0;
        assert!((jaccard(&a, &c) - expected).abs() < 1e-9); // 部分重叠
    }

    #[test]
    fn cosine_equal_orthogonal_mismatch_zero() {
        assert!((cosine(&[1.0, 0.0], &[1.0, 0.0]) - 1.0).abs() < 1e-9); // 全等
        assert!((cosine(&[1.0, 0.0], &[0.0, 1.0])).abs() < 1e-9); // 正交 ~0
        // 不等长
        assert_eq!(cosine(&[1.0], &[1.0, 2.0]), 0.0);
        // 零向量
        assert_eq!(cosine(&[0.0, 0.0], &[0.0, 0.0]), 0.0);
        // 非正交近似
        let v = cosine(&[1.0, 2.0, 3.0], &[4.0, 5.0, 6.0]);
        assert!(v > 0.97 && v < 0.98);
    }
}
