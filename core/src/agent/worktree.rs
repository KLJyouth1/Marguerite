//! git worktree 并行任务编排（多代理并行开发的隔离基础设施）
//!
//! ## 为什么用 git worktree
//! 多个子代理并行改代码时，若共享同一个工作区，会互相踩暂存区/工作区文件，
//! 一个代理的半成品改动会被另一个代理看到并污染其 diff。`git worktree` 让
//! 每个子代理独占一棵**独立检出的工作树**（独立 HEAD / 索引 / 工作区文件），
//! 同时共享同一个对象库（不复制历史），是多代理并行开发的隔离基础：
//! 代理间天然互不干扰，完成后各分支独立合并/取舍。
//!
//! ## 为什么不用 submodule
//! submodule 是「把外部仓库挂成子目录」的依赖管理机制：它共享检出语义、
//! 对 `.git` 目录/索引锁有共享写竞争，并不提供「同一仓库的并行写隔离」。
//! 子代理场景需要的是**同一仓库多分支并行检出**，worktree 语义精确匹配；
//! submodule 反而引入跨仓库版本钉扎与锁竞争问题。
//!
//! ## 为什么路径限制在 `.marguerite/worktrees` 下
//! 1. **路径穿越防护**：所有落点强制收敛在固定子树内，`alloc` 的名字即使被
//!    污染成 `../../x` 也无法越出（见 [`WorktreeManager::alloc`] 的
//!    [`crate::tools::fs_path::safe_resolve`] 兜底）。
//! 2. **与主工作区隔离**：`.marguerite/` 是本项目既有约定目录（trash /
//!    subagents 转录同用），主工作树的用户文件不会与临时工作树混排。
//! 3. **易整体清理**：整个子树都是一次性临时产物，可按目录统一回收
//!    （配合 [`Self::prune`] 清理失效登记）。
//!
//! ## 安全伦理（fail-closed）
//! - 仓库校验：`git rev-parse --is-inside-work-tree` 通过才接受 base；
//! - 参数注入防护：名字/分支名校验拒绝 `-` 开头、路径分隔符、`..` 等形态
//!   （参数永远经 `Command` 数组传递，不经 shell）；
//! - 危险 flag 黑名单：`--force` / `-f` / `--hard` / `--delete` / `-D` 等
//!   一律拒绝（与 [`crate::tools::git`] 同一伦理）；
//! - 拒绝把主工作树当工作树移除；
//! - 并发安全：`alloc` 经 [`WORKTREE_LOCKS`] 按仓库根分桶串行化，防并发
//!   抢占同名 ID / 分支。

use std::collections::HashMap;
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, LazyLock, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{anyhow, Context, Result};
use uuid::Uuid;

use crate::proc::SilentCommand;
use crate::tools::git::GitOutput;

/// git 调用默认超时（与 `harness/checkpoint::DEFAULT_GIT_TIMEOUT` 同值）
const DEFAULT_GIT_TIMEOUT: Duration = Duration::from_secs(30);

/// 允许的 git 子命令白名单（fail-closed：不在表内一律拒绝）。
/// worktree 编排只涉及 add/list/remove/prune/rev-parse 五种动作。
const ALLOWED_SUBCOMMANDS: &[&str] = &["worktree", "rev-parse"];

/// 危险 flag 黑名单（与 `tools/git.rs` 同一伦理，fail-closed：命中即拒绝）。
/// 即便将来子命令白名单放宽，这些 flag 也不放行：它们能让清理动作越过
/// 「无损移除」边界（丢弃未提交改动 / 递归清目录 / 删分支等不可逆形态）。
const DENIED_FLAGS: &[&str] = &[
    "--force",
    "-f",
    "--force-with-lease",
    "--hard",
    "--global",
    "--system",
    "--no-verify",
    "--mirror",
    "--prune-tags",
    "--delete",
    "-D",
    "--orphan",
];

/// 返回 `extra` 中首个命中的危险 flag（若有）
fn denied_flag<'a>(extra: &[&'a str]) -> Option<&'a str> {
    extra.iter().copied().find(|a| DENIED_FLAGS.contains(a))
}

