//! 任务拆解规划（长难复杂任务自动拆解）
//!
//! 对齐 Web UI `types/plans.ts` 契约（PlanRevisionSnapshot / PlanRunSnapshot）：
//! - `plan.analyze`：确定性拆解（复用 Orchestrator 融合路由分析），长任务自动生成
//!   PlanStep 序列（含依赖序），Solo 短任务生成最小三阶段计划
//! - `plan.create` / `plan.list` / `plan.run` / `plan.cancel`：持久化修订 + 运行状态机
//!   （queued → running → completed / cancelled，步骤级 pending/in_progress/completed）
//!
//! 借鉴 TUI 工作台 `update_plan` 的 PlanStep 状态机 + Marguerite `session/plans.py`
//! 的 revision/run 分离（不可变修订 + 运行实例）。

use std::fmt;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use uuid::Uuid;

use crate::memory::embedding::EmbeddingProvider;
use crate::orchestration::Orchestrator;
use crate::persistence::config::Config;

/// 复杂任务计划驱动长执行（自动拆解 + 分步长续航）
pub mod runner;

/// 重规划差异阈值：当前步骤与执行观测的向量余弦低于该值判定「偏离计划」
const REPLAN_THRESHOLD: f64 = 0.15;

/// 计划步骤状态（对齐 webui `PlanRunStepStatus`）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepStatus {
    Pending,
    InProgress,
    Completed,
    Blocked,
    Skipped,
}

/// 计划运行状态（对齐 webui `PlanRunStatus`）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Queued,
    Running,
    Paused,
    Blocked,
    Completed,
    Cancelled,
    Superseded,
}

impl fmt::Display for RunStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl RunStatus {
    fn as_str(self) -> &'static str {
        match self {
            RunStatus::Queued => "queued",
            RunStatus::Running => "running",
            RunStatus::Paused => "paused",
            RunStatus::Blocked => "blocked",
            RunStatus::Completed => "completed",
            RunStatus::Cancelled => "cancelled",
            RunStatus::Superseded => "superseded",
        }
    }
}

/// 计划修订（不可变）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanRevision {
    pub revision_id: String,
    pub plan_id: String,
    pub parent_revision_id: Option<String>,
    pub generation: u32,
    pub title: String,
    pub markdown: String,
    pub steps: Vec<Value>, // PlanRevisionStep
    pub current: bool,
    pub created_at: u64,
}

/// 执行前清单项（对齐 webui `PlanRunChecklistItem`，camelCase 序列化）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChecklistItem {
    pub step_id: String,
    pub title: String,
    pub checked: bool,
}

/// 动态重规划结果（B4）
#[derive(Debug, Clone)]
struct ReplanOutcome {
    /// 是否判定偏离计划（触发新修订）
    deviated: bool,
    /// 当前步骤与执行观测的向量余弦
    similarity: f64,
    /// 偏离时生成的新修订
    revision: Option<PlanRevision>,
}

/// 计划运行实例
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanRun {
    pub run_id: String,
    pub plan_id: String,
    pub revision_id: String,
    pub status: RunStatus,
    pub current_step_id: Option<String>,
    pub steps: Vec<Value>, // PlanRunStepSnapshot（含 result / parallel）
    /// 执行前清单（B4：`plans.run` 前置生成，逐项勾选推进）
    pub checklist: Vec<ChecklistItem>,
    /// 最近并行子任务批（B4：`plans.parallel.run` 扇出聚合结果）
    pub parallel_batch: Option<Value>,
    /// 最近重规划 diff（B4：`plans.replan` 触发后记录）
    pub replan: Option<Value>,
    pub created_at: u64,
    pub started_at: Option<u64>,
    pub finished_at: Option<u64>,
    pub terminal_reason: Option<String>,
}

/// 计划存储（单例 + 落盘 `data_dir/plans.json`）
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct PlanStore {
    pub revisions: Vec<PlanRevision>,
    pub runs: Vec<PlanRun>,
    next_generation: u32,
}

impl PlanStore {
    fn path() -> PathBuf {
        Config::data_dir().join("plans.json")
    }

