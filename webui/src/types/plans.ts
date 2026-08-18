export interface PlanRevisionStep {
  stepId: string
  title: string
  details?: string
}

export interface PlanRevisionSnapshot {
  revisionId: string
  planId: string
  parentRevisionId?: string
  generation?: number
  title: string
  markdown: string
  steps: PlanRevisionStep[]
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

/** 执行前清单项（B4：`plans.run` 前置生成，逐项勾选推进） */
export interface PlanRunChecklistItem {
  stepId: string
  title: string
  checked: boolean
}

/** 并行子任务批（B4：`plans.parallel.run` 扇出聚合结果） */
export interface PlanParallelBatch {
  status: 'completed' | 'failed'
  successful: number
  failed: number
  total: number
  results: PlanParallelSubResult[]
}

export interface PlanParallelSubResult {
  stepId: string
  title: string
  status: 'completed' | 'failed'
  summary: string
}

/** 动态重规划 diff（B4：`plans.replan` 偏离时记录） */
export interface PlanReplanInfo {
  revisionId: string
  planId: string
  previousRevisionId: string
  similarity: number
  prompt: string
  stepsChanged: number
  createdAt?: number
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
}

export type PlanCardAction = 'implement-current' | 'implement-new' | 'replan'

export interface PlanCardActionTarget {
  planId: string
  revisionId: string
}

export interface PlanRevisionRequest extends PlanCardActionTarget {
  prompt: string
}
