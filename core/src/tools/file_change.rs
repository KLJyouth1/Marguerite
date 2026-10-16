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
/// 命令类工具（`shell_exec`）产出物的 `kind` 标记。
///
/// 与文件类工具的 `created`/`modified`/`appended` 区分开：**只对本类做
/// 临时辅助脚本过滤**，用户明确要求创建的交付物不受影响。
pub const GENERATED_KIND: &str = "generated";

/// 文件名是否像**临时辅助脚本**（而非交付物）。
///
/// # 为什么需要（实测 2026-10-09）
///
/// 智能体生成 office 文件时，常先在工作区写一次性脚本再执行它（实测
/// `_mk_xlsx.py`、`_locate.py`、`_mk_v5budget.py`、`_verify_v5budget.py`）。
/// 它们经产物登记进入**产物面板**，于是交付物被临时脚本淹没 —— 用户看到的是
/// 「怎么一堆 `_*.py`」，真正要的表格反而被埋掉。
///
/// 提示词层（`agent/store.rs` 规则 5b）已要求「一次性脚本写到工作区之外」，
/// **但实测无效**（模型仍用 `write_file` 写进工作区）→ 只能在登记侧兜底。
///
/// # 判据（刻意保守：宁可少过滤，也不误杀交付物）
///
/// 只覆盖**有明确临时语义**的命名，不做「看起来像脚本就过滤」的启发式：
/// - 备份类：`~$*`、`*.bak`、`*~`、`*.tmp`（不限后缀）；
/// - 临时脚本：`_`/`__`/`tmp`/`temp` 前缀 **且** 后缀为 `.py/.ps1/.sh/.bat/.cmd`。
///
/// 交付物形态（`.xlsx/.docx/.pptx/.md/.html`、以及 `_final.xlsx` 这类非脚本的
/// 下划线前缀文件）均**不受影响**。
pub fn looks_like_temp_helper(name: &str) -> bool {
    let lower = name.to_lowercase();
    if lower.starts_with("~$")
        || lower.ends_with(".bak")
        || lower.ends_with('~')
        || lower.ends_with(".tmp")
    {
        return true;
    }
    let is_script = [".py", ".ps1", ".sh", ".bat", ".cmd"]
        .iter()
        .any(|ext| lower.ends_with(ext));
    if !is_script {
        return false;
    }
    let stem = lower.rsplit_once('.').map(|(s, _)| s).unwrap_or(&lower);
    stem.starts_with('_') || stem.starts_with("tmp") || stem.starts_with("temp")
}

/// 登记后向网关总线发布 `artifact.updated`（未启动网关时 `global_bus()` 为 None → no-op）。
pub fn record_artifact_with_session(session: &str, path: &str, kind: &str) {
    let name = std::path::Path::new(path)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| path.to_string());
    // 过滤「临时辅助脚本」噪声 —— **对所有 kind 生效**。
    //
    // 起初只过滤 `generated`（shell_exec 产物），想避免误杀用户明确创建的
    // `_x.py`。但实测证明**那条路走不通**：提示词已明确要求「一次性脚本写到
    // 工作区之外」，模型仍照旧用 `write_file` 把 `_mk_v5budget.py` /
    // `_verify_v5budget.py` 写进工作区（日志可证，kind=created）→ 只拦
    // `generated` 等于没拦。
    //
    // 权衡后的取舍：**按名字模式过滤，不看 kind**。判据刻意保守（见
    // [`looks_like_temp_helper`]：仅备份类 + `_`/tmp/temp 前缀**且**脚本后缀），
    // 因此 `budget.xlsx` / `report.docx` / `_final.xlsx` 等交付物形态不受影响；
    // 代价是「用户点名要一个 `_helper.py`」这一罕见情形也会被隐去（文件仍在磁盘、
    // 仍可打开，只是不进产物面板）。此处按 info 留痕，便于事后审计。
    if looks_like_temp_helper(&name) {
        tracing::info!(
            path = path,
            kind = kind,
            "判定为临时辅助脚本，不登记为产物（文件仍在磁盘）"
        );
        return;
    }
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
    // 登记**必须留痕**：本条记录的会话键就是 `artifacts.list` 的查询键。
    // 此前成功路径完全静默，导致「磁盘有产物、右栏空态」只能靠猜 —— 有日志后
    // 一次 `grep 登记产物` 即可看出产物落在哪个键下、与前端查询键是否一致。
    tracing::info!(
        session = %session,
        path = path,
        kind = kind,
        size = size,
        "登记产物"
    );
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
    items.sort_by_key(|a| std::cmp::Reverse(a.ts));
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

