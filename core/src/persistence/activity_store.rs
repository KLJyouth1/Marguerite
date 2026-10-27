//! 活动台账落盘（文件改动 + 团队运行快照）。
//!
//! ## 为什么必须落盘
//!
//! 此前两份台账都只在进程内存里：
//! - `tools::file_change` 的变更记录（含 before/after 正文）—— **重启即失**，于是
//!   「代码评审视图」和交付回执在重启后全部为空，历史会话的改动无从回看；
//! - 团队成员的实时动向 —— 页面晚一步打开就什么都没有。
//!
//! 这直接违背用户对「可回溯、可审计」的期待：明明改过文件，重启后系统说自己没改过。
//!
//! ## 为什么用 JSONL 而不是 SQLite
//!
//! - 写入是**纯追加**，没有事务/关联查询需求（按会话过滤在 Rust 侧做就够了）；
//! - 崩溃安全：追加写只会丢最后一行，不会损坏整库；SQLite 需要 WAL + 恢复逻辑；
//! - 与既有 `harness.db`（编排台账）职责不重叠：那边记 run/item 事件，这边记
//!   「文件正文差分」与「成员状态快照」，体量与访问模式都不同。
//!
//! ## 有界
//!
//! - 文件改动：超过 [`MAX_BYTES`] 时按行裁剪并原子重写（保留最近
//!   [`MAX_LINES`] 行），**不会无限增长**；
//! - 团队快照：每个会话一个文件，只保留最新一份（快照语义，天然有界）；
//! - 所有 IO 失败都返回 `Result`，调用方决定是否忽略——绝不 panic 拖垮主流程。

use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::persistence::config::Config;

/// 单条文件改动记录的落盘形态（与内存 `FileChange` 一一对应）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileChangeRow {
    pub session: String,
    pub path: String,
    pub kind: String,
    pub tool: String,
    pub ts: i64,
    #[serde(default)]
    pub before: String,
    #[serde(default)]
    pub after: String,
}

/// 台账目录上限：8MB（超了就裁剪，避免长跑 daemon 把磁盘写满）。
const MAX_BYTES: u64 = 8 * 1024 * 1024;
/// 裁剪后保留的行数
const MAX_LINES: usize = 2000;

fn root() -> PathBuf {
    Config::data_dir().join("activity")
}

fn changes_path(root: &Path) -> PathBuf {
    root.join("file_changes.jsonl")
}

fn members_dir(root: &Path) -> PathBuf {
    root.join("team_members")
}

/// 会话键 → 文件名（只保留安全字符，避免路径穿越；空会话用 `_default`）。
///
/// 扩展名由我们自己拼（`.json`），所以点号一律不当安全字符：否则
/// `..%2f..` 这类输入会产出含 `..` 的文件名——虽然去掉了分隔符后无法穿越，
/// 但"文件名里不许出现 .."是一条更好验证的不变式。
fn safe_session_file(session: &str) -> String {
    let mut out = String::with_capacity(session.len());
    for ch in session.chars() {
        if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
            out.push(ch);
        } else {
            out.push('_');
        }
    }
    if out.is_empty() {
        out.push_str("_default");
    }
    // 文件名长度上限（Windows 255）：保留尾部，尾部含会话唯一 id 的部分更可能不同
    if out.len() > 120 {
        out = out[out.len() - 120..].to_string();
    }
    out
}

/// 追加一条文件改动（无界增长由裁剪兜底）。
pub(crate) fn append_file_change(root: &Path, row: &FileChangeRow) -> std::io::Result<()> {
    std::fs::create_dir_all(root)?;
    let path = changes_path(root);
    let line = serde_json::to_string(row)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    {
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)?;
        f.write_all(line.as_bytes())?;
        f.write_all(b"\n")?;
    }
    if let Ok(meta) = std::fs::metadata(&path) {
        if meta.len() > MAX_BYTES {
            let _ = trim_changes(root);
        }
    }
    Ok(())
}

