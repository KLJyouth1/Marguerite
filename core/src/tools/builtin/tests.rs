//! Tests for builtin tools (split from original tests.rs)

use super::*;

/// Shared test fixture: creates a temp workspace + fully-registered tool registry.
/// Sibling test modules import this via `use super::tests::{setup, put};`.
pub(super) fn setup() -> (ToolRegistry, PathBuf) {
        let dir = std::env::temp_dir().join(format!("marguerite_tools_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("hello.txt"), "line1\nline2\nline3\n").unwrap();
        std::fs::write(dir.join("data.json"), r#"{"a":{"b":[10,20,30]}}"#).unwrap();
        let mut reg = ToolRegistry::new();
        register_builtin_tools(&mut reg, &dir, None, None, None).unwrap();
        (reg, dir)
    }

    pub(super) fn put(m: &mut HashMap<String, String>, k: &str, v: &str) {
        m.insert(k.to_string(), v.to_string());
    }

    #[tokio::test]
    async fn registers_all_tools() {
        let (reg, dir) = setup();
        assert!(
            reg.local_tool_count() >= 20,
            "expected >=20 tools, got {}",
            reg.local_tool_count()
        );
        for name in [
            "read_file",
            "write_file",
            "grep",
            "shell_exec",
            "web_fetch",
            "json_query",
            "diff",
            "browser",
            "web_search",
            "system_control",
            "memory_save",
            "memory_search",
            "memory_list",
            "apply_patch",
            "glob_file_search",
            "todo_write",
        ] {
            assert!(reg.has_tool(name), "missing tool: {}", name);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ─────────────────────────────────────────────────────────────────────
    // P1-E 回归：内置工具名必须「首次注册即合法」，不得刷「非法字符已自动修正」
    // ─────────────────────────────────────────────────────────────────────

    /// 捕获 tracing 输出的最小 `MakeWriter`（线程本地，不干扰并行用例）。
    #[derive(Clone, Default)]
    struct CaptureWriter(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

    impl std::io::Write for CaptureWriter {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for CaptureWriter {
        type Writer = CaptureWriter;
        fn make_writer(&'a self) -> Self::Writer {
            self.clone()
        }
    }

    /// 全部内置工具名必须满足 OpenAI/DeepSeek function name 规范 `^[a-zA-Z0-9_-]+$`。
    ///
    /// 修复前 14 个 baota 工具用点号名（`baota.system_info`）注册，`register_local`
    /// 每次都要「非法名 → 自动修正」，于是**干净首启**的 stderr 被 14 行 WARN 刷屏。
    /// 本用例从三个面机械守卫根因不再复发：
    /// 1. 注册表不得产生任何历史别名（别名非空 ⇔ 走过自动修正分支 ⇔ 会刷告警）；
    /// 2. `tools.catalog` 对外名与 `list_tools` 派发键都必须合法且可派发；
    /// 3. 真实注册过程的 tracing 输出里不得出现「非法字符」告警文本。
    #[tokio::test]
    async fn all_builtin_tool_names_are_legal_at_registration() {
        let dir = std::env::temp_dir().join(format!("marguerite_names_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();

        let sink = std::sync::Arc::new(std::sync::Mutex::new(Vec::<u8>::new()));
        let subscriber = tracing_subscriber::fmt()
            .with_writer(CaptureWriter(sink.clone()))
            .with_ansi(false)
            .with_max_level(tracing::Level::TRACE)
            .finish();

        let mut reg = ToolRegistry::new();
        {
            let _guard = tracing::subscriber::set_default(subscriber);
            register_builtin_tools(&mut reg, &dir, None, None, None).unwrap();
        }

        let logged = String::from_utf8(sink.lock().unwrap().clone()).unwrap();
        assert!(
            !logged.contains("非法字符"),
            "内置工具注册不得再刷「非法字符」告警（P1-E 首启噪音）。实际 tracing 输出:\n{logged}"
        );

        // 1) 没有任何工具走过「注册后修正」。
        assert!(
            reg.legacy_aliases().is_empty(),
            "内置工具名必须在首次注册即合法，实际别名: {:?}",
            reg.legacy_aliases()
        );

        // 2) 对外目录名（raw name）合法且可派发。
        let catalog = reg.catalog();
        assert!(catalog.len() >= 20, "内置工具数异常: {}", catalog.len());
        for item in &catalog {
            for c in item.name.chars() {
                assert!(
                    c.is_ascii_alphanumeric() || c == '_' || c == '-',
                    "工具名 {:?} 含非法字符 {:?}（规范 ^[a-zA-Z0-9_-]+$）",
                    item.name,
                    c
                );
            }
            assert!(
                reg.has_tool(&item.name),
                "目录名必须可派发: {}",
                item.name
            );
        }

        // 3) 派发键集合同样合法（与 catalog 同集合，且含全部 baota 工具）。
        let keys = reg.list_tools();
        for n in &keys {
            assert_eq!(
                crate::mcp::registry::sanitize_tool_name(n),
                *n,
                "派发键 {:?} 非法",
                n
            );
        }
        assert!(
            keys.iter().any(|n| n == "baota_system_info"),
            "缺少 baota_system_info（baota 工具应以下划线名注册）"
        );
        assert!(
            !keys.iter().any(|n| n.starts_with("baota.")),
            "不得存在点号形式的 baota 工具名"
        );
        // 便于 `--nocapture` 人工核对（默认被 libtest 捕获，不污染常规输出）
        println!(
            "内置工具名 {} 个，全部合法（^[a-zA-Z0-9_-]+$），其中 baota: {}",
            keys.len(),
            keys.iter()
                .filter(|n| n.starts_with("baota"))
                .cloned()
                .collect::<Vec<_>>()
                .join(", ")
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 历史别名兼容：注册非法名后，**旧名与新名都必须可派发**，且不再 WARN。
    #[tokio::test]
    async fn illegal_registration_name_keeps_legacy_alias_callable() {
        let mut reg = ToolRegistry::new();
        reg.register_local(crate::tools::make_local_tool(
            "legacy.dotted_name",
            "历史别名回归用工具",
            serde_json::json!({"type": "object", "properties": {}}),
            std::sync::Arc::new(|_args: &HashMap<String, String>| {
                Box::pin(async { Ok(ToolResult::Success {
                    data: "alias-ok".to_string(),
                    metadata: HashMap::new(),
                }) })
            }),
        ));

        assert_eq!(
            reg.legacy_aliases().get("legacy.dotted_name"),
            Some(&"legacy_dotted_name".to_string()),
            "非法名必须被登记为历史别名"
        );
        for name in ["legacy.dotted_name", "legacy_dotted_name"] {
            assert!(reg.has_tool(name), "{name} 应可派发");
            let r = reg
                .execute_local_tool(name, &HashMap::new())
                .await
                .unwrap_or_else(|e| panic!("{name} 派发失败: {e}"));
            assert!(r.is_success(), "{name} 应执行成功");
        }

        // 移除后别名不得悬垂
        reg.remove_tool("legacy.dotted_name");
        assert!(!reg.has_tool("legacy_dotted_name"));
        assert!(!reg.has_tool("legacy.dotted_name"));
        assert!(
            reg.execute_local_tool("legacy.dotted_name", &HashMap::new())
                .await
                .is_err(),
            "移除后旧名不应再可派发"
        );
    }

    #[tokio::test]
    async fn read_and_write_roundtrip() {
        let (reg, dir) = setup();
        let mut a = HashMap::new();
        put(&mut a, "path", "out.txt");
        put(&mut a, "content", "hello world");
        let r = reg.execute_local_tool("write_file", &a).await.unwrap();
        assert!(r.is_success(), "write should succeed");

        let mut b = HashMap::new();
        put(&mut b, "path", "out.txt");
        let r = reg.execute_local_tool("read_file", &b).await.unwrap();
        match r {
            ToolResult::Success { data, .. } => assert_eq!(data, "hello world"),
            _ => panic!("read failed"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn edit_file_replaces() {
        let (reg, dir) = setup();
        let mut e = HashMap::new();
        put(&mut e, "path", "hello.txt");
        put(&mut e, "old_string", "line2");
        put(&mut e, "new_string", "LINE_TWO");
        let r = reg.execute_local_tool("edit_file", &e).await.unwrap();
        assert!(r.is_success());

        let mut b = HashMap::new();
        put(&mut b, "path", "hello.txt");
        let r = reg.execute_local_tool("read_file", &b).await.unwrap();
        match r {
            ToolResult::Success { data, .. } => assert!(data.contains("LINE_TWO")),
            _ => panic!("read failed"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn apply_patch_applies_hunks() {
        let (reg, dir) = setup();
        let mut w = HashMap::new();
        put(&mut w, "path", "src/app.rs");
        put(
            &mut w,
            "content",
            "fn main() {\n    println!(\"a\");\n    println!(\"b\");\n}\n",
        );
        assert!(reg
            .execute_local_tool("write_file", &w)
            .await
            .unwrap()
            .is_success());

        let patch = "--- a/src/app.rs\n+++ b/src/app.rs\n@@ -2,3 +2,4 @@\n-    println!(\"b\");\n+    println!(\"B\");\n+    println!(\"c\");\n }\n";
        let mut a = HashMap::new();
        put(&mut a, "patch", patch);
        let r = reg.execute_local_tool("apply_patch", &a).await.unwrap();
        assert!(r.is_success(), "apply_patch 应成功: {r:?}");

        let mut b = HashMap::new();
        put(&mut b, "path", "src/app.rs");
        let r = reg.execute_local_tool("read_file", &b).await.unwrap();
        match r {
            ToolResult::Success { data, .. } => {
                assert!(data.contains("println!(\"B\")"), "got: {data}");
                assert!(data.contains("println!(\"c\")"), "got: {data}");
                assert!(!data.contains("println!(\"b\")"), "got: {data}");
            }
            _ => panic!("read failed"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn apply_hunks_rejects_mismatched_context() {
        let patch = "@@ -1,1 +1,1 @@\n-不存在的行\n+替换行\n";
        assert!(apply_hunks("fn main() {}", patch).is_err());
    }

    #[test]
    fn glob_matches_patterns() {
        assert!(glob_matches("**/*.rs", "src/main.rs"));
        assert!(glob_matches("**/*.rs", "main.rs"));
        assert!(glob_matches("src/*.ts", "src/app.ts"));
        assert!(!glob_matches("src/*.ts", "src/nested/app.ts"));
        assert!(glob_matches("src/**/*.ts", "src/nested/app.ts"));
        assert!(glob_matches("*.md", "README.md"));
        assert!(glob_matches("f?o.rs", "foo.rs"));
        assert!(!glob_matches("f?o.rs", "fooo.rs"));
    }

    #[tokio::test]
    async fn todo_write_roundtrip() {
        let (reg, dir) = setup();
        let mut s = HashMap::new();
        put(&mut s, "action", "set");
        put(
            &mut s,
            "items",
            r#"[{"step":"读测试","status":"completed"},{"step":"写代码","status":"in_progress"}]"#,
        );
        let r = reg.execute_local_tool("todo_write", &s).await.unwrap();
        assert!(r.is_success(), "todo_write set 应成功: {r:?}");

        let mut g = HashMap::new();
        put(&mut g, "action", "get");
        let r = reg.execute_local_tool("todo_write", &g).await.unwrap();
        match r {
            ToolResult::Success { data, .. } => {
                assert!(data.contains("写代码"), "get 应读回清单: {data}");
                assert!(data.contains("in_progress"), "get 应含状态: {data}");
            }
            _ => panic!("todo_write get 失败"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn grep_finds_match() {
        let (reg, dir) = setup();
        let mut g = HashMap::new();
        put(&mut g, "pattern", "line3");
        let r = reg.execute_local_tool("grep", &g).await.unwrap();
        match r {
            ToolResult::Success { data, .. } => {
                assert!(data.contains("hello.txt:3:line3"), "got: {}", data)
            }
            _ => panic!("grep failed"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn json_query_extracts() {
        let (reg, dir) = setup();
        let mut q = HashMap::new();
        put(&mut q, "json", r#"{"a":{"b":[10,20,30]}}"#);
        put(&mut q, "path", "a.b.1");
        let r = reg.execute_local_tool("json_query", &q).await.unwrap();
        match r {
            ToolResult::Success { data, .. } => assert_eq!(data, "20"),
            _ => panic!("json_query failed"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn diff_produces_change_lines() {
        let (reg, dir) = setup();
        let mut d = HashMap::new();
        put(&mut d, "old", "a\nb\nc");
        put(&mut d, "new", "a\nB\nc\nd");
        let r = reg.execute_local_tool("diff", &d).await.unwrap();
        match r {
            ToolResult::Success { data, .. } => {
                assert!(data.contains("- b"));
                assert!(data.contains("+ B"));
                assert!(data.contains("+ d"));
            }
            _ => panic!("diff failed"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn path_escape_rejected() {
        let (reg, dir) = setup();
        let mut a = HashMap::new();
        put(&mut a, "path", "../../etc/passwd");
        put(&mut a, "content", "x");
        let r = reg.execute_local_tool("write_file", &a).await;
        assert!(r.is_err(), "path escape must be rejected");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn list_and_find_work() {
        let (reg, dir) = setup();
        let r = reg
            .execute_local_tool("list_dir", &HashMap::new())
            .await
            .unwrap();
        assert!(r.is_success());

        let mut f = HashMap::new();
        put(&mut f, "name", "hello.txt");
        let r = reg.execute_local_tool("find_files", &f).await.unwrap();
        match r {
            ToolResult::Success { data, .. } => assert!(data.contains("hello.txt")),
            _ => panic!("find failed"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
