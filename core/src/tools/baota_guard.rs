//! 宝塔写入统一门禁：三闸收口 + 远端备份 + 失败回滚（需求 #6①）
//!
//! ## 修复的真实缺口
//!
//! `tools/baota.rs` 已实现三道独立闸门：
//! - `require_ready`（配置齐备 + dry_run fail-closed）
//! - `check_confirm`（严格布尔二次确认，fail-closed）
//! - `precheck_code`（tree-sitter 语法门禁）
//!
//! 但它们**分散在每个工具闭包里逐个手写调用**，且写入路径**没有任何备份/回滚**：
//! 一次 `SaveFileBody` 半成功（面板返回 `status=false`、连接中断、内容被截断）
//! 就会让远端文件处于被污染状态且无从恢复。
//!
//! 本模块补齐两件事，且全程 fail-closed：
//! 1. [`PrecheckGate`]：把三道闸门收口成**一次**调用，任一不过即拒绝。
//! 2. [`guarded_write`]：备份 → 写入 → 读回校验 → 失败自动回滚，备份按 FIFO 保留。
//!
//! ## 诚实边界（不夸大能力）
//! - 备份是**远端同目录旁路文件**（`<path>.mgbak.<epoch_ms>`），不是快照卷；
//!   面板磁盘写满或权限不足时备份会失败，此时**拒绝写入**而非"带病上传"。
//! - 备份索引落在本地台账（JSON）。台账被删除只会导致远端遗留孤儿备份文件，
//!   不会造成数据损坏；本模块不会去猜测未登记的远端文件。
//! - 读回校验若被 `BAOTA_SKIP_WRITE_VERIFY=1` 关闭，报告中 `verified` 为 `false`，
//!   绝不谎报"已校验"。
//!
//! ## 环境变量
//! - `BAOTA_BACKUP_KEEP`            备份保留份数（默认 5，0 表示不清理）
//! - `BAOTA_ALLOW_UNBACKED_WRITE`   默认 false：备份不可用时是否仍允许写入
//! - `BAOTA_SKIP_WRITE_VERIFY`      默认 false：跳过写入后的读回校验
//! - `BAOTA_CONFIRM_WRITE_FILE`     默认 false：写文件是否也要求 `confirm=true`
//! - `MARGUERITE_BAOTA_BACKUP_DB`   备份台账路径覆盖（测试用）

use std::collections::HashMap;
use std::path::PathBuf;

use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::baota::{
    check_confirm, env_bool, precheck_code, require_ready, BaotaClient, BaotaConfig,
};

/// 备份文件名中缀：便于人工在面板上一眼识别并批量清理。
pub(crate) const BACKUP_MARKER: &str = ".mgbak.";

/// 默认保留的备份份数。
const DEFAULT_KEEP: usize = 5;

// ─────────────────────────────────────────────────────────────────────────
// 远端文件抽象
// ─────────────────────────────────────────────────────────────────────────

/// 远端文件存取抽象。
///
/// 抽出 trait 的唯一目的是**让备份/回滚逻辑可在无面板环境下单测**——
/// 生产实现是 [`BaotaClient`]，测试实现是内存版 `MemFiles`。
#[async_trait]
pub(crate) trait RemoteFiles: Send + Sync {
    /// 读取远端文件。`Ok(None)` 表示**确认不存在**；无法确认时必须返回 `Err`。
    async fn read(&self, path: &str) -> Result<Option<String>>;

    /// 写入远端文件（覆盖）。
    async fn write(&self, path: &str, content: &str) -> Result<()>;

    /// 删除远端文件（仅用于清理过期备份，失败不影响主流程）。
    async fn remove(&self, path: &str) -> Result<()>;
}

// ─────────────────────────────────────────────────────────────────────────
// 预检收口
// ─────────────────────────────────────────────────────────────────────────

/// 门禁拒绝原因：与 `ToolResult::Error` 的 `code` 字段一一对应，便于上层分类处理。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GateReject {
    /// 错误码：`CONFIG` / `CONFIRM` / `SYNTAX`
    pub code: &'static str,
    /// 面向人的可执行提示
    pub message: String,
}

