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

use std::collections::HashMap;
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

/// Plan-mode 审批闭环（DSH plan-mode 二开，Track 4）
pub mod approval;

/// 重规划差异阈值：当前步骤与执行观测的向量余弦低于该值判定「偏离计划」
const REPLAN_THRESHOLD: f64 = 0.15;

/// 前瞻窗口上限：重规划时最多向前考察多少个**未完成**步骤（含当前步）。
///
/// 长计划（如 20 步）里「第 15 步」与观测相似并不构成「已推进到第 15 步」的证据；
/// 限定窗口既避免整段计划被一次误判跳掉，也让判定成本与计划长度解耦。
const REPLAN_LOOKAHEAD_WINDOW: usize = 4;

/// 前瞻跳步的相对优势门槛：命中步相似度须高出当前步该幅度。
///
/// 近义标题（「实施与产出」/「实施与验证」）容易互相串味，只按绝对阈值判定会误跳步。
const REPLAN_ADVANCE_MARGIN: f64 = 0.1;

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
    /// 编排形态**机器键**（[`crate::domain::AgentMode::as_key`]，如 `"pipeline"`）。
    ///
    /// A5：只下发机器键，展示名由前端按 i18n 渲染；旧修订无此字段 → `None`。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub strategy: Option<String>,
    /// 决策理由（引擎自述文本）；旧修订无此字段 → `None`。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rationale: Option<String>,
    /// D-05/T08：需求文本的**归一化指纹**（16 位弱哈希，仅用于去重复用，
    /// 不作安全用途）。旧计划无此字段 → `None`（天然不参与复用）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fingerprint: Option<String>,
    pub current: bool,
    pub created_at: u64,
}

/// D-05/T08：需求文本归一化指纹——trim + 空白折叠 + 小写 + sha256 前 16 hex。
///
/// **弱哈希声明**：仅用于「同一需求重复规划」的去重/复用匹配，不承担任何安全用途
/// （16 hex = 64 bit，足够避免日常重试碰撞，且不含敏感信息）。
pub fn normalize_fingerprint(text: &str) -> String {
    let normalized = text
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase();
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(normalized.as_bytes());
    digest.iter().take(8).map(|b| format!("{b:02x}")).collect()
}

/// 执行前清单项（对齐 webui `PlanRunChecklistItem`，camelCase 序列化）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChecklistItem {
    pub step_id: String,
    pub title: String,
    pub checked: bool,
}

/// 重规划判定模式（对齐 webui `PlanReplanInfo.mode`）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReplanMode {
    /// 观测仍在当前步骤范围内 → 计划有效，不改动运行
    OnPlan,
    /// 观测命中的是**后续**步骤 → 计划已实际推进过去（前瞻跳步）
    Advance,
    /// 观测与剩余计划均不相关 → 判定偏离，重新拆解
    Rebuild,
}

impl ReplanMode {
    fn as_str(self) -> &'static str {
        match self {
            ReplanMode::OnPlan => "on_plan",
            ReplanMode::Advance => "advance",
            ReplanMode::Rebuild => "rebuild",
        }
    }
}

/// 前瞻命中项（真实向量比对结果，非占位值）
#[derive(Debug, Clone)]
struct LookaheadHit {
    step_id: String,
    title: String,
    similarity: f64,
    /// 相对当前步的前向距离（0 = 当前步本身）
    distance: usize,
}

/// 动态重规划结果（B4 + 前瞻）
#[derive(Debug, Clone)]
struct ReplanOutcome {
    /// 是否判定偏离计划（触发新修订）
    deviated: bool,
    /// 当前步骤与执行观测的向量余弦
    similarity: f64,
    /// 前瞻判定模式
    mode: ReplanMode,
    /// 判定是否已落到运行实例（`advance` 需显式 `apply`；`rebuild` 生成修订即算落地）
    applied: bool,
    /// 前瞻窗口内逐步相似度（含当前步，distance=0）
    lookahead: Vec<Value>,
    /// 前瞻命中步（无命中为 null）
    best_match: Value,
    /// 实际被跳过的步骤数（仅 advance 且落地时非 0）
    skipped_steps: usize,
    /// advance 判定下将被跳过的步骤 id（未落地时即为「建议」）
    proposed_skipped: Vec<String>,
    /// advance 判定因前置未完成而不能落地时的前置步骤 id
    blocked_by_deps: Vec<String>,
    /// 重规划前剩余代价（实测样本，无样本则为 null）
    cost_before: Value,
    /// 重规划后剩余代价（rebuild 为新计划的投影代价）
    cost_after: Value,
    /// 偏离时生成的新修订
    revision: Option<PlanRevision>,
}

