//! Auto-extracted from builtin.rs — see parent module for shared infrastructure.
//!
//! Shell exec + HTML parsing + web_fetch + http_request

use super::*;

/// 多根快照：逐根采集后按文件路径合并；任一根被截断即整体标记截断
/// （截断快照不参与差分，避免把没扫到的文件误判为新增）。
fn snapshot_all_roots(ws: &RootSet) -> crate::tools::file_change::WorkspaceSnapshot {
    let mut merged = crate::tools::file_change::WorkspaceSnapshot::default();
    for root in ws.all() {
        let snap = crate::tools::file_change::snapshot_workspace(root);
        merged.truncated |= snap.truncated;
        for (path, meta) in snap.files {
            merged.files.insert(path, meta);
        }
    }
    merged
}

pub(crate) fn shell_exec_tool(
    workspace: impl Into<RootSet>,
    guard: Arc<CommandGuard>,
) -> Box<dyn ToolExecutor> {
    let workspace: RootSet = workspace.into();
    boxed(
        "shell_exec",
        "在工作区内执行 shell 命令（经命令护栏拦截高危操作，超时强制终止进程树，输出有上限）。\
         参数: command, timeout_sec(可选, 1-900), cwd(可选, 工作区内相对目录), \
         shell(可选: default|cmd|powershell|pwsh|bash|sh), max_output_bytes(可选, 4096-4194304)",
        json!({
            "type": "object",
            "properties": {
                "command": {"type": "string", "description": "要执行的命令"},
                "timeout_sec": {"type": "number", "description": "超时秒数（1-900，默认 30）"},
                "cwd": {"type": "string", "description": "工作区内的相对目录（可选）"},
                "shell": {"type": "string", "description": "default|cmd|powershell|pwsh|bash|sh（可选）"},
                "max_output_bytes": {"type": "number", "description": "stdout/stderr 各自上限（默认 256KB）"}
            },
            "required": ["command"]
        }),
        Arc::new(move |args| {
            let ws = workspace.clone();
            let guard = guard.clone();
            Box::pin(async move {
                let cmd = args.get("command").ok_or_else(|| anyhow!("missing 'command'"))?;
                let timeout = match args.get("timeout_sec").and_then(|t| t.parse::<u64>().ok()) {
                    // 上限与 agent 工具回路的 900s 对齐：再长就该走长任务队列而不是 shell。
                    Some(s) => Duration::from_secs(s.clamp(1, 900)),
                    None => DEFAULT_TIMEOUT,
                };
                let shell = match args
                    .get("shell")
                    .map(|s| s.trim())
                    .filter(|s| !s.is_empty())
                    .and_then(ExecShell::from_id)
                {
                    Some(s) => s,
                    None if args
                        .get("shell")
                        .map(|s| !s.trim().is_empty())
                        .unwrap_or(false) =>
                    {
                        return err(
                            "未知 shell：仅支持 default / cmd / powershell / pwsh / bash / sh"
                                .to_string(),
                            "INVALID_PARAMS",
                        );
                    }
                    None => ExecShell::Default,
                };
                let max_output = args
                    .get("max_output_bytes")
                    .and_then(|v| v.parse::<usize>().ok())
                    .map(|n| n.clamp(4096, 4 * 1024 * 1024))
                    .unwrap_or(DEFAULT_MAX_OUTPUT);
                // cwd 限定工作区内：模型给的是相对路径（多根会话可命中任一登记根）；
                // 越界按既有文案报告根目录。
                let cwd = match args
                    .get("cwd")
                    .map(|s| s.trim())
                    .filter(|s| !s.is_empty())
                {
                    Some(raw) => match ws.resolve(raw) {
                        Ok(p) => p,
                        Err(e) => return err(format!("{e}"), "INVALID_PATH"),
                    },
                    None => ws.primary().to_path_buf(),
                };
                if !cwd.is_dir() {
                    return err(format!("工作目录不存在：{}", cwd.display()), "INVALID_PATH");
                }
                // L2+L3 在 L1（exec_with 内的 CommandGuard）之前先行裁决。
                // 补的是 CommandGuard 正则表里没有的洞：chroot / unshare / nsenter /
                // pkexec / docker --privileged 等沙箱逃逸原语，以及 /dev/tcp 反弹 shell。
                if let Some(denied) = gate(&PREFLIGHT.check_exec_multi(cmd, &guard_roots(&ws))) {
                    return denied;
                }
                // 命令类工具此前**完全不登记产物**：模型用 `python make_ppt.py` 之类的
                // 脚本产出交付物时，磁盘上有文件、产物面板却是空的（真机缺陷）。
                // 用「命令前后工作区快照差分」补齐，有界扫描（见 file_change 模块常量）。
                // 多根：逐个根快照并按路径合并（产物可能落在附加根）。
                let before = snapshot_all_roots(&ws);
                let req = ExecRequest::new(cmd, &cwd)
                    .with_timeout(timeout)
                    .with_shell(shell)
                    .with_max_output(max_output);
                match exec_with(&req, &guard).await {
                    Ok(o) => {
                        let after = snapshot_all_roots(&ws);
                        let registered = crate::tools::file_change::register_workspace_diff(
                            &before,
                            &after,
                            &current_session(),
                            "generated",
                        );
                        // 登记条数按 info 留痕（含 0）：「命令成功但登记 0 条」是
                        // 「产物面板空」的另一半根因，此前完全不可见。
                        tracing::info!(
                            registered,
                            workspace = %ws.primary().display(),
                            exit_code = o.exit_code,
                            "命令产物差分登记完成"
                        );
                        let trunc = if o.truncated {
                            format!("\n[输出已截断：超过 {} 字节，仅保留前 {} 字节]",
                                max_output, max_output)
                        } else {
                            String::new()
                        };
                        ok(format!(
                            "[exit {}] [shell {}] [cwd {}]{} \n--- stdout ---\n{}\n--- stderr ---\n{}",
                            o.exit_code,
                            shell.id(),
                            cwd.display(),
                            trunc,
                            o.stdout,
                            o.stderr
                        ))
                    }
                    Err(e) => err(e.to_string(), "EXEC_ERROR"),
                }
            })
        }),
    )
}

