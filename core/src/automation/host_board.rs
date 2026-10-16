//! 任务看板 Host 权威账本（上游 task-board 二开落地，Track 4）。
//!
//! 对齐 DeepSeek Harness 任务看板的核心语义：
//! - Host 权威：账本持久化于 Host 侧（JSON 落盘），浏览器关闭后仍由 Host 执行与结算；
//! - 多列看板：任务按 column 归类，可 move_task 流转；
//! - 钉住：任务可钉住 workspace / agent 预设 / 权限；
//! - 5 段 cron + 不补跑：next_run 由 CronScheduler::next_after 计算（严格前向），
//!   错过触发点不补跑。
//!
//! 纯逻辑 + JSON 持久化，离线可测；真实执行器经 Host 运行时消费 due()/advance()。

use std::path::{Path, PathBuf};

use anyhow::{anyhow, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::automation::cron::CronScheduler;
use crate::persistence::config::Config;

/// 看板任务
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BoardTask {
    pub id: String,
    pub title: String,
    /// 所属列（如 todo / in_progress / done，自由命名）
    pub column: String,
    /// 5 段 cron 表达式（可选；None = 不自动调度）
    pub cron_expr: Option<String>,
    /// 钉住的工作区路径
    pub pinned_workspace: Option<String>,
    /// 钉住的 agent 预设 id
    pub pinned_agent: Option<String>,
    /// 钉住的权限（自由字符串列表）
    pub pinned_permissions: Vec<String>,
    pub enabled: bool,
    pub last_run_ms: Option<i64>,
    pub next_run_ms: Option<i64>,
}

/// Host 权威账本
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct HostBoard {
    tasks: Vec<BoardTask>,
}

impl HostBoard {
    fn path() -> PathBuf {
        Config::data_dir().join("board.json")
    }

    pub fn load() -> Self {
        Self::load_from(&Self::path())
    }

    pub fn load_from(path: &Path) -> Self {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|raw| serde_json::from_str(&raw).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) -> Result<()> {
        self.save_to(&Self::path())
    }

    pub fn save_to(&self, path: &Path) -> Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(path, serde_json::to_string_pretty(self)?)?;
        Ok(())
    }

    /// upsert 任务（id 为空则新建）；按 now 计算 next_run（仅未来触发）。返回 id。
    pub fn upsert(&mut self, mut task: BoardTask, now: DateTime<Utc>) -> String {
        if task.id.trim().is_empty() {
            task.id = format!("task-{}", Utc::now().timestamp_millis());
        }
        task.next_run_ms = task
            .cron_expr
            .as_ref()
            .and_then(|e| CronScheduler::parse(e).ok())
            .and_then(|s| s.next_after(now))
            .map(|t| t.timestamp_millis());
        if let Some(idx) = self.tasks.iter().position(|t| t.id == task.id) {
            self.tasks[idx] = task.clone();
        } else {
            self.tasks.push(task.clone());
        }
        task.id
    }

    pub fn move_task(&mut self, id: &str, column: &str) -> Result<()> {
        let t = self
            .tasks
            .iter_mut()
            .find(|t| t.id == id)
            .ok_or_else(|| anyhow!("未找到任务: {id}"))?;
        t.column = column.to_string();
        Ok(())
    }

    pub fn toggle(&mut self, id: &str, enabled: bool) -> Result<()> {
        let t = self
            .tasks
            .iter_mut()
            .find(|t| t.id == id)
            .ok_or_else(|| anyhow!("未找到任务: {id}"))?;
        t.enabled = enabled;
        Ok(())
    }

    pub fn get(&self, id: &str) -> Option<&BoardTask> {
        self.tasks.iter().find(|t| t.id == id)
    }

    pub fn list(&self) -> &[BoardTask] {
        &self.tasks
    }

    pub fn list_column(&self, column: &str) -> Vec<&BoardTask> {
        self.tasks.iter().filter(|t| t.column == column).collect()
    }

    pub fn len(&self) -> usize {
        self.tasks.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tasks.is_empty()
    }

    /// 到点任务（enabled 且 next_run <= now；严格不补跑——错过即顺延到下一个未来点）。
    pub fn due(&self, now: DateTime<Utc>) -> Vec<&BoardTask> {
        let now_ms = now.timestamp_millis();
        self.tasks
            .iter()
            .filter(|t| t.enabled && t.next_run_ms.map(|n| n <= now_ms).unwrap_or(false))
            .collect()
    }

    /// 触发后推进：last_run=now，next_run=下一个未来触发点（严格前向，不补跑）。
    pub fn advance(&mut self, id: &str, now: DateTime<Utc>) -> Result<Option<i64>> {
        let t = self
            .tasks
            .iter_mut()
            .find(|t| t.id == id)
            .ok_or_else(|| anyhow!("未找到任务: {id}"))?;
        t.last_run_ms = Some(now.timestamp_millis());
        t.next_run_ms = t
            .cron_expr
            .as_ref()
            .and_then(|e| CronScheduler::parse(e).ok())
            .and_then(|s| s.next_after(now))
            .map(|x| x.timestamp_millis());
        Ok(t.next_run_ms)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn task(title: &str, column: &str, cron: Option<&str>) -> BoardTask {
        BoardTask {
            id: String::new(),
            title: title.to_string(),
            column: column.to_string(),
            cron_expr: cron.map(|s| s.to_string()),
            pinned_workspace: Some("/ws".to_string()),
            pinned_agent: Some("main".to_string()),
            pinned_permissions: vec!["workspace-write".to_string()],
            enabled: true,
            last_run_ms: None,
            next_run_ms: None,
        }
    }

    #[test]
    fn upsert_and_move_and_list_column() {
        let now = Utc.with_ymd_and_hms(2026, 1, 1, 9, 0, 0).single().unwrap();
        let mut b = HostBoard::default();
        let id = b.upsert(task("重构部署", "todo", None), now);
        assert!(!id.is_empty());
        assert_eq!(b.list_column("todo").len(), 1);
        b.move_task(&id, "in_progress").unwrap();
        assert!(b.list_column("todo").is_empty());
        assert_eq!(b.list_column("in_progress").len(), 1);
        assert!(b.move_task("ghost", "done").is_err());
    }

    #[test]
    fn upsert_is_idempotent_by_id() {
        let now = Utc.with_ymd_and_hms(2026, 1, 1, 9, 0, 0).single().unwrap();
        let mut b = HostBoard::default();
        let id = b.upsert(task("A", "todo", None), now);
        let mut t2 = task("A2", "done", None);
        t2.id = id.clone();
        b.upsert(t2, now);
        assert_eq!(b.len(), 1, "同 id 应更新而非新增");
        assert_eq!(b.get(&id).unwrap().title, "A2");
        assert_eq!(b.get(&id).unwrap().column, "done");
    }

    #[test]
    fn next_run_is_strictly_forward_no_backfill() {
        // cron 0 3 * * *（每日 3 点）；now 为 9 点 → 应排到次日 3 点，而非错过的当日 3 点
        let now = Utc.with_ymd_and_hms(2026, 1, 1, 9, 0, 0).single().unwrap();
        let next_day_3 = Utc.with_ymd_and_hms(2026, 1, 2, 3, 0, 0).single().unwrap();
        let mut b = HostBoard::default();
        let id = b.upsert(task("凌晨任务", "todo", Some("0 3 * * *")), now);
        assert_eq!(
            b.get(&id).unwrap().next_run_ms,
            Some(next_day_3.timestamp_millis())
        );
    }

    #[test]
    fn due_and_advance() {
        let t0 = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).single().unwrap();
        let t10 = Utc.with_ymd_and_hms(2026, 1, 1, 0, 10, 0).single().unwrap();
        let t20 = Utc.with_ymd_and_hms(2026, 1, 1, 0, 20, 0).single().unwrap();
        let mut b = HostBoard::default();
        let id = b.upsert(task("每10分钟", "todo", Some("*/10 * * * *")), t0);
        assert!(b.due(t0).is_empty(), "t0 未到点");
        assert_eq!(b.due(t10).len(), 1, "t10 到点");
        b.advance(&id, t10).unwrap();
        assert_eq!(
            b.get(&id).unwrap().last_run_ms,
            Some(t10.timestamp_millis())
        );
        assert_eq!(
            b.get(&id).unwrap().next_run_ms,
            Some(t20.timestamp_millis())
        );
        assert!(b.due(t10).is_empty(), "推进后 t10 不再到点");
    }

    #[test]
    fn toggle_disables_due() {
        let t0 = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).single().unwrap();
        let t10 = Utc.with_ymd_and_hms(2026, 1, 1, 0, 10, 0).single().unwrap();
        let mut b = HostBoard::default();
        let id = b.upsert(task("x", "todo", Some("*/10 * * * *")), t0);
        assert_eq!(b.due(t10).len(), 1);
        b.toggle(&id, false).unwrap();
        assert!(b.due(t10).is_empty());
    }

    #[test]
    fn save_load_roundtrip() {
        let now = Utc.with_ymd_and_hms(2026, 1, 1, 9, 0, 0).single().unwrap();
        let tmp = std::env::temp_dir().join(format!(
            "mrg_board_test_{}.json",
            Utc::now().timestamp_millis()
        ));
        let id;
        {
            let mut b = HostBoard::default();
            id = b.upsert(task("持久化", "todo", Some("0 3 * * *")), now);
            b.save_to(&tmp).unwrap();
        }
        {
            let b = HostBoard::load_from(&tmp);
            assert_eq!(b.len(), 1);
            let t = b.get(&id).unwrap();
            assert_eq!(t.title, "持久化");
            assert_eq!(t.pinned_agent.as_deref(), Some("main"));
            assert_eq!(t.pinned_permissions, vec!["workspace-write".to_string()]);
        }
        let _ = std::fs::remove_file(&tmp);
    }
}
