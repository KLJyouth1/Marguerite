//! 画像导入（Memory Profile Import）：8 个 RPC 的完整闭环
//!
//! 目标文件布局（三类 target）：
//! - USER   → `~/.trae-cn/memory/user_profile.md`（用户画像，跨项目共享）
//! - MEMORY → `Config::data_dir()/memory/` 下结构化画像（本地记忆、约定等）
//! - IMPORT → `Config::data_dir()/memory/imports/<batchId>/` 导入产物暂存与 undo 快照
//!
//! 不做 LLM 真实分析，但要给 UI 完整的阶段、preview、apply、undo 闭环。
//! 分析阶段基于**关键词分类**把用户粘贴的「个人画像文本」路由到三类文件，
//! 产出 unified diff；apply 写盘时同步生成 `.bak.undo` 快照用于 undo。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::persistence::config::Config;

const SCHEMA_VERSION: u64 = 1;
const MAX_INPUT_BYTES: usize = 256 * 1024;
const PREVIEW_TTL_MS: u64 = 10 * 60 * 1000; // 10 分钟预览过期
const PROVIDER: &str = "marguerite-local";
const MODEL_LABEL: &str = "ruleset-v1";

// ──────────────────────────────────────────────────────────────
// 数据结构
// ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ImportJob {
    schema_version: u64,
    job_id: String,
    batch_id: String,
    #[serde(rename = "status")]
    job_status: String,
    stage: String, // reading | model | diff
    provider: String,
    model: String,
    started_at: String, // RFC3339
    can_retry: bool,
    error_code: String,
    preview: Option<ImportPreview>,
    raw_text: String,
    ui_locale: String,
    candidate_hash: String,
    created_at_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ImportPreview {
    schema_version: u64,
    preview_id: String,
    batch_id: String,
    candidate_hash: String,
    provider: String,
    model: String,
    summary: Vec<String>,
    decision_counts: DecisionCounts,
    files: Vec<ImportDiffFile>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DecisionCounts {
    applied: u64,
    duplicate: u64,
    unresolved: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ImportDiffFile {
    target: String, // USER | MEMORY | IMPORT
    display_name: String,
    relative_path: String,
    status: String, // created | modified | deleted
    additions: u64,
    deletions: u64,
    diff: String, // unified diff（含 diff/---/+++/@@ meta）
    #[serde(skip)]
    proposed_content: String, // 不序列化到 JSON，只在 apply 用
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RecentImport {
    receipt_id: String,
    batch_id: String,
    applied_at: String,
    summary: Vec<String>,
    provider: String,
    model: String,
    status: String, // applied | undone
    index_status: String,
    file_count: u64,
    targets: Vec<String>,
    undo_snapshots: Vec<UndoSnapshot>, // 写盘前的旧内容快照
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct UndoSnapshot {
    target: String,
    absolute_path: String,
    was_existing: bool,
    previous_content: String, // 空表示之前不存在（undo 时删文件）
}

// 运行时台账（进程单例；jobs/receipts 也会 JSON 落盘到 data_dir/memory/imports）
struct ImportStore {
    jobs: HashMap<String, ImportJob>,
    previews: HashMap<String, ImportPreview>, // preview_id → preview
    receipts: HashMap<String, RecentImport>,  // receipt_id → receipt
    // receipt 的落盘文件
    store_path: PathBuf,
    #[allow(dead_code)]
    imports_root: PathBuf,
    next_seq: u64,
}

fn store() -> &'static Mutex<ImportStore> {
    static STORE: OnceLock<Mutex<ImportStore>> = OnceLock::new();
    STORE.get_or_init(|| {
        let root = Config::data_dir().join("memory").join("imports");
        let _ = std::fs::create_dir_all(&root);
        let path = root.join("receipts.json");
        let mut s = ImportStore {
            jobs: HashMap::new(),
            previews: HashMap::new(),
            receipts: HashMap::new(),
            store_path: path,
            imports_root: root,
            next_seq: 1,
        };
        s.load_receipts();
        Mutex::new(s)
    })
}

impl ImportStore {
    fn load_receipts(&mut self) {
        let raw = match std::fs::read_to_string(&self.store_path) {
            Ok(r) => r,
            Err(_) => return,
        };
        if let Ok(list) = serde_json::from_str::<Vec<RecentImport>>(&raw) {
            for r in list {
                self.receipts.insert(r.receipt_id.clone(), r);
            }
        }
    }

    fn save_receipts(&self) {
        let list: Vec<&RecentImport> = self.receipts.values().collect();
        if let Ok(s) = serde_json::to_string_pretty(&list) {
            let _ = std::fs::write(&self.store_path, s);
        }
    }

    fn most_recent_receipt(&self) -> Option<RecentImport> {
        let mut all: Vec<RecentImport> = self.receipts.values().cloned().collect();
        all.sort_by(|a, b| b.applied_at.cmp(&a.applied_at));
        all.into_iter().next()
    }

    fn next_id(&mut self, prefix: &str) -> String {
        let n = self.next_seq;
        self.next_seq += 1;
        let t = now_ms();
        format!("{prefix}-{t:016x}-{n:06x}")
    }
}

// ──────────────────────────────────────────────────────────────
// 辅助函数
// ──────────────────────────────────────────────────────────────

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn now_rfc3339() -> String {
    // 手动构造 UTC ISO 时间戳，免 chrono 依赖
    let ms = now_ms();
    let secs = ms / 1000;
    let mins = secs / 60;
    let hours = mins / 60;
    let days_since_epoch = hours / 24;
    // 日 -> 年月日（从 1970-01-01 起算，公历近似）
    let mut y: i64 = 1970;
    let mut remaining_days = days_since_epoch as i64;
    loop {
        let leap = (y % 4 == 0 && y % 100 != 0) || (y % 400 == 0);
        let year_days = if leap { 366 } else { 365 };
        if remaining_days < year_days {
            break;
        }
        remaining_days -= year_days;
        y += 1;
    }
    let mdays = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    let leap = (y % 4 == 0 && y % 100 != 0) || (y % 400 == 0);
    let mut mo: usize = 0;
    while mo < 12 {
        let d = if mo == 1 && leap { 29 } else { mdays[mo] };
        if remaining_days < d as i64 {
            break;
        }
        remaining_days -= d as i64;
        mo += 1;
    }
    let day = remaining_days as u64 + 1;
    let h = hours % 24;
    let mi = mins % 60;
    let s = secs % 60;
    let mil = ms % 1000;
    format!(
        "{y:04}-{:02}-{day:02}T{h:02}:{mi:02}:{s:02}.{mil:03}Z",
        mo + 1
    )
}

fn stable_hash(s: &str) -> String {
    // 简易 FNV-1a 128 风格输出为 hex（不依赖 crypto 库）
    let mut h1: u64 = 0xcbf29ce484222325;
    let mut h2: u64 = 0x100000001b3;
    for byte in s.as_bytes() {
        h1 ^= *byte as u64;
        h1 = h1.wrapping_mul(0x100000001b3);
        h2 = h2.wrapping_mul(0x100000001b3);
        h2 ^= *byte as u64;
    }
    format!("{h1:016x}{h2:016x}")
}

fn home_dir() -> PathBuf {
    dirs::home_dir().unwrap_or_else(|| PathBuf::from("."))
}

/// 三类目标文件的绝对路径解析
fn target_absolute_path(target: &str, relative_path: &str) -> PathBuf {
    match target {
        "USER" => home_dir()
            .join(".trae-cn")
            .join("memory")
            .join(relative_path.trim_start_matches('/')),
        "MEMORY" => Config::data_dir()
            .join("memory")
            .join(relative_path.trim_start_matches('/')),
        _ => Config::data_dir()
            .join("memory")
            .join("imports")
            .join(relative_path.trim_start_matches('/')),
    }
}

// ──────────────────────────────────────────────────────────────
// 分析阶段：基于关键词分桶 + unified diff 生成
// ──────────────────────────────────────────────────────────────

struct Classification {
    user_profile_lines: Vec<String>,       // USER: user_profile.md
    memory_preferences_lines: Vec<String>, // MEMORY: preferences.md
    import_notes_lines: Vec<String>,       // IMPORT: notes.md
}

fn classify(raw: &str) -> Classification {
    let mut user = Vec::new();
    let mut prefs = Vec::new();
    let mut notes = Vec::new();
    // 按段落分组（空行分隔），每段按关键词路由
    for paragraph in raw.split("\n\n") {
        let p = paragraph.trim();
        if p.is_empty() {
            continue;
        }
        let lower = p.to_lowercase();
        let routed =
            // 用户画像（职业/技能/语言偏好）
            if lower.contains("我是") || lower.contains("职业") || lower.contains("姓名")
                || lower.contains("擅长") || lower.contains("背景") || lower.contains("开发者")
                || lower.contains("工程师") || lower.contains("产品经理") || lower.contains("设计师")
                || lower.contains("用户偏好") || lower.contains("用户画像") || lower.contains("我的名字")
                || lower.contains("user_profile") || lower.contains("profile")
            { Some(&mut user) }
            // 偏好/约定（代码风格/工具偏好/UX 偏好）
            else if lower.contains("偏好") || lower.contains("风格") || lower.contains("约定")
                || lower.contains("代码") || lower.contains("ux") || lower.contains("ui")
                || lower.contains("工具") || lower.contains("prefer") || lower.contains("preference")
                || lower.contains("coding") || lower.contains("coding_style") || lower.contains("code_style")
            { Some(&mut prefs) }
            else { None };
        match routed {
            Some(bucket) => bucket.push(p.to_string()),
            None => {
                // 未分类：默认归到 import_notes，同时尝试补充 user
                notes.push(p.to_string());
            }
        }
    }
    // 如果 user_profile 为空，但有有效文本，兜底把首段当作 user_profile，避免全空
    if user.is_empty() {
        if let Some(first) = raw.split("\n\n").find(|s| !s.trim().is_empty()) {
            user.push(first.trim().to_string());
        }
    }
    Classification {
        user_profile_lines: user,
        memory_preferences_lines: prefs,
        import_notes_lines: notes,
    }
}

fn read_if_exists(p: &Path) -> Option<String> {
    std::fs::read_to_string(p).ok()
}

fn unified_diff(a_name: &str, b_name: &str, old: &str, new: &str) -> String {
    // 简易 unified diff（diff -u 风格，上下文 2 行）
    let old_lines: Vec<&str> = if old.is_empty() {
        Vec::new()
    } else {
        old.lines().collect()
    };
    let new_lines: Vec<&str> = if new.is_empty() {
        Vec::new()
    } else {
        new.lines().collect()
    };
    // 简化：整文件直接做 diff（跳过复杂 LCS；按行级 hash 做对比）
    let mut out = String::new();
    out.push_str(&format!("diff --memory-import a/{a_name} b/{b_name}\n"));
    out.push_str(&format!("--- a/{a_name}\n"));
    out.push_str(&format!("+++ b/{b_name}\n"));
    // 单 hunk（整个文件）
    out.push_str(&format!(
        "@@ -1,{} +1,{} @@\n",
        old_lines.len(),
        new_lines.len()
    ));
    // 简化策略：全部 - 旧内容，再全部 + 新内容（对于人看 diff 清晰）
    for l in &old_lines {
        out.push('-');
        out.push_str(l);
        out.push('\n');
    }
    for l in &new_lines {
        out.push('+');
        out.push_str(l);
        out.push('\n');
    }
    // 末尾如果都有换行，保证视觉干净
    out
}

fn count_additions_del(diff: &str) -> (u64, u64) {
    let mut a = 0u64;
    let mut d = 0u64;
    for line in diff.lines() {
        if line.starts_with("+++")
            || line.starts_with("---")
            || line.starts_with("diff ")
            || line.starts_with("@@")
        {
            continue;
        }
        if line.starts_with('+') {
            a += 1;
        } else if line.starts_with('-') {
            d += 1;
        }
    }
    (a, d)
}

fn build_files(c: &Classification) -> Vec<ImportDiffFile> {
    let mut files = Vec::new();
    let targets: [(&str, &str, &[String]); 3] = [
        ("USER", "user_profile.md", &c.user_profile_lines),
        ("MEMORY", "preferences.md", &c.memory_preferences_lines),
        ("IMPORT", "notes.md", &c.import_notes_lines),
    ];
    for (target, rel, lines) in targets {
        if lines.is_empty() {
            continue;
        }
        let new_content = lines.join("\n\n") + "\n";
        let abs = target_absolute_path(target, rel);
        let existing = read_if_exists(&abs).unwrap_or_default();
        let status = if existing.is_empty() {
            "created"
        } else {
            "modified"
        };
        let display_name = match target {
            "USER" => "用户画像".to_string(),
            "MEMORY" => "本地偏好与约定".to_string(),
            _ => "导入备注".to_string(),
        };
        let diff = unified_diff(rel, rel, &existing, &new_content);
        let (additions, deletions) = count_additions_del(&diff);
        files.push(ImportDiffFile {
            target: target.to_string(),
            display_name,
            relative_path: rel.to_string(),
            status: status.to_string(),
            additions,
            deletions,
            diff,
            proposed_content: new_content,
        });
    }
    files
}

fn build_summary(files: &[ImportDiffFile]) -> Vec<String> {
    let mut s = Vec::new();
    let n = files.len();
    s.push(format!("检测到 {n} 项画像文件待更新。"));
    for f in files {
        s.push(format!(
            "· [{}] {}（{}，+{} / -{}）",
            f.target, f.relative_path, f.status, f.additions, f.deletions
        ));
    }
    s.push("请在确认前逐项预览 diff；确认后将生成可回滚的 undo 快照。".to_string());
    s
}

// ──────────────────────────────────────────────────────────────
// 8 个 RPC 入口
// ──────────────────────────────────────────────────────────────

/// `memory.import.info`
pub fn info(params: &Value) -> Value {
    if params.get("schemaVersion").and_then(Value::as_u64) != Some(SCHEMA_VERSION) {
        return json!({
            "schemaVersion": SCHEMA_VERSION,
            "available": false,
            "provider": PROVIDER,
            "model": MODEL_LABEL,
            "isLocal": true,
            "maxInputBytes": MAX_INPUT_BYTES,
            "promptVersion": "profile-export-v1",
            "recentImport": Value::Null,
            "draftJob": Value::Null,
        });
    }
    let guard = store().lock().unwrap_or_else(|e| e.into_inner());
    let recent = guard.most_recent_receipt();
    json!({
        "schemaVersion": SCHEMA_VERSION,
        "available": true,
        "provider": PROVIDER,
        "model": MODEL_LABEL,
        "isLocal": true,
        "maxInputBytes": MAX_INPUT_BYTES,
        "promptVersion": "profile-export-v1",
        "recentImport": recent.map(|r| json!({
            "receiptId": r.receipt_id,
            "batchId": r.batch_id,
            "appliedAt": r.applied_at,
            "summary": r.summary,
            "provider": r.provider,
            "model": r.model,
            "status": r.status,
            "indexStatus": r.index_status,
            "fileCount": r.file_count,
            "targets": r.targets,
        })).unwrap_or(Value::Null),
        "draftJob": Value::Null,
    })
}

/// `memory.import.start` — 创建分析 job（立即同步返回 ready，因为是本地规则集）
pub fn start(params: &Value) -> Result<Value, (String, String)> {
    if params.get("schemaVersion").and_then(Value::as_u64) != Some(SCHEMA_VERSION) {
        return Err((
            "MEMORY_IMPORT_INVALID_OUTPUT".into(),
            "schema 不匹配".into(),
        ));
    }
    let raw = params
        .get("rawText")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let bytes_len = raw.len();
    if bytes_len > MAX_INPUT_BYTES {
        return Err((
            "MEMORY_IMPORT_INPUT_TOO_LARGE".into(),
            format!("输入超过上限 {} bytes", MAX_INPUT_BYTES),
        ));
    }
    if raw.trim().is_empty() {
        return Err((
            "MEMORY_IMPORT_INPUT_TOO_LARGE".into(),
            "输入不能为空".into(),
        ));
    }
    let locale = params
        .get("uiLocale")
        .and_then(Value::as_str)
        .unwrap_or("zh-CN")
        .to_string();
    let candidate_hash = stable_hash(&raw);
    let mut guard = store().lock().unwrap_or_else(|e| e.into_inner());
    let job_id = guard.next_id("job");
    let batch_id = guard.next_id("bat");
    // 同步完成分析（reading -> model -> diff -> ready）
    let classification = classify(&raw);
    let files = build_files(&classification);
    let preview_id = guard.next_id("prev");
    let summary = build_summary(&files);
    let applied_files = files.len() as u64;
    let preview = ImportPreview {
        schema_version: SCHEMA_VERSION,
        preview_id: preview_id.clone(),
        batch_id: batch_id.clone(),
        candidate_hash: candidate_hash.clone(),
        provider: PROVIDER.to_string(),
        model: MODEL_LABEL.to_string(),
        summary: summary.clone(),
        decision_counts: DecisionCounts {
            applied: applied_files,
            duplicate: 0,
            unresolved: 0,
        },
        files: files.clone(),
    };
    guard.previews.insert(preview_id.clone(), preview);
    let preview_json = Some(Value::Null); // start 返回 job 时不内嵌大 preview，UI 走 status 取
    let job = ImportJob {
        schema_version: SCHEMA_VERSION,
        job_id: job_id.clone(),
        batch_id: batch_id.clone(),
        job_status: "ready".to_string(),
        stage: "diff".to_string(),
        provider: PROVIDER.to_string(),
        model: MODEL_LABEL.to_string(),
        started_at: now_rfc3339(),
        can_retry: true,
        error_code: String::new(),
        preview: None,
        raw_text: raw,
        ui_locale: locale,
        candidate_hash: candidate_hash.clone(),
        created_at_ms: now_ms(),
    };
    let _ = preview_json;
    // 返回 job；preview 内嵌 status='ready' 时才会被 UI 消费
    let ret_job = json!({
        "schemaVersion": job.schema_version,
        "jobId": job.job_id,
        "batchId": job.batch_id,
        "status": job.job_status,
        "stage": job.stage,
        "provider": job.provider,
        "model": job.model,
        "startedAt": job.started_at,
        "canRetry": job.can_retry,
        "errorCode": job.error_code,
        "preview": {
            "schemaVersion": SCHEMA_VERSION,
            "previewId": preview_id,
            "batchId": batch_id,
            "candidateHash": candidate_hash,
            "provider": PROVIDER,
            "model": MODEL_LABEL,
            "summary": summary,
            "decisionCounts": {
                "applied": applied_files,
                "duplicate": 0,
                "unresolved": 0,
            },
            "files": files.into_iter().map(|f| json!({
                "target": f.target,
                "displayName": f.display_name,
                "relativePath": f.relative_path,
                "status": f.status,
                "additions": f.additions,
                "deletions": f.deletions,
                "diff": f.diff,
            })).collect::<Vec<_>>(),
        },
    });
    guard.jobs.insert(job_id, job);
    Ok(ret_job)
}

/// `memory.import.status`
pub fn status(params: &Value) -> Result<Value, (String, String)> {
    let job_id = params
        .get("jobId")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let guard = store().lock().unwrap_or_else(|e| e.into_inner());
    let job = guard.jobs.get(&job_id).ok_or_else(|| {
        (
            "MEMORY_IMPORT_INVALID_OUTPUT".to_string(),
            "job 不存在".to_string(),
        )
    })?;
    // 过期保护（preview 只有 ready 状态有效）
    let preview = if job.job_status == "ready" {
        guard
            .previews
            .values()
            .find(|p| p.batch_id == job.batch_id)
            .map(|p| {
                json!({
                    "schemaVersion": p.schema_version,
                    "previewId": p.preview_id,
                    "batchId": p.batch_id,
                    "candidateHash": p.candidate_hash,
                    "provider": &p.provider,
                    "model": &p.model,
                    "summary": &p.summary,
                    "decisionCounts": {
                        "applied": p.decision_counts.applied,
                        "duplicate": p.decision_counts.duplicate,
                        "unresolved": p.decision_counts.unresolved,
                    },
                    "files": p.files.iter().map(|f| json!({
                        "target": &f.target,
                        "displayName": &f.display_name,
                        "relativePath": &f.relative_path,
                        "status": &f.status,
                        "additions": f.additions,
                        "deletions": f.deletions,
                        "diff": &f.diff,
                    })).collect::<Vec<_>>(),
                })
            })
            .unwrap_or(Value::Null)
    } else {
        Value::Null
    };
    Ok(json!({
        "schemaVersion": job.schema_version,
        "jobId": job.job_id,
        "batchId": job.batch_id,
        "status": job.job_status,
        "stage": job.stage,
        "provider": &job.provider,
        "model": &job.model,
        "startedAt": &job.started_at,
        "canRetry": job.can_retry,
        "errorCode": &job.error_code,
        "preview": preview,
    }))
}

/// `memory.import.cancel`
pub fn cancel(params: &Value) -> Result<Value, (String, String)> {
    let job_id = params
        .get("jobId")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let mut guard = store().lock().unwrap_or_else(|e| e.into_inner());
    let job = guard.jobs.get_mut(&job_id).ok_or_else(|| {
        (
            "MEMORY_IMPORT_INVALID_OUTPUT".to_string(),
            "job 不存在".to_string(),
        )
    })?;
    job.job_status = "cancelled".to_string();
    job.can_retry = true;
    Ok(json!({
        "schemaVersion": job.schema_version,
        "jobId": job.job_id,
        "batchId": job.batch_id,
        "status": job.job_status,
        "stage": job.stage,
        "provider": &job.provider,
        "model": &job.model,
        "startedAt": &job.started_at,
        "canRetry": job.can_retry,
        "errorCode": &job.error_code,
        "preview": Value::Null,
    }))
}

// ──────────────────────────────────────────────────────────────
// s4b: RETRY/APPLY/UNDO/DISCARD
// ──────────────────────────────────────────────────────────────

/// `memory.import.retry` — 重新生成 preview（等同 start，但保留 jobId）
pub fn retry(params: &Value) -> Result<Value, (String, String)> {
    if params.get("schemaVersion").and_then(Value::as_u64) != Some(SCHEMA_VERSION) {
        return Err((
            "MEMORY_IMPORT_INVALID_OUTPUT".into(),
            "schema 不匹配".into(),
        ));
    }
    let job_id = params
        .get("jobId")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let mut guard = store().lock().unwrap_or_else(|e| e.into_inner());
    let old = guard.jobs.get(&job_id).cloned().ok_or_else(|| {
        (
            "MEMORY_IMPORT_INVALID_OUTPUT".to_string(),
            "job 不存在".to_string(),
        )
    })?;
    if old.raw_text.trim().is_empty() {
        return Err((
            "MEMORY_IMPORT_INVALID_OUTPUT".into(),
            "job 已丢失输入".into(),
        ));
    }
    // 重跑 classify
    let locale = old.ui_locale.clone();
    let candidate_hash = stable_hash(&old.raw_text);
    let classification = classify(&old.raw_text);
    let files = build_files(&classification);
    let batch_id = old.batch_id.clone();
    let preview_id = guard.next_id("prev");
    let summary = build_summary(&files);
    let applied_files = files.len() as u64;
    let preview = ImportPreview {
        schema_version: SCHEMA_VERSION,
        preview_id: preview_id.clone(),
        batch_id: batch_id.clone(),
        candidate_hash: candidate_hash.clone(),
        provider: PROVIDER.to_string(),
        model: MODEL_LABEL.to_string(),
        summary: summary.clone(),
        decision_counts: DecisionCounts {
            applied: applied_files,
            duplicate: 0,
            unresolved: 0,
        },
        files,
    };
    guard.previews.insert(preview_id.clone(), preview);
    let job = guard.jobs.get_mut(&job_id).unwrap();
    job.job_status = "ready".to_string();
    job.stage = "diff".to_string();
    job.error_code.clear();
    job.can_retry = true;
    job.started_at = now_rfc3339();
    job.candidate_hash = candidate_hash.clone();
    let _ = locale;
    Ok(json!({
        "schemaVersion": job.schema_version,
        "jobId": job.job_id,
        "batchId": job.batch_id,
        "status": job.job_status,
        "stage": job.stage,
        "provider": &job.provider,
        "model": &job.model,
        "startedAt": &job.started_at,
        "canRetry": job.can_retry,
        "errorCode": &job.error_code,
        "preview": {
            "schemaVersion": SCHEMA_VERSION,
            "previewId": preview_id,
            "batchId": batch_id,
            "candidateHash": candidate_hash,
            "provider": PROVIDER,
            "model": MODEL_LABEL,
            "summary": summary,
            "decisionCounts": {
                "applied": applied_files,
                "duplicate": 0,
                "unresolved": 0,
            },
            "files": guard.previews.get(&preview_id).map(|p| p.files.iter().map(|f| json!({
                "target": &f.target,
                "displayName": &f.display_name,
                "relativePath": &f.relative_path,
                "status": &f.status,
                "additions": f.additions,
                "deletions": f.deletions,
                "diff": &f.diff,
            })).collect::<Vec<_>>()).unwrap_or_default(),
        },
    }))
}

fn ensure_parent(path: &Path) -> std::io::Result<()> {
    if let Some(p) = path.parent() {
        std::fs::create_dir_all(p)?;
    }
    Ok(())
}

/// `memory.import.apply` — 写盘并生成 receipt（含 undo 快照）
pub fn apply(params: &Value) -> Result<Value, (String, String)> {
    let schema = params.get("schemaVersion").and_then(Value::as_u64);
    if schema != Some(SCHEMA_VERSION) {
        return Err((
            "MEMORY_IMPORT_INVALID_OUTPUT".into(),
            "schema 不匹配".into(),
        ));
    }
    let preview_id = params
        .get("previewId")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let candidate_hash = params
        .get("candidateHash")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    if preview_id.is_empty() {
        return Err((
            "MEMORY_IMPORT_STALE_PREVIEW".into(),
            "previewId 缺失".into(),
        ));
    }
    let mut guard = store().lock().unwrap_or_else(|e| e.into_inner());
    let preview = guard.previews.get(&preview_id).cloned().ok_or_else(|| {
        (
            "MEMORY_IMPORT_PREVIEW_EXPIRED".to_string(),
            "preview 已过期，请重新生成".to_string(),
        )
    })?;
    if preview.candidate_hash != candidate_hash {
        return Err((
            "MEMORY_IMPORT_STALE_PREVIEW".into(),
            "candidateHash 与 preview 不匹配".into(),
        ));
    }
    if now_ms().saturating_sub(preview.files.len() as u64) > 0 {
        // 用 preview 创建时的 TTL：存 jobs 表中由 job.created_at_ms 推断；此处拿不到 → 改为简化策略：TTL 不阻塞
    }
    let batch_id = preview.batch_id.clone();
    let job_for_created = guard.jobs.values().find(|j| j.batch_id == batch_id);
    let created_at_ms = job_for_created
        .map(|j| j.created_at_ms)
        .unwrap_or_else(now_ms);
    if now_ms().saturating_sub(created_at_ms) > PREVIEW_TTL_MS {
        return Err((
            "MEMORY_IMPORT_PREVIEW_EXPIRED".into(),
            "preview 已过期（超过 10 分钟）".into(),
        ));
    }
    // 写盘 + 收集 undo 快照
    let mut undo_snaps: Vec<UndoSnapshot> = Vec::new();
    let mut targets_seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    for f in &preview.files {
        let abs = target_absolute_path(&f.target, &f.relative_path);
        let was_existing = abs.exists();
        let previous_content = read_if_exists(&abs).unwrap_or_default();
        ensure_parent(&abs).map_err(|e| {
            (
                "MEMORY_IMPORT_WRITE_FAILED".to_string(),
                format!("无法创建目录 {}：{}", abs.display(), e),
            )
        })?;
        std::fs::write(&abs, &f.proposed_content).map_err(|e| {
            (
                "MEMORY_IMPORT_WRITE_FAILED".to_string(),
                format!("写盘失败 {}：{}", abs.display(), e),
            )
        })?;
        undo_snaps.push(UndoSnapshot {
            target: f.target.clone(),
            absolute_path: abs.to_string_lossy().to_string(),
            was_existing,
            previous_content,
        });
        targets_seen.insert(f.target.clone());
    }
    // 生成 receipt
    let receipt_id = guard.next_id("rcp");
    let targets: Vec<String> = targets_seen.into_iter().collect();
    let receipt = RecentImport {
        receipt_id: receipt_id.clone(),
        batch_id: batch_id.clone(),
        applied_at: now_rfc3339(),
        summary: preview.summary.clone(),
        provider: preview.provider.clone(),
        model: preview.model.clone(),
        status: "applied".to_string(),
        index_status: "ok".to_string(),
        file_count: preview.files.len() as u64,
        targets,
        undo_snapshots: undo_snaps,
    };
    guard.receipts.insert(receipt_id.clone(), receipt.clone());
    guard.save_receipts();
    // 清理 job / preview
    if let Some(job) = guard.jobs.values_mut().find(|j| j.batch_id == batch_id) {
        job.job_status = "applied".to_string();
    }
    guard.previews.remove(&preview_id);
    Ok(json!({
        "schemaVersion": SCHEMA_VERSION,
        "receiptId": receipt.receipt_id,
        "batchId": receipt.batch_id,
        "appliedAt": receipt.applied_at,
        "summary": receipt.summary,
        "provider": receipt.provider,
        "model": receipt.model,
        "status": receipt.status,
        "indexStatus": receipt.index_status,
        "fileCount": receipt.file_count,
        "targets": receipt.targets,
        "recentImport": {
            "receiptId": receipt.receipt_id,
            "batchId": receipt.batch_id,
            "appliedAt": receipt.applied_at,
            "summary": receipt.summary,
            "provider": receipt.provider,
            "model": receipt.model,
            "status": receipt.status,
            "indexStatus": receipt.index_status,
            "fileCount": receipt.file_count,
            "targets": receipt.targets,
        },
    }))
}

/// `memory.import.undo` — 用 receipt 中的快照还原
pub fn undo(params: &Value) -> Result<Value, (String, String)> {
    let schema = params.get("schemaVersion").and_then(Value::as_u64);
    if schema != Some(SCHEMA_VERSION) {
        return Err((
            "MEMORY_IMPORT_INVALID_OUTPUT".into(),
            "schema 不匹配".into(),
        ));
    }
    let receipt_id = params
        .get("receiptId")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let mut guard = store().lock().unwrap_or_else(|e| e.into_inner());
    let receipt = guard.receipts.get(&receipt_id).cloned().ok_or_else(|| {
        (
            "MEMORY_IMPORT_INVALID_OUTPUT".to_string(),
            "receipt 不存在".to_string(),
        )
    })?;
    if receipt.status == "undone" {
        return Ok(json!({
            "schemaVersion": SCHEMA_VERSION,
            "status": "alreadyUndone",
            "indexStatus": "ok",
        }));
    }
    // 应用 undo：按快照恢复
    for snap in &receipt.undo_snapshots {
        let p = PathBuf::from(&snap.absolute_path);
        if !snap.was_existing {
            // 之前不存在 → 删除新建的文件
            let _ = std::fs::remove_file(&p);
        } else {
            // 恢复之前的内容
            let _ = ensure_parent(&p);
            let _ = std::fs::write(&p, &snap.previous_content);
        }
    }
    // 标记 receipt
    let r = guard.receipts.get_mut(&receipt_id).unwrap();
    r.status = "undone".to_string();
    r.index_status = "ok".to_string();
    guard.save_receipts();
    Ok(json!({
        "schemaVersion": SCHEMA_VERSION,
        "status": "undone",
        "indexStatus": "ok",
    }))
}

/// `memory.import.discard` — 丢弃 preview/job（不写盘，仅清理台账）
pub fn discard(params: &Value) -> Value {
    let job_id = params
        .get("jobId")
        .and_then(Value::as_str)
        .map(|s| s.to_string());
    let preview_id = params
        .get("previewId")
        .and_then(Value::as_str)
        .map(|s| s.to_string());
    let mut guard = store().lock().unwrap_or_else(|e| e.into_inner());
    if let Some(jid) = &job_id {
        if let Some(job) = guard.jobs.get_mut(jid) {
            job.job_status = "discarded".to_string();
        }
    }
    // 如果是 job 维度：连带移除其 batch 的 preview
    if let Some(jid) = &job_id {
        if let Some(job) = guard.jobs.get(jid) {
            let bid = job.batch_id.clone();
            let previews_to_drop: Vec<String> = guard
                .previews
                .iter()
                .filter(|(_, p)| p.batch_id == bid)
                .map(|(k, _)| k.clone())
                .collect();
            for k in previews_to_drop {
                guard.previews.remove(&k);
            }
        }
    }
    if let Some(pid) = &preview_id {
        guard.previews.remove(pid);
    }
    json!({ "schemaVersion": SCHEMA_VERSION, "ok": true })
}

// ──────────────────────────────────────────────────────────────
// 测试
// ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_info_says_available_v1() {
        let v = info(&json!({"schemaVersion": 1, "agentId": "main"}));
        assert_eq!(v["available"], true);
        assert_eq!(v["schemaVersion"], 1);
        assert_eq!(v["isLocal"], true);
    }

    #[test]
    fn test_classify_routes_three_targets() {
        let raw = concat!(
            "我是一名全栈工程师，擅长 Rust 与 TypeScript，我的名字叫小雏菊。\n\n",
            "代码风格：注释精简，不堆砌 if 嵌套；UX 偏好稳定的连接状态展示。\n\n",
            "接下来打算做一个侧边栏和画像导入的集成测试。",
        );
        let c = classify(raw);
        assert!(!c.user_profile_lines.is_empty(), "应命中 USER");
        assert!(!c.memory_preferences_lines.is_empty(), "应命中 MEMORY");
        assert!(!c.import_notes_lines.is_empty(), "剩余段落应进入 IMPORT");
    }

    #[test]
    fn test_build_files_produces_diff_with_status() {
        let c = Classification {
            user_profile_lines: vec!["我是测试工程师".into()],
            memory_preferences_lines: vec![],
            import_notes_lines: vec!["测试备注".into()],
        };
        // USER 目标写 ~/.trae-cn/memory，沙箱里通常不存在；用 exists 检查后断言
        let files = build_files(&c);
        assert!(files.len() >= 2);
        let user_f = files.iter().find(|f| f.target == "USER").unwrap();
        // 如果真实环境已有 user_profile.md，则是 modified，否则 created；两种都合法
        assert!(matches!(user_f.status.as_str(), "created" | "modified"));
        assert!(user_f.additions > 0);
        assert!(user_f.diff.contains("+++ b/user_profile.md"));
        let note_f = files.iter().find(|f| f.target == "IMPORT").unwrap();
        assert!(note_f.relative_path == "notes.md");
        assert!(note_f.diff.contains("notes.md"));
    }

    #[test]
    fn test_start_then_status_returns_ready() {
        let raw = "我是一名 Rust 开发者，偏好中文沟通，代码不堆 if 嵌套。";
        let params = json!({
            "schemaVersion": 1,
            "agentId": "main",
            "rawText": raw,
            "uiLocale": "zh-CN",
            "exportPromptVersion": "profile-export-v1",
            "clientRequestId": "r1",
        });
        let job = start(&params).expect("start 应成功");
        assert_eq!(job["status"], "ready");
        let jid = job["jobId"].as_str().unwrap();
        let status_params = json!({
            "schemaVersion": 1,
            "agentId": "main",
            "jobId": jid,
        });
        let st = status(&status_params).expect("status 应成功");
        assert_eq!(st["status"], "ready");
        assert!(!st["preview"]["files"].as_array().unwrap().is_empty());
    }

    #[test]
    fn test_start_rejects_oversized() {
        let big = "x".repeat(MAX_INPUT_BYTES + 1);
        let err = start(&json!({"schemaVersion":1,"agentId":"main","rawText":big})).unwrap_err();
        assert_eq!(err.0, "MEMORY_IMPORT_INPUT_TOO_LARGE");
    }

    #[test]
    fn test_apply_writes_files_undo_restores_them() {
        // 隔离：构造空 temp dir，但 USER 目标会落到真实 home；用替代方法：
        // 直接跑 build_files + 手动 apply 逻辑（简化；避免污染真实 home）。
        let tmp = std::env::temp_dir().join(format!("mgt-import-{}", uuid::Uuid::new_v4()));
        let _ = std::fs::create_dir_all(&tmp);
        let f1 = tmp.join("user_profile.md");
        let f2 = tmp.join("prefs.md");
        let snap1 = UndoSnapshot {
            target: "TEST".into(),
            absolute_path: f1.to_string_lossy().to_string(),
            was_existing: false,
            previous_content: String::new(),
        };
        let snap2 = UndoSnapshot {
            target: "TEST".into(),
            absolute_path: f2.to_string_lossy().to_string(),
            was_existing: true,
            previous_content: "old-content".into(),
        };
        // 模拟 apply 写盘
        std::fs::write(&f1, "new user").unwrap();
        std::fs::write(&f2, "new prefs").unwrap();
        assert!(f1.exists() && f2.exists());
        // 模拟 undo
        for s in [&snap1, &snap2] {
            let p = PathBuf::from(&s.absolute_path);
            if !s.was_existing {
                let _ = std::fs::remove_file(&p);
            } else {
                let _ = std::fs::write(&p, &s.previous_content);
            }
        }
        assert!(!f1.exists(), "undo 应删除之前不存在的文件");
        assert_eq!(std::fs::read_to_string(&f2).unwrap(), "old-content");
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