/// 裁剪到最近 [`MAX_LINES`] 行，并以「写临时文件 + rename」原子替换。
fn trim_changes(root: &Path) -> std::io::Result<()> {
    let path = changes_path(root);
    let raw = std::fs::read_to_string(&path)?;
    let lines: Vec<&str> = raw.lines().collect();
    let keep_from = lines.len().saturating_sub(MAX_LINES);
    let mut body = String::with_capacity(raw.len() / 2);
    for line in &lines[keep_from..] {
        body.push_str(line);
        body.push('\n');
    }
    let tmp = path.with_extension("jsonl.tmp");
    std::fs::write(&tmp, body)?;
    std::fs::rename(&tmp, &path)?;
    Ok(())
}

/// 读取某会话的改动（最新在前）。文件不存在 = 空集，不是错误。
pub(crate) fn read_file_changes(
    root: &Path,
    session: &str,
    limit: usize,
) -> Vec<FileChangeRow> {
    if session.trim().is_empty() || limit == 0 {
        return Vec::new();
    }
    let Ok(raw) = std::fs::read_to_string(changes_path(root)) else {
        return Vec::new();
    };
    let mut out: Vec<FileChangeRow> = Vec::new();
    for line in raw.lines() {
        let Ok(row) = serde_json::from_str::<FileChangeRow>(line) else {
            continue; // 损坏行跳过，不因一行坏数据丢整份历史
        };
        if row.session == session {
            out.push(row);
        }
    }
    out.reverse();
    out.truncate(limit);
    out
}

/// 保存某会话的团队快照（原子写：先 tmp 再 rename，避免读到半个文件）。
pub(crate) fn save_team_snapshot(root: &Path, session: &str, payload: &serde_json::Value) -> std::io::Result<()> {
    let dir = members_dir(root);
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!("{}.json", safe_session_file(session)));
    let body = serde_json::to_vec_pretty(payload)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, body)?;
    std::fs::rename(&tmp, &path)?;
    Ok(())
}

/// 读取某会话的团队快照（不存在返回 None）。
pub(crate) fn load_team_snapshot(root: &Path, session: &str) -> Option<serde_json::Value> {
    let path = members_dir(root).join(format!("{}.json", safe_session_file(session)));
    let raw = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&raw).ok()
}

/// 最近一次有快照的会话（用于「没传 session 也想看正在跑/刚跑完的团队」）。
pub(crate) fn latest_team_snapshot(root: &Path) -> Option<(String, serde_json::Value)> {
    let dir = members_dir(root);
    let rd = std::fs::read_dir(&dir).ok()?;
    let mut best: Option<(std::time::SystemTime, PathBuf)> = None;
    for entry in rd.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let Ok(meta) = entry.metadata() else { continue };
        let Ok(modified) = meta.modified() else { continue };
        if best.as_ref().map(|(t, _)| modified > *t).unwrap_or(true) {
            best = Some((modified, path));
        }
    }
    let (_, path) = best?;
    let raw = std::fs::read_to_string(&path).ok()?;
    let value: serde_json::Value = serde_json::from_str(&raw).ok()?;
    let session = value
        .get("session")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    Some((session, value))
}

// ── 生产入口（测试态不写真实用户目录） ─────────────────────────────────

/// 落盘一条文件改动。
///
/// `cfg!(test)` 时**不写**：单测会大量触发文件工具，若写到真实
/// `data_dir()` 会污染用户台账（仓库既有台账同此约定）。落盘本身的正确性
/// 由本模块的 `*_from/append_*` 纯函数（传入临时目录）覆盖。
pub fn persist_file_change(row: &FileChangeRow) {
    if cfg!(test) {
        return;
    }
    let _ = append_file_change(&root(), row);
}

/// 从磁盘读某会话的改动（测试态读到的通常是空集，安全）。
pub fn load_file_changes(session: &str, limit: usize) -> Vec<FileChangeRow> {
    if cfg!(test) {
        return Vec::new();
    }
    read_file_changes(&root(), session, limit)
}

/// 保存团队快照（测试态不写）。
pub fn persist_team_snapshot(session: &str, payload: &serde_json::Value) {
    if cfg!(test) {
        return;
    }
    let _ = save_team_snapshot(&root(), session, payload);
}

