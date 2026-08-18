//! 自动化内核
//!
//! 提供 cron 定时任务管理能力：注册/启停自动化、按 cron 触发执行、
//! 维护每自动化的记忆台账，并向事件总线发布 `automation.*` 事件。

pub mod audit;
pub mod autofix;
pub mod builtin;
pub mod cron;
pub mod ledger;
pub mod long_running;

use std::collections::HashMap;
use std::path::PathBuf;

use anyhow::{anyhow, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

use crate::automation::cron::CronScheduler;
use crate::automation::ledger::{AutomationLedger, RunRecord, RunStatus};

/// 自动化事件信封（与事件总线 Envelope 对齐；events 模块就绪后可统一替换）
#[derive(Debug, Clone)]
pub struct Envelope {
    /// 事件类型，如 `automation.finished.<id>`
    pub r#type: String,
    /// 事件载荷
    pub payload: serde_json::Value,
    /// 时间戳
    pub ts: DateTime<Utc>,
}

/// 任务类型
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum JobKind {
    /// 质量审计轮转
    Audit,
    /// 生成报告
    Report,
    /// 记忆蒸馏
    Distill,
    /// 自定义命令
    Custom { command: String },
}

impl JobKind {
    /// 任务类型名称
    pub fn name(&self) -> &'static str {
        match self {
            JobKind::Audit => "Audit",
            JobKind::Report => "Report",
            JobKind::Distill => "Distill",
            JobKind::Custom { .. } => "Custom",
        }
    }
}

/// 单个自动化定义
#[derive(Debug, Clone)]
pub struct Automation {
    /// 唯一 ID
    pub id: String,
    /// 名称
    pub name: String,
    /// 标准 5 字段 cron 表达式
    pub cron_expr: String,
    /// 任务类型
    pub job: JobKind,
    /// 是否启用
    pub enabled: bool,
    /// 上次运行时间
    pub last_run: Option<DateTime<Utc>>,
    /// 下次预计运行时间
    pub next_run: Option<DateTime<Utc>>,
}

impl Automation {
    /// 构造自动化并校验 cron 最小间隔
    pub fn new(id: &str, name: &str, cron_expr: &str, job: JobKind) -> Result<Self> {
        CronScheduler::parse(cron_expr)?;
        Ok(Self {
            id: id.to_string(),
            name: name.to_string(),
            cron_expr: cron_expr.to_string(),
            job,
            enabled: true,
            last_run: None,
            next_run: None,
        })
    }
}

/// 一次触发的处理结果
#[derive(Debug, Clone)]
pub struct TriggerResult {
    /// 自动化 ID
    pub id: String,
    /// 是否实际触发执行
    pub triggered: bool,
    /// 产生的运行记录（若执行）
    pub run: Option<RunRecord>,
    /// 错误信息（解析/执行异常）
    pub error: Option<String>,
}

/// 自动化管理器
#[derive(Debug)]
pub struct AutomationManager {
    /// 已注册的自动化
    pub automations: HashMap<String, Automation>,
    /// 最近一次使用的调度器（供 introspection）
    pub cron: CronScheduler,
    /// 各自动化的记忆台账
    pub ledgers: HashMap<String, AutomationLedger>,
    /// 事件发布通道（可选）
    pub event_tx: Option<mpsc::Sender<Envelope>>,
    /// 台账根目录（可选；未设置时回退到用户主目录 ~/.marguerite/automations）
    pub ledger_root: Option<PathBuf>,
}

impl Default for AutomationManager {
    fn default() -> Self {
        Self::new()
    }
}

impl AutomationManager {
    /// 创建空管理器（默认挂载一个合法占位调度器）
    pub fn new() -> Self {
        Self {
            automations: HashMap::new(),
            cron: CronScheduler::parse("0 0 * * *").expect("内置默认 cron 表达式必须合法"),
            ledgers: HashMap::new(),
            event_tx: None,
            ledger_root: None,
        }
    }

    /// 绑定事件发布通道
    pub fn set_event_sender(&mut self, tx: mpsc::Sender<Envelope>) {
        self.event_tx = Some(tx);
    }

    /// 设置台账根目录（覆盖默认的用户主目录），用于测试或自定义数据位置
    pub fn set_ledger_root(&mut self, root: PathBuf) {
        self.ledger_root = Some(root);
    }