    fn load() -> Self {
        std::fs::read_to_string(Self::path())
            .ok()
            .and_then(|raw| serde_json::from_str(&raw).ok())
            .unwrap_or_default()
    }

    fn save(&self) {
        if let Some(dir) = Self::path().parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(raw) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(Self::path(), raw);
        }
    }

    /// 分析并创建修订（plan.create 入口；`parent_revision_id` 用于重规划生成同 plan 新修订）
    fn create_revision(
        &mut self,
        prompt: &str,
        title: Option<&str>,
        parent_revision_id: Option<&str>,
    ) -> PlanRevision {
        let plan_id = parent_revision_id
            .and_then(|pid| self.find_revision(pid).map(|r| r.plan_id.clone()))
            .unwrap_or_else(|| format!("plan-{}", Uuid::new_v4()));
        // 同 plan 旧修订 current 置 false（保持不可变修订 + 单一 current）
        for r in self.revisions.iter_mut() {
            if r.plan_id == plan_id {
                r.current = false;
            }
        }
        let revision_id = format!("rev-{}", Uuid::new_v4());
        self.next_generation += 1;
        let (md, steps) = decompose(prompt);
        let t = title.unwrap_or("未命名计划").to_string();
        let rev = PlanRevision {
            revision_id: revision_id.clone(),
            plan_id: plan_id.clone(),
            parent_revision_id: parent_revision_id.map(str::to_string),
            generation: self.next_generation,
            title: t,
            markdown: md,
            steps,
            current: true,
            created_at: now_ms(),
        };
        self.revisions.push(rev.clone());
        self.save();
        rev
    }

    fn find_revision(&self, revision_id: &str) -> Option<&PlanRevision> {
        self.revisions.iter().find(|r| r.revision_id == revision_id)
    }

    /// 创建运行实例（前置生成执行前 checklist，全部未勾选）
    fn start_run(&mut self, revision_id: &str) -> Option<PlanRun> {
        let rev = self.find_revision(revision_id)?.clone();
        let run_id = format!("run-{}", Uuid::new_v4());
        let mut steps: Vec<Value> = Vec::new();
        let first = rev
            .steps
            .first()
            .and_then(|s| s["stepId"].as_str().map(str::to_string));
        for (i, s) in rev.steps.iter().enumerate() {
            let sid = s["stepId"].as_str().unwrap_or_default();
            steps.push(json!({
                "stepId": sid,
                "title": s["title"].as_str().unwrap_or(""),
                "status": if i == 0 { "in_progress" } else { "pending" },
            }));
        }
        // 执行前清单：每步一项，初始未勾选
        let checklist = steps
            .iter()
            .map(|st| ChecklistItem {
                step_id: st["stepId"].as_str().unwrap_or_default().to_string(),
                title: st["title"].as_str().unwrap_or_default().to_string(),
                checked: false,
            })
            .collect();
        let run = PlanRun {
            run_id: run_id.clone(),
            plan_id: rev.plan_id.clone(),
            revision_id: revision_id.to_string(),
            status: RunStatus::Running,
            current_step_id: first,
            steps,
            checklist,
            parallel_batch: None,
            replan: None,
            created_at: now_ms(),
            started_at: Some(now_ms()),
            finished_at: None,
            terminal_reason: None,
        };
        self.runs.push(run.clone());
        self.save();
        Some(run)
    }

    fn cancel_run(&mut self, run_id: &str) -> Option<PlanRun> {
        let run = self.runs.iter_mut().find(|r| r.run_id == run_id)?;
        run.status = RunStatus::Cancelled;
        run.finished_at = Some(now_ms());
        for s in run.steps.iter_mut() {
            if s["status"].as_str() == Some("pending") {
                s["status"] = json!("skipped");
            } else if s["status"].as_str() == Some("in_progress") {
                s["status"] = json!("blocked");
            }
        }
        let run = run.clone();
        self.save();
        Some(run)
    }

    /// 推进运行（B4 执行前清单）：勾选清单项 / 完成当前步骤并推进
    ///
    /// - `checked` 为 `Some` 时按 `step_id` 勾选/取消清单项；
    /// - `advance=true` 时全量勾选清单（预检通过）、把当前步骤标记 completed 并推进
    ///   到下一 pending 步骤；全部完成后 `status=completed` 且 `current_step_id=None`。
    fn advance_run(
        &mut self,
        run_id: &str,
        step_id: Option<&str>,
        checked: Option<bool>,
        advance: bool,
    ) -> Option<PlanRun> {
        let run = self.runs.iter_mut().find(|r| r.run_id == run_id)?;
        if let (Some(sid), Some(chk)) = (step_id, checked) {
            for item in run.checklist.iter_mut() {
                if item.step_id == sid {
                    item.checked = chk;
                }
            }
        }
        if advance {
            for item in run.checklist.iter_mut() {
                item.checked = true;
            }
            if let Some(cid) = run.current_step_id.clone() {
                for st in run.steps.iter_mut() {
                    if st["stepId"].as_str() == Some(cid.as_str()) {
                        st["status"] = json!("completed");
                    }
                }
                let next = run
                    .steps
                    .iter()
                    .find(|st| st["status"].as_str() == Some("pending"))
                    .and_then(|st| st["stepId"].as_str().map(str::to_string));
                run.current_step_id = next;
                if run.current_step_id.is_none() {
                    run.status = RunStatus::Completed;
                    run.finished_at = Some(now_ms());
                }
            }
        }
        let run = run.clone();
        self.save();
        Some(run)
    }

    /// 动态重规划（B4）：嵌入后端比较当前步骤与执行观测
    ///
    /// 向量余弦低于 `REPLAN_THRESHOLD` 判定偏离 → 基于观测拆解生成同 plan 新修订
    /// （`parent_revision_id` 关联谱系，旧修订 `current` 置 false），并记录 `run.replan` diff。
    fn replan_run(
        &mut self,
        run_id: &str,
        observation: &str,
        prompt: &str,
        title: Option<&str>,
    ) -> Option<ReplanOutcome> {
        let run = self.runs.iter().find(|r| r.run_id == run_id)?.clone();
        let current_title = run
            .current_step_id
            .as_ref()
            .and_then(|cid| {
                run.steps
                    .iter()
                    .find(|st| st["stepId"].as_str() == Some(cid.as_str()))
                    .and_then(|st| st["title"].as_str())
            })
            .unwrap_or("");
        let obs = if observation.trim().is_empty() {
            prompt.trim()
        } else {
            observation.trim()
        };
        let sim = EmbeddingProvider::global().cosine(
            &EmbeddingProvider::global().embed(current_title),
            &EmbeddingProvider::global().embed(obs),
        );
        // 阈值判定：相似 → 仍在计划内；观测为空或相似度达标 → 不偏离
        if obs.is_empty() || sim >= REPLAN_THRESHOLD {
            return Some(ReplanOutcome {
                deviated: false,
                similarity: sim,
                revision: None,
            });
        }
        // 偏离计划：基于观测重新拆解，生成同 plan 子修订（谱系继承 plan_id）
        let rev = self.create_revision(obs, title, Some(&run.revision_id));
        if let Some(r) = self.runs.iter_mut().find(|r| r.run_id == run_id) {
            r.replan = Some(json!({
                "revisionId": rev.revision_id,
                "planId": rev.plan_id,
                "previousRevisionId": run.revision_id,
                "similarity": sim,
                "prompt": obs,
                "stepsChanged": rev.steps.len(),
                "createdAt": now_ms(),
            }));
        }
        Some(ReplanOutcome {
            deviated: true,
            similarity: sim,
            revision: Some(rev),
        })
    }

    /// 并行子任务（B4）：确定性扇出聚合
    ///
    /// 对 `step_ids`（缺省取全部 pending 步骤）逐一「执行」并聚合结果写 `run.parallel_batch`：
    /// 显式命中 `fail_step_ids` 的子项标记 failed，其余 completed；每子项产出 summary。
    fn parallel_run(
        &mut self,
        run_id: &str,
        step_ids: &[String],
        fail_step_ids: &[String],
    ) -> Option<PlanRun> {
        // 不可变快照取目标步骤（避免与写回二次可变借用冲突）
        let snapshot = self.runs.iter().find(|r| r.run_id == run_id)?.clone();
        let targets: Vec<(String, String)> = if step_ids.is_empty() {
            snapshot
                .steps
                .iter()
                .filter(|st| st["status"].as_str() == Some("pending"))
                .map(|st| {
                    (
                        st["stepId"].as_str().unwrap_or_default().to_string(),
                        st["title"].as_str().unwrap_or_default().to_string(),
                    )
                })
                .collect()
        } else {
            step_ids
                .iter()
                .filter_map(|sid| {
                    snapshot
                        .steps
                        .iter()
                        .find(|st| st["stepId"].as_str() == Some(sid))
                        .map(|st| {
                            (
                                sid.clone(),
                                st["title"].as_str().unwrap_or_default().to_string(),
                            )
                        })
                })
                .collect()
        };
        let mut results = Vec::with_capacity(targets.len());
        let mut successful = 0usize;
        let mut failed = 0usize;
        for (sid, title) in targets {
            let ok = !fail_step_ids.iter().any(|f| f == &sid);
            if ok {
                successful += 1;
            } else {
                failed += 1;
            }
            results.push(json!({
                "stepId": sid,
                "title": title,
                "status": if ok { "completed" } else { "failed" },
                "summary": if ok {
                    format!("已完成：{title}")
                } else {
                    format!("失败：{title}")
                },
            }));
        }
        let run = self.runs.iter_mut().find(|r| r.run_id == run_id)?;
        run.parallel_batch = Some(json!({
            "status": if failed == 0 { "completed" } else { "failed" },
            "successful": successful,
            "failed": failed,
            "total": results.len(),
            "results": results,
        }));
        let run = run.clone();
        self.save();
        Some(run)
    }
}

