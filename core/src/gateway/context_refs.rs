//! `@` 上下文引用：搜索可引用项，并把用户选中的引用渲染进本轮输入。
//!
//! 三类来源：
//! - 工作区（文件 / 目录）——按会话绑定的项目根解析，读文件时做**边界校验**，
//!   任何逃出项目根的真实路径都拒绝（防止 `../../` 读出仓库外内容）；
//! - 规则（`rules.list` 的本地 Markdown）——整条正文进材料；
//! - 历史会话——只取摘要 + 最近几轮截断文本，不整库搬运。
//!
//! 渲染结果拼在**用户消息尾部**（而不是塞 system）：历史/导出/复盘都能看到
//! 「这一轮到底引用了什么」，模型也无需额外约定即可读到材料。
//! 总量有硬上限（[`MAX_BLOCK_CHARS`]），超出按引用顺序截断并留标记。

use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use crate::persistence::config::Config;

/// 单文件读取上限（字符）。
const MAX_FILE_CHARS: usize = 8_000;
/// 单个目录列举上限。
const MAX_DIR_ENTRIES: usize = 200;
/// 历史会话引用上限（字符）。
const MAX_SESSION_CHARS: usize = 3_000;
/// 整块引用材料上限（字符）。
const MAX_BLOCK_CHARS: usize = 16_000;
/// 搜索时最多访问的目录项（防止在巨型仓库里卡住）。
const MAX_WALK_ENTRIES: usize = 6_000;
/// 搜索深度上限（root = 0）。
const MAX_WALK_DEPTH: usize = 4;

const SKIP_DIRS: &[&str] = &[
    "node_modules",
    "target",
    ".git",
    "dist",
    ".next",
    "venv",
    ".venv",
    "__pycache__",
    ".idea",
    ".vscode",
    "build",
];

fn trim_chars(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut out: String = text.chars().take(max).collect();
    out.push_str("\n…（已截断）");
    out
}

/// 会话绑定的项目根（没有则不提供文件类引用）。
fn workspace_root(session: &str) -> Option<PathBuf> {
    crate::gateway::chat::resolve_session_project(session)
        .map(PathBuf::from)
        .filter(|p| p.is_dir())
        .and_then(|p| p.canonicalize().ok())
}

/// 校验候选路径确实落在项目根内（符号链接/`..` 都会被 canonicalize 暴露）。
fn inside_root(root: &Path, candidate: &Path) -> Option<PathBuf> {
    let real = candidate.canonicalize().ok()?;
    real.starts_with(root).then_some(real)
}

