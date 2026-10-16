//! git 检查点与回滚
//!
//! 在高风险自动编辑前留一个可回滚快照，出错时一键恢复。
//!
//! ## 实现取舍（诚实说明）
//! - 快照 = 一个 **git tree 对象**：经 `GIT_INDEX_FILE` 指向的**临时索引**执行
//!   `add -A` + `write-tree` 得到。用临时索引的原因是**绝不破坏用户已暂存的内容**
//!   （这是 `git stash` 的同款手法）；同时 tree 对象不需要 `user.name/user.email`，
//!   在未配置 git 身份的 CI 上同样可用。
//! - 回滚 = `read-tree` 载入索引 + `checkout-index -a -f` 写回工作区。
//! - **已知边界**：回滚会恢复快照内的文件内容，但**不会删除**快照之后新建的文件
//!   （删除需要 `git clean`，属破坏性操作，本模块刻意不提供）。
//! - **已知边界**：悬挂 tree 对象可能被 `git gc --prune` 回收，检查点适用于
//!   单次会话内的短期回滚，不作为长期备份。
//!
//! ## 安全约束
//! - 全程 `std::process::Command` 直连 `git` 二进制，**不经 shell**（shell=false），
//!   天然免疫 shell 注入；
//! - 子命令走白名单（fail-closed），参数全部为内部字面量或已校验对象哈希；
//! - [`CheckpointId`] 只接受 40/64 位十六进制，因此**不可能**以 `-` 开头被 git
//!   误当作选项（杜绝参数注入）；
//! - 仓库路径经 canonicalize 且校验为 git 仓库后才作为 `current_dir`；
//! - 每次调用带超时，超时即 kill，避免 git 卡死拖住 agent。

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{anyhow, Context, Result};
use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::proc::SilentCommand;
use crate::tools::git::GitOutput;

/// git 调用默认超时
pub const DEFAULT_GIT_TIMEOUT: Duration = Duration::from_secs(30);

/// 检查点允许的 git 子命令白名单（fail-closed）
///
/// 刻意排除 `clean`/`reset`/`push` 等破坏性或有副作用的子命令。
const CHECKPOINT_SUBCOMMANDS: &[&str] = &[
    "rev-parse",
    "add",
    "write-tree",
    "read-tree",
    "checkout-index",
];

/// 检查点 ID（git 对象哈希）
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CheckpointId(String);

impl CheckpointId {
    /// 校验并构造
    ///
    /// 仅接受 40 位（SHA-1）或 64 位（SHA-256 仓库）小写十六进制。
    /// 这是**参数注入的关键防线**：任何含 `-`、空格、路径分隔符的输入一律拒绝。
    pub fn parse(raw: &str) -> Result<Self> {
        let s = raw.trim();
        let valid_len = s.len() == 40 || s.len() == 64;
        let all_hex = !s.is_empty() && s.chars().all(|c| c.is_ascii_hexdigit());
        if !valid_len || !all_hex {
            return Err(anyhow!("非法检查点 ID（需 40/64 位十六进制）：{:?}", raw));
        }
        Ok(Self(s.to_ascii_lowercase()))
    }

    /// 原始哈希字符串
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for CheckpointId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// 检查点句柄
#[derive(Debug, Clone)]
pub struct Checkpoint {
    /// 快照对象 ID（git tree）
    pub id: CheckpointId,
    /// 所属仓库根（已 canonicalize）
    pub repo: PathBuf,
    /// 创建时刻
    pub created_at: DateTime<Utc>,
}

/// git 检查点管理器
#[derive(Debug, Clone)]
pub struct GitCheckpoints {
    /// 单次 git 调用超时
    timeout: Duration,
}

impl Default for GitCheckpoints {
    fn default() -> Self {
        Self {
            timeout: DEFAULT_GIT_TIMEOUT,
        }
    }
}

impl GitCheckpoints {
    /// 创建管理器（默认超时）
    pub fn new() -> Self {
        Self::default()
    }

    /// 创建管理器（自定义超时）
    pub fn with_timeout(timeout: Duration) -> Self {
        Self { timeout }
    }

    /// 当前超时设置
    pub fn timeout(&self) -> Duration {
        self.timeout
    }

