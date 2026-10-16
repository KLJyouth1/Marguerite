import { describe, it, expect } from 'vitest'
import { ISLAND_TRACKED_EVENTS } from '../islandRuntime'
import { agentPromptFor, npcReplyFor, pickupGlowStrength, scenePulseFor, type PickupContext } from './agentBinding'
import { WORLD3D_TIERS, tierProfile } from './tierProfile'

const ctx = (over: Partial<PickupContext> = {}): PickupContext => ({
  kind: 'code_crystal',
  interaction: 'analyze',
  name: 'main.ts',
  tier: 'efficient',
  ...over,
})

describe('agentPromptFor（游戏 → Agent）', () => {
  it('五种交互各生成一句确定性委托', () => {
    const cases: [PickupContext, string][] = [
      [ctx(), '岛上拾取了代码结晶「main.ts」，请分析这份产出并总结要点。'],
      [ctx({ kind: 'doc_scroll', interaction: 'explain', name: 'report.md' }), '岛上拾取了文档卷轴「report.md」，请讲解这份文档的核心内容。'],
      [ctx({ kind: 'data_fruit', interaction: 'insight', name: 'metrics' }), '岛上拾取了数据果实「metrics」，请给出这批数据的洞察结论。'],
      [ctx({ kind: 'quest_beacon', interaction: 'quest', name: 'quest-1' }), '岛上点亮了任务灯塔「quest-1」，请认领并推进这项委托。'],
      [ctx({ kind: 'inspiration_bloom', interaction: 'brainstorm', name: 'memory' }), '岛上绽放了灵感花「memory」，请围绕它做一轮头脑风暴。'],
    ]
    for (const [c, expected] of cases) {
      expect(agentPromptFor(c)).toBe(expected)
    }
  })

  it('空名字回退到道具种类名（不生成空主题委托）', () => {
    const prompt = agentPromptFor(ctx({ name: '  ' }))
    expect(prompt).toContain('code_crystal')
  })
})

describe('pickupGlowStrength', () => {
  it('拾取辉光随档位粒子预算缩放且夹取在 0.15..1', () => {
    expect(pickupGlowStrength('flagship')).toBeCloseTo(1)
    expect(pickupGlowStrength('sprint')).toBeGreaterThanOrEqual(0.15)
    expect(pickupGlowStrength('sprint')).toBeLessThan(0.5)
  })
})

describe('scenePulseFor（Agent → 游戏）', () => {
  it('每个受追踪事件都产生脉冲，未知事件返回 null', () => {
    for (const event of ISLAND_TRACKED_EVENTS) {
      expect(scenePulseFor(event, 'efficient')).not.toBeNull()
    }
    expect(scenePulseFor('session.event.unknown', 'efficient')).toBeNull()
    expect(scenePulseFor('', 'efficient')).toBeNull()
  })

  it('脉冲强度随档位预算缩放', () => {
    const strong = scenePulseFor('task.running', 'flagship')!
    const weak = scenePulseFor('task.running', 'sprint')!
    expect(strong.intensity).toBeGreaterThan(weak.intensity)
    expect(strong.intensity).toBeLessThanOrEqual(1)
    expect(weak.intensity).toBeGreaterThanOrEqual(0.15)
  })

  it('profileOverride 覆盖档位预算（渲染层实时档位切换用）', () => {
    const p = scenePulseFor('task.running', 'sprint', { ...tierProfile('flagship') })!
    expect(p.intensity).toBe(1)
  })
})

describe('npcReplyFor（AI NPC 对话复杂度随档位缩放）', () => {
  it('台词条数 = npcDialogueDepth', () => {
    for (const tier of WORLD3D_TIERS) {
      expect(npcReplyFor('极光', tier)).toHaveLength(tierProfile(tier).npcDialogueDepth)
    }
    expect(npcReplyFor('极光', 'flagship').length).toBe(3)
    expect(npcReplyFor('极光', 'sprint').length).toBe(1)
  })

  it('台词确定性（同输入同输出）', () => {
    expect(npcReplyFor('测试', 'omni')).toEqual(npcReplyFor('测试', 'omni'))
  })
})
