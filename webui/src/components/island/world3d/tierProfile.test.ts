import { describe, it, expect } from 'vitest'
import type { EngineSnapshot } from '../game/gameLogic'
import { MODEL_ROUTING_MODES, type ModelRoutingMode } from '@/types/modelRouting'
import {
  WORLD3D_TIERS,
  particleBudgetOrder,
  tierForRoutingMode,
  tierForSnapshot,
  tierProfile,
} from './tierProfile'

function snap(partial: Partial<EngineSnapshot>): EngineSnapshot {
  return {
    connected: true,
    modeKind: null,
    reasoningEffort: null,
    maxSwarmShards: null,
    hasTierRpc: false,
    hasBudgetRpc: false,
    hasGameRpc: false,
    ...partial,
  }
}

describe('tierProfile', () => {
  it('粒子预算随档位「重→轻」单调不增', () => {
    const order = particleBudgetOrder()
    for (let i = 1; i < order.length; i += 1) {
      expect(order[i - 1]).toBeGreaterThanOrEqual(order[i])
    }
  })

  it('每个档位的预算为正且时间流速为正', () => {
    for (const tier of WORLD3D_TIERS) {
      const p = tierProfile(tier)
      expect(p.particleBudget).toBeGreaterThan(0)
      expect(p.spawnIntervalMs).toBeGreaterThan(0)
      expect(p.timeScale).toBeGreaterThan(0)
      expect(p.npcDialogueDepth).toBeGreaterThanOrEqual(1)
      expect(p.npcDialogueDepth).toBeLessThanOrEqual(3)
    }
  })
})

describe('tierForRoutingMode', () => {
  it('五档 UI 词表全部映射到神力档', () => {
    expect(tierForRoutingMode('omni_agent')).toBe('omni')
    expect(tierForRoutingMode('bloom_ensemble')).toBe('flagship')
    expect(tierForRoutingMode('llm_ensemble')).toBe('efficient')
    expect(tierForRoutingMode('squilla_router')).toBe('efficient')
    expect(tierForRoutingMode('off')).toBe('sprint')
  })

  it('MODEL_ROUTING_MODES 无遗漏（新增路由档必须显式映射）', () => {
    for (const mode of MODEL_ROUTING_MODES) {
      expect(WORLD3D_TIERS).toContain(tierForRoutingMode(mode as ModelRoutingMode))
    }
  })
})

describe('tierForSnapshot 真实档位优先', () => {
  it('model.tier.get.effective_tier 覆盖 mode/effort 的二次推导', () => {
    // 真实档位是路由层算出的结论，不能被 mode=bloom 之类的粗粒度信号改回去。
    expect(tierForSnapshot(snap({ effectiveTier: 'flagship', modeKind: 'plan' }))).toBe('flagship')
    expect(tierForSnapshot(snap({ effectiveTier: 'balanced', modeKind: 'bloom' }))).toBe('efficient')
    expect(tierForSnapshot(snap({ effectiveTier: 'turbo', reasoningEffort: 'max' }))).toBe('sprint')
  })

  it('缺失 effectiveTier 时回退到 mode/effort 推导（不改变旧行为）', () => {
    expect(tierForSnapshot(snap({ modeKind: 'bloom' }))).toBe('flagship')
    expect(tierForSnapshot(snap({ modeKind: 'omni-transcend' }))).toBe('omni')
    expect(tierForSnapshot(snap({ reasoningEffort: 'max' }))).toBe('flagship')
    expect(tierForSnapshot(snap({ reasoningEffort: 'off' }))).toBe('sprint')
    expect(tierForSnapshot(snap({}))).toBe('efficient')
  })
})

describe('tierForSnapshot', () => {
  it('modeKind 优先于 reasoningEffort', () => {
    expect(tierForSnapshot(snap({ modeKind: 'omni-transcend', reasoningEffort: 'off' }))).toBe('omni')
    expect(tierForSnapshot(snap({ modeKind: 'bloom', reasoningEffort: 'low' }))).toBe('flagship')
    expect(tierForSnapshot(snap({ modeKind: 'ensemble' }))).toBe('efficient')
  })

  it('无 modeKind 时按 reasoningEffort 推导', () => {
    expect(tierForSnapshot(snap({ reasoningEffort: 'max' }))).toBe('flagship')
    expect(tierForSnapshot(snap({ reasoningEffort: 'deep' }))).toBe('flagship')
    expect(tierForSnapshot(snap({ reasoningEffort: 'off' }))).toBe('sprint')
    expect(tierForSnapshot(snap({ reasoningEffort: 'low' }))).toBe('sprint')
    expect(tierForSnapshot(snap({ reasoningEffort: 'normal' }))).toBe('efficient')
  })

  it('引擎未知 → 保守回退 efficient（不冒充旗舰也不假装极速）', () => {
    expect(tierForSnapshot(snap({ connected: false }))).toBe('efficient')
  })
})
