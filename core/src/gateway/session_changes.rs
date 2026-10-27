//! `sessions.fileChanges`：把「这一会话改过哪些文件」连同可渲染的 diff 交给前端。
//!
//! ## 数据从哪里来
//!
//! `tools::file_change` 的文件变更台账已经逐条记录了 `(path, kind, before, after, tool)`。
//! 此前它只被交付回执用来数行数，**正文没有出口**——于是前端只能显示"已编辑 3 个文件"，
//! 用户点不开看改了什么（对标竞品的代码评审视图）。
//!
//! ## 为什么 diff 在这里算
//!
//! 台账在 Rust 进程内、正文只在内存里；让前端自己算就得先把 before/after 全量推过去，
//! 体积是 diff 的数倍。所以在服务端算好 unified diff，前端只做渲染与配色。
//!
//! ## 有界与诚实
//!
//! - 单文件行数超过 [`MAX_DIFF_LINES`] 时退化为「块级 diff」（删完再增），并在
//!   `mode` 字段里如实标注 `block`，不假装是逐行对齐；
//! - 单条 diff 超过 [`MAX_DIFF_CHARS`] 截断并置 `truncated`；
//! - 总量超过 [`MAX_TOTAL_CHARS`] 时按时间倒序保留最近的若干条，并在返回体里
//!   写 `dropped` 条数——绝不静默丢。

use serde_json::{json, Value};

use crate::tools::file_change::{list_file_changes_for, FileChange};

/// 单文件超过这个行数就不做 LCS（O(n²) 会吃掉几百 MB），改块级 diff。
const MAX_DIFF_LINES: usize = 1200;
/// 单条 diff 字符上限
const MAX_DIFF_CHARS: usize = 40_000;
/// 所有 diff 的字符总上限（保护 WS 帧体积）
const MAX_TOTAL_CHARS: usize = 200_000;
/// 最多返回的变更条数
const MAX_CHANGES: usize = 120;

/// 行级 diff（LCS），返回 unified 风格文本；超长时退化为块级。
///
/// 返回 `(diff_text, added, removed, mode)`，mode ∈ `line` / `block`。
fn unified_diff(before: &str, after: &str) -> (String, usize, usize, &'static str) {
    let a: Vec<&str> = before.lines().collect();
    let b: Vec<&str> = after.lines().collect();
    if a.len() + b.len() > MAX_DIFF_LINES {
        // 块级：先全删后全增。诚实标注，不冒充逐行对齐。
        let mut out = String::new();
        for line in &a {
            out.push_str("- ");
            out.push_str(line);
            out.push('\n');
        }
        for line in &b {
            out.push_str("+ ");
            out.push_str(line);
            out.push('\n');
        }
        return (out, b.len(), a.len(), "block");
    }

    let n = a.len();
    let m = b.len();
    let mut dp = vec![vec![0u32; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            dp[i][j] = if a[i] == b[j] {
                dp[i + 1][j + 1] + 1
            } else {
                dp[i + 1][j].max(dp[i][j + 1])
            };
        }
    }
    let mut out = String::new();
    let (mut i, mut j) = (0usize, 0usize);
    let (mut added, mut removed) = (0usize, 0usize);
    while i < n && j < m {
        if a[i] == b[j] {
            out.push_str("  ");
            out.push_str(a[i]);
            out.push('\n');
            i += 1;
            j += 1;
        } else if dp[i + 1][j] >= dp[i][j + 1] {
            out.push_str("- ");
            out.push_str(a[i]);
            out.push('\n');
            removed += 1;
            i += 1;
        } else {
            out.push_str("+ ");
            out.push_str(b[j]);
            out.push('\n');
            added += 1;
            j += 1;
        }
    }
    while i < n {
        out.push_str("- ");
        out.push_str(a[i]);
        out.push('\n');
        removed += 1;
        i += 1;
    }
    while j < m {
        out.push_str("+ ");
        out.push_str(b[j]);
        out.push('\n');
        added += 1;
        j += 1;
    }
    (out, added, removed, "line")
}

