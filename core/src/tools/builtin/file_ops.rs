//! Auto-extracted from builtin.rs — see parent module for shared infrastructure.
//!
//! File operations: copy/move/delete/make_dir

use super::*;

pub(crate) fn copy_file_tool(
    workspace: impl Into<RootSet>,
    versions: Arc<Option<VersionManager>>,
) -> Box<dyn ToolExecutor> {
    let workspace: RootSet = workspace.into();
    boxed(
        "copy_file",
        "复制文件到目标路径（覆盖前快照目标）。参数: src, dst",
        json!({"type":"object","properties":{"src":{"type":"string"},"dst":{"type":"string"}}}),
        Arc::new(move |args| {
            let ws = workspace.clone();
            let versions = versions.clone();
            Box::pin(async move {
                let src = args.get("src").ok_or_else(|| anyhow!("missing 'src'"))?;
                let dst = args.get("dst").ok_or_else(|| anyhow!("missing 'dst'"))?;
                let abs_src = ws.resolve(src)?;
                let abs_dst = ws.resolve(dst)?;
                if !abs_src.is_file() {
                    return err(format!("source not a file: {}", src), "NOT_FOUND");
                }
                // 目标路径过 L3；源文件内容过 L2（复制是"内容落到新位置"，
                // 等价于一次写入，不能因为内容不是本次生成就免检）。
                if let Some(denied) =
                    gate(&PREFLIGHT.check_path_multi(OpKind::WriteFile, &abs_dst, &guard_roots(&ws)))
                {
                    return denied;
                }
                if let Ok(src_text) = std::fs::read_to_string(&abs_src) {
                    if let Some(denied) = gate(&PREFLIGHT.check_content(&src_text)) {
                        return denied;
                    }
                }
                if let Some(parent) = abs_dst.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                if let Some(vm) = versions.as_ref() {
                    if abs_dst.exists() {
                        let _ = vm.snapshot_file(&abs_dst);
                    }
                }
                std::fs::copy(&abs_src, &abs_dst)?;
                let sess = current_session();
                crate::tools::file_change::record_artifact(&sess, dst, "copied");
                ok(format!("copied {} -> {}", src, dst))
            })
        }),
    )
}

pub(crate) fn move_file_tool(workspace: impl Into<RootSet>) -> Box<dyn ToolExecutor> {
    let workspace: RootSet = workspace.into();
    boxed(
        "move_file",
        "移动/重命名文件。参数: src, dst",
        json!({"type":"object","properties":{"src":{"type":"string"},"dst":{"type":"string"}}}),
        Arc::new(move |args| {
            let ws = workspace.clone();
            Box::pin(async move {
                let src = args.get("src").ok_or_else(|| anyhow!("missing 'src'"))?;
                let dst = args.get("dst").ok_or_else(|| anyhow!("missing 'dst'"))?;
                let abs_src = ws.resolve(src)?;
                let abs_dst = ws.resolve(dst)?;
                // 移动同时改变源与目标两端，两端都要过 L3
                // （例如把文件移进 .ssh/，或把 .git 里的文件移出来）。
                // 多根：源端落在任一登记根内即放行（P2-9）。
                if let Some(denied) =
                    gate(&PREFLIGHT.check_path_multi(OpKind::Move, &abs_src, &guard_roots(&ws)))
                {
                    return denied;
                }
                if let Some(denied) =
                    gate(&PREFLIGHT.check_path_multi(OpKind::WriteFile, &abs_dst, &guard_roots(&ws)))
                {
                    return denied;
                }
                if let Some(parent) = abs_dst.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::rename(&abs_src, &abs_dst)?;
                let sess = current_session();
                crate::tools::file_change::record_artifact(&sess, dst, "moved");
                ok(format!("moved {} -> {}", src, dst))
            })
        }),
    )
}

