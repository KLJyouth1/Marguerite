//! 内置浏览器资料采集 / 分析 / 测试（agent 浏览器工具，无外部 API）
//!
//! 复用 [`super::BrowserBridge`]（CDP 驱动真实 Chromium）做三件事，全部**不依赖任何
//! 搜索/抓取 API key**，直接在网络上用智能体自己的浏览器通道完成：
//!
//! - [`collect_page`]：导航 → 在页面上下文用 JS 抽取可读文本/标题/链接/标题层级 →
//!   结构化返回（资料采集分析 / 浏览器查看）。
//! - [`browser_search`]：在真实浏览器里打开搜索引擎、从**渲染后** DOM 抽取结果——
//!   比 `tools::builtin::run_web_search` 的 HTML 字符串爬取更抗反爬，且无需 search API key。
//! - [`run_browser_test`]：对页面跑 DOM 断言（标题/文本/元素存在/计数），供智能体做
//!   浏览器自动化测试。
//!
//! 无 Chromium 时，调用方应回退到 `tools::builtin::run_web_search` 的直接抓取路径。

use std::time::Duration;

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::BrowserBridge;

/// 采集到的页面结构化摘要。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PageDigest {
    pub title: String,
    pub url: String,
    pub text: String,
    pub headings: Vec<String>,
    pub links: Vec<PageLink>,
}

/// 页面内抽取出的链接。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PageLink {
    pub text: String,
    pub href: String,
}

impl PageDigest {
    /// 截断正文到 `max_chars`（用于深读 / 摘要场景）。
    pub fn summarize(&self, max_chars: usize) -> String {
        if self.text.len() <= max_chars {
            self.text.clone()
        } else {
            format!("{}…", &self.text[..max_chars.saturating_sub(1)])
        }
    }
}

/// 页面上下文抽取 JS（返回 JSON 字符串）。纯函数，离线可测。
///
/// **必须带 `textContent` 兜底**：`innerText` 依赖布局（layout）计算，而本通道跑的是
/// `--headless=new --disable-gpu` 的无窗口 Chromium。实测（2026-10-23，threejs.org）
/// `document.readyState === "complete"` 且节点数正常，但 `document.body.innerText`
/// 长度恒为 **0** —— 于是 `web_fetch`/`web_research` 拿到空正文，子智能体以为抓取失败，
/// 反复换 URL 重试（真机截图里连续 6 次 web_fetch 全部空转）。
/// `textContent` 不依赖布局，是可靠兜底。
pub fn collect_page_js() -> &'static str {
    r#"(() => {
  const txt = (el) => el ? (el.innerText || el.textContent || "") : "";
  const title = document.title || "";
  const body = document.body;
  const text = txt(body).replace(/\s+/g, " ").trim();
  const headings = Array.from(document.querySelectorAll("h1,h2,h3"))
    .map(h => txt(h).replace(/\s+/g, " ").trim())
    .filter(Boolean);
  const links = Array.from(document.querySelectorAll("a[href]"))
    .map(a => ({ text: txt(a).replace(/\s+/g, " ").trim(), href: a.href }))
    .filter(l => l.href && l.href.startsWith("http"));
  return JSON.stringify({ title, text, headings, links, ready: document.readyState });
})()"#
}

/// 从 [`collect_page_js`] 的结果 JSON 解析出 [`PageDigest`]（纯函数，离线可测）。
pub fn extract_digest_from_json(raw: &str, url: &str) -> Result<PageDigest> {
    let v: Value = serde_json::from_str(raw).map_err(|e| anyhow!("解析页面摘要 JSON 失败: {e}"))?;
    Ok(PageDigest {
        title: v
            .get("title")
            .and_then(|s| s.as_str())
            .unwrap_or("")
            .to_string(),
        url: url.to_string(),
        text: v
            .get("text")
            .and_then(|s| s.as_str())
            .unwrap_or("")
            .to_string(),
        headings: v
            .get("headings")
            .and_then(|a| a.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default(),
        links: v
            .get("links")
            .and_then(|a| a.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|x| {
                        let text = x
                            .get("text")
                            .and_then(|s| s.as_str())
                            .unwrap_or("")
                            .to_string();
                        let href = x
                            .get("href")
                            .and_then(|s| s.as_str())
                            .unwrap_or("")
                            .to_string();
                        if href.starts_with("http") {
                            Some(PageLink { text, href })
                        } else {
                            None
                        }
                    })
                    .collect()
            })
            .unwrap_or_default(),
    })
}