// ── 工作区快照 / 差分登记（命令类工具的产物可见性）──────────────────────────
//
// 真机缺陷：产物台账此前只由「文件类工具」（write_file / edit_file / apply_patch /
// copy / move / mkdir）登记。模型用 `shell_exec` 跑脚本产出交付物时（真机案例：
// `python make_ppt.py` 生成 presentation.pptx / presentation.pdf），磁盘上确实有文件，
// 但产物面板全空 —— 用户看到的就是「明明有产物，面板却说没有」。
//
// 这里用「命令前后工作区快照差分」补齐这一类产物。**有界**是硬要求：跳过依赖/产物
// 目录、限制扫描条数与深度；一旦快照被截断就**不登记**（宁可少登记，也不猜），
// 避免把整棵依赖树灌进产物台账。

/// 快照时跳过的重型/非交付目录（大小写不敏感）。
const SNAPSHOT_SKIP_DIRS: &[&str] = &[
    ".git",
    "node_modules",
    "target",
    "__pycache__",
    ".venv",
    "venv",
    "env",
    "dist",
    "build",
    ".next",
    ".turbo",
    ".cache",
    ".idea",
    ".vscode",
];

/// 单次快照最多收录的文件数（超出即标记截断）。
const SNAPSHOT_MAX_FILES: usize = 20_000;
/// 单次快照的最大递归深度。
const SNAPSHOT_MAX_DEPTH: usize = 8;
/// 单条命令最多登记多少条新增/改动产物。
const DIFF_MAX_REGISTER: usize = 50;
/// 改动文件数超过该阈值视为批量操作（装依赖、批量重写），不做产物登记。
const DIFF_SKIP_IF_MORE_THAN: usize = 200;

/// 工作区快照：文件路径 → (mtime 纳秒, 字节数)。
#[derive(Debug, Default)]
pub struct WorkspaceSnapshot {
    pub files: std::collections::BTreeMap<std::path::PathBuf, (u128, u64)>,
    /// 扫描因条数上限被截断 —— 截断的快照不可用于差分（否则会把未扫描到的文件误判为新增）。
    pub truncated: bool,
}

/// 采集工作区快照（有界；见上方常量）。
pub fn snapshot_workspace(root: &std::path::Path) -> WorkspaceSnapshot {
    let mut snap = WorkspaceSnapshot::default();
    let mut stack: Vec<(std::path::PathBuf, usize)> = vec![(root.to_path_buf(), 0)];
    while let Some((dir, depth)) = stack.pop() {
        if depth > SNAPSHOT_MAX_DEPTH {
            continue;
        }
        let Ok(rd) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in rd.flatten() {
            let Ok(ft) = entry.file_type() else {
                continue;
            };
            let path = entry.path();
            if ft.is_dir() {
                let name = entry.file_name().to_string_lossy().to_string();
                if SNAPSHOT_SKIP_DIRS
                    .iter()
                    .any(|s| name.eq_ignore_ascii_case(s))
                {
                    continue;
                }
                stack.push((path, depth + 1));
            } else if ft.is_file() {
                if snap.files.len() >= SNAPSHOT_MAX_FILES {
                    snap.truncated = true;
                    return snap;
                }
                let Ok(md) = entry.metadata() else {
                    continue;
                };
                let mtime = md
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_nanos())
                    .unwrap_or(0);
                snap.files.insert(path, (mtime, md.len()));
            }
        }
    }
    snap
}