/// 统一预检闸门：一次调用跑完 `require_ready` + `check_confirm` + `precheck_code`。
#[derive(Debug, Clone, Copy)]
pub(crate) struct PrecheckGate {
    /// 是否为写操作（决定 dry_run 是否拦截）
    pub needs_write: bool,
    /// 是否强制要求 `confirm=true`
    pub require_confirm: bool,
    /// 是否跑语法门禁（非代码写入场景可关）
    pub check_syntax: bool,
}

impl PrecheckGate {
    /// 写文件场景的标准闸门配置。
    pub fn for_write_file() -> Self {
        Self {
            needs_write: true,
            require_confirm: env_bool("BAOTA_CONFIRM_WRITE_FILE", false),
            check_syntax: true,
        }
    }

    /// 依次跑三道闸门；任一不过即 fail-closed 返回。
    pub fn admit(
        &self,
        cfg: &BaotaConfig,
        args: &HashMap<String, String>,
        path: &str,
        content: &str,
    ) -> std::result::Result<(), GateReject> {
        require_ready(cfg, self.needs_write).map_err(|message| GateReject {
            code: "CONFIG",
            message,
        })?;
        if self.require_confirm {
            check_confirm(args, cfg).map_err(|message| GateReject {
                code: "CONFIRM",
                message,
            })?;
        }
        if self.check_syntax {
            precheck_code(path, content).map_err(|message| GateReject {
                code: "SYNTAX",
                message,
            })?;
        }
        Ok(())
    }
}

// ─────────────────────────────────────────────────────────────────────────
// 备份台账
// ─────────────────────────────────────────────────────────────────────────

/// 一条备份记录。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct BackupRecord {
    /// 被备份的目标路径
    pub target: String,
    /// 备份文件的远端路径
    pub backup: String,
    /// 备份创建时刻（epoch 毫秒）
    pub created_ms: u64,
    /// 原文字节数（用于人工核对备份是否被截断）
    pub bytes: usize,
}

/// 备份台账：本地 JSON，记录"哪些远端备份是本工具创建的"。
///
/// 之所以不去 `GetDir` 列目录做 FIFO，是因为面板各版本的目录列举返回结构不稳定，
/// 解析失败会静默漏删或误删用户文件。本地台账是**确定性**的：只清理自己登记过的。
#[derive(Debug, Clone)]
pub(crate) struct BackupLedger {
    path: PathBuf,
    records: Vec<BackupRecord>,
    keep: usize,
}

impl BackupLedger {
    /// 从磁盘载入；文件缺失或损坏时退化为空台账（不阻断写入主流程）。
    pub fn load(path: impl Into<PathBuf>, keep: usize) -> Self {
        let path = path.into();
        let records = std::fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str::<Vec<BackupRecord>>(&s).ok())
            .unwrap_or_default();
        Self {
            path,
            records,
            keep,
        }
    }

    /// 默认台账位置：`<data_dir>/baota_backups.json`，可被环境变量覆盖（测试用）。
    pub fn default_path() -> PathBuf {
        if let Ok(p) = std::env::var("MARGUERITE_BAOTA_BACKUP_DB") {
            if !p.trim().is_empty() {
                return PathBuf::from(p);
            }
        }
        crate::persistence::config::Config::data_dir().join("baota_backups.json")
    }

    /// 按环境变量决定保留份数的默认台账。
    pub fn default_ledger() -> Self {
        let keep = std::env::var("BAOTA_BACKUP_KEEP")
            .ok()
            .and_then(|v| v.trim().parse::<usize>().ok())
            .unwrap_or(DEFAULT_KEEP);
        Self::load(Self::default_path(), keep)
    }

    /// 登记一条备份，返回**因超出保留份数而应被删除**的旧备份（按同一 target FIFO）。
    pub fn record(&mut self, rec: BackupRecord) -> Vec<BackupRecord> {
        let target = rec.target.clone();
        self.records.push(rec);
        if self.keep == 0 {
            return Vec::new();
        }
        // 只在同一 target 维度做 FIFO：不同文件的备份互不挤占。
        let mut same: Vec<usize> = self
            .records
            .iter()
            .enumerate()
            .filter(|(_, r)| r.target == target)
            .map(|(i, _)| i)
            .collect();
        if same.len() <= self.keep {
            return Vec::new();
        }
        same.sort_by_key(|&i| self.records[i].created_ms);
        let drop_count = same.len() - self.keep;
        let drop_idx: Vec<usize> = same.into_iter().take(drop_count).collect();
        let evicted: Vec<BackupRecord> =
            drop_idx.iter().map(|&i| self.records[i].clone()).collect();
        let mut kept = Vec::with_capacity(self.records.len() - drop_count);
        for (i, r) in self.records.iter().enumerate() {
            if !drop_idx.contains(&i) {
                kept.push(r.clone());
            }
        }
        self.records = kept;
        evicted
    }

    /// 落盘。失败只回传错误，由调用方降级为告警——备份已在远端，台账写失败不损坏数据。
    pub fn save(&self) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&self.path, serde_json::to_vec_pretty(&self.records)?)?;
        Ok(())
    }

    /// 当前登记条数（测试与可观测性用）。
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.records.len()
    }
}