/// 正文可读下限：低于该字符数视为「首屏还没渲染出来」，继续轮询等待。
const MIN_READABLE_CHARS: usize = 200;
/// 就绪轮询：最多 10 轮 × 300ms ≈ 3s。SPA / 重资源页首屏常超过固定 600ms，
/// 但也不该无限等——3s 未出正文就交给调用方回退（reqwest 直抓）。
const READINESS_ATTEMPTS: usize = 10;
const READINESS_INTERVAL: Duration = Duration::from_millis(300);

/// 抽取结果里的 `document.readyState` 是否为 `complete`（纯函数，离线可测）。
///
/// 用于「短页面提前收工」：正文不足 [`MIN_READABLE_CHARS`] 但文档已 complete 且
/// 有正文时，没必要再轮询到 3s。
fn page_is_complete(raw: &str) -> bool {
    serde_json::from_str::<Value>(raw)
        .ok()
        .and_then(|v| v.get("ready").and_then(|r| r.as_str()).map(|s| s == "complete"))
        .unwrap_or(false)
}

/// 用内置浏览器导航并采集页面（无外部 API）。需本机有 Chromium。
///
/// 固定 sleep 换成**就绪轮询**：此前写死 600ms 就抽正文，重资源页抽到空串
/// （实测 threejs.org 首帧正文为空），工具却返回「成功但零正文」，子智能体无法判断
/// 是「页面没内容」还是「抓取没生效」，于是反复重试白烧预算。
/// 现在等到正文达到可读下限再返回；确实为空则**明确报错**，让调用方走回退路径。
pub async fn collect_page(bridge: &BrowserBridge, url: &str) -> Result<PageDigest> {
    let mut page = bridge.open(url).await?;
    let mut digest = PageDigest {
        title: String::new(),
        url: url.to_string(),
        text: String::new(),
        headings: Vec::new(),
        links: Vec::new(),
    };
    for _ in 0..READINESS_ATTEMPTS {
        tokio::time::sleep(READINESS_INTERVAL).await;
        let raw = page
            .session
            .evaluate(collect_page_js(), true)
            .await
            .map_err(|e| anyhow!("采集页面 JS 求值失败: {e}"))?;
        digest = extract_digest_from_json(&raw, url)?;
        let complete = page_is_complete(&raw);
        // 两种收工条件：
        //  - 正文够长（重资源页即使还没 idle，正文也已可读）；
        //  - 文档已 complete 且已有正文（短页面不必陪跑满 3s）。
        if digest.text.chars().count() >= MIN_READABLE_CHARS
            || (complete && !digest.text.trim().is_empty())
        {
            return Ok(digest);
        }
    }
    if digest.text.trim().is_empty() {
        return Err(anyhow!(
            "页面无可读正文（可能需要登录 / 被风控 / 非 HTML）: {url}"
        ));
    }
    Ok(digest)
}

/// 搜索引擎结果抽取 JS（从渲染后 DOM 取标题/链接/摘要）。纯函数，离线可测。
///
/// 选择器口径（P0-3 收敛）：只认 Bing 的结果块 `li.b_algo`——**去掉** Google 的 `.g`
/// 与泛用的 `.result`（它们会捞到导航/广告块）；兜底也**不再取任意 `<a>`**，杜绝把
/// 引擎首页的导航链接当成「结果」。
pub fn search_results_js() -> &'static str {
    r#"(() => {
  const items = Array.from(document.querySelectorAll("li.b_algo"))
    .slice(0, 10)
    .map(li => {
      const a = li.querySelector("h2 a, a h2");
      const cite = li.querySelector("cite");
      const snip = li.querySelector(".b_caption p, .VwiC3b, .aCOpRe");
      return {
        title: a ? (a.innerText || "").trim() : "",
        href: a ? a.href : "",
        snippet: snip ? (snip.innerText || "").trim() : (cite ? (cite.innerText || "").trim() : "")
      };
    })
    .filter(r => r.href && r.href.startsWith("http"));
  return JSON.stringify(items);
})()"#
}