/// 按仓库根分桶的进程级互斥锁（仿 `tools/trash.rs` 的 `TRASH_LOCKS`）。
///
/// `alloc` 会做「查名 → 建目录 → 起 git → 失败回滚」多步操作，若两个线程
/// 并发给同一仓库 alloc，会互相抢占同名工作树/分支、或在失败回滚时把对方
/// 刚建好的目录当半成品清掉。按仓库根串行化 `alloc` 即消除这类竞争。
static WORKTREE_LOCKS: LazyLock<Mutex<HashMap<PathBuf, Arc<Mutex<()>>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn worktree_lock(repo: &Path) -> Arc<Mutex<()>> {
    let mut map = WORKTREE_LOCKS.lock().unwrap_or_else(|e| e.into_inner());
    map.entry(repo.to_path_buf()).or_default().clone()
}

/// 校验工作树名字（目录名 / 分支段）：只收 `[A-Za-z0-9_-]` 且非空、不以 `-` 开头。
///
/// 这是参数注入的关键防线：以 `-` 开头会被 git 当作选项；含路径分隔符 /
/// `..` 会越出 `.marguerite/worktrees` 子树；空名会指向子树根本身。
fn validate_name(name: &str) -> Result<()> {
    if name.is_empty() {
        return Err(anyhow!("工作树名字不能为空"));
    }
    if name.starts_with('-') {
        return Err(anyhow!(
            "非法工作树名字（不得以 '-' 开头，避免被 git 当作选项）：{name:?}"
        ));
    }
    let ok = name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    if !ok {
        return Err(anyhow!(
            "非法工作树名字（仅允许字母/数字/'_'/'-'，拒绝路径分隔符与 '..'）：{name:?}"
        ));
    }
    Ok(())
}

/// 词法归一化：折叠 `.` 与 `..`（不触盘，仿 `tools/trash.rs` 的同名实现）。
fn normalize_lexically(p: &Path) -> PathBuf {
    let mut stack: Vec<Component> = Vec::new();
    for comp in p.components() {
        match comp {
            Component::ParentDir => {
                if matches!(stack.last(), Some(Component::Normal(_))) {
                    stack.pop();
                }
            }
            Component::CurDir => {}
            other => stack.push(other),
        }
    }
    stack.iter().collect()
}

