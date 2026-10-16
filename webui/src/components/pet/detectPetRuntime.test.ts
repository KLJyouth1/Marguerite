import { describe, expect, it, vi, beforeEach, afterEach } from 'vitest'
import { detectPetRuntime, detectWebGL, renderTierOf } from './detectPetRuntime'
import desktopPetSource from './DesktopPet.vue?raw'
import petViewSource from '@/views/PetView.vue?raw'
import petOverlaySource from '@/views/PetOverlayView.vue?raw'

function mockCanvas(glType: 'webgl2' | 'webgl1' | 'none') {
  const ctx = glType === 'none' ? null : ({}) as WebGLRenderingContext
  vi.stubGlobal('document', {
    createElement: vi.fn(() => ({
      getContext: vi.fn((type: string) =>
        (type === 'webgl2' && glType === 'webgl2') ||
        (type === 'webgl' && glType === 'webgl1') ||
        (type === 'experimental-webgl' && glType === 'webgl1')
          ? ctx
          : null,
      ),
    })),
  })
}

function mockHw(cores: number, memoryGB: number) {
  vi.stubGlobal('navigator', { hardwareConcurrency: cores, deviceMemory: memoryGB })
}

describe('detectPetRuntime —— 能力诚实探测', () => {
  beforeEach(() => { vi.stubGlobal('window', {}) })
  afterEach(() => { vi.unstubAllGlobals() })

  it('无 WebGL → fallback（绝不假装 3D）', () => {
    mockCanvas('none')
    mockHw(8, 16)
    const r = detectPetRuntime()
    expect(r.quality).toBe('fallback')
    expect(r.reason).toBe('no-webgl')
    expect(r.label).toContain('精简模式')
  })

  it('WebGL2 + 中高硬件 → high', () => {
    mockCanvas('webgl2')
    mockHw(8, 16)
    expect(detectPetRuntime().quality).toBe('high')
  })

  it('仅 WebGL1 → low（省电），不落 fallback 也不假装 high', () => {
    mockCanvas('webgl1')
    mockHw(2, 2)
    const r = detectPetRuntime()
    expect(r.quality).toBe('low')
    expect(r.reason).toBe('webgl1')
    expect(r.label).toContain('轻量')
  })

  it('WebGL2 但硬件受限 → medium（中间档，不是直接掉到 low）', () => {
    mockCanvas('webgl2')
    mockHw(2, 2)
    const r = detectPetRuntime()
    expect(r.quality).toBe('medium')
    expect(r.reason).toBe('webgl2+hw-limited')
    expect(r.label).toContain('均衡')
    // 核心数够但内存偏低 / 内存够但核心数偏低，同样是中端机
    mockHw(8, 2)
    expect(detectPetRuntime().quality).toBe('medium')
    mockHw(2, 16)
    expect(detectPetRuntime().quality).toBe('medium')
    // 边界值：恰好达标即 high（阈值不得被 off-by-one 挪动）
    mockHw(4, 4)
    expect(detectPetRuntime().quality).toBe('high')
  })

  it('renderTierOf：fallback 不落进「全开」档（兜底最低档）', () => {
    expect(renderTierOf('high')).toBe('high')
    expect(renderTierOf('medium')).toBe('medium')
    expect(renderTierOf('low')).toBe('low')
    expect(renderTierOf('fallback')).toBe('low')
  })

  it('三处桌宠构造点都按 renderTierOf 透传档位（DesktopPet 已切 PetSvg，不得把档位压成两档）', () => {
    // 回归背景：三处构造点此前都写 `quality === 'high' ? 'high' : 'low'`，
    // 于是新增的 medium 档在**调用点**就被压回 low，档位表再正确也不生效。
    const sites = [
      ['DesktopPet.vue', desktopPetSource],
      ['PetView.vue', petViewSource],
      ['PetOverlayView.vue', petOverlaySource],
    ] as const
    const strip = (src: string) => src
      .replace(/\/\*[\s\S]*?\*\//g, '')
      .split('\n')
      .filter(line => !line.trimStart().startsWith('//'))
      .join('\n')
    for (const [name, src] of sites) {
      const code = strip(src)
      expect(code, `${name} 未透传档位（应使用 renderTierOf）`).toContain('renderTierOf(')
      const collapsed = code
        .split('\n')
        .map((line, i) => [i + 1, line] as const)
        .filter(([, line]) => /quality\s*===\s*'high'\s*\?/.test(line))
      expect(collapsed, `${name} 把档位压成两档：${JSON.stringify(collapsed)}`).toEqual([])
    }
    // DesktopPet 已从 Pet3D 切到 PetSvg：构造点必须是 PetSvg，且档位经 renderTierOf 透传。
    const desktop = strip(desktopPetSource)
    expect(desktop, 'DesktopPet 应构造 PetSvg（不再用 Pet3D）').toContain('new PetSvg(')
    expect(
      /new PetSvg\(\{[\s\S]*?quality:\s*renderTierOf\(/.test(desktop),
      'DesktopPet 的 PetSvg 构造必须按 renderTierOf 透传档位',
    ).toBe(true)
  })

  it('override 可强制 fallback（测试/诊断注入）', () => {
    mockCanvas('webgl2')
    mockHw(8, 16)
    expect(detectPetRuntime({ override: () => false }).quality).toBe('fallback')
  })

  it('detectWebGL 三态分类正确', () => {
    mockCanvas('none'); expect(detectWebGL()).toBe('none')
    mockCanvas('webgl1'); expect(detectWebGL()).toBe('webgl1')
    mockCanvas('webgl2'); expect(detectWebGL()).toBe('webgl2')
  })
})