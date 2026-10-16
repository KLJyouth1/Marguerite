//! Query 蒸馏（P1-1）：把用户「原始整句」剥离交付类指令与称呼语，得到真正的检索词。
//!
//! 归因（架构师实测复现）：用户截图故障的**直接主因**是「整句原样当检索词」——
//! 把「搜索今天的AI新闻，并整理成html网页再成PPT给我看」整句丢给 `cn.bing`，会返回
//! 一屏「搜索引擎首页」（`sogou.com` / `so.com` / `baidu.com`…），因为它们恰好匹配了
//! 整句里的「搜索」等词。只换引擎 URL **不能**解决；必须从源头把交付指令剥离。
//!
//! 本模块是**纯规则、离线可测、零 LLM** 的确定性瘦身：只做「整句 → 检索词」的裁剪，
//! 不做语义改写、不引入相关性打分。防过度：若整句被裁空，则**原样保留**输入，绝不返回空。

/// 从句分隔符（强标点）。交付指令通常自成一句，按标点切句后整句丢弃即可。
const CLAUSE_SEPARATORS: &[char] = &[
    '，', ',', '。', '；', ';', '！', '!', '？', '?', '、', '…', '\n', '\r', '\t',
];

/// 交付类指令「构词」关键词：命中即判定该从句不是检索意图，整段丢弃。
///
/// 刻意**只收构词**（如 `整理成` / `做成`）而**不收裸动词**（如 `整理` / `生成`），
/// 避免误杀把动词当主题的真检索词（例如「生成式AI」不应因含「生成」而被丢）。
const DELIVERY_KEYWORDS: &[&str] = &[
    "整理成",
    "整理为",
    "整理一下",
    "做成",
    "再成",
    "转成",
    "转换成",
    "转换为",
    "转为",
    "导出为",
    "导出成",
    "导出到",
    "导出",
    "写成",
    "编写成",
    "排版",
    "汇总成",
    "汇总为",
    "合并成",
    "合并为",
    "做一个",
    "做个",
    "做一份",
    "写一个",
    "写个",
    "写一份",
    "制作成",
    "制作一份",
    "渲染成",
    "输出为",
    "输出成",
    "保存为",
    "保存成",
    "打包成",
    "压缩成",
    "改成",
    "变成",
    "编辑成",
    "美化",
    "呈现成",
    "列成",
    "列一个",
    "给我看",
    "给我",
];

/// 前置检索动词（按最长优先匹配，只剥一次）。
const LEADING_VERBS: &[&str] = &[
    "帮我搜索一下",
    "帮我搜一下",
    "请帮我搜索",
    "请帮我搜",
    "帮我搜索",
    "帮我搜",
    "请搜索",
    "请帮我",
    "帮我",
    "帮忙",
    "麻烦",
    "我想要",
    "我要",
    "我想",
    "搜索一下",
    "搜一下",
    "查一下",
    "查询",
    "检索",
    "搜索",
    "搜搜",
    "查找",
    "搜",
    "查",
];

/// 尾部语气 / 交付残留（循环剥离，最长优先靠列表顺序近似）。
const TRAILING_NOISE: &[&str] = &[
    "给我看",
    "给我",
    "看一下",
    "看看",
    "谢谢",
    "一下",
    "拜托",
    "吧",
    "呢",
    "啊",
    "的",
];

/// 首尾连接词 / 残留标点（清理用）。
fn is_edge_noise(c: char) -> bool {
    matches!(
        c,
        '，' | ','
            | '。'
            | '；'
            | ';'
            | '、'
            | '…'
            | ' '
            | '\u{3000}'
            | '并'
            | '与'
            | '和'
            | '及'
            | '再'
    )
}

/// 从用户原始输入蒸馏出检索词。
///
/// - 空输入返回空串（由调用方决定回退到原始输入）。
/// - 若所有从句都被判为交付指令（裁空），则**原样返回输入**，绝不返回空。
pub fn distill_query(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    // 1) 按强标点切句，丢弃含交付指令构词的从句。
    let kept: Vec<&str> = trimmed
        .split(CLAUSE_SEPARATORS)
        .map(str::trim)
        .filter(|clause| !clause.is_empty())
        .filter(|clause| !DELIVERY_KEYWORDS.iter().any(|k| clause.contains(*k)))
        .collect();
    let mut text = if kept.is_empty() {
        trimmed.to_string()
    } else {
        kept.join(" ")
    };

    // 2) 剥掉前置检索动词（只剥一次；剥后为空则保留原样）。
    for verb in LEADING_VERBS {
        if let Some(rest) = text.strip_prefix(*verb) {
            let rest = rest.trim_start();
            if !rest.is_empty() {
                text = rest.to_string();
            }
            break;
        }
    }

    // 3) 循环剥掉尾部语气 / 交付残留。
    loop {
        let before = text.clone();
        for suffix in TRAILING_NOISE {
            if let Some(rest) = text.strip_suffix(*suffix) {
                let rest = rest.trim_end().to_string();
                if !rest.is_empty() {
                    text = rest;
                }
                break;
            }
        }
        if text == before {
            break;
        }
    }

    // 4) 清理首尾连接词 / 残留标点；清空则回退到当前文本。
    let edge_trimmed = text.trim_matches(is_edge_noise).trim();
    if edge_trimmed.is_empty() {
        text.trim().to_string()
    } else {
        edge_trimmed.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // 用户截图故障原文：整句交付指令必须被蒸馏掉。
    const RAW_SENTENCE: &str = "搜索今天的AI新闻，并整理成html网页再成PPT给我看";

    #[test]
    fn distill_strips_delivery_clauses_from_the_screenshot_sentence() {
        let q = distill_query(RAW_SENTENCE);
        assert_eq!(q, "今天的AI新闻", "got: {q:?}");
        for forbidden in ["做成PPT", "整理成html", "给我看", "再成PPT", "html网页"] {
            assert!(!q.contains(forbidden), "蒸馏结果不应含 {:?}: {q:?}", forbidden);
        }
    }

    #[test]
    fn distill_leaves_a_clean_query_untouched() {
        assert_eq!(distill_query("AI新闻"), "AI新闻");
        assert_eq!(distill_query("今天天气怎么样"), "今天天气怎么样");
        assert_eq!(distill_query("防城港"), "防城港");
    }

    #[test]
    fn distill_trims_surrounding_whitespace() {
        assert_eq!(distill_query("  今天的 AI 新闻  "), "今天的 AI 新闻");
    }

    #[test]
    fn distill_drops_trailing_delivery_clause_and_keeps_topic() {
        let q = distill_query("帮我搜索一下最新的特斯拉Model 3价格，整理成表格给我");
        assert!(q.contains("特斯拉"), "应保留检索主题: {q:?}");
        assert!(!q.contains("整理成表格"), "应剔除交付从句: {q:?}");
        assert!(!q.contains("给我"), "应剔除交付尾巴: {q:?}");
    }

    #[test]
    fn distill_never_returns_empty_for_non_empty_input() {
        // 纯交付指令（无检索意图）也要原样保留，避免把查询清空。
        assert_eq!(distill_query("整理成html网页"), "整理成html网页");
        assert_eq!(distill_query("给我看"), "给我看");
    }

    #[test]
    fn distill_empty_input_returns_empty() {
        assert_eq!(distill_query("   "), "");
        assert_eq!(distill_query(""), "");
    }

    #[test]
    fn distill_does_not_over_strip_topical_verbs() {
        // 「生成式AI」含裸动词「生成」，不应被当作交付指令剔除。
        assert_eq!(distill_query("生成式AI是什么"), "生成式AI是什么");
    }
}
