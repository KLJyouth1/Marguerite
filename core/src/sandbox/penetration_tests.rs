//! 渗透测试（对应深化 D3.7 P1-P12）
//!
//! 目标：穿透/穿越/逃逸用例全拒；未知命令默认拒绝；审计落盘；截断与超时生效。

use std::path::Path;
use std::time::Duration;

use async_trait::async_trait;

use crate::mode::ModeKind;

use super::{CommandRunner, ResourceLimit, RunnerOutput, Sandbox, SandboxResult};

fn v(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| s.to_string()).collect()
}

fn ws_dir() -> std::path::PathBuf {
    std::env::temp_dir()
}

/// 注入式执行器：大输出 / 延迟
struct MockRunner {
    output: Vec<u8>,
    delay: Duration,
}

#[async_trait]
impl CommandRunner for MockRunner {
    async fn run(&self, _argv: &[String], _cwd: &Path, _timeout: Duration) -> RunnerOutput {
        if !self.delay.is_zero() {
            tokio::time::sleep(self.delay).await;
        }
        RunnerOutput {
            stdout: self.output.clone(),
            stderr: Vec::new(),
            exit_code: 0,
            timed_out: false,
        }
    }
}

/// 仅判定层（不执行）：返回 Some = 被拒
fn check_only(argv: &[String], cwd: &Path, mode: ModeKind) -> Option<SandboxResult> {
    let sb = Sandbox::new(&ws_dir());
    sb.check(argv, cwd, mode)
}

#[tokio::test]
async fn p1_rm_rf_denied() {
    let r = check_only(&v(&["rm", "-rf", "/"]), &ws_dir(), ModeKind::Bloom).unwrap();
    assert_eq!(r.decision, "deny");
    assert!(r.stderr.contains("rm"));
}

#[tokio::test]
async fn p2_relative_rm_denied() {
    // ./rm 伪装 → basename=rm → Deny
    let r = check_only(&v(&["./rm", "x"]), &ws_dir(), ModeKind::Bloom).unwrap();
    assert_eq!(r.decision, "deny");
}

#[tokio::test]
async fn p3_sudo_denied() {
    let r = check_only(
        &v(&["sudo", "apt", "install", "x"]),
        &ws_dir(),
        ModeKind::Bloom,
    )
    .unwrap();
    assert_eq!(r.decision, "deny");
}

#[tokio::test]
async fn p4_curl_pipe_sh_denied() {
    // curl 在 DENY（且管道提示写意图）
    let r = check_only(
        &v(&["curl", "http://x", "|", "sh"]),
        &ws_dir(),
        ModeKind::Bloom,
    )
    .unwrap();
    assert_eq!(r.decision, "deny");
}

#[tokio::test]
async fn p5_python_c_denied() {
    let r = check_only(
        &v(&["python", "-c", "os.system('rm')"]),
        &ws_dir(),
        ModeKind::Bloom,
    )
    .unwrap();
    assert_eq!(r.decision, "deny");
    assert!(r.stderr.contains("内联"));
}

#[tokio::test]
async fn p6_node_e_denied() {
    let r = check_only(
        &v(&["node", "-e", "console.log(1)"]),
        &ws_dir(),
        ModeKind::Bloom,
    )
    .unwrap();
    assert_eq!(r.decision, "deny");
}

#[tokio::test]
async fn p7_git_push_denied() {
    let r = check_only(&v(&["git", "push"]), &ws_dir(), ModeKind::Bloom).unwrap();
    assert_eq!(r.decision, "deny");
}

#[tokio::test]
async fn p8_git_status_plan_allowed() {
    // ReadOnly 在 plan 模式放行
    let r = check_only(&v(&["git", "status"]), &ws_dir(), ModeKind::Plan);
    assert!(r.is_none(), "git status 应放行");
}

#[tokio::test]
async fn p9_cwd_escape_denied() {
    // cwd 在工作区外 → 路径守卫拦截
    let binding = ws_dir();
    let outside = binding.parent().unwrap_or(Path::new(".")).to_path_buf();
    let r = check_only(&v(&["touch", "x"]), &outside, ModeKind::Bloom).unwrap();
    assert_eq!(r.decision, "path_denied");
    assert!(r.stderr.contains("工作区"));
}

#[tokio::test]
async fn p10_unknown_tool_denied() {
    let r = check_only(&v(&["evil_tool", "--flag"]), &ws_dir(), ModeKind::Bloom).unwrap();
    assert_eq!(r.decision, "deny");
}

#[tokio::test]
async fn p11_output_capped_and_truncated() {
    let ws = std::env::temp_dir();
    let big = vec![b'x'; 5000];
    let sb = Sandbox::with_runner(
        &ws,
        Box::new(MockRunner {
            output: big,
            delay: Duration::ZERO,
        }),
    );
    let mut sb = sb;
    sb.set_limits(ResourceLimit {
        max_output_bytes: 1024,
        ..ResourceLimit::default()
    });
    let result = sb
        .execute(&v(&["echo", "big"]), &ws, ModeKind::Bloom)
        .await
        .unwrap();
    assert_eq!(result.decision, "allow");
    assert_eq!(result.stdout.len(), 1024, "输出应截断到 1024 字节");
    assert!(result.truncated, "应标记 truncated");
}

#[tokio::test]
async fn p12_timeout_kills_and_marks_timeout() {
    let ws = std::env::temp_dir();
    // 执行器延迟 2s > 限制 1s → 超时
    let sb = Sandbox::with_runner(
        &ws,
        Box::new(MockRunner {
            output: Vec::new(),
            delay: Duration::from_secs(2),
        }),
    );
    let mut sb = sb;
    sb.set_limits(ResourceLimit {
        timeout_sec: 1,
        ..ResourceLimit::default()
    });
    let result = sb
        .execute(&v(&["echo", "slow"]), &ws, ModeKind::Bloom)
        .await
        .unwrap();
    assert_eq!(result.decision, "timeout");
    assert_eq!(result.exit_code, -1);
}