/// 默认删除工具：一律转回收站（可恢复）。
///
/// 语义（T1）：Marguerite 自有删除工具的默认行为是「删除转回收站」——
/// 永久删除必须显式调用 `delete_file_permanent`，此处不提供 to_trash=false
/// 之类的静默降级路径，避免「以为进回收站了其实永久删了」。
pub(crate) fn delete_file_tool(workspace: impl Into<RootSet>) -> Box<dyn ToolExecutor> {
    let workspace: RootSet = workspace.into();
    boxed(
        "delete_file",
        "删除工作区内文件或目录（移入回收站 .marguerite/trash，可用 restore_deleted 按 id 或原路径恢复）。参数: path。不可恢复的永久删除请改用 delete_file_permanent。",
        json!({
            "type":"object",
            "properties":{
                "path":{"type":"string"}
            }
        }),
        Arc::new(move |args| {
            let ws = workspace.clone();
            Box::pin(async move {
                let p = args.get("path").ok_or_else(|| anyhow!("missing 'path'"))?;
                let abs = ws.resolve(p)?;
                if let Some(denied) =
                    gate(&PREFLIGHT.check_path_multi(OpKind::Delete, &abs, &guard_roots(&ws)))
                {
                    return denied;
                }
                let was_file = abs.is_file();
                let before = if was_file {
                    std::fs::read_to_string(&abs).unwrap_or_default()
                } else {
                    String::new()
                };
                // 软删除：移入回收站。失败时错误透传，不静默回退硬删。
                // 多根：回收站归属于该文件所在的根（restore 时才能原位还原）。
                let store_root = ws.root_containing(&abs);
                let entry = trash::TrashStore::new(&store_root)
                    .delete(&abs, "delete_file")
                    .map_err(|e| anyhow!("trash::delete failed: {}", e))?;
                if was_file {
                    crate::tools::file_change::record_file_change(
                        p,
                        "delete",
                        &before,
                        "",
                        "delete_file",
                    );
                }
                ok(format!(
                    "moved to recycle bin: {} (trash id {}, restorable via restore_deleted)",
                    p, entry.id
                ))
            })
        }),
    )
}

/// 永久删除：显式高危动词，无回收站兜底。
///
/// 与 `delete_file`（默认转回收站）分离：调用方必须明确选择不可逆语义，
/// 上层审批链路会按高风险动作提级确认（见 guard::tool_intent 的 Delete 判定）。
pub(crate) fn delete_file_permanent_tool(workspace: impl Into<RootSet>) -> Box<dyn ToolExecutor> {
    let workspace: RootSet = workspace.into();
    boxed(
        "delete_file_permanent",
        "永久删除工作区内文件或目录（递归删目录；不可恢复、无回收站兜底）。仅当用户明确要求永久删除时使用；可恢复删除请用 delete_file。参数: path",
        json!({
            "type":"object",
            "properties":{
                "path":{"type":"string"}
            }
        }),
        Arc::new(move |args| {
            let ws = workspace.clone();
            Box::pin(async move {
                let p = args.get("path").ok_or_else(|| anyhow!("missing 'path'"))?;
                let abs = ws.resolve(p)?;
                if let Some(denied) =
                    gate(&PREFLIGHT.check_path_multi(OpKind::Delete, &abs, &guard_roots(&ws)))
                {
                    return denied;
                }
                let was_file = abs.is_file();
                let before = if was_file {
                    std::fs::read_to_string(&abs).unwrap_or_default()
                } else {
                    String::new()
                };
                if abs.is_dir() {
                    trash::clear_readonly_for_delete_pub(&abs)?;
                    std::fs::remove_dir_all(&abs)?;
                } else {
                    trash::clear_readonly_for_delete_pub(&abs)?;
                    std::fs::remove_file(&abs)?;
                }
                if was_file {
                    crate::tools::file_change::record_file_change(
                        p,
                        "delete",
                        &before,
                        "",
                        "delete_file_permanent",
                    );
                }
                ok(format!("permanently deleted {} (no recycle bin)", p))
            })
        }),
    )
}

