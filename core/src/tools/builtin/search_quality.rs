//! 检索质量门禁（P0-3）：识别「搜索引擎首页 / 空壳」这类假结果。
//!
//! 归因（架构师实测复现）：截图里 5 条"结果"全是 `sogou.com` / `cn.bing.com` /
//! `so.com` 等引擎首页——它们的标题/摘要其实是首页导航流水文字。浏览器通道抓的是
//! 渲染后 DOM 的 `innerText`（见 `web_search.rs` 深读路径），于是首页导航被当成正文。
//!
//! 口径**刻意保守**（只解决「引擎首页当结果」这一明确故障，不引入相关性打分，避免误杀真新闻）：
//!   1. 引擎首页域名黑名单 **且** 路径为空/根 → 低质；
//!   2. 标题为空 / 纯空白 → 低质；
//!   3. 摘要命中首页导航模板文案（slogan）→ 低质。
//!
//! 注意：**不含**裸词「搜索引擎」这类泛词（真正的新闻文章也会出现），否则会误杀。

use super::SearchResult;

/// 引擎首页域名黑名单（**仅当路径为空/根**才算首页，避免误杀内容页，
/// 例如 `baike.baidu.com/item/…` 是真实内容页，不应被剔除）。
/// host 已小写并去 `www.` 前缀后再比对。
const ENGINE_HOMEPAGE_HOSTS: &[&str] = &[
    "sogou.com",
    "m.sogou.com",
    "so.com",
    "m.so.com",
    "baidu.com",
    "m.baidu.com",
    "sm.cn",
    "m.sm.cn",
    "17so.cn",
    "google.com",
    "bing.com",
    "cn.bing.com",
    "haosou.com",
    "360.cn",
];

/// 首页导航 / 模板文案片段（slogan）。命中即判低质。
const NAV_TEMPLATE_PHRASES: &[&str] = &[
    "全球最大的中文搜索引擎",
    "上网从搜狗开始",
    "请输入搜索词",
    "精准、便捷的搜索服务",
    "互动式搜索引擎",
    "一站式搜索引擎",
    "搜索引擎大全",
    "多引擎聚合搜索",
    "为永远充满好奇心的人",
    "搜你想搜的",
];

/// 提取 url 的 host（小写、去 `www.` 前缀）与路径。
fn host_and_path(url: &str) -> (String, String) {
    let rest = url.split_once("://").map(|(_, r)| r).unwrap_or(url);
    let rest = rest.split(['#', '?']).next().unwrap_or(rest);
    match rest.find('/') {
        Some(i) => (normalize_host(&rest[..i]), rest[i..].to_string()),
        None => (normalize_host(rest), String::new()),
    }
}

/// 归一化 host：去 userinfo / 端口 / 结尾点号，小写，去 `www.` 前缀。
fn normalize_host(host: &str) -> String {
    let host = host.rsplit('@').next().unwrap_or(host);
    let host = host.split(':').next().unwrap_or(host);
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    host.strip_prefix("www.").unwrap_or(host.as_str()).to_string()
}

/// 该 url 是否为某搜索引擎的**首页**（域名在黑名单 **且** 路径为空/根）。
pub fn is_engine_homepage(url: &str) -> bool {
    if url.trim().is_empty() {
        return false;
    }
    let (host, path) = host_and_path(url);
    if !ENGINE_HOMEPAGE_HOSTS.contains(&host.as_str()) {
        return false;
    }
    path.trim_end_matches('/').is_empty()
}

/// 摘要是否命中首页导航模板文案。
pub fn is_nav_template(snippet: &str) -> bool {
    let s = snippet.trim();
    if s.is_empty() {
        return false;
    }
    NAV_TEMPLATE_PHRASES.iter().any(|p| s.contains(*p))
}