/// 从 [`search_results_js`] JSON 解析为 [`SearchResult`](crate::tools::builtin::SearchResult)
/// （复用 `tools::builtin` 的既有类型，保持 agent 内部工具与 WebUI RPC 同一套结构）。
/// `pub(crate)`：仅 crate 内部（搜索工具）复用，避免向公开 API 泄露 `pub(crate)` 类型。
pub(crate) fn extract_search_results_from_json(
    raw: &str,
) -> Vec<crate::tools::builtin::SearchResult> {
    let v: Value = match serde_json::from_str(raw) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };
    v.as_array()
        .map(|a| {
            a.iter()
                .filter(|x| {
                    // 过滤空 href（搜索引擎 DOM 常含无链接的结果块 / 广告位）
                    x.get("href")
                        .and_then(|s| s.as_str())
                        .map(|s| !s.is_empty())
                        .unwrap_or(false)
                })
                .map(|x| crate::tools::builtin::SearchResult {
                    title: x
                        .get("title")
                        .and_then(|s| s.as_str())
                        .unwrap_or("")
                        .to_string(),
                    url: x
                        .get("href")
                        .and_then(|s| s.as_str())
                        .map(|s| s.to_string()),
                    snippet: x
                        .get("snippet")
                        .and_then(|s| s.as_str())
                        .unwrap_or("")
                        .to_string(),
                })
                .collect()
        })
        .unwrap_or_default()
}

/// 轻量 query 编码（空格→+；UTF-8 由 Chromium 负责编码，足够驱动搜索引擎）。
fn encode_query(q: &str) -> String {
    q.replace(' ', "+")
}

/// 用内置浏览器在搜索引擎里查 `query` 并抽取结果（无 search API key）。需本机有 Chromium。
/// `pub(crate)`：仅 crate 内部（搜索工具）复用，避免向公开 API 泄露 `pub(crate)` 类型。
pub(crate) async fn browser_search(
    bridge: &BrowserBridge,
    query: &str,
    max: usize,
) -> Result<Vec<crate::tools::builtin::SearchResult>> {
    // P0-3：改用 cn.bing.com（国内直连可用），避免 www.bing.com 的 302 重定向开销。
    //
    // 市场/语言必须跟随查询语言（与直连引擎 `web_search::search_market` 同一口径）：
    // cn.bing 对裸 `?q=` 默认按中文市场重排，英文技术查询会被中文站点甚至词典模块挤占
    // —— 实测子智能体因此回报「搜索受到中文词典结果干扰」并改道 web_fetch。
    let (mkt, setlang) = crate::tools::builtin::web_search::search_market(query);
    let url = format!(
        "https://cn.bing.com/search?q={}&mkt={mkt}&setlang={setlang}",
        encode_query(query)
    );
    let mut page = bridge.open(&url).await?;
    // 结果块就绪轮询（替代固定 800ms）：导航是异步的，慢网/首屏渲染未完成时
    // 固定睡眠会抽到空列表，把「还没加载完」误判成「引擎 DOM 改版」，进而白白回退。
    //
    // 预算刻意只给 4 轮（≈1.2s）：实测（2026-10-23）headless 下 cn.bing 2.5s 后
    // 仍是 `readyState=loading`、`document.body === null`，等到 3s 也换不来结果，
    // 而直连引擎 0.7s 就能给出 5 条 —— 多等只是在每次搜索上白扣时间。
    // 拿不到就让上层回退，熔断器会把这台机器的实际情况记住（10 分钟内不再试探）。
    let mut res = Vec::new();
    for _ in 0..4 {
        tokio::time::sleep(READINESS_INTERVAL).await;
        let raw = page
            .session
            .evaluate(search_results_js(), true)
            .await
            .map_err(|e| anyhow!("搜索引擎结果抽取失败: {e}"))?;
        res = extract_search_results_from_json(&raw);
        if !res.is_empty() {
            break;
        }
    }
    res.truncate(max);
    Ok(res)
}

// ─────────────────────────────────────────────────────────────────────────────
// 浏览器测试（DOM 断言）
// ─────────────────────────────────────────────────────────────────────────────

/// 单条断言类型。序列化后注入页面 JS 执行。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowserAssertion {
    /// `text_contains` / `element_exists` / `title_equals` / `count_ge`
    pub kind: String,
    /// 文本或 CSS 选择器
    pub target: String,
    /// `count_ge` 的期望下限
    #[serde(default)]
    pub expect: Option<u32>,
}

/// 单条断言结果。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssertionResult {
    pub kind: String,
    pub target: String,
    pub passed: bool,
    pub detail: String,
}

/// 一组断言的测试报告。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowserTestReport {
    pub url: String,
    pub passed: usize,
    pub failed: usize,
    pub results: Vec<AssertionResult>,
}