// ─────────────────────────────────────────────────────────────────────────
// 受保护写入
// ─────────────────────────────────────────────────────────────────────────

/// 一次受保护写入的结果收据。字段严格反映**实际发生了什么**，不做乐观标注。
#[derive(Debug, Clone, Default, Serialize)]
pub(crate) struct GuardedWriteReport {
    /// 备份文件远端路径；`None` 表示目标是新建文件（无原文可备）
    pub backup: Option<String>,
    /// 是否完成了写入后的读回校验（被 `BAOTA_SKIP_WRITE_VERIFY` 关闭时为 false）
    pub verified: bool,
    /// 读回内容与期望仅在换行风格上不同（已归一化后判定相等）
    pub newline_normalized: bool,
    /// 是否发生了回滚
    pub rolled_back: bool,
    /// 本次被 FIFO 清理掉的旧备份路径
    pub pruned: Vec<String>,
    /// 非致命告警（台账落盘失败、旧备份删除失败等）
    pub warnings: Vec<String>,
}

/// 生成备份路径：`<path>.mgbak.<epoch_ms>`。
fn backup_path_for(path: &str, ts_ms: u64) -> String {
    format!("{path}{BACKUP_MARKER}{ts_ms}")
}

/// 当前 epoch 毫秒。
fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// 换行归一化：仅用于判定"差异是否只在 CRLF/LF"。
fn normalize_newlines(s: &str) -> String {
    s.replace("\r\n", "\n").replace('\r', "\n")
}