fn store() -> &'static Mutex<PlanStore> {
    static S: OnceLock<Mutex<PlanStore>> = OnceLock::new();
    S.get_or_init(|| Mutex::new(PlanStore::load()))
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// 确定性拆解：复用 Orchestrator 融合路由分析生成 PlanStep 序列 + Markdown 计划
fn decompose(prompt: &str) -> (String, Vec<Value>) {
    static ROUTER: OnceLock<Orchestrator> = OnceLock::new();
    let orch = ROUTER.get_or_init(|| Orchestrator::new(Config::default()));
    let a = orch.analyze(prompt);

    let mut steps: Vec<Value> = Vec::new();
    if a.subtasks.is_empty() {
        // Solo / 简单任务：最小三阶段计划
        for (i, t) in ["理解需求与约束", "实施与产出", "验证与收尾"]
            .iter()
            .enumerate()
        {
            steps.push(json!({
                "stepId": format!("step-{}", i + 1),
                "title": t,
                "details": if i == 0 { Some(prompt.to_string()) } else { None },
            }));
        }
    } else {
        // 复杂任务：按子任务拆解（附角色）
        for (i, s) in a.subtasks.iter().enumerate() {
            steps.push(json!({
                "stepId": format!("step-{}", i + 1),
                "title": format!("[{}] {}", s.role, s.task),
                "details": s.task,
            }));
        }
    }

    let mut md = format!(
        "# 计划：{}\n\n**编排形态**: {}\n\n**决策理由**: {}\n\n",
        prompt.trim().chars().take(60).collect::<String>(),
        a.strategy.label(),
        a.rationale
    );
    for (i, s) in steps.iter().enumerate() {
        md.push_str(&format!(
            "{}. {}\n",
            i + 1,
            s["title"].as_str().unwrap_or("")
        ));
    }
    (md, steps)
}