/// 构造页面内执行断言的 JS 表达式（把 `assertions_json` 作为 IIFE 参数注入）。纯函数，离线可测。
pub fn eval_assertions_js(assertions_json: &str) -> String {
    // 用占位符 + replace 注入，避免 format! 对 JS 大括号的转义负担（{{ }} 易错且难读）
    const TEMPLATE: &str = r#"((A) => {
  const out = [];
  for (const a of A) {
    try {
      if (a.kind === "text_contains") {
        const hit = (document.body ? document.body.innerText : "").includes(a.target);
        out.push({kind:a.kind, target:a.target, passed:hit, detail: hit ? "" : "正文未包含 '" + a.target + "'"});
      } else if (a.kind === "element_exists") {
        const el = document.querySelector(a.target);
        out.push({kind:a.kind, target:a.target, passed: !!el, detail: el ? "" : "选择器未命中 " + a.target});
      } else if (a.kind === "title_equals") {
        const eq = document.title.trim() === a.target;
        out.push({kind:a.kind, target:a.target, passed:eq, detail: eq ? "" : "标题=" + document.title});
      } else if (a.kind === "count_ge") {
        const n = document.querySelectorAll(a.target).length;
        const want = (a.expect && typeof a.expect === "number") ? a.expect : 1;
        out.push({kind:a.kind, target:a.target, passed: n >= want, detail: "count=" + n});
      } else {
        out.push({kind:a.kind, target:a.target, passed:false, detail:"未知断言类型"});
      }
    } catch (e) {
      out.push({kind:a.kind, target:a.target, passed:false, detail:"ERR:" + e});
    }
  }
  return JSON.stringify(out);
})(__ASSERTIONS__)"#;
    TEMPLATE.replace("__ASSERTIONS__", assertions_json)
}

/// 从 [`eval_assertions_js`] 结果解析为 [`BrowserTestReport`]。纯函数，离线可测。
pub fn extract_test_report(raw: &str, url: &str) -> Result<BrowserTestReport> {
    let arr: Value =
        serde_json::from_str(raw).map_err(|e| anyhow!("解析断言结果 JSON 失败: {e}"))?;
    let results: Vec<AssertionResult> = arr
        .as_array()
        .map(|a| {
            a.iter()
                .map(|x| AssertionResult {
                    kind: x
                        .get("kind")
                        .and_then(|s| s.as_str())
                        .unwrap_or("")
                        .to_string(),
                    target: x
                        .get("target")
                        .and_then(|s| s.as_str())
                        .unwrap_or("")
                        .to_string(),
                    passed: x.get("passed").and_then(|b| b.as_bool()).unwrap_or(false),
                    detail: x
                        .get("detail")
                        .and_then(|s| s.as_str())
                        .unwrap_or("")
                        .to_string(),
                })
                .collect()
        })
        .unwrap_or_default();
    let passed = results.iter().filter(|r| r.passed).count();
    let failed = results.len() - passed;
    Ok(BrowserTestReport {
        url: url.to_string(),
        passed,
        failed,
        results,
    })
}

