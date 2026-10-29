//! 记忆检索的**唯一**分词入口。
//!
//! # 为什么存在
//!
//! 此前 `long_term::tokenize` 与 `flyhash::tokenize` 各抄了一份同样的实现，注释里写着
//! 「与 long_term 的 tokenize 相同口径，保证精排一致性」——**靠注释维持的口径一致必然漂移**，
//! 而中文召回质量恰好取决于两处是否同步（实测：中文改述查询的全库最佳余弦中位 0.346，
//! 正好卡在门 0.35 之下，于是整档被饿死）。
//!
//! 口径只在这里定义一次：
//! - [`tokens`]：嵌入的**主通道**词元 —— 拉丁词（ascii 字母数字串，小写），加上非 ascii
//!   字母的单字与**相邻二字组合**。中文的词大多是两字，只喂单字等于把「审批」「卡片」拆成
//!   「审」「批」——桶被单字占满，区分力全靠运气。
//! - [`latin_bigrams`]：嵌入的**弱通道**（相邻拉丁词对，权重 0.5）。
//! - [`content_words`]：**实词**口径（去停用词、去纯数字、去单字母）。用于
//!   「共享实词数」这一路检索打分，也用于回归夹具的自证断言（零词面重合的改述）。
//!
//! 分词刻意**不引入外部词典/分词器**：可选依赖会让「装了才绿、没装就红」
//! （`pyengine` 的 tiktoken 教训已付过学费）。
//!
//! # 口径版本号
//!
//! [`TOKENIZER_VERSION`] 用来给**已落盘的向量**钉口径：换分词而维度不变时，
//! 旧的「维度不符自动重建」结构上看不见任何变化，历史向量会被静默当作同口径数据
//! 参与余弦。`long_term` 把它写进库里的 `memory_meta` 行，失配即重建一次。

use std::collections::HashSet;

/// 分词口径版本。**改动 [`tokens`] / [`latin_bigrams`] 的输出时必须一起改它。**
///
/// 命名规则：`vN`（N 单调递增）+ 一句人话说明变了什么，便于日志里回溯「这批向量是哪一代」。
pub const TOKENIZER_VERSION: &str = "v2-cjk-bigram";

/// 英文停用词（功能词）。口径与回归夹具的自证断言共用，改动会让夹具的
/// 「零重合」证明变松或变严——两边必须一起看。
const EN_STOPWORDS: &[&str] = &[
    "a", "an", "the", "of", "in", "on", "at", "to", "for", "and", "or", "but", "if", "is", "are",
    "was", "were", "be", "been", "being", "this", "that", "these", "those", "it", "its", "as",
    "by", "with", "without", "from", "into", "over", "under", "not", "no", "nor", "so", "than",
    "then", "there", "here", "they", "them", "their", "we", "you", "your", "he", "she", "his",
    "her", "i", "me", "my", "our", "us", "one", "two", "every", "each", "all", "any", "some",
    "such", "only", "just", "very", "more", "most", "other", "when", "where", "why", "how",
    "what", "which", "who", "whom", "whose", "do", "does", "did", "doing", "have", "has", "had",
    "having", "would", "could", "should", "may", "might", "must", "shall", "can", "will", "need",
    "before", "after", "during", "while", "since", "until", "because", "though", "although",
    "once", "twice", "again", "never", "always", "often", "seldom", "few", "many", "much", "lot",
];

/// 中日韩功能字（单字粒度；双字组合一律视为实词，因为「状态」「卡片」正是检索目标）。
const CJK_STOPCHARS: &str =
    "的了在是有和与就都而及把被让这那我他它它们的一个不没要能会对从以之也很更最还只\
     其此该等吗呢吧啊呀哦嗯以及因为所以但是如果虽然如或或者还是到过得给为以已再又也许\
     下上中里外前后多小数";

fn is_cjk_stop_char(ch: char) -> bool {
    CJK_STOPCHARS.contains(ch)
}

/// 主通道词元：拉丁词 + 非 ascii 字母单字 + 相邻非 ascii 字母二字组合。
///
/// 返回顺序即文本顺序（供嵌入的弱通道判断相邻关系时使用 `latin_bigrams`，
/// 而不是在本列表上做 `windows` —— 本列表里相邻的两项可能是「词 + 字」这种垃圾对）。
pub fn tokens(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut word = String::new();
    // 前一个「非 ascii 字母」字符（用于拼二字组）；遇到任何分隔符或拉丁字符即失效
    let mut prev_other: Option<char> = None;

    for ch in text.chars() {
        if ch.is_ascii_alphanumeric() {
            word.push(ch);
            prev_other = None;
            continue;
        }
        if !word.is_empty() {
            out.push(std::mem::take(&mut word).to_lowercase());
        }
        if ch.is_alphabetic() && !ch.is_ascii() {
            let s = ch.to_string();
            out.push(s.clone());
            if let Some(p) = prev_other {
                out.push(format!("{p}{s}"));
            }
            prev_other = Some(ch);
        } else {
            prev_other = None;
        }
    }
    if !word.is_empty() {
        out.push(word.to_lowercase());
    }
    out
}

