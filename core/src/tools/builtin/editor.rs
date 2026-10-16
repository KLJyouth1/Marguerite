//! Auto-extracted from builtin.rs — see parent module for shared infrastructure.
//!
//! Str-replace editor + file operations (copy/move/delete/make_dir)

use super::*;

// ---------------------------------------------------------------------------
// str_replace_editor：模型侧文件编辑器（对齐 DSH dsh-tool-str-replace-editor）
//
// command 子命令：view / create / str_replace / insert / undo_edit。
// - 路径经 safe_resolve 约束在工作区内，沿用仓库「相对工作区路径」惯例
//   （DSH 参考实现要求绝对路径，这里与仓库其余文件工具保持一致）。
// - str_replace 为**逐字节精确**替换：old_str 必须恰好命中一次——命中 0 次
//   报 NO_MATCH，命中多次报 AMBIGUOUS_EDIT。与 DSH 参考实现 replaceInFile 的
//   「唯一匹配」语义一致（任务曾表述为「替换全部」，实测 DSH 源码是唯一匹配，
//   故以此为准）。不做任何换行符归一化，CRLF 文件逐字节匹配，不会误伤。
// - insert 的 insert_line 采用 DSH 语义：0 = 文件头；1..=行数 = 插入到该行之后；
//   行数 = 文件尾。合法区间 [0, 内容按 '\n' 切分的行数]（尾随换行产生一个空尾行）。
// - undo_edit 回退最近一次成功的 create/str_replace/insert（进程内单槽快照）；
//   无历史报 NO_HISTORY。create 的回退即删除该文件。
// - 文件不存在：view/str_replace/insert 报错（不隐式建文件）；create 创建父目录。
// ---------------------------------------------------------------------------

/// 最近一次编辑快照，供 `undo_edit` 回退。
pub(crate) struct EditorSnapshot {
    /// 相对工作区路径（消息/台账展示用）
    rel: String,
    /// 绝对路径（落盘用）
    abs: PathBuf,
    /// 编辑前文件是否存在（create 时为 false，undo 需删除该文件）
    existed_before: bool,
    /// 编辑前内容（existed_before == false 时为冗余空串）
    before: String,
}

pub(crate) async fn editor_view(ws: &Path, p: &str) -> Result<ToolResult> {
    if p.trim().is_empty() {
        return err("path must be a non-empty string".into(), "INVALID_PARAMS");
    }
    let abs = safe_resolve(ws, p)?;
    if !abs.exists() {
        return err(format!("The path {} does not exist.", p), "NOT_FOUND");
    }
    if !abs.is_file() {
        return err(
            format!("The path {} is not a regular file.", p),
            "NOT_REGULAR_FILE",
        );
    }
    let content = std::fs::read_to_string(&abs).map_err(|e| anyhow!("read {}: {}", p, e))?;
    let lines: Vec<&str> = content.split('\n').collect();
    let mut out = format!(
        "Here's the content of {} with line numbers (which has a total of {} lines):\n",
        p,
        lines.len()
    );
    for (i, line) in lines.iter().enumerate() {
        out.push_str(&format!("{:>6}  {}\n", i + 1, line));
    }
    ok(out)
}

pub(crate) async fn editor_create(
    ws: &Path,
    undo: &Arc<Mutex<Option<EditorSnapshot>>>,
    p: &str,
    file_text: Option<&String>,
) -> Result<ToolResult> {
    if p.trim().is_empty() {
        return err("path must be a non-empty string".into(), "INVALID_PARAMS");
    }
    let content = match file_text {
        Some(s) => s.clone(),
        None => {
            return err(
                "Parameter `file_text` is required for command: create".into(),
                "INVALID_PARAMS",
            )
        }
    };
    let abs = safe_resolve(ws, p)?;
    if abs.exists() {
        return err(
            format!(
                "File already exists at: {}. Cannot overwrite files using command `create`.",
                p
            ),
            "ALREADY_EXISTS",
        );
    }
    if let Some(denied) = gate(&PREFLIGHT.check_write(&abs, &normalize_workspace(ws), &content)) {
        return denied;
    }
    if let Some(parent) = abs.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&abs, &content)?;
    crate::tools::file_change::record_file_change(p, "create", "", &content, "str_replace_editor");
    let sess = current_session();
    crate::tools::file_change::record_artifact(&sess, p, "created");
    *undo.lock().unwrap_or_else(|e| e.into_inner()) = Some(EditorSnapshot {
        rel: p.to_string(),
        abs: abs.clone(),
        existed_before: false,
        before: String::new(),
    });
    ok(format!("New file created successfully at: {}", p))
}