// ── 全程联网：web_fetch 正文提取（readability 风格，零依赖）──
//
// 对齐 外部参考 snapshot-ref / readability 范式：抓取页面后剔除脚本与导航噪音，
// 提取 标题 + meta 描述 + 主内容段落，供 Agent 深读正文而非原始 HTML 噪音。

pub(crate) fn looks_like_html(text: &str) -> bool {
    let lower = text.trim_start().to_ascii_lowercase();
    if !lower.starts_with('<') {
        return false;
    }
    lower.starts_with("<!doctype html")
        || lower.starts_with("<html")
        || lower.starts_with("<head")
        || lower.starts_with("<body")
        || lower.contains("<title")
}

/// 去掉 HTML 中的脚本/样式/模板/内嵌 SVG 噪音区间
pub(crate) fn strip_noise_tags(html: &str) -> String {
    let re = regex::Regex::new(
        r"(?is)<(?:script|style|noscript|template|svg)\b[^>]*>.*?</(?:script|style|noscript|template|svg)\s*>",
    )
    .expect("noise tag regex 应恒可编译");
    re.replace_all(html, "").into_owned()
}

/// 提取首个标签对的内层文本（如 <title>…</title>）
pub(crate) fn extract_tag_content(html: &str, tag: &str) -> String {
    let re = regex::Regex::new(&format!(r"(?is)<{tag}\b[^>]*>(.*?)</{tag}\s*>"))
        .expect("tag regex 应恒可编译");
    re.captures(html)
        .and_then(|c| c.get(1))
        .map(|m| strip_html_tags(m.as_str()))
        .unwrap_or_default()
}

