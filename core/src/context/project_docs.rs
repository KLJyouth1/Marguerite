//! 强制项目记忆 / 进度文档。
//!
//! # 规则（用户新增）
//!
//! - **任务执行前**必须读取主工作区根下的 `.marguerite/project-memory.md` 与
//!   `.marguerite/progress.md`；
//! - **任务完成后**必须把本轮结论追加进进度文档。
//!
//! 读取侧由 [`read_bounded`] / [`read_block`] 生成一段可注入的动态提示；写入侧由
//! [`append_progress`] 追加单行记录（原子写 + 超限裁剪）。多根工作区只写主根；
//! 读取时附加根若有同名文件，一并纳入并标注来源，避免「附加根里写下的约定被静默忽略」。

use std::io;
use std::path::{Path, PathBuf};

/// 单文件读取上限：8 KiB（超出截断并标注）。
pub const MAX_FILE_BYTES: usize = 8 * 1024;
/// 进度文件轮转阈值：256 KiB。
pub const MAX_PROGRESS_BYTES: u64 = 256 * 1024;
/// 裁剪后保留的最近行数。
pub const PROGRESS_KEEP_LINES: usize = 200;
/// 单条进度摘要上限（字符数）。
pub const MAX_SUMMARY_CHARS: usize = 400;

const MEMORY_FILE: &str = "project-memory.md";
const PROGRESS_FILE: &str = "progress.md";

fn doc_path(root: &Path, name: &str) -> PathBuf {
    root.join(".marguerite").join(name)
}

/// 按字节截断到 UTF-8 字符边界，并标注 `[已截断]`。
fn truncate_utf8(raw: &str, limit: usize) -> (String, bool) {
    if raw.len() <= limit {
        return (raw.to_string(), false);
    }
    let mut end = limit;
    while end > 0 && !raw.is_char_boundary(end) {
        end -= 1;
    }
    (format!("{}\n[已截断]", &raw[..end]), true)
}

/// 读取单个文件；不存在或不可读返回 `None`（不报错、不阻断任务）。
fn read_capped(path: &Path) -> Option<(String, bool)> {
    let raw = std::fs::read_to_string(path).ok()?;
    if raw.trim().is_empty() {
        return None;
    }
    Some(truncate_utf8(&raw, MAX_FILE_BYTES))
}

/// 读取项目记忆与进度（每份 ≤ 8 KiB）。
///
/// 返回 `(注入块, 文件清单)`：文件清单每项形如
/// `"/abs/path (truncated)"` / `"/abs/path"`，用于告知模型「读了哪些文件、是否被截断」。
/// 两份文件都不存在时返回 `(String::new(), Vec::new())`。
pub fn read_bounded(roots: &[PathBuf]) -> (String, Vec<String>) {
    let Some(primary) = roots.first() else {
        return (String::new(), Vec::new());
    };
    let mut sections: Vec<String> = Vec::new();
    let mut files: Vec<String> = Vec::new();
    let mut push = |path: &Path, label: String, files: &mut Vec<String>| {
        if let Some((body, truncated)) = read_capped(path) {
            sections.push(format!("{label}\n{body}"));
            files.push(if truncated {
                format!("{} (truncated)", path.display())
            } else {
                path.display().to_string()
            });
        }
    };
    for name in [MEMORY_FILE, PROGRESS_FILE] {
        push(
            &doc_path(primary, name),
            format!("### .marguerite/{name}"),
            &mut files,
        );
    }
    for root in roots.iter().skip(1) {
        for name in [MEMORY_FILE, PROGRESS_FILE] {
            push(
                &doc_path(root, name),
                format!("### [附加根] {}/.marguerite/{name}", root.display()),
                &mut files,
            );
        }
    }
    if sections.is_empty() {
        return (String::new(), Vec::new());
    }
    let block = format!(
        "【项目记忆与进度（强制）】\n\
         本次任务必须遵守项目记忆中的约定与进度，不要重复已完成工作。\n\
         若与你的计划冲突，以项目记忆为准；完成后必须调用项目进度文档写回（由系统自动完成）。\n\n{}",
        sections.join("\n\n")
    );
    (block, files)
}