pub(crate) async fn editor_str_replace(
    ws: &Path,
    versions: &Arc<Option<VersionManager>>,
    undo: &Arc<Mutex<Option<EditorSnapshot>>>,
    p: &str,
    old_str: Option<&String>,
    new_str: Option<&String>,
) -> Result<ToolResult> {
    if p.trim().is_empty() {
        return err("path must be a non-empty string".into(), "INVALID_PARAMS");
    }
    let old = match old_str {
        None => {
            return err(
                "Parameter `old_str` is required for command: str_replace".into(),
                "INVALID_PARAMS",
            )
        }
        Some(s) if s.is_empty() => {
            return err(
                "Parameter `old_str` is empty for command: str_replace".into(),
                "INVALID_PARAMS",
            )
        }
        Some(s) => s.clone(),
    };
    let new = new_str.cloned().unwrap_or_default();
    let abs = safe_resolve(ws, p)?;
    if !abs.is_file() {
        return err(
            format!("The path {} does not exist or is not a regular file.", p),
            "NOT_FOUND",
        );
    }
    if let Some(denied) = gate(&PREFLIGHT.check_write(&abs, &normalize_workspace(ws), &new)) {
        return denied;
    }
    let content = std::fs::read_to_string(&abs).map_err(|e| anyhow!("read {}: {}", p, e))?;
    // 逐字节精确匹配（不做换行符归一化，CRLF 不会误伤）。
    let offsets: Vec<usize> = content
        .match_indices(old.as_str())
        .map(|(o, _)| o)
        .collect();
    if offsets.is_empty() {
        return err(
            format!(
                "No replacement was performed, old_str `{}` did not appear verbatim in {}.",
                old, p
            ),
            "NO_MATCH",
        );
    }
    if offsets.len() > 1 {
        let line_nos: Vec<String> = offsets
            .iter()
            .map(|&o| (content[..o].matches('\n').count() + 1).to_string())
            .collect();
        return err(
            format!(
                "No replacement was performed. Multiple occurrences of old_str `{}` in lines [{}]. Please ensure it is unique",
                old,
                line_nos.join(", ")
            ),
            "AMBIGUOUS_EDIT",
        );
    }
    let o = offsets[0];
    let replaced = format!("{}{}{}", &content[..o], new, &content[o + old.len()..]);
    if let Some(vm) = versions.as_ref() {
        let _ = vm.snapshot_file(&abs);
    }
    std::fs::write(&abs, &replaced)?;
    crate::tools::file_change::record_file_change(
        p,
        "modify",
        &content,
        &replaced,
        "str_replace_editor",
    );
    let sess = current_session();
    crate::tools::file_change::record_artifact(&sess, p, "modified");
    *undo.lock().unwrap_or_else(|e| e.into_inner()) = Some(EditorSnapshot {
        rel: p.to_string(),
        abs: abs.clone(),
        existed_before: true,
        before: content,
    });
    ok(format!("The file {} has been edited successfully.", p))
}

pub(crate) async fn editor_insert(
    ws: &Path,
    versions: &Arc<Option<VersionManager>>,
    undo: &Arc<Mutex<Option<EditorSnapshot>>>,
    p: &str,
    insert_line: Option<&String>,
    new_str: Option<&String>,
) -> Result<ToolResult> {
    if p.trim().is_empty() {
        return err("path must be a non-empty string".into(), "INVALID_PARAMS");
    }
    let new = match new_str {
        Some(s) => s.clone(),
        None => {
            return err(
                "Parameter `new_str` is required for command: insert".into(),
                "INVALID_PARAMS",
            )
        }
    };
    let insert_line = match insert_line {
        None => {
            return err(
                "Parameter `insert_line` is required for command: insert".into(),
                "INVALID_PARAMS",
            )
        }
        Some(s) => match s.trim().parse::<i64>() {
            Ok(n) => n,
            Err(_) => {
                return err(
                    format!(
                        "Invalid `insert_line` parameter: {}. It should be an integer.",
                        s
                    ),
                    "INVALID_PARAMS",
                )
            }
        },
    };
    let abs = safe_resolve(ws, p)?;
    if !abs.is_file() {
        return err(
            format!("The path {} does not exist or is not a regular file.", p),
            "NOT_FOUND",
        );
    }
    if let Some(denied) = gate(&PREFLIGHT.check_write(&abs, &normalize_workspace(ws), &new)) {
        return denied;
    }
    let content = std::fs::read_to_string(&abs).map_err(|e| anyhow!("read {}: {}", p, e))?;
    let lines: Vec<&str> = content.split('\n').collect();
    if insert_line < 0 || insert_line > lines.len() as i64 {
        return err(
            format!(
                "Invalid `insert_line` parameter: {}. It should be within the range of lines of the file: [0, {}]",
                insert_line,
                lines.len()
            ),
            "INVALID_PARAMS",
        );
    }
    let il = insert_line as usize;
    // 与 DSH insertInFile 一致：lines[..il] + new.split('\n') + lines[il..] 以 '\n' 连接。
    let mut parts: Vec<&str> = Vec::with_capacity(lines.len() + 1);
    parts.extend_from_slice(&lines[..il]);
    parts.extend(new.split('\n'));
    parts.extend_from_slice(&lines[il..]);
    let after = parts.join("\n");
    if let Some(vm) = versions.as_ref() {
        let _ = vm.snapshot_file(&abs);
    }
    std::fs::write(&abs, &after)?;
    crate::tools::file_change::record_file_change(
        p,
        "modify",
        &content,
        &after,
        "str_replace_editor",
    );
    let sess = current_session();
    crate::tools::file_change::record_artifact(&sess, p, "modified");
    *undo.lock().unwrap_or_else(|e| e.into_inner()) = Some(EditorSnapshot {
        rel: p.to_string(),
        abs: abs.clone(),
        existed_before: true,
        before: content,
    });
    ok(format!("The file {} has been edited successfully.", p))
}