/// 读取团队快照：指定会话优先，其次取最近更新的那份（测试态返回 None）。
pub fn load_team_snapshot_any(session: Option<&str>) -> Option<serde_json::Value> {
    if cfg!(test) {
        return None;
    }
    let root = root();
    match session.map(str::trim).filter(|s| !s.is_empty()) {
        Some(s) => load_team_snapshot(&root, s),
        None => latest_team_snapshot(&root).map(|(_, v)| v),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_root(tag: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!(
            "mgt_activity_{}_{}",
            tag,
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&p);
        p
    }

    fn row(session: &str, path: &str, ts: i64) -> FileChangeRow {
        FileChangeRow {
            session: session.to_string(),
            path: path.to_string(),
            kind: "modify".to_string(),
            tool: "edit_file".to_string(),
            ts,
            before: "a\n".to_string(),
            after: "b\n".to_string(),
        }
    }

    #[test]
    fn file_change_roundtrip_filters_by_session_and_orders_newest_first() {
        let root = tmp_root("fc");
        append_file_change(&root, &row("s1", "a.rs", 1)).unwrap();
        append_file_change(&root, &row("s2", "b.rs", 2)).unwrap();
        append_file_change(&root, &row("s1", "c.rs", 3)).unwrap();

        let s1 = read_file_changes(&root, "s1", 10);
        assert_eq!(s1.len(), 2);
        assert_eq!(s1[0].path, "c.rs", "最新在前");
        assert_eq!(s1[1].path, "a.rs");
        assert_eq!(s1[0].before, "a\n", "before/after 正文必须无损");

        let s2 = read_file_changes(&root, "s2", 10);
        assert_eq!(s2.len(), 1);
        assert_eq!(s2[0].path, "b.rs");

        assert!(read_file_changes(&root, "nope", 10).is_empty());
        assert!(read_file_changes(&root, "", 10).is_empty(), "空会话返回空集");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn corrupted_line_does_not_break_the_rest() {
        let root = tmp_root("bad");
        append_file_change(&root, &row("s", "ok1.rs", 1)).unwrap();
        // 手工插入一行坏数据（模拟写盘中断）
        let path = changes_path(&root);
        let mut f = std::fs::OpenOptions::new().append(true).open(&path).unwrap();
        f.write_all(b"{not json\n").unwrap();
        drop(f);
        append_file_change(&root, &row("s", "ok2.rs", 2)).unwrap();

        let got = read_file_changes(&root, "s", 10);
        assert_eq!(got.len(), 2, "坏行应被跳过而不是让整份历史失效：{got:?}");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn oversized_ledger_is_trimmed_and_stays_bounded() {
        let root = tmp_root("trim");
        // 造 2100 行（超过 MAX_LINES 2000）
        let big = "x".repeat(100);
        for i in 0..2100 {
            let mut r = row("s", &format!("f{i}.txt"), i as i64);
            r.before = big.clone();
            append_file_change(&root, &r).unwrap();
        }
        let _ = trim_changes(&root);
        let raw = std::fs::read_to_string(changes_path(&root)).unwrap();
        assert!(
            raw.lines().count() <= MAX_LINES,
            "裁剪后行数必须 ≤ {MAX_LINES}，实际 {}",
            raw.lines().count()
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn team_snapshot_roundtrip_and_latest_lookup() {
        let root = tmp_root("team");
        let payload = serde_json::json!({
            "session": "sess-a",
            "members": [{ "id": "m1", "status": "running" }]
        });
        save_team_snapshot(&root, "sess-a", &payload).unwrap();
        let got = load_team_snapshot(&root, "sess-a").unwrap();
        assert_eq!(got["members"][0]["id"], "m1");

        let (session, value) = latest_team_snapshot(&root).unwrap();
        assert_eq!(session, "sess-a");
        assert_eq!(value["members"].as_array().unwrap().len(), 1);

        assert!(load_team_snapshot(&root, "missing").is_none());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn session_file_names_are_path_safe_and_bounded() {
        let long = "a".repeat(400);
        for s in [
            "agent:main:webchat:abc",
            "../../etc/passwd",
            "C:\\Windows\\System32",
            "",
            &long,
        ] {
            let name = safe_session_file(s);
            assert!(!name.contains('/'), "不得含斜杠：{name}");
            assert!(!name.contains('\\'), "不得含反斜杠：{name}");
            assert!(!name.contains(".."), "不得含 ..：{name}");
            assert!(name.len() <= 120, "长度必须有界：{name}");
        }
    }
}