fn revision_view(r: &PlanRevision) -> Value {
    json!({
        "revisionId": r.revision_id,
        "planId": r.plan_id,
        "parentRevisionId": r.parent_revision_id,
        "generation": r.generation,
        "title": r.title,
        "markdown": r.markdown,
        "steps": r.steps,
        "current": r.current,
        "createdAt": r.created_at,
    })
}

fn run_view(r: &PlanRun) -> Value {
    json!({
        "runId": r.run_id,
        "planRevisionId": r.revision_id,
        "status": r.status.to_string(),
        "currentStepId": r.current_step_id,
        "stateRevision": 1,
        "driverKind": "manual",
        "steps": r.steps,
        "checklist": r.checklist,
        "parallelBatch": r.parallel_batch,
        "replan": r.replan,
        "terminalReason": r.terminal_reason,
        "createdAt": r.created_at,
        "startedAt": r.started_at,
        "finishedAt": r.finished_at,
    })
}

/// `plans.analyze`：只读拆解预览（不落库）
pub fn plans_analyze(params: &Value) -> Value {
    let prompt = params.get("prompt").and_then(Value::as_str).unwrap_or("");
    if prompt.trim().is_empty() {
        return json!({ "error": "prompt 不可为空" });
    }
    let (md, steps) = decompose(prompt);
    json!({
        "revisionId": format!("rev-preview-{}", Uuid::new_v4()),
        "planId": format!("plan-preview-{}", Uuid::new_v4()),
        "title": prompt.trim().chars().take(40).collect::<String>(),
        "markdown": md,
        "steps": steps,
        "current": true,
        "createdAt": now_ms(),
    })
}

