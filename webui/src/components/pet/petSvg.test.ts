/**
 * 桌宠 SVG 渲染器契约测试。
 *
 * 只断言**结构性事实**（DOM 结构 / data-* 状态 / 渐变层级），不断言像素；
 * 位姿数学由 `petPose.computePose` 复用，这里不重写动画数学。
 *
 * @vitest-environment happy-dom
 */
import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest'
import { PetSvg, PET_SVG_SIZE, createPetSvg, resolvePetSvgSpec } from './petSvg'
import { getPetModelSpec } from './petSpec'

/** 已挂载实例（afterEach 统一 dispose，避免 rAF 泄漏） */
const live: PetSvg[] = []

function mount(skinId: string, quality: 'high' | 'medium' | 'low' = 'high') {
  const container = document.createElement('div')
  document.body.appendChild(container)
  const inst = new PetSvg({ container, spec: getPetModelSpec(skinId), quality })
  live.push(inst)
  const svg = container.querySelector('svg') as SVGSVGElement
  return { inst, container, svg }
}

beforeAll(() => {
  // happy-dom 未实现 Web Animations API，而 burst() 依赖 Element.animate。
  // 用一个惰性 stub 顶替：这里只关心「节点是否入 DOM / 是否被 dispose 摘除」这条结构事实。
  const proto = Element.prototype as unknown as { animate?: unknown }
  if (typeof proto.animate !== 'function') {
    proto.animate = () => ({ onfinish: null, cancel() {}, finish() {} })
  }
})

afterEach(() => {
  while (live.length) live.pop()!.dispose()
  document.body.innerHTML = ''
})

describe('petSvg —— 结构与材质分层（伪 3D 的物质基础）', () => {
  it('构造后容器内出现 <svg>，defs 至少 10 组渐变且有 filter', () => {
    const { svg } = mount('cat')
    expect(svg).toBeTruthy()
    expect(svg.tagName.toLowerCase()).toBe('svg')
    // 画布尺寸来自单一事实源，不是写死数字
    expect(svg.getAttribute('viewBox')).toBe(`0 0 ${PET_SVG_SIZE} ${PET_SVG_SIZE}`)
    const defs = svg.querySelector('defs')
    expect(defs).toBeTruthy()
    // 材质分层真实存在：每个材质一组渐变（毛/肤/裙/鞋/虹膜/发/翅/光环/投影…）
    expect(defs!.querySelectorAll('radialGradient, linearGradient').length).toBeGreaterThanOrEqual(10)
    // 软阴影 / 柔光必须有 filter
    expect(defs!.querySelectorAll('filter').length).toBeGreaterThanOrEqual(1)
  })

  it('皮肤切换：cat（兽形）与 blue（人形）结构不同（人形多出蝶翼等部件）', () => {
    const cat = mount('cat')
    const blue = mount('blue')
    // data-side：兽形只有左右耳（2）；人形是左右耳 + 左右蝶翼（4）
    expect(cat.svg.querySelectorAll('[data-side]').length).toBe(2)
    expect(blue.svg.querySelectorAll('[data-side]').length).toBe(4)
    // 人形部件更多（后发/裙/臂/翅/捧花），path 数量严格更大
    expect(blue.svg.querySelectorAll('path').length).toBeGreaterThan(cat.svg.querySelectorAll('path').length)
  })

  it('光照约定不退化：渐变停靠点同时存在「较亮」与「较暗」的 stop（ramp 真被使用）', () => {
    const { svg } = mount('cat')
    const lum = (color: string | null): number | null => {
      const m = /^#([0-9a-fA-F]{6})$/.exec((color ?? '').trim())
      if (!m) return null
      const v = parseInt(m[1], 16)
      return 0.2126 * ((v >> 16) & 0xff) + 0.7152 * ((v >> 8) & 0xff) + 0.0722 * (v & 0xff)
    }
    const spreadsOf = (grad: Element): Array<number> =>
      Array.from(grad.querySelectorAll('stop'))
        .map(s => lum(s.getAttribute('stop-color')))
        .filter((x): x is number => x !== null)

    const grads = Array.from(svg.querySelectorAll('radialGradient, linearGradient'))
    expect(grads.length).toBeGreaterThanOrEqual(10)
    const withSpread = grads
      .map(g => spreadsOf(g))
      .filter(lums => lums.length >= 2 && Math.max(...lums) - Math.min(...lums) > 10)
    // 多数材质都不是单色平涂，而是有明暗跨度（体积感来源）
    expect(withSpread.length).toBeGreaterThanOrEqual(4)

    // 关键材质「毛」必须同时含较亮与较暗停靠点（回归：ramp 被压成单色 → 无立体感）
    const fur = svg.querySelector('radialGradient[id$="-fur"]')
    expect(fur, '缺少毛材质渐变').toBeTruthy()
    const furLums = spreadsOf(fur!)
    expect(furLums.length).toBeGreaterThanOrEqual(2)
    expect(Math.max(...furLums) - Math.min(...furLums), '毛材质渐变必须有明暗跨度').toBeGreaterThan(20)
  })
})

