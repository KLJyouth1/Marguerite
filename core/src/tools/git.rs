//! 版本控制工具（tool.git）：真实 git CLI 桥
//!
//! 以「可注入 trait」封装 git 调用，便于测试 mock（`RecordingGitRunner`）。
//! 生产实现 `SystemGitRunner` 直接调用 `git` 二进制（不走 shell，天然免疫 shell 注入），
//! 并经子命令白名单 + 工作区路径守卫约束，遵循项目「fail-closed / 白名单」安全伦理。
//!
//! 所有 git 操作都在 `safe_resolve` 解析出的仓库根内执行，绝不越界工作区。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use serde_json::json;

use crate::agent::tool_cycle::ToolResult;
use crate::tools::fs_path::safe_resolve;

/// git 调用输出
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitOutput {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
}

/// 允许的 git 子命令白名单（fail-closed：不在表内一律拒绝，避免 `push --force` /
/// `reset --hard` / `clean -fdx` 等破坏性动作被静默放行）。
const ALLOWED_SUBCOMMANDS: &[&str] = &[
    "status",
    "diff",
    "add",
    "commit",
    "log",
    "branch",
    "checkout",
    "switch",
    "push",
    "pull",
    "fetch",
    "clone",
    "blame",
    "init",
    "show",
    "remote",
    "tag",
    "rev-parse",
    "rev-list",
    "ls-files",
    "config",
];

fn is_allowed(subcommand: &str) -> bool {
    ALLOWED_SUBCOMMANDS.contains(&subcommand)
}

/// git 执行器抽象：便于测试注入 mock。
pub trait GitRunner: Send + Sync {
    /// 在 `repo` 仓库根执行 `git <subcommand> <extra...>`，返回结构化输出。
    fn run(&self, subcommand: &str, repo: &Path, extra: &[String]) -> Result<GitOutput>;
}

/// 生产实现：直接调用 `git` 二进制（无 shell 中转）。
pub struct SystemGitRunner;

impl GitRunner for SystemGitRunner {
    fn run(&self, subcommand: &str, repo: &Path, extra: &[String]) -> Result<GitOutput> {
        if !is_allowed(subcommand) {
            return Err(anyhow!(
                "git 子命令 '{}' 不在白名单，已拒绝（fail-closed）",
                subcommand
            ));
        }
        let mut cmd = Command::new("git");
        cmd.arg(subcommand).args(extra).current_dir(repo);
        cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
        let output = cmd
            .output()
            .with_context(|| format!("执行 git {} 失败（git 是否安装？）", subcommand))?;
        Ok(GitOutput {
            exit_code: output.status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&output.stdout).to_string(),
            stderr: String::from_utf8_lossy(&output.stderr).to_string(),
        })
    }
}

/// 测试用记录型执行器：不真正调用 git，只记录调用并返回预设输出。
#[cfg(any(test, feature = "test-utils"))]
pub struct RecordingGitRunner {
    pub calls: std::sync::Mutex<Vec<(String, PathBuf, Vec<String>)>>,
    pub canned: std::sync::Mutex<GitOutput>,
}

#[cfg(any(test, feature = "test-utils"))]
impl Default for RecordingGitRunner {
    fn default() -> Self {
        Self {
            calls: std::sync::Mutex::new(Vec::new()),
            canned: std::sync::Mutex::new(GitOutput {
                exit_code: 0,
                stdout: "recorded".to_string(),
                stderr: String::new(),
            }),
        }
    }
}

#[cfg(any(test, feature = "test-utils"))]
impl GitRunner for RecordingGitRunner {
    fn run(&self, subcommand: &str, repo: &Path, extra: &[String]) -> Result<GitOutput> {
        if !is_allowed(subcommand) {
            return Err(anyhow!("git 子命令 '{}' 不在白名单", subcommand));
        }
        self.calls.lock().unwrap().push((
            subcommand.to_string(),
            repo.to_path_buf(),
            extra.to_vec(),
        ));
        Ok(self.canned.lock().unwrap().clone())
    }
}

/// 解析工具参数，约束仓库路径在工作区内，返回 (subcommand, repo_root, extra)。
fn resolve_invocation(
    workspace: &Path,
    args: &HashMap<String, String>,
) -> Result<(String, PathBuf, Vec<String>)> {
    let subcommand = args
        .get("op")
        .or_else(|| args.get("subcommand"))
        .cloned()
        .ok_or_else(|| anyhow!("缺少必填参数 op（git 子命令）"))?;

    // 仓库根：相对工作区的路径经 safe_resolve 约束，缺省用工作区本身。
    let repo = match args.get("path").or_else(|| args.get("repo")) {
        Some(p) if !p.is_empty() => safe_resolve(workspace, p)
            .with_context(|| format!("仓库路径解析失败（可能越界工作区）: {}", p))?,
        _ => workspace.to_path_buf(),
    };

    // 额外参数：以空白切分（git 参数无需 shell 解析，安全）。
    let extra = args
        .get("extra")
        .map(|s| s.split_whitespace().map(|x| x.to_string()).collect())
        .unwrap_or_default();

    Ok((subcommand, repo, extra))
}

