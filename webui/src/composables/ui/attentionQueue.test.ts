import { describe, expect, it } from 'vitest'

import {
  attentionCounts,
  attentionDebt,
  attentionScore,
  dedupeAttention,
  KIND_BASE,
  rankAttention,
  RECENCY_MAX,
  recencyBonus,
  RECENCY_SPAN_MS,
  type AttentionItem,
  type AttentionKind,
} from './attentionQueue'

const NOW = 1_800_000_000_000

function item(
  id: string,
  kind: AttentionKind,
  ageMs = 0,
  extra: Partial<AttentionItem> = {},
): AttentionItem {
  return { id, kind, at: NOW - ageMs, title: id, ...extra }
}

describe('attentionQueue — 排序不变量', () => {
  it('kind 基座的相邻间隔必须大于整个 recency 区间（否则时效会翻转类别优先级）', () => {
    const bases = (Object.keys(KIND_BASE) as AttentionKind[])
      .map((kind) => KIND_BASE[kind])
      .sort((a, b) => a - b)

    // 这条断言是整个模块的设计前提，不是实现细节：一旦有人把某个基座调小到
    // 「新鲜 choice 能越过旧 approval」，货架就退化成通知流，必须在这里先红。
    expect(RECENCY_MAX).toBeLessThan(300)
    for (let i = 1; i < bases.length; i += 1) {
      expect(bases[i] - bases[i - 1]).toBeGreaterThan(RECENCY_MAX)
    }
  })

  it('刚出现的 choice 不得越过搁置两小时的 approval', () => {
    const stale = item('ap-1', 'approval', 2 * 60 * 60 * 1000)
    const fresh = item('ch-1', 'choice', 0)

    const ranked = rankAttention([fresh, stale], NOW)

    expect(ranked.map((r) => r.id)).toEqual(['ap-1', 'ch-1'])
  })

  it('同类内越新越靠前', () => {
    const older = item('f-old', 'failed', 10 * 60 * 1000)
    const newer = item('f-new', 'failed', 0)

    expect(rankAttention([older, newer], NOW).map((r) => r.id)).toEqual(['f-new', 'f-old'])
  })

  it('完全同分时按 id 升序，且与输入顺序无关（避免两次渲染抖动）', () => {
    const a = item('b', 'choice', RECENCY_SPAN_MS * 2)
    const b = item('a', 'choice', RECENCY_SPAN_MS * 2)

    expect(rankAttention([a, b], NOW).map((r) => r.id)).toEqual(['a', 'b'])
    expect(rankAttention([b, a], NOW).map((r) => r.id)).toEqual(['a', 'b'])
  })
})

describe('attentionQueue — recency', () => {
  it('新项拿满额加成，超期项归零，中间线性衰减', () => {
    expect(recencyBonus(NOW, NOW)).toBe(RECENCY_MAX)
    expect(recencyBonus(NOW - RECENCY_SPAN_MS, NOW)).toBe(0)
    expect(recencyBonus(NOW - RECENCY_SPAN_MS * 5, NOW)).toBe(0)

    const half = recencyBonus(NOW - RECENCY_SPAN_MS / 2, NOW)
    expect(half).toBeGreaterThan(0)
    expect(half).toBeLessThan(RECENCY_MAX)
  })

  it('时钟偏移（at 在未来）按满额处理，不产生负分或乱序', () => {
    const future = item('ap', 'approval', -60_000)
    expect(recencyBonus(future.at, NOW)).toBe(RECENCY_MAX)
    expect(attentionScore(future, NOW)).toBe(KIND_BASE.approval + RECENCY_MAX)
  })

  it('非有限时间戳不参与排序加成', () => {
    expect(recencyBonus(Number.NaN, NOW)).toBe(0)
    expect(recencyBonus(NOW, Number.POSITIVE_INFINITY)).toBe(0)
  })
})

describe('attentionQueue — 计数与去重', () => {
  it('limit 只截断展示，不改变排序', () => {
    const items = [item('c1', 'choice'), item('a1', 'approval'), item('f1', 'failed')]
    expect(rankAttention(items, NOW, 2).map((r) => r.id)).toEqual(['a1', 'f1'])
    expect(rankAttention(items, NOW, 0)).toHaveLength(3)
  })

  it('counts 统计全部条目，不看展示切片', () => {
    const items = [item('a1', 'approval'), item('a2', 'approval'), item('c1', 'choice')]
    expect(attentionCounts(items)).toEqual({ approval: 2, blocked: 0, failed: 0, choice: 1 })
  })

  it('同一 id 重复上报时只算一笔债，并保留最早的 at', () => {
    const early = item('ap-1', 'approval', 60 * 60 * 1000)
    const late = item('ap-1', 'approval', 1_000)

    const deduped = dedupeAttention([late, early])

    expect(deduped).toHaveLength(1)
    expect(deduped[0].at).toBe(early.at)
    expect(attentionDebt([late, early])).toBe(1)
  })
})
