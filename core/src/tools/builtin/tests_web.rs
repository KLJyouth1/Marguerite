//! Tests for builtin tools (split from original tests.rs)

use super::*;

use super::tests::{setup, put};

/// 词典/翻译噪音门禁：用**真实抓取样本**锁定（2026-09-24 cn.bing 实测）。
///
/// 原始 8 条里有 4 条是词典词条，子智能体的实际反应是「搜索受到中文词典结果干扰」，
/// 随后改道 web_fetch 直抓并白烧十余秒。门禁必须精确剔掉这 4 条、保留技术站点。
#[test]
fn dictionary_noise_gate_drops_real_bing_dictionary_results() {
    let mk = |title: &str, url: &str| SearchResult {
        title: title.to_string(),
        url: Some(url.to_string()),
        snippet: String::new(),
    };
    // 真实样本（标题逐字取自 cn.bing 抓取结果）
    let results = vec![
        mk("Three.js – JavaScript 3D 库 - Three.js 3D 库", "https://threejs.org/"),
        mk(
            "three是什么意思_three的翻译_音标_读音_用法_例句_爱词霸 ...",
            "https://www.iciba.com/word?w=three",
        ),
        mk("Three.js – JavaScript 3D Library", "https://threejs.org/"),
        mk(
            "THREE中文 (简体)翻译：剑桥词典",
            "https://dictionary.cambridge.org/zhs/词典/英语-汉语-简体/three",
        ),
        mk(
            "three（表示数字3的英文单词）_百度百科",
            "https://baike.baidu.com/item/three",
        ),
        mk("Three.js中文网", "http://www.webgl3d.cn/"),
    ];
    let query = "Three.js floating island 3D scene tutorial low poly";
    let (kept, removed) = filter_dictionary_noise(results, query);
    assert_eq!(removed.len(), 3, "应剔掉 3 条词典条目，实际 {:?}", removed);
    assert_eq!(kept.len(), 3);
    // 技术站点必须保留
    assert!(kept.iter().any(|r| r.title.contains("Three.js – JavaScript 3D Library")));
    assert!(kept.iter().any(|r| r.title == "Three.js中文网"));
    // 保留的条目里不得有词典噪音（防误杀反向断言）
    for x in kept.iter() {
        assert!(!is_dictionary_noise(x), "误杀真实结果: {}", x.title);
    }
}

/// 反向守卫：查询本身在查词义时，词典结果**必须保留**（不能把「查词典」也拦掉）。
#[test]
fn dictionary_gate_yields_when_query_asks_for_meaning() {
    assert!(query_seeks_definition("serendipity 意思"));
    assert!(query_seeks_definition("define serendipity"));
    assert!(query_seeks_definition("three 翻译"));
    assert!(!query_seeks_definition(
        "Three.js floating island 3D scene tutorial low poly"
    ));

    let results = vec![SearchResult {
        title: "three是什么意思_three的翻译".to_string(),
        url: Some("https://www.iciba.com/word?w=three".to_string()),
        snippet: String::new(),
    }];
    let (kept, removed) = filter_dictionary_noise(results, "three 是什么意思");
    assert_eq!(removed.len(), 0, "查词义时不得剔除词典结果");
    assert_eq!(kept.len(), 1);
}

/// 词典域黑名单本身也要生效（标题不含任何特征词，但域名是词典站）。
#[test]
fn dictionary_gate_drops_by_host_even_without_title_marker() {
    let r = SearchResult {
        title: "three - 牛津高阶词典".to_string(), // 命中「词典」；再用纯英文标题验证 host 规则
        url: Some("https://dict.youdao.com/result?word=three".to_string()),
        snippet: String::new(),
    };
    assert!(is_dictionary_noise(&r));
    let plain = SearchResult {
        title: "three".to_string(),
        url: Some("https://dict.youdao.com/result?word=three".to_string()),
        snippet: String::new(),
    };
    assert!(is_dictionary_noise(&plain), "纯标题也应按域名判定为词典噪音");
    let normal = SearchResult {
        title: "three".to_string(),
        url: Some("https://threejs.org/docs/".to_string()),
        snippet: String::new(),
    };
    assert!(!is_dictionary_noise(&normal));
}


