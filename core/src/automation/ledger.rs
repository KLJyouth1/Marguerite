//! 自动化记忆（ledger）
//!
//! 每个自动化拥有独立的 memory.md 风格记忆文件：
//! 路径 `~/.marguerite/automations/<id>/memory.md`
//!
//! 文件包含可读的运行摘要，以及一段 `json` 代码块保存结构化的运行记录，
//! 以保证 load/save 往返无损。

use std::path::PathBuf;

use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// 单次运行状态
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RunStatus {
    /// 运行中
    Running,
    /// 成功（审计通过 / 无异常）
    Success,
    /// 警告（审计发现问题但不致命；用于真实反映质量门禁而非假成功）
    Warning,
    /// 失败
    Failed,
    /// 超时
    Timeout,
}

impl RunStatus {
    /// 状态的人类可读名称
    pub fn as_str(&self) -> &'static str {
        match self {
            RunStatus::Running => "RUNNING",
            RunStatus::Success => "DONE",
            RunStatus::Warning => "WARN",
            RunStatus::Failed => "FAILED",
            RunStatus::Timeout => "TIMEOUT",
        }
    }
}

/// 单次运行记录
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunRecord {
    /// 运行 ID
    pub run_id: String,
    /// 开始时间
    pub started_at: DateTime<Utc>,
    /// 结束时间（未结束为 None）
    pub finished_at: Option<DateTime<Utc>>,
    /// 状态
    pub status: RunStatus,
    /// 结果摘要
    pub result: Option<String>,
    /// 错误信息
    pub error: Option<String>,
    /// 跨维度议题 / 发现
    pub notes: Vec<String>,
}

/// 自动化记忆台账
#[derive(Debug)]
pub struct AutomationLedger {
    memory_path: PathBuf,
    runs: Vec<RunRecord>,
}

impl AutomationLedger {
    /// 为指定自动化创建台账（确保目录存在）
    pub fn new(id: &str) -> Result<Self> {
        let home = dirs::home_dir().ok_or_else(|| anyhow::anyhow!("无法定位用户主目录"))?;
        let dir = home.join(".marguerite").join("automations").join(id);
        std::fs::create_dir_all(&dir)
            .map_err(|e| anyhow::anyhow!("创建自动化目录失败 {}: {e}", dir.display()))?;
        let memory_path = dir.join("memory.md");
        Ok(Self {
            memory_path,
            runs: Vec::new(),
        })
    }

    /// 从已有路径加载（用于测试注入临时路径）
    pub fn from_path(path: PathBuf) -> Self {
        Self {
            memory_path: path,
            runs: Vec::new(),
        }
    }

    /// 记忆文件路径
    pub fn memory_path(&self) -> &PathBuf {
        &self.memory_path
    }

    /// 运行记录列表
    pub fn runs(&self) -> &[RunRecord] {
        &self.runs
    }

    /// 按 run_id 取单条记录
    pub fn get(&self, run_id: &str) -> Option<&RunRecord> {
        self.runs.iter().find(|r| r.run_id == run_id)
    }

    /// 记录一次运行开始，返回 Running 态记录
    pub fn record_start(&mut self) -> RunRecord {
        let rec = RunRecord {
            run_id: uuid::Uuid::new_v4().to_string(),
            started_at: Utc::now(),
            finished_at: None,
            status: RunStatus::Running,
            result: None,
            error: None,
            notes: Vec::new(),
        };
        self.runs.push(rec.clone());
        rec
    }

    /// 结束一次运行（设置状态与结果）
    pub fn record_finish(&mut self, run_id: &str, status: RunStatus, result: Option<&str>) {
        if let Some(r) = self.runs.iter_mut().find(|r| r.run_id == run_id) {
            r.finished_at = Some(Utc::now());
            r.status = status;
            if let Some(res) = result {
                r.result = Some(res.to_string());
            }
        }
    }

    /// 追加一条备注
    pub fn append_note(&mut self, run_id: &str, note: &str) {
        if let Some(r) = self.runs.iter_mut().find(|r| r.run_id == run_id) {
            r.notes.push(note.to_string());
        }
    }

    /// 从磁盘加载历史记录（文件不存在视为空）
    pub fn load(&mut self) -> Result<()> {
        if !self.memory_path.exists() {
            self.runs.clear();
            return Ok(());
        }
        let text = std::fs::read_to_string(&self.memory_path)
            .map_err(|e| anyhow::anyhow!("读取记忆文件失败 {}: {e}", self.memory_path.display()))?;
        self.runs = parse_runs_from_md(&text).unwrap_or_default();
        Ok(())
    }

