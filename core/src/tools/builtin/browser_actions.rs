//! Auto-extracted from builtin.rs — see parent module for shared infrastructure.
//!
//! Browser action dispatcher: run_browser_action

use super::*;

/// 在已就绪的页面上执行一次动作
pub(crate) async fn run_browser_action(
    page: &mut PageSession,
    args: &HashMap<String, String>,
    action: &str,
    url: &str,
    fresh: bool,
    workspace: &RootSet,
    vision_cap: &Option<Arc<VisionCapability>>,
) -> Result<ToolResult> {
    // 复用会话且给了新 url → 先导航到目标页
    if !fresh
        && !url.trim().is_empty()
        && !matches!(action, "close" | "wait")
        && page.session.navigate(url).await.is_ok()
    {
        tokio::time::sleep(Duration::from_millis(800)).await;
    }
    let max_chars: usize = args
        .get("max_chars")
        .and_then(|m| m.parse().ok())
        .unwrap_or(4000);
    match action {
        // B5：relay 连接用户真实 Chrome（复用登录态），返回会话摘要
        "relay" => {
            let title = page
                .session
                .evaluate("document.title", false)
                .await
                .unwrap_or_default();
            let loc = page
                .session
                .evaluate("location.href", false)
                .await
                .unwrap_or_default();
            let cookies = page
                .session
                .evaluate(
                    "document.cookie ? document.cookie.split(';').filter(function(s){return s.trim();}).length : 0",
                    false,
                )
                .await
                .unwrap_or_default();
            let storage = page
                .session
                .evaluate("Object.keys(localStorage).length", false)
                .await
                .unwrap_or_default();
            ok(format!(
                "真实 Chrome 会话已连接（复用登录态）\n标题: {title}\n地址: {loc}\nCookies: {cookies} 条\nlocalStorage: {storage} 项"
            ))
        }
        "open" | "text" => {
            let title = page
                .session
                .evaluate("document.title", false)
                .await
                .unwrap_or_else(|_| "<读取失败>".into());
            let body = page
                .session
                .evaluate("document.body ? document.body.innerText : ''", false)
                .await
                .unwrap_or_default();
            let body: String = body.chars().take(max_chars).collect();
            if action == "text" {
                ok(body)
            } else {
                ok(format!("标题: {title}\n\n{body}"))
            }
        }
        "links" => {
            let max: usize = args.get("max").and_then(|m| m.parse().ok()).unwrap_or(30);
            let js = format!(
                "Array.from(document.querySelectorAll('a')).map(a=>a.innerText.trim()+' => '+a.href).filter(s=>s.length>0).slice(0,{max}).join('\\n')"
            );
            let out = page.session.evaluate(&js, true).await.unwrap_or_default();
            let out: String = out.chars().take(max_chars).collect();
            if out.trim().is_empty() {
                ok("页面无可用链接".into())
            } else {
                ok(out)
            }
        }
        "html" => {
            let js = "document.documentElement ? document.documentElement.outerHTML : ''".to_string();
            let out = page.session.evaluate(&js, true).await.unwrap_or_default();
            let out: String = out.chars().take(max_chars).collect();
            ok(out)
        }
        "evaluate" => {
            let js = args.get("js").cloned().unwrap_or_default();
            if js.trim().is_empty() {
                return err("evaluate 需要非空 js".into(), "INVALID_PARAMS");
            }
            match page.session.evaluate(&js, true).await {
                Ok(v) => {
                    let v: String = v.chars().take(max_chars).collect();
                    ok(v)
                }
                Err(e) => err(format!("evaluate 失败: {e}"), "BROWSER_ERROR"),
            }
        }
        "click" => {
            let selector = args.get("selector").cloned().unwrap_or_default();
            if selector.trim().is_empty() {
                return err("click 需要非空 selector".into(), "INVALID_PARAMS");
            }
            let js = format!(
                "(function(){{var el=document.querySelector({sel});if(!el)return 'ELEMENT_NOT_FOUND';el.click();return 'clicked: '+el.tagName;}})()",
                sel = js_string(&selector)
            );
            match page.session.evaluate(&js, true).await {
                Ok(v) => ok(v),
                Err(e) => err(format!("click 失败: {e}"), "BROWSER_ERROR"),
            }
        }
        "type" => {
            let selector = args.get("selector").cloned().unwrap_or_default();
            let text = args.get("text").cloned().unwrap_or_default();
            if selector.trim().is_empty() || text.trim().is_empty() {
                return err("type 需要非空 selector 与 text".into(), "INVALID_PARAMS");
            }
            let js = format!(
                "(function(){{var el=document.querySelector({sel});if(!el)return 'ELEMENT_NOT_FOUND';el.value={text};el.dispatchEvent(new Event('input',{{bubbles:true}}));el.dispatchEvent(new Event('change',{{bubbles:true}}));return 'typed: '+el.tagName;}})()",
                sel = js_string(&selector),
                text = js_string(&text)
            );
            match page.session.evaluate(&js, true).await {
                Ok(v) => ok(v),
                Err(e) => err(format!("type 失败: {e}"), "BROWSER_ERROR"),
            }
        }
        "scroll" => {
            let px: i64 = args.get("px").and_then(|p| p.parse().ok()).unwrap_or(400);
            let js = format!("window.scrollBy(0,{px}); 'scrolled'");
            match page.session.evaluate(&js, true).await {
                Ok(v) => ok(v),
                Err(e) => err(format!("scroll 失败: {e}"), "BROWSER_ERROR"),
            }
        }
        "screenshot" => {
            let out = args.get("out").cloned().unwrap_or_default();
            if out.trim().is_empty() {
                return err("screenshot 需要非空 out".into(), "INVALID_PARAMS");
            }
            let abs = match workspace.resolve(&out) {
                Ok(p) => p,
                Err(e) => return err(format!("路径解析失败: {e}"), "INVALID_PATH"),
            };
            let png = match page.session.screenshot_png().await {
                Ok(b) => b,
                Err(e) => return err(format!("截图失败: {e}"), "BROWSER_ERROR"),
            };
            if let Err(e) = std::fs::write(&abs, &png) {
                return err(format!("保存截图失败: {e}"), "WRITE_ERROR");
            }
            ok(format!("截图已保存: {} ({} bytes)", abs.display(), png.len()))
        }
        "a11y" => {
            // 非视觉通道·语义快照：Accessibility.getFullAXTree → 可交互节点清单
            let nodes = match page.session.a11y_nodes().await {
                Ok(n) => n,
                Err(e) => return err(format!("a11y 快照失败: {e}"), "BROWSER_ERROR"),
            };
            let mut lines = Vec::new();
            for (i, n) in nodes.iter().enumerate() {
                let (x, y, w, h) = (n.bbox[0], n.bbox[1], n.bbox[2], n.bbox[3]);
                let name = if n.name.is_empty() { "<空>" } else { &n.name };
                lines.push(format!(
                    "[{i}] role={} name={} ref={} bbox=({x:.0},{y:.0},{w:.0}x{h:.0})",
                    n.role, name, n.backend_node_id
                ));
            }
            let mut out = lines.join("\n");
            out = out.chars().take(max_chars).collect();
            ok(format!("可交互节点 {} 个：\n{out}", nodes.len()))
        }
        "click_ref" => {
            // 非视觉通道·语义点击：ref → DOM.getBoxModel 中心 → Input 真实点击
            let ref_id: i64 = match args.get("ref").and_then(|v| v.parse().ok()) {
                Some(v) => v,
                None => return err("click_ref 需要整数 ref（来自 a11y 快照）".into(), "INVALID_PARAMS"),
            };
            let (cx, cy) = match page.session.box_center(ref_id).await {
                Ok(c) => c,
                Err(e) => return err(format!("定位元素失败: {e}"), "BROWSER_ERROR"),
            };
            if let Err(e) = page.session.click_at(cx, cy).await {
                return err(format!("点击失败: {e}"), "BROWSER_ERROR");
            }
            ok(format!("已点击 ref={ref_id} @ ({cx:.0},{cy:.0})"))
        }
        "vision" => {
            // 视觉通道·观察：截图 → 视觉模型描述（无能力时优雅降级）
            let cap = match vision_cap {
                Some(c) => c.clone(),
                None => {
                    return err("未注入视觉能力（VisionCapability），无法执行 vision".into(), "NOT_AVAILABLE")
                }
            };
            let png = match page.session.screenshot_png().await {
                Ok(b) => b,
                Err(e) => return err(format!("截图失败: {e}"), "BROWSER_ERROR"),
            };
            let hint = args.get("hint").cloned().unwrap_or_default();
            let desc = cap.describe_image(&png, &hint).await;
            ok(desc)
        }
        "vision_click" => {
            // 视觉通道·操纵：截图 → VLM 坐标定位（截图像素）→ 映射视口 CSS → Input 真实点击
            let cap = match vision_cap {
                Some(c) => c.clone(),
                None => {
                    return err("未注入视觉能力（VisionCapability），无法执行 vision_click".into(), "NOT_AVAILABLE")
                }
            };
            let hint = args.get("hint").cloned().unwrap_or_default();
            if hint.trim().is_empty() {
                return err("vision_click 需要 hint（目标元素描述，如「右上角的搜索按钮」）".into(), "INVALID_PARAMS");
            }
            let png = match page.session.screenshot_png().await {
                Ok(b) => b,
                Err(e) => return err(format!("截图失败: {e}"), "BROWSER_ERROR"),
            };
            let loc = match cap.locate(&png, &hint).await {
                Some(l) => l,
                None => {
                    return err(
                        "视觉定位失败（无视觉模型 / transport 不支持 / 模型未返回坐标 JSON）".into(),
                        "VISION_LOCATE_ERROR",
                    )
                }
            };
            if loc.confidence < 0.2 {
                return err(
                    format!("视觉定位置信度过低（{:.2}），拒绝点击", loc.confidence),
                    "VISION_LOCATE_ERROR",
                );
            }
            // 截图像素 → 视口 CSS 像素（DPR/缩放适配）
            let (v_w, v_h) = match page.session.viewport_size().await {
                Ok(s) => s,
                Err(e) => return err(format!("读取视口失败: {e}"), "BROWSER_ERROR"),
            };
            let meta = image_metadata(&png);
            let png_w = meta.width.unwrap_or(v_w as u32) as f64;
            let png_h = meta.height.unwrap_or(v_h as u32) as f64;
            let x_css = loc.x * v_w / png_w;
            let y_css = loc.y * v_h / png_h;
            if let Err(e) = page.session.click_at(x_css, y_css).await {
                return err(format!("点击失败: {e}"), "BROWSER_ERROR");
            }
            ok(format!(
                "已按视觉定位点击 @ 截图({:.0},{:.0}) → 视口({:.0},{:.0})，置信度 {:.2}",
                loc.x, loc.y, x_css, y_css, loc.confidence
            ))
        }
        other => err(
            format!("未知 action: {other}（支持 open/text/links/html/evaluate/click/type/screenshot/a11y/click_ref/vision/vision_click/wait/scroll/close）"),
            "INVALID_PARAMS",
        ),
    }
}