/// 备份 → 写入 → 读回校验 → 失败回滚。
///
/// 失败语义（全部 fail-closed）：
/// - 原文读取失败（无法确认能否恢复）→ 拒绝写入，除非 `BAOTA_ALLOW_UNBACKED_WRITE=1`
/// - 备份写入失败或备份读回不一致 → 拒绝写入
/// - 主写入失败 → 用原文回滚，报告 `rolled_back=true`
/// - 读回校验不一致（且非纯换行差异）→ 用原文回滚，报告 `rolled_back=true`
pub(crate) async fn guarded_write<F: RemoteFiles + ?Sized>(
    files: &F,
    path: &str,
    content: &str,
    ledger: &mut BackupLedger,
) -> std::result::Result<GuardedWriteReport, String> {
    let mut report = GuardedWriteReport::default();
    let allow_unbacked = env_bool("BAOTA_ALLOW_UNBACKED_WRITE", false);

    // 1) 取原文。读不到 ≠ 不存在：无法确认时 fail-closed。
    let original: Option<String> = match files.read(path).await {
        Ok(v) => v,
        Err(e) => {
            if !allow_unbacked {
                return Err(format!(
                    "写入被拒绝：无法读取远端原文以创建备份（{e}）。\
                     这可能是权限不足或面板不可达；确认无需回滚保护时可设 \
                     BAOTA_ALLOW_UNBACKED_WRITE=1 后重试。"
                ));
            }
            report
                .warnings
                .push(format!("跳过备份（原文读取失败且已显式放行）：{e}"));
            None
        }
    };

    // 2) 有原文才需要备份；新建文件天然无可回滚对象。
    if let Some(orig) = original.as_deref() {
        let bpath = backup_path_for(path, now_ms());
        if let Err(e) = files.write(&bpath, orig).await {
            if !allow_unbacked {
                return Err(format!(
                    "写入被拒绝：备份文件 {bpath} 创建失败（{e}）。\
                     磁盘满或目录不可写时不应带病上传；\
                     确认无需回滚保护时可设 BAOTA_ALLOW_UNBACKED_WRITE=1。"
                ));
            }
            report
                .warnings
                .push(format!("备份创建失败但已显式放行：{e}"));
        } else {
            // 备份必须可读回且一致，否则等于没有备份。
            match files.read(&bpath).await {
                Ok(Some(back)) if normalize_newlines(&back) == normalize_newlines(orig) => {
                    report.backup = Some(bpath.clone());
                }
                other => {
                    if !allow_unbacked {
                        let detail = match other {
                            Ok(Some(_)) => "备份内容与原文不一致".to_string(),
                            Ok(None) => "备份文件写入后读不到".to_string(),
                            Err(e) => format!("备份读回失败：{e}"),
                        };
                        return Err(format!(
                            "写入被拒绝：{detail}（备份路径 {bpath}）。\
                             无法保证可回滚，已中止本次上传。"
                        ));
                    }
                    report
                        .warnings
                        .push(format!("备份校验未通过但已显式放行（{bpath}）"));
                }
            }
        }
    }

    // 3) 主写入。失败即回滚。
    if let Err(e) = files.write(path, content).await {
        if let Some(orig) = original.as_deref() {
            match files.write(path, orig).await {
                Ok(()) => report.rolled_back = true,
                Err(re) => {
                    return Err(format!(
                        "写入失败（{e}），且回滚同样失败（{re}）。\
                         远端文件可能处于不一致状态，请立即人工从备份 {} 恢复。",
                        report.backup.as_deref().unwrap_or("(无备份)")
                    ));
                }
            }
        }
        return Err(format!(
            "写入失败：{e}{}",
            if report.rolled_back {
                "；已用原文回滚，远端内容保持写入前状态"
            } else {
                ""
            }
        ));
    }

    // 4) 读回校验。
    if env_bool("BAOTA_SKIP_WRITE_VERIFY", false) {
        report
            .warnings
            .push("已跳过写入后读回校验（BAOTA_SKIP_WRITE_VERIFY=1）".into());
    } else {
        match files.read(path).await {
            Ok(Some(actual)) if actual == content => report.verified = true,
            Ok(Some(actual)) if normalize_newlines(&actual) == normalize_newlines(content) => {
                report.verified = true;
                report.newline_normalized = true;
            }
            other => {
                let detail = match other {
                    Ok(Some(a)) => {
                        format!("读回内容长度 {} 与期望 {} 不一致", a.len(), content.len())
                    }
                    Ok(None) => "写入后读不到该文件".to_string(),
                    Err(e) => format!("读回失败：{e}"),
                };
                if let Some(orig) = original.as_deref() {
                    match files.write(path, orig).await {
                        Ok(()) => report.rolled_back = true,
                        Err(re) => {
                            return Err(format!(
                                "写入后校验失败（{detail}），且回滚失败（{re}）。\
                                 请立即人工从备份 {} 恢复。",
                                report.backup.as_deref().unwrap_or("(无备份)")
                            ));
                        }
                    }
                }
                return Err(format!(
                    "写入后校验失败：{detail}{}",
                    if report.rolled_back {
                        "；已回滚到写入前内容"
                    } else {
                        ""
                    }
                ));
            }
        }
    }

    // 5) 登记备份 + FIFO 清理（best-effort，不影响本次写入成功的事实）。
    if let Some(bpath) = report.backup.clone() {
        let evicted = ledger.record(BackupRecord {
            target: path.to_string(),
            backup: bpath,
            created_ms: now_ms(),
            bytes: original.as_deref().map(str::len).unwrap_or(0),
        });
        for old in evicted {
            match files.remove(&old.backup).await {
                Ok(()) => report.pruned.push(old.backup),
                Err(e) => report
                    .warnings
                    .push(format!("旧备份 {} 清理失败：{e}", old.backup)),
            }
        }
        if let Err(e) = ledger.save() {
            report.warnings.push(format!("备份台账落盘失败：{e}"));
        }
    }

    Ok(report)
}