/// 拼接项目记忆与进度的注入块（主根 + 附加根）。
///
/// 返回 `None` 表示所有根下都没有这两个文件；文件清单见 [`read_bounded`]。
pub fn read_block(roots: &[PathBuf]) -> Option<String> {
    let (block, _) = read_bounded(roots);
    if block.is_empty() {
        None
    } else {
        Some(block)
    }
}

/// 项目记忆骨架：三节固定结构，内容如实写「待补充」（不编造结论）。
const MEMORY_SKELETON: &str = "# 项目记忆\n\n\
> 由 Marguerite 在首轮任务结束后创建。请把稳定的项目约定、已知缺陷与教训补在这里。\n\n\
## 项目约定\n\n待补充\n\n\
## 已知缺陷\n\n待补充\n\n\
## 教训\n\n待补充\n";

/// 若 `project-memory.md` 不存在则创建骨架（已存在时不动）。
///
/// 返回 `true` 表示本次新建。失败只返回 `io::Error`，由调用方记录日志。
pub fn ensure_memory_skeleton(roots: &[PathBuf]) -> io::Result<bool> {
    let Some(primary) = roots.first() else {
        return Ok(false);
    };
    let dir = primary.join(".marguerite");
    let path = dir.join(MEMORY_FILE);
    if path.is_file() {
        return Ok(false);
    }
    std::fs::create_dir_all(&dir)?;
    let tmp = dir.join("project-memory.md.tmp");
    std::fs::write(&tmp, MEMORY_SKELETON)?;
    std::fs::rename(&tmp, &path)?;
    Ok(true)
}

/// 单行化：换行/回车折叠为空格，并限制到 [`MAX_SUMMARY_CHARS`] 字符。
fn single_line(text: &str) -> String {
    let flat = text
        .replace(['\r', '\n'], " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if flat.chars().count() <= MAX_SUMMARY_CHARS {
        return flat;
    }
    let mut out: String = flat.chars().take(MAX_SUMMARY_CHARS - 1).collect();
    out.push('…');
    out
}