    /// 创建检查点
    ///
    /// 用临时索引快照当前工作区（含未暂存改动与未跟踪文件），返回 tree 对象 ID。
    /// **不改动**用户的真实索引、HEAD 与任何分支。
    pub fn create_checkpoint(&self, repo_path: &Path) -> Result<Checkpoint> {
        let repo = self.validate_repo(repo_path)?;
        let git_dir = self.absolute_git_dir(&repo)?;

        // 临时索引与 .git 同盘，避免跨卷 rename 与权限问题
        let temp_index = git_dir.join(format!("marguerite_ckpt_{}.index", Uuid::new_v4()));
        let index_env = vec![(
            "GIT_INDEX_FILE".to_string(),
            temp_index.to_string_lossy().to_string(),
        )];

        let result = (|| -> Result<CheckpointId> {
            // `add -A` 写入临时索引：全部跟踪 + 未跟踪文件（尊重 .gitignore）
            let added = self.run_git("add", &repo, &["-A".to_string()], &index_env)?;
            if added.exit_code != 0 {
                return Err(anyhow!(
                    "git add 失败（exit={}）：{}",
                    added.exit_code,
                    added.stderr.trim()
                ));
            }
            let tree = self.run_git("write-tree", &repo, &[], &index_env)?;
            if tree.exit_code != 0 {
                return Err(anyhow!(
                    "git write-tree 失败（exit={}）：{}",
                    tree.exit_code,
                    tree.stderr.trim()
                ));
            }
            CheckpointId::parse(&tree.stdout)
        })();

        // 临时索引无论成败都清理（失败不掩盖主错误）
        let _ = std::fs::remove_file(&temp_index);

        Ok(Checkpoint {
            id: result?,
            repo,
            created_at: Utc::now(),
        })
    }

    /// 回滚到检查点
    ///
    /// 恢复快照中的文件内容到索引与工作区。
    /// 注意：快照之后**新建**的文件不会被删除（见模块文档「已知边界」）。
    pub fn rollback(&self, checkpoint: &Checkpoint) -> Result<()> {
        self.rollback_in(&checkpoint.repo, &checkpoint.id)
    }

    /// 回滚到指定仓库的指定对象
    pub fn rollback_in(&self, repo_path: &Path, id: &CheckpointId) -> Result<()> {
        let repo = self.validate_repo(repo_path)?;

        // 对象必须存在且确实是 tree，避免把任意哈希写进索引
        let spec = format!("{}^{{tree}}", id.as_str());
        let verified = self.run_git(
            "rev-parse",
            &repo,
            &["--verify".to_string(), "--quiet".to_string(), spec],
            &[],
        )?;
        if verified.exit_code != 0 {
            return Err(anyhow!(
                "检查点对象 {} 不存在或不是 tree（可能已被 git gc 回收）",
                id
            ));
        }

        let read = self.run_git("read-tree", &repo, &[id.as_str().to_string()], &[])?;
        if read.exit_code != 0 {
            return Err(anyhow!(
                "git read-tree 失败（exit={}）：{}",
                read.exit_code,
                read.stderr.trim()
            ));
        }

        let checkout = self.run_git(
            "checkout-index",
            &repo,
            &["-a".to_string(), "-f".to_string()],
            &[],
        )?;
        if checkout.exit_code != 0 {
            return Err(anyhow!(
                "git checkout-index 失败（exit={}）：{}",
                checkout.exit_code,
                checkout.stderr.trim()
            ));
        }
        Ok(())
    }

    /// 校验并归一化仓库路径
    fn validate_repo(&self, repo_path: &Path) -> Result<PathBuf> {
        if !repo_path.is_dir() {
            return Err(anyhow!("仓库路径不存在或不是目录：{}", repo_path.display()));
        }
        let repo = std::fs::canonicalize(repo_path)
            .with_context(|| format!("无法归一化仓库路径：{}", repo_path.display()))?;
        let out = self.run_git(
            "rev-parse",
            &repo,
            &["--is-inside-work-tree".to_string()],
            &[],
        )?;
        if out.exit_code != 0 || out.stdout.trim() != "true" {
            return Err(anyhow!("不是 git 工作区：{}", repo.display()));
        }
        Ok(repo)
    }

    /// 取仓库的绝对 .git 目录
    fn absolute_git_dir(&self, repo: &Path) -> Result<PathBuf> {
        let out = self.run_git("rev-parse", repo, &["--absolute-git-dir".to_string()], &[])?;
        if out.exit_code != 0 {
            return Err(anyhow!("无法定位 .git 目录：{}", out.stderr.trim()));
        }
        let dir = PathBuf::from(out.stdout.trim());
        if !dir.is_dir() {
            return Err(anyhow!("解析出的 .git 目录不可用：{}", dir.display()));
        }
        Ok(dir)
    }

