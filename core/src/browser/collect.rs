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
pub fn collect_page_js() -> &'static str {
    r#"(() => {
  const title = document.title || "";
  const body = document.body;
  const text = body ? body.innerText.replace(/\s+/g, " ").trim() : "";
  const headings = Array.from(document.querySelectorAll("h1,h2,h3"))
    .map(h => (h.innerText || "").replace(/\s+/g, " ").trim())
    .filter(Boolean);
  const links = Array.from(document.querySelectorAll("a[href]"))
    .map(a => ({ text: (a.innerText || "").replace(/\s+/g, " ").trim(), href: a.href }))
    .filter(l => l.href && l.href.startsWith("http"));
  return JSON.stringify({ title, text, headings, links });
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

/// 用内置浏览器导航并采集页面（无外部 API）。需本机有 Chromium。
pub async fn collect_page(bridge: &BrowserBridge, url: &str) -> Result<PageDigest> {
    let mut page = bridge.open(url).await?;
    // 等首屏渲染（真实浏览器已加载完成，短延时足够让 innerText 稳定）
    tokio::time::sleep(Duration::from_millis(600)).await;
    let raw = page
        .session
        .evaluate(collect_page_js(), true)
        .await
        .map_err(|e| anyhow!("采集页面 JS 求值失败: {e}"))?;
    extract_digest_from_json(&raw, url)
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
    let url = format!("https://cn.bing.com/search?q={}", encode_query(query));
    let mut page = bridge.open(&url).await?;
    tokio::time::sleep(Duration::from_millis(800)).await;
    let raw = page
        .session
        .evaluate(search_results_js(), true)
        .await
        .map_err(|e| anyhow!("搜索引擎结果抽取失败: {e}"))?;
    let mut res = extract_search_results_from_json(&raw);
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