/// 追加一条进度记录：`- [YYYY-MM-DD HH:MM] <session>/<agent>：<summary>`。
///
/// - 只写主根；
/// - `.marguerite/` 不存在时自动创建并写表头；
/// - 文件 > 256 KiB 时按「保留最近 200 行」裁剪（原子写）；
/// - 读旧 + 写 `.tmp` + rename，避免半截文件。
pub fn append_progress(
    roots: &[PathBuf],
    session: &str,
    agent: &str,
    summary: &str,
) -> io::Result<()> {
    let Some(primary) = roots.first() else {
        return Ok(());
    };
    let dir = primary.join(".marguerite");
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(PROGRESS_FILE);

    let (mut existing, created) = match std::fs::read_to_string(&path) {
        Ok(text) => (text, false),
        Err(e) if e.kind() == io::ErrorKind::NotFound => (String::new(), true),
        Err(e) => return Err(e),
    };
    // 超限裁剪：保留最近 200 行（不是整段丢弃/轮转，历史仍可读最近部分）。
    if existing.len() as u64 > MAX_PROGRESS_BYTES {
        let lines: Vec<&str> = existing.lines().collect();
        let start = lines.len().saturating_sub(PROGRESS_KEEP_LINES);
        existing = lines[start..].join("\n");
        if !existing.is_empty() {
            existing.push('\n');
        }
    }

    let when = chrono::Local::now().format("%Y-%m-%d %H:%M");
    let session = single_line(session);
    let agent = single_line(agent);
    let summary = single_line(summary);
    let line = format!(
        "- [{when}] {}/{agent}：{summary}\n",
        if session.is_empty() { "unknown" } else { &session }
    );
    let mut out = String::new();
    if created {
        out.push_str("# 项目进度\n\n");
    }
    out.push_str(&existing);
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
    out.push_str(&line);

    let tmp = dir.join("progress.md.tmp");
    std::fs::write(&tmp, out)?;
    std::fs::rename(&tmp, &path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn temp_root(tag: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!(
            "mgt_pdocs_{}_{}_{}",
            std::process::id(),
            tag,
            uuid::Uuid::new_v4()
        ));
        let _ = fs::remove_dir_all(&p);
        fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn read_block_is_none_without_docs() {
        let root = temp_root("empty");
        assert!(read_block(&[]).is_none());
        assert!(read_block(std::slice::from_ref(&root)).is_none());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn read_block_includes_primary_and_extra_roots_with_headers() {
        let root = temp_root("primary");
        let extra = temp_root("extra");
        fs::create_dir_all(root.join(".marguerite")).unwrap();
        fs::write(root.join(".marguerite").join(MEMORY_FILE), "约定：只监听回环。").unwrap();
        fs::write(root.join(".marguerite").join(PROGRESS_FILE), "- 已完成 A\n").unwrap();
        fs::create_dir_all(extra.join(".marguerite")).unwrap();
        fs::write(extra.join(".marguerite").join(MEMORY_FILE), "附加根约定：使用 Rust。").unwrap();

        let block = read_block(&[root.clone(), extra.clone()]).expect("应有注入块");
        assert!(block.contains("### .marguerite/project-memory.md"), "{block}");
        assert!(block.contains("### .marguerite/progress.md"), "{block}");
        assert!(block.contains("约定：只监听回环。"), "{block}");
        assert!(block.contains("- 已完成 A"), "{block}");
        assert!(block.contains("[附加根]"), "{block}");
        assert!(block.contains("附加根约定：使用 Rust。"), "{block}");
        // 强制规则正文（不是只靠后端注入）
        assert!(block.contains("不要重复已完成工作"), "{block}");
        assert!(block.contains("以项目记忆为准"), "{block}");
        assert!(block.contains("完成后必须调用项目进度文档写回"), "{block}");
        // 字节稳定：同一输入两次调用完全一致
        assert_eq!(
            block,
            read_block(&[root.clone(), extra.clone()]).unwrap(),
            "同一轮内容必须字节稳定（KV 前缀稳定性前提）"
        );
        let _ = fs::remove_dir_all(&root);
        let _ = fs::remove_dir_all(&extra);
    }

    #[test]
    fn read_block_truncates_oversized_file_and_marks_it() {
        let root = temp_root("truncate");
        fs::create_dir_all(root.join(".marguerite")).unwrap();
        let big = "中".repeat(MAX_FILE_BYTES); // 三字节/字 → 必然超限
        fs::write(root.join(".marguerite").join(MEMORY_FILE), &big).unwrap();
        let block = read_block(std::slice::from_ref(&root)).expect("应有注入块");
        assert!(block.contains("[已截断]"), "超限必须标注已截断");
        // 截断后正文不超过上限（加标题与说明的开销在外）
        assert!(block.len() < big.len(), "超限文件必须被截断");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn append_progress_creates_dir_and_writes_single_line() {
        let base = temp_root("append");
        let root = base.join("project");
        fs::create_dir_all(&root).unwrap();
        assert!(!root.join(".marguerite").exists());
        append_progress(std::slice::from_ref(&root), "sess-1", "main", "第一行\n第二行").unwrap();
        let text = fs::read_to_string(root.join(".marguerite").join(PROGRESS_FILE)).unwrap();
        assert!(text.starts_with("# 项目进度"), "新文件应写表头: {text}");
        assert!(text.contains("- ["), "格式应为列表项: {text}");
        assert!(text.contains("sess-1/main："), "{text}");
        assert!(text.contains("第一行 第二行"), "摘要必须单行: {text}");
        assert_eq!(
            text.lines().filter(|l| l.starts_with("- [")).count(),
            1,
            "新增文件只有一条记录: {text}"
        );
        // 再追加一条：只增不改
        append_progress(std::slice::from_ref(&root), "sess-1", "main", "第二条").unwrap();
        let text = fs::read_to_string(root.join(".marguerite").join(PROGRESS_FILE)).unwrap();
        assert_eq!(text.lines().filter(|l| l.starts_with("- [")).count(), 2, "{text}");
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn append_progress_caps_summary_at_400_chars() {
        let base = temp_root("cap");
        let root = base.join("project");
        fs::create_dir_all(&root).unwrap();
        let long = "字".repeat(500);
        append_progress(std::slice::from_ref(&root), "s", "a", &long).unwrap();
        let text = fs::read_to_string(root.join(".marguerite").join(PROGRESS_FILE)).unwrap();
        let summary = text.split('：').next_back().unwrap().trim();
        assert!(
            summary.chars().count() <= MAX_SUMMARY_CHARS,
            "摘要应 ≤ {MAX_SUMMARY_CHARS} 字，实际 {}",
            summary.chars().count()
        );
        assert!(summary.ends_with('…'), "截断后应有省略号: {summary}");
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn append_progress_trims_to_recent_200_lines_when_file_exceeds_threshold() {
        let base = temp_root("trim");
        let root = base.join("project");
        let dir = root.join(".marguerite");
        fs::create_dir_all(&dir).unwrap();
        // 300 行、每行足够长 → 必然超过 256 KiB。
        let mut old = String::new();
        for i in 0..300 {
            old.push_str(&format!("- old line {i} {}\n", "y".repeat(1200)));
        }
        assert!(old.len() as u64 > MAX_PROGRESS_BYTES);
        fs::write(dir.join(PROGRESS_FILE), &old).unwrap();

        append_progress(std::slice::from_ref(&root), "sess-2", "agent-1", "裁剪后的新记录").unwrap();
        let fresh = fs::read_to_string(dir.join(PROGRESS_FILE)).unwrap();
        assert!(fresh.len() as u64 <= MAX_PROGRESS_BYTES, "裁剪后必须回到阈值内");
        assert!(fresh.contains("裁剪后的新记录"), "{fresh}");
        assert!(fresh.contains("old line 299"), "最近的行必须保留");
        assert!(!fresh.contains("old line 0 "), "最旧的行应被裁掉");
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn read_bounded_reports_paths_and_truncation() {
        let root = temp_root("bounded");
        let dir = root.join(".marguerite");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join(MEMORY_FILE), "约定：测试项目").unwrap();
        let big = "中".repeat(MAX_FILE_BYTES);
        fs::write(dir.join(PROGRESS_FILE), &big).unwrap();

        let (block, files) = read_bounded(std::slice::from_ref(&root));
        assert!(block.contains("约定：测试项目"), "{block}");
        assert_eq!(files.len(), 2, "两个文件都要出现在清单里: {files:?}");
        assert!(
            files.iter().any(|f| f.ends_with("project-memory.md")),
            "{files:?}"
        );
        assert!(
            files
                .iter()
                .any(|f| f.ends_with("progress.md (truncated)")),
            "被截断的文件必须标注: {files:?}"
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn ensure_memory_skeleton_creates_three_sections_once() {
        let base = temp_root("skeleton");
        let root = base.join("project");
        fs::create_dir_all(&root).unwrap();
        assert!(ensure_memory_skeleton(std::slice::from_ref(&root)).unwrap());
        let text = fs::read_to_string(root.join(".marguerite").join(MEMORY_FILE)).unwrap();
        for section in ["## 项目约定", "## 已知缺陷", "## 教训"] {
            assert!(text.contains(section), "骨架缺少 {section}: {text}");
        }
        assert!(text.contains("待补充"), "骨架不得编造结论: {text}");
        // 已存在时不覆盖
        fs::write(root.join(".marguerite").join(MEMORY_FILE), "用户已写内容").unwrap();
        assert!(!ensure_memory_skeleton(std::slice::from_ref(&root)).unwrap());
        let text = fs::read_to_string(root.join(".marguerite").join(MEMORY_FILE)).unwrap();
        assert_eq!(text, "用户已写内容");
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn append_progress_without_roots_is_noop() {
        assert!(append_progress(&[], "s", "a", "x").is_ok());
    }

    #[test]
    fn single_line_collapses_whitespace() {
        assert_eq!(single_line("a\nb\r\n c"), "a b c");
    }
}
