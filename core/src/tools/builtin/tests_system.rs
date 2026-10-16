//! Tests for builtin tools (split from original tests.rs)

use super::*;

use super::tests::{setup, put};

    #[tokio::test]
    async fn memory_save_search_roundtrip() {
        let dir = std::env::temp_dir().join(format!("marguerite_mem_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("mem.db");
        let mut reg = ToolRegistry::new();
        for t in memory_tools(db.clone()) {
            reg.register_local(t);
        }

        let mut s = HashMap::new();
        put(
            &mut s,
            "body",
            "Rust 的所有权模型保证了内存安全，无需垃圾回收。",
        );
        put(&mut s, "title", "Rust 所有权");
        put(&mut s, "kind", "pattern");
        put(&mut s, "tags", "rust,memory");
        let r = reg.execute_local_tool("memory_save", &s).await.unwrap();
        assert!(r.is_success(), "save failed");

        let mut q = HashMap::new();
        put(&mut q, "query", "Rust 内存安全");
        let r = reg.execute_local_tool("memory_search", &q).await.unwrap();
        match r {
            ToolResult::Success { data, .. } => {
                assert!(data.contains("Rust 所有权"), "got: {data}")
            }
            _ => panic!("memory_search failed"),
        }

        let mut l = HashMap::new();
        put(&mut l, "kind", "pattern");
        let r = reg.execute_local_tool("memory_list", &l).await.unwrap();
        match r {
            ToolResult::Success { data, .. } => {
                assert!(data.contains("Rust 所有权"), "got: {data}")
            }
            _ => panic!("memory_list failed"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn memory_save_requires_body() {
        let dir = std::env::temp_dir().join(format!("marguerite_mem_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut reg = ToolRegistry::new();
        for t in memory_tools(dir.join("mem.db")) {
            reg.register_local(t);
        }
        let r = reg
            .execute_local_tool("memory_save", &HashMap::new())
            .await
            .unwrap();
        assert!(r.is_error(), "empty body must be rejected");
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ---- 系统级鼠标键盘控制（system_control）----

    #[test]
    fn sendkeys_for_maps_keys_and_combos() {
        assert_eq!(sendkeys_for("enter").as_deref(), Some("{ENTER}"));
        assert_eq!(sendkeys_for("f5").as_deref(), Some("{F5}"));
        assert_eq!(sendkeys_for("ctrl+c").as_deref(), Some("^c"));
        assert_eq!(sendkeys_for("alt+tab").as_deref(), Some("%{TAB}"));
        assert_eq!(sendkeys_for("ctrl+shift+p").as_deref(), Some("^+p"));
        assert_eq!(sendkeys_for("a").as_deref(), Some("a"));
        assert!(sendkeys_for("boguskey").is_none());
    }

    #[test]
    fn system_control_ps_builds_scripts() {
        let mut a = HashMap::new();
        put(&mut a, "x", "100");
        put(&mut a, "y", "200");
        let ps = system_control_ps("mouse_move", &a).unwrap();
        assert!(ps.contains("[MgtSC]::SetCursorPos(100,200)"), "ps=[{ps}]");

        let mut a = HashMap::new();
        put(&mut a, "x", "10");
        put(&mut a, "y", "20");
        put(&mut a, "button", "right");
        put(&mut a, "double", "1");
        let ps = system_control_ps("mouse_click", &a).unwrap();
        assert!(ps.contains("SetCursorPos(10,20)"));
        assert!(ps.contains("0x8"));
        assert!(ps.contains("0x10"));
        // double → 两次 down/up（共 4 次 [MgtSC]::mouse_event 调用）
        assert_eq!(ps.matches("[MgtSC]::mouse_event").count(), 4);

        let mut a = HashMap::new();
        put(&mut a, "lines", "-3");
        let ps = system_control_ps("mouse_scroll", &a).unwrap();
        assert!(ps.contains("0x0800"));

        let ps = system_control_ps("mouse_pos", &HashMap::new()).unwrap();
        assert!(ps.contains("GetCursorPos"));
        let ps = system_control_ps("screen_size", &HashMap::new()).unwrap();
        assert!(ps.contains("GetSystemMetrics(0)"));

        let mut a = HashMap::new();
        put(&mut a, "key", "ctrl+s");
        let ps = system_control_ps("key_press", &a).unwrap();
        assert!(ps.contains("SendWait('^s')"));

        let mut a = HashMap::new();
        put(&mut a, "text", "hello 'world'");
        let ps = system_control_ps("type_text", &a).unwrap();
        // 文本直接内联（单引号字面量），不再经进程级 env；`'` 须被转义为 `''`
        assert!(ps.contains("SetText('hello ''world''')"), "ps=[{ps}]");
        assert!(!ps.contains("MARG_SYS_TEXT"), "不应再依赖进程级 env: {ps}");

        // monitors：应枚举 AllScreens
        let ps = system_control_ps("monitors", &HashMap::new()).unwrap();
        assert!(ps.contains("AllScreens"), "ps=[{ps}]");

        // screenshot 无 out → 报错
        assert!(system_control_ps("screenshot", &HashMap::new()).is_err());
        // screenshot：默认全屏 + 无 OCR
        let mut a = HashMap::new();
        put(&mut a, "out", "shot.png");
        let ps = system_control_ps("screenshot", &a).unwrap();
        assert!(ps.contains("CopyFromScreen"), "ps=[{ps}]");
        // 输出路径直接内联（单引号字面量），不再经进程级 env
        assert!(ps.contains("$out='shot.png'"), "ps=[{ps}]");
        assert!(!ps.contains("MARG_SYS_OUT"), "不应再依赖进程级 env: {ps}");
        assert!(!ps.contains("Windows.Media.Ocr"), "默认不应含 OCR: {ps}");
        // screenshot + monitor + ocr
        let mut a = HashMap::new();
        put(&mut a, "out", "shot.png");
        put(&mut a, "monitor", "1");
        put(&mut a, "ocr", "1");
        let ps = system_control_ps("screenshot", &a).unwrap();
        assert!(ps.contains("$screens[$monitor]"), "应支持指定显示器: {ps}");
        assert!(ps.contains("Windows.Media.Ocr"), "ocr=1 应含 OCR 逻辑");

        assert!(system_control_ps("bogus", &HashMap::new()).is_err());
        // 缺参数路径应报错（不触发 PowerShell）
        assert!(system_control_ps("mouse_move", &HashMap::new()).is_err());
    }

    #[tokio::test]
    async fn system_control_rejects_invalid_params_before_powershell() {
        let (reg, dir) = setup();
        assert!(reg.has_tool("system_control"), "system_control 应已注册");
        // 缺 action → INVALID_PARAMS
        let r = reg
            .execute_local_tool("system_control", &HashMap::new())
            .await
            .unwrap();
        assert!(r.is_error());
        // mouse_move 缺 x/y → INVALID_PARAMS（在启动 PowerShell 前拦截，无副作用）
        let mut a = HashMap::new();
        put(&mut a, "action", "mouse_move");
        let r = reg.execute_local_tool("system_control", &a).await.unwrap();
        match r {
            ToolResult::Error { code, .. } => assert_eq!(code, "INVALID_PARAMS"),
            _ => panic!("expected INVALID_PARAMS"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
