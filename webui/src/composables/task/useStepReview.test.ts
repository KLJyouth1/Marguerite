// useStepReview 纯函数层：待审查判定 / 台账归一化与持久化 / 容量与坏数据自愈。
// node 环境：所有存储交互通过注入 storage 验证（defaultStorage 在 node 下安全降级为 null）。
import { describe, expect, it } from 'vitest'
import type { TaskStep } from '@/types/rpc'
import {
  STEP_REVIEW_MAX_RECORDS,
  STEP_REVIEW_STORAGE_KEY,
  createStepReview,
  loadStepReviewStore,
  needsStepReview,
  normalizeStepReviewStore,
  saveStepReviewStore,
  stepReviewRunKey,
} from '@/composables/task/useStepReview'

function step(overrides: Partial<TaskStep> = {}): TaskStep {
  return {
    id: 'd1',
    role: 'writer',
    title: 'Draft',
    kind: 'Document',
    targetPath: 'out/draft.md',
    // 默认已落盘：done + artifact 才进待审查（单测逐条覆盖缺产物/失败等分支）
    artifactPath: 'out/draft.md',
    dependsOn: [],
    status: 'done',
    ...overrides,
  }
}

function memoryStorage(initial: Record<string, string> = {}): {
  store: Record<string, string>
  storage: Pick<Storage, 'getItem' | 'setItem'>
} {
  const store: Record<string, string> = { ...initial }
  return {
    store,
    storage: {
      getItem: (key: string) => (key in store ? store[key] : null),
      setItem: (key: string, value: string) => {
        store[key] = value
      },
    },
  }
}

describe('needsStepReview', () => {
  it('requires a real completion with the artifact on disk', () => {
    expect(needsStepReview(step())).toBe(true)
    expect(needsStepReview(step({ status: 'done', artifactPath: null }))).toBe(false)
    expect(needsStepReview(step({ status: 'failed' }))).toBe(false)
    expect(needsStepReview(step({ status: 'in_progress' }))).toBe(false)
    expect(needsStepReview(step({ status: 'pending' }))).toBe(false)
  })
})

describe('stepReviewRunKey', () => {
  it('composes session and task, falling back to a single bucket when both are empty', () => {
    expect(stepReviewRunKey('agent:a:webchat:x', 'task-9')).toBe('agent:a:webchat:x::task-9')
    expect(stepReviewRunKey('', 'task-9')).toBe('::task-9')
    expect(stepReviewRunKey('', '')).toBe('current')
  })
})

describe('normalizeStepReviewStore', () => {
  it('drops malformed records instead of trusting storage', () => {
    const store = normalizeStepReviewStore({
      run1: {
        d1: { verdict: 'approved', note: 'ok', at: 10 },
        d2: { verdict: 'maybe', at: 11 },
        d3: { verdict: 'flagged' },
        d4: 'junk',
      },
      bad: 'not-a-bucket',
    })
    expect(Object.keys(store)).toEqual(['run1'])
    expect(store.run1.d1).toEqual({ verdict: 'approved', note: 'ok', at: 10 })
    expect(store.run1.d2).toBeUndefined()
    expect(store.run1.d3).toBeUndefined()
    expect(store.run1.d4).toBeUndefined()
  })

  it('caps the ledger by dropping the oldest records', () => {
    const raw: Record<string, Record<string, unknown>> = {}
    for (let i = 0; i < STEP_REVIEW_MAX_RECORDS + 5; i += 1) {
      raw[`s${i}`] = { verdict: 'approved', note: '', at: i }
    }
    const store = normalizeStepReviewStore({ run: raw })
    const ids = Object.keys(store.run)
    expect(ids).toHaveLength(STEP_REVIEW_MAX_RECORDS)
    // 205 条留最新 200 条：at=0..4（s0..s4）被丢，首条是 s5
    expect(ids[0]).toBe('s5')
  })

  it('treats non-object input as an empty ledger', () => {
    expect(normalizeStepReviewStore(null)).toEqual({})
    expect(normalizeStepReviewStore('junk')).toEqual({})
  })
})

describe('persistence round-trip (injected storage)', () => {
  it('saves and reloads verdicts under the documented key', () => {
    const { storage, store } = memoryStorage()
    saveStepReviewStore(
      normalizeStepReviewStore({
        'run-a': {
          d1: { verdict: 'approved', note: '', at: 5 },
          d2: { verdict: 'flagged', note: '数据不对', at: 6 },
        },
      }),
      storage,
    )
    expect(store[STEP_REVIEW_STORAGE_KEY]).toBeTruthy()
    const loaded = loadStepReviewStore(storage)
    expect(loaded['run-a'].d1.verdict).toBe('approved')
    expect(loaded['run-a'].d2.note).toBe('数据不对')
  })

  it('self-heals from corrupted or missing payloads', () => {
    expect(loadStepReviewStore(memoryStorage().storage)).toEqual({})
    expect(loadStepReviewStore(memoryStorage({ [STEP_REVIEW_STORAGE_KEY]: '{not json' }).storage)).toEqual({})
    expect(loadStepReviewStore(memoryStorage({ [STEP_REVIEW_STORAGE_KEY]: '[1,2]' }).storage)).toEqual({})
  })

  it('survives a throwing storage without breaking the caller', () => {
    const throwing = {
      getItem: () => {
        throw new Error('blocked')
      },
      setItem: () => {
        throw new Error('blocked')
      },
    }
    expect(loadStepReviewStore(throwing)).toEqual({})
    expect(() => saveStepReviewStore({}, throwing)).not.toThrow()
  })
})

describe('createStepReview (reactive ledger)', () => {
  it('records verdicts per run without cross-run leakage', () => {
    const review = createStepReview()
    review.setReview('run-a', 'd1', 'approved')
    review.setReview('run-a', 'd2', 'flagged', '数据不对')
    review.setReview('run-b', 'd1', 'approved')

    expect(review.reviewFor('run-a', 'd1')?.verdict).toBe('approved')
    expect(review.reviewFor('run-a', 'd2')?.note).toBe('数据不对')
    expect(review.reviewFor('run-c', 'd1')).toBeNull()
    // 空参数不写账
    review.setReview('', 'd1', 'approved')
    review.setReview('run-a', '', 'approved')
    expect(review.reviewFor('run-a', '')).toBeNull()
  })

  it('counts pending reviews only for done steps with an artifact and no verdict', () => {
    const review = createStepReview()
    const steps = [
      step({ id: 'd1' }),
      step({ id: 'd2', artifactPath: null }),
      step({ id: 'd3', status: 'failed' }),
      step({ id: 'd4' }),
    ]
    expect(review.pendingReviewIds(steps, 'run-a')).toEqual(['d1', 'd4'])

    review.setReview('run-a', 'd1', 'approved')
    expect(review.pendingReviewIds(steps, 'run-a')).toEqual(['d4'])
    // 换一个 run：结论不串桶
    expect(review.pendingReviewIds(steps, 'run-b')).toEqual(['d1', 'd4'])
  })
})