/// 弱通道词元：相邻**拉丁词**对（`"hello world"`）。
///
/// 只在拉丁词之间建立，不跨 CJK 字——旧实现在混合流上做 `windows(2)`，
/// 于是会产出「最后一个英文词 + 第一个汉字」这种永远匹配不上的桶。
pub fn latin_bigrams(text: &str) -> Vec<String> {
    let words: Vec<String> = text
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(|w| w.to_lowercase())
        .collect();
    words.windows(2).map(|w| format!("{} {}", w[0], w[1])).collect()
}

/// 实词集合（去重、去停用词、去纯数字、去单拉丁字母）。
///
/// 与 [`tokens`] 同源：单字与二字组都参与，所以中文「共享实词数」实际是
/// 「共享的字与二字词的数量」——二字词的区分力更高，正是检索要的那个信号。
pub fn content_words(text: &str) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for t in tokens(text) {
        let chars: Vec<char> = t.chars().collect();
        let latin_first = chars.first().is_some_and(|c| c.is_ascii());
        if chars.iter().all(|c| c.is_ascii_digit()) {
            continue;
        }
        if latin_first {
            if chars.len() < 2 || EN_STOPWORDS.contains(&t.as_str()) {
                continue;
            }
        } else if chars.len() == 1 && is_cjk_stop_char(chars[0]) {
            continue;
        }
        if seen.insert(t.clone()) {
            out.push(t);
        }
    }
    out
}

/// 共享实词数（路 A 的相关性度量：朴素、可解释，实测打赢了单通道余弦）。
pub fn shared_content_words(query_words: &[String], doc_words: &[String]) -> usize {
    let doc: HashSet<&str> = doc_words.iter().map(|s| s.as_str()).collect();
    query_words.iter().filter(|q| doc.contains(q.as_str())).count()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn latin_words_are_lowercased_and_split_on_punctuation() {
        assert_eq!(tokens("X-Forwarded-For line"), ["x", "forwarded", "for", "line"]);
    }

    #[test]
    fn cjk_yields_unigrams_and_adjacent_bigrams_in_text_order() {
        // 「受光面」= 3 字 + 2 二字组，二字组是中文检索真正需要的单位
        assert_eq!(
            tokens("受光面"),
            ["受", "光", "受光", "面", "光面"].to_vec()
        );
    }

    #[test]
    fn bigrams_do_not_cross_a_separator() {
        // 「审批」「卡片」各自成词，但「批卡」这种跨空格的组合绝不产出
        assert_eq!(
            tokens("审批 卡片"),
            ["审", "批", "审批", "卡", "片", "卡片"].to_vec()
        );
    }

    #[test]
    fn mixed_text_does_not_pair_a_word_with_a_han_char() {
        // 旧实现：[REST, 路, 径] → windows 产出 "REST 路" 这种永不复现的桶
        assert!(latin_bigrams("REST 路径").is_empty());
        assert_eq!(latin_bigrams("call the API"), ["call the", "the api"].to_vec());
    }

    /// `Vec<String>` 的 `contains` 要的是 `&String`，写 `"route"` 编译不过；
    /// 这个可读性补丁比满屏 `&"...".to_string()` 干净。
    fn has(words: &[String], needle: &str) -> bool {
        words.iter().any(|w| w == needle)
    }

    #[test]
    fn content_words_drop_stopwords_digits_and_single_letters() {
        let w = content_words("the 501 route is 审批");
        assert!(has(&w, "route"), "{w:?}");
        assert!(has(&w, "审批"), "{w:?}");
        assert!(has(&w, "审") && has(&w, "批"), "{w:?}");
        assert!(!has(&w, "the"), "{w:?}");
        assert!(!has(&w, "is"), "{w:?}");
        assert!(!has(&w, "501"), "{w:?}");
    }

    #[test]
    fn shared_word_count_is_a_set_overlap() {
        let q = content_words("审批 卡片 折叠");
        let d = content_words("审批 卡片 重连 以后 自己 折叠");
        assert!(shared_content_words(&q, &d) >= 3, "至少命中三个词面单位");
        let other = content_words("npm lockfile 依赖");
        assert_eq!(shared_content_words(&q, &other), 0);
    }
}