/// 单条结果是否低质（引擎首页 / 空标题 / 导航模板摘要）。
fn is_low_quality_result(r: &SearchResult) -> bool {
    if r.title.trim().is_empty() {
        return true;
    }
    if let Some(url) = r.url.as_deref() {
        if is_engine_homepage(url) {
            return true;
        }
    }
    is_nav_template(&r.snippet)
}

/// 整批结果是否**整体低质**：过半（严格多数）为引擎首页/空壳即判定为真，
/// 以便调用方（`run_web_search`）不再采纳浏览器通道、果断回退健康直连引擎。
///
/// 空批次返回 `false`——「无结果」由调用方各自处理，不属于本门禁职责。
pub fn is_low_quality(results: &[SearchResult]) -> bool {
    if results.is_empty() {
        return false;
    }
    let low = results.iter().filter(|r| is_low_quality_result(r)).count();
    low * 2 > results.len()
}

/// 过滤低质结果：返回 `(保留结果, 被剔除的原下标)`，下标供日志/排查使用。
pub fn filter_low_quality(results: Vec<SearchResult>) -> (Vec<SearchResult>, Vec<usize>) {
    let mut kept = Vec::with_capacity(results.len());
    let mut removed = Vec::new();
    for (index, result) in results.into_iter().enumerate() {
        if is_low_quality_result(&result) {
            removed.push(index);
        } else {
            kept.push(result);
        }
    }
    (kept, removed)
}

/// 浏览器检索通道的**熔断器**：连续拿到不可采纳结果后，暂时跳过该通道。
///
/// # 为什么需要（实测依据）
///
/// 2026-10-08 实测：一轮 office 交付任务里 `run_web_search` 被调用 **14 次**，
/// 且**每一次都先尝试浏览器通道、拿到低质结果（引擎首页/空壳）之后才回退直连引擎**，
/// 集中在 3 分钟内 —— 单轮白耗数分钟，且与任务本身无关（「创建 Excel」根本不需要联网）。
///
/// 浏览器通道失败几乎总是**环境性**的（引擎 DOM 改版、无头/无显示环境、站点风控），
/// 在一次会话内不会自愈；因此重复尝试的期望收益接近零，而代价是线性累积的耗时。
///
/// # 语义（纯状态机，可离线单测）
///
/// - 连续 `threshold` 次**不可采纳** → 打开熔断，冷却 `cooldown_ms`；
/// - 冷却期内 `should_skip` 为 `true`（调用方直接走直连引擎）；
/// - 冷却结束后自动允许重试一次；**只要出现一次可采纳结果即完全复位**。
///
/// 刻意不做「永久关闭」：环境可能被修复（装了 Chromium、引擎改回），
/// 所以冷却到点必须给一次机会 —— 否则会把可用通道误判成永久不可用。
#[derive(Debug)]
pub struct BrowserSearchCircuit {
    consecutive_bad: u32,
    open_until_ms: Option<i64>,
    threshold: u32,
    cooldown_ms: i64,
}

/// 默认策略：连续 3 次不可采纳即熔断，冷却 10 分钟。
///
/// 取 3 是为了容忍偶发单次失败（网络抖动）；取 10 分钟是因为这类环境性问题
/// 在一次任务内不会变化，而更短会让同一轮里反复试探。
pub const BROWSER_CIRCUIT_THRESHOLD: u32 = 3;
pub const BROWSER_CIRCUIT_COOLDOWN_MS: i64 = 10 * 60 * 1000;

impl BrowserSearchCircuit {
    pub fn new() -> Self {
        Self::with_policy(BROWSER_CIRCUIT_THRESHOLD, BROWSER_CIRCUIT_COOLDOWN_MS)
    }

    /// 显式策略版本（供单测用小阈值/短冷却驱动）。
    pub fn with_policy(threshold: u32, cooldown_ms: i64) -> Self {
        Self {
            consecutive_bad: 0,
            open_until_ms: None,
            threshold: threshold.max(1),
            cooldown_ms: cooldown_ms.max(0),
        }
    }