    /// 注册自动化（校验 cron 并计算下次运行）
    pub fn register(&mut self, auto: Automation) -> Result<()> {
        // 防御性再校验（构造阶段已校验）
        CronScheduler::parse(&auto.cron_expr)?;
        let next = CronScheduler::parse(&auto.cron_expr)
            .ok()
            .and_then(|s| s.next_after(Utc::now()));
        let mut a = auto;
        a.next_run = next;
        self.automations.insert(a.id.clone(), a);
        Ok(())
    }

    /// 注销自动化
    pub fn unregister(&mut self, id: &str) -> Result<()> {
        if self.automations.remove(id).is_some() {
            self.ledgers.remove(id);
            Ok(())
        } else {
            Err(anyhow!("未找到自动化: {id}"))
        }
    }

    /// 启用自动化
    pub fn enable(&mut self, id: &str) -> Result<()> {
        let a = self
            .automations
            .get_mut(id)
            .ok_or_else(|| anyhow!("未找到自动化: {id}"))?;
        a.enabled = true;
        a.next_run = CronScheduler::parse(&a.cron_expr)
            .ok()
            .and_then(|s| s.next_after(Utc::now()));
        Ok(())
    }

    /// 禁用自动化
    pub fn disable(&mut self, id: &str) -> Result<()> {
        let a = self
            .automations
            .get_mut(id)
            .ok_or_else(|| anyhow!("未找到自动化: {id}"))?;
        a.enabled = false;
        Ok(())
    }

    /// 列出所有自动化
    pub fn list(&self) -> Vec<&Automation> {
        self.automations.values().collect()
    }

    /// 按 ID 获取自动化
    pub fn get(&self, id: &str) -> Option<&Automation> {
        self.automations.get(id)
    }

    /// 计算某自动化的下次运行时间
    pub fn next_run_of(&self, id: &str) -> Option<DateTime<Utc>> {
        let a = self.automations.get(id)?;
        let base = a.last_run.unwrap_or_else(Utc::now);
        CronScheduler::parse(&a.cron_expr)
            .ok()
            .and_then(|s| s.next_after(base))
    }

    /// 检查并触发所有到点的自动化（使用当前时间）
    pub fn tick(&mut self) -> Vec<TriggerResult> {
        self.tick_at(Utc::now())
    }

    /// 在指定时刻检查并触发（便于测试）
    fn tick_at(&mut self, now: DateTime<Utc>) -> Vec<TriggerResult> {
        let ids: Vec<String> = self
            .automations
            .iter()
            .filter(|(_, a)| a.enabled)
            .map(|(id, _)| id.clone())
            .collect();
        let mut out = Vec::new();
        for id in ids {
            // 不可变借用计算是否到点，避免与后续 run 的可变借用冲突
            let (due, parse_err) = {
                let a = self.automations.get(&id).unwrap();
                match CronScheduler::parse(&a.cron_expr) {
                    Ok(s) => {
                        let d = s.should_run(now) && !same_minute(a.last_run, now);
                        (d, None)
                    }
                    Err(e) => (false, Some(e.to_string())),
                }
            };
            if let Some(e) = parse_err {
                out.push(TriggerResult {
                    id,
                    triggered: false,
                    run: None,
                    error: Some(format!("cron 解析失败: {e}")),
                });
                continue;
            }
            if due {
                match self.run(&id) {
                    Ok(rec) => out.push(TriggerResult {
                        id,
                        triggered: true,
                        run: Some(rec),
                        error: None,
                    }),
                    Err(e) => out.push(TriggerResult {
                        id,
                        triggered: true,
                        run: None,
                        error: Some(e.to_string()),
                    }),
                }
            }
        }
        out
    }

