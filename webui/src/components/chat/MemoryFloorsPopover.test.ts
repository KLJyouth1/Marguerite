// @vitest-environment happy-dom
import { afterEach, describe, expect, it, vi } from 'vitest'
import type { App } from 'vue'

const mounted: App[] = []

const LAYERS = [
  { id: 'sensory', floor: 1, name: '感知层', description: '当前会话的历史消息', backend: 'session', enabled: true },
  { id: 'short_term', floor: 2, name: '短期层', description: '会话摘要', backend: 'sqlite-fts', enabled: true },
  { id: 'long_term', floor: 3, name: '长期层', description: '向量长时池', backend: 'vector', enabled: true },
  { id: 'recall', floor: 4, name: '召回层', description: '四类持久记忆', backend: 'json', enabled: true },
  { id: 'cross_project', floor: 5, name: '跨项目层', description: '其他项目经验', backend: 'json', enabled: true },
  { id: 'behavior', floor: 6, name: '行为层', description: '最近动作台账', backend: 'ledger', enabled: true },
  { id: 'tiered', floor: 7, name: '分层稀疏层', description: 'flyhash 近邻', backend: 'flyhash', enabled: false },
  { id: 'experience', floor: 8, name: '经验层', description: '踩坑/决策/模式', backend: 'experience.db', enabled: true },
]

async function settle() {
  for (let index = 0; index < 10; index++) await Promise.resolve()
}

async function mountPopover() {
  vi.resetModules()
  document.body.innerHTML = ''
  const calls: Array<{ method: string, params?: Record<string, unknown> }> = []
  let state = LAYERS.map(l => ({ ...l }))
  const call = vi.fn(async (method: string, params?: Record<string, unknown>) => {
    calls.push({ method, params })
    if (method === 'memory.layers.switches.get') {
      return { ok: true, count: state.length, enabledCount: state.filter(l => l.enabled).length, layers: state }
    }
    if (method === 'memory.layers.switches.set') {
      const patch = (params?.layers || {}) as Record<string, boolean>
      state = state.map(l => (l.id in patch ? { ...l, enabled: patch[l.id] } : l))
      return { ok: true, enabledCount: state.filter(l => l.enabled).length, layers: state }
    }
    throw new Error(`unexpected method: ${method}`)
  })
  vi.doMock('@/stores/rpc', () => ({
    useRpcStore: () => ({
      methods: [] as string[],
      supportsMethod: () => true,
      call,
    }),
  }))

  const { createApp } = await import('vue')
  const Component = (await import('./MemoryFloorsPopover.vue')).default
  const el = document.createElement('div')
  document.body.appendChild(el)
  const app = createApp(Component)
  app.mount(el)
  mounted.push(app)
  await settle()
  return { el, calls }
}

afterEach(() => {
  while (mounted.length) mounted.pop()!.unmount()
  vi.doUnmock('@/stores/rpc')
  vi.restoreAllMocks()
  document.body.innerHTML = ''
})

describe('MemoryFloorsPopover', () => {
  // 回归（巡检 2026-10-24）：徽标只在首次展开时才拉数据，导致未打开前恒显示 0，
  // 与弹窗内「8 层全开」自相矛盾。现在挂载即预取。
  it('shows the real enabled count before the popover is ever opened', async () => {
    const { el } = await mountPopover()
    const chip = el.querySelector('[data-testid="memory-floors-trigger"]') as HTMLButtonElement
    expect(chip.textContent || '').toContain('7')
    expect(chip.getAttribute('title')).toContain('7/8')
  })

  it('shows 8 floors stacked with 经验 on top after opening', async () => {
    const { el } = await mountPopover()
    ;(el.querySelector('[data-testid="memory-floors-trigger"]') as HTMLButtonElement).click()
    await settle()
    const stack = el.querySelector('[data-testid="memory-floors-stack"]')
    expect(stack).toBeTruthy()
    const floors = stack!.querySelectorAll('[data-testid^="memory-floor-"]')
    expect(floors.length).toBe(8)
    // 数组顺序即楼层顺序：第 1 层在前，经验层最后（CSS column-reverse 负责视觉反转）
    expect(floors[0].getAttribute('data-testid')).toBe('memory-floor-sensory')
    expect(floors[7].getAttribute('data-testid')).toBe('memory-floor-experience')
    expect(floors[7].textContent || '').toContain('经验层')
  })

  it('reflects the disabled tiered floor from the backend', async () => {
    const { el } = await mountPopover()
    ;(el.querySelector('[data-testid="memory-floors-trigger"]') as HTMLButtonElement).click()
    await settle()
    const tiered = el.querySelector('[data-testid="memory-floor-tiered"]')
    expect(tiered?.getAttribute('data-enabled')).toBe('false')
    const chip = el.querySelector('[data-testid="memory-floors-trigger"]')!.textContent || ''
    expect(chip).toContain('7') // 8 层里 7 层开启
  })

  it('toggles a single floor and persists only that layer', async () => {
    const { el, calls } = await mountPopover()
    ;(el.querySelector('[data-testid="memory-floors-trigger"]') as HTMLButtonElement).click()
    await settle()
    ;(el.querySelector('[data-testid="memory-floor-long_term"]') as HTMLButtonElement).click()
    await settle()
    const set = calls.find(c => c.method === 'memory.layers.switches.set')
    expect(set?.params?.layers).toEqual({ long_term: false })
    expect(
      el.querySelector('[data-testid="memory-floor-long_term"]')?.getAttribute('data-enabled'),
    ).toBe('false')
  })

  it('"全关" disables every floor in one call', async () => {
    const { el, calls } = await mountPopover()
    ;(el.querySelector('[data-testid="memory-floors-trigger"]') as HTMLButtonElement).click()
    await settle()
    const allOff = Array.from(el.querySelectorAll('.mfloors__all'))
      .find(b => (b.textContent || '').includes('全关')) as HTMLButtonElement
    allOff.click()
    await settle()
    const set = calls.filter(c => c.method === 'memory.layers.switches.set').pop()
    const patch = set?.params?.layers as Record<string, boolean>
    expect(Object.keys(patch).length).toBe(8)
    expect(Object.values(patch).every(v => v === false)).toBe(true)
    expect(el.querySelectorAll('.mfloor.is-on').length).toBe(0)
  })
})
