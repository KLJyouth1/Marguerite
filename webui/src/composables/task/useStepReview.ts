/**
 * A4 交付流水线「待审查门」——前端审查台账（诚实边界版）。
 *
 * 数据面事实（本轮核对过 core）：
 * - chat 编排链（step_start/step_finish）由后端连续推进，**没有步间暂停点**；
 *   前端无法在不新增 core 契约的前提下真正「挡住下一步」。
 * - `plans.run.advance`（B4）本身就是人推进步语义（RunStatus::Paused/Blocked 存在），
 *   plan-run 面的人审已由 core 提供。
 * - 因此本轮实现的是**人工审查台账**：每个 done 且落盘产物的步骤进入「待审查」，
 *   由人给出 通过/打回 结论并持久化（localStorage，按 run 分桶）。
 *   UI 明示「审查不会暂停后端流水线」——不谎称阻断（铁律 R-10）。
 *   core 侧的硬门（步间暂停等人）与 PlanGate「只审计」取舍重评登记在台账 Z-I5。
 */

import { ref } from 'vue'
import type { Ref } from 'vue'
import type { TaskStep } from '@/types/rpc'

export type StepReviewVerdict = 'approved' | 'flagged'

export interface StepReviewRecord {
  verdict: StepReviewVerdict
  /** 打回时的说明（人写什么就存什么，不加工）。 */
  note: string
  /** 结论时刻（Date.now()）。 */
  at: number
}

/** runKey → stepId → 审查结论。runKey = `${sessionKey}::${taskId}`（缺段用空串占位）。 */
export type StepReviewStore = Record<string, Record<string, StepReviewRecord>>

export const STEP_REVIEW_STORAGE_KEY = 'marguerite.task.review.v1'

/** 台账容量上限：超出后丢弃最旧的记录（审查台账是工作记忆，不是审计归档）。 */
export const STEP_REVIEW_MAX_RECORDS = 200

type MinimalStorage = Pick<Storage, 'getItem' | 'setItem'>

function defaultStorage(): MinimalStorage | null {
  try {
    if (typeof localStorage === 'undefined') return null
    return localStorage
  } catch {
    return null
  }
}

/** runKey 组装：两段都可缺（历史运行无 taskId），全空落 'current' 桶。 */
export function stepReviewRunKey(sessionKey: string, taskId: string): string {
  const key = [sessionKey || '', taskId || ''].join('::')
  return key === '::' ? 'current' : key
}

/** 「待审查」的唯一定义：步骤真实完成且产物已落盘。失败/纯研究步骤不进审查。 */
export function needsStepReview(step: TaskStep): boolean {
  return step.status === 'done' && !!step.artifactPath
}

function normalizeRecord(raw: unknown): StepReviewRecord | null {
  if (!raw || typeof raw !== 'object') return null
  const row = raw as Record<string, unknown>
  if (row.verdict !== 'approved' && row.verdict !== 'flagged') return null
  const at = typeof row.at === 'number' && Number.isFinite(row.at) ? row.at : null
  if (at === null) return null
  return { verdict: row.verdict, note: typeof row.note === 'string' ? row.note : '', at }
}

/** 归一化：坏形状整条丢弃；超出容量按 `at` 丢最旧。 */
export function normalizeStepReviewStore(raw: unknown): StepReviewStore {
  if (!raw || typeof raw !== 'object') return {}
  const source = raw as Record<string, unknown>
  const records: Array<{ runKey: string; stepId: string; record: StepReviewRecord }> = []
  for (const [runKey, bucket] of Object.entries(source)) {
    if (!bucket || typeof bucket !== 'object') continue
    for (const [stepId, entry] of Object.entries(bucket as Record<string, unknown>)) {
      const record = normalizeRecord(entry)
      if (record) records.push({ runKey, stepId, record })
    }
  }
  records.sort((a, b) => a.record.at - b.record.at)
  const kept = records.slice(-STEP_REVIEW_MAX_RECORDS)
  const store: StepReviewStore = {}
  for (const { runKey, stepId, record } of kept) {
    store[runKey] = store[runKey] ?? {}
    store[runKey][stepId] = record
  }
  return store
}

export function loadStepReviewStore(storage: MinimalStorage | null = defaultStorage()): StepReviewStore {
  try {
    const raw = storage?.getItem(STEP_REVIEW_STORAGE_KEY)
    if (!raw) return {}
    return normalizeStepReviewStore(JSON.parse(raw))
  } catch {
    return {}
  }
}

export function saveStepReviewStore(
  store: StepReviewStore,
  storage: MinimalStorage | null = defaultStorage(),
): void {
  try {
    storage?.setItem(STEP_REVIEW_STORAGE_KEY, JSON.stringify(store))
  } catch {
    /* 偏好写失败不打断审查动作 */
  }
}

export function createStepReview() {
  const reviews: Ref<StepReviewStore> = ref(loadStepReviewStore())

  function reviewFor(runKey: string, stepId: string): StepReviewRecord | null {
    return reviews.value[runKey]?.[stepId] ?? null
  }

  function setReview(runKey: string, stepId: string, verdict: StepReviewVerdict, note = ''): void {
    if (!runKey || !stepId) return
    const bucket = { ...(reviews.value[runKey] ?? {}) }
    bucket[stepId] = { verdict, note, at: Date.now() }
    reviews.value = normalizeStepReviewStore({ ...reviews.value, [runKey]: bucket })
    saveStepReviewStore(reviews.value)
  }

  /** 当前 run 里仍待审查的步骤 id（done + 有产物 + 无结论）。 */
  function pendingReviewIds(steps: readonly TaskStep[], runKey: string): string[] {
    const bucket = reviews.value[runKey] ?? {}
    return steps.filter((step) => needsStepReview(step) && !bucket[step.id]).map((step) => step.id)
  }

  return { reviews, reviewFor, setReview, pendingReviewIds }
}

export type StepReview = ReturnType<typeof createStepReview>