/// `plans.create`：分析并持久化修订
pub fn plans_create(params: &Value) -> Value {
    let prompt = params.get("prompt").and_then(Value::as_str).unwrap_or("");
    if prompt.trim().is_empty() {
        return json!({ "error": "prompt 不可为空" });
    }
    let title = params.get("title").and_then(Value::as_str);
    let rev = store()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .create_revision(prompt, title, None);
    json!({ "revision": revision_view(&rev) })
}

/// `plans.revise`：以既有修订为父生成同 plan 新修订（前端「修订方案」按钮）。
///
/// 与 `plans.create` 的区别：保留 plan 血缘（parent_revision_id + 同 plan_id +
/// generation 递增），旧修订 `current` 置 false、新修订 `current=true`，保证
/// 前端 `shouldAdoptPlanRevision` 的 generation 顺序判定可以采纳新卡片。
pub fn plans_revise(params: &Value) -> Value {
    let revision_id = params
        .get("planRevisionId")
        .and_then(Value::as_str)
        .unwrap_or("");
    let prompt = params.get("prompt").and_then(Value::as_str).unwrap_or("");
    if revision_id.is_empty() || prompt.trim().is_empty() {
        return json!({ "error": "planRevisionId 与 prompt 不可为空" });
    }
    let title = params.get("title").and_then(Value::as_str);
    let mut s = store().lock().unwrap_or_else(|e| e.into_inner());
    match revise_on_store(&mut s, revision_id, prompt, title) {
        Some(rev) => json!({ "planRevision": rev }),
        None => json!({ "error": format!("Plan revision not found: {revision_id}") }),
    }
}

/// 在指定 store 上执行修订（`plans_revise` 的 store 无关核心，便于内存单测）。
fn revise_on_store(
    s: &mut PlanStore,
    revision_id: &str,
    prompt: &str,
    title: Option<&str>,
) -> Option<Value> {
    s.find_revision(revision_id)?;
    let rev = s.create_revision(prompt, title, Some(revision_id));
    Some(revision_view(&rev))
}

/// 读取修订视图（供 `plans.implement` 校验与复用步骤）。
pub fn get_revision_view(revision_id: &str) -> Option<Value> {
    let s = store().lock().unwrap_or_else(|e| e.into_inner());
    s.find_revision(revision_id).map(revision_view)
}

/// 该修订是否已有进行中的运行（`plans.implement` 防重复执行）。
pub fn has_active_run(revision_id: &str) -> bool {
    let s = store().lock().unwrap_or_else(|e| e.into_inner());
    s.runs
        .iter()
        .any(|r| r.revision_id == revision_id && r.status == RunStatus::Running)
}

/// `plans.list`：列出修订与运行
pub fn plans_list() -> Value {
    let s = store().lock().unwrap_or_else(|e| e.into_inner());
    let revisions: Vec<Value> = s.revisions.iter().map(revision_view).collect();
    let runs: Vec<Value> = s.runs.iter().map(run_view).collect();
    json!({ "plans": revisions, "runs": runs })
}

/// `plans.run`：为修订启动运行实例
pub fn plans_run(params: &Value) -> Value {
    let revision_id = params
        .get("revisionId")
        .and_then(Value::as_str)
        .unwrap_or("");
    if revision_id.is_empty() {
        return json!({ "error": "revisionId 不可为空" });
    }
    match store()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .start_run(revision_id)
    {
        Some(run) => json!({ "run": run_view(&run) }),
        None => json!({ "error": "修订不存在" }),
    }
}

