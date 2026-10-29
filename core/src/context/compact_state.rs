//! 压缩后的**结构化交接状态**（spec §7.2 第 1 层）。
//!
//! # 设计红线：宁缺毋滥
//!
//! 每个导不出的字段一律 `None` / 空，并在 [`CompactState::source_gaps`] 里写一句
//! **具体**的原因。绝不把「没数据」渲染成「值为空 / 值为 0 / 值为 main」——
//! 那是最难查的一类错（AGENTS.md R57：某一侧的「不知道」被当成另一侧的「不存在」）。
//!
//! 前端据此显示「未知（本引擎无此数据）」，而不是一个会被当成阴性答案的 0。
//!
//! # 与台账锁的关系
//!
//! [`collect_for`] **不碰** `context::session::ledger()`：它唯一的调用现场是
//! `Ledger::compact()`，而那里已经持有同一把锁 —— 再取一次就是自死锁。
//! 会话自己的数据（项目根、快照数）由调用方从 `SessionContext` 传入。

use serde::{Deserialize, Serialize};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

// `GitRunner::run` 是 **trait 方法**：不进作用域就解析不到 `SystemGitRunner::run`，
// 报错是 `E0599: no method named run found for struct SystemGitRunner` —— 看起来像
// "方法不存在"，实际只差这一行 import（2026-10-29 由并行会话引入，整包测试因此编译不过）。
use crate::tools::git::GitRunner;

/// 一条**真跑过**的测试结果。
///
/// 目前没有任何数据源能填它（见 [`collect_for`] 的 `source_gaps`）——
/// 类型先定下来，是为了将来接线时不用改对外契约。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestStatus {
    /// 触发测试的命令（如 `cargo test`）。
    pub command: String,
    /// 进程退出码。
    pub exit_code: i32,
    /// 毫秒时间戳。
    pub ran_at_ms: u64,
}

/// 活动计划（来自计划库）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActivePlan {
    pub plan_id: String,
    pub title: String,
    /// 当前步骤；`None` = 未开始或已收尾。
    pub current_step: Option<String>,
}

/// 最近一次用户裁决。
///
/// **只导出参数指纹，不导出明文参数**（同 AGENTS.md「参数指纹」纪律）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserConfirm {
    pub tool: String,
    /// `guard::trust::args_fingerprint` 的结果。
    pub args_fingerprint: String,
    pub at_ms: u64,
}

/// 压缩交接状态。旧台账缺此键 ⇒ `None`（见 `SessionContext::compact_state`）。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CompactState {
    /// 当前分支；非 git 仓库 / 未绑定项目 ⇒ `None`。
    #[serde(default)]
    pub branch: Option<String>,
    /// 最近提交短 sha。
    #[serde(default)]
    pub last_commit: Option<String>,
    /// `git status --porcelain` 里的未提交路径（最多 50 条）。
    #[serde(default)]
    pub dirty_files: Vec<String>,
    #[serde(default)]
    pub test_status: Option<TestStatus>,
    #[serde(default)]
    pub active_plan: Option<ActivePlan>,
    #[serde(default)]
    pub last_user_confirm: Option<UserConfirm>,
    /// 本会话改过的文件（最近优先，去重）。
    #[serde(default)]
    pub open_files: Vec<String>,
    #[serde(default)]
    pub unresolved_questions: Vec<String>,
    /// 可用预压缩快照数（真实存在、可回滚的量）。
    #[serde(default)]
    pub snapshot_count: u32,
    /// 生成本结构的时间（毫秒）。
    #[serde(default)]
    pub generated_at_ms: u64,
    /// 「哪些字段为什么是空的」——把缺失变成可显示的状态，而不是 0。
    #[serde(default)]
    pub source_gaps: Vec<String>,
}

/// 采集一份交接状态。
///
/// 数据源：
/// - `branch` / `last_commit` / `dirty_files`：`git rev-parse` + `git status`
///   （**只走 `tools::git` 既有白名单**，不新增动词；`DENIED_FLAGS` 已挡破坏性旗标）；
/// - `open_files`：`tools::file_change::list_file_changes_for`（内存 + 磁盘历史合并）；
/// - `snapshot_count`：调用方从 `SessionContext` 传入；
/// - 其余三栏（`test_status` / `active_plan` / `last_user_confirm` /
///   `unresolved_questions`）本引擎**没有结构化数据源**，如实留空并点名原因。
pub fn collect_for(session: &str, project_path: Option<&str>, snapshot_count: u32) -> CompactState {
    let mut s = CompactState {
        snapshot_count,
        generated_at_ms: now_ms(),
        ..Default::default()
    };

    match project_path.map(str::trim).filter(|p| !p.is_empty()) {
        Some(dir) => collect_git(&mut s, Path::new(dir)),
        None => s
            .source_gaps
            .push("branch/last_commit/dirty_files: 会话未绑定项目目录".to_string()),
    }

    s.open_files = recent_files(session);
    if s.open_files.is_empty() {
        s.source_gaps
            .push("open_files: 本会话还没有文件变更记录".to_string());
    }

    // 本引擎不使用 git stash：回滚走预压缩快照（tar + sha256），
    // 所以 spec 里的 `stash` 一栏改由 `snapshot_count` 承载。
    s.source_gaps
        .push("stash: 本引擎不使用 git stash（回滚走预压缩快照）".to_string());
    s.source_gaps.push(
        "test_status: 本引擎不记录结构化的测试结果（只有 terminal 的 exitCode，未归属到会话）"
            .to_string(),
    );
    s.source_gaps.push(
        "active_plan: 计划库（plans.json）不按会话归属，无法判定「本会话的活动计划」".to_string(),
    );
    s.source_gaps.push(
        "last_user_confirm: 审批裁决不落持久化账本（只在当次链路内存中），无法导出".to_string(),
    );
    s.source_gaps
        .push("unresolved_questions: 确定性摘要不产生待决问题".to_string());
    s
}