    /// 手动触发一次运行
    pub fn run(&mut self, id: &str) -> Result<RunRecord> {
        // 提取自动化信息（避免与 ledger/automations 可变借用冲突）
        let (cron_expr, name, job) = {
            let a = self
                .automations
                .get(id)
                .ok_or_else(|| anyhow!("未找到自动化: {id}"))?;
            (a.cron_expr.clone(), a.name.clone(), a.job.clone())
        };

        // 确保台账存在
        if !self.ledgers.contains_key(id) {
            let ledger = match &self.ledger_root {
                Some(root) => {
                    let dir = root.join(id);
                    std::fs::create_dir_all(&dir).map_err(|e| {
                        anyhow::anyhow!("创建自动化目录失败 {}: {e}", dir.display())
                    })?;
                    AutomationLedger::from_path(dir.join("memory.md"))
                }
                None => AutomationLedger::new(id)?,
            };
            self.ledgers.insert(id.to_string(), ledger);
        }

        let started_at = Utc::now();
        let rec = {
            let ledger = self.ledgers.get_mut(id).unwrap();
            ledger.record_start()
        };

        // 真实执行：Audit 委托 audit::run_audit（F4 修复，取代占位返回）；
        // Report / Distill / Custom 仍为占位（不在本轮 10 维审计范围内）。
        let (status, result) = match &job {
            JobKind::Audit => {
                let base = std::env::current_dir().unwrap_or_default();
                let (s, r) = audit::run_audit(id, &base);
                (s, Some(r))
            }
            JobKind::Report => (RunStatus::Success, Some("报告生成占位完成".to_string())),
            JobKind::Distill => (RunStatus::Success, Some("记忆蒸馏占位完成".to_string())),
            JobKind::Custom { command } => (
                RunStatus::Success,
                Some(format!("自定义命令占位执行: {command}")),
            ),
        };

        {
            let ledger = self.ledgers.get_mut(id).unwrap();
            ledger.record_finish(&rec.run_id, status, result.as_deref());
            ledger.append_note(&rec.run_id, &format!("触发：{}（{}）", name, job.name()));
            ledger.save()?;
        }

        // 更新运行时字段
        if let Some(a) = self.automations.get_mut(id) {
            a.last_run = Some(started_at);
            a.next_run = CronScheduler::parse(&cron_expr)
                .ok()
                .and_then(|s| s.next_after(started_at));
        }
        // 记录最近使用的调度器
        if let Ok(s) = CronScheduler::parse(&cron_expr) {
            self.cron = s;
        }

        // 发布事件（best-effort）
        if let Some(tx) = &self.event_tx {
            let _ = tx.try_send(Envelope {
                r#type: format!("automation.finished.{id}"),
                payload: serde_json::json!({
                    "run_id": rec.run_id,
                    "status": status.as_str(),
                }),
                ts: Utc::now(),
            });
        }

        let ledger = self.ledgers.get(id).unwrap();
        Ok(ledger.get(&rec.run_id).cloned().unwrap_or(rec))
    }
}

