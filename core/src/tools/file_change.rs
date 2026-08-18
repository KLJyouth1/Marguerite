//! 文件变更台账（前端「文件更改卡片」的数据源）
//!
//! 记录 Agent 通过内置工具对工作区文件的每次**真实**改动（新建 / 修改 / 删除），
//! 进程级有界保留最近若干条，经 `workspace.file_changes.list` RPC 供前端读取，
//! 渲染为可折叠的 diff 卡片（展示 / 折叠 / 提问 / 交互的前置数据层）。
//!
//! 诚实边界：只记录「确实落盘」的改动；被 L2/L3 护栏拒绝或写入失败的操作不会进入台账。

use std::collections::VecDeque;
use std::sync::Mutex;

use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::events::{global_bus, Envelope};

/// 进程级最多保留的文件变更条数（FIFO 淘汰最旧）。
const MAX_CHANGES: usize = 200;

/// 一条文件变更记录。
#[derive(Debug, Clone, Serialize)]
pub struct FileChange {
    /// 相对工作区的路径
    pub path: String,
    /// 变更类型：create / modify / delete
    pub kind: String,
    /// 变更前内容（create 为空；delete 为删除前内容）
    pub before: String,
    /// 变更后内容（delete 为空）
    pub after: String,
    /// 触发工具名（write_file / edit_file / apply_patch / delete_file 等）
    pub tool: String,
    /// unix 秒级时间戳
    pub ts: i64,
}

#[derive(Default)]
struct Ledger {
    changes: VecDeque<FileChange>,
}

fn ledger() -> &'static Mutex<Ledger> {
    static L: std::sync::OnceLock<Mutex<Ledger>> = std::sync::OnceLock::new();
    L.get_or_init(|| {
        Mutex::new(Ledger {
            changes: VecDeque::new(),
        })
    })
}

/// 记录一次文件变更（幂等追加，超上限淘汰最旧）。
pub fn record_file_change(path: &str, kind: &str, before: &str, after: &str, tool: &str) {
    let mut l = ledger().lock().unwrap_or_else(|e| e.into_inner());
    l.changes.push_back(FileChange {
        path: path.to_string(),
        kind: kind.to_string(),
        before: before.to_string(),
        after: after.to_string(),
        tool: tool.to_string(),
        ts: chrono::Utc::now().timestamp(),
    });
    while l.changes.len() > MAX_CHANGES {
        l.changes.pop_front();
    }
}

/// 列出最近 `limit` 条变更（最新在前）。
pub fn list_file_changes(limit: usize) -> Vec<FileChange> {
    let l = ledger().lock().unwrap_or_else(|e| e.into_inner());
    l.changes.iter().rev().take(limit).cloned().collect()
}

/// 清空台账（测试辅助）。
pub fn clear_file_changes() {
    ledger()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .changes
        .clear();
}

// ── Artifacts（产物台账：按会话记录 Agent 生成的文件）──────────────────────

use std::collections::HashMap;

#[derive(Default)]
struct ArtifactLedger {
    /// session_key -> Vec<artifact>
    by_session: HashMap<String, Vec<ArtifactEntry>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ArtifactEntry {
    pub id: String,
    pub name: String,
    pub path: String,
    pub kind: String,
    pub size: u64,
    pub ts: i64,
    /// MIME 类型（按扩展名推断；未知为 None）
    pub mime: Option<String>,
    /// 归属会话 key（与 SESSION task-local 对齐；空串表示默认会话）
    pub session_key: String,
    /// 文件内容 SHA-256（十六进制小写）；用于内联编辑前的竞态校验。
    /// 目录类产物或文件不可读时为 None。
    pub sha256: Option<String>,
}

fn artifact_ledger() -> &'static Mutex<ArtifactLedger> {
    static A: std::sync::OnceLock<Mutex<ArtifactLedger>> = std::sync::OnceLock::new();
    A.get_or_init(|| Mutex::new(ArtifactLedger::default()))
}

/// 记录一个产物文件到指定会话。
///
/// 兼容入口：优先取 task-local `SESSION` 作为会话；若该上下文不可用（如 TUI 路径），
/// 回退到调用方传入的 `session_key`（可能为空），并记录到默认会话 + 告警（R4）。
pub fn record_artifact(session: &str, path: &str, kind: &str) {
    let resolved = crate::usage::SESSION
        .try_with(|s| s.to_string())
        .unwrap_or_else(|_| session.to_string());
    if resolved.is_empty() {
        tracing::warn!(
            path = path,
            "record_artifact: 无可用 SESSION，回退到传入 session_key（空），记录到默认会话"
        );
    }
    record_artifact_with_session(&resolved, path, kind);
}

/// 核心记录（带 `artifact.updated` 事件广播）：所有入口最终都走这里。
///
/// `session` 明确传入（工具从 `SESSION` 解析、COM 驱动从调用上下文传入），
/// 登记后向网关总线发布 `artifact.updated`（未启动网关时 `global_bus()` 为 None → no-op）。
pub fn record_artifact_with_session(session: &str, path: &str, kind: &str) {
    let name = std::path::Path::new(path)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| path.to_string());
    let size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    let id = format!("artifact:{}:{}", session, path);
    let mime = mime_for(path);
    let sha256 = sha256_of(path);
    let ts = chrono::Utc::now().timestamp();
    let entry = ArtifactEntry {
        id: id.clone(),
        name,
        path: path.to_string(),
        kind: kind.to_string(),
        size,
        ts,
        mime: mime.clone(),
        session_key: session.to_string(),
        sha256: sha256.clone(),
    };
    {
        let mut l = artifact_ledger().lock().unwrap_or_else(|e| e.into_inner());
        let list = l.by_session.entry(session.to_string()).or_default();
        // 去重：同路径替换
        if let Some(existing) = list.iter_mut().find(|a| a.path == path) {
            *existing = entry;
        } else {
            list.push(entry);
        }
    }
    // 广播 artifact.updated（R2）：网关总线未注册时 no-op
    if let Some(bus) = global_bus() {
        bus.publish(Envelope::artifact_updated(
            session, &id, session, kind, path, ts,
        ));
    }
}