fn collect_git(s: &mut CompactState, dir: &Path) {
    let runner = crate::tools::git::SystemGitRunner;
    match runner.run(
        "rev-parse",
        dir,
        &["--abbrev-ref".to_string(), "HEAD".to_string()],
    ) {
        Ok(out) if out.exit_code == 0 => {
            let branch = out.stdout.trim();
            // 分离头指针时 rev-parse 回 "HEAD" —— 那不是分支名，如实留空。
            if !branch.is_empty() && branch != "HEAD" {
                s.branch = Some(branch.to_string());
            }
        }
        _ => s
            .source_gaps
            .push("branch: 不是 git 仓库，或 git 不可用".to_string()),
    }
    match runner.run("rev-parse", dir, &["--short".to_string(), "HEAD".to_string()]) {
        Ok(out) if out.exit_code == 0 && !out.stdout.trim().is_empty() => {
            s.last_commit = Some(out.stdout.trim().to_string());
        }
        _ => s
            .source_gaps
            .push("last_commit: 仓库还没有提交，或无法读取 HEAD".to_string()),
    }
    match runner.run("status", dir, &["--porcelain".to_string()]) {
        Ok(out) if out.exit_code == 0 => {
            s.dirty_files = out
                .stdout
                .lines()
                .filter(|l| l.len() > 3)
                .map(|l| l[3..].trim().to_string())
                .filter(|p| !p.is_empty())
                .take(50)
                .collect();
        }
        _ => s
            .source_gaps
            .push("dirty_files: 无法读取 git status".to_string()),
    }
}

fn recent_files(session: &str) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    crate::tools::file_change::list_file_changes_for(session, 50)
        .into_iter()
        .map(|c| c.path)
        .filter(|p| seen.insert(p.clone()))
        .take(20)
        .collect()
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_field_with_no_source_is_null_and_named_in_source_gaps() {
        // 没有跑过测试命令的会话：test_status 必须是 None，且必须**点名**为什么。
        let s = collect_for("mgt-t4-none", None, 0);
        assert!(s.test_status.is_none());
        assert!(
            s.source_gaps.iter().any(|g| g.contains("test_status")),
            "空白必须可解释：{:?}",
            s.source_gaps
        );
        assert!(s.active_plan.is_none());
        assert!(s.last_user_confirm.is_none());
        assert!(s.generated_at_ms > 0, "生成时间必须是真的");
    }

    #[test]
    fn collect_never_fabricates_a_branch_from_a_non_repo_directory() {
        // 未绑定项目 / 非 git 目录 ⇒ branch 是 None，不是 "" 也不是 "main"。
        let s = collect_for("mgt-t4-nonrepo", None, 0);
        assert!(s.branch.is_none(), "不得凭空给 main：{:?}", s.branch);
        assert!(s.last_commit.is_none());
        assert!(s.dirty_files.is_empty());
        assert!(
            s.source_gaps.iter().any(|g| g.contains("branch")),
            "缺 branch 必须点名：{:?}",
            s.source_gaps
        );
    }

    #[test]
    fn a_non_repo_directory_yields_gaps_instead_of_an_empty_success() {
        // 绑定了目录但不是 git 仓库：git 会失败 ⇒ 每栏各自点名，不能静默为空。
        let dir = std::env::temp_dir().join(format!("mgt-t4-plain-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let s = collect_for("mgt-t4-plain", dir.to_str(), 3);
        assert_eq!(s.snapshot_count, 3, "快照数由台账原样透传");
        assert!(s.branch.is_none());
        assert!(
            s.source_gaps.iter().any(|g| g.contains("branch")),
            "非 git 目录必须点名 branch 缺失：{:?}",
            s.source_gaps
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn compact_state_roundtrips_through_json_with_a_missing_field() {
        // 旧台账 JSON 没有 compact_state 键 —— #[serde(default)] 必须让它照样解析。
        let s: CompactState = serde_json::from_str("{}").expect("缺字段的旧载荷必须解析成功");
        assert!(s.branch.is_none() && s.source_gaps.is_empty());
        assert_eq!(s.snapshot_count, 0);
        assert_eq!(s.generated_at_ms, 0);
        // 再序列化回 JSON 也必须可行（前端契约）。
        let v = serde_json::to_value(&s).expect("必须可序列化");
        assert!(v.get("source_gaps").is_some());
    }
}