    /// 本次调用是否应**跳过**浏览器通道（直接走直连引擎）。
    pub fn should_skip(&self, now_ms: i64) -> bool {
        matches!(self.open_until_ms, Some(until) if now_ms < until)
    }

    /// 记录一次浏览器通道结果。`adoptable` 为真表示结果可采纳（复位）。
    pub fn record(&mut self, adoptable: bool, now_ms: i64) {
        if adoptable {
            self.consecutive_bad = 0;
            self.open_until_ms = None;
            return;
        }
        self.consecutive_bad = self.consecutive_bad.saturating_add(1);
        if self.consecutive_bad >= self.threshold {
            self.open_until_ms = Some(now_ms.saturating_add(self.cooldown_ms));
        }
    }

    /// 当前连续不可采纳次数（供日志/诊断）。
    pub fn consecutive_bad(&self) -> u32 {
        self.consecutive_bad
    }
}

impl Default for BrowserSearchCircuit {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sr(title: &str, url: &str, snippet: &str) -> SearchResult {
        SearchResult {
            title: title.to_string(),
            url: Some(url.to_string()),
            snippet: snippet.to_string(),
        }
    }

    /// 真实抓取样本（`curl -sL "https://cn.bing.com/search?q=<整句>&mkt=zh-CN"`，
    /// 整句 = 「搜索今天的AI新闻，并整理成html网页再成PPT给我看」）——
    /// 9 条结果里 7 条是引擎首页，与用户截图现象一致。
    fn low_quality_fixture() -> Vec<SearchResult> {
        vec![
            sr(
                "搜狗搜索引擎 - 上网从搜狗开始",
                "https://www.sogou.com/",
                "搜狗搜索是全球第三代互动式搜索引擎，支持微信公众号和文章搜索、知乎搜索、英文搜索及翻译等，通过自主研发的人工智能算法为 …",
            ),
            sr(
                "搜索 - Microsoft 必应",
                "https://cn.bing.com/",
                "使用 Microsoft 必应进行搜索，并利用 AI 的强大功能查找信息、浏览网页、图像、视频、地图等。为永远充满好奇心的人提供的智能搜 …",
            ),
            sr(
                "360搜索，SO靠谱",
                "https://www.so.com/",
                "搜你想搜的 搜全网 问AI 搜索 豆包 京东 淘宝 deepseek 腾讯视频 快手 4399小游戏 小红书 抖音 哔哩哔哩 凤凰网 163邮箱 qq邮箱 微博 …",
            ),
            sr(
                "一起搜 - 搜索引擎大全 | 一次搜索，全网直达",
                "https://www.17so.cn/",
                "一起搜是免费的多引擎聚合搜索工具，一次输入即可同时打开 Google、百度、Bing 等多个搜索引擎，支持自定义搜 …",
            ),
            sr(
                "百度一下",
                "https://m.baidu.com/",
                "全球最大的中文搜索引擎、致力于让网民更便捷地获取信息，找到所求。百度超过千亿的中文网页数据库，可以瞬间找到相关 ...",
            ),
            sr(
                "神马搜索",
                "https://m.sm.cn/",
                "神马搜索有什么特色？ 神马为移动而生，我们会聚焦在解决手机用户的需求和痛点，比如大家已经能够看到App搜索、小说搜索、周边 …",
            ),
            sr(
                "Sogou - 搜狗搜索",
                "https://m.sogou.com/",
                "搜狗搜索是中国领先的中文搜索引擎，提供精准、便捷的搜索服务。",
            ),
        ]
    }