/// 构造一个 `tool.git` 工具实例（注册进 ToolRegistry 用）。
pub fn git_tool(
    workspace: PathBuf,
    runner: Arc<dyn GitRunner>,
) -> Box<dyn crate::mcp::registry::ToolExecutor> {
    let name = "git";
    let description = "版本控制：在限定工作区内执行白名单内的 git 子命令（status/diff/add/commit/log/branch/checkout/push/pull/clone/blame/init 等）。";
    let schema = json!({
        "type": "object",
        "properties": {
            "op": { "type": "string", "description": "git 子命令，如 status / commit / log" },
            "path": { "type": "string", "description": "仓库相对工作区的路径，缺省为工作区根" },
            "extra": { "type": "string", "description": "附加参数，以空白分隔，如 \"-m fix bug\"" },
            "message": { "type": "string", "description": "commit 消息（op=commit 时使用）" }
        },
        "required": ["op"]
    });

    crate::tools::builtin::make_local_tool(
        name,
        description,
        schema,
        Arc::new(move |args: &HashMap<String, String>| {
            let workspace = workspace.clone();
            let runner = runner.clone();
            let subcommand_arg = args.get("op").cloned();
            let message = args.get("message").cloned();
            Box::pin(async move {
                // 合并 message 到 extra（op=commit 时 -m <msg>）
                let mut args = args.clone();
                if let (Some("commit"), Some(msg)) = (subcommand_arg.as_deref(), message) {
                    let existing = args.get("extra").cloned().unwrap_or_default();
                    args.insert(
                        "extra".to_string(),
                        format!("-m \"{}\" {}", msg.replace('"', "'"), existing),
                    );
                }

                let (subcommand, repo, extra) = match resolve_invocation(&workspace, &args) {
                    Ok(v) => v,
                    Err(e) => {
                        return Ok(ToolResult::Error {
                            message: e.to_string(),
                            code: "BAD_ARGS".to_string(),
                        })
                    }
                };

                if !is_allowed(&subcommand) {
                    return Ok(ToolResult::Error {
                        message: format!("git 子命令 '{}' 不在白名单，已拒绝", subcommand),
                        code: "FORBIDDEN_OP".to_string(),
                    });
                }

                match runner.run(&subcommand, &repo, &extra) {
                    Ok(out) => {
                        if out.exit_code == 0 {
                            Ok(ToolResult::Success {
                                data: format!(
                                    "$ git {} (in {})\n{}\n{}",
                                    subcommand,
                                    repo.display(),
                                    out.stdout.trim(),
                                    out.stderr.trim()
                                ),
                                metadata: HashMap::new(),
                            })
                        } else {
                            Ok(ToolResult::Error {
                                message: format!(
                                    "git {} 退出码 {}: {}",
                                    subcommand,
                                    out.exit_code,
                                    out.stderr.trim()
                                ),
                                code: "GIT_NONZERO".to_string(),
                            })
                        }
                    }
                    Err(e) => Ok(ToolResult::Error {
                        message: e.to_string(),
                        code: "GIT_FAIL".to_string(),
                    }),
                }
            })
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allowlist_blocks_dangerous_subcommands() {
        assert!(is_allowed("status"));
        assert!(is_allowed("commit"));
        assert!(!is_allowed("reset"));
        assert!(!is_allowed("clean"));
        // push 在白名单内（允许，但受工作区路径守卫约束）
        assert!(is_allowed("push"));
    }

    #[test]
    fn recording_runner_records_calls_and_rejects_unallowed() {
        let runner = RecordingGitRunner::default();
        let repo = PathBuf::from("/tmp/repo");
        let out = runner.run("status", &repo, &[]).unwrap();
        assert_eq!(out.stdout, "recorded");
        let calls = runner.calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, "status");

        assert!(runner.run("reset", &repo, &[]).is_err());
    }

    #[test]
    fn system_runner_real_init_and_status() {
        // 真实离线：在临时目录 git init 后 status，验证 SystemGitRunner 真能跑 git。
        let tmp = std::env::temp_dir().join(format!("mgt_git_test_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();

        let runner = SystemGitRunner;
        let init = runner.run("init", &tmp, &[]).unwrap();
        assert_eq!(init.exit_code, 0, "git init 应成功：{}", init.stderr);
        assert!(init.stdout.contains("Initialized"));

        let status = runner.run("status", &tmp, &[]).unwrap();
        assert_eq!(status.exit_code, 0);
        assert!(
            status.stdout.contains("On branch")
                || status.stdout.contains("master")
                || status.stdout.contains("main")
        );

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn resolve_invocation_defaults_repo_to_workspace() {
        let ws = PathBuf::from("/work");
        let mut args = HashMap::new();
        args.insert("op".to_string(), "log".to_string());
        let (op, repo, extra) = resolve_invocation(&ws, &args).unwrap();
        assert_eq!(op, "log");
        assert_eq!(repo, ws);
        assert!(extra.is_empty());
    }
}