/// 重规划结果基底：各分支只覆盖自己关心的字段，避免每处重复构造全部字段。
fn base_outcome(mode: ReplanMode, similarity: f64, cost_before: Value) -> ReplanOutcome {
    ReplanOutcome {
        deviated: mode == ReplanMode::Rebuild,
        similarity,
        mode,
        applied: false,
        lookahead: Vec::new(),
        best_match: Value::Null,
        skipped_steps: 0,
        proposed_skipped: Vec::new(),
        blocked_by_deps: Vec::new(),
        cost_after: cost_before.clone(),
        cost_before,
        revision: None,
    }
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

/// plans.json 专用写线程（惰性启动）。
///
/// 计划执行路径上 `PlanStore::save()` 被高频调用，而 plans.json 随历史运行
/// 无界增长（实测 463KB / 194 runs）。同步 `std::fs::write` 会把 tokio worker
/// 占住，连带同 runtime 的 WebSocket 事件循环停摆——用户侧就是「一跑任务
/// 就和后端断连」（tick 停发 → 前端心跳超时 → 被动重连）。
///
/// 这里用一条专用线程做磁盘 IO，并**合并连续快照**（只落最后一份），
/// 既移出关键路径，又避免并发写把文件写裂。
static PLAN_WRITE_TX: std::sync::OnceLock<
    std::sync::mpsc::Sender<String>,
> = std::sync::OnceLock::new();

fn plan_write_tx() -> Option<&'static std::sync::mpsc::Sender<String>> {
    Some(PLAN_WRITE_TX.get_or_init(|| {
        let (tx, rx) = std::sync::mpsc::channel::<String>();
        let spawned = std::thread::Builder::new()
            .name("marguerite-plan-writer".to_string())
            .spawn(move || {
                while let Ok(next) = rx.recv() {
                    // 合并连续快照：只落最后一份，更早的中间态已被后续状态覆盖。
                    let mut pending = next;
                    while let Ok(newer) = rx.try_recv() {
                        pending = newer;
                    }
                    let raw = pending;
                    let path = Config::data_dir().join("plans.json");
                    if let Some(dir) = path.parent() {
                        let _ = std::fs::create_dir_all(dir);
                    }
                    // 先写临时文件再原子替换，避免读者看到半份 JSON。
                    let tmp = path.with_extension("json.tmp");
                    if std::fs::write(&tmp, &raw).is_ok() {
                        let _ = std::fs::rename(&tmp, &path);
                    } else {
                        let _ = std::fs::write(&path, &raw);
                    }
                }
            });
        if spawned.is_err() {
            tracing::warn!("plans.json 写线程启动失败，退回同步落盘");
        }
        tx
    }))
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

    /// 落盘（见 `save` 的说明：序列化后投递给专用写线程，避免同步磁盘 IO
    /// 占住 tokio worker 而拖停同 runtime 的 WS 事件循环）。
    fn save(&self) {
        let Ok(raw) = serde_json::to_string_pretty(self) else {
            return;
        };
        // 落盘卸载到专用线程：`save()` 在计划执行的关键路径上被高频调用
        // （每步多次），而 plans.json 随历史运行无界增长（实测 463KB/194 runs）。
        // 同步 `std::fs::write` 会把 tokio worker 占住数十毫秒到数秒，
        // 连带同 runtime 的 WS 事件循环停摆——表现为长任务期间 tick 停发、
        // 前端心跳超时后被动重连（「运行任务就和后端断连」的根因之一）。
        // 这里只做序列化 + 投递，磁盘 IO 与并发合并在写线程里完成。
        if let Some(tx) = plan_write_tx() {
            let _ = tx.send(raw);
        } else {
            // 写线程尚未启动（极早期/测试）：直接同步写，保证不丢状态。
            if let Some(dir) = Self::path().parent() {
                let _ = std::fs::create_dir_all(dir);
            }
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
        let d = decompose(prompt);
        let t = title.map(str::to_string).unwrap_or_else(|| derived_title(prompt));
        let rev = PlanRevision {
            revision_id: revision_id.clone(),
            plan_id: plan_id.clone(),
            parent_revision_id: parent_revision_id.map(str::to_string),
            generation: self.next_generation,
            title: t,
            markdown: d.markdown,
            steps: d.steps,
            strategy: Some(d.strategy),
            rationale: Some(d.rationale),
            fingerprint: Some(normalize_fingerprint(prompt)),
            current: true,
            created_at: now_ms(),
        };
        self.revisions.push(rev.clone());
        self.save();
        rev
    }

    /// M2-C2：基于既有修订派生新修订（步骤级增删改），保留同 plan 谱系与
    /// generation 递增、current 唯一。与 `create_revision` 的区别：不重新
    /// decompose，直接用调用方传来的 `steps`（用户执行前增删改后的步骤），
    /// 保证「计划编辑影响后续 Orchestrator 执行」（后续 `start_run` 消费新 steps）。
    fn spawn_child(
        &mut self,
        parent_revision_id: &str,
        title: String,
        steps: Vec<Value>,
    ) -> Option<PlanRevision> {
        let parent = self.find_revision(parent_revision_id)?.clone();
        // 同 plan 旧修订 current 置 false（保持不可变修订 + 单一 current）
        for r in self.revisions.iter_mut() {
            if r.plan_id == parent.plan_id {
                r.current = false;
            }
        }
        let revision_id = format!("rev-{}", Uuid::new_v4());
        self.next_generation += 1;
        let rev = PlanRevision {
            revision_id: revision_id.clone(),
            plan_id: parent.plan_id.clone(),
            parent_revision_id: Some(parent.revision_id.clone()),
            generation: self.next_generation,
            title: title.clone(),
            markdown: steps_to_markdown(&steps),
            steps,
            // 编辑派生修订沿用父修订的编排形态与决策理由（形态未因步骤增删而改变）。
            strategy: parent.strategy.clone(),
            rationale: parent.rationale.clone(),
            // 编辑派生修订沿用父修订指纹（需求文本未变）。
            fingerprint: parent.fingerprint.clone(),
            current: true,
            created_at: now_ms(),
        };
        self.revisions.push(rev.clone());
        self.save();
        Some(rev)
    }

    /// D-05/T08：按指纹找最近一次**终态成功**的修订（用于「继续上次计划」候选）。
    ///
    /// 只认可存在 `Completed` 运行实例的修订——未跑通/失败的计划不作为复用候选，
    /// 避免把「上次就没成功」的计划推给用户。旧修订无指纹 → 不参与。
    pub fn find_latest_by_fingerprint(&self, fingerprint: &str) -> Option<&PlanRevision> {
        if fingerprint.is_empty() {
            return None;
        }
        self.revisions
            .iter()
            .filter(|r| r.fingerprint.as_deref() == Some(fingerprint))
            .filter(|r| {
                self.runs.iter().any(|run| {
                    run.revision_id == r.revision_id && run.status == RunStatus::Completed
                })
            })
            .max_by_key(|r| r.created_at)
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
        // 清单随步骤终态同步（skipped/blocked 步骤不得显示为已勾选）
        sync_checklist(run);
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
            // 清单勾选与步骤状态严格一致（仅 completed 步骤勾选；skipped/blocked 不勾）
            sync_checklist(run);
        }
        let run = run.clone();
        self.save();
        Some(run)
    }

    /// 动态重规划（B4 + 前瞻）：多步前瞻判定 + 实测代价模型
    ///
    /// 与单步比对（原实现只看当前步骤）的区别：
    /// - **多步前瞻**：把观测与「当前步 + 窗口内后续未完成步」逐一做向量比对。观测命中
    ///   **后续**步骤说明计划其实已经推进过去（[`ReplanMode::Advance`]）——此时应跳步，
    ///   而不是丢弃整段剩余计划去重新拆解；只有与整个窗口都不相关才是真偏离
    ///   （[`ReplanMode::Rebuild`]）。二者在单步比对下同为「低相似度」，行为被混为一谈。
    /// - **依赖安全**：跳步目标若仍有未完成前置（修订 `deps` DAG），只回报判定、绝不落地，
    ///   避免破坏依赖序导致后续步骤读不到前置产物。
    /// - **代价模型**：以**实测**样本估算重规划前后的剩余代价，无样本时为 null，绝不虚报。
    ///
    /// `apply=false` 时跳步只作为建议返回（丢弃已排定步骤须由调用方显式确认）；
    /// `rebuild` 生成新修订不破坏运行实例，故不受 `apply` 约束（保持既有契约）。
    fn replan_run(
        &mut self,
        run_id: &str,
        observation: &str,
        prompt: &str,
        title: Option<&str>,
        apply: bool,
    ) -> Option<ReplanOutcome> {
        let run = self.runs.iter().find(|r| r.run_id == run_id)?.clone();
        let now = now_ms();
        let cost_before = estimate_cost(&run, now);
        let obs = if observation.trim().is_empty() {
            prompt.trim()
        } else {
            observation.trim()
        };
        let Some(current_idx) = run.current_step_id.as_ref().and_then(|cid| {
            run.steps
                .iter()
                .position(|st| st["stepId"].as_str() == Some(cid.as_str()))
        }) else {
            // 运行已收口（无当前步骤）→ 无重规划对象，保持计划不变。
            return Some(base_outcome(ReplanMode::OnPlan, 0.0, cost_before));
        };
        if obs.is_empty() {
            // 观测为空 = 没有偏离证据 → 不因缺证据改计划。
            return Some(base_outcome(ReplanMode::OnPlan, 0.0, cost_before));
        }

        let hits = lookahead_hits(&run.steps, current_idx, obs);
        let current_sim = hits
            .iter()
            .find(|h| h.distance == 0)
            .map(|h| h.similarity)
            .unwrap_or(0.0);
        let (mode, hit_idx) = decide_replan_mode(current_sim, &hits);
        let lookahead: Vec<Value> = hits.iter().map(hit_json).collect();
        let best_match = hit_idx.map(|i| hit_json(&hits[i])).unwrap_or(Value::Null);

        if mode == ReplanMode::OnPlan {
            let mut o = base_outcome(ReplanMode::OnPlan, current_sim, cost_before);
            o.lookahead = lookahead;
            o.best_match = best_match;
            return Some(o);
        }

        if mode == ReplanMode::Rebuild {
            // 偏离计划：基于观测重新拆解，生成同 plan 子修订（谱系继承 plan_id）
            let rev = self.create_revision(obs, title, Some(&run.revision_id));
            // 新计划的投影代价：沿用实测平均单步耗时，步数换成新计划的步骤数。
            let cost_after = projected_cost(&cost_before, rev.steps.len());
            if let Some(r) = self.runs.iter_mut().find(|r| r.run_id == run_id) {
                r.replan = Some(json!({
                    "mode": ReplanMode::Rebuild.as_str(),
                    "revisionId": rev.revision_id,
                    "planId": rev.plan_id,
                    "previousRevisionId": run.revision_id,
                    "similarity": current_sim,
                    "prompt": obs,
                    "stepsChanged": rev.steps.len(),
                    "bestMatch": best_match,
                    "costBefore": cost_before,
                    "costAfter": cost_after,
                    "createdAt": now,
                }));
            }
            let mut o = base_outcome(ReplanMode::Rebuild, current_sim, cost_before);
            o.applied = true;
            o.lookahead = lookahead;
            o.best_match = best_match;
            o.cost_after = cost_after;
            o.revision = Some(rev);
            self.save();
            return Some(o);
        }

        // ReplanMode::Advance：观测命中后续步骤 → 跳步（跳过其间的未完成步骤）
        let hit = &hits[hit_idx.expect("advance 判定必然带命中下标")];
        let target_idx = current_idx + hit.distance;
        let mut o = base_outcome(ReplanMode::Advance, current_sim, cost_before);
        o.lookahead = lookahead;
        o.best_match = best_match;
        o.proposed_skipped = run
            .steps
            .iter()
            .take(target_idx)
            .skip(current_idx)
            .filter(|st| matches!(st["status"].as_str(), Some("pending" | "in_progress")))
            .filter_map(|st| st["stepId"].as_str().map(str::to_string))
            .collect();
        o.blocked_by_deps = self.unmet_deps(&run, &hit.step_id);
        if !o.blocked_by_deps.is_empty() || !apply {
            // 前置未完成 / 调用方未确认 → 只回报前瞻判定，不改运行。
            return Some(o);
        }
        let skipped_count = o.proposed_skipped.len();
        if let Some(r) = self.runs.iter_mut().find(|r| r.run_id == run_id) {
            for st in r.steps.iter_mut() {
                let sid = st["stepId"].as_str().unwrap_or_default();
                if o.proposed_skipped.iter().any(|s| s == sid) {
                    st["status"] = json!("skipped");
                } else if sid == hit.step_id {
                    st["status"] = json!("in_progress");
                }
            }
            r.current_step_id = Some(hit.step_id.clone());
            // 清单随步骤终态同步（跳过的步骤不得显示为已勾选）
            sync_checklist(r);
        }
        let cost_after = self
            .runs
            .iter()
            .find(|r| r.run_id == run_id)
            .map(|r| estimate_cost(r, now))
            .unwrap_or_else(|| o.cost_before.clone());
        if let Some(r) = self.runs.iter_mut().find(|r| r.run_id == run_id) {
            r.replan = Some(json!({
                "mode": ReplanMode::Advance.as_str(),
                "revisionId": run.revision_id,
                "planId": run.plan_id,
                "previousRevisionId": run.revision_id,
                "similarity": current_sim,
                "prompt": obs,
                "stepsChanged": skipped_count,
                "skippedStepIds": o.proposed_skipped,
                "bestMatch": o.best_match,
                "costBefore": o.cost_before,
                "costAfter": cost_after,
                "createdAt": now,
            }));
        }
        self.save();
        o.applied = true;
        o.skipped_steps = skipped_count;
        o.cost_after = cost_after;
        Some(o)
    }

    /// 前瞻命中步骤是否存在**未完成**前置（修订 `deps` DAG）。
    ///
    /// 跳步改变执行顺序：目标步骤的前置若还没完成，跳过去等于破坏依赖序
    /// （后续步骤可能读不到前置产物），因此此时只回报判定、不落地跳步。
    fn unmet_deps(&self, run: &PlanRun, step_id: &str) -> Vec<String> {
        let Some(deps) = self
            .find_revision(&run.revision_id)
            .and_then(|rev| {
                rev.steps
                    .iter()
                    .find(|s| s["stepId"].as_str() == Some(step_id))
                    .and_then(|s| s["deps"].as_array())
                    .cloned()
            })
        else {
            return Vec::new();
        };
        deps.iter()
            .filter_map(Value::as_str)
            .filter(|dep| {
                run.steps
                    .iter()
                    .find(|st| st["stepId"].as_str() == Some(*dep))
                    .is_some_and(|st| st["status"].as_str() != Some("completed"))
            })
            .map(str::to_string)
            .collect()
    }

    /// 并行目标快照（`step_ids` 缺省取全部 pending；指定则按存在的步骤取）
    fn parallel_targets(&self, run_id: &str, step_ids: &[String]) -> Option<Vec<(String, String)>> {
        let snapshot = self.runs.iter().find(|r| r.run_id == run_id)?;
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
        Some(targets)
    }

    /// 并行子任务（B4）：确定性扇出聚合——**离线回退路径，非真执行**。
    ///
    /// 对 `step_ids`（缺省取全部 pending 步骤）逐一标记并聚合结果写
    /// `run.parallel_batch`（`simulated=true` 显式声明未真正干活，绝不冒充）；
    /// 显式命中 `fail_step_ids` 的子项标记 failed，其余 completed。
    fn parallel_run(
        &mut self,
        run_id: &str,
        step_ids: &[String],
        fail_step_ids: &[String],
    ) -> Option<PlanRun> {
        let targets = self.parallel_targets(run_id, step_ids)?;
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
            "simulated": true,
        }));
        let run = run.clone();
        self.save();
        Some(run)
    }

    /// 并行真执行结果回填（v19 真执行化）：应用 runner 各步骤真实执行产物。
    ///
    /// - 逐项写步骤终态（completed / failed，含真实 summary）；
    /// - 写 `run.parallel_batch`（`executed=true`，与离线 simulated 批区分）；
    /// - 步骤全部到达终态（无 pending / in_progress）→ 运行置 completed 并
    ///   清 current_step_id，对齐 webui `deliveryReady` 交付契约。
    fn parallel_run_apply(&mut self, run_id: &str, results: &[Value]) -> Option<PlanRun> {
        let run = self.runs.iter_mut().find(|r| r.run_id == run_id)?;
        let successful = results
            .iter()
            .filter(|r| r["status"].as_str() == Some("completed"))
            .count();
        let failed = results.len().saturating_sub(successful);
        for r in results {
            let sid = r["stepId"].as_str().unwrap_or_default();
            let status = r["status"].as_str().unwrap_or("failed");
            if let Some(st) = run
                .steps
                .iter_mut()
                .find(|st| st["stepId"].as_str() == Some(sid))
            {
                st["status"] = json!(status);
                if !r["summary"].as_str().unwrap_or("").is_empty() {
                    st["result"] = r["summary"].clone();
                }
            }
        }
        run.parallel_batch = Some(json!({
            "status": if failed == 0 { "completed" } else { "failed" },
            "successful": successful,
            "failed": failed,
            "total": results.len(),
            "results": results,
            "executed": true,
        }));
        // 步骤全部终态 → 运行收口（清单视为已预检）
        let all_terminal = run
            .steps
            .iter()
            .all(|st| matches!(st["status"].as_str(), Some("completed" | "failed")));
        if all_terminal {
            run.status = RunStatus::Completed;
            run.current_step_id = None;
            run.finished_at = Some(now_ms());
        }
        // 清单勾选与步骤状态严格一致（failed 步骤不得勾选）
        sync_checklist(run);
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

/// 清单与步骤状态一致性（Gap3）：令 `checklist[i].checked` 恒等于对应步骤
/// `status == "completed"`。
///
/// 修复「步骤 skipped/blocked/failed 但 checklist.checked=true」的自相矛盾：清单勾选
/// 只表示「该步骤已真实完成」，不再无条件全量勾选（原 `advance=true` 会把清单全部置勾）。
fn sync_checklist(run: &mut PlanRun) {
    let completed: std::collections::HashSet<String> = run
        .steps
        .iter()
        .filter(|st| st["status"].as_str() == Some("completed"))
        .filter_map(|st| st["stepId"].as_str().map(str::to_string))
        .collect();
    for item in run.checklist.iter_mut() {
        item.checked = completed.contains(&item.step_id);
    }
}

/// 多步前瞻：把观测与「当前步 + 窗口内后续未完成步」逐一做向量比对。
///
/// 只考察 `pending`/`in_progress`：已完成/已跳过/已失败的步骤不再是前瞻目标。
/// 返回项含 `distance`（0 = 当前步），调用方据此区分「仍在当前步」与「已推进到后续步」。
fn lookahead_hits(steps: &[Value], from: usize, obs: &str) -> Vec<LookaheadHit> {
    let backend = EmbeddingProvider::global();
    let obs_vec = backend.embed(obs);
    let mut out = Vec::new();
    for (offset, st) in steps.iter().skip(from).enumerate() {
        if !matches!(st["status"].as_str(), Some("pending" | "in_progress")) {
            continue;
        }
        let title = st["title"].as_str().unwrap_or("");
        if title.trim().is_empty() {
            continue;
        }
        out.push(LookaheadHit {
            step_id: st["stepId"].as_str().unwrap_or_default().to_string(),
            title: title.to_string(),
            similarity: backend.cosine(&obs_vec, &backend.embed(title)),
            distance: offset,
        });
        if out.len() >= REPLAN_LOOKAHEAD_WINDOW {
            break;
        }
    }
    out
}

/// 前瞻命中项 → 契约 JSON（webui `PlanReplanLookaheadHit`）
fn hit_json(h: &LookaheadHit) -> Value {
    json!({
        "stepId": h.step_id,
        "title": h.title,
        "similarity": h.similarity,
        "distance": h.distance,
    })
}

/// 前瞻判定（纯函数，语义由单测锁定）。
///
/// 当前步达标 → 仍在计划内；否则取窗口内相似度最高的**后续**步骤，须同时满足
/// 「达到偏离阈值」且「显著高于当前步」才判为已推进，否则判偏离、重新拆解。
fn decide_replan_mode(current_sim: f64, hits: &[LookaheadHit]) -> (ReplanMode, Option<usize>) {
    if current_sim >= REPLAN_THRESHOLD {
        return (ReplanMode::OnPlan, None);
    }
    let best = hits
        .iter()
        .enumerate()
        .filter(|(_, h)| h.distance >= 1 && h.similarity >= REPLAN_THRESHOLD)
        .max_by(|a, b| a.1.similarity.total_cmp(&b.1.similarity));
    match best {
        Some((idx, hit)) if hit.similarity >= current_sim + REPLAN_ADVANCE_MARGIN => {
            (ReplanMode::Advance, Some(idx))
        }
        _ => (ReplanMode::Rebuild, None),
    }
}

/// 代价模型的实测样本（毫秒）。
///
/// 优先级：并行批的**逐步实测**耗时（`parallel_batch.results[].elapsedMs`）→ 本运行
/// 墙钟均值（整轮实测 ÷ 已完成步数；串行路径没有逐步耗时，用整轮折算仍是实测而非
/// 常量）→ 无任何已完成样本时返回空（调用方据此回传 null，绝不虚报）。
fn cost_samples(run: &PlanRun, now: u64) -> Vec<u64> {
    let mut samples: Vec<u64> = run
        .parallel_batch
        .as_ref()
        .and_then(|b| b.get("results"))
        .and_then(Value::as_array)
        .map(|rs| {
            rs.iter()
                .filter(|r| r["status"].as_str() == Some("completed"))
                .filter_map(|r| r["elapsedMs"].as_u64())
                .collect()
        })
        .unwrap_or_default();
    if samples.is_empty() {
        let completed = run
            .steps
            .iter()
            .filter(|st| st["status"].as_str() == Some("completed"))
            .count();
        if completed > 0 {
            if let Some(started) = run.started_at {
                samples.push(now.saturating_sub(started) / completed as u64);
            }
        }
    }
    samples
}

/// 剩余代价估算（`run_view.cost` 与 `plans.replan` 的 `costBefore`/`costAfter` 同源）。
///
/// 剩余步数 = 仍需推进的 `pending`/`in_progress` 步骤；平均单步耗时与预计剩余均来自
/// [`cost_samples`] 的**实测**样本，无样本时为 `null`——前端只在
/// `estimatedRemainingMs` 非空时展示剩余时间，缺证据就不显示。
fn estimate_cost(run: &PlanRun, now: u64) -> Value {
    let remaining = run
        .steps
        .iter()
        .filter(|st| matches!(st["status"].as_str(), Some("pending" | "in_progress")))
        .count();
    let samples = cost_samples(run, now);
    let avg = if samples.is_empty() {
        None
    } else {
        Some(samples.iter().sum::<u64>() / samples.len() as u64)
    };
    json!({
        "remainingSteps": remaining,
        "sampleCount": samples.len(),
        "avgStepMs": avg,
        "estimatedRemainingMs": avg.map(|a| a.saturating_mul(remaining as u64)),
    })
}

/// 新计划的投影剩余代价：沿用当前实测平均单步耗时，步数换成新计划的步骤数。
///
/// 这是**投影**而非实测（新修订尚未执行）——`sampleCount` 如实回传样本来源，
/// 无样本时 avg/eta 仍为 null。
fn projected_cost(cost_before: &Value, steps: usize) -> Value {
    let avg = cost_before.get("avgStepMs").and_then(Value::as_u64);
    json!({
        "remainingSteps": steps,
        "sampleCount": cost_before.get("sampleCount").cloned().unwrap_or(json!(0)),
        "avgStepMs": avg,
        "estimatedRemainingMs": avg.map(|a| a.saturating_mul(steps as u64)),
    })
}

/// 确定性拆解：复用 Orchestrator 融合路由分析生成 PlanStep 序列 + Markdown 计划
/// 未显式给标题时的派生标题：取 prompt 前 40 个字符。
///
/// ⚠️ 这里曾硬编码中文占位 `"未命名计划"` —— 那是**引擎里的硬编码语言**：
/// 界面切成 en/ja/de 之后，计划卡标题仍会冒出中文。引擎不假设语言，
/// 用**用户自己的话**做标题在任何 locale 下都成立。
/// `plans.analyze`（只读预览）与 `plans.create`（落库）必须用同一个派生规则，
/// 否则预览与落库的卡片标题会不一致。
fn derived_title(prompt: &str) -> String {
    prompt.trim().chars().take(40).collect()
}

/// 确定性拆解结果：Markdown 正文（**仅**步骤列表）+ 步骤 + 编排形态键 + 决策理由。
///
/// A4：计划标题由 [`derived_title`] **单一派生**（卡片 `<h3>` 已承载标题与 `aria`
/// 名称），正文不再重复写 `# 计划：…`（旧实现两处截断长度还不同：40 vs 60）。
/// A5：编排形态只下发**机器键**，展示标签由前端按 i18n 渲染——正文里不得出现硬编码
/// 中文标签（`**编排形态**` / `**决策理由**`），否则 en/de/fr/es/ja 界面会冒中文。
struct Decomposition {
    markdown: String,
    steps: Vec<Value>,
    strategy: String,
    rationale: String,
}

/// 步骤角色徽标（**仅展示层**）。
///
/// 机器标识 `role` 是 [`crate::orchestration::SubtaskSpec`] 的依赖匹配键，**不得改动**；
/// 这里只把交付物角色映射为本地化类名，消除中英混排（`[检索]` / `[HTML]` / `[PPT]`）。
fn step_role_badge(role: &str) -> &str {
    match role {
        "HTML" => "网页",
        "PPT" => "演示文稿",
        other => other,
    }
}

fn decompose(prompt: &str) -> Decomposition {
    static ROUTER: OnceLock<Orchestrator> = OnceLock::new();
    let orch = ROUTER.get_or_init(|| Orchestrator::new(Config::default()));
    let a = orch.analyze(prompt);

    let mut steps: Vec<Value> = Vec::new();
    if a.subtasks.is_empty() {
        // Solo / 简单任务：最小三阶段计划（线性依赖，构成可可视化 DAG）
        for (i, t) in ["理解需求与约束", "实施与产出", "验证与收尾"]
            .iter()
            .enumerate()
        {
            steps.push(json!({
                "stepId": format!("step-{}", i + 1),
                "title": t,
                "details": if i == 0 { Some(prompt.to_string()) } else { None },
                "deps": if i == 0 { Vec::<Value>::new() } else { vec![json!(format!("step-{}", i))] },
            }));
        }
    } else {
        // 复杂任务：按子任务拆解（附角色）。M2-C2：由 Orchestrator 的
        // SubtaskSpec.deps（角色依赖）映射为步骤间 DAG（deps=前置 stepId 列表），
        // 供 WebUI 计划卡直接可视化；无依赖步骤自动并行。
        let mut role_to_idx: HashMap<&str, usize> = HashMap::new();
        for (i, s) in a.subtasks.iter().enumerate() {
            role_to_idx.entry(s.role).or_insert(i);
        }
        for (i, s) in a.subtasks.iter().enumerate() {
            let deps: Vec<Value> = s
                .deps
                .iter()
                .filter_map(|role| {
                    role_to_idx
                        .get(role.as_str())
                        .map(|idx| json!(format!("step-{}", idx + 1)))
                })
                .collect();
            steps.push(json!({
                "stepId": format!("step-{}", i + 1),
                // 步骤文案优先取 subtask.task（交付流水线的子任务已有明确 task）；
                // 为空时退化为角色名，避免出现「[角色] 」这种空标题。
                // 徽标用 `step_role_badge` 做展示层本地化，机器 role 保持不变。
                "title": format!(
                    "[{}] {}",
                    step_role_badge(s.role),
                    if s.task.trim().is_empty() { s.role } else { s.task.as_str() }
                ),
                "details": s.task,
                "deps": deps,
            }));
        }
    }

    // A4：正文只承载步骤列表。
    let mut markdown = String::new();
    for (i, s) in steps.iter().enumerate() {
        markdown.push_str(&format!(
            "{}. {}\n",
            i + 1,
            s["title"].as_str().unwrap_or("")
        ));
    }
    Decomposition {
        markdown,
        steps,
        strategy: a.strategy.as_key().to_string(),
        rationale: a.rationale,
    }
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
        "strategy": r.strategy,
        "rationale": r.rationale,
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
        // 代价模型：剩余步数 + 实测平均单步耗时 + 预计剩余（无样本时为 null）。
        // `step_progress` 事件流在页面重载/重连后缺位，快照侧带上同一口径的估算值，
        // 前端才能在任何时刻显示 ETA，而不是退化成纯进度条。
        "cost": estimate_cost(r, now_ms()),
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
    let d = decompose(prompt);
    json!({
        "revisionId": format!("rev-preview-{}", Uuid::new_v4()),
        "planId": format!("plan-preview-{}", Uuid::new_v4()),
        "title": derived_title(prompt),
        "markdown": d.markdown,
        "steps": d.steps,
        "strategy": d.strategy,
        "rationale": d.rationale,
        "current": true,
        "createdAt": now_ms(),
    })
}

/// `plans.create`：分析并持久化修订
///
/// D-05/T08：落新修订前按**需求指纹**查最近一次终态成功的同需求修订；命中则把
/// `reuseCandidate`（revisionId/title/createdAt）随响应返回（**不静默复用**——
/// 由前端「继续上次计划」按钮决定），供用户选择复用而非重新规划。
pub fn plans_create(params: &Value) -> Value {
    let prompt = params.get("prompt").and_then(Value::as_str).unwrap_or("");
    if prompt.trim().is_empty() {
        return json!({ "error": "prompt 不可为空" });
    }
    let title = params.get("title").and_then(Value::as_str);
    let fingerprint = normalize_fingerprint(prompt);
    let mut store = store().lock().unwrap_or_else(|e| e.into_inner());
    let candidate = store.find_latest_by_fingerprint(&fingerprint).map(|r| {
        json!({
            "revisionId": r.revision_id,
            "revision_id": r.revision_id,
            "title": r.title,
            "createdAt": r.created_at,
            "created_at": r.created_at,
        })
    });
    let rev = store.create_revision(prompt, title, None);
    drop(store);
    let mut out = json!({ "revision": revision_view(&rev) });
    if let Some(candidate) = candidate {
        out["reuseCandidate"] = candidate.clone();
        out["reuse_candidate"] = candidate;
    }
    out
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

/// 由步骤序列重建计划 Markdown（供步骤编辑派生修订复用；含依赖标注）。
///
/// A4：正文只承载步骤列表，不再写 `# 计划：{title}` —— 标题由卡片 `<h3>` 单一呈现。
fn steps_to_markdown(steps: &[Value]) -> String {
    let mut md = String::new();
    for (i, s) in steps.iter().enumerate() {
        let deps: Vec<String> = s["deps"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();
        let dep_note = if deps.is_empty() {
            String::new()
        } else {
            format!("（依赖 {}）", deps.join(" → "))
        };
        md.push_str(&format!(
            "{}. {}{}\n",
            i + 1,
            s["title"].as_str().unwrap_or(""),
            dep_note
        ));
    }
    md
}

/// M2-C2：计划步骤编辑动作（执行前增删改子任务）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StepEditAction {
    Add,
    Remove,
    Update,
}

/// 生成唯一步骤 id：给定（或生成）stepId 与已有步骤集合去冲突。
fn unique_step_id(base: &str, existing: &[Value]) -> String {
    let mut candidate = if base.trim().is_empty() {
        format!("step-{}", Uuid::new_v4().to_string().chars().take(6).collect::<String>())
    } else {
        base.to_string()
    };
    let taken: Vec<&str> = existing
        .iter()
        .filter_map(|s| s["stepId"].as_str())
        .collect();
    if taken.contains(&candidate.as_str()) {
        candidate = format!("{}-{}", candidate, Uuid::new_v4().to_string().chars().take(6).collect::<String>());
    }
    candidate
}

/// M2-C2：在既有修订上执行步骤级增删改，派生同 plan 新修订。
///
/// 参数动作（`params`，camelCase 对齐前端契约）：
/// - Add   ：`{ planRevisionId?, stepId?, stepTitle?, details?, deps?, beforeStepId? }`
///   （beforeStepId 可选，指定插入到该步骤之前；缺省追加到最后）
/// - Remove：`{ planRevisionId?, stepId }`（同时清理其它步对该 stepId 的依赖引用）
/// - Update：`{ planRevisionId?, stepId, title?, details?, deps? }`
fn edit_steps_on_store(
    s: &mut PlanStore,
    revision_id: &str,
    action: StepEditAction,
    params: &Value,
) -> Option<Value> {
    let rev = s.find_revision(revision_id)?.clone();
    let mut steps: Vec<Value> = rev.steps.clone();
    let title = params
        .get("title")
        .and_then(Value::as_str)
        .filter(|t| !t.trim().is_empty())
        .map(str::to_string)
        .unwrap_or(rev.title.clone());
    let step_id = params.get("stepId").and_then(Value::as_str).unwrap_or("");

    match action {
        StepEditAction::Add => {
            let new_id = unique_step_id(
                params
                    .get("stepId")
                    .and_then(Value::as_str)
                    .unwrap_or(""),
                &steps,
            );
            let deps: Vec<Value> = params
                .get("deps")
                .and_then(Value::as_array)
                .map(|arr| {
                    arr.iter()
                        .filter_map(Value::as_str)
                        .map(|s| json!(s))
                        .collect()
                })
                .unwrap_or_default();
            let new_step = json!({
                "stepId": new_id,
                "title": params.get("stepTitle").and_then(Value::as_str).unwrap_or("未命名步骤"),
                "details": params.get("details").and_then(Value::as_str),
                "deps": deps,
            });
            let before = params
                .get("beforeStepId")
                .and_then(Value::as_str)
                .unwrap_or("");
            if before.is_empty() {
                steps.push(new_step);
            } else if let Some(pos) = steps.iter().position(|st| st["stepId"].as_str() == Some(before)) {
                steps.insert(pos, new_step);
            } else {
                steps.push(new_step);
            }
        }
        StepEditAction::Remove => {
            if step_id.is_empty() {
                return None;
            }
            steps.retain(|st| st["stepId"].as_str() != Some(step_id));
            // 清理其它步骤对已删 stepId 的依赖引用（保持 DAG 无悬空边）
            for st in steps.iter_mut() {
                if let Some(arr) = st["deps"].as_array_mut() {
                    arr.retain(|d| d.as_str() != Some(step_id));
                }
            }
        }
        StepEditAction::Update => {
            if step_id.is_empty() {
                return None;
            }
            let target = steps
                .iter_mut()
                .find(|st| st["stepId"].as_str() == Some(step_id))?;
            if let Some(t) = params.get("title").and_then(Value::as_str) {
                target["title"] = json!(t);
            }
            if let Some(d) = params.get("details") {
                target["details"] = d.clone();
            }
            if let Some(deps) = params.get("deps").and_then(Value::as_array) {
                target["deps"] = json!(deps);
            }
        }
    }

    let child = s.spawn_child(&rev.revision_id, title, steps)?;
    Some(revision_view(&child))
}

/// `plans.step.add`：执行前为计划增加子任务（派生新修订，影响后续执行）
pub fn plans_step_add(params: &Value) -> Value {
    let revision_id = params
        .get("planRevisionId")
        .and_then(Value::as_str)
        .unwrap_or("");
    if revision_id.is_empty() {
        return json!({ "error": "planRevisionId 不可为空" });
    }
    let mut s = store().lock().unwrap_or_else(|e| e.into_inner());
    match edit_steps_on_store(&mut s, revision_id, StepEditAction::Add, params) {
        Some(rev) => json!({ "planRevision": rev }),
        None => json!({ "error": "Plan revision not found" }),
    }
}

/// `plans.step.remove`：执行前删除子任务（派生新修订，清理依赖引用）
pub fn plans_step_remove(params: &Value) -> Value {
    let revision_id = params
        .get("planRevisionId")
        .and_then(Value::as_str)
        .unwrap_or("");
    if revision_id.is_empty() {
        return json!({ "error": "planRevisionId 不可为空" });
    }
    let mut s = store().lock().unwrap_or_else(|e| e.into_inner());
    match edit_steps_on_store(&mut s, revision_id, StepEditAction::Remove, params) {
        Some(rev) => json!({ "planRevision": rev }),
        None => json!({ "error": "Plan revision not found" }),
    }
}

/// `plans.step.update`：执行前修改子任务（标题/详情/依赖，派生新修订）
pub fn plans_step_update(params: &Value) -> Value {
    let revision_id = params
        .get("planRevisionId")
        .and_then(Value::as_str)
        .unwrap_or("");
    if revision_id.is_empty() {
        return json!({ "error": "planRevisionId 不可为空" });
    }
    let mut s = store().lock().unwrap_or_else(|e| e.into_inner());
    match edit_steps_on_store(&mut s, revision_id, StepEditAction::Update, params) {
        Some(rev) => json!({ "planRevision": rev }),
        None => json!({ "error": "Plan revision not found" }),
    }
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
        Some(run) => {
            // M3-D1 产品事件：计划确认（开始执行）
            crate::events::product::emit(
                crate::events::product::PLAN_CONFIRMED,
                params.get("session").and_then(Value::as_str).unwrap_or("default"),
                json!({ "revision_id": revision_id, "run_id": run.run_id }),
            );
            json!({ "run": run_view(&run) })
        }
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

/// v3.3：查询运行的步骤进度（当前第几步 / 总步数）。
///
/// 供 runner 发布 `step_progress` 进度事件使用；运行不存在时返回 (0, 0)。
pub fn plan_run_progress(run_id: &str) -> (usize, usize) {
    let s = store().lock().unwrap_or_else(|e| e.into_inner());
    match s.runs.iter().find(|r| r.run_id == run_id) {
        Some(run) => {
            let total = run.steps.len();
            // 当前步 = 第一个非 completed 步骤（1-based）；全完成则返回 total
            let idx = run
                .steps
                .iter()
                .position(|st| st.get("status").and_then(Value::as_str) != Some("completed"))
                .map(|i| i + 1)
                .unwrap_or(total);
            (idx, total)
        }
        None => (0, 0),
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

/// `plans.replan`（B4 + 前瞻）：多步前瞻动态重规划
///
/// 参数：`runId` / `observation`（或 `prompt`）/ `title?` / `apply?`。
/// `apply=true` 才允许「跳步」这类破坏性动作落地（跳过已排定步骤）；默认只回报
/// 前瞻判定与代价估算，便于调用方先看后定。`rebuild`（生成新修订）不破坏运行实例，
/// 故不受 `apply` 约束。
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
    let apply = params
        .get("apply")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let mut s = store().lock().unwrap_or_else(|e| e.into_inner());
    match s.replan_run(run_id, observation, prompt, title, apply) {
        Some(outcome) => {
            let mut res = json!({
                "deviated": outcome.deviated,
                "similarity": outcome.similarity,
                "mode": outcome.mode.as_str(),
                "applied": outcome.applied,
                "lookahead": outcome.lookahead,
                "bestMatch": outcome.best_match,
                "skippedSteps": outcome.skipped_steps,
                "proposedSkippedStepIds": outcome.proposed_skipped,
                "blockedByDeps": outcome.blocked_by_deps,
                "costBefore": outcome.cost_before,
                "costAfter": outcome.cost_after,
            });
            if let Some(rev) = &outcome.revision {
                res["revision"] = revision_view(rev);
            }
            // 落地的判定必须回传运行快照（跳步改了步骤状态、rebuild 改了 run.replan）
            if outcome.applied {
                if let Some(run) = s.runs.iter().find(|r| r.run_id == run_id) {
                    res["run"] = run_view(run);
                }
            }
            res
        }
        None => json!({ "error": "运行不存在" }),
    }
}

/// 并行执行目标快照（runner 真执行用）：缺省取全部 pending 步骤。
pub(crate) fn parallel_snapshot(
    run_id: &str,
    step_ids: &[String],
) -> Option<Vec<(String, String)>> {
    store()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .parallel_targets(run_id, step_ids)
}

/// 并行真执行结果回填（runner 真执行用）：写步骤终态 + `run.parallel_batch`。
pub(crate) fn parallel_apply(run_id: &str, results: &[Value]) -> Option<PlanRun> {
    store()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .parallel_run_apply(run_id, results)
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
        let d = decompose("解释什么是闭包");
        let steps = d.steps;
        assert_eq!(steps.len(), 3);
        assert_eq!(steps[0]["stepId"], "step-1");
        // A4：正文只承载步骤列表，不再重复写 `# 计划：…` 标题行（标题由卡片 `<h3>` 呈现）。
        assert!(
            !d.markdown.contains("# 计划："),
            "方案正文不得重复计划标题：{}",
            d.markdown
        );
        // A4/A5：正文不得残留硬编码中文标签。
        assert!(!d.markdown.contains("**编排形态**"));
        assert!(!d.markdown.contains("**决策理由**"));
    }

    #[test]
    fn test_decompose_complex_task_multiple_steps() {
        let steps = decompose("实现登录模块、写单元测试并部署到服务器").steps;
        assert!(steps.len() >= 2, "复杂任务应拆解为多步: {}", steps.len());
        // 每步含角色前缀
        assert!(steps
            .iter()
            .any(|s| s["title"].as_str().is_some_and(|t| t.starts_with('['))));
    }

    /// A10 回归护栏：角色徽标只在**展示层**本地化，机器标识（[`SubtaskSpec::role`] /
    /// `deps` 匹配键）**一字不改**。
    ///
    /// 背景：`[检索]` / `[HTML]` / `[PPT]` 中英混排。若有人把 `step_role_badge` 的映射
    /// 删掉（或写宽），或把本地化误落到机器 `role` 上，本用例必须变红。
    #[test]
    fn a10_role_badge_localized_display_machine_roles_unchanged() {
        // 1) 展示层映射：交付物角色 → 本地化类名。
        assert_eq!(step_role_badge("HTML"), "网页", "HTML 徽标应本地化为「网页」");
        assert_eq!(
            step_role_badge("PPT"),
            "演示文稿",
            "PPT 徽标应本地化为「演示文稿」"
        );
        // 2) 其余角色必须**原样透传**（防映射写宽，把不该改的也改了）。
        for role in ["检索", "表格", "文档", "交付", "编码实现", "测试", "Research", "HTMLx"] {
            assert_eq!(
                step_role_badge(role),
                role,
                "非 HTML/PPT 角色应原样透传，不得改写：{role}"
            );
        }

        // 3) 机器标识不变：复合交付任务的 SubtaskSpec.role 仍是原始机器键（deps 匹配键）。
        const COMPOSITE: &str = "搜索今天的AI新闻，并整理成html网页再成PPT给我看";
        let c = crate::orchestration::deliverable::detect(COMPOSITE)
            .expect("复合交付任务应识别出交付物契约");
        let specs = crate::orchestration::deliverable::contract_to_specs(&c);
        let roles: Vec<&str> = specs.iter().map(|s| s.role).collect();
        assert_eq!(
            roles,
            vec!["检索", "HTML", "PPT"],
            "机器 role 必须保持原始标识（deps 匹配键），本地化只发生在展示层"
        );
        let deps: Vec<Vec<String>> = specs.iter().map(|s| s.deps.clone()).collect();
        assert_eq!(
            deps,
            vec![
                Vec::<String>::new(),
                vec!["检索".to_string()],
                vec!["HTML".to_string()],
            ],
            "SubtaskSpec.deps 仍由机器 role 解析（不因展示层本地化而漂移）"
        );

        // 4) 端到端：计划卡步骤展示本地化徽标，且 DAG deps 由机器 role 正确映射为 stepId。
        let steps = decompose(COMPOSITE).steps;
        let titles: Vec<String> = steps
            .iter()
            .map(|s| s["title"].as_str().unwrap_or_default().to_string())
            .collect();
        assert!(
            titles.iter().any(|t| t.starts_with("[网页]")),
            "HTML 步骤应展示本地化徽标「[网页]」，实际：{titles:?}"
        );
        assert!(
            titles.iter().any(|t| t.starts_with("[演示文稿]")),
            "PPT 步骤应展示本地化徽标「[演示文稿]」，实际：{titles:?}"
        );
        assert!(
            !titles
                .iter()
                .any(|t| t.starts_with("[HTML]") || t.starts_with("[PPT]")),
            "展示层不得残留中英混排的原始徽标「[HTML]」/「[PPT]」，实际：{titles:?}"
        );
        let html_idx = titles
            .iter()
            .position(|t| t.starts_with("[网页]"))
            .expect("应有本地化的 HTML 步骤");
        let ppt_idx = titles
            .iter()
            .position(|t| t.starts_with("[演示文稿]"))
            .expect("应有本地化的 PPT 步骤");
        assert_eq!(
            steps[ppt_idx]["deps"],
            json!(vec![format!("step-{}", html_idx + 1)]),
            "PPT 步骤的 DAG 依赖应由机器 role「HTML」映射到对应 stepId（证明机器标识未被本地化）"
        );
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

    /// Gap3：取消运行后，清单勾选必须与步骤状态一致（无完成步骤 → 全不勾选）。
    #[test]
    fn test_cancel_run_checklist_matches_step_status() {
        let mut s = PlanStore::default();
        let rev = s.create_revision("测试计划：重构并优化", None, None);
        let run = s.start_run(&rev.revision_id).unwrap();
        let cancelled = s.cancel_run(&run.run_id).expect("应能取消");
        assert!(
            cancelled.checklist.iter().all(|c| !c.checked),
            "取消后（无 completed 步骤）不得有勾选项：{:?}",
            cancelled.checklist
        );
        for st in &cancelled.steps {
            assert_ne!(
                st["status"].as_str(),
                Some("completed"),
                "取消后不得有 completed 步骤"
            );
        }
    }

    /// 未显式给标题时，标题必须由 prompt 派生，而不是硬编码的中文占位。
    ///
    /// 回归背景：`create_revision` 曾用 `"未命名计划"` 作兜底，于是计划卡标题
    /// 在任何非中文界面里都显示中文（实测在 en 界面截图里就是中文）。
    /// 同时钉住「引擎不硬编码语言」这条不变量。
    #[test]
    fn test_untitled_revision_derives_title_from_prompt() {
        let mut s = PlanStore::default();
        const CJK_START: char = '\u{4e00}';
        const CJK_END: char = '\u{9fff}';

        // 1) 无显式标题 -> 用 prompt 本身（用户自己的话）
        let prompt = "帮我做一个完善的 FPS 第一人称射击游戏";
        assert_eq!(s.create_revision(prompt, None, None).title, prompt);

        // 2) 不硬编码语言：英文 prompt 派生出的标题里不应出现 CJK
        let ascii_prompt = "build an FPS game in Rust";
        let ascii = s.create_revision(ascii_prompt, None, None);
        assert_eq!(ascii.title, ascii_prompt);
        assert!(
            !ascii.title.chars().any(|c| (CJK_START..=CJK_END).contains(&c)),
            "英文 prompt 派生的标题不应含中文：{}",
            ascii.title
        );

        // 3) 显式标题优先；超长 prompt 截到 40 字符（与 plans.analyze 预览一致）
        assert_eq!(
            s.create_revision(prompt, Some("自定义标题"), None).title,
            "自定义标题"
        );
        assert_eq!(
            s.create_revision(&"x".repeat(100), None, None)
                .title
                .chars()
                .count(),
            40
        );
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
        let outcome = s
            .replan_run(&run.run_id, &current_title, "", None, false)
            .unwrap();
        assert!(!outcome.deviated);
        assert_eq!(outcome.mode, ReplanMode::OnPlan);
        assert!(outcome.similarity > 0.9);
        assert!(outcome.revision.is_none());
        // 前瞻窗口含当前步（distance=0），未落地任何改动
        assert_eq!(outcome.lookahead[0]["distance"], 0);
        assert!(!outcome.applied);
        assert_eq!(outcome.cost_before, outcome.cost_after);
    }

    #[test]
    fn test_replan_deviated_creates_child_revision() {
        let mut s = PlanStore::default();
        let rev = s.create_revision("测试计划：重构并优化", None, None);
        let run = s.start_run(&rev.revision_id).unwrap();

        // 完全不相关观测 → 偏离 → 生成同 plan 子修订（谱系关联）
        let outcome = s
            .replan_run(&run.run_id, "遭遇网络超时，切换代理方案", "", None, false)
            .unwrap();
        assert!(outcome.deviated);
        assert_eq!(outcome.mode, ReplanMode::Rebuild);
        assert!(outcome.applied);
        assert!(outcome.similarity < 0.15);
        let child = outcome.revision.expect("偏离应生成新修订");
        assert_eq!(child.plan_id, rev.plan_id);
        assert_eq!(
            child.parent_revision_id.as_deref(),
            Some(rev.revision_id.as_str())
        );
        assert!(child.generation > rev.generation);
        // 运行实例记录 replan diff（含判定模式与代价模型）
        let run = s.runs.iter().find(|r| r.run_id == run.run_id).unwrap();
        let replan = run.replan.as_ref().expect("应记录 replan diff");
        assert_eq!(replan["previousRevisionId"], rev.revision_id);
        assert_eq!(replan["mode"], "rebuild");
        assert!(replan["costBefore"]["remainingSteps"].as_u64().unwrap() > 0);
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

    /// 构造受控修订：步骤标题用无交集 token，使向量余弦可预期（0 / 1）。
    fn store_with_steps(steps: Vec<Value>) -> (PlanStore, PlanRun) {
        let rev = PlanRevision {
            revision_id: "rev-fixed".to_string(),
            plan_id: "plan-fixed".to_string(),
            parent_revision_id: None,
            generation: 1,
            title: "controlled".to_string(),
            markdown: "controlled".to_string(),
            steps,
            strategy: None,
            rationale: None,
            fingerprint: None,
            current: true,
            created_at: 0,
        };
        let mut s = PlanStore {
            revisions: vec![rev],
            runs: Vec::new(),
            next_generation: 1,
        };
        let run = s.start_run("rev-fixed").expect("受控修订应能开跑");
        (s, run)
    }

    fn step(id: &str, title: &str, deps: Vec<&str>) -> Value {
        json!({ "stepId": id, "title": title, "deps": deps })
    }

    #[test]
    fn test_lookahead_window_bounded_and_skips_settled_steps() {
        let mut steps: Vec<Value> = (0..6)
            .map(|i| step(&format!("s{i}"), &format!("alpha{i} bravo{i}"), vec![]))
            .collect();
        // 已完成的步骤不再作为前瞻目标（不该被重新"命中"）
        steps[1]["status"] = json!("completed");
        for st in steps.iter_mut() {
            if st["status"].is_null() {
                st["status"] = json!("pending");
            }
        }
        let hits = lookahead_hits(&steps, 0, "alpha3 bravo3");
        assert_eq!(
            hits.len(),
            REPLAN_LOOKAHEAD_WINDOW,
            "前瞻窗口必须封顶，长计划不得整段参与判定"
        );
        assert!(
            hits.iter().all(|h| h.step_id != "s1"),
            "已完成步骤不得出现在前瞻窗口"
        );
        // distance 单调递增且 0 起（0 = 当前步）
        assert_eq!(hits[0].distance, 0);
        assert!(hits.windows(2).all(|w| w[0].distance < w[1].distance));
    }

    #[test]
    fn test_decide_replan_mode_semantics() {
        let hit = |id: &str, sim: f64, distance: usize| LookaheadHit {
            step_id: id.to_string(),
            title: id.to_string(),
            similarity: sim,
            distance,
        };
        // 当前步达标 → 计划内（即便后续步更相似也不跳）
        assert_eq!(
            decide_replan_mode(0.9, &[hit("a", 0.1, 0), hit("b", 1.0, 1)]),
            (ReplanMode::OnPlan, None)
        );
        // 当前步不达标、后续步显著命中 → 已推进（跳步），下标指向命中项
        let hits = vec![hit("a", 0.0, 0), hit("b", 0.05, 1), hit("c", 0.6, 2)];
        assert_eq!(decide_replan_mode(0.0, &hits), (ReplanMode::Advance, Some(2)));
        // 后续步全不达标 → 真偏离
        assert_eq!(
            decide_replan_mode(0.0, &[hit("a", 0.0, 0), hit("b", 0.05, 1)]),
            (ReplanMode::Rebuild, None)
        );
        // 仅命中当前步（distance=0）不算前瞻推进
        assert_eq!(
            decide_replan_mode(0.0, &[hit("a", 0.0, 0)]),
            (ReplanMode::Rebuild, None)
        );
        // 近义串味保护：命中步须显著高于当前步，微弱领先不足以跳步
        assert_eq!(
            decide_replan_mode(0.10, &[hit("a", 0.10, 0), hit("b", 0.16, 1)]),
            (ReplanMode::Rebuild, None)
        );
    }

    #[test]
    fn test_cost_model_never_fabricates_without_samples() {
        let (s, run) = store_with_steps(vec![
            step("s1", "alpha one", vec![]),
            step("s2", "bravo two", vec![]),
            step("s3", "charlie three", vec![]),
        ]);
        // 无任何已完成步骤 → 剩余步数可数，但平均/预计必须为 null（不虚报 ETA）
        let cost = estimate_cost(&run, run.started_at.unwrap_or(0) + 5_000);
        assert_eq!(cost["remainingSteps"], 3);
        assert_eq!(cost["sampleCount"], 0);
        assert!(cost["avgStepMs"].is_null());
        assert!(cost["estimatedRemainingMs"].is_null());

        // 并行批逐步实测样本 → 用真实均值外推（此处 1000/2000 → 均值 1500 × 剩 2 步）
        let mut s = s;
        s.runs[0].steps[0]["status"] = json!("completed");
        s.runs[0].parallel_batch = Some(json!({
            "results": [
                { "stepId": "s1", "status": "completed", "elapsedMs": 1000 },
                { "stepId": "s9", "status": "failed", "elapsedMs": 99999 },
            ],
        }));
        let cost = estimate_cost(&s.runs[0], run.started_at.unwrap_or(0) + 60_000);
        assert_eq!(cost["sampleCount"], 1, "失败步骤耗时不得计入样本");
        assert_eq!(cost["avgStepMs"], 1000);
        assert_eq!(cost["remainingSteps"], 2);
        assert_eq!(cost["estimatedRemainingMs"], 2000);
    }

    #[test]
    fn test_replan_advance_requires_apply_and_skips_intervening_steps() {
        let (mut s, run) = store_with_steps(vec![
            step("s1", "alpha bravo", vec![]),
            step("s2", "delta echo", vec![]),
            step("s3", "zulu yankee xray", vec![]),
        ]);
        // 观测命中第 3 步 → 前瞻判定「已推进」，但默认只回报建议、不改运行
        let dry = s
            .replan_run(&run.run_id, "zulu yankee xray", "", None, false)
            .unwrap();
        assert_eq!(dry.mode, ReplanMode::Advance);
        assert!(!dry.applied);
        assert_eq!(dry.best_match["stepId"], "s3");
        assert_eq!(dry.proposed_skipped, vec!["s1".to_string(), "s2".to_string()]);
        assert_eq!(dry.skipped_steps, 0);
        let untouched = s.runs.iter().find(|r| r.run_id == run.run_id).unwrap();
        assert_eq!(untouched.current_step_id.as_deref(), Some("s1"));
        assert!(untouched.replan.is_none(), "未落地不得写 run.replan");

        // apply=true → 落地跳步：其间未完成步骤置 skipped，命中步成为当前步
        let applied = s
            .replan_run(&run.run_id, "zulu yankee xray", "", None, true)
            .unwrap();
        assert!(applied.applied);
        assert_eq!(applied.skipped_steps, 2);
        let updated = s.runs.iter().find(|r| r.run_id == run.run_id).unwrap();
        assert_eq!(updated.current_step_id.as_deref(), Some("s3"));
        let status = |id: &str| {
            updated
                .steps
                .iter()
                .find(|st| st["stepId"].as_str() == Some(id))
                .and_then(|st| st["status"].as_str())
                .unwrap_or_default()
                .to_string()
        };
        assert_eq!(status("s1"), "skipped");
        assert_eq!(status("s2"), "skipped");
        assert_eq!(status("s3"), "in_progress");
        // 清单与步骤状态一致：被跳过的步骤不得显示为已勾选
        assert!(updated.checklist.iter().all(|c| !c.checked));
        let replan = updated.replan.as_ref().expect("落地跳步应记录 diff");
        assert_eq!(replan["mode"], "advance");
        assert_eq!(replan["stepsChanged"], 2);
        assert_eq!(replan["bestMatch"]["stepId"], "s3");
        assert_eq!(applied.cost_after["remainingSteps"], 1);
    }

    #[test]
    fn test_replan_advance_refuses_to_break_dependency_order() {
        // s3 依赖 s2：s2 未完成时跳过 s2 会破坏依赖序 → 只回报、不落地
        let (mut s, run) = store_with_steps(vec![
            step("s1", "alpha bravo", vec![]),
            step("s2", "delta echo", vec![]),
            step("s3", "zulu yankee xray", vec!["s2"]),
        ]);
        let outcome = s
            .replan_run(&run.run_id, "zulu yankee xray", "", None, true)
            .unwrap();
        assert_eq!(outcome.mode, ReplanMode::Advance);
        assert!(!outcome.applied, "前置未完成时不得落地跳步");
        assert_eq!(outcome.blocked_by_deps, vec!["s2".to_string()]);
        let untouched = s.runs.iter().find(|r| r.run_id == run.run_id).unwrap();
        assert_eq!(untouched.current_step_id.as_deref(), Some("s1"));
        assert!(untouched.replan.is_none());
    }

    #[test]
    fn test_plans_replan_rpc_validates_and_reports_unknown_run() {
        assert!(plans_replan(&json!({})).get("error").is_some());
        assert!(plans_replan(&json!({ "runId": "run-missing" }))
            .get("error")
            .is_some());
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

        // 全部成功：聚合 completed、successful=总数；离线路径显式 simulated=true
        let ok = s.parallel_run(&run.run_id, &ids, &[]).unwrap();
        let batch = ok.parallel_batch.expect("应记录 parallel_batch");
        assert_eq!(batch["status"], "completed");
        assert_eq!(batch["successful"], ids.len() as u64);
        assert_eq!(batch["failed"], 0);
        assert_eq!(batch["total"], ids.len() as u64);
        assert_eq!(batch["simulated"], true, "确定性扇出必须显式声明 simulated");

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
    fn test_parallel_snapshot_defaults_to_pending() {
        let mut s = PlanStore::default();
        let rev = s.create_revision("测试计划：重构并优化", None, None);
        let run = s.start_run(&rev.revision_id).unwrap();
        // start_run 后：step-1 in_progress、其余 pending → 快照应取全部 pending（排除 in_progress）
        let targets = s.parallel_targets(&run.run_id, &[]).unwrap();
        let pending_ids: Vec<&String> = targets.iter().map(|(sid, _)| sid).collect();
        assert!(pending_ids.iter().all(|sid| *sid != "step-1"));
        assert!(!targets.is_empty());
        // 指定 stepIds 时按存在的步骤取（含 in_progress 的 step-1）
        let all: Vec<String> = run
            .steps
            .iter()
            .filter_map(|st| st["stepId"].as_str().map(str::to_string))
            .collect();
        let explicit = s.parallel_targets(&run.run_id, &all).unwrap();
        assert_eq!(explicit.len(), run.steps.len());
        // 运行不存在 → None
        assert!(s.parallel_targets("run-missing", &[]).is_none());
    }

    #[test]
    fn test_parallel_run_apply_true_execution_results() {
        let mut s = PlanStore::default();
        let rev = s.create_revision("测试计划：重构并优化", None, None);
        let run = s.start_run(&rev.revision_id).unwrap();
        let results: Vec<Value> = run
            .steps
            .iter()
            .enumerate()
            .map(|(i, st)| {
                let ok = i > 0;
                json!({
                    "stepId": st["stepId"].as_str().unwrap_or_default(),
                    "title": st["title"].as_str().unwrap_or_default(),
                    "status": if ok { "completed" } else { "failed" },
                    "summary": if ok { "真实执行结论" } else { "执行失败：模拟网络错误" },
                })
            })
            .collect();
        let applied = s.parallel_run_apply(&run.run_id, &results).unwrap();
        // executed=true（与离线 simulated 区分）；统计来自真实结果
        let batch = applied.parallel_batch.unwrap();
        assert_eq!(batch["executed"], true);
        assert_eq!(batch["simulated"], Value::Null);
        assert_eq!(batch["successful"], (results.len() - 1) as u64);
        assert_eq!(batch["failed"], 1);
        // 步骤终态 + result 回填
        assert_eq!(applied.steps[0]["status"], "failed");
        assert!(applied.steps[0]["result"]
            .as_str()
            .unwrap()
            .contains("模拟网络错误"));
        assert_eq!(applied.steps[1]["status"], "completed");
        // 全部终态 → 运行收口（completed + current_step_id=None，对齐 deliveryReady）
        assert_eq!(applied.status.to_string(), "completed");
        assert!(applied.current_step_id.is_none());
        assert!(applied.finished_at.is_some());
        // Gap3：清单勾选必须与步骤状态严格一致——failed 步骤不得勾选，其余勾选。
        assert!(
            !applied.checklist[0].checked,
            "failed 步骤不得显示为已勾选（修复 checklist 与 steps 自相矛盾）"
        );
        assert!(applied.checklist[1..].iter().all(|c| c.checked));
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

    /// D-05/T08：指纹归一化——大小写/首尾与内部空白折叠后一致；固定 16 hex。
    #[test]
    fn normalize_fingerprint_is_case_and_whitespace_insensitive() {
        let a = normalize_fingerprint("  做一个  FPS  游戏 ");
        let b = normalize_fingerprint("做一个 fps 游戏");
        let c = normalize_fingerprint("做一个\nfps\t游戏");
        assert_eq!(a, b, "大小写与空白折叠后指纹必须一致");
        assert_eq!(a, c, "换行/制表符等空白同样折叠");
        assert_eq!(a.len(), 16, "指纹固定 16 hex（弱哈希去重用途）");
        assert_ne!(a, normalize_fingerprint("做一个赛车游戏"));
        assert!(a.chars().all(|ch| ch.is_ascii_hexdigit()));
    }

    /// D-05/T08：复用候选只认「有终态成功运行」的近期修订；无成功运行不算候选。
    #[test]
    fn find_latest_by_fingerprint_requires_completed_run() {
        let mut s = PlanStore::default();
        let fp = normalize_fingerprint("给我做一个日历应用");
        let older = s.create_revision("给我做一个日历应用", None, None);
        // 让 older 有一次 Completed 运行
        let run = s.start_run(&older.revision_id).expect("run");
        if let Some(r) = s.runs.iter_mut().find(|r| r.run_id == run.run_id) {
            r.status = RunStatus::Completed;
        }
        // 同需求再规划一次：新修订尚无成功运行，不应盖过 older。
        let newer = s.create_revision("给我做一个日历应用", None, None);
        assert_eq!(
            s.find_latest_by_fingerprint(&fp).map(|r| r.revision_id.clone()),
            Some(older.revision_id.clone()),
            "只认有终态成功运行的近期修订"
        );
        // 给 newer 也补一次成功运行 → 取更近的修订。
        let run2 = s.start_run(&newer.revision_id).expect("run2");
        if let Some(r) = s.runs.iter_mut().find(|r| r.run_id == run2.run_id) {
            r.status = RunStatus::Completed;
        }
        assert_eq!(
            s.find_latest_by_fingerprint(&fp).map(|r| r.revision_id.clone()),
            Some(newer.revision_id.clone()),
            "同需求取最近一次成功修订"
        );
        // 不同需求指纹无候选；旧修订无指纹（None）不参与。
        assert!(s.find_latest_by_fingerprint("deadbeefdeadbeef").is_none());
    }

    /// D-05/T08 真实入口：`plans.create` 同需求第二次提交时返回 `reuseCandidate`
    /// （前提是首次计划已有一次终态成功运行）——且不静默复用（仍返回全新修订）。
    #[test]
    fn plans_create_offers_reuse_candidate_on_second_identical_prompt() {
        let prompt = format!("T08 复用候选测试 {}", Uuid::new_v4());
        let first = plans_create(&json!({ "prompt": prompt }));
        let rev1 = first["revision"]["revisionId"]
            .as_str()
            .expect("首次应返回 revision")
            .to_string();
        assert!(
            first.get("reuseCandidate").is_none(),
            "首次规划无同需求成功修订 → 无候选"
        );

        // 让 rev1 拥有一次终态成功运行（真实 store + start_run）。
        {
            let mut s = store().lock().unwrap_or_else(|e| e.into_inner());
            let run = s.start_run(&rev1).expect("应能启动运行");
            if let Some(r) = s.runs.iter_mut().find(|r| r.run_id == run.run_id) {
                r.status = RunStatus::Completed;
            }
            s.save();
        }

        let second = plans_create(&json!({ "prompt": prompt }));
        assert_eq!(
            second["reuseCandidate"]["revisionId"], rev1,
            "第二次同需求提交应给出复用候选（且不静默复用，仍返回新 revision）"
        );
        assert_ne!(
            second["revision"]["revisionId"], rev1,
            "候选不等于自动复用：仍生成全新修订，由用户决定是否继续上次"
        );
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

    // ── M2-C2 计划编辑（执行前增删改 + DAG 可视化数据）──

    #[test]
    fn test_decompose_solo_steps_form_linear_dag() {
        let steps = decompose("解释什么是闭包").steps;
        assert_eq!(steps.len(), 3);
        // 线性链：step-2 依赖 step-1，step-3 依赖 step-2，step-1 无依赖
        assert!(steps[0]["deps"].as_array().unwrap().is_empty());
        assert_eq!(steps[1]["deps"][0], "step-1");
        assert_eq!(steps[2]["deps"][0], "step-2");
        // deps 引用的 stepId 全部真实存在（DAG 无悬空边）
        let ids: std::collections::HashSet<&str> = steps
            .iter()
            .filter_map(|s| s["stepId"].as_str())
            .collect();
        for s in steps.iter() {
            for d in s["deps"].as_array().unwrap() {
                assert!(ids.contains(d.as_str().unwrap()), "悬空依赖 {}", d);
            }
        }
    }

    #[test]
    fn test_decompose_complex_steps_deps_reference_existing_steps() {
        let steps = decompose("实现登录模块、写单元测试并部署到服务器").steps;
        assert!(steps.len() >= 2);
        let ids: std::collections::HashSet<&str> = steps
            .iter()
            .filter_map(|s| s["stepId"].as_str())
            .collect();
        for s in steps.iter() {
            for d in s["deps"].as_array().unwrap() {
                assert!(ids.contains(d.as_str().unwrap()), "悬空依赖 {}", d);
            }
        }
    }

    #[test]
    fn test_step_edit_add_remove_update_end_to_end() {
        let mut s = PlanStore::default();
        let parent = s.create_revision("做一个 Markdown 转 PDF 工具", None, None);
        let parent_id = parent.revision_id.clone();
        let original_len = parent.steps.len();
        assert!(original_len >= 2);

        // 1) 执行前新增子任务（追加到最后）
        let after_add = edit_steps_on_store(
            &mut s,
            &parent_id,
            StepEditAction::Add,
            &json!({
                "stepTitle": "打包为独立可执行文件",
                "details": "生成 exe 供无环境机器使用",
                "deps": [format!("step-{}", original_len)],
            }),
        )
        .expect("add 应成功");
        let add_id = after_add["revisionId"].as_str().unwrap().to_string();
        let add_steps = after_add["steps"].as_array().unwrap();
        assert_eq!(add_steps.len(), original_len + 1);
        assert_eq!(
            add_steps.last().unwrap()["title"],
            "打包为独立可执行文件"
        );
        // 派生修订谱系：同 plan + generation 递增 + current 唯一
        assert_eq!(after_add["planId"], parent.plan_id);
        assert_eq!(
            after_add["generation"].as_u64().unwrap(),
            parent.generation as u64 + 1
        );
        assert_eq!(after_add["parentRevisionId"], parent_id);

        // 2) 新增步骤执行前被删除
        let after_remove = edit_steps_on_store(
            &mut s,
            &add_id,
            StepEditAction::Remove,
            &json!({ "stepId": add_steps.last().unwrap()["stepId"] }),
        )
        .expect("remove 应成功");
        assert_eq!(
            after_remove["steps"].as_array().unwrap().len(),
            original_len
        );
        // stepId 引用唯一：删除的 id 不再存在
        let remove_id = after_remove["revisionId"].as_str().unwrap().to_string();

        // 3) 修改既有步骤标题/依赖
        let after_update = edit_steps_on_store(
            &mut s,
            &remove_id,
            StepEditAction::Update,
            &json!({
                "stepId": format!("step-{}", original_len),
                "title": "校验转换结果并输出报告",
                "deps": [],
            }),
        )
        .expect("update 应成功");
        let update_id = after_update["revisionId"].as_str().unwrap().to_string();
        let last = after_update["steps"].as_array().unwrap().last().unwrap();
        assert_eq!(last["title"], "校验转换结果并输出报告");
        assert!(last["deps"].as_array().unwrap().is_empty());

        // 4) 计划编辑影响后续 Orchestrator 执行：start_run 消费编辑后 steps
        let run = s.start_run(&update_id).expect("应能为编辑后修订启动运行");
        assert_eq!(run.steps.len(), original_len);
        assert!(run.steps[original_len - 1]["title"]
            .as_str()
            .unwrap()
            .contains("校验转换结果"));
        // 编辑派生链上旧修订 current 均置 false，最终修订 current=true
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
                .find(|r| r.revision_id == update_id)
                .unwrap()
                .current
        );
    }

    #[test]
    fn test_step_remove_cleans_up_dangling_dependencies() {
        let mut s = PlanStore::default();
        let parent = s.create_revision("做一个 FPS 游戏", None, None);
        let parent_id = parent.revision_id.clone();
        // 线性链（步骤 i 依赖 step-(i-1)）→ 删除中间步骤后，后置步骤依赖被清理或前移
        let mid_id = parent.steps[1]["stepId"].as_str().unwrap();
        let after = edit_steps_on_store(
            &mut s,
            &parent_id,
            StepEditAction::Remove,
            &json!({ "stepId": mid_id }),
        )
        .expect("remove 应成功");
        let steps = after["steps"].as_array().unwrap();
        assert_eq!(steps.len(), parent.steps.len() - 1);
        // 不存在任何对已删 stepId 的依赖（parse stepId 引用集合并校验）
        let removed = ["step-1", "step-2", "step-3"]; // 中间步骤触发清理后索引变化，直接校验无悬空
        for id in removed.iter() {
            for st in steps.iter() {
                for d in st["deps"].as_array().unwrap() {
                    assert!(
                        *d != json!(id),
                        "删除 {id} 后仍存在悬空依赖 in step {}",
                        st["stepId"]
                    );
                }
            }
        }
    }

    #[test]
    fn test_plans_step_edit_validation() {
        // 缺 planRevisionId → 报错
        for (name, f) in [
            ("add", plans_step_add as fn(&Value) -> Value),
            ("remove", plans_step_remove),
            ("update", plans_step_update),
        ] {
            let err = f(&json!({}));
            assert!(err.get("error").is_some(), "{name} 缺参应报错");
            let missing = f(&json!({
                "planRevisionId": "rev-573410db-0000-0000-0000-000000000000",
                "stepId": "step-1",
                "stepTitle": "x",
            }));
            assert!(
                missing["error"]
                    .as_str()
                    .unwrap()
                    .contains("Plan revision not found"),
                "{name} 未知修订应报 Plan revision not found"
            );
        }
        // remove/update 缺 stepId → 报错
        let mut s = PlanStore::default();
        let rev = s.create_revision("测试计划", None, None);
        assert!(
            edit_steps_on_store(&mut s, &rev.revision_id, StepEditAction::Remove, &json!({}))
                .is_none()
        );
        assert!(
            edit_steps_on_store(&mut s, &rev.revision_id, StepEditAction::Update, &json!({}))
                .is_none()
        );
    }
}
