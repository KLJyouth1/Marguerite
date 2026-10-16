//! 网关层联网检索 RPC（tools.web_search / tools.web_research / tools.web_fetch）
//!
//! 背景：此前 `tools.catalog` 宣称提供 `web_search` / `web_research`，但网关 RPC 层
//! 没有对应方法 → 前端调用必得 `NOT_IMPLEMENTED`，被用户判定为「联网搜索任务差极了」。
//! 本模块新增真实实现，复用 `tools::builtin::run_web_search`（cn.bing 主 + DDG 回退，
//! 零 API key），网关与 agent 工具层共用同一套搜索逻辑，避免双实现漂移。
//!
//! 注意：**`tools.catalog` 不在这里**。它曾在此处硬编码 23 条工具名，其中
//! `file_read` / `terminal_exec` / `kb_search` 等名字在真实工具面上并不存在——
//! 前端「工具与能力」因此展示编造的工具名。现已改为派发到
//! [`crate::gateway::workspace::tools_catalog`]（真实注册表 + `enabled` 标注）。

use serde_json::{json, Value};

/// `tools.web_search`：网关层联网检索（异步）。返回结构化标题/链接/摘要数组。
///
/// 参数：`q`（必填）、`max_results`（默认 5，上限 10）。
pub async fn web_search(params: &Value) -> Value {
    let q = params
        .get("q")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or_default();
    if q.is_empty() {
        return json!({ "ok": false, "error": "web_search 需要非空 q", "code": "INVALID_PARAMS" });
    }
    let max: usize = params
        .get("max_results")
        .and_then(Value::as_u64)
        .map(|m| (m as usize).min(10))
        .unwrap_or(5)
        .max(1);
    match crate::tools::builtin::run_web_search(q, max).await {
        Ok(results) => {
            let items: Vec<Value> = results
                .iter()
                .map(|r| {
                    json!({
                        "title": r.title,
                        "url": r.url,
                        "snippet": r.snippet,
                    })
                })
                .collect();
            json!({ "ok": true, "query": q, "count": items.len(), "results": items })
        }
        Err(e) => json!({ "ok": false, "error": e, "code": "NET_ERROR" }),
    }
}

/// `tools.web_fetch`：抓取单个网页正文（异步，SSRF 防护）。
///
/// 参数：`url`（必填）、`max_chars`（默认 8000，上限 50000）。
pub async fn web_fetch(params: &Value) -> Value {
    let url = params
        .get("url")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or_default();
    if url.is_empty() {
        return json!({ "ok": false, "error": "web_fetch 需要非空 url", "code": "INVALID_PARAMS" });
    }
    let max_chars: usize = params
        .get("max_chars")
        .and_then(Value::as_u64)
        .map(|m| (m as usize).min(50000))
        .unwrap_or(8000)
        .max(200);

    let ssrf = crate::security::SsrfGuard::new();
    if let Err(e) = ssrf.check(url) {
        return json!({ "ok": false, "error": format!("SSRF 防护拦截: {e}"), "code": "SSRF_BLOCKED" });
    }
    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        .build()
    {
        Ok(c) => c,
        Err(e) => return json!({ "ok": false, "error": format!("HTTP 客户端构建失败: {e}") }),
    };
    let body = match client.get(url).send().await {
        Ok(resp) if resp.status().is_success() => match resp.text().await {
            Ok(t) => t,
            Err(e) => return json!({ "ok": false, "error": format!("读取失败: {e}") }),
        },
        Ok(resp) => {
            return json!({
                "ok": false,
                "error": format!("状态异常: HTTP {}", resp.status()),
                "code": "HTTP_ERROR",
            });
        }
        Err(e) => {
            return json!({ "ok": false, "error": format!("请求失败: {e}"), "code": "NET_ERROR" })
        }
    };
    let text = if crate::tools::builtin::looks_like_html(&body) {
        crate::tools::builtin::extract_readable(&body, max_chars)
    } else {
        body.chars().take(max_chars).collect()
    };
    json!({ "ok": true, "url": url, "text": text, "chars": text.chars().count() })
}

/// `tools.web_research`：网关层「检索 + 深读」一次调用（异步）。
///
/// 参数：`q`（必填）、`max_results`（默认 5，上限 10）、`max_read`（默认 3，上限 8）、
/// `max_chars`（默认 4000/条，上限 20000）。返回每条结果的标题/链接/摘要 + 正文摘录。
pub async fn web_research(params: &Value) -> Value {
    let q = params
        .get("q")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or_default();
    if q.is_empty() {
        return json!({ "ok": false, "error": "web_research 需要非空 q", "code": "INVALID_PARAMS" });
    }
    let max_results: usize = params
        .get("max_results")
        .and_then(Value::as_u64)
        .map(|m| (m as usize).min(10))
        .unwrap_or(5)
        .max(1);
    let max_read: usize = params
        .get("max_read")
        .and_then(Value::as_u64)
        .map(|m| (m as usize).min(8))
        .unwrap_or(3)
        .max(1);
    let max_chars: usize = params
        .get("max_chars")
        .and_then(Value::as_u64)
        .map(|m| (m as usize).min(20000))
        .unwrap_or(4000);

    let results = match crate::tools::builtin::run_web_search(q, max_results).await {
        Ok(rs) => rs,
        Err(e) => return json!({ "ok": false, "error": e, "code": "NET_ERROR" }),
    };
    if results.is_empty() {
        return json!({ "ok": true, "query": q, "count": 0, "digest": "搜索无结果。" });
    }

    // SSRF 防护：抓取前逐一校验目标 URL（与 agent 工具层同源策略）。
    let ssrf = crate::security::SsrfGuard::new();
    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        .build()
    {
        Ok(c) => c,
        Err(e) => return json!({ "ok": false, "error": format!("HTTP 客户端构建失败: {e}") }),
    };

    let mut digest: Vec<Value> = Vec::new();
    let mut fetched = 0usize;
    for (i, r) in results.iter().enumerate() {
        if fetched >= max_read {
            break;
        }
        let Some(url) = r.url.as_deref().filter(|u| !u.is_empty()) else {
            continue;
        };
        if let Err(e) = ssrf.check(url) {
            digest.push(json!({
                "index": i + 1,
                "title": r.title,
                "url": url,
                "skipped": true,
                "reason": format!("SSRF 防护拦截: {e}"),
            }));
            continue;
        }
        let body = match client.get(url).send().await {
            Ok(resp) if resp.status().is_success() => match resp.text().await {
                Ok(t) => {
                    fetched += 1;
                    if crate::tools::builtin::looks_like_html(&t) {
                        crate::tools::builtin::extract_readable(&t, max_chars)
                    } else {
                        t.chars().take(max_chars).collect()
                    }
                }
                Err(_) => {
                    digest.push(json!({
                        "index": i + 1,
                        "title": r.title,
                        "url": url,
                        "skipped": true,
                        "reason": "读取失败",
                    }));
                    continue;
                }
            },
            _ => {
                digest.push(json!({
                    "index": i + 1,
                    "title": r.title,
                    "url": url,
                    "skipped": true,
                    "reason": "状态异常",
                }));
                continue;
            }
        };
        digest.push(json!({
            "index": i + 1,
            "title": r.title,
            "url": url,
            "snippet": r.snippet,
            "excerpt": body,
        }));
    }

    json!({
        "ok": true,
        "query": q,
        "count": results.len(),
        "read": fetched,
        "digest": digest,
    })
}