    /// 持久化到磁盘
    pub fn save(&self) -> Result<()> {
        let json = serde_json::to_string_pretty(&self.runs)
            .map_err(|e| anyhow::anyhow!("序列化运行记录失败: {e}"))?;
        let mut content = String::new();
        content.push_str("# automations memory\n\n");
        content.push_str("## 运行机制\n");
        content.push_str(&format!("- runs: {}\n", self.runs.len()));
        content.push_str(&format!(
            "- last: {}\n",
            self.runs
                .last()
                .map(|r| r.started_at.to_rfc3339())
                .unwrap_or_else(|| "-".to_string())
        ));
        content.push_str("\n## 轮次记录\n\n");
        for r in &self.runs {
            content.push_str(&format!(
                "### Run {} ({}) — {}\n",
                &r.run_id[..8.min(r.run_id.len())],
                r.started_at.to_rfc3339(),
                r.status.as_str()
            ));
            content.push_str(&format!(
                "- result: {}\n",
                r.result.as_deref().unwrap_or("-")
            ));
            if let Some(e) = &r.error {
                content.push_str(&format!("- error: {e}\n"));
            }
            if !r.notes.is_empty() {
                content.push_str("- notes:\n");
                for n in &r.notes {
                    content.push_str(&format!("  - {n}\n"));
                }
            }
            content.push('\n');
        }
        content.push_str("```json\n");
        content.push_str(&json);
        content.push_str("\n```\n");

        if let Some(parent) = self.memory_path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| anyhow::anyhow!("创建目录失败 {}: {e}", parent.display()))?;
        }
        std::fs::write(&self.memory_path, content)
            .map_err(|e| anyhow::anyhow!("写入记忆文件失败 {}: {e}", self.memory_path.display()))?;
        Ok(())
    }
}

/// 从 memory.md 中提取结构化运行记录（解析 ```json 代码块）
fn parse_runs_from_md(text: &str) -> Option<Vec<RunRecord>> {
    let start = text.find("```json")? + "```json".len();
    let rest = &text[start..];
    let end = rest.find("```")?;
    let json = rest[..end].trim();
    serde_json::from_str(json).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_ledger(name: &str) -> AutomationLedger {
        let dir =
            std::env::temp_dir().join(format!("marguerite_auto_{}_{}", name, std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        AutomationLedger::from_path(dir.join("memory.md"))
    }

    #[test]
    fn new_creates_dir_and_path() {
        let home = dirs::home_dir().unwrap();
        let _ = std::fs::remove_dir_all(home.join(".marguerite/automations/__probe__"));
        let ledger = AutomationLedger::new("__probe__").unwrap();
        assert!(
            ledger.memory_path().exists()
                || ledger
                    .memory_path()
                    .parent()
                    .map(|p| p.exists())
                    .unwrap_or(false)
        );
        let _ = std::fs::remove_dir_all(home.join(".marguerite/automations/__probe__"));
    }

    #[test]
    fn start_finish_note_and_roundtrip() {
        let mut ledger = temp_ledger("roundtrip");
        let rec = ledger.record_start();
        let rid = rec.run_id.clone();
        assert_eq!(rec.status, RunStatus::Running);
        ledger.record_finish(&rid, RunStatus::Success, Some("2 高危修复"));
        ledger.append_note(&rid, "跨维度议题：clippy 警告上升");
        ledger.save().unwrap();
        assert!(ledger.memory_path().exists());

        // 重新加载（直接挂载已保存文件；勿再调用 temp_ledger，否则会 remove_dir_all 清掉刚写的 memory.md）
        let mut loaded = AutomationLedger::from_path(ledger.memory_path().clone());
        loaded.load().unwrap();
        assert_eq!(loaded.runs().len(), 1);
        let r = loaded.get(&rid).unwrap();
        assert_eq!(r.status, RunStatus::Success);
        assert_eq!(r.result.as_deref(), Some("2 高危修复"));
        assert_eq!(r.notes.len(), 1);
        assert_eq!(r.notes[0], "跨维度议题：clippy 警告上升");

        let _ = std::fs::remove_dir_all(ledger.memory_path().parent().unwrap());
    }

    #[test]
    fn load_missing_file_is_empty() {
        let mut ledger = temp_ledger("missing");
        ledger.load().unwrap();
        assert!(ledger.runs().is_empty());
        let _ = std::fs::remove_dir_all(ledger.memory_path().parent().unwrap());
    }
}