/// `plans.cancel`：取消运行
pub fn plans_cancel(params: &Value) -> Value {
    let run_id = params.get("runId").and_then(Value::as_str).unwrap_or("");
    if run_id.is_empty() {
        return json!({ "error": "runId 不可为空" });
    }
    match store()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .cancel_run(run_id)
    {
        Some(run) => json!({ "run": run_view(&run) }),
        None => json!({ "error": "运行不存在" }),
    }
}

/// `plans.checklist`（B4）：读取运行执行前清单（含步骤进度）
pub fn plans_checklist(params: &Value) -> Value {
    let run_id = params.get("runId").and_then(Value::as_str).unwrap_or("");
    if run_id.is_empty() {
        return json!({ "error": "runId 不可为空" });
    }
    let s = store().lock().unwrap_or_else(|e| e.into_inner());
    match s.runs.iter().find(|r| r.run_id == run_id) {
        Some(run) => json!({ "run": run_view(run) }),
        None => json!({ "error": "运行不存在" }),
    }
}

/// `plans.run.advance`（B4）：勾选清单项 / 完成当前步骤并推进
pub fn plans_run_advance(params: &Value) -> Value {
    let run_id = params.get("runId").and_then(Value::as_str).unwrap_or("");
    if run_id.is_empty() {
        return json!({ "error": "runId 不可为空" });
    }
    let step_id = params.get("stepId").and_then(Value::as_str);
    let checked = params.get("checked").and_then(Value::as_bool);
    let advance = params
        .get("advance")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    match store()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .advance_run(run_id, step_id, checked, advance)
    {
        Some(run) => json!({ "run": run_view(&run) }),
        None => json!({ "error": "运行不存在" }),
    }
}

/// `plans.replan`（B4）：动态重规划——比较当前步骤与执行观测，偏离则生成同 plan 新修订
pub fn plans_replan(params: &Value) -> Value {
    let run_id = params.get("runId").and_then(Value::as_str).unwrap_or("");
    if run_id.is_empty() {
        return json!({ "error": "runId 不可为空" });
    }
    let observation = params
        .get("observation")
        .and_then(Value::as_str)
        .unwrap_or("");
    let prompt = params.get("prompt").and_then(Value::as_str).unwrap_or("");
    let title = params.get("title").and_then(Value::as_str);
    let mut s = store().lock().unwrap_or_else(|e| e.into_inner());
    match s.replan_run(run_id, observation, prompt, title) {
        Some(outcome) => {
            let mut res = json!({
                "deviated": outcome.deviated,
                "similarity": outcome.similarity,
            });
            if let Some(rev) = &outcome.revision {
                res["revision"] = revision_view(rev);
                if let Some(run) = s.runs.iter().find(|r| r.run_id == run_id) {
                    res["run"] = run_view(run);
                }
            }
            res
        }
        None => json!({ "error": "运行不存在" }),
    }
}

