//! 命令审计（JSONL，v2 工作台）
//!
//! 落盘：`$workspace/.marguerite-audit/audit-YYYY-MM-DD.jsonl`。
//! 按日期文件名天然每日轮转；写入时顺带清理 30 天前的旧文件；
//! 审计采用轻量同步写，不阻塞沙箱主流程。

use std::fs::{self, OpenOptions};
use std::io::Write as _;
use std::path::{Path, PathBuf};

use chrono::Local;
use serde::{Deserialize, Serialize};

/// 单条审计记录
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditRecord {
    /// 时间戳（本地，RFC3339）
    pub ts: String,
    /// 追踪 ID（一次命令执行一个）
    pub trace_id: String,
    /// 决策：allow / deny / path_denied / timeout / error
    pub decision: String,
    /// 命令名（basename）
    pub command: String,
    /// 完整参数
    pub args: Vec<String>,
    /// 工作目录
    pub cwd: String,
    /// 退出码（-1 = 未执行/超时）
    pub exit_code: i32,
    /// 耗时（毫秒）
    pub duration_ms: u64,
    /// stdout 字节数（截断前）
    pub stdout_bytes: usize,
    /// stderr 字节数
    pub stderr_bytes: usize,
}

/// 审计落盘目录（`$workspace/.marguerite-audit/`）
pub fn default_audit_dir(workspace: &Path) -> PathBuf {
    workspace.join(".marguerite-audit")
}

/// JSONL 审计器
pub struct AuditSink {
    dir: PathBuf,
}

impl AuditSink {
    /// 创建审计器（目录不存在则惰性创建）
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    /// 今日审计文件路径
    fn daily_file(&self, day: &str) -> PathBuf {
        self.dir.join(format!("audit-{day}.jsonl"))
    }

    /// 追加一条记录（写失败静默忽略，审计不阻塞业务）
    pub fn record(&self, rec: &AuditRecord) {
        let _ = self.append(rec);
        self.prune();
    }

    fn append(&self, rec: &AuditRecord) -> std::io::Result<()> {
        fs::create_dir_all(&self.dir)?;
        let day = Local::now().format("%Y-%m-%d");
        let path = self.daily_file(&day.to_string());
        let mut f = OpenOptions::new().create(true).append(true).open(path)?;
        let line = serde_json::to_string(rec).unwrap_or_else(|_| "{}".to_string());
        writeln!(f, "{line}")?;
        Ok(())
    }

    /// 清理 30 天前的审计文件（保留最近 30 个日期文件）
    fn prune(&self) {
        let Ok(entries) = fs::read_dir(&self.dir) else {
            return;
        };
        let today = Local::now();
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            // 文件名格式 audit-YYYY-MM-DD.jsonl
            let Some(stem) = name
                .strip_prefix("audit-")
                .and_then(|s| s.strip_suffix(".jsonl"))
            else {
                continue;
            };
            let Ok(date) = chrono::NaiveDate::parse_from_str(stem, "%Y-%m-%d") else {
                continue;
            };
            let age_days = (today.date_naive() - date).num_days();
            if age_days > 30 {
                let _ = fs::remove_file(entry.path());
            }
        }
    }

    /// 读取全部审计记录（测试/诊断用，按文件顺序）
    pub fn read_all(&self) -> Vec<AuditRecord> {
        let mut records = Vec::new();
        let Ok(entries) = fs::read_dir(&self.dir) else {
            return records;
        };
        let mut files: Vec<PathBuf> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e == "jsonl"))
            .collect();
        files.sort();
        for f in files {
            if let Ok(content) = fs::read_to_string(&f) {
                for line in content.lines() {
                    if let Ok(rec) = serde_json::from_str::<AuditRecord>(line) {
                        records.push(rec);
                    }
                }
            }
        }
        records
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_record() -> AuditRecord {
        AuditRecord {
            ts: Local::now().to_rfc3339(),
            trace_id: "tr_1".to_string(),
            decision: "deny".to_string(),
            command: "rm".to_string(),
            args: vec!["-rf".to_string(), "/".to_string()],
            cwd: "/workspace".to_string(),
            exit_code: -1,
            duration_ms: 0,
            stdout_bytes: 0,
            stderr_bytes: 0,
        }
    }

    #[test]
    fn test_record_roundtrip() {
        let dir = std::env::temp_dir().join(format!("marguerite_audit_{}", uuid::Uuid::new_v4()));
        let sink = AuditSink::new(dir.clone());
        sink.record(&test_record());
        sink.record(&test_record());
        let all = sink.read_all();
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].command, "rm");
        assert_eq!(all[0].decision, "deny");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_daily_file_naming() {
        let dir = std::env::temp_dir().join(format!("marguerite_audit_n_{}", uuid::Uuid::new_v4()));
        let sink = AuditSink::new(dir.clone());
        let path = sink.daily_file("2026-08-04");
        assert_eq!(
            path.file_name().unwrap().to_str().unwrap(),
            "audit-2026-08-04.jsonl"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_prune_removes_old_files() {
        let dir = std::env::temp_dir().join(format!("marguerite_audit_p_{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        // 40 天前的旧文件（相对当前日期动态计算，避免硬编码日期随时间漂移过期）
        let now = Local::now();
        let old_day = (now - chrono::Duration::days(40)).format("%Y-%m-%d");
        let today = now.format("%Y-%m-%d");
        let old = dir.join(format!("audit-{old_day}.jsonl"));
        fs::write(&old, "{}").unwrap();
        // 今天的文件
        let new = dir.join(format!("audit-{today}.jsonl"));
        fs::write(&new, "{}").unwrap();
        let sink = AuditSink::new(dir.clone());
        sink.record(&test_record()); // 触发 prune
        assert!(!old.exists(), "40 天前的文件应被清理");
        assert!(new.exists(), "今天的文件应保留");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
