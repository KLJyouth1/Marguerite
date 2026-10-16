import { describe, expect, it } from 'vitest'
import { detectIslandRuntime, renderTierOf } from './detectIslandRuntime'
import islandViewSource from '@/views/IslandView.vue?raw'
import island3dSource from './Island3D.ts?raw'

describe('detectIslandRuntime — 诚实降级', () => {
  it('无 WebGL → fallback（不创建 3D，走 2D 示意图）', () => {
    const info = detectIslandRuntime({ probe: () => 'none' })
    expect(info.quality).toBe('fallback')
    expect(info.reason).toBe('no-webgl')
    expect(info.labelKey).toBe('island.quality.fallback')
  })

  it('显式禁用 → fallback override，且优先于探测结果', () => {
    const info = detectIslandRuntime({ override: () => false, probe: () => 'webgl2' })
    expect(info.quality).toBe('fallback')
    expect(info.reason).toBe('override-2d')
  })

  it('WebGL2 + 充足硬件 → high', () => {
    const info = detectIslandRuntime({ probe: () => 'webgl2', budget: () => ({ cores: 8, memoryGB: 16 }) })
    expect(info.quality).toBe('high')
    expect(info.labelKey).toBe('island.quality.high')
  })

  it('WebGL2 但硬件吃紧 → medium（中端机不再被迫二选一）', () => {
    const cases = [
      { cores: 2, memoryGB: 16 },
      { cores: 8, memoryGB: 2 },
      { cores: 2, memoryGB: 2 },
    ]
    for (const budget of cases) {
      const info = detectIslandRuntime({ probe: () => 'webgl2', budget: () => budget })
      expect(info.quality, `cores=${budget.cores} mem=${budget.memoryGB}`).toBe('medium')
      expect(info.reason).toBe('webgl2+hw-limited')
      expect(info.labelKey).toBe('island.quality.medium')
    }
  })

  it('阈值边界：恰好达标即 high（不得被 off-by-one 挪动）', () => {
    const info = detectIslandRuntime({ probe: () => 'webgl2', budget: () => ({ cores: 4, memoryGB: 4 }) })
    expect(info.quality).toBe('high')
    // 任一项差一点即中档
    expect(detectIslandRuntime({ probe: () => 'webgl2', budget: () => ({ cores: 3, memoryGB: 4 }) }).quality).toBe('medium')
    expect(detectIslandRuntime({ probe: () => 'webgl2', budget: () => ({ cores: 4, memoryGB: 3 }) }).quality).toBe('medium')
  })

  it('仅 WebGL1 → low（省电档，不降级到 2D，也不冒充 medium）', () => {
    const info = detectIslandRuntime({ probe: () => 'webgl1', budget: () => ({ cores: 16, memoryGB: 32 }) })
    expect(info.quality).toBe('low')
    expect(info.reason).toBe('webgl1')
    expect(info.labelKey).toBe('island.quality.low')
  })

  it('node 环境（无 document）实测走 fallback，不会抛错', () => {
    const info = detectIslandRuntime()
    expect(info.quality).toBe('fallback')
  })
})

describe('renderTierOf — 档位透传（不得压回两档）', () => {
  it('fallback 不落进「全开」档（兜底最低档）', () => {
    expect(renderTierOf('high')).toBe('high')
    expect(renderTierOf('medium')).toBe('medium')
    expect(renderTierOf('low')).toBe('low')
    expect(renderTierOf('fallback')).toBe('low')
  })

  it('构造点与渲染层都不把档位压成两档（源码守卫）', () => {
    // 回归背景：视图里此前写 `quality: runtime.quality === 'high' ? 'high' : 'low'`，
    // 渲染器里三处写 `quality === 'high' ? ... : ...` —— 新增的 medium 在**调用点与渲染层**
    // 就被静默压成 low，档位表再正确也不生效。这两类写法一旦复活即判红。
    const strip = (src: string) => src
      .replace(/\/\*[\s\S]*?\*\//g, '')
      .split('\n')
      .filter(line => !line.trimStart().startsWith('//') && !line.trimStart().startsWith('*'))
      .join('\n')

    const view = strip(islandViewSource)
    expect(view, 'IslandView 未透传档位（应使用 renderTierOf）').toContain('renderTierOf(')

    for (const [name, src] of [['IslandView.vue', view], ['Island3D.ts', strip(island3dSource)]] as const) {
      const collapsed = src
        .split('\n')
        .map((line, i) => [i + 1, line.trim()] as const)
        .filter(([, line]) => /quality\s*[!=]==?\s*'high'\s*\?/.test(line))
      expect(collapsed, `${name} 把档位压成两档：${JSON.stringify(collapsed)}`).toEqual([])
    }
  })
})