/// 按 id 查找产物（跨会话扫描；id = `artifact:<session>:<path>`）。
pub fn get_artifact(id: &str) -> Option<ArtifactEntry> {
    let l = artifact_ledger().lock().unwrap_or_else(|e| e.into_inner());
    for list in l.by_session.values() {
        if let Some(a) = list.iter().find(|a| a.id == id) {
            return Some(a.clone());
        }
    }
    None
}

/// 计算文件内容的 SHA-256（十六进制小写）。文件不可读（不存在 / 目录 / 无权限）时返回 None。
///
/// 供 `artifacts.edit_inline` 在写入前做竞态校验：比对前端传入的 `expectedSha256`
/// 与磁盘当前内容哈希，不一致说明文件在用户编辑期间已被其它 turn 重新生成。
pub fn sha256_of(path: &str) -> Option<String> {
    std::fs::read(path).ok().map(|bytes| {
        let mut hasher = Sha256::new();
        hasher.update(&bytes);
        let digest = hasher.finalize();
        format!("{digest:x}")
    })
}

/// 按扩展名推断 MIME 类型（未知返回 None）。
pub fn mime_for(path: &str) -> Option<String> {
    let ext = std::path::Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let m = match ext.as_str() {
        "txt" => "text/plain",
        "md" => "text/markdown",
        "json" => "application/json",
        "yaml" | "yml" => "application/yaml",
        "toml" => "application/toml",
        "html" | "htm" => "text/html",
        "css" => "text/css",
        "js" | "mjs" | "cjs" => "text/javascript",
        "ts" => "application/typescript",
        "tsx" | "jsx" => "text/jsx",
        "csv" => "text/csv",
        "xml" => "application/xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "svg" => "image/svg+xml",
        "webp" => "image/webp",
        "bmp" => "image/bmp",
        "ico" => "image/x-icon",
        "pdf" => "application/pdf",
        "pptx" => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        "ppt" => "application/vnd.ms-powerpoint",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "doc" => "application/msword",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "xls" => "application/vnd.ms-excel",
        "mp3" => "audio/mpeg",
        "wav" => "audio/wav",
        "ogg" => "audio/ogg",
        "mp4" => "video/mp4",
        "webm" => "video/webm",
        "zip" => "application/zip",
        "rs" => "text/x-rust",
        "py" => "text/x-python",
        "go" => "text/x-go",
        "java" => "text/x-java",
        "c" | "h" => "text/x-c",
        "cpp" | "cc" | "hpp" => "text/x-c++",
        _ => return None,
    };
    Some(m.to_string())
}

/// 列出指定会话的产物（游标分页：按 ts 倒序；`cursor` 为上一页末条 ts）。
///
/// 返回 `(page, has_more, next_cursor)`：`has_more` 表示是否还有后续，`next_cursor` 为
/// 下一次分页起点的 ts（前端 `artifacts.list` 兼容既有调用，新增 `cursor`/`limit` 参数）。
pub fn list_artifacts(
    session: &str,
    cursor: Option<u64>,
    limit: usize,
) -> (Vec<ArtifactEntry>, bool, Option<i64>) {
    let mut items = {
        let l = artifact_ledger().lock().unwrap_or_else(|e| e.into_inner());
        l.by_session.get(session).cloned().unwrap_or_default()
    };
    // 倒序：最新在前
    items.sort_by(|a, b| b.ts.cmp(&a.ts));
    // 游标：取 ts 严格小于 cursor 的后续（避免与上一页末条重复）
    if let Some(c) = cursor {
        items.retain(|a| a.ts < c as i64);
    }
    let limit = limit.max(1);
    let has_more = items.len() > limit;
    let page: Vec<ArtifactEntry> = items.into_iter().take(limit).collect();
    let next_cursor = if has_more {
        page.last().map(|a| a.ts)
    } else {
        None
    };
    (page, has_more, next_cursor)
}

/// 清空指定会话的产物（测试辅助）。
pub fn clear_artifacts(session: &str) {
    artifact_ledger()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .by_session
        .remove(session);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_and_list_returns_newest_first() {
        clear_file_changes();
        record_file_change("a.rs", "create", "", "fn a(){}", "write_file");
        record_file_change("b.rs", "modify", "old", "new", "edit_file");
        let items = list_file_changes(10);
        assert_eq!(items.len(), 2);
        // 最新在前
        assert_eq!(items[0].path, "b.rs");
        assert_eq!(items[0].kind, "modify");
        assert_eq!(items[1].path, "a.rs");
        assert_eq!(items[1].kind, "create");
        clear_file_changes();
    }

    #[test]
    fn list_respects_limit() {
        clear_file_changes();
        for i in 0..5 {
            record_file_change(&format!("f{i}.rs"), "modify", "", "", "edit_file");
        }
        assert_eq!(list_file_changes(3).len(), 3);
        assert_eq!(list_file_changes(0).len(), 0);
        clear_file_changes();
    }
}