describe('petSvg —— 位姿映射与交互（不重写动画数学，只验事实）', () => {
  it('poke / jump / setMood / spin / dance 不抛错，且 data-mood / data-behaviour 如实变化', async () => {
    const { inst, svg } = mount('cat')
    // 交互入口全部不抛错
    expect(() => {
      inst.poke('head')
      inst.jump(1)
      inst.spin(0.6)
      inst.triggerMoodAction('excited')
    }).not.toThrow()

    // data-mood 同步落地（setMood 直接写属性）
    inst.setMood('happy')
    expect(svg.getAttribute('data-mood')).toBe('happy')

    // data-behaviour 由逐帧循环写回；每种行为用独立实例，避免优先级相互干扰
    const spinning = mount('cat')
    spinning.inst.spin(1)
    await vi.waitFor(() => expect(spinning.svg.getAttribute('data-behaviour')).toBe('spin'))

    const dancing = mount('cat')
    dancing.inst.dance(1)
    await vi.waitFor(() => expect(dancing.svg.getAttribute('data-behaviour')).toBe('dance'))

    const walking = mount('cat')
    walking.inst.setWalking(true)
    await vi.waitFor(() => expect(walking.svg.getAttribute('data-behaviour')).toBe('walk'))
  })

  it('setPointer / lookAt 不抛错（视线跟随路径被驱动，结构保持完整）', () => {
    const { inst, svg } = mount('cat')
    expect(() => {
      inst.setPointer(0.5, -0.5, true)
      inst.lookAt(-1, 1)
      inst.setPointer(0, 0, false)
    }).not.toThrow()
    expect(svg.querySelector('defs')).toBeTruthy()
    expect(svg.querySelector('[data-side]')).toBeTruthy()
  })

  it('burst() 向 fx 层追加 star/heart/ellipse 三类粒子；dispose() 后 svg 从 DOM 移除', () => {
    const { inst, container, svg } = mount('cat')
    const fx = svg.querySelector('.pet-svg-fx')
    expect(fx, '缺少特效层 .pet-svg-fx').toBeTruthy()
    const before = fx!.childElementCount
    inst.burst()
    const added = Array.from(fx!.children)
    // 12 颗（高档 budget=1）：i%3 交替 star(path)/heart(path)/ellipse
    expect(added.length - before).toBeGreaterThanOrEqual(12)
    expect(added.some(el => el.tagName.toLowerCase() === 'ellipse'), '缺少 ellipse 类粒子').toBe(true)
    const pathDs = added
      .filter(el => el.tagName.toLowerCase() === 'path')
      .map(el => el.getAttribute('d') ?? '')
    expect(new Set(pathDs).size, 'star 与 heart 两种路径形状都要出现').toBeGreaterThanOrEqual(2)
    // 档位只调粒子预算，不减到 0
    const low = mount('cat', 'low')
    const lowFx = low.svg.querySelector('.pet-svg-fx')!
    const lowBefore = lowFx.childElementCount
    low.inst.burst()
    expect(lowFx.childElementCount - lowBefore).toBeGreaterThan(0)
    // 销毁后从 DOM 移除
    inst.dispose()
    expect(container.querySelector('svg')).toBeNull()
  })

  it('工厂与规格解析对称（宿主迁移入口可用）', () => {
    const container = document.createElement('div')
    document.body.appendChild(container)
    const inst = createPetSvg({ container, spec: resolvePetSvgSpec('blue') })
    live.push(inst)
    expect(container.querySelector('svg')).toBeTruthy()
    expect(resolvePetSvgSpec('cat').kind).toBe('beast')
    expect(resolvePetSvgSpec('blue').kind).toBe('humanoid')
  })
})