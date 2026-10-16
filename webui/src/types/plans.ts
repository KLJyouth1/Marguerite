export interface PlanRevisionStep {
  stepId: string
  title: string
  details?: string
  /** M2-C2：依赖的前置 stepId 列表（可视化 DAG 边；空数组=无依赖可并行） */
  deps?: string[]
}

/**
 * D-05/T08：同需求「上次跑通」计划的复用候选（随 `session.event.plan_revision`
 * 的 `reuse_candidate` 下发）。**非静默复用**——由 PlanCard 的「继续上次计划」
 * 按钮决定是否走 `plans.implement` 复用该 revision。
 */
export interface PlanReuseCandidate {
  revisionId: string
  title: string
  createdAt?: number
}

export interface PlanRevisionSnapshot {
  revisionId: string
  planId: string
  parentRevisionId?: string
  generation?: number
  title: string
  markdown: string
  steps: PlanRevisionStep[]
  /**
   * A5：编排形态**机器键**（如 `'pipeline'` / `'solo'`）。展示名由 `PlanCard` 按
   * i18n 渲染；旧修订可能缺省该字段。
   */
  strategy?: string
  /** A5：决策理由（引擎自述文本）。旧修订可能缺省该字段。 */
  rationale?: string
  /** D-05/T08：复用候选（仅在同需求指纹命中上次成功计划时出现）。 */
  reuseCandidate?: PlanReuseCandidate
  current: boolean
  createdAt?: number
}

export type CollaborationMode = 'default' | 'plan'

export interface CollaborationSnapshot {
  mode: CollaborationMode
  revision: number
}

export type PlanRunStatus =
  | 'queued'
  | 'running'
  | 'paused'
  | 'blocked'
  | 'completed'
  | 'cancelled'
  | 'superseded'

export type PlanRunStepStatus =
  | 'pending'
  | 'in_progress'
  | 'completed'
  | 'blocked'
  | 'skipped'

export interface PlanRunStepSnapshot {
  stepId: string
  title: string
  status: PlanRunStepStatus
  reason?: string
}

/** v3.3：步骤开始进度事件（Rust runner `step_progress`，Progress 主题） */
export interface StepProgressPayload {
  kind: 'step_progress'
  stepId: string
  stepTitle: string
  stepIndex: number
  stepsTotal: number
  percent: number
  phase: 'started' | 'finished'
  /** 任务类型徽章（编码/修复/测试/文档/检索/对话…） */
  taskKind?: string
  /** M2-C3：真实耗时/预计剩余（core runner 基于每步实测耗时估算） */
  elapsedMs?: number
  /** M2-C3：预计剩余毫秒（估算值，异常时缺省） */
  etaMs?: number
  /** M2-C3：单步平均耗时（毫秒，用于进度估算） */
  avgStepMs?: number
}

/** M2-C3：计划运行实时时序（事件驱动，来自 `step_progress` 状态流） */
export interface PlanRunTiming {
  elapsedMs: number
  etaMs?: number
  avgStepMs?: number
  stepIndex: number
  stepsTotal: number
  phase: 'started' | 'finished'
  stepTitle: string
  /** N1：本地收到该 `elapsedMs` 的时刻（毫秒）。UI 据此在事件之间续算「已用」，
   *  避免进度条显示冻结（思考已 173s 而「已用」仍停在 1m16s）。 */
  atMs?: number
}

/** 执行前清单项（B4：`plans.run` 前置生成，逐项勾选推进） */
export interface PlanRunChecklistItem {
  stepId: string
  title: string
  checked: boolean
}

/** 并行子任务批（B4：`plans.parallel.run` 扇出聚合结果）
 *  v19 真执行化：`executed=true` 表示经 Orchestrator 真实执行；
 *  `simulated=true` 为离线回退的确定性标记（未真正执行，如实展示）。 */
export interface PlanParallelBatch {
  status: 'completed' | 'failed'
  successful: number
  failed: number
  total: number
  results: PlanParallelSubResult[]
  executed?: boolean
  simulated?: boolean
}

export interface PlanParallelSubResult {
  stepId: string
  title: string
  status: 'completed' | 'failed'
  summary: string
  /** 本步真实墙钟耗时（core runner 实测，含退避重试等待）；缺省 = 无数据，不显示 */
  elapsedMs?: number
  /** 实际尝试次数（1 = 一次通过；>1 表示发生过退避重试） */
  attempts?: number
  /** 是否因耗尽单步总预算被判超时（与「远端明确报错」区分） */
  timedOut?: boolean
}

/** 重规划判定模式（core `ReplanMode`） */
export type PlanReplanMode = 'on_plan' | 'advance' | 'rebuild'

/** 前瞻命中项（观测与某步骤的真实向量相似度） */
export interface PlanReplanLookaheadHit {
  stepId: string
  title: string
  similarity: number
  /** 相对当前步的前向距离（0 = 当前步本身） */
  distance: number
}

/** 动态重规划 diff（B4：`plans.replan` 偏离/跳步时记录） */
export interface PlanReplanInfo {
  revisionId: string
  planId: string
  previousRevisionId: string
  similarity: number
  prompt: string
  stepsChanged: number
  createdAt?: number
  /** 判定模式：`rebuild` 重新拆解 / `advance` 前瞻跳步 */
  mode?: PlanReplanMode
  /** `advance` 判定下被跳过的步骤 id */
  skippedStepIds?: string[]
  /** 前瞻命中步（`advance` 时即跳步目标） */
  bestMatch?: PlanReplanLookaheadHit
}

/** 运行代价模型（core `estimate_cost`：实测样本；无样本时 avg/eta 为 null） */
export interface PlanRunCost {
  /** 仍需推进的步骤数 */
  remainingSteps: number
  /** 实测样本条数（0 = 无证据，不显示 ETA） */
  sampleCount: number
  avgStepMs: number | null
  estimatedRemainingMs: number | null
}

export interface PlanRunSnapshot {
  runId: string
  planRevisionId: string
  status: PlanRunStatus
  currentStepId?: string
  stateRevision?: number
  /** Execution owner. `manual` today; a future durable goal driver can reuse the same run. */
  driverKind?: string
  driverId?: string
  activeTaskId?: string
  pauseReason?: string
  terminalReason?: string
  createdAt?: number
  updatedAt?: number
  startedAt?: number
  finishedAt?: number
  steps: PlanRunStepSnapshot[]
  /** 执行前清单（B4） */
  checklist?: PlanRunChecklistItem[]
  /** 最近并行子任务批（B4） */
  parallelBatch?: PlanParallelBatch
  /** 最近重规划 diff（B4） */
  replan?: PlanReplanInfo
  /** 剩余代价模型（实测样本；事件流缺位时前端据此兜底显示 ETA） */
  cost?: PlanRunCost
}

export type PlanCardAction = 'implement-current' | 'implement-new' | 'replan'

export interface PlanCardActionTarget {
  planId: string
  revisionId: string
}

export interface PlanRevisionRequest extends PlanCardActionTarget {
  prompt: string
}