    /// 执行一次 git 调用（白名单 + 无 shell + 超时 + 管道全量读取）
    ///
    /// `extra` 仅允许内部字面量或已校验的对象哈希；`envs` 用于注入
    /// `GIT_INDEX_FILE` 等仅作用于本次调用的变量。
    fn run_git(
        &self,
        subcommand: &str,
        repo: &Path,
        extra: &[String],
        envs: &[(String, String)],
    ) -> Result<GitOutput> {
        if !CHECKPOINT_SUBCOMMANDS.contains(&subcommand) {
            return Err(anyhow!(
                "git 子命令 '{}' 不在检查点白名单，已拒绝（fail-closed）",
                subcommand
            ));
        }

        let mut cmd = Command::new("git");
        cmd.silent()
            .arg(subcommand)
            .args(extra)
            .current_dir(repo)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        for (k, v) in envs {
            cmd.env(k, v);
        }

        let mut child = cmd
            .spawn()
            .with_context(|| format!("启动 git {subcommand} 失败（git 是否安装？）"))?;

        // 独立线程抽干管道：避免输出超过管道缓冲导致子进程阻塞而永不退出
        let mut out_pipe = child
            .stdout
            .take()
            .ok_or_else(|| anyhow!("无法捕获 git stdout"))?;
        let mut err_pipe = child
            .stderr
            .take()
            .ok_or_else(|| anyhow!("无法捕获 git stderr"))?;
        let out_reader = thread::spawn(move || {
            let mut buf = String::new();
            let _ = out_pipe.read_to_string(&mut buf);
            buf
        });
        let err_reader = thread::spawn(move || {
            let mut buf = String::new();
            let _ = err_pipe.read_to_string(&mut buf);
            buf
        });

        let deadline = Instant::now() + self.timeout;
        let status = loop {
            match child.try_wait()? {
                Some(status) => break status,
                None => {
                    if Instant::now() >= deadline {
                        let _ = child.kill();
                        let _ = child.wait();
                        return Err(anyhow!(
                            "git {} 超时（>{:?}），已终止",
                            subcommand,
                            self.timeout
                        ));
                    }
                    thread::sleep(Duration::from_millis(20));
                }
            }
        };

        let stdout = out_reader
            .join()
            .map_err(|_| anyhow!("git stdout 读取线程 panic"))?;
        let stderr = err_reader
            .join()
            .map_err(|_| anyhow!("git stderr 读取线程 panic"))?;

        Ok(GitOutput {
            exit_code: status.code().unwrap_or(-1),
            stdout,
            stderr,
        })
    }
}

/// 创建检查点（默认超时）
pub fn create_checkpoint(repo_path: &Path) -> Result<Checkpoint> {
    GitCheckpoints::new().create_checkpoint(repo_path)
}

/// 回滚到检查点（默认超时）
pub fn rollback(checkpoint: &Checkpoint) -> Result<()> {
    GitCheckpoints::new().rollback(checkpoint)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 本机是否可用 git（不可用时相关用例直接放过，不伪造成功）
    fn git_available() -> bool {
        Command::new("git")
            .arg("--version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }

    /// 建一个独立的临时仓库（含一次初始提交所需的最小配置）
    fn temp_repo(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("marguerite_ckpt_{tag}_{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let run = |args: &[&str]| {
            Command::new("git")
                .args(args)
                .current_dir(&dir)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .unwrap();
        };
        run(&["init"]);
        // 仓库级身份配置：tree 快照本身不需要，但保持仓库可用状态
        run(&["config", "user.email", "harness@example.com"]);
        run(&["config", "user.name", "harness"]);
        std::fs::canonicalize(&dir).unwrap_or(dir)
    }

    #[test]
    fn test_checkpoint_id_rejects_injection_shapes() {
        // 参数注入面：以 - 开头会被 git 当作选项
        assert!(CheckpointId::parse("--upload-pack=evil").is_err());
        assert!(CheckpointId::parse("-n").is_err());
        // 路径穿越 / 命令拼接形态
        assert!(CheckpointId::parse("../../etc/passwd").is_err());
        assert!(CheckpointId::parse("abc; rm -rf /").is_err());
        assert!(CheckpointId::parse("HEAD").is_err());
        // 长度不符
        assert!(CheckpointId::parse("deadbeef").is_err());
        assert!(CheckpointId::parse("").is_err());
        // 非十六进制字符
        assert!(CheckpointId::parse(&"z".repeat(40)).is_err());
    }

    #[test]
    fn test_checkpoint_id_accepts_valid_hashes() {
        let sha1 = "a".repeat(40);
        assert_eq!(CheckpointId::parse(&sha1).unwrap().as_str(), sha1);
        let sha256 = "b".repeat(64);
        assert!(CheckpointId::parse(&sha256).is_ok());
        // 大写归一化为小写，且容忍首尾空白（git 输出带换行）
        let upper = format!("  {}\n", "AB".repeat(20));
        assert_eq!(
            CheckpointId::parse(&upper).unwrap().as_str(),
            "ab".repeat(20)
        );
    }

    #[test]
    fn test_subcommand_whitelist_is_fail_closed() {
        let ck = GitCheckpoints::new();
        let repo = std::env::temp_dir();
        // 破坏性子命令必须被拒（且不产生任何进程）
        for bad in ["clean", "reset", "push", "checkout"] {
            let err = ck
                .run_git(bad, &repo, &[], &[])
                .expect_err("非白名单子命令必须被拒绝")
                .to_string();
            assert!(err.contains("白名单"), "错误信息应说明白名单：{err}");
        }
    }

    #[test]
    fn test_validate_repo_rejects_non_repo() {
        if !git_available() {
            return;
        }
        let ck = GitCheckpoints::new();
        let dir = std::env::temp_dir().join(format!("marguerite_not_repo_{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        assert!(ck.validate_repo(&dir).is_err(), "非 git 目录不应通过校验");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_validate_repo_rejects_missing_path() {
        let ck = GitCheckpoints::new();
        let missing = std::env::temp_dir().join(format!("nope_{}", Uuid::new_v4()));
        assert!(ck.validate_repo(&missing).is_err());
    }

    #[test]
    fn test_create_and_rollback_restores_content() {
        if !git_available() {
            return;
        }
        let repo = temp_repo("roundtrip");
        let file = repo.join("note.txt");
        std::fs::write(&file, "原始内容").unwrap();

        let ck = GitCheckpoints::new();
        let checkpoint = ck.create_checkpoint(&repo).expect("创建检查点应成功");
        assert_eq!(checkpoint.id.as_str().len(), 40);

        // 破坏文件
        std::fs::write(&file, "被改坏了").unwrap();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "被改坏了");

        ck.rollback(&checkpoint).expect("回滚应成功");
        assert_eq!(
            std::fs::read_to_string(&file).unwrap(),
            "原始内容",
            "回滚后文件内容应恢复"
        );

        let _ = std::fs::remove_dir_all(&repo);
    }

    #[test]
    fn test_create_checkpoint_preserves_user_index() {
        if !git_available() {
            return;
        }
        let repo = temp_repo("index");
        std::fs::write(repo.join("staged.txt"), "staged").unwrap();
        std::fs::write(repo.join("untracked.txt"), "untracked").unwrap();

        // 用户只暂存了 staged.txt
        Command::new("git")
            .args(["add", "staged.txt"])
            .current_dir(&repo)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .unwrap();

        let ck = GitCheckpoints::new();
        ck.create_checkpoint(&repo).expect("创建检查点应成功");

        // 检查点走临时索引，用户真实索引不应被 add -A 污染
        let out = Command::new("git")
            .args(["diff", "--cached", "--name-only"])
            .current_dir(&repo)
            .output()
            .unwrap();
        let staged = String::from_utf8_lossy(&out.stdout);
        assert!(staged.contains("staged.txt"), "原暂存文件应保留：{staged}");
        assert!(
            !staged.contains("untracked.txt"),
            "检查点不得把未跟踪文件塞进用户索引：{staged}"
        );

        let _ = std::fs::remove_dir_all(&repo);
    }

    #[test]
    fn test_no_leftover_temp_index() {
        if !git_available() {
            return;
        }
        let repo = temp_repo("cleanup");
        std::fs::write(repo.join("a.txt"), "a").unwrap();

        let ck = GitCheckpoints::new();
        ck.create_checkpoint(&repo).unwrap();

        let git_dir = repo.join(".git");
        let leftovers: Vec<_> = std::fs::read_dir(&git_dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.file_name()
                    .to_string_lossy()
                    .starts_with("marguerite_ckpt_")
            })
            .collect();
        assert!(leftovers.is_empty(), "临时索引文件应被清理");

        let _ = std::fs::remove_dir_all(&repo);
    }

    #[test]
    fn test_rollback_rejects_unknown_object() {
        if !git_available() {
            return;
        }
        let repo = temp_repo("unknown");
        let ck = GitCheckpoints::new();
        let bogus = CheckpointId::parse(&"0".repeat(40)).unwrap();
        let err = ck
            .rollback_in(&repo, &bogus)
            .expect_err("不存在的对象应回滚失败")
            .to_string();
        assert!(
            err.contains("不存在") || err.contains("tree"),
            "错误应可诊断：{err}"
        );
        let _ = std::fs::remove_dir_all(&repo);
    }

    #[test]
    fn test_timeout_is_configurable() {
        let ck = GitCheckpoints::with_timeout(Duration::from_millis(1));
        assert_eq!(ck.timeout(), Duration::from_millis(1));
        assert_eq!(GitCheckpoints::new().timeout(), DEFAULT_GIT_TIMEOUT);
    }
}
