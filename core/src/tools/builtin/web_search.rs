//! Auto-extracted from builtin.rs — see parent module for shared infrastructure.
//!
//! Web search/research + result parsing

use super::*;

/// 既有零 API key 直接抓取引擎（cn.bing 主 + DuckDuckGo 回退）。
///
/// 作为 [`run_web_search`] 在无 Chromium / 浏览器通道异常时的回退实现，保证纯粹离线 /
/// 无浏览器环境依然可用。
pub(crate) async fn run_web_search_direct(q: &str, max: usize) -> Result<Vec<SearchResult>, String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(6))
        .build()
        .unwrap_or_default();
    let ua = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0 Safari/537.36";
    // v3.3.2 引擎重排（国内网络实测）：cn.bing.com 直连可用（10 结果/查询），
    // duckduckgo.com 被墙恒超时（15s 白等）、www.bing.com 302 反爬。
    // 主引擎 = cn.bing.com；备选 = DDG（海外网络环境）。
    let mut primary_error = String::new();
    let mut results: Vec<SearchResult> = Vec::new();
    // 记录被质量门禁剔除的条数：全部结果都是引擎首页/空壳时，给出「请调整检索词」提示，
    // 而不是把整句原样回显给用户（P0-3 / P1-1 的收口）。
    let mut filtered_out = 0usize;
    match client
        .get("https://cn.bing.com/search")
        .query(&[
            ("q", q.to_string()),
            ("mkt", "zh-CN".to_string()),
            ("count", max.to_string()),
        ])
        .header("User-Agent", ua)
        .send()
        .await
    {
        Ok(r) => {
            if let Ok(html) = r.text().await {
                let (kept, removed) = filter_low_quality(parse_bing_results(&html, max));
                if !removed.is_empty() {
                    warn!(dropped = removed.len(), "cn.bing 结果命中低质门禁（引擎首页/空壳），已剔除");
                }
                filtered_out += removed.len();
                results = kept;
            }
        }
        Err(e) => primary_error = e.to_string(),
    }
    if !results.is_empty() {
        return Ok(results);
    }
    // 备选引擎：DuckDuckGo HTML（海外网络 / cn.bing 不可用时）
    let mut ddg_error = String::new();
    let mut ddg_results: Vec<SearchResult> = Vec::new();
    match client
        .post("https://html.duckduckgo.com/html/")
        .form(&[("q", q.to_string())])
        .header("User-Agent", ua)
        .send()
        .await
    {
        Ok(r) => {
            if let Ok(html) = r.text().await {
                let (kept, removed) = filter_low_quality(parse_ddg_results(&html, max));
                if !removed.is_empty() {
                    warn!(dropped = removed.len(), "DuckDuckGo 结果命中低质门禁（引擎首页/空壳），已剔除");
                }
                filtered_out += removed.len();
                ddg_results = kept;
            }
        }
        Err(e) => ddg_error = e.to_string(),
    }
    if !ddg_results.is_empty() {
        return Ok(ddg_results);
    }
    // 引擎有返回但全部被判低质：明确提示调整检索词，不回显整句。
    if filtered_out > 0 {
        return Err(format!(
            "联网搜索未获取到有效结果（{filtered_out} 条疑似搜索引擎首页/空壳，已剔除），请调整检索词"
        ));
    }
    let detail = if !primary_error.is_empty() {
        format!("cn.bing: {primary_error}")
    } else {
        "cn.bing: 无结果".to_string()
    };
    let detail = if !ddg_error.is_empty() {
        format!("{detail}; DuckDuckGo: {ddg_error}")
    } else {
        format!("{detail}; DuckDuckGo: 无结果")
    };
    Err(format!("联网搜索不可用（多引擎均无返回）: {detail}"))
}