    /// 真实抓取样本（`curl -sL "https://cn.bing.com/search?q=AI新闻&mkt=zh-CN"`）——
    /// 健康路径返回的真实站点，必须**保留**。
    fn healthy_fixture() -> Vec<SearchResult> {
        vec![
            sr(
                "每日AI资讯 - 猫目",
                "https://maomu.com/news",
                "1 小时前 · 每日AI资讯是您获取2025年人工智能领域最新动态的首选。 我们每日更新全球及国内AI行业资讯，涵盖AI大模型突破（ …",
            ),
            sr(
                "每日AI资讯、热点、动态、融资、产品发布 | AI工具集",
                "https://ai-bot.cn/daily-ai-news/",
                "1 天前 · 全球化AI互动娱乐平台海艺宣布完成超亿元B轮融资，由视觉中国、华盖创赢、祥峰投资联合领投。",
            ),
            sr(
                "2026.09.24 全球 AI 新闻日报 | AITNT",
                "https://www.aitntnews.com/ainews/zh-CN",
                "1 小时前 · 全球 AI 新闻日报 浏览 1125 天 、 共 29244+ 条 以 agent-first 构建、对所有技术栈开放的 AI 新闻。",
            ),
            sr(
                "AI 日报：今日 AI 新闻、产品更新与大模型动态 | TodayAI",
                "https://todayai.com.cn/ai-daily",
                "1 天前 · 每日整理 AI 新闻、AI 产品更新、大模型、Agent 与行业最新动态，快速了解今天值得关注的人工智能进展。",
            ),
            sr(
                "AI新闻快讯 - 最新 AI 行业动态热点 | AI工具网",
                "https://www.ai138.com/daily",
                "2026年9月11日 · AI新闻快讯聚合最新 AI 行业动态与热点，精选产品发布、融资、开源与模型资讯，持续更新。",
            ),
            sr(
                "每日AI资讯 - 最新人工智能动态（2026年09月24日更新）| AIFans",
                "https://aifans.net.cn/ai-news/",
                "1 小时前 · AIFans每日精选AI资讯，汇聚量子位、36氪等媒体最新人工智能报道，涵盖大模型发布、AI工具上线、行业融资等动态。",
            ),
            sr(
                "人工智能 - 最新人工智能实时滚动快讯，聚合所有人工智能 ...",
                "https://tags.news.sina.com.cn/%E4%BA%BA%E5%B7%A5%E6%99%BA%E8%83%BD",
                "1 小时前 · 人工智能最新快讯，新浪聚合所有人工智能相关的新闻快讯，并为你提供最新的相关资讯。",
            ),
            sr(
                "AI日报 - 每天三分钟关注AI行业趋势_AIbase",
                "https://www.aibase.com/zh/daily",
                "AI日报为您提供最新的人工智能行业资讯，每天仅需三分钟，全面掌握AI技术发展、行业动态和市场趋势。",
            ),
        ]
    }

    #[test]
    fn engine_homepages_are_detected() {
        for url in [
            "https://www.sogou.com/",
            "https://cn.bing.com/",
            "https://cn.bing.com",
            "https://www.so.com/",
            "https://www.17so.cn/",
            "https://m.baidu.com/",
            "https://m.sm.cn/",
            "https://m.sogou.com/",
        ] {
            assert!(is_engine_homepage(url), "应判为引擎首页: {url}");
        }
    }

    #[test]
    fn engine_results_and_content_pages_are_not_homepages() {
        // 引擎自身的**结果页**（带路径）不是首页
        assert!(!is_engine_homepage("https://cn.bing.com/search?q=ai"));
        // 内容页域名不同 → 即使根路径也不算首页
        assert!(!is_engine_homepage("https://www.toutiao.com/"));
        // 百度百科内容页（百度系域名 + 非根路径）必须放行
        assert!(!is_engine_homepage(
            "https://baike.baidu.com/item/%E6%90%9C%E7%B4%A2/5395421"
        ));
        assert!(!is_engine_homepage(""));
    }

    #[test]
    fn low_quality_fixture_is_flagged_and_fully_removed() {
        let low = low_quality_fixture();
        assert!(is_low_quality(&low), "整句返回的引擎首页批次应判为整体低质");
        let (kept, removed) = filter_low_quality(low);
        assert!(kept.is_empty(), "引擎首页应全部剔除: {kept:?}");
        assert_eq!(removed, vec![0, 1, 2, 3, 4, 5, 6]);
    }