/// 从回收站恢复：按 trash id 或原路径。
///
/// 恢复目标必须位于当前工作区内（路径穿越拒绝）；目标已存在时自动安全命名，
/// 绝不覆盖在位文件。
pub(crate) fn restore_from_trash_tool(workspace: impl Into<RootSet>) -> Box<dyn ToolExecutor> {
    let workspace: RootSet = workspace.into();
    boxed(
        "restore_from_trash",
        "从回收站恢复已删除的文件/目录。参数: id（trash id 或原路径，原路径多条同路径记录时取最新）。恢复目标经工作区校验，冲突时自动安全命名。",
        json!({
            "type":"object",
            "properties":{
                "id":{"type":"string","description":"trash id（数字）或原路径"}
            },
            "required":["id"]
        }),
        Arc::new(move |args| {
            let ws = workspace.clone();
            Box::pin(async move {
                let id = args
                    .get("id")
                    .ok_or_else(|| anyhow!("missing 'id'"))?
                    .trim()
                    .to_string();
                if id.is_empty() {
                    return err("restore_from_trash 需要非空的 id（trash id 或原路径）".into(), "INVALID_PARAMS");
                }
                // 多根：逐个根的回收站尝试恢复（条目归属哪个根就由哪个根还原）
                let mut restored = None;
                for root in ws.all() {
                    if let Ok(out) = trash::TrashStore::new(root).restore(&id, root) {
                        restored = Some(out);
                        break;
                    }
                }
                match restored {
                    Some(out) => {
                        let note = if out.renamed {
                            "（目标已存在，已安全改名避免覆盖）"
                        } else {
                            ""
                        };
                        // 恢复 = 把内容写回工作区：登记为 artifact，供上层审计/回滚。
                        let sess = current_session();
                        crate::tools::file_change::record_artifact(
                            &sess,
                            &out.restored_to.display().to_string(),
                            "restored",
                        );
                        ok(format!(
                            "restored trash entry {} -> {}{}",
                            id,
                            out.restored_to.display(),
                            note
                        ))
                    }
                    None => err(
                        format!("restore_from_trash failed: 所有工作区根都找不到条目 {id}"),
                        "RESTORE_FAILED",
                    ),
                }
            })
        }),
    )
}

/// 清空回收站：需显式 confirm=true 二次确认（本工具层的确认语义；
/// 上层审批链路仍会按 Delete/Reset 意图提级）。
pub(crate) fn empty_trash_tool(workspace: impl Into<RootSet>) -> Box<dyn ToolExecutor> {
    let workspace: RootSet = workspace.into();
    boxed(
        "empty_trash",
        "清空回收站（危险：所有已删除文件不可恢复）。参数: confirm 必须为 true。",
        json!({
            "type":"object",
            "properties":{
                "confirm":{"type":"boolean","description":"必须为 true 才会执行"}
            },
            "required":["confirm"]
        }),
        Arc::new(move |args| {
            let ws = workspace.clone();
            Box::pin(async move {
                let confirm = args
                    .get("confirm")
                    .map(|s| s == "true" || s == "1")
                    .unwrap_or(false);
                if !confirm {
                    return err(
                        "empty_trash 需要 confirm=true 二次确认（清空后不可恢复）".into(),
                        "INVALID_PARAMS",
                    );
                }
                // 多根：清空每个根各自的回收站，汇总条数
                let mut total = 0usize;
                for root in ws.all() {
                    match trash::TrashStore::new(root).empty() {
                        Ok(n) => total += n,
                        Err(e) => return err(format!("empty_trash failed: {e}"), "EMPTY_FAILED"),
                    }
                }
                ok(format!("emptied recycle bin: {} entries removed", total))
            })
        }),
    )
}

pub(crate) fn make_dir_tool(workspace: impl Into<RootSet>) -> Box<dyn ToolExecutor> {
    let workspace: RootSet = workspace.into();
    boxed(
        "make_dir",
        "创建目录（含父目录）。参数: path",
        json!({"type":"object","properties":{"path":{"type":"string"}}}),
        Arc::new(move |args| {
            let ws = workspace.clone();
            Box::pin(async move {
                let p = args.get("path").ok_or_else(|| anyhow!("missing 'path'"))?;
                let abs = ws.resolve(p)?;
                // 建目录也是写操作：阻止在工作区内凭空造出 .ssh/ .aws/ 等凭证目录
                if let Some(denied) =
                    gate(&PREFLIGHT.check_path_multi(OpKind::WriteFile, &abs, &guard_roots(&ws)))
                {
                    return denied;
                }
                std::fs::create_dir_all(&abs)?;
                let sess = current_session();
                crate::tools::file_change::record_artifact(&sess, p, "directory");
                ok(format!("created dir {}", p))
            })
        }),
    )
}