fn rel(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

/// 遍历命中的一项：(展示名, 绝对路径)。
type Hit = (String, PathBuf);

/// 广度优先遍历，返回 (文件, 目录) 候选；命中查询即收集，最多 `limit` 条/类。
fn walk(root: &Path, query: &str, limit: usize) -> (Vec<Hit>, Vec<Hit>) {
    let needle = query.trim().to_lowercase();
    let mut files = Vec::new();
    let mut dirs = Vec::new();
    let mut queue: Vec<(PathBuf, usize)> = vec![(root.to_path_buf(), 0)];
    let mut visited = 0usize;
    while let Some((dir, depth)) = queue.pop() {
        if depth > MAX_WALK_DEPTH || visited > MAX_WALK_ENTRIES {
            break;
        }
        let Ok(read) = std::fs::read_dir(&dir) else {
            continue;
        };
        let mut entries: Vec<_> = read.filter_map(Result::ok).collect();
        entries.sort_by_key(|e| e.file_name());
        for entry in entries {
            visited += 1;
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            let is_dir = path.is_dir();
            if is_dir {
                if SKIP_DIRS.contains(&name.as_str()) {
                    continue;
                }
                if (needle.is_empty() || name.to_lowercase().contains(&needle)) && dirs.len() < limit {
                    dirs.push((name.clone(), path.clone()));
                }
                queue.push((path, depth + 1));
            } else if files.len() < limit
                && (needle.is_empty() || name.to_lowercase().contains(&needle))
            {
                files.push((name, path));
            }
            if files.len() >= limit && dirs.len() >= limit {
                return (files, dirs);
            }
        }
    }
    (files, dirs)
}

fn file_item(root: &Path, name: &str, path: &Path) -> Value {
    let size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    json!({
        "kind": "file",
        "id": rel(root, path),
        "label": name,
        "subtitle": rel(root, path),
        "path": path.to_string_lossy(),
        "size": size,
    })
}

/// `context.references`：按查询返回分组候选（文件/目录/规则/历史会话）。
pub fn references(params: &Value) -> Value {
    let session = params
        .get("sessionKey")
        .or_else(|| params.get("session"))
        .and_then(Value::as_str)
        .unwrap_or("");
    let query = params.get("q").and_then(Value::as_str).unwrap_or("");
    let limit = params
        .get("limit")
        .and_then(Value::as_u64)
        .map(|n| n.clamp(1, 30) as usize)
        .unwrap_or(8);

    let mut groups: Vec<Value> = Vec::new();

    // 优先用请求显式带的工作区（新会话还没绑定时也能搜到文件），再回退会话绑定。
    let root = params
        .get("workspaceId")
        .and_then(Value::as_str)
        .and_then(crate::gateway::workspaces::workspace_path_for)
        .map(PathBuf::from)
        .filter(|p| p.is_dir())
        .and_then(|p| p.canonicalize().ok())
        .or_else(|| workspace_root(session));

    if let Some(root) = root.as_ref() {
        let (files, dirs) = walk(root, query, limit);
        if !files.is_empty() {
            groups.push(json!({
                "id": "file",
                "label": "文件",
                "items": files.iter().map(|(n, p)| file_item(root, n, p)).collect::<Vec<_>>(),
            }));
        }
        if !dirs.is_empty() {
            groups.push(json!({
                "id": "folder",
                "label": "目录",
                "items": dirs.iter().map(|(n, p)| json!({
                    "kind": "folder",
                    "id": rel(root, p),
                    "label": n,
                    "subtitle": rel(root, p),
                    "path": p.to_string_lossy(),
                })).collect::<Vec<_>>(),
            }));
        }
    }

    let needle = query.trim().to_lowercase();
    let rules: Vec<Value> = crate::rules::list()
        .into_iter()
        .filter(|r| needle.is_empty() || r.name.to_lowercase().contains(&needle))
        .take(limit)
        .map(|r| {
            json!({
                "kind": "rule",
                "id": r.id,
                "label": r.name,
                "subtitle": format!("{} · {} 字", if r.scope == "project" { "项目规则" } else { "个人规则" }, r.chars),
            })
        })
        .collect();
    if !rules.is_empty() {
        groups.push(json!({ "id": "rule", "label": "规则", "items": rules }));
    }

    let sessions: Vec<Value> = recent_sessions(&needle, limit)
        .into_iter()
        .filter(|(k, _, _)| k != session)
        .map(|(key, title, updated)| {
            json!({
                "kind": "session",
                "id": key,
                "label": title,
                "subtitle": updated,
            })
        })
        .collect();
    if !sessions.is_empty() {
        groups.push(json!({ "id": "session", "label": "历史会话", "items": sessions }));
    }

    json!({
        "ok": true,
        "workspace": root.map(|p| p.to_string_lossy().to_string()),
        "groups": groups,
    })
}

/// 最近会话（只读 sessions.db；库不存在时返回空）。
fn recent_sessions(needle: &str, limit: usize) -> Vec<(String, String, String)> {
    let path = Config::config_dir().join("sessions.db");
    if !path.is_file() {
        return Vec::new();
    }
    let Ok(conn) = rusqlite::Connection::open_with_flags(
        path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    ) else {
        return Vec::new();
    };
    let Ok(mut stmt) = conn.prepare(
        "SELECT session_id, summary, updated_at FROM sessions ORDER BY updated_at DESC LIMIT 40",
    ) else {
        return Vec::new();
    };
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
        ))
    });
    let Ok(rows) = rows else {
        return Vec::new();
    };
    rows.filter_map(Result::ok)
        .filter(|(_, title, _)| needle.is_empty() || title.to_lowercase().contains(needle))
        .take(limit)
        .collect()
}

/// 读历史会话片段：摘要 + 最近 6 条消息（各截断）。
fn session_snippet(key: &str) -> Option<String> {
    let path = Config::config_dir().join("sessions.db");
    let conn = rusqlite::Connection::open_with_flags(
        path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .ok()?;
    let mut stmt = conn
        .prepare("SELECT role, content FROM session_messages WHERE session_id = ?1 ORDER BY id DESC LIMIT 6")
        .ok()?;
    let rows = stmt
        .query_map([key], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
            ))
        })
        .ok()?;
    let mut lines: Vec<String> = rows
        .filter_map(Result::ok)
        .map(|(role, content)| {
            let text: String = content.trim().chars().take(300).collect();
            format!("{role}: {text}")
        })
        .collect();
    if lines.is_empty() {
        return None;
    }
    lines.reverse();
    Some(trim_chars(&lines.join("\n"), MAX_SESSION_CHARS))
}