/// 用内置浏览器对 `url` 跑一组 DOM 断言（浏览器测试）。需本机有 Chromium。
pub async fn run_browser_test(
    bridge: &BrowserBridge,
    url: &str,
    assertions: &[BrowserAssertion],
) -> Result<BrowserTestReport> {
    let assertions_json = serde_json::to_string(assertions)?;
    let mut page = bridge.open(url).await?;
    tokio::time::sleep(Duration::from_millis(600)).await;
    let raw = page
        .session
        .evaluate(&eval_assertions_js(&assertions_json), true)
        .await
        .map_err(|e| anyhow!("浏览器测试 JS 求值失败: {e}"))?;
    extract_test_report(&raw, url)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_digest_from_json_parses() {
        let raw = r#"{"title":"示例页","text":"这是正文 内容","headings":["标题一","标题二"],"links":[{"text":"百度","href":"https://baidu.com"},{"text":"内链","href":"/x"}]}"#;
        let d = extract_digest_from_json(raw, "https://e.com").unwrap();
        assert_eq!(d.title, "示例页");
        assert_eq!(d.url, "https://e.com");
        assert_eq!(d.headings.len(), 2);
        // 内链（非 http）应被过滤
        assert_eq!(d.links.len(), 1);
        assert_eq!(d.links[0].href, "https://baidu.com");
        assert!(d.summarize(4).ends_with('…'));
    }

    #[test]
    fn extract_search_results_from_json_parses() {
        let raw = r#"[{"title":"A","href":"https://a.com","snippet":"摘要A"},{"title":"B","href":"","snippet":""}]"#;
        let r = extract_search_results_from_json(raw);
        assert_eq!(r.len(), 1, "空 href 结果应被过滤");
        assert_eq!(r[0].title, "A");
        assert_eq!(r[0].url.as_deref(), Some("https://a.com"));
    }

    #[test]
    fn extract_search_results_from_json_handles_garbage() {
        assert!(extract_search_results_from_json("not json").is_empty());
        assert!(extract_search_results_from_json("[]").is_empty());
    }

    /// 真机缺陷回归：headless Chromium 的 `innerText` 可能恒为空（依赖布局），
    /// 抽取脚本必须用 `textContent` 兜底，否则抓取到的正文永远是空串。
    #[test]
    fn collect_page_js_falls_back_to_text_content() {
        let js = collect_page_js();
        assert!(js.contains("innerText"), "优先用 innerText（屏幕阅读顺序更好）");
        assert!(
            js.contains("textContent"),
            "必须用 textContent 兜底：headless 下 innerText 可能为空"
        );
        // 兜底必须是 `innerText || textContent` 的前后关系，而不是各自独立取
        let inner_first = js.find("innerText").unwrap();
        let tc = js.find("textContent").unwrap();
        assert!(inner_first < tc, "兜底顺序应为 innerText 优先、textContent 兜底");
        assert!(js.contains("readyState"), "抽取脚本需回传就绪状态供提前收工");
    }

    #[test]
    fn page_is_complete_reads_ready_state() {
        assert!(page_is_complete(r#"{"title":"t","text":"x","ready":"complete"}"#));
        assert!(!page_is_complete(r#"{"title":"t","text":"x","ready":"loading"}"#));
        assert!(!page_is_complete(r#"{"title":"t","text":"x"}"#), "缺字段不应崩");
        assert!(!page_is_complete("not json"));
    }

    #[test]
    fn eval_assertions_js_injects_json_and_is_well_formed() {
        let js = eval_assertions_js(r#"[{"kind":"title_equals","target":"X"}]"#);
        assert!(js.contains(r#"[{"kind":"title_equals","target":"X"}]"#));
        assert!(js.starts_with("(("));
        assert!(js.trim_end().ends_with(")"));
    }

    #[test]
    fn extract_test_report_counts_pass_fail() {
        let raw = r#"[{"kind":"t","target":"a","passed":true,"detail":""},{"kind":"t","target":"b","passed":false,"detail":"x"}]"#;
        let r = extract_test_report(raw, "https://u.com").unwrap();
        assert_eq!(r.url, "https://u.com");
        assert_eq!(r.passed, 1);
        assert_eq!(r.failed, 1);
        assert_eq!(r.results.len(), 2);
    }

    // 实时集成测试：需本机 Chromium。手动 `cargo test -- --ignored` 验证内置浏览器采集。
    #[tokio::test]
    #[ignore = "需要本机 Chromium；验证内置浏览器资料采集（无外部 API）"]
    async fn live_collect_page_returns_digest() {
        let bridge = BrowserBridge::new();
        assert!(bridge.is_available(), "未检测到 Chromium，跳过");
        let d = collect_page(
            &bridge,
            "data:text/html,<title>采集测试</title><h1>主标题</h1><p>正文内容</p><a href='https://example.com'>外链</a>",
        )
        .await
        .expect("采集应成功");
        assert_eq!(d.title, "采集测试");
        assert!(d.text.contains("正文内容"));
        assert!(d.headings.iter().any(|h| h.contains("主标题")));
        assert!(d.links.iter().any(|l| l.href.contains("example.com")));
    }

    // 实时集成测试：浏览器 DOM 断言。需本机 Chromium。
    #[tokio::test]
    #[ignore = "需要本机 Chromium；验证浏览器测试断言"]
    async fn live_browser_test_runs_assertions() {
        let bridge = BrowserBridge::new();
        assert!(bridge.is_available(), "未检测到 Chromium，跳过");
        let report = run_browser_test(
            &bridge,
            "data:text/html,<h1>Hi</h1><button id='b'>GO</button><button>OK</button>",
            &[
                BrowserAssertion {
                    kind: "title_equals".into(),
                    target: "".into(),
                    expect: None,
                },
                BrowserAssertion {
                    kind: "element_exists".into(),
                    target: "#b".into(),
                    expect: None,
                },
                BrowserAssertion {
                    kind: "count_ge".into(),
                    target: "button".into(),
                    expect: Some(2),
                },
            ],
        )
        .await
        .expect("测试应成功");
        assert!(
            report.passed >= 2,
            "title/element/count 应多数通过: {report:?}"
        );
    }
}
