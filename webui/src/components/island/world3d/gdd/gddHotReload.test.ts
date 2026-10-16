import { describe, it, expect } from 'vitest'
import { GddHotReloadManager } from './gddHotReload'
import type { GddChangeEvent } from './gddHotReload'
import type { Gdd } from './gddSchema'

function validGdd(over: Record<string, unknown> = {}): Record<string, unknown> {
  return {
    version: '1.0.0',
    theme: '玛格丽特小岛',
    pillars: ['工作可视化'],
    props: [
      {
        kind: 'code_crystal',
        interaction: 'analyze',
        geometry: { main: 'octahedron', accent: 'prism', detail: 2, scale: 0.8 },
        maxInstances: 6,
        labelKey: 'code_crystal',
        expReward: 12,
      },
    ],
    quests: [{ id: 'q1', title: '初次上岛', trigger: 'runStatus=done', rewardRegion: '' }],
    regions: [{ id: 'r1', name: '工坊湾区', locked: true }],
    balance: { propSpawnIntervalMs: 4000, expCurve: { base: 100, exponent: 1.5 } },
    ...over,
  }
}

describe('GddHotReloadManager.loadFromJson', () => {
  it('合法 GDD 热加载成功，revision 从 1 开始递增', () => {
    const mgr = new GddHotReloadManager()
    expect(mgr.getRevision()).toBe(0)

    const r1 = mgr.loadFromJson(validGdd())
    expect(r1.ok).toBe(true)
    expect(r1.revision).toBe(1)

    const r2 = mgr.loadFromJson(validGdd({ version: '1.1.0' }))
    expect(r2.ok).toBe(true)
    expect(r2.revision).toBe(2)
    expect(mgr.getRevision()).toBe(2)
    expect((mgr.getCurrent() as Gdd).version).toBe('1.1.0')
  })

  it('JSON 字符串与对象两种输入等价', () => {
    const mgr = new GddHotReloadManager()
    const byString = mgr.loadFromJson(JSON.stringify(validGdd()))
    const byObject = mgr.loadFromJson(validGdd())
    expect(byString.ok).toBe(true)
    expect(byObject.ok).toBe(true)
    expect(byObject.revision).toBe((byString.revision as number) + 1)
  })

  it('校验失败保持旧配置继续跑（绝不半更新），错误透传', () => {
    const mgr = new GddHotReloadManager()
    const first = mgr.loadFromJson(validGdd({ theme: '旧世界' }))
    expect(first.ok).toBe(true)

    const bad = validGdd({ theme: '新世界' })
    ;(bad.props as Record<string, unknown>[])[0].maxInstances = 0
    const r = mgr.loadFromJson(bad)

    expect(r.ok).toBe(false)
    expect(r.revision).toBeNull()
    expect(r.errors.some((e) => e.path === 'props[0].maxInstances')).toBe(true)
    // 旧配置原样保留：theme 仍是「旧世界」，revision 未递增
    expect((mgr.getCurrent() as Gdd).theme).toBe('旧世界')
    expect(mgr.getRevision()).toBe(1)
    expect(mgr.getLastErrors()).toEqual(r.errors)
  })

  it('JSON 解析失败同样保持旧配置，错误码 invalid_json', () => {
    const mgr = new GddHotReloadManager()
    mgr.loadFromJson(validGdd())
    const r = mgr.loadFromJson('{ not json')
    expect(r.ok).toBe(false)
    expect(r.errors[0].code).toBe('invalid_json')
    expect((mgr.getCurrent() as Gdd).version).toBe('1.0.0')
    expect(mgr.getRevision()).toBe(1)
  })
})

describe('GddHotReloadManager.subscribe', () => {
  it('成功替换才通知订阅方，失败不打扰', () => {
    const mgr = new GddHotReloadManager()
    const events: GddChangeEvent[] = []
    const off = mgr.subscribe((e) => events.push(e))

    mgr.loadFromJson(validGdd())
    mgr.loadFromJson(validGdd({ version: '1.1.0' }))
    mgr.loadFromJson(validGdd({ props: [] })) // 非法 → 不通知

    expect(events).toHaveLength(2)
    expect(events.map((e) => e.revision)).toEqual([1, 2])
    expect(events[1].gdd.version).toBe('1.1.0')

    off()
    mgr.loadFromJson(validGdd({ version: '1.2.0' }))
    expect(events).toHaveLength(2) // 退订后不再收通知
  })
})

describe('GddHotReloadManager.exportCurrent', () => {
  it('导出与热加载构成无损往返（export → load 得到同一份 GDD）', () => {
    const mgr = new GddHotReloadManager()
    const source = validGdd()
    mgr.loadFromJson(source)

    const exported = mgr.exportCurrent()
    expect(exported).not.toBeNull()

    const mgr2 = new GddHotReloadManager()
    const r = mgr2.loadFromJson(exported as string)
    expect(r.ok).toBe(true)
    expect(mgr2.getCurrent()).toEqual(mgr.getCurrent())
  })

  it('未加载任何 GDD 时导出 null（不伪造默认配置）', () => {
    const mgr = new GddHotReloadManager()
    expect(mgr.exportCurrent()).toBeNull()
    expect(mgr.getCurrent()).toBeNull()
  })
})
