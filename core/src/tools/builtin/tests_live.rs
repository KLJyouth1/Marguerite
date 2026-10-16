//! Tests for builtin tools (split from original tests.rs)

use super::*;

use super::tests::{setup, put};

    #[tokio::test]
    #[ignore = "需真实网络；手动 --ignored 运行以验证 web_search/web_research 端到端"]
    async fn live_web_research_search_then_read() {
        let (reg, dir) = setup();
        let args = |q: &str, max_read: &str| {
            let mut m = HashMap::new();
            put(&mut m, "q", q);
            put(&mut m, "max_read", max_read);
            m
        };
        let data = |r: ToolResult| match r {
            ToolResult::Success { data, .. } => data,
            other => panic!("应成功: {other:?}"),
        };
        // 1) 纯搜索：结构化 标题/链接/摘要
        let search_text = data(
            reg.execute_local_tool("web_search", &args("Rust KV cache", "1"))
                .await
                .unwrap(),
        );
        assert!(
            search_text.contains("http"),
            "web_search 应含链接: {search_text}"
        );
        // 2) 检索+深读：应含 来源 与 正文摘录
        let text = data(
            reg.execute_local_tool("web_research", &args("Rust KV cache", "2"))
                .await
                .unwrap(),
        );
        assert!(text.contains("【检索"), "缺少检索头: {text}");
        assert!(text.contains("来源:"), "缺少来源标注: {text}");
        assert!(text.contains("正文摘录"), "缺少正文深读: {text}");
        eprintln!(
            "=== web_research 端到端输出（前 1500 字）===\n{}",
            text.chars().take(1500).collect::<String>()
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
    #[tokio::test]
    #[ignore = "需要本机 Chromium；手动运行以验证 browser 工具视觉双通道"]
    async fn live_browser_tool_vision_and_vision_click() {
        use crate::vision::{LocateResult, VisionProviderResolver, VisionTransport};
        use async_trait::async_trait;

        struct CapResolver;
        #[async_trait]
        impl VisionProviderResolver for CapResolver {
            async fn resolve_vision_model(&self) -> Option<String> {
                Some("mock-vision".into())
            }
        }
        struct CapTransport {
            describe_reply: &'static str,
        }
        #[async_trait]
        impl VisionTransport for CapTransport {
            async fn describe(&self, _: &[u8], _: &str, _: &str) -> Result<String, String> {
                Ok(self.describe_reply.to_string())
            }
            async fn locate(&self, _: &[u8], _: &str, _: &str) -> Option<LocateResult> {
                // 按钮 fixed 于 (100,100) 尺寸 120x40 → 中心 (160,120)（相对截图/视口，DPR=1 比例 1:1）
                Some(LocateResult {
                    x: 160.0,
                    y: 120.0,
                    confidence: 0.99,
                })
            }
        }

        let cap = Arc::new(VisionCapability::offline_default());
        cap.set_resolver(Box::new(CapResolver));
        cap.set_transport(Box::new(CapTransport {
            describe_reply: "页面中央有一个 CLICK ME 按钮",
        }));

        let dir =
            std::env::temp_dir().join(format!("marguerite_browser_vis_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut reg = ToolRegistry::new();
        register_builtin_tools(&mut reg, &dir, None, Some(cap), None).unwrap();

        let html = "data:text/html,<button id='b1' style='position:fixed;left:100px;top:100px;width:120px;height:40px' onclick='window.__clicked=1'>CLICK ME</button>";
        let mut a = HashMap::new();
        put(&mut a, "action", "open");
        put(&mut a, "url", html);
        put(&mut a, "browser_id", "t1");
        let r = reg.execute_local_tool("browser", &a).await.unwrap();
        assert!(r.is_success(), "open 应成功: {r:?}");
        tokio::time::sleep(Duration::from_millis(1200)).await;

        // vision：截图 → 视觉模型描述（mock）
        let mut b = HashMap::new();
        put(&mut b, "action", "vision");
        put(&mut b, "browser_id", "t1");
        let r = reg.execute_local_tool("browser", &b).await.unwrap();
        if let ToolResult::Success { data, .. } = &r {
            assert!(
                data.contains("CLICK ME 按钮"),
                "vision 描述应包含 mock 回复: {data}"
            );
        } else {
            panic!("vision 应成功: {r:?}");
        }

        // vision_click：截图 → VLM 坐标 → 映射视口 → 真实点击
        let mut c = HashMap::new();
        put(&mut c, "action", "vision_click");
        put(&mut c, "browser_id", "t1");
        put(&mut c, "hint", "点击 CLICK ME 按钮");
        let r = reg.execute_local_tool("browser", &c).await.unwrap();
        assert!(r.is_success(), "vision_click 应成功: {r:?}");

        // 同会话校验点击副作用
        tokio::time::sleep(Duration::from_millis(200)).await;
        let mut d = HashMap::new();
        put(&mut d, "action", "evaluate");
        put(&mut d, "browser_id", "t1");
        put(&mut d, "js", "window.__clicked || 0");
        let r = reg.execute_local_tool("browser", &d).await.unwrap();
        if let ToolResult::Success { data, .. } = &r {
            assert_eq!(
                data.trim(),
                "1",
                "vision_click 应触发按钮 onclick，实际: {data}"
            );
        } else {
            panic!("evaluate 应成功: {r:?}");
        }
        println!(
            "[live] browser 工具视觉双通道通过：vision 描述 + vision_click 坐标点击副作用生效"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    // 实时联网冒烟：DDG 不可达时应回退 Bing；web_fetch 应提取正文而非原始 HTML。
    #[tokio::test]
    #[ignore = "需联网；手动运行验证 B5 联网实时信息（多引擎回退 + 正文提取）"]
    async fn live_web_search_falls_back_to_bing() {
        let (reg, dir) = setup();
        let mut w = HashMap::new();
        put(&mut w, "q", "Rust programming language 2026");
        let r = reg.execute_local_tool("web_search", &w).await.unwrap();
        match r {
            ToolResult::Success { data, .. } => {
                assert!(!data.trim().is_empty(), "搜索应返回结果，got: {data}");
                assert!(data.contains("\n  "), "结果应含链接行: {data}");
                println!("[live] web_search 多引擎回退返回 {} 字符", data.len());
            }
            ToolResult::Error { message, code } => {
                panic!("web_search 应成功（DDG 失败时回退 Bing），got [{code}]: {message}");
            }
            ToolResult::Timeout { .. } => panic!("web_search 超时"),
        }

        // web_fetch 正文提取（readability 风格）
        let mut f = HashMap::new();
        put(&mut f, "url", "https://www.baidu.com");
        let r = reg.execute_local_tool("web_fetch", &f).await.unwrap();
        match r {
            ToolResult::Success { data, .. } => {
                assert!(
                    data.contains("标题:") || data.contains("描述:") || data.contains("百度"),
                    "正文提取应含标题/描述/内容: {data}"
                );
                assert!(!data.contains("<html"), "不应返回原始 HTML: {data}");
                println!("[live] web_fetch 正文提取返回 {} 字符", data.len());
            }
            ToolResult::Error { message, code } => {
                panic!("web_fetch 应成功，got [{code}]: {message}")
            }
            ToolResult::Timeout { .. } => panic!("web_fetch 超时"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    #[ignore = "Windows 真实桌面；手动运行验证 monitors/screenshot+OCR 端到端"]
    async fn live_monitors_and_screenshot_ocr() {
        let (reg, dir) = setup();
        let mut m = HashMap::new();
        m.insert("action".into(), "monitors".into());
        let r = reg.execute_local_tool("system_control", &m).await.unwrap();
        match r {
            ToolResult::Success { data, .. } => {
                eprintln!("=== monitors 输出 ===\n{data}");
                assert!(data.contains("monitor"), "应列出显示器: {data}");
            }
            other => panic!("monitors 应成功: {other:?}"),
        }
        // 截图 + OCR：保存到临时目录，验证 PNG 生成与 OCR 文本输出
        let shot = dir.join("shot.png").to_string_lossy().to_string();
        let mut m = HashMap::new();
        m.insert("action".into(), "screenshot".into());
        m.insert("out".into(), shot.clone());
        m.insert("ocr".into(), "1".into());
        let r = reg.execute_local_tool("system_control", &m).await.unwrap();
        match r {
            ToolResult::Success { data, .. } => {
                eprintln!("=== screenshot+OCR 输出 ===\n{data}");
                assert!(
                    data.contains("已保存截图") || data.contains("OCR"),
                    "应返回保存信息或 OCR 结果: {data}"
                );
            }
            other => panic!("screenshot 应成功: {other:?}"),
        }
        assert!(
            std::path::Path::new(&shot).exists(),
            "截图文件应存在: {shot}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 联网搜索实测（需要真实网络；默认 #[ignore]，用 --ignored 运行验证）。
    /// 验证「简单联网搜索任务」链路真实可用：cn.bing 主引擎应返回结构化结果。
    #[tokio::test]
    #[ignore = "requires real network"]
    async fn web_search_live_returns_results() {
        let results = run_web_search("防城港", 5).await.expect("搜索应成功返回");
        assert!(!results.is_empty(), "cn.bing 应返回至少一条结果");
        let first = &results[0];
        assert!(!first.title.is_empty(), "标题不应为空");
        assert!(first.url.is_some(), "链接不应为空");
        eprintln!("[web_search 实测] 首条: {} ({:?})", first.title, first.url);
    }
