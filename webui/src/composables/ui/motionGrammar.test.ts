// @vitest-environment happy-dom
import { describe, expect, it } from 'vitest'

import {
  animationShorthand,
  cueFromCounts,
  MOTION_CUES,
  MOTION_RECIPES,
  motionAttrs,
  prefersReducedMotion,
} from './motionGrammar'

describe('motionGrammar — 只在状态跃迁时出动效', () => {
  it('数量不变就没有动效（返回 null，而不是一个默认动画）', () => {
    expect(cueFromCounts(0, 0)).toBeNull()
    expect(cueFromCounts(3, 3)).toBeNull()
  })

  it('新增是 arrive，减少是 settle（从用户视角，离开货架 = 已裁决）', () => {
    expect(cueFromCounts(0, 1)).toBe('arrive')
    expect(cueFromCounts(2, 5)).toBe('arrive')
    expect(cueFromCounts(2, 1)).toBe('settle')
  })

  it('没有 success 这个 cue —— 成功就是 settle，不给它单独的庆祝动画', () => {
    expect(MOTION_CUES).toEqual(['arrive', 'settle', 'yield', 'refuse'])
  })
})

describe('motionGrammar — 令牌纪律', () => {
  it('每个 cue 的时长/缓动都必须走 token，且 keyframe 名非空', () => {
    for (const cue of MOTION_CUES) {
      const recipe = MOTION_RECIPES[cue]
      expect(recipe.durationToken.startsWith('--dur-')).toBe(true)
      expect(recipe.easingToken.startsWith('--ease-')).toBe(true)
      expect(recipe.keyframe.length).toBeGreaterThan(0)
    }
  })

  it('animation 简写里不出现裸毫秒/裸缓动关键字（否则 check-webui-motion 会判红）', () => {
    // 与 scripts/check-webui-motion.mjs 同规则：先剥掉 var(...) 再判裸字面量。
    // 不剥的话 `--ease-out` 会被 \b 边界误判成裸关键字（第一版就是这么假红的）。
    const stripVars = (s: string) => s.replace(/var\([^()]*\)/g, '')
    for (const cue of MOTION_CUES) {
      const stripped = stripVars(animationShorthand(cue, false))
      expect(stripped).not.toMatch(/\b\d+m?s\b/)
      expect(stripped).not.toMatch(/\b(?:ease|ease-in|ease-out|ease-in-out)\b/)
    }
  })

  it('只有 yield 是循环，其余一次性播放', () => {
    expect(MOTION_RECIPES.yield.loop).toBe(true)
    expect(animationShorthand('yield', false).endsWith('infinite')).toBe(true)
    expect(animationShorthand('arrive', false).endsWith('1')).toBe(true)
    expect(animationShorthand('arrive', true).endsWith('1')).toBe(true)
  })
})

describe('motionGrammar — 属性投影', () => {
  it('无 cue 时不产出任何属性（元素保持完全静止）', () => {
    expect(motionAttrs(null, false)).toEqual({})
    expect(motionAttrs(null, true)).toEqual({})
  })

  it('零位移 cue 即使在完整动效下也标记为 static（纯透明度）', () => {
    expect(motionAttrs('settle', false)).toEqual({
      'data-motion': 'settle',
      'data-motion-static': 'true',
    })
  })

  it('reduced-motion 把有位移的 cue 降级为 static，但保留 cue 名（信息不丢）', () => {
    expect(motionAttrs('arrive', false)).toEqual({ 'data-motion': 'arrive' })
    expect(motionAttrs('arrive', true)).toEqual({
      'data-motion': 'arrive',
      'data-motion-static': 'true',
    })
  })

  it('循环 cue 带上 loop 标记，供 CSS 选择持续呼吸', () => {
    expect(motionAttrs('yield', false)['data-motion-loop']).toBe('true')
  })

  it('reduced-motion 下改用 -static 关键帧，位移为 0 但仍有可见变化', () => {
    expect(animationShorthand('arrive', true)).toContain('attn-arrive-static')
    expect(animationShorthand('arrive', false)).toContain('attn-arrive ')
  })
})

describe('motionGrammar — 环境兜底', () => {
  it('matchMedia 缺失时按「未声明偏好」处理，不能当成 reduced', () => {
    const original = window.matchMedia
    // happy-dom 可能没有实现 matchMedia；两者都必须返回 false 而不是抛错。
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    ;(window as any).matchMedia = undefined
    expect(prefersReducedMotion()).toBe(false)
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    ;(window as any).matchMedia = original
  })
})