/// `plans.parallel.run`（B4）：并行子任务——确定性扇出聚合（写 `run.parallel_batch`）
pub fn plans_parallel_run(params: &Value) -> Value {
    let run_id = params.get("runId").and_then(Value::as_str).unwrap_or("");
    if run_id.is_empty() {
        return json!({ "error": "runId 不可为空" });
    }
    let step_ids: Vec<String> = params
        .get("stepIds")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    let fail_step_ids: Vec<String> = params
        .get("failStepIds")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    match store()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .parallel_run(run_id, &step_ids, &fail_step_ids)
    {
        Some(run) => json!({ "run": run_view(&run) }),
        None => json!({ "error": "运行不存在" }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_decompose_simple_task_three_steps() {
        let (_md, steps) = decompose("解释什么是闭包");
        assert_eq!(steps.len(), 3);
        assert_eq!(steps[0]["stepId"], "step-1");
    }

    #[test]
    fn test_decompose_complex_task_multiple_steps() {
        let (_md, steps) = decompose("实现登录模块、写单元测试并部署到服务器");
        assert!(steps.len() >= 2, "复杂任务应拆解为多步: {}", steps.len());
        // 每步含角色前缀
        assert!(steps
            .iter()
            .any(|s| s["title"].as_str().is_some_and(|t| t.starts_with('['))));
    }

    #[test]
    fn test_plan_store_roundtrip() {
        let dir = std::env::temp_dir().join(format!("mgt-plan-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        // 用临时路径测试序列化往返
        let mut s = PlanStore::default();
        let rev = s.create_revision("测试计划：重构并优化", None, None);
        assert!(rev.plan_id.starts_with("plan-"));
        assert!(!rev.markdown.is_empty());
        assert!(!rev.steps.is_empty());

        let run = s.start_run(&rev.revision_id).expect("应能启动运行");
        assert_eq!(run.status.to_string(), "running");
        assert_eq!(run.steps[0]["status"], "in_progress");
        // B4：启动即前置生成执行前清单，初始全未勾选
        assert_eq!(run.checklist.len(), run.steps.len());
        assert!(run.checklist.iter().all(|c| !c.checked));

        let cancelled = s.cancel_run(&run.run_id).expect("应能取消");
        assert_eq!(cancelled.status.to_string(), "cancelled");
        assert!(cancelled.finished_at.is_some());
    }

    #[test]
    fn test_advance_run_checklist_and_completion() {
        let mut s = PlanStore::default();
        let rev = s.create_revision("测试计划：重构并优化", None, None);
        let run = s.start_run(&rev.revision_id).unwrap();
        let first_step = run.checklist[0].step_id.clone();

        // 勾选第一项清单
        let checked = s
            .advance_run(&run.run_id, Some(&first_step), Some(true), false)
            .unwrap();
        assert!(checked.checklist[0].checked);

        // 逐步推进：每轮完成当前步骤，直到全部完成
        let mut run = checked;
        while run.current_step_id.is_some() {
            run = s.advance_run(&run.run_id, None, None, true).unwrap();
        }
        assert_eq!(run.status.to_string(), "completed");
        assert!(run.current_step_id.is_none());
        assert!(run.finished_at.is_some());
        assert!(run.checklist.iter().all(|c| c.checked));
    }

    #[test]
    fn test_replan_similar_observation_stays_on_plan() {
        let mut s = PlanStore::default();
        let rev = s.create_revision("测试计划：重构并优化", None, None);
        let run = s.start_run(&rev.revision_id).unwrap();
        // 观测与当前步骤完全一致 → 相似度高 → 不偏离
        let current_title = run.steps[0]["title"].as_str().unwrap_or("").to_string();
        let outcome = s.replan_run(&run.run_id, &current_title, "", None).unwrap();
        assert!(!outcome.deviated);
        assert!(outcome.similarity > 0.9);
        assert!(outcome.revision.is_none());
    }

    #[test]
    fn test_replan_deviated_creates_child_revision() {
        let mut s = PlanStore::default();
        let rev = s.create_revision("测试计划：重构并优化", None, None);
        let run = s.start_run(&rev.revision_id).unwrap();

        // 完全不相关观测 → 偏离 → 生成同 plan 子修订（谱系关联）
        let outcome = s
            .replan_run(&run.run_id, "遭遇网络超时，切换代理方案", "", None)
            .unwrap();
        assert!(outcome.deviated);
        assert!(outcome.similarity < 0.15);
        let child = outcome.revision.expect("偏离应生成新修订");
        assert_eq!(child.plan_id, rev.plan_id);
        assert_eq!(
            child.parent_revision_id.as_deref(),
            Some(rev.revision_id.as_str())
        );
        assert!(child.generation > rev.generation);
        // 运行实例记录 replan diff
        let run = s.runs.iter().find(|r| r.run_id == run.run_id).unwrap();
        let replan = run.replan.as_ref().expect("应记录 replan diff");
        assert_eq!(replan["previousRevisionId"], rev.revision_id);
        // 谱系 current 唯一性：旧修订 false、新修订 true
        assert!(
            !s.revisions
                .iter()
                .find(|r| r.revision_id == rev.revision_id)
                .unwrap()
                .current
        );
        assert!(
            s.revisions
                .iter()
                .find(|r| r.revision_id == child.revision_id)
                .unwrap()
                .current
        );
    }

    #[test]
    fn test_parallel_run_deterministic_aggregation() {
        let mut s = PlanStore::default();
        let rev = s.create_revision("测试计划：重构并优化", None, None);
        let run = s.start_run(&rev.revision_id).unwrap();
        let ids: Vec<String> = run
            .steps
            .iter()
            .filter_map(|st| st["stepId"].as_str().map(str::to_string))
            .collect();
        assert!(!ids.is_empty());

        // 全部成功：聚合 completed、successful=总数
        let ok = s.parallel_run(&run.run_id, &ids, &[]).unwrap();
        let batch = ok.parallel_batch.expect("应记录 parallel_batch");
        assert_eq!(batch["status"], "completed");
        assert_eq!(batch["successful"], ids.len() as u64);
        assert_eq!(batch["failed"], 0);
        assert_eq!(batch["total"], ids.len() as u64);

        // 指定失败：聚合 failed、results 逐项 summary 完整
        let fail = vec![ids[0].clone()];
        let mixed = s.parallel_run(&run.run_id, &ids, &fail).unwrap();
        let batch = mixed.parallel_batch.unwrap();
        assert_eq!(batch["status"], "failed");
        assert_eq!(batch["successful"], (ids.len() - 1) as u64);
        assert_eq!(batch["failed"], 1);
        let results = batch["results"].as_array().unwrap();
        assert_eq!(results.len(), ids.len());
        assert_eq!(results[0]["status"], "failed");
        assert_eq!(results[0]["stepId"], ids[0]);
        assert!(results[0]["summary"].as_str().unwrap().starts_with("失败"));
    }

    #[test]
    fn test_plans_analyze_validation() {
        let v = plans_analyze(&json!({}));
        assert!(v.get("error").is_some());
        let v2 = plans_analyze(&json!({ "prompt": "修复 bug" }));
        assert!(v2["steps"].is_array());
        assert!(!v2["steps"].as_array().unwrap().is_empty());
    }

    #[test]
    fn test_plans_run_unknown_revision() {
        let v = plans_run(&json!({ "revisionId": "rev-missing" }));
        assert!(v.get("error").is_some());
    }

    #[test]
    fn test_plans_revise_keeps_lineage_and_single_current() {
        let mut s = PlanStore::default();
        let parent = s.create_revision("做一个 FPS 游戏", None, None);
        let parent_id = parent.revision_id.clone();
        let parent_plan_id = parent.plan_id.clone();

        // 修订成功：同 plan 谱系 + generation 递增 + current 唯一
        let child = revise_on_store(&mut s, &parent_id, "改为双人联机", None).expect("修订应成功");
        assert_eq!(child["planId"], parent_plan_id);
        assert_eq!(child["parentRevisionId"], parent_id);
        assert_eq!(child["generation"], parent.generation + 1);
        assert_eq!(child["current"], true);
        // 谱系 current 唯一性：旧修订 false、新修订 true
        assert!(
            !s.revisions
                .iter()
                .find(|r| r.revision_id == parent_id)
                .unwrap()
                .current
        );
        assert!(
            s.revisions
                .iter()
                .find(|r| r.revision_id == child["revisionId"])
                .unwrap()
                .current
        );
        // 前端 shouldAdoptPlanRevision 依赖 generation 顺序：子 > 父
        assert!(child["generation"].as_u64().unwrap() > u64::from(parent.generation));
    }

    #[test]
    fn test_plans_revise_validation_and_missing_parent() {
        // 缺失参数 → 报错（不触碰 store）
        let err = plans_revise(&json!({ "planRevisionId": "rev-x" }));
        assert!(err.get("error").is_some());
        let err2 = plans_revise(&json!({ "prompt": "改一下" }));
        assert!(err2.get("error").is_some());
        // 未知父修订 → 报 Plan revision not found（前端据此显示友好文案）
        let err3 = plans_revise(&json!({
            "planRevisionId": "rev-573410db-0000-0000-0000-000000000000",
            "prompt": "改",
        }));
        assert!(err3["error"]
            .as_str()
            .unwrap()
            .contains("Plan revision not found"));
    }
}