    #[test]
    fn healthy_fixture_is_preserved() {
        let healthy = healthy_fixture();
        assert!(
            !is_low_quality(&healthy),
            "真实站点批次不应判为低质（避免误杀真新闻）"
        );
        let (kept, removed) = filter_low_quality(healthy);
        assert_eq!(kept.len(), 8, "真实站点应全部保留");
        assert!(removed.is_empty(), "不应剔除任何真实站点: {removed:?}");
    }

    #[test]
    fn nav_template_snippet_is_flagged_even_on_unknown_host() {
        // 域名不在黑名单，但摘要是首页导航 slogan → 低质
        let r = sr(
            "某导航站",
            "https://mirror.example.com/",
            "请输入搜索词，全球最大的中文搜索引擎为您服务",
        );
        assert!(is_low_quality(&[r]));
    }

    #[test]
    fn empty_title_is_low_quality() {
        let r = sr("   ", "https://news.example.com/a/1", "有摘要但无标题");
        assert!(is_low_quality(&[r]));
    }

    #[test]
    fn real_article_mentioning_search_engine_is_kept() {
        // 反例护栏：真实文章摘要里出现「搜索引擎」泛词不应被误杀
        let r = sr(
            "AI 搜索引擎算法更新解读",
            "https://www.example-news.com/tech/ai-search",
            "本文分析主流搜索引擎近期的算法变化及其对内容生态的影响。",
        );
        assert!(!is_low_quality(std::slice::from_ref(&r)));
        let (kept, removed) = filter_low_quality(vec![r]);
        assert_eq!(kept.len(), 1);
        assert!(removed.is_empty());
    }

    #[test]
    fn empty_batch_is_not_low_quality() {
        assert!(!is_low_quality(&[]));
    }

    #[test]
    fn circuit_opens_after_threshold_and_skips_during_cooldown() {
        let mut c = BrowserSearchCircuit::with_policy(3, 1000);
        assert!(!c.should_skip(0), "初始不应跳过");
        // 前两次失败还不该熔断（容忍偶发抖动）
        c.record(false, 0);
        c.record(false, 0);
        assert!(!c.should_skip(0), "未达阈值不得熔断");
        // 第三次达到阈值 → 打开
        c.record(false, 0);
        assert!(c.should_skip(0), "达阈值后应跳过浏览器通道");
        assert_eq!(c.consecutive_bad(), 3);
        // 冷却期内持续跳过
        assert!(c.should_skip(999), "冷却期内应跳过");
        // 冷却结束 → 必须给一次重试机会（环境可能已修好）
        assert!(!c.should_skip(1000), "冷却到点必须允许重试");
    }

    #[test]
    fn circuit_resets_on_adoptable_result() {
        let mut c = BrowserSearchCircuit::with_policy(2, 1000);
        c.record(false, 0);
        c.record(false, 0);
        assert!(c.should_skip(0));
        // 一次可采纳结果即完全复位
        c.record(true, 0);
        assert!(!c.should_skip(0), "成功后必须复位");
        assert_eq!(c.consecutive_bad(), 0);
    }

    #[test]
    fn circuit_does_not_count_successes_toward_threshold() {
        let mut c = BrowserSearchCircuit::with_policy(3, 1000);
        // 交错成功不应累积（否则会误熔断可用通道）
        c.record(false, 0);
        c.record(true, 0);
        c.record(false, 0);
        c.record(true, 0);
        c.record(false, 0);
        assert!(!c.should_skip(0), "交错成功不得累积到阈值");
    }

    #[test]
    fn circuit_default_is_not_open() {
        let c = BrowserSearchCircuit::new();
        assert!(!c.should_skip(i64::MAX), "全新状态永不跳过");
        assert_eq!(c.consecutive_bad(), 0);
    }
}