/// 判断两个时刻是否落在同一分钟（用于避免同一分钟内重复触发）
fn same_minute(last: Option<DateTime<Utc>>, now: DateTime<Utc>) -> bool {
    match last {
        Some(l) => l.timestamp() / 60 == now.timestamp() / 60,
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn auto(id: &str, cron: &str) -> Automation {
        Automation::new(id, id, cron, JobKind::Audit).unwrap()
    }

    /// 创建一个管理器并把台账根指向隔离的临时目录，避免写入真实用户主目录
    /// （本机用户主目录在文件安全层下不可写，会导致 os error 5）。
    fn mgr_with_temp_ledger(tag: &str) -> (AutomationManager, PathBuf) {
        let dir = std::env::temp_dir().join(format!(
            "marguerite_auto_mgr_{}_{}",
            tag,
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut mgr = AutomationManager::new();
        mgr.set_ledger_root(dir.clone());
        (mgr, dir)
    }

    #[test]
    fn new_register_list() {
        let mut mgr = AutomationManager::new();
        mgr.register(auto("a1", "0 */2 * * *")).unwrap();
        mgr.register(auto("a2", "0 0 * * *")).unwrap();
        assert_eq!(mgr.list().len(), 2);
        assert!(mgr.get("a1").is_some());
    }

    #[test]
    fn register_invalid_cron_rejected() {
        let mut mgr = AutomationManager::new();
        let bad = Automation {
            id: "bad".into(),
            name: "bad".into(),
            cron_expr: "* * * * *".into(),
            job: JobKind::Audit,
            enabled: true,
            last_run: None,
            next_run: None,
        };
        assert!(mgr.register(bad).is_err());
    }

    #[test]
    fn enable_disable_toggles() {
        let mut mgr = AutomationManager::new();
        mgr.register(auto("a1", "0 * * * *")).unwrap();
        mgr.disable("a1").unwrap();
        assert!(!mgr.get("a1").unwrap().enabled);
        mgr.enable("a1").unwrap();
        assert!(mgr.get("a1").unwrap().enabled);
        assert!(mgr.disable("nope").is_err());
    }

    #[test]
    fn unregister_removes() {
        let mut mgr = AutomationManager::new();
        mgr.register(auto("a1", "0 * * * *")).unwrap();
        mgr.unregister("a1").unwrap();
        assert!(mgr.get("a1").is_none());
        assert!(mgr.unregister("a1").is_err());
        assert!(mgr.run("a1").is_err());
    }

    #[test]
    fn run_produces_persisted_record_with_real_result() {
        let (mut mgr, root) = mgr_with_temp_ledger("a1");
        mgr.register(auto("a1", "0 * * * *")).unwrap();
        let rec = mgr.run("a1").unwrap();
        // 未知审计维度 → 真实返回 Failed（不再假 Success），但记录仍落盘
        assert_eq!(rec.status, RunStatus::Failed);
        assert!(rec.result.is_some());
        assert!(!rec.result.unwrap().is_empty());
        // 台账已落盘（隔离的临时目录）
        let path = root.join("a1/memory.md");
        assert!(path.exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn tick_fires_enabled_due_automation() {
        let mut mgr = AutomationManager::new();
        // 整点触发，last_run 设在两小时前，now 设为整点
        let mut a = auto("hourly", "0 * * * *");
        a.last_run = Some(Utc.with_ymd_and_hms(2026, 1, 1, 8, 0, 0).single().unwrap());
        mgr.register(a).unwrap();
        let now = Utc.with_ymd_and_hms(2026, 1, 1, 10, 0, 0).single().unwrap();
        let results = mgr.tick_at(now);
        assert_eq!(results.len(), 1);
        assert!(results[0].triggered);
        assert_eq!(results[0].id, "hourly");
        assert!(mgr.get("hourly").unwrap().last_run.is_some());
    }

    #[test]
    fn tick_skips_disabled() {
        let mut mgr = AutomationManager::new();
        let mut a = auto("hourly", "0 * * * *");
        a.last_run = Some(Utc.with_ymd_and_hms(2026, 1, 1, 8, 0, 0).single().unwrap());
        mgr.register(a).unwrap();
        mgr.disable("hourly").unwrap();
        let now = Utc.with_ymd_and_hms(2026, 1, 1, 10, 0, 0).single().unwrap();
        assert!(mgr.tick_at(now).is_empty());
    }

    #[test]
    fn tick_skips_already_run_same_minute() {
        let mut mgr = AutomationManager::new();
        let now = Utc.with_ymd_and_hms(2026, 1, 1, 10, 0, 0).single().unwrap();
        let mut a = auto("hourly", "0 * * * *");
        a.last_run = Some(now); // 同一分钟已运行
        mgr.register(a).unwrap();
        assert!(mgr.tick_at(now).is_empty());
    }

    #[test]
    fn custom_job_placeholder() {
        let (mut mgr, root) = mgr_with_temp_ledger("c1");
        let a = Automation::new(
            "c1",
            "c1",
            "0 * * * *",
            JobKind::Custom {
                command: "echo hi".into(),
            },
        )
        .unwrap();
        mgr.register(a).unwrap();
        let rec = mgr.run("c1").unwrap();
        assert!(rec.result.unwrap().contains("echo hi"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn run_publishes_event() {
        let (mut mgr, root) = mgr_with_temp_ledger("a1_evt");
        mgr.register(auto("a1", "0 * * * *")).unwrap();
        let (tx, mut rx) = tokio::sync::mpsc::channel(8);
        mgr.set_event_sender(tx);
        let rec = mgr.run("a1").unwrap();
        // run 是同步的，事件已通过 try_send 入队
        let env = rx.try_recv().expect("应收到事件");
        assert_eq!(env.r#type, "automation.finished.a1");
        assert_eq!(env.payload["run_id"], rec.run_id);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn register_builtins_count() {
        let mut mgr = AutomationManager::new();
        let n = builtin::register_builtins(&mut mgr).unwrap();
        assert_eq!(n, 10);
        assert_eq!(mgr.list().len(), 10);
        // build/startup 触发的两个默认禁用
        assert!(!mgr.get("audit-placeholders").unwrap().enabled);
        assert!(!mgr.get("audit-config-drift").unwrap().enabled);
        // 其余默认启用
        assert!(mgr.get("audit-code-quality").unwrap().enabled);
    }
}