/// 格式化搜索结果（每条约 3 行：标题 / 链接 / 摘要）
pub(crate) fn format_search_results(results: &[SearchResult]) -> String {
    results
        .iter()
        .enumerate()
        .map(|(i, r)| {
            let url = r.url.as_deref().unwrap_or("（无链接）");
            format!("{}. {} ({url})\n   {}", i + 1, r.title, r.snippet)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub(crate) fn web_search_tool() -> Box<dyn ToolExecutor> {
    boxed(
        "web_search",
        "联网搜索（DuckDuckGo 主引擎 + Bing 回退，无需 API key）。返回 标题/链接/摘要 结构化结果。参数: q(必填), max_results(默认5)",
        json!({"type":"object","properties":{"q":{"type":"string"},"max_results":{"type":"number"}},"required":["q"]}),
        Arc::new(move |args| {
            Box::pin(async move {
                let q = args.get("q").cloned().unwrap_or_default();
                if q.trim().is_empty() {
                    return err("web_search 需要非空 q".into(), "INVALID_PARAMS");
                }
                let max: usize = args.get("max_results").and_then(|m| m.parse().ok()).unwrap_or(5).min(10);
                match run_web_search(&q, max).await {
                    Ok(results) if results.is_empty() => {
                        ok("未获取到有效结果，请调整检索词。".to_string())
                    }
                    Ok(results) => ok(format_search_results(&results)),
                    Err(detail) => err(detail, "NET_ERROR"),
                }
            })
        }),
    )
}

/// 联网检索升级：search → 自动抓取前 N 条正文 → 汇总摘要（一次调用完成「检索+深读」）。
/// 对齐主流工作台（上游工作台的 search-then-read）能力，弥补单一搜索只有标题/摘要的不足。
pub(crate) fn web_research_tool(ssrf: Arc<SsrfGuard>) -> Box<dyn ToolExecutor> {
    boxed(
        "web_research",
        "联网检索+深读：搜索 query，自动抓取前 max_read 条结果的正文（readability 提取），返回结构化研究摘要。参数: q(必填), max_results(可选,默认5), max_read(可选,默认3), max_chars(可选,默认4000/条)",
        json!({
            "type": "object",
            "properties": {
                "q": {"type": "string"},
                "max_results": {"type": "number"},
                "max_read": {"type": "number"},
                "max_chars": {"type": "number"}
            },
            "required": ["q"]
        }),
        Arc::new(move |args| {
            let ssrf = ssrf.clone();
            Box::pin(async move {
                let q = args.get("q").cloned().unwrap_or_default();
                if q.trim().is_empty() {
                    return err("web_research 需要非空 q".into(), "INVALID_PARAMS");
                }
                let max_results: usize = args.get("max_results").and_then(|m| m.parse().ok()).unwrap_or(5).min(10);
                let max_read: usize = args.get("max_read").and_then(|m| m.parse().ok()).unwrap_or(3).min(8);
                let max_chars: usize = args.get("max_chars").and_then(|m| m.parse().ok()).unwrap_or(4000).min(20000);
                let results = match run_web_search(&q, max_results).await {
                    Ok(rs) => rs,
                    Err(detail) => return err(detail, "NET_ERROR"),
                };
                if results.is_empty() {
                    return ok("未获取到有效结果，请调整检索词。".to_string());
                }
                let client = reqwest::Client::builder()
                    .timeout(Duration::from_secs(20))
                    .build()
                    .unwrap_or_default();
                // 内置浏览器桥（供资料采集/深读优先走自研浏览器通道，无外部 API）
                let bridge = BrowserBridge::new();
                let mut digest = Vec::new();
                let mut fetched = 0usize;
                for (i, r) in results.iter().enumerate() {
                    if fetched >= max_read {
                        break;
                    }
                    let url = match r.url.as_deref() {
                        Some(u) if !u.is_empty() => u,
                        _ => continue,
                    };
                    if let Err(e) = ssrf.check(url) {
                        digest.push(format!("{}. {} — 跳过（SSRF:{e}）", i + 1, r.title));
                        continue;
                    }
                    // 优先用内置浏览器读取正文（资料采集分析，无外部 API）；失败/无 Chromium 则回退 reqwest 直接抓取
                    let body: Option<String> = if bridge.is_available() {
                        match crate::browser::collect::collect_page(&bridge, url).await {
                            Ok(d) => {
                                fetched += 1;
                                Some(d.text.chars().take(max_chars).collect())
                            }
                            Err(e) => {
                                warn!(error = %e, "内置浏览器深读失败，回退 reqwest 直接抓取");
                                None
                            }
                        }
                    } else {
                        None
                    };
                    let body = match body {
                        Some(b) => b,
                        None => match client.get(url).send().await {
                            Ok(resp) if resp.status().is_success() => match resp.text().await {
                                Ok(t) => {
                                    fetched += 1;
                                    if looks_like_html(&t) {
                                        extract_readable(&t, max_chars)
                                    } else {
                                        t.chars().take(max_chars).collect()
                                    }
                                }
                                Err(_) => {
                                    digest.push(format!("{}. {} — 读取失败", i + 1, r.title));
                                    continue;
                                }
                            },
                            _ => {
                                digest.push(format!("{}. {} — 状态异常", i + 1, r.title));
                                continue;
                            }
                        },
                    };
                    digest.push(format!(
                        "### {}\n来源: {}\n摘要: {}\n正文摘录:\n{}\n",
                        i + 1,
                        url,
                        if r.snippet.is_empty() { "（无）" } else { &r.snippet },
                        body
                    ));
                }
                if digest.is_empty() {
                    return ok(format_search_results(&results));
                }
                ok(format!("【检索: {}】共 {} 条结果，深读 {} 条。\n\n{}", q, results.len(), fetched, digest.join("\n")))
            })
        }),
    )
}

/// 从 `haystack[from..]` 中安全提取 `attr="…"` 值（find 定位，避免按字节切片落入多字节字符）
pub(crate) fn extract_attr_from(haystack: &str, from: usize, attr: &str) -> Option<String> {
    let key = format!("{attr}=\"");
    let rel = haystack[from..].find(&key)?;
    let s = from + rel + key.len();
    let end = haystack[s..].find('"')? + s;
    Some(haystack[s..end].to_string())
}

/// 单条搜索结果（结构化，供 Agent 精确取链接深读）。
/// `pub(crate)` 供网关 RPC 层复用。
#[derive(Debug, Clone)]
pub(crate) struct SearchResult {
    pub title: String,
    pub url: Option<String>,
    pub snippet: String,
}

/// 从 Bing HTML 响应中提取 (标题, 链接, 摘要) 列表（轻量子串解析，零依赖）
pub(crate) fn parse_bing_results(html: &str, max: usize) -> Vec<SearchResult> {
    pub(crate) fn strip_tags(s: &str) -> String {
        let mut out = String::with_capacity(s.len());
        let mut in_tag = false;
        for c in s.chars() {
            match c {
                '<' => in_tag = true,
                '>' => in_tag = false,
                _ if !in_tag => out.push(c),
                _ => {}
            }
        }
        out.replace("&amp;", "&")
            .replace("&quot;", "\"")
            .replace("&#39;", "'")
    }
    let mut out = Vec::new();
    for block in html.split("class=\"b_algo\"") {
        if out.len() >= max {
            break;
        }
        // 标题锚点：<h2><a href="…">title</a></h2>
        let Some(h2) = block.find("<h2") else {
            continue;
        };
        let Some(anchor) = block[h2..].find("<a") else {
            continue;
        };
        let a_start = h2 + anchor;
        let href = extract_attr_from(block, a_start, "href").map(|h| h.replace("&amp;", "&"));
        let Some(title_start) = block[a_start..].find('>') else {
            continue;
        };
        let t_start = a_start + title_start + 1;
        let Some(title_end) = block[t_start..].find("</a>") else {
            continue;
        };
        let title = strip_tags(&block[t_start..t_start + title_end]);
        // 摘要：块内首个 <p>…</p>
        let snippet = block
            .find("<p")
            .and_then(|i| block[i..].find('>').map(|j| i + j + 1))
            .and_then(|start| {
                block[start..]
                    .find("</p>")
                    .map(|end| strip_tags(&block[start..start + end]))
            })
            .unwrap_or_default();
        out.push(SearchResult {
            title,
            url: href,
            snippet,
        });
    }
    out
}

/// 从 DuckDuckGo HTML 响应中提取 (标题, 链接, 摘要) 列表（轻量子串解析，零依赖）
pub(crate) fn parse_ddg_results(html: &str, max: usize) -> Vec<SearchResult> {
    pub(crate) fn strip_tags(s: &str) -> String {
        let mut out = String::with_capacity(s.len());
        let mut in_tag = false;
        for c in s.chars() {
            match c {
                '<' => in_tag = true,
                '>' => in_tag = false,
                _ if !in_tag => out.push(c),
                _ => {}
            }
        }
        out.replace("&amp;", "&")
            .replace("&quot;", "\"")
            .replace("&#x27;", "'")
    }
    let mut out = Vec::new();
    for block in html.split("class=\"result__body\"") {
        if out.len() >= max {
            break;
        }
        let Some(a_start) = block.find("class=\"result__a\"") else {
            continue; // 无标题锚点的块（如"无结果"提示）跳过
        };
        let snippet_start = block.find("class=\"result__snippet\"");
        let title = block[a_start..]
            .find('>')
            .and_then(|j| {
                let start = a_start + j + 1;
                block[start..]
                    .find("</a>")
                    .map(|end| strip_tags(&block[start..start + end]))
            })
            .unwrap_or_else(|| "（无标题）".into());
        let snippet = snippet_start
            .and_then(|i| block[i..].find('>').map(|j| i + j + 1))
            .and_then(|start| {
                block[start..]
                    .find("</a>")
                    .map(|end| strip_tags(&block[start..start + end]))
            })
            .unwrap_or_default();
        let href = extract_attr_from(block, a_start, "href").map(|h| h.replace("&amp;", "&"));
        out.push(SearchResult {
            title,
            url: href,
            snippet,
        });
    }
    out
}

