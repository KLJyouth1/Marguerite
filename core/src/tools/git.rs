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
use crate::proc::SilentCommand;

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
    // 并行任务隔离：`git worktree add/list/remove/prune` 是多代理并行开发的
    // 基础设施（每个子代理独占一棵工作树，互不踩工作区）。remove/prune 不带
    // `--force` 不会丢未提交改动，且 `--force/-f/--hard` 均在 DENIED_FLAGS 里
    // fail-closed 拦截，故可安全纳入白名单。
    "worktree",
];

fn is_allowed(subcommand: &str) -> bool {
    ALLOWED_SUBCOMMANDS.contains(&subcommand)
}

/// 危险 flag 黑名单（fail-closed：命中即拒绝）。
///
/// 子命令白名单只约束「做什么」，约束不了「怎么做」——`git push --force`、
/// `git config --global ...`、`git commit --no-verify` 都能在白名单子命令下
/// 造成不可逆破坏 / 越出工作区 / 绕过钩子。这些 flag 在任何子命令下都不放行。
const DENIED_FLAGS: &[&str] = &[
    "--force",
    "-f",
    "--force-with-lease",
    "--hard",
    "--global",
    "--system",
    "--no-verify",
    "--mirror",
    "--prune",
    "--delete",
    "-D",
];

/// 返回 `extra` 中首个命中的危险 flag（若有）。
fn denied_flag(extra: &[String]) -> Option<&str> {
    extra
        .iter()
        .find(|a| DENIED_FLAGS.contains(&a.as_str()))
        .map(String::as_str)
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
        cmd.silent().arg(subcommand).args(extra).current_dir(repo);
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
    workspace: &crate::tools::fs_path::RootSet,
    args: &HashMap<String, String>,
) -> Result<(String, PathBuf, Vec<String>)> {
    let subcommand = args
        .get("op")
        .or_else(|| args.get("subcommand"))
        .cloned()
        .ok_or_else(|| anyhow!("缺少必填参数 op（git 子命令）"))?;

    // 仓库根：相对路径可命中任一登记根（多根会话），缺省用主根。
    let repo = match args.get("path").or_else(|| args.get("repo")) {
        Some(p) if !p.is_empty() => workspace
            .resolve(p)
            .with_context(|| format!("仓库路径解析失败（可能越界工作区）: {}", p))?,
        _ => workspace.primary().to_path_buf(),
    };

    // 额外参数：以空白切分（git 参数无需 shell 解析，安全）。
    let mut extra: Vec<String> = args
        .get("extra")
        .map(|s| s.split_whitespace().map(|x| x.to_string()).collect())
        .unwrap_or_default();

    // commit 消息：作为**独立 argv 元素**追加，避免多词消息被空白切散
    // （此前把 `-m "msg"` 拼进 extra 串再 split_whitespace → `["-m","\"fix","bug\""]`）。
    if subcommand == "commit" {
        if let Some(msg) = args.get("message").filter(|m| !m.is_empty()) {
            extra.push("-m".to_string());
            extra.push(msg.clone());
        }
    }

    Ok((subcommand, repo, extra))
}

/// 构造一个 `tool.git` 工具实例（注册进 ToolRegistry 用）。
pub fn git_tool(
    workspace: impl Into<crate::tools::fs_path::RootSet>,
    runner: Arc<dyn GitRunner>,
) -> Box<dyn crate::mcp::registry::ToolExecutor> {
    let workspace: crate::tools::fs_path::RootSet = workspace.into();
    let name = "git";
    let description = "版本控制：在限定工作区内执行白名单内的 git 子命令（status/diff/add/commit/log/branch/checkout/push/pull/clone/blame/init/worktree 等）。worktree 用于为并行子任务创建隔离工作树（git worktree add <path> <branch>）。";
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
            Box::pin(async move {
                let (subcommand, repo, extra) = match resolve_invocation(&workspace, args) {
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

                // 子命令白名单之上的参数级复核：危险 flag 一律拒绝（fail-closed）。
                if let Some(flag) = denied_flag(&extra) {
                    return Ok(ToolResult::Error {
                        message: format!(
                            "git 参数 '{}' 属于危险操作（force/global/no-verify 等），已拒绝",
                            flag
                        ),
                        code: "FORBIDDEN_FLAG".to_string(),
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

    fn v(xs: &[&str]) -> Vec<String> {
        xs.iter().map(|s| s.to_string()).collect()
    }

    /// 危险 flag 在**任何**子命令下都不放行：force/global/no-verify 等。
    #[test]
    fn denied_flags_reject_force_and_global() {
        assert_eq!(denied_flag(&v(&["--force"])), Some("--force"));
        assert_eq!(denied_flag(&v(&["-f"])), Some("-f"));
        assert_eq!(
            denied_flag(&v(&["--global", "user.name", "x"])),
            Some("--global")
        );
        assert_eq!(denied_flag(&v(&["--no-verify"])), Some("--no-verify"));
        assert_eq!(denied_flag(&v(&["--hard"])), Some("--hard"));
        // 良性 flag 不受影响
        assert!(denied_flag(&v(&["--oneline", "-n", "5"])).is_none());
        assert!(denied_flag(&[]).is_none());
    }

    /// commit 多词消息必须保持为**单个** argv 元素，不能被空白切散。
    #[test]
    fn commit_message_kept_as_single_argv_element() {
        let ws = crate::tools::fs_path::RootSet::single(PathBuf::from("/work"));
        let mut args = HashMap::new();
        args.insert("op".to_string(), "commit".to_string());
        args.insert("message".to_string(), "fix: multi word message".to_string());
        let (op, _repo, extra) = resolve_invocation(&ws, &args).unwrap();
        assert_eq!(op, "commit");
        assert_eq!(
            extra,
            vec!["-m".to_string(), "fix: multi word message".to_string()]
        );
    }

    /// 非 commit 子命令不注入 -m。
    #[test]
    fn non_commit_ignores_message() {
        let ws = crate::tools::fs_path::RootSet::single(PathBuf::from("/work"));
        let mut args = HashMap::new();
        args.insert("op".to_string(), "status".to_string());
        args.insert("message".to_string(), "ignored".to_string());
        let (_, _, extra) = resolve_invocation(&ws, &args).unwrap();
        assert!(extra.is_empty());
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
        let ws = crate::tools::fs_path::RootSet::single(PathBuf::from("/work"));
        let mut args = HashMap::new();
        args.insert("op".to_string(), "log".to_string());
        let (op, repo, extra) = resolve_invocation(&ws, &args).unwrap();
        assert_eq!(op, "log");
        assert_eq!(repo, ws.primary().to_path_buf());
        assert!(extra.is_empty());
    }
}
