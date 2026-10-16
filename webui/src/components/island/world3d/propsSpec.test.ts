import { describe, it, expect } from 'vitest'
import { emptyRuntimeInput, type IslandRuntimeInput } from '../islandWorld'
import {
  PROP_KINDS,
  buildPropPlacements,
  propExpReward,
  propSpec,
  propSourceCountsOf,
} from './propsSpec'

function input(partial: Partial<IslandRuntimeInput>): IslandRuntimeInput {
  return { ...emptyRuntimeInput(), ...partial }
}

describe('propSpec', () => {
  it('≥5 种道具，全部可交互且几何描述完整', () => {
    expect(PROP_KINDS.length).toBeGreaterThanOrEqual(5)
    for (const kind of PROP_KINDS) {
      const spec = propSpec(kind)
      expect(spec.kind).toBe(kind)
      expect(spec.interaction.length).toBeGreaterThan(0)
      expect(spec.maxInstances).toBeGreaterThan(0)
      expect(spec.expReward).toBeGreaterThan(0)
      expect(spec.geometry.scale).toBeGreaterThan(0)
    }
  })
})

describe('propSourceCountsOf', () => {
  it('来源计数全部来自真实运行时归约', () => {
    const counts = propSourceCountsOf(
      input({
        tools: [{ id: 't1', name: 'Read', isRunning: false, isError: false }],
        artifacts: [{ id: 'a1', name: 'report.md' }],
        planSteps: [{ id: 'p1', title: 'step', status: 'running' }],
        ensembleMembers: [{ label: 'm1', done: true }],
        recall: { query: 'q', hits: 1, topScore: 0.5, ids: ['i1'], reinforced: 1, at: 0 },
      }),
    )
    expect(counts).toEqual({ tools: 1, artifacts: 1, recalls: 1, quests: 1, ensembleMembers: 1 })
  })
})

describe('buildPropPlacements', () => {
  const counts = { tools: 3, artifacts: 2, recalls: 1, quests: 6, ensembleMembers: 2 }

  it('确定性：同 (counts, seed) 永远同一套坐标', () => {
    expect(buildPropPlacements(counts, 'seed-a')).toEqual(buildPropPlacements(counts, 'seed-a'))
  })

  it('无真实来源 → 该类道具不出现（诚实布局）', () => {
    const empty = buildPropPlacements(
      { tools: 0, artifacts: 0, recalls: 0, quests: 0, ensembleMembers: 0 },
      'seed-a',
    )
    // 数据果实保底 1 枚（岛果，不代表任何 Agent 活动），其余全为 0
    expect(empty.every((p) => p.kind === 'data_fruit')).toBe(true)
  })

  it('数量受 maxInstances 上限约束，id 唯一', () => {
    const many = buildPropPlacements(
      { tools: 99, artifacts: 99, recalls: 99, quests: 99, ensembleMembers: 99 },
      'seed-a',
    )
    for (const kind of PROP_KINDS) {
      const count = many.filter((p) => p.kind === kind).length
      expect(count).toBeLessThanOrEqual(propSpec(kind).maxInstances)
    }
    expect(new Set(many.map((p) => p.id)).size).toBe(many.length)
  })

  it('拾取奖励为正', () => {
    for (const kind of PROP_KINDS) {
      expect(propExpReward(kind)).toBeGreaterThan(0)
    }
  })
})