/// 把本轮引用渲染成一段材料；没有有效引用时返回 `None`（零开销）。
pub fn render_for_turn(session: &str, refs: Option<&Value>, text: &str) -> Option<String> {
    let refs = refs?.as_array()?;
    if refs.is_empty() {
        return None;
    }
    let root = workspace_root(session);
    let mut blocks: Vec<String> = Vec::new();
    let mut used = 0usize;
    for item in refs.iter().take(12) {
        let kind = item.get("kind").and_then(Value::as_str).unwrap_or("");
        let id = item
            .get("id")
            .or_else(|| item.get("path"))
            .and_then(Value::as_str)
            .unwrap_or("");
        if id.trim().is_empty() {
            continue;
        }
        let rendered = match kind {
            "file" => root.as_ref().and_then(|r| {
                let candidate = if item.get("path").and_then(Value::as_str).is_some() {
                    PathBuf::from(item.get("path").and_then(Value::as_str).unwrap_or(""))
                } else {
                    r.join(id)
                };
                inside_root(r, &candidate).and_then(|real| {
                    let body = std::fs::read_to_string(&real).ok()?;
                    Some(format!(
                        "### 文件：{}\n```\n{}\n```",
                        rel(r, &real),
                        trim_chars(&body, MAX_FILE_CHARS)
                    ))
                })
            }),
            "folder" => root.as_ref().and_then(|r| {
                let candidate = r.join(id);
                inside_root(r, &candidate).map(|real| {
                    let mut names: Vec<String> = std::fs::read_dir(&real)
                        .ok()
                        .into_iter()
                        .flatten()
                        .filter_map(Result::ok)
                        .map(|e| {
                            let n = e.file_name().to_string_lossy().to_string();
                            if e.path().is_dir() {
                                format!("{n}/")
                            } else {
                                n
                            }
                        })
                        .take(MAX_DIR_ENTRIES)
                        .collect();
                    names.sort();
                    format!("### 目录：{}\n{}", rel(r, &real), names.join("\n"))
                })
            }),
            "rule" => crate::rules::get(id).map(|(entry, body)| {
                format!(
                    "### 规则：{}\n{}",
                    entry.name,
                    trim_chars(body.trim(), MAX_SESSION_CHARS)
                )
            }),
            // 技能不进引用材料：技能由 use_skill 工具在需要时加载，
            // 在这里再贴一份描述只会重复占用上下文。
            "session" => session_snippet(id)
                .map(|body| format!("### 历史会话：{id}\n```\n{body}\n```")),
            _ => None,
        };
        let Some(block) = rendered else {
            continue;
        };
        let len = block.chars().count();
        if used + len > MAX_BLOCK_CHARS {
            blocks.push("（其余引用因长度上限未展开）".to_string());
            break;
        }
        used += len;
        blocks.push(block);
    }
    if blocks.is_empty() {
        return None;
    }
    Some(format!(
        "{text}\n\n---\n【用户通过 @ 指定的引用材料（仅本轮有效；材料里没写的不要编造）】\n{}",
        blocks.join("\n\n")
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 越界路径必须被拒绝：`..` 与指向根外的绝对路径都不能读。
    #[test]
    fn inside_root_rejects_escapes() {
        let root = std::env::temp_dir().join(format!("marg_ref_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(root.join("sub")).unwrap();
        let root = root.canonicalize().unwrap();
        std::fs::write(root.join("sub/a.txt"), "hi").unwrap();

        assert!(inside_root(&root, &root.join("sub/a.txt")).is_some());
        assert!(inside_root(&root, &root.join("../outside.txt")).is_none());
        assert!(inside_root(&root, &PathBuf::from("C:/Windows/win.ini")).is_none());
        let _ = std::fs::remove_dir_all(&root);
    }

    /// 无引用时零开销：不产生任何材料，原文本不受影响。
    #[test]
    fn empty_refs_render_none() {
        assert!(render_for_turn("s", None, "hi").is_none());
        assert!(render_for_turn("s", Some(&json!([])), "hi").is_none());
    }
}