pub(crate) async fn editor_undo(undo: &Arc<Mutex<Option<EditorSnapshot>>>) -> Result<ToolResult> {
    let snapshot = undo.lock().unwrap_or_else(|e| e.into_inner()).take();
    match snapshot {
        None => err("No edits to undo.".into(), "NO_HISTORY"),
        Some(snap) => {
            if snap.existed_before {
                let now = std::fs::read_to_string(&snap.abs).unwrap_or_default();
                std::fs::write(&snap.abs, &snap.before)
                    .map_err(|e| anyhow!("undo {}: {}", snap.rel, e))?;
                crate::tools::file_change::record_file_change(
                    &snap.rel,
                    "modify",
                    &now,
                    &snap.before,
                    "str_replace_editor",
                );
            } else {
                let now = std::fs::read_to_string(&snap.abs).unwrap_or_default();
                std::fs::remove_file(&snap.abs).map_err(|e| anyhow!("undo {}: {}", snap.rel, e))?;
                crate::tools::file_change::record_file_change(
                    &snap.rel,
                    "delete",
                    &now,
                    "",
                    "str_replace_editor",
                );
            }
            ok(format!("Undid the last edit to {}.", snap.rel))
        }
    }
}

pub(crate) fn str_replace_editor_tool(
    workspace: PathBuf,
    versions: Arc<Option<VersionManager>>,
) -> Box<dyn ToolExecutor> {
    let undo: Arc<Mutex<Option<EditorSnapshot>>> = Arc::new(Mutex::new(None));
    boxed(
        "str_replace_editor",
        "文件编辑器。command 可选 view/create/str_replace/insert/undo_edit。参数: path(相对路径), file_text(create 必填), old_str+new_str(str_replace), insert_line+new_str(insert)。str_replace 逐字节精确匹配且必须唯一命中。insert 在 insert_line 行后插入(0=文件头, 行数=文件尾)。undo_edit 回退最近一次编辑。",
        json!({
            "type": "object",
            "properties": {
                "command": {"type": "string", "enum": ["view", "create", "str_replace", "insert", "undo_edit"]},
                "path": {"type": "string"},
                "file_text": {"type": "string"},
                "old_str": {"type": "string"},
                "new_str": {"type": "string"},
                "insert_line": {"type": "integer"}
            },
            "required": ["command", "path"]
        }),
        Arc::new(move |args| {
            let ws = workspace.clone();
            let versions = versions.clone();
            let undo = undo.clone();
            Box::pin(async move {
                let command = args.get("command").map(|s| s.as_str()).unwrap_or("");
                let path = args.get("path").map(|s| s.as_str()).unwrap_or("");
                match command {
                    "view" => editor_view(&ws, path).await,
                    "create" => editor_create(&ws, &undo, path, args.get("file_text")).await,
                    "str_replace" => {
                        editor_str_replace(
                            &ws,
                            &versions,
                            &undo,
                            path,
                            args.get("old_str"),
                            args.get("new_str"),
                        )
                        .await
                    }
                    "insert" => {
                        editor_insert(
                            &ws,
                            &versions,
                            &undo,
                            path,
                            args.get("insert_line"),
                            args.get("new_str"),
                        )
                        .await
                    }
                    "undo_edit" => editor_undo(&undo).await,
                    _ => err(format!("unknown command: {}", command), "INVALID_COMMAND"),
                }
            })
        }),
    )
}