fn change_json(c: &FileChange) -> Value {
    let (raw_diff, added, removed, mode) = unified_diff(&c.before, &c.after);
    let truncated = raw_diff.chars().count() > MAX_DIFF_CHARS;
    let diff: String = if truncated {
        raw_diff.chars().take(MAX_DIFF_CHARS).collect()
    } else {
        raw_diff
    };
    json!({
        "path": c.path,
        "kind": match c.kind.as_str() {
            "create" => "create",
            "delete" => "delete",
            _ => "modify",
        },
        "tool": c.tool,
        "added": added,
        "removed": removed,
        "diff": diff,
        "mode": mode,
        "truncated": truncated,
        "ts": c.ts,
    })
}

/// `sessions.fileChanges { session }`
pub fn file_changes(params: &Value) -> Value {
    let session = params
        .get("session")
        .or_else(|| params.get("sessionKey"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let Some(session) = session else {
        return json!({
            "ok": false,
            "code": "INVALID_REQUEST",
            "error": "sessions.fileChanges 需要 session（会话键）",
            "changes": [],
        });
    };

    let all = list_file_changes_for(session, MAX_CHANGES);
    let total = all.len();
    let mut changes: Vec<Value> = Vec::with_capacity(total);
    let mut used_chars = 0usize;
    let mut dropped = 0usize;
    // 台账是追加序（旧→新）；返回给前端按**最新在前**，符合"这一回合改了什么"的读法。
    for c in all.iter().rev() {
        let v = change_json(c);
        let len = v["diff"].as_str().map(str::len).unwrap_or(0);
        if used_chars + len > MAX_TOTAL_CHARS {
            dropped += 1;
            continue;
        }
        used_chars += len;
        changes.push(v);
    }

    if changes.is_empty() {
        let note = if total == 0 {
            "本会话还没有文件改动记录"
        } else {
            "改动记录存在，但总正文超过上限被全部丢弃（请缩小范围后重试）"
        };
        return json!({
            "ok": true,
            "session": session,
            "changes": [],
            "total": total,
            "note": note,
        });
    }

    json!({
        "ok": true,
        "session": session,
        "changes": changes,
        "total": total,
        "dropped": dropped,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unified_diff_counts_added_and_removed() {
        let (text, added, removed, mode) = unified_diff("a\nb\nc\n", "a\nB\nc\nd\n");
        assert_eq!(mode, "line");
        assert_eq!(added, 2, "B 与 d 是新增：\n{text}");
        assert_eq!(removed, 1, "b 被删：\n{text}");
        assert!(text.contains("- b"), "{text}");
        assert!(text.contains("+ B"), "{text}");
        assert!(text.contains("  a"), "未变行应保留上下文：{text}");
    }

    #[test]
    fn create_and_delete_produce_one_sided_diff() {
        let (t1, a1, r1, _) = unified_diff("", "x\ny\n");
        assert_eq!((a1, r1), (2, 0));
        assert!(t1.lines().all(|l| l.starts_with("+ ")));
        let (t2, a2, r2, _) = unified_diff("x\ny\n", "");
        assert_eq!((a2, r2), (0, 2));
        assert!(t2.lines().all(|l| l.starts_with("- ")));
    }

    #[test]
    fn oversized_files_fall_back_to_block_mode_honestly() {
        let big_a: String = (0..MAX_DIFF_LINES).map(|i| format!("a{i}\n")).collect();
        let big_b: String = (0..MAX_DIFF_LINES).map(|i| format!("b{i}\n")).collect();
        let (text, added, removed, mode) = unified_diff(&big_a, &big_b);
        assert_eq!(mode, "block");
        assert_eq!(added, MAX_DIFF_LINES);
        assert_eq!(removed, MAX_DIFF_LINES);
        assert!(text.lines().count() == MAX_DIFF_LINES * 2);
    }

    #[test]
    fn rejects_missing_session_with_explicit_code() {
        let v = file_changes(&json!({}));
        assert_eq!(v["ok"], false);
        assert_eq!(v["code"], "INVALID_REQUEST");
        assert_eq!(v["changes"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn unknown_session_returns_empty_with_note() {
        let v = file_changes(&json!({ "session": "no-such-session-xyz" }));
        assert_eq!(v["ok"], true);
        assert_eq!(v["changes"].as_array().unwrap().len(), 0);
        assert!(v["note"].as_str().unwrap().contains("还没有文件改动"));
    }
}