// ─────────────────────────────────────────────────────────────────────────
// 生产实现：宝塔面板
// ─────────────────────────────────────────────────────────────────────────

/// 解析 `GetFileBody` 响应。
///
/// fail-closed：只有能**确认**是"文件不存在"时才返回 `Ok(None)`；
/// 其余无法归类的失败一律 `Err`，交由上层拒绝写入。
fn parse_read_response(v: &Value) -> Result<Option<String>> {
    if let Some(data) = v.get("data").and_then(Value::as_str) {
        return Ok(Some(data.to_string()));
    }
    let status_false = matches!(v.get("status"), Some(Value::Bool(false)));
    let msg = v
        .get("msg")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    if status_false {
        let lower = msg.to_ascii_lowercase();
        let missing = msg.contains("不存在")
            || msg.contains("未找到")
            || lower.contains("not exist")
            || lower.contains("no such")
            || lower.contains("not found");
        if missing {
            return Ok(None);
        }
        return Err(anyhow::anyhow!("面板读取文件失败: {msg}"));
    }
    Err(anyhow::anyhow!("面板返回无法解析的读取结果: {v}"))
}

/// 解析 `SaveFileBody` / `DeleteFile` 响应。缺少 `status` 字段时按失败处理。
fn parse_write_response(v: &Value) -> Result<()> {
    match v.get("status") {
        Some(Value::Bool(true)) => Ok(()),
        Some(Value::Bool(false)) => Err(anyhow::anyhow!(
            "面板写入失败: {}",
            v.get("msg").and_then(Value::as_str).unwrap_or("(无消息)")
        )),
        _ => Err(anyhow::anyhow!(
            "面板返回无 status 字段，无法确认写入结果: {v}"
        )),
    }
}

#[async_trait]
impl RemoteFiles for BaotaClient {
    async fn read(&self, path: &str) -> Result<Option<String>> {
        let mut d = HashMap::new();
        d.insert("path".to_string(), path.to_string());
        let v = self.post("/files?action=GetFileBody", d).await?;
        parse_read_response(&v)
    }

    async fn write(&self, path: &str, content: &str) -> Result<()> {
        let mut d = HashMap::new();
        d.insert("path".to_string(), path.to_string());
        d.insert("content".to_string(), content.to_string());
        let v = self.post("/files?action=SaveFileBody", d).await?;
        parse_write_response(&v)
    }

    async fn remove(&self, path: &str) -> Result<()> {
        let mut d = HashMap::new();
        d.insert("path".to_string(), path.to_string());
        let v = self.post("/files?action=DeleteFile", d).await?;
        parse_write_response(&v)
    }
}