/// 搜索市场必须跟随查询语言。
///
/// 注意：这条**不能**修掉词典噪音（实测 mkt/setlang/ensearch 三种改法结果逐字相同，
/// 真正的拦截在 `dictionary_noise_gate_*`）。它修的是另一件事：英文查询不该被安上
/// 中文市场偏好，否则排序天然偏向中文站点。
#[test]
fn search_market_follows_query_language() {
    // 英文技术查询 → 英文市场
    assert_eq!(
        super::web_search::search_market("Three.js floating island 3D scene tutorial low poly"),
        ("en-US", "en")
    );
    // 纯数字/符号也算英文市场（没有 CJK）
    assert_eq!(super::web_search::search_market("three.js r185"), ("en-US", "en"));
    // 中文查询 → 中文市场
    assert_eq!(super::web_search::search_market("今天的 AI 新闻"), ("zh-CN", "zh-hans"));
    // 中英混排：含 CJK 即走中文市场（判据可预测，不做比例阈值）
    assert_eq!(
        super::web_search::search_market("Three.js 中文文档"),
        ("zh-CN", "zh-hans")
    );
    // 日文假名 / 韩文同样归入东亚市场
    assert_eq!(super::web_search::search_market("スリージェイエス"), ("zh-CN", "zh-hans"));
    assert_eq!(super::web_search::search_market("쓰리제이에스"), ("zh-CN", "zh-hans"));
    // 空查询不 panic，按英文市场兜底
    assert_eq!(super::web_search::search_market(""), ("en-US", "en"));
}

    #[test]
    fn parse_ddg_results_extracts_links() {
        let html = r#"<div class="result__body">
            <a rel="nofollow" class="result__a" href="https://example.com/rust?q=1&amp;r=2">Rust 语言官网</a>
            <a class="result__snippet" href="https://example.com/rust">Rust 是一门系统编程语言</a>
        </div><div class="result__body">
            <a rel="nofollow" class="result__a" href="https://example.org/two">第二个结果</a>
        </div>"#;
        let res = parse_ddg_results(html, 5);
        assert_eq!(res.len(), 2);
        assert_eq!(res[0].title, "Rust 语言官网", "got: {:?}", res[0]);
        assert_eq!(
            res[0].url.as_deref(),
            Some("https://example.com/rust?q=1&r=2"),
            "got: {:?}",
            res[0]
        );
        assert!(res[0].snippet.contains("系统编程语言"), "got: {:?}", res[0]);
        assert_eq!(res[1].title, "第二个结果", "got: {:?}", res[1]);
    }

    #[test]
    fn parse_ddg_results_respects_max_and_empty() {
        assert!(parse_ddg_results("<html>no results</html>", 5).is_empty());
        let html = r#"<div class="result__body"><a class="result__a" href="https://a.b">1</a></div>
            <div class="result__body"><a class="result__a" href="https://c.d">2</a></div>"#;
        let res = parse_ddg_results(html, 1);
        assert_eq!(res.len(), 1);
    }

    #[test]
    fn parse_bing_results_extracts_links() {
        let html = r#"<ol id="b_results"><li class="b_algo">
            <h2><a href="https://www.example.com/rust?q=1&amp;r=2">Rust 编程语言介绍</a></h2>
            <p>Rust 是一门注重性能与安全的系统编程语言。</p>
        </li><li class="b_algo">
            <h2><a href="https://www.example.org/bing">第二个结果</a></h2>
            <p>这里没有摘要。</p>
        </li><li class="b_pag"></li></ol>"#;
        let res = parse_bing_results(html, 5);
        assert_eq!(res.len(), 2, "got: {:?}", res);
        assert_eq!(res[0].title, "Rust 编程语言介绍", "got: {:?}", res[0]);
        assert_eq!(
            res[0].url.as_deref(),
            Some("https://www.example.com/rust?q=1&r=2"),
            "got: {:?}",
            res[0]
        );
        assert!(res[0].snippet.contains("系统编程语言"), "got: {:?}", res[0]);
        assert_eq!(res[1].title, "第二个结果", "got: {:?}", res[1]);
        let limited = parse_bing_results(html, 1);
        assert_eq!(limited.len(), 1);
    }

    #[test]
    fn extract_readable_removes_noise_and_keeps_main() {
        let html = r##"<!DOCTYPE html><html><head>
            <title>今日实时要闻</title>
            <meta name="description" content="每日实时新闻聚合，覆盖全球科技与财经。">
            <script>var bad = '被剔除';</script><style>.x{color:red}</style>
        </head><body>
            <nav><a href="#">导航一</a><a href="#">导航二</a></nav>
            <main><article>
                <h1>头条标题</h1>
                <p>第一段：这里是最新科技动态正文内容。</p>
                <p>第二段：继续报道详细进展与影响分析。</p>
            </article></main>
            <footer>版权信息 &amp; 备案</footer>
        </body></html>"##;
        let out = extract_readable(html, 5000);
        assert!(out.contains("标题: 今日实时要闻"), "got: {out}");
        assert!(
            out.contains("描述: 每日实时新闻聚合，覆盖全球科技与财经。"),
            "got: {out}"
        );
        assert!(
            out.contains("第一段：这里是最新科技动态正文内容。"),
            "got: {out}"
        );
        assert!(
            out.contains("第二段：继续报道详细进展与影响分析。"),
            "got: {out}"
        );
        // 噪音与脚本被剔除
        assert!(!out.contains("导航一"), "导航应被剔除: {out}");
        assert!(!out.contains("版权信息"), "footer 应被剔除: {out}");
        assert!(!out.contains("被剔除"), "script 应被剔除: {out}");
        assert!(!out.contains("<p>"), "不应残留标签: {out}");
    }

    #[test]
    fn looks_like_html_detects_html_documents() {
        assert!(looks_like_html("<!doctype html><html>..."));
        assert!(looks_like_html("<html lang='zh'><head><title>t</title>"));
        // JSON 不以 < 开头，即使内含 <body> 子串也不误判为 HTML
        assert!(!looks_like_html("{\"title\":\"<body>\"}"));
        assert!(!looks_like_html("{\"a\":1,\"text\":\"纯 JSON\"}"));
        assert!(!looks_like_html("纯文本内容，没有任何标记"));
    }

    #[tokio::test]
    async fn web_search_requires_query() {
        let (reg, dir) = setup();
        let r = reg
            .execute_local_tool("web_search", &HashMap::new())
            .await
            .unwrap();
        assert!(r.is_error(), "empty query must be rejected");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn browser_requires_url_and_action() {
        let (reg, dir) = setup();
        let r = reg
            .execute_local_tool("browser", &HashMap::new())
            .await
            .unwrap();
        assert!(r.is_error(), "missing action/url must be rejected");

        let mut a = HashMap::new();
        put(&mut a, "action", "open");
        let r = reg.execute_local_tool("browser", &a).await.unwrap();
        assert!(r.is_error(), "missing url must be rejected");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn browser_unavailable_reports_clear_error() {
        let bridge = BrowserBridge::new();
        if bridge.is_available() {
            return; // 环境有 Chrome 时跳过（真实打开依赖外网，不纳入单元测试）
        }
        let (reg, dir) = setup();
        let mut a = HashMap::new();
        put(&mut a, "action", "open");
        put(&mut a, "url", "https://example.com");
        let r = reg.execute_local_tool("browser", &a).await.unwrap();
        match r {
            ToolResult::Error { code, .. } => assert_eq!(code, "NOT_AVAILABLE"),
            _ => panic!("expected NOT_AVAILABLE, got success"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    // P0-3：低质（引擎首页/空壳）结果**不得**被采纳 → 触发回退直连引擎
    // （等价于 `run_web_search` 运行时日志出现「回退直连引擎」）。
    #[test]
    fn low_quality_browser_search_results_trigger_fallback() {
        let low: Vec<SearchResult> = ["https://www.sogou.com/", "https://cn.bing.com/", "https://www.so.com/"]
            .iter()
            .map(|url| SearchResult {
                title: format!("{url} 首页"),
                url: Some((*url).to_string()),
                snippet: "全球最大的中文搜索引擎".to_string(),
            })
            .collect();
        assert!(is_low_quality(&low), "引擎首页批次应判为低质");
        assert!(!browser_results_adoptable(&low), "低质结果不得被采纳（须回退直连引擎）");

        // 健康结果：采纳，不回退。
        let healthy = vec![SearchResult {
            title: "AI 日报：今日 AI 新闻".to_string(),
            url: Some("https://ai-bot.cn/daily-ai-news/".to_string()),
            snippet: "最新 AI 行业动态与产品发布".to_string(),
        }];
        assert!(browser_results_adoptable(&healthy), "真实站点应被采纳");

        // 空结果：不可采纳（走「空结果回退」分支）。
        assert!(!browser_results_adoptable(&[]), "空结果不可采纳");
    }