/// 提取 <meta name="description" content="…">（兼容 name=description / property=og:description）
pub(crate) fn extract_meta_description(html: &str) -> String {
    let re = regex::Regex::new(
        r#"(?is)<meta\b[^>]*(?:name|property)\s*=\s*["'](?:description|og:description)["'][^>]*>"#,
    )
    .expect("meta regex 应恒可编译");
    let Some(m) = re.find(html) else {
        return String::new();
    };
    let re_meta = regex::Regex::new(r#"<meta\b[^>]*>"#).expect("meta 标签 regex 应恒可编译");
    let content_re =
        regex::Regex::new(r#"content\s*=\s*["']([^"']*)["']"#).expect("content regex 应恒可编译");
    for tag in re_meta.find_iter(m.as_str()) {
        if let Some(c) = content_re.captures(tag.as_str()) {
            return html_unescape(&c[1]).trim().to_string();
        }
    }
    String::new()
}

/// 提取指定容器（article/main/body 等）标签对内文本
pub(crate) fn tag_zone(html: &str, tag: &str) -> Option<String> {
    let re = regex::Regex::new(&format!(r"(?is)<{tag}\b[^>]*>(.*)</{tag}\s*>"))
        .expect("zone regex 应恒可编译");
    re.captures(html).map(|c| c[1].to_string())
}

/// 收集容器内全部 <p> 段落文本（保留顺序，去内嵌标签）
pub(crate) fn extract_paragraphs(zone: &str) -> Vec<String> {
    let p_re = regex::Regex::new(r"(?is)<p\b[^>]*>(.*?)</p\s*>").expect("p regex 应恒可编译");
    let mut out = Vec::new();
    for cap in p_re.captures_iter(zone) {
        let text = strip_html_tags(&cap[1]);
        let text = text.trim().to_string();
        if !text.is_empty() {
            out.push(text);
        }
    }
    out
}

/// 去除段内标签并解码常见 HTML 实体
pub(crate) fn strip_html_tags(s: &str) -> String {
    let tag_re = regex::Regex::new(r"(?s)<[^>]*>").expect("tag strip regex 应恒可编译");
    html_unescape(&tag_re.replace_all(s, ""))
}

pub(crate) fn html_unescape(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&#x27;", "'")
        .replace("&nbsp;", " ")
}

/// readability 风格正文提取：标题 + 描述 + 主内容段落（article/main 优先，回退 body 全部 <p>）
///
/// `pub(crate)` 供网关 RPC 层（`gateway::tools::web_research`）复用。
pub(crate) fn extract_readable(html: &str, max_chars: usize) -> String {
    let stripped = strip_noise_tags(html);
    let title = extract_tag_content(&stripped, "title");
    let description = extract_meta_description(&stripped);

    // 主内容：优先 article/main 段落；不足 2 段则回退到 body 级 <p>
    let mut paragraphs: Vec<String> = Vec::new();
    for container in ["article", "main"] {
        if let Some(zone) = tag_zone(&stripped, container) {
            let ps = extract_paragraphs(&zone);
            if ps.len() >= 2 {
                paragraphs = ps;
                break;
            }
        }
    }
    if paragraphs.is_empty() {
        paragraphs = extract_paragraphs(&stripped);
    }

    let mut out = String::new();
    if !title.is_empty() {
        out.push_str(&format!("标题: {title}\n"));
    }
    if !description.is_empty() {
        out.push_str(&format!("描述: {description}\n"));
    }
    for p in paragraphs {
        out.push_str(&format!("\n{p}"));
    }
    out.chars().take(max_chars).collect()
}

/// 构造带 **SSRF 重定向复检 + 硬超时** 的 reqwest client。
///
/// 缺陷：`web_fetch`/`http_request` 只校验**初始** URL，而 reqwest 默认跟随重定向
/// → 攻击者用一个公网 URL 302 到 `http://169.254.169.254/` 即可绕过 SSRF 防护；
/// 且 `Client::new()` 无超时，慢速/挂死端点会永久占用工具调用。
///
/// 修复：自定义重定向策略，对**每一跳**目标重新过 [`SsrfGuard::check`]；
/// 并设置整体请求超时。
pub(crate) fn guarded_http_client(ssrf: &Arc<SsrfGuard>) -> reqwest::Client {
    let guard = ssrf.clone();
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::custom(move |attempt| {
            // 限制跳数，避免重定向环
            if attempt.previous().len() >= 5 {
                return attempt.error("too many redirects");
            }
            // 逐跳复检：302 到内网/元数据地址必须拦截
            if guard.check(attempt.url().as_str()).is_err() {
                return attempt.error("redirect to blocked address (ssrf)");
            }
            attempt.follow()
        }))
        .build()
        .unwrap_or_else(|_| reqwest::Client::new())
}

pub(crate) fn web_fetch_tool(ssrf: Arc<SsrfGuard>) -> Box<dyn ToolExecutor> {
    boxed(
        "web_fetch",
        "抓取网页/接口文本（经 SSRF 防护，禁止内网/元数据地址；HTML 自动提取正文 readability 风格）。参数: url, max_chars(可选, 默认8000)",
        json!({"type":"object","properties":{"url":{"type":"string"},"max_chars":{"type":"number"}}}),
        Arc::new(move |args| {
            let ssrf = ssrf.clone();
            Box::pin(async move {
                let url = args.get("url").ok_or_else(|| anyhow!("missing 'url'"))?;
                if let Err(e) = ssrf.check(url) {
                    return err(format!("ssrf guard: {}", e), "SSRF_BLOCKED");
                }
                let max: usize = args.get("max_chars").and_then(|m| m.parse().ok()).unwrap_or(8000);
                let resp = match guarded_http_client(&ssrf).get(url).send().await {
                    Ok(r) => r,
                    Err(e) => return err(format!("request failed: {}", e), "NET_ERROR"),
                };
                if !resp.status().is_success() {
                    return err(format!("http status {}", resp.status()), "HTTP_ERROR");
                }
                let text = match resp.text().await {
                    Ok(t) => t,
                    Err(e) => return err(format!("read body: {}", e), "READ_ERROR"),
                };
                // HTML 走 readability 正文提取；纯文本/JSON 原样截断
                let body = if looks_like_html(&text) {
                    extract_readable(&text, max)
                } else {
                    text.chars().take(max).collect()
                };
                ok(body)
            })
        }),
    )
}

pub(crate) fn http_request_tool(ssrf: Arc<SsrfGuard>) -> Box<dyn ToolExecutor> {
    boxed(
        "http_request",
        "发起通用 HTTP 请求（GET/POST/...，经 SSRF 防护）。参数: method, url, body(可选), max_chars(可选)",
        json!({"type":"object","properties":{"method":{"type":"string"},"url":{"type":"string"},"body":{"type":"string"},"max_chars":{"type":"number"}}}),
        Arc::new(move |args| {
            let ssrf = ssrf.clone();
            Box::pin(async move {
                let method = args.get("method").map(|s| s.to_uppercase()).unwrap_or_else(|| "GET".into());
                let url = args.get("url").ok_or_else(|| anyhow!("missing 'url'"))?;
                if let Err(e) = ssrf.check(url) {
                    return err(format!("ssrf guard: {}", e), "SSRF_BLOCKED");
                }
                let client = guarded_http_client(&ssrf);
                let mut req = client.request(
                    reqwest::Method::from_bytes(method.as_bytes()).map_err(|_| anyhow!("bad method"))?,
                    url,
                );
                if let Some(body) = args.get("body") {
                    req = req.header("Content-Type", "application/json").body(body.clone());
                }
                let resp = match req.send().await {
                    Ok(r) => r,
                    Err(e) => return err(format!("request failed: {}", e), "NET_ERROR"),
                };
                let max: usize = args.get("max_chars").and_then(|m| m.parse().ok()).unwrap_or(8000);
                let status = resp.status().as_u16();
                let text = resp.text().await.unwrap_or_default();
                let truncated: String = text.chars().take(max).collect();
                ok(format!("[status {}]\n{}", status, truncated))
            })
        }),
    )
}

// ── 浏览器自动化（真实 Chromium，CDP，Agent 可控有状态会话）──
//
// 对齐 GUI_Agent/gwwc 的 browser_act 思路，落地 Marguerite 融合路由的
// 「内置 Agent 可控浏览器」：视觉（screenshot 截图）与非视觉（DOM 读取 +
// evaluate/click/type/scroll 操控）双通道。
// - `browser_id`：复用同一页面会话（跨工具调用持续操控）；不传则每次新建。
// - 会话注册表进程内常驻，空闲 10 分钟自动回收，上限 8 个。
// - 无 Chrome 时返回明确错误而非静默失败。

/// 有状态浏览器会话（进程内常驻，drop 时杀 Chromium）
pub(crate) struct BrowserEntry {
    pub(crate) page: PageSession,
    pub(crate) touched: std::time::Instant,
}

pub(crate) const BROWSER_SESSION_TTL: Duration = Duration::from_secs(600);
pub(crate) const BROWSER_SESSION_MAX: usize = 8;