/// 路径比较用的统一形态键：折叠 `.`/`..` + 去 `\\?\` 长路径前缀 +
/// 分隔符统一为 `/` + 小写化。
///
/// 为什么必须统一形态再比较：同一个目录在本模块里会以**四种形态**出现——
/// 1. `std::fs::canonicalize` 输出（Windows 带 `\\?\` 前缀、反斜杠）；
/// 2. 本模块对外暴露的路径（已 trim `\\?\`，反斜杠）；
/// 3. git CLI 的输出（正斜杠，无 `\\?\`）；
/// 4. 调用方/测试可能直接传入的任意一种。
///
/// 形态不同则 `Path` 相等 / `starts_with` 判定随之前后摇摆（主工作树误判、
/// 子树边界误判），因此所有**跨来源**路径比较统一走此键：
/// 按分隔符切段、小写（Windows 路径大小写不敏感）后逐段比较。
fn path_key(p: &Path) -> String {
    normalize_lexically(p)
        .to_string_lossy()
        .trim_start_matches(r"\\?\")
        .replace('\\', "/")
        .to_lowercase()
}

/// `p` 与 `q` 是否为同一路径（形态无关：`\\?\` / 分隔符 / 大小写差异均视为相同）
fn same_path(p: &Path, q: &Path) -> bool {
    path_key(p) == path_key(q)
}

/// `p` 是否落在 `root` 之下（含相等；形态无关）
fn is_under(p: &Path, root: &Path) -> bool {
    let pk = path_key(p);
    let rk = path_key(root);
    pk == rk || pk.starts_with(&format!("{rk}/"))
}

/// 传给 git 的路径形态：去 `\\?\` 长路径前缀。
///
/// git 的路径处理按 `\\?\E:\...` 原样解析会得到畸形落点
/// （如 `//?/E:/...`），因此所有进入 git 命令行的路径必须先转普通形态。
fn git_arg(p: &Path) -> String {
    p.to_string_lossy()
        .trim_start_matches(r"\\?\")
        .to_string()
}

/// 一棵已分配的工作树句柄
///
/// 持有 path / branch 双标识；[`Drop`] 默认自动清理（RAII），可经
/// [`Self::disarm`] 关闭自动清理（例如要把分支成果留给主工作树合并时）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorktreeHandle {
    /// 工作树检出目录（`.marguerite/worktrees/<session>/<name>`）
    pub path: PathBuf,
    /// 工作树所在分支（`marguerite/<session>/<name>`）
    pub branch: String,
    /// 所属仓库根（已 canonicalize）
    repo: PathBuf,
    /// 是否在 Drop 时自动清理
    auto_cleanup: bool,
}

impl WorktreeHandle {
    /// 关闭 Drop 自动清理（句柄保留，交给 [`WorktreeManager::remove`] / [`Self::cleanup`] 手动收尾）
    pub fn disarm(&mut self) {
        self.auto_cleanup = false;
    }

    /// 是否仍会在 Drop 时自动清理
    pub fn auto_cleanup(&self) -> bool {
        self.auto_cleanup
    }

    /// 主动清理（幂等：重复调用无副作用）
    ///
    /// 走「`git worktree remove`（无损，拒绝带危险 flag）→ 失败则目录回收 +
    /// `git worktree prune`（清理失效登记）」的兜底链。清理失败只做记录，
    /// 不向 Drop 路径 panic。
    pub fn cleanup(&mut self) -> Result<()> {
        self.auto_cleanup = false;
        remove_worktree_quiet(&self.repo, &self.path)
    }
}

impl Drop for WorktreeHandle {
    fn drop(&mut self) {
        if self.auto_cleanup {
            // Drop 里绝不 panic：清理失败静默留待 prune 兜底
            let _ = remove_worktree_quiet(&self.repo, &self.path);
        }
    }
}

/// 工作树登记项（[`WorktreeManager::list`] 的返回元素）
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorktreeInfo {
    /// 检出目录（git 原样输出）
    pub path: PathBuf,
    /// 关联提交（git 原样输出）
    pub head: String,
    /// 是否为裸仓库之外的主工作树
    pub is_main: bool,
}

/// git worktree 编排管理器
#[derive(Debug, Clone)]
pub struct WorktreeManager {
    /// 仓库根（已 canonicalize，经 `rev-parse --is-inside-work-tree` 校验）
    base_repo: PathBuf,
    /// 本次任务/会话 ID（作为分支命名空间，防跨会话互踩）
    session_id: String,
    /// 单次 git 调用超时
    timeout: Duration,
}

impl WorktreeManager {
    /// 创建管理器并校验 `base_repo` 是 git 工作区
    ///
    /// `session_id` 用作分支命名空间（`marguerite/<session_id>/<name>`）；
    /// 不合法时（空 / 含分隔符等）回退到 UUID，绝不静默用坏值拼进 ref 名。
    pub fn new(base_repo: PathBuf, session_id: &str) -> Result<Self> {
        let timeout = DEFAULT_GIT_TIMEOUT;
        let base = validate_repo(&base_repo, timeout)?;
        // trim `\\?\` 前缀：git 输出普通路径，canonicalize 可能带 `\\?\`；
        // 统一形态便于后续 is_main 比较。
        let base_str = base.to_string_lossy().to_string();
        let base = PathBuf::from(base_str.trim_start_matches(r"\\?\"));
        let session_id = if validate_name(session_id).is_ok() {
            session_id.to_string()
        } else {
            Uuid::new_v4().to_string()
        };
        Ok(Self {
            base_repo: base,
            session_id,
            timeout,
        })
    }

    /// 仓库根
    pub fn base_repo(&self) -> &Path {
        &self.base_repo
    }

    /// 会话 ID（分支命名空间）
    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    /// 工作树子树根：`<repo>/.marguerite/worktrees/<session_id>`（trim `\\?\` 前缀）
    pub fn worktree_root(&self) -> PathBuf {
        let p = self
            .base_repo
            .join(".marguerite")
            .join("worktrees")
            .join(&self.session_id);
        let s = p.to_string_lossy().to_string();
        PathBuf::from(s.trim_start_matches(r"\\?\"))
    }

    /// 分支名：`marguerite/<session_id>/<name>`
    pub fn branch_for(&self, name: &str) -> Result<String> {
        validate_name(name)?;
        Ok(format!("marguerite/{}/{name}", self.session_id))
    }

    /// 为子代理分配一棵隔离工作树
    ///
    /// 实际动作：`git worktree add <path> -b <branch>`（以当前 HEAD 起新分支）。
    /// 路径固定落在 `.marguerite/worktrees/<session_id>/<name>`（safe_resolve
    /// 兜底防穿越）；失败时自动收尾（目录回收 + `git worktree prune`）。
    ///
    /// 并发安全：整个 alloc 过程持 [`worktree_lock`]，同仓库串行。
    pub fn alloc(&self, name: &str) -> Result<WorktreeHandle> {
        validate_name(name)?;
        let branch = self.branch_for(name)?;

        // 按仓库根串行化：防并发抢占同名 ID/分支、防回滚误伤并发 alloc
        let lock = worktree_lock(&self.base_repo);
        let _held = lock.lock().unwrap_or_else(|e| e.into_inner());

        let path = crate::tools::fs_path::safe_resolve(&self.worktree_root(), name)
            .with_context(|| format!("非法工作树路径：{name:?}"))?;
        // safe_resolve 返回 canonicalize 后的路径（Windows 带 \\?\ 前缀），
        // git 不认这种形态；转回普通字符串传给 git。
        let path = PathBuf::from(git_arg(&path));
        if path.exists() {
            return Err(anyhow!(
                "工作树已存在：{}（请换名或先移除）",
                path.display()
            ));
        }

        let out = self.run_git(
            "worktree",
            &[
                "add".to_string(),
                git_arg(&path),
                "-b".to_string(),
                branch.clone(),
            ],
        )?;
        if out.exit_code != 0 {
            // 失败自动收尾：git 可能已留下半成品目录/登记，统一回收
            let _ = remove_worktree_quiet(&self.base_repo, &path);
            return Err(anyhow!(
                "git worktree add 失败（exit={}）：{}",
                out.exit_code,
                out.stderr.trim()
            ));
        }

        Ok(WorktreeHandle {
            path,
            branch,
            repo: self.base_repo.clone(),
            auto_cleanup: true,
        })
    }

    /// 列出本仓库全部工作树登记（含主工作树）
    ///
    /// 解析 `git worktree list --porcelain`：每个条目以 `worktree <path>` 起始，
    /// 可选 `HEAD <sha>` / `bare` / `detached` 等标注行。
    pub fn list(&self) -> Result<Vec<WorktreeInfo>> {
        let lock = worktree_lock(&self.base_repo);
        let _held = lock.lock().unwrap_or_else(|e| e.into_inner());
        let out = self.run_git("worktree", &["list".to_string(), "--porcelain".to_string()])?;
        if out.exit_code != 0 {
            return Err(anyhow!(
                "git worktree list 失败（exit={}）：{}",
                out.exit_code,
                out.stderr.trim()
            ));
        }
        Ok(parse_worktree_list(&out.stdout, &self.base_repo))
    }

    /// 清理失效的工作树登记（目录已被移走/删除的条目）
    ///
    /// 只有在 `git worktree list` 成功且登记数 ≥2 时才执行 `git worktree prune`
    /// （prune 本身无损：只清登记簿，不动任何工作区文件），避免空仓库上误伤。
    pub fn prune(&self) -> Result<()> {
        let lock = worktree_lock(&self.base_repo);
        let _held = lock.lock().unwrap_or_else(|e| e.into_inner());
        let count = self
            .run_git("worktree", &["list".to_string(), "--porcelain".to_string()])
            .map(|o| parse_worktree_list(&o.stdout, &self.base_repo).len())
            .unwrap_or(1);
        if count < 2 {
            return Ok(());
        }
        let out = self.run_git("worktree", &["prune".to_string()])?;
        if out.exit_code != 0 {
            return Err(anyhow!(
                "git worktree prune 失败（exit={}）：{}",
                out.exit_code,
                out.stderr.trim()
            ));
        }
        Ok(())
    }

    /// 移除一棵工作树（幂等：目录已不在则退化为 prune）
    ///
    /// 安全约束：拒绝移除主工作树；路径必须落在 `.marguerite/worktrees/` 下；
    /// 全程不接受任何调用方 flag（危险 flag 走 [`denied_flag`] 兜底拒绝）。
    pub fn remove(&self, handle: &WorktreeHandle) -> Result<()> {
        if !same_path(&handle.repo, &self.base_repo) {
            return Err(anyhow!(
                "句柄所属仓库与本管理器不一致：{} ≠ {}",
                handle.repo.display(),
                self.base_repo.display()
            ));
        }
        self.remove_path(&handle.path)
    }

    /// 按路径移除工作树（[`Self::remove`] 的内核，幂等）
    pub fn remove_path(&self, path: &Path) -> Result<()> {
        // 主工作树绝不可当工作树移除（形态无关比较：`\\?\` / 分隔符 / 大小写
        // 差异不改变判定）
        if same_path(path, &self.base_repo) {
            return Err(anyhow!("拒绝移除主工作树：{}", path.display()));
        }
        // 路径穿越防护：必须落在本管理器的 worktrees 子树下
        if !is_under(path, &self.worktree_root()) {
            return Err(anyhow!(
                "拒绝移除 .marguerite/worktrees 之外的路径（root: {}）：{}",
                self.worktree_root().display(),
                path.display()
            ));
        }

        let lock = worktree_lock(&self.base_repo);
        let _held = lock.lock().unwrap_or_else(|e| e.into_inner());
        if !path.exists() {
            // 目录已不在：只清登记
            return self.prune_locked();
        }
        remove_worktree_quiet(&self.base_repo, path)
    }

    /// 持锁调用 prune 内核（供已持 [`worktree_lock`] 的路径复用）
    fn prune_locked(&self) -> Result<()> {
        let count = self
            .run_git("worktree", &["list".to_string(), "--porcelain".to_string()])
            .map(|o| parse_worktree_list(&o.stdout, &self.base_repo).len())
            .unwrap_or(1);
        if count < 2 {
            return Ok(());
        }
        let out = self.run_git("worktree", &["prune".to_string()])?;
        if out.exit_code != 0 {
            return Err(anyhow!(
                "git worktree prune 失败（exit={}）：{}",
                out.exit_code,
                out.stderr.trim()
            ));
        }
        Ok(())
    }

    /// 执行一次 git 调用（白名单 + 危险 flag 拒绝 + 无 shell + 超时 + 管道抽干）
    fn run_git(&self, subcommand: &str, extra: &[String]) -> Result<GitOutput> {
        run_git_inner(subcommand, &self.base_repo, extra, self.timeout)
    }
}

/// 移除工作树的静默内核：`git worktree remove`（无损）→ 失败则目录回收 +
/// `git worktree prune` 兜底。返回清理结果但不 panic。
fn remove_worktree_quiet(repo: &Path, path: &Path) -> Result<()> {
    let out = run_git_inner(
        "worktree",
        repo,
        &["remove".to_string(), git_arg(path)],
        DEFAULT_GIT_TIMEOUT,
    )?;
    if out.exit_code == 0 {
        return Ok(());
    }
    // 兜底：目录本体回收（Windows 只读位先清）+ prune 清登记
    if path.exists() {
        let _ = crate::tools::trash::clear_readonly_for_delete_pub(path);
        let _ = std::fs::remove_dir_all(path);
    }
    let _ = run_git_inner("worktree", repo, &["prune".to_string()], DEFAULT_GIT_TIMEOUT);
    Ok(())
}

/// 校验 `repo` 是 git 工作区（`rev-parse --is-inside-work-tree` 通过）并归一化
fn validate_repo(repo_path: &Path, timeout: Duration) -> Result<PathBuf> {
    if !repo_path.is_dir() {
        return Err(anyhow!(
            "仓库路径不存在或不是目录：{}",
            repo_path.display()
        ));
    }
    let repo = std::fs::canonicalize(repo_path)
        .with_context(|| format!("无法归一化仓库路径：{}", repo_path.display()))?;
    let out = run_git_inner(
        "rev-parse",
        &repo,
        &["--is-inside-work-tree".to_string()],
        timeout,
    )?;
    if out.exit_code != 0 || out.stdout.trim() != "true" {
        return Err(anyhow!("不是 git 工作区：{}", repo.display()));
    }
    Ok(repo)
}

/// 会**覆盖 git 仓库发现**的环境变量（必须对每个子进程清掉）
///
/// 为什么必须清（实测踩过）：本模块总是用 `current_dir(repo)` 显式指定目标仓库，
/// 但 git 的 `GIT_DIR` / `GIT_INDEX_FILE` / `GIT_WORK_TREE` **优先级高于工作目录发现**。
/// 当本模块被 git 自己派生的进程调用时（最典型：pre-commit 钩子里跑 `cargo test`，
/// 或 `git commit` 触发的任何子进程），git 已把 `GIT_DIR`、`GIT_INDEX_FILE` 导出给子进程，
/// 于是嵌套 git 会作用到**外层仓库**：实测报
/// `fatal: Unable to create '<外层仓库>/.git/index.lock': File exists`（工作树建到别处），
/// 严重时还会改写用户外层仓库的索引。清掉这些变量后，目标仓库只由 `current_dir` 决定。
const GIT_ENV_OVERRIDES: [&str; 7] = [
    "GIT_DIR",
    "GIT_INDEX_FILE",
    "GIT_WORK_TREE",
    "GIT_COMMON_DIR",
    "GIT_OBJECT_DIRECTORY",
    "GIT_ALTERNATE_OBJECT_DIRECTORIES",
    "GIT_PREFIX",
];

fn sanitize_git_env(cmd: &mut Command) {
    for key in GIT_ENV_OVERRIDES {
        cmd.env_remove(key);
    }
}

/// git 调用内核（白名单 + 危险 flag 拒绝 + 无 shell + 超时 kill + 管道抽干）
///
/// 模式对齐 `harness/checkpoint::run_git`：独立线程抽干管道防子进程阻塞，
/// `try_wait` 轮询 + 超时 kill 防 git 卡死拖住 agent。
fn run_git_inner(subcommand: &str, repo: &Path, extra: &[String], timeout: Duration) -> Result<GitOutput> {
    if !ALLOWED_SUBCOMMANDS.contains(&subcommand) {
        return Err(anyhow!(
            "git 子命令 '{subcommand}' 不在 worktree 白名单，已拒绝（fail-closed）"
        ));
    }
    if let Some(flag) = denied_flag(
        &extra.iter().map(String::as_str).collect::<Vec<_>>(),
    ) {
        return Err(anyhow!("危险 git flag '{flag}' 已拒绝（fail-closed）"));
    }

    let mut cmd = Command::new("git");
    sanitize_git_env(&mut cmd);
    cmd.silent()
        .arg(subcommand)
        .args(extra)
        .current_dir(repo)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
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

    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait()? {
            Some(status) => break status,
            None => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(anyhow!("git {subcommand} 超时（>{timeout:?}），已终止"));
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

/// 解析 `git worktree list --porcelain` 输出
///
/// `is_main` 用 [`same_path`] 形态无关比较：git 输出是正斜杠普通形态，
/// 而 `base_repo` 可能来自 canonicalize（`\\?\` 前缀）或调用方任意形态。
fn parse_worktree_list(stdout: &str, base_repo: &Path) -> Vec<WorktreeInfo> {
    let mut items = Vec::new();
    for line in stdout.lines() {
        if let Some(p) = line.strip_prefix("worktree ") {
            let raw = p.trim();
            let path = PathBuf::from(raw);
            let is_main = same_path(&path, base_repo);
            items.push(WorktreeInfo {
                path,
                head: String::new(),
                is_main,
            });
        } else if let Some(h) = line.strip_prefix("HEAD ") {
            if let Some(last) = items.last_mut() {
                last.head = h.trim().to_string();
            }
        }
    }
    items
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Stdio;

    /// 本机是否可用 git（不可用时相关用例直接放过，不伪造成功）。
    /// 同样清 `GIT_*` 覆盖变量：本文件在 pre-commit 钩子（git 派生的进程）里也会被执行。
    fn git_available() -> bool {
        let mut cmd = Command::new("git");
        sanitize_git_env(&mut cmd);
        cmd.arg("--version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }

    /// 建一个独立的临时仓库（含一次初始提交所需的最小配置）
    fn temp_repo(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("marguerite_wt_{tag}_{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let run = |args: &[&str]| {
            let mut cmd = Command::new("git");
            sanitize_git_env(&mut cmd);
            cmd.args(args)
                .current_dir(&dir)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .unwrap();
        };
        run(&["init"]);
        run(&["config", "user.email", "harness@example.com"]);
        run(&["config", "user.name", "harness"]);
        // worktree add 以 HEAD 起新分支，需要至少一次提交
        std::fs::write(dir.join("README.md"), "# temp\n").unwrap();
        run(&["add", "README.md"]);
        run(&["commit", "-m", "init"]);
        std::fs::canonicalize(&dir).unwrap_or(dir)
    }

    #[test]
    fn test_worktree_alloc_creates_isolated_tree() {
        if !git_available() {
            return;
        }
        let repo = temp_repo("alloc");
        let mgr = WorktreeManager::new(repo.clone(), "sess").unwrap();

        let handle = mgr.alloc("agent-a").unwrap();
        assert!(handle.path.is_dir(), "alloc 后工作树目录应存在");
        assert!(is_under(&handle.path, &mgr.worktree_root()));
        assert_eq!(handle.branch, "marguerite/sess/agent-a");
        // 工作树里应有主仓库的检出内容
        assert!(handle.path.join("README.md").is_file());
        // 该工作树确实是 git 工作树
        assert!(handle.path.join(".git").exists() || handle.path.join(".git").is_file());

        let _ = std::fs::remove_dir_all(&repo);
    }

    #[test]
    fn test_worktree_cleanup_removes_tree_and_registration() {
        if !git_available() {
            return;
        }
        let repo = temp_repo("cleanup");
        let mgr = WorktreeManager::new(repo.clone(), "sess").unwrap();

        let mut handle = mgr.alloc("agent-b").unwrap();
        let path = handle.path.clone();
        handle.cleanup().unwrap();
        assert!(!path.exists(), "cleanup 后工作树目录应被回收");

        let listed = mgr.list().unwrap();
        assert!(
            !listed.iter().any(|w| same_path(&w.path, &path)),
            "cleanup 后登记簿不应再含该工作树"
        );

        let _ = std::fs::remove_dir_all(&repo);
    }

    #[test]
    fn test_worktree_drop_auto_cleanup_is_raii() {
        if !git_available() {
            return;
        }
        let repo = temp_repo("raii");
        let mgr = WorktreeManager::new(repo.clone(), "sess").unwrap();

        let path;
        {
            let handle = mgr.alloc("agent-c").unwrap();
            path = handle.path.clone();
            assert!(path.is_dir());
            // 作用域结束 → Drop 自动清理
        }
        assert!(!path.exists(), "Drop 后工作树目录应被自动回收");

        // disarm 后不再自动清理
        let path2;
        {
            let mut handle = mgr.alloc("agent-d").unwrap();
            handle.disarm();
            path2 = handle.path.clone();
        }
        assert!(path2.exists(), "disarm 后 Drop 不应清理");

        let _ = std::fs::remove_dir_all(&repo);
    }

    #[test]
    fn test_worktree_concurrent_alloc_no_conflict() {
        if !git_available() {
            return;
        }
        let repo = temp_repo("concurrent");
        let mgr = Arc::new(WorktreeManager::new(repo.clone(), "sess").unwrap());

        let mut joins = Vec::new();
        for i in 0..6 {
            let mgr = Arc::clone(&mgr);
            joins.push(std::thread::spawn(move || {
                // 每线程同名 + 异名混合：同名必须串行化出清晰错误，异名互不冲突
                let name = format!("agent-{i}");
                mgr.alloc(&name).unwrap()
            }));
        }
        let mut handles = Vec::new();
        for j in joins {
            handles.push(j.join().unwrap());
        }
        assert_eq!(handles.len(), 6);

        // 同名并发 alloc：第二个必须失败（不产生第二个同名工作树）
        let dup = std::thread::spawn({
            let mgr = Arc::clone(&mgr);
            move || mgr.alloc("agent-0")
        });
        assert!(dup.join().unwrap().is_err(), "同名并发 alloc 必须失败");

        // 全部路径互不相同且都在 worktree 子树下
        for h in &handles {
            assert!(is_under(&h.path, &mgr.worktree_root()));
        }
        let mut uniq: Vec<_> = handles.iter().map(|h| path_key(&h.path)).collect();
        uniq.sort();
        uniq.dedup();
        assert_eq!(uniq.len(), 6, "6 次 alloc 必须得到 6 个互不相同的工作树");

        let _ = std::fs::remove_dir_all(&repo);
    }

    #[test]
    fn test_worktree_path_traversal_rejected() {
        if !git_available() {
            return;
        }
        let repo = temp_repo("traversal");
        let mgr = WorktreeManager::new(repo.clone(), "sess").unwrap();

        // 名字穿越 / 选项注入 / 空名
        for bad in ["../escape", "..", "a/b", "-b", "--force", "", "a b"] {
            assert!(
                mgr.alloc(bad).is_err(),
                "非法名字必须被拒绝：{bad:?}"
            );
        }
        // remove_path 越界保护
        assert!(mgr.remove_path(&repo).is_err(), "主工作树不可移除");
        assert!(
            mgr.remove_path(Path::new("/tmp/outside")).is_err(),
            "子树外路径不可移除"
        );

        let _ = std::fs::remove_dir_all(&repo);
    }

    #[test]
    fn test_worktree_illegal_branch_name_rejected() {
        if !git_available() {
            return;
        }
        let repo = temp_repo("branch");
        let mgr = WorktreeManager::new(repo.clone(), "sess").unwrap();

        for bad in ["-x", "a b", "a/b", "..", "", "x\ty"] {
            assert!(
                mgr.branch_for(bad).is_err(),
                "非法分支名必须被拒绝：{bad:?}"
            );
        }
        assert_eq!(mgr.branch_for("ok_name-1").unwrap(), "marguerite/sess/ok_name-1");
        // 非法 session_id 回退到 UUID（不拼进 ref 名）
        let mgr2 = WorktreeManager::new(repo.clone(), "bad/sess").unwrap();
        assert!(mgr2.branch_for("n").unwrap().starts_with("marguerite/"));

        let _ = std::fs::remove_dir_all(&repo);
    }

    #[test]
    fn test_worktree_non_git_repo_rejected() {
        let dir = std::env::temp_dir().join(format!("marguerite_wt_nogit_{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let err = WorktreeManager::new(dir.clone(), "sess")
            .expect_err("非 git 仓库必须报错")
            .to_string();
        assert!(err.contains("git"), "错误应指明 git 校验原因：{err}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_worktree_prune_cleans_stale_registration() {
        if !git_available() {
            return;
        }
        let repo = temp_repo("prune");
        let mgr = WorktreeManager::new(repo.clone(), "sess").unwrap();

        let handle = mgr.alloc("agent-p").unwrap();
        let path = handle.path.clone();
        // 模拟「目录被外部移走」的失效登记：直接删目录，登记仍在
        let _ = crate::tools::trash::clear_readonly_for_delete_pub(&path);
        std::fs::remove_dir_all(&path).unwrap();
        assert!(mgr.list().unwrap().iter().any(|w| same_path(&w.path, &path)));

        mgr.prune().unwrap();
        assert!(
            !mgr.list().unwrap().iter().any(|w| same_path(&w.path, &path)),
            "prune 后失效登记应被清理"
        );

        let _ = std::fs::remove_dir_all(&repo);
    }

    #[test]
    fn test_worktree_list_reports_main_and_children() {
        if !git_available() {
            return;
        }
        let repo = temp_repo("list");
        let mgr = WorktreeManager::new(repo.clone(), "sess").unwrap();

        let before = mgr.list().unwrap();
        assert_eq!(before.len(), 1, "初始只有主工作树");
        assert!(before[0].is_main);
        assert!(!before[0].head.is_empty(), "主工作树应带 HEAD");

        let a = mgr.alloc("agent-x").unwrap();
        let b = mgr.alloc("agent-y").unwrap();
        let after = mgr.list().unwrap();
        assert_eq!(after.len(), 3, "主 + 2 子工作树");
        assert!(after.iter().filter(|w| w.is_main).count() == 1);
        assert!(
            after
                .iter()
                .any(|w| same_path(&w.path, &a.path) && !w.is_main)
        );
        assert!(
            after
                .iter()
                .any(|w| same_path(&w.path, &b.path) && !w.is_main)
        );
        // 子工作树 HEAD 非空（指向新分支提交）
        assert!(after.iter().filter(|w| !w.is_main).all(|w| !w.head.is_empty()));

        let _ = std::fs::remove_dir_all(&repo);
    }

    #[test]
    fn test_worktree_denied_flags_blocked() {
        if !git_available() {
            return;
        }
        let repo = temp_repo("denied");
        let _mgr = WorktreeManager::new(repo.clone(), "sess").unwrap();
        // 危险 flag 兜底：即便绕过上层 API 直接拼参数也必须被拒
        for bad in ["--force", "-f", "--hard", "-D", "--delete"] {
            let err = run_git_inner(
                "worktree",
                &repo,
                &["remove".to_string(), bad.to_string()],
                DEFAULT_GIT_TIMEOUT,
            )
            .expect_err("危险 flag 必须被拒绝");
            assert!(err.to_string().contains("拒绝"), "错误应说明拒绝：{bad}");
        }
        // 非白名单子命令
        assert!(run_git_inner("clean", &repo, &[], DEFAULT_GIT_TIMEOUT).is_err());

        let _ = std::fs::remove_dir_all(&repo);
    }

    #[test]
    fn test_worktree_remove_main_rejected() {
        if !git_available() {
            return;
        }
        let repo = temp_repo("main");
        let mgr = WorktreeManager::new(repo.clone(), "sess").unwrap();
        let err = mgr
            .remove_path(&repo)
            .expect_err("主工作树必须拒绝移除")
            .to_string();
        assert!(err.contains("主工作树"), "错误应指明主工作树：{err}");
        let _ = std::fs::remove_dir_all(&repo);
    }
}