/// 对比两次快照，把新增/改动的文件登记为产物；返回登记条数。
///
/// 不登记的情形（都如实记日志，不静默）：快照被截断、无改动、改动条数超阈值。
pub fn register_workspace_diff(
    before: &WorkspaceSnapshot,
    after: &WorkspaceSnapshot,
    session: &str,
    kind: &str,
) -> usize {
    if before.truncated || after.truncated {
        tracing::info!(
            before_truncated = before.truncated,
            after_truncated = after.truncated,
            "工作区快照被条数上限截断，跳过差分产物登记（不猜）"
        );
        return 0;
    }
    let mut changed: Vec<&std::path::PathBuf> = after
        .files
        .iter()
        .filter(|(p, meta)| before.files.get(*p) != Some(meta))
        .map(|(p, _)| p)
        .collect();
    if changed.is_empty() {
        // 不静默：「命令跑完但产物台账没变化」有多种可能（写在工作区外、
        // 阈值跳过、确实无改动），留痕才能一次判定，避免只能靠猜。
        tracing::info!(
            before_files = before.files.len(),
            after_files = after.files.len(),
            "命令后工作区无文件级变化，未登记产物（产物可能写在工作区之外）"
        );
        return 0;
    }
    if changed.len() > DIFF_SKIP_IF_MORE_THAN {
        tracing::warn!(
            count = changed.len(),
            "命令改动的文件数超过阈值，疑似批量操作（装依赖/批量重写），跳过产物登记"
        );
        return 0;
    }
    changed.truncate(DIFF_MAX_REGISTER);
    for p in &changed {
        record_artifact(session, &p.to_string_lossy(), kind);
    }
    changed.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    // 本模块两个用例共享进程级全局台账（`ledger()` 为有界全局 VecDeque），
    // 默认并行执行会互相插入条目导致计数错位（flaky）。用进程级互斥串行化它们。
    static LEDGER_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn record_and_list_returns_newest_first() {
        let _guard = LEDGER_TEST_LOCK.lock().unwrap();
        clear_file_changes();
        record_file_change("a.rs", "create", "", "fn a(){}", "write_file");
        record_file_change("b.rs", "modify", "old", "new", "edit_file");
        let items = list_file_changes(10);
        // 只校验「本次写入的两条是否存在且最新在前」，避免被并发的其他用例/工具写入干扰
        let pos_b = items
            .iter()
            .position(|c| c.path == "b.rs" && c.kind == "modify");
        let pos_a = items
            .iter()
            .position(|c| c.path == "a.rs" && c.kind == "create");
        assert!(pos_b.is_some(), "应记录 b.rs: {items:?}");
        assert!(pos_a.is_some(), "应记录 a.rs: {items:?}");
        assert!(
            pos_b.unwrap() < pos_a.unwrap(),
            "最新（b.rs）应在前: {items:?}"
        );
        clear_file_changes();
    }

    #[test]
    fn list_respects_limit() {
        let _guard = LEDGER_TEST_LOCK.lock().unwrap();
        clear_file_changes();
        for i in 0..5 {
            record_file_change(&format!("f{i}.rs"), "modify", "", "", "edit_file");
        }
        assert_eq!(list_file_changes(3).len(), 3);
        assert_eq!(list_file_changes(0).len(), 0);
        clear_file_changes();
    }

    // ── 工作区差分登记（命令类工具产物可见性）────────────────────────────────
    //
    // 每个用例用**独立会话 key**（产物台账按会话分桶），因此互不干扰、可并行。

    fn tmp_ws(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "marguerite-snapshot-{tag}-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// 真机缺陷回归：脚本新建的交付物必须能被差分登记（此前面板完全看不到）。
    #[test]
    fn workspace_diff_registers_created_file() {
        let ws = tmp_ws("create");
        let session = format!("ws.diff.create.{}", uuid::Uuid::new_v4());
        std::fs::write(ws.join("make_ppt.py"), "print(1)").unwrap();

        let before = snapshot_workspace(&ws);
        std::fs::write(ws.join("presentation.pptx"), "deck-bytes").unwrap();

        let after = snapshot_workspace(&ws);
        assert_eq!(
            register_workspace_diff(&before, &after, &session, "generated"),
            1
        );
        let listed = list_artifacts(&session, None, 50).0;
        assert_eq!(listed.len(), 1, "新建文件应登记为产物: {listed:?}");
        assert!(listed[0].path.ends_with("presentation.pptx"));

        clear_artifacts(&session);
        let _ = std::fs::remove_dir_all(&ws);
    }

    /// 内容被改写的文件同样要被登记。
    ///
    /// 判定量是 (mtime 纳秒, 字节数)：**同尺寸且落在同一个文件系统时间戳刻度内**的
    /// 改写无法识别（Windows 上实测存在该情形，见 `same_size_rewrite_within_one_tick`）。
    /// 这里用「尺寸变化」这一确定性情形覆盖主路径。
    #[test]
    fn workspace_diff_registers_modified_file() {
        let ws = tmp_ws("modify");
        let session = format!("ws.diff.modify.{}", uuid::Uuid::new_v4());
        let target = ws.join("report.md");
        std::fs::write(&target, "aaaa").unwrap();
        let before = snapshot_workspace(&ws);

        std::fs::write(&target, "bbbbbbbb").unwrap();
        let after = snapshot_workspace(&ws);

        assert_eq!(
            register_workspace_diff(&before, &after, &session, "generated"),
            1
        );
        clear_artifacts(&session);
        let _ = std::fs::remove_dir_all(&ws);
    }

    /// 已知边界（如实固定下来，避免后人误以为「任何改写都能识别」）：
    /// 同尺寸改写若 mtime 未变（同一刻度内），差分不做登记 —— 少登记，不猜。
    #[test]
    fn same_size_rewrite_within_one_tick_is_not_detected() {
        let mut before = WorkspaceSnapshot::default();
        let mut after = WorkspaceSnapshot::default();
        let p = std::path::PathBuf::from("C:/ws/same.bin");
        before.files.insert(p.clone(), (1_000, 4));
        after.files.insert(p, (1_000, 4));
        let session = format!("ws.diff.sametick.{}", uuid::Uuid::new_v4());
        assert_eq!(register_workspace_diff(&before, &after, &session, "generated"), 0);
        clear_artifacts(&session);
    }

    /// 没有改动时不得凭空登记（否则每条无害命令都会污染面板）。
    #[test]
    fn workspace_diff_without_changes_registers_nothing() {
        let ws = tmp_ws("noop");
        let session = format!("ws.diff.noop.{}", uuid::Uuid::new_v4());
        std::fs::write(ws.join("a.txt"), "x").unwrap();
        let before = snapshot_workspace(&ws);
        let after = snapshot_workspace(&ws);
        assert_eq!(register_workspace_diff(&before, &after, &session, "generated"), 0);
        assert!(list_artifacts(&session, None, 50).0.is_empty());
        clear_artifacts(&session);
        let _ = std::fs::remove_dir_all(&ws);
    }

    /// 依赖/产物目录必须被跳过：否则 `pip install`/`npm install` 会把整棵依赖树灌进面板。
    #[test]
    fn snapshot_skips_dependency_and_build_dirs() {
        let ws = tmp_ws("skip");
        std::fs::write(ws.join("keep.py"), "x").unwrap();
        for dir in ["node_modules", "target", "__pycache__", ".git"] {
            let d = ws.join(dir).join("nested");
            std::fs::create_dir_all(&d).unwrap();
            std::fs::write(d.join("noise.bin"), "y").unwrap();
        }
        let snap = snapshot_workspace(&ws);
        assert!(!snap.truncated);
        assert_eq!(snap.files.len(), 1, "只应看到 keep.py: {:?}", snap.files.keys());
        assert!(snap.files.contains_key(&ws.join("keep.py")));
        let _ = std::fs::remove_dir_all(&ws);
    }

    /// 快照被截断时**绝不登记**：截断意味着有文件没被扫到，差分结果不可信。
    #[test]
    fn truncated_snapshot_never_registers() {
        let session = format!("ws.diff.trunc.{}", uuid::Uuid::new_v4());
        let mut before = WorkspaceSnapshot::default();
        let mut after = WorkspaceSnapshot::default();
        after.files.insert(std::path::PathBuf::from("C:/ws/new.pptx"), (1, 1));
        // 未截断时该文件会被登记（差分有效）
        before.truncated = false;
        assert_eq!(register_workspace_diff(&before, &after, &session, "generated"), 1);
        clear_artifacts(&session);
        // 任一侧截断即拒绝登记（宁可少登记，也不猜）
        before.truncated = true;
        assert_eq!(register_workspace_diff(&before, &after, &session, "generated"), 0);
        clear_artifacts(&session);
    }

    #[test]
    fn temp_helper_naming_is_recognized() {
        // 实测出现的两个真实噪音样本
        assert!(looks_like_temp_helper("_mk_xlsx.py"));
        assert!(looks_like_temp_helper("_locate.py"));
        assert!(looks_like_temp_helper("tmp_report.py"));
        assert!(looks_like_temp_helper("~$budget.xlsx"));
        assert!(looks_like_temp_helper("a.bak"));
        assert!(looks_like_temp_helper("notes~"));
        // 交付物必须**不被**误杀
        assert!(!looks_like_temp_helper("budget.xlsx"));
        assert!(!looks_like_temp_helper("report.docx"));
        assert!(!looks_like_temp_helper("presentation.pptx"));
        assert!(!looks_like_temp_helper("index.html"));
        assert!(!looks_like_temp_helper("competitive-brief.md"));
        // 交付物即便以 _ 开头，只要不是脚本，也放行（用户可能真要这个文件）
        assert!(!looks_like_temp_helper("_final.xlsx"));
        assert!(!looks_like_temp_helper("_notes.md"));
    }

    #[test]
    fn generated_temp_helper_is_not_registered_but_real_deliverable_is() {
        let dir = std::env::temp_dir().join(format!("marguerite-tmphelper-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let session = "tmp-helper-session";
        clear_artifacts(session);

        let helper = dir.join("_mk.py");
        let deliverable = dir.join("budget.xlsx");
        std::fs::write(&helper, b"print(1)").unwrap();
        std::fs::write(&deliverable, b"PK").unwrap();

        // 命令生成：临时脚本被过滤
        record_artifact_with_session(session, &helper.to_string_lossy(), GENERATED_KIND);
        // 命令生成：真实交付物照常登记
        record_artifact_with_session(session, &deliverable.to_string_lossy(), GENERATED_KIND);
        // ⚠️ 实测决定：**即便走 `write_file`（kind=created）也过滤** ——
        // 提示词要求「脚本写到工作区外」对模型无效，只拦 generated 等于没拦。
        record_artifact_with_session(session, &helper.to_string_lossy(), "created");

        let (items, _, _) = list_artifacts(session, None, 50);
        let paths: Vec<String> = items.iter().map(|a| a.path.clone()).collect();
        assert!(
            paths.contains(&deliverable.to_string_lossy().to_string()),
            "真实交付物必须登记: {paths:?}"
        );
        assert!(
            !paths.iter().any(|p| p.ends_with("_mk.py")),
            "临时辅助脚本不得进产物列表（含 created 路径）: {paths:?}"
        );

        clear_artifacts(session);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