// ─────────────────────────────────────────────────────────────────────────
// 测试
// ─────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// 内存版远端：可注入"第 N 次写入失败"等故障，用于验证回滚路径。
    #[derive(Default)]
    struct MemFiles {
        files: Mutex<HashMap<String, String>>,
        /// 命中这些路径前缀的写入将失败
        fail_write_on: Mutex<Vec<String>>,
        /// 命中这些路径的读取将失败
        fail_read_on: Mutex<Vec<String>>,
        /// 写入后悄悄篡改内容（模拟半成功/截断）
        corrupt_on: Mutex<Vec<String>>,
    }

    impl MemFiles {
        fn with(entries: &[(&str, &str)]) -> Self {
            let m = Self::default();
            {
                let mut g = m.files.lock().unwrap();
                for (k, v) in entries {
                    g.insert((*k).to_string(), (*v).to_string());
                }
            }
            m
        }
        fn get(&self, path: &str) -> Option<String> {
            self.files.lock().unwrap().get(path).cloned()
        }
        fn count_backups(&self, target: &str) -> usize {
            self.files
                .lock()
                .unwrap()
                .keys()
                .filter(|k| k.starts_with(&format!("{target}{BACKUP_MARKER}")))
                .count()
        }
    }

    #[async_trait]
    impl RemoteFiles for MemFiles {
        async fn read(&self, path: &str) -> Result<Option<String>> {
            if self.fail_read_on.lock().unwrap().iter().any(|p| p == path) {
                return Err(anyhow::anyhow!("injected read failure"));
            }
            Ok(self.files.lock().unwrap().get(path).cloned())
        }
        async fn write(&self, path: &str, content: &str) -> Result<()> {
            if self.fail_write_on.lock().unwrap().iter().any(|p| p == path) {
                return Err(anyhow::anyhow!("injected write failure"));
            }
            // corrupt_on 一次性生效：模拟一次半成功/截断写入后移除标记，
            // 使后续回滚写入能正常落盘（否则回滚原文也会被再次截断）。
            let stored = {
                let mut corrupt = self.corrupt_on.lock().unwrap();
                if corrupt.iter().any(|p| p == path) {
                    corrupt.retain(|p| p != path);
                    format!("{}<<TRUNCATED>>", &content[..content.len() / 2])
                } else {
                    content.to_string()
                }
            };
            self.files.lock().unwrap().insert(path.to_string(), stored);
            Ok(())
        }
        async fn remove(&self, path: &str) -> Result<()> {
            self.files.lock().unwrap().remove(path);
            Ok(())
        }
    }

    fn tmp_ledger(keep: usize) -> BackupLedger {
        let p = std::env::temp_dir().join(format!(
            "mg-baota-ledger-{}-{}.json",
            std::process::id(),
            now_ms()
        ));
        let _ = std::fs::remove_file(&p);
        BackupLedger::load(p, keep)
    }

    #[tokio::test]
    async fn write_creates_backup_and_verifies() {
        let files = MemFiles::with(&[("/www/app.py", "print(1)\n")]);
        let mut ledger = tmp_ledger(5);
        let rep = guarded_write(&files, "/www/app.py", "print(2)\n", &mut ledger)
            .await
            .expect("写入应成功");
        assert!(rep.verified, "应完成读回校验");
        assert!(!rep.rolled_back);
        let backup = rep.backup.expect("应创建备份");
        assert_eq!(
            files.get(&backup).as_deref(),
            Some("print(1)\n"),
            "备份应存原文"
        );
        assert_eq!(files.get("/www/app.py").as_deref(), Some("print(2)\n"));
    }

    #[tokio::test]
    async fn new_file_has_no_backup_but_still_verified() {
        let files = MemFiles::default();
        let mut ledger = tmp_ledger(5);
        let rep = guarded_write(&files, "/www/new.py", "x = 1\n", &mut ledger)
            .await
            .expect("新建文件应成功");
        assert!(rep.backup.is_none(), "新建文件无原文可备份");
        assert!(rep.verified);
    }

    #[tokio::test]
    async fn write_failure_rolls_back_to_original() {
        let files = MemFiles::with(&[("/www/app.py", "ORIGINAL")]);
        files
            .fail_write_on
            .lock()
            .unwrap()
            .push("/www/app.py".to_string());
        let mut ledger = tmp_ledger(5);
        let e = guarded_write(&files, "/www/app.py", "NEW", &mut ledger)
            .await
            .expect_err("注入写失败后应返回错误");
        assert!(e.contains("写入失败"), "错误应说明写入失败: {e}");
        assert_eq!(
            files.get("/www/app.py").as_deref(),
            Some("ORIGINAL"),
            "回滚后远端应保持原文"
        );
    }

    #[tokio::test]
    async fn corrupted_write_is_detected_and_rolled_back() {
        let files = MemFiles::with(&[("/www/app.py", "ORIGINAL-CONTENT")]);
        files
            .corrupt_on
            .lock()
            .unwrap()
            .push("/www/app.py".to_string());
        let mut ledger = tmp_ledger(5);
        let e = guarded_write(&files, "/www/app.py", "BRAND-NEW-CONTENT", &mut ledger)
            .await
            .expect_err("读回不一致应报错");
        assert!(e.contains("校验失败"), "应报校验失败: {e}");
        assert!(e.contains("已回滚"), "应说明已回滚: {e}");
        assert_eq!(
            files.get("/www/app.py").as_deref(),
            Some("ORIGINAL-CONTENT")
        );
    }

    #[tokio::test]
    async fn unreadable_original_blocks_write_fail_closed() {
        let files = MemFiles::with(&[("/www/app.py", "ORIGINAL")]);
        files
            .fail_read_on
            .lock()
            .unwrap()
            .push("/www/app.py".to_string());
        let mut ledger = tmp_ledger(5);
        let e = guarded_write(&files, "/www/app.py", "NEW", &mut ledger)
            .await
            .expect_err("读不到原文时必须拒绝写入");
        assert!(e.contains("无法读取远端原文"), "{e}");
        assert_eq!(
            files.get("/www/app.py").as_deref(),
            Some("ORIGINAL"),
            "拒绝写入后远端不应被改动"
        );
    }

    #[tokio::test]
    async fn backups_are_fifo_pruned_per_target() {
        let files = MemFiles::with(&[("/www/a.py", "v0")]);
        let mut ledger = tmp_ledger(2);
        for i in 1..=4 {
            // now_ms 分辨率为毫秒，连续写入可能同毫秒；显式错开保证 FIFO 顺序确定。
            tokio::time::sleep(std::time::Duration::from_millis(2)).await;
            guarded_write(&files, "/www/a.py", &format!("v{i}"), &mut ledger)
                .await
                .expect("写入应成功");
        }
        assert_eq!(
            files.count_backups("/www/a.py"),
            2,
            "保留份数为 2 时，远端只应剩 2 份备份"
        );
        assert_eq!(ledger.len(), 2, "台账也应同步收敛到 2 条");
    }

    #[test]
    fn parse_read_distinguishes_missing_from_error() {
        let missing = serde_json::json!({"status": false, "msg": "文件不存在"});
        assert!(matches!(parse_read_response(&missing), Ok(None)));

        let denied = serde_json::json!({"status": false, "msg": "权限不足"});
        assert!(
            parse_read_response(&denied).is_err(),
            "权限类失败不得被当成『文件不存在』"
        );

        let weird = serde_json::json!({"whatever": 1});
        assert!(
            parse_read_response(&weird).is_err(),
            "无法解析必须 fail-closed"
        );
    }

    #[test]
    fn parse_write_requires_explicit_status_true() {
        assert!(parse_write_response(&serde_json::json!({"status": true})).is_ok());
        assert!(parse_write_response(&serde_json::json!({"status": false, "msg": "x"})).is_err());
        assert!(
            parse_write_response(&serde_json::json!({"msg": "ok"})).is_err(),
            "缺 status 必须视为失败"
        );
    }

    #[test]
    fn ledger_round_trips_through_disk() {
        let path = std::env::temp_dir().join(format!("mg-ledger-rt-{}.json", now_ms()));
        let _ = std::fs::remove_file(&path);
        let mut l = BackupLedger::load(&path, 5);
        l.record(BackupRecord {
            target: "/t".into(),
            backup: "/t.mgbak.1".into(),
            created_ms: 1,
            bytes: 3,
        });
        l.save().expect("台账应可落盘");
        let reloaded = BackupLedger::load(&path, 5);
        assert_eq!(reloaded.len(), 1);
        let _ = std::fs::remove_file(&path);
    }
}
