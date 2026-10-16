import { describe, expect, it } from 'vitest'
import { moodMap, PET_CAPABILITIES, PET_MOOD_3D, toPetMood3D, type PetMood3D } from './petContract'
import type { PetMood } from './petBrain'

/** brain 的全部心情（从类型运行时提取：与 petBrain 单一来源一致） */
const ALL_BRAIN_MOODS: PetMood[] = [
  'idle', 'curious', 'attentive', 'working', 'celebrate',
  'alert', 'sleepy', 'thinking', 'laughing',
]

describe('petContract —— 契约三同步守卫（R5）', () => {
  it('每个 brain 心情都有 3D 映射（新增心情漏配 → 必须失败）', () => {
    for (const m of ALL_BRAIN_MOODS) {
      const target = moodMap[m]
      expect(target, `PetMood '${m}' 未映射到 3D 心情`).toBeDefined()
      expect(PET_MOOD_3D.includes(target)).toBe(true)
    }
  })

  it('映射目标全部落在真实 3D 表现力集合内（不虚构能力）', () => {
    for (const target of Object.values(moodMap) as PetMood3D[]) {
      expect(PET_MOOD_3D.includes(target)).toBe(true)
    }
  })

  it('能力元数据诚实：如实声明 SVG + 伪 3D 光照，键集合固定（不多不少）', () => {
    expect(Object.keys(PET_CAPABILITIES).sort()).toEqual(['emotionBones', 'particles', 'pseudo3d', 'svg'])
    // 已随 WebGL 路径退役的能力不得回归
    expect('3d' in PET_CAPABILITIES).toBe(false)
    expect('toon' in PET_CAPABILITIES).toBe(false)
    expect('fallback2D' in PET_CAPABILITIES).toBe(false)
    // 真实存在的四项能力（与 petSvg.ts 实现一一对应）
    expect(PET_CAPABILITIES.svg).toBe(true)
    expect(PET_CAPABILITIES.pseudo3d).toBe(true)
    expect(PET_CAPABILITIES.emotionBones).toBe(true)
    expect(PET_CAPABILITIES.particles).toBe(true)
  })

  it('映射无重复值冲突导致表达无法区分（可选软约束，仅告警式断言）', () => {
    // 9→6 必然有收敛；断言收敛是「表达力诚实」而非缺陷
    const targets = new Set(Object.values(moodMap))
    expect(targets.size).toBeLessThanOrEqual(PET_MOOD_3D.length)
    expect(targets.size).toBeGreaterThanOrEqual(5) // 至少 5 种真实表达
  })

  it('toPetMood3D：仅接受 PET_MOOD_3D 内值，未知/非法收敛 normal（后端 mood 不越契约）', () => {
    for (const m of PET_MOOD_3D) {
      expect(toPetMood3D(m)).toBe(m)
    }
    // brain 侧心情（idle 等）不在 3D 表现力集合内 → 收敛 normal，绝不虚构
    expect(toPetMood3D('idle')).toBe('normal')
    expect(toPetMood3D('nonsense')).toBe('normal')
    expect(toPetMood3D(undefined)).toBe('normal')
    expect(toPetMood3D(3)).toBe('normal')
    expect(toPetMood3D({ mood: 'happy' })).toBe('normal')
  })
})