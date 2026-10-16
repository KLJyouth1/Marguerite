import { describe, expect, it } from 'vitest'
import { DEFAULT_ISLAND_SPEC } from './islandSpec'
import {
  buildIslandWorld,
  emptyRuntimeInput,
  plotFor,
  type IslandRuntimeInput,
} from './islandWorld'

const SPEC = DEFAULT_ISLAND_SPEC

function input(patch: Partial<IslandRuntimeInput> = {}): IslandRuntimeInput {
  return { ...emptyRuntimeInput(), sessionKey: 'sess-1234567890', ...patch }
}

/** 记忆召回快照（联想记忆网络的一次检索） */
const RECALL = {
  query: 'deploy checklist',
  hits: 4,
  topScore: 0.83,
  ids: ['m1', 'm2', 'm3', 'm4'],
  reinforced: 2,
  at: 1_700_000_000_000,
}

describe('islandWorld — 诚实可视化（无数据不建建筑）', () => {
  it('空快照只有中央主堡，并标记 empty', () => {
    const world = buildIslandWorld(input(), SPEC)
    expect(world.entities).toHaveLength(1)
    expect(world.entities[0].kind).toBe('core')
    expect(world.empty).toBe(true)
    expect(world.links).toHaveLength(0)
  })

  it('没有路由决策就没有灯塔；有策略或融合成员才出现', () => {
    expect(buildIslandWorld(input(), SPEC).counts.beacon).toBe(0)
    const withStrategy = buildIslandWorld(input({ strategy: 'EXPERT_TEAM' }), SPEC)
    expect(withStrategy.counts.beacon).toBe(1)
    expect(withStrategy.entities.find((e) => e.kind === 'beacon')?.title).toBe('EXPERT_TEAM')

    const withMembers = buildIslandWorld(
      input({ ensembleMembers: [{ label: 'p1', done: true }, { label: 'p2', done: false }] }),
      SPEC,
    )
    const beacon = withMembers.entities.find((e) => e.kind === 'beacon')
    expect(beacon?.detail).toBe('1/2')
  })

  it('没有计划就没有栈桥；有步骤时铺板进度 = 完成比', () => {
    expect(buildIslandWorld(input(), SPEC).counts.pier).toBe(0)
    const world = buildIslandWorld(
      input({
        planSteps: [
          { id: 's1', title: '勘察', status: 'completed' },
          { id: 's2', title: '施工', status: 'running' },
          { id: 's3', title: '验收', status: 'pending' },
        ],
      }),
      SPEC,
    )
    const pier = world.entities.find((e) => e.kind === 'pier')
    expect(pier?.progress).toBeCloseTo(1 / 3, 6)
    expect(pier?.status).toBe('active')
    expect(world.plan).toEqual({ total: 3, done: 1, progress: expect.closeTo(1 / 3, 6) })
  })

  it('没有记忆召回就没有记忆塔；有召回才出现，且携带真实召回数据', () => {
    // 诚实可视化：无召回 → 岛上不出现记忆塔（counts 为 0）
    expect(buildIslandWorld(input(), SPEC).counts.athenaeum).toBe(0)
    expect(buildIslandWorld(input({ recall: null }), SPEC).entities.some((e) => e.kind === 'athenaeum')).toBe(false)

    const world = buildIslandWorld(input({ recall: RECALL }), SPEC)
    const tower = world.entities.find((e) => e.kind === 'athenaeum')
    expect(world.counts.athenaeum).toBe(1)
    expect(tower?.id).toBe('athenaeum')
    expect(tower?.status).toBe('success') // hits>0 且 reinforced>0
    expect(tower?.title).toBe('deploy checklist')
    expect(tower?.meta).toEqual({
      recallQuery: 'deploy checklist',
      recallHits: 4,
      recallTopScore: 0.83,
      reinforced: 2,
      memoryIds: ['m1', 'm2', 'm3', 'm4'],
    })

    // 只命中未强化 → active；空 ids 不伪造条目列表
    const plain = buildIslandWorld(input({ recall: { ...RECALL, ids: [], reinforced: 0 } }), SPEC)
      .entities.find((e) => e.kind === 'athenaeum')
    expect(plain?.status).toBe('active')
    expect(plain?.meta?.memoryIds).toBeUndefined()
  })
})

describe('islandWorld — 运行时数据映射', () => {
  it('工具 → 工坊（稳定 id + 形态派生 + 三态）', () => {
    const world = buildIslandWorld(
      input({
        tools: [
          { id: 't1', name: 'read_file', isRunning: false, isError: false },
          { id: 't2', name: 'run_command', isRunning: true, isError: false },
          { id: 't3', name: 'web_search', isRunning: false, isError: true },
        ],
      }),
      SPEC,
    )
    const workshops = world.entities.filter((e) => e.kind === 'workshop')
    expect(workshops.map((w) => w.id)).toEqual(['workshop:t1', 'workshop:t2', 'workshop:t3'])
    expect(workshops.map((w) => w.flavor)).toEqual(['library', 'forge', 'observatory'])
    expect(workshops.map((w) => w.status)).toEqual(['success', 'active', 'error'])
    expect(world.counts.workshop).toBe(3)
    expect(world.empty).toBe(false)
  })

  it('同类实体超出上限时只保留最近的一批', () => {
    const tools = Array.from({ length: 12 }, (_, i) => ({
      id: `t${i}`,
      name: 'read_file',
      isRunning: false,
      isError: false,
    }))
    const world = buildIslandWorld(input({ tools }), SPEC)
    expect(world.counts.workshop).toBe(8)
    // 保留的是最后 8 个（t4..t11），最新活动不被裁掉
    const ids = world.entities.filter((e) => e.kind === 'workshop').map((e) => e.id)
    expect(ids[0]).toBe('workshop:t4')
    expect(ids[ids.length - 1]).toBe('workshop:t11')
  })

  it('子代理 → 哨所，产物 → 仓库（含装载率）', () => {
    const world = buildIslandWorld(
      input({
        subagents: [{ key: 'child-aaaa-bbbb', status: 'completed' }],
        artifacts: [{ id: 'a1', name: 'report.md', kind: 'document' }],
      }),
      SPEC,
    )
    const outpost = world.entities.find((e) => e.kind === 'outpost')
    expect(outpost?.title).toBe('…-aaaa-bbbb')
    expect(outpost?.status).toBe('success')
    const depot = world.entities.find((e) => e.kind === 'depot')
    expect(depot?.title).toBe('report.md')
    expect(depot?.progress).toBeCloseTo(1 / 5, 6)
  })

  it('道路连接主堡与每座建筑，且只在目标活动中时跑货物流', () => {
    const world = buildIslandWorld(
      input({
        tools: [
          { id: 't1', name: 'read_file', isRunning: true, isError: false },
          { id: 't2', name: 'read_file', isRunning: false, isError: false },
        ],
      }),
      SPEC,
    )
    expect(world.links).toHaveLength(world.entities.length - 1)
    for (const link of world.links) {
      expect(link.from).toBe('core')
      const target = world.entities.find((e) => e.id === link.to)
      expect(link.active).toBe(target?.status === 'active')
    }
  })

  it('会话运行态进入主堡详情；长 key 截断', () => {
    const world = buildIslandWorld(
      input({ runStatus: 'approval_pending', strategy: 'SOLO_AGENT', sessionKey: 'abcdefghijklmnop' }),
      SPEC,
    )
    const core = world.entities[0]
    expect(core.status).toBe('waiting')
    expect(core.detail).toBe('approval_pending · SOLO_AGENT')
    expect(core.title).toBe('…ghijklmnop')
  })
})

describe('islandWorld — 详情面板数据（meta 按类只填自己的字段）', () => {
  it('主堡携带会话与编排信息；无策略时留空而不编造', () => {
    const bare = buildIslandWorld(input(), SPEC).entities[0]
    expect(bare.meta).toEqual({ sessionKey: 'sess-1234567890', taskTitle: undefined, runStatus: 'idle', strategy: undefined })

    const rich = buildIslandWorld(
      input({ runStatus: 'running', taskTitle: '岛屿可视化', strategy: 'MAX' }),
      SPEC,
    ).entities[0]
    expect(rich.meta).toMatchObject({ taskTitle: '岛屿可视化', runStatus: 'running', strategy: 'MAX' })
  })

  it('工坊携带工具标识/形态/起止时间；运行中的工具没有结束时间', () => {
    const world = buildIslandWorld(
      input({
        tools: [
          { id: 't1', name: 'run_command', isRunning: true, isError: false, startedAt: 1000 },
          { id: 't2', name: 'read_file', isRunning: false, isError: true, startedAt: 500, endedAt: 1500 },
        ],
      }),
      SPEC,
    )
    const running = world.entities.find((e) => e.id === 'workshop:t1')
    expect(running?.meta).toEqual({
      toolId: 't1',
      toolFlavor: 'forge',
      startedAt: 1000,
      endedAt: undefined,
      hasError: false,
    })
    const failed = world.entities.find((e) => e.id === 'workshop:t2')
    expect(failed?.meta).toMatchObject({ toolFlavor: 'library', startedAt: 500, endedAt: 1500, hasError: true })
  })

  it('哨所/仓库/灯塔/栈桥各自携带对应字段，互不串味', () => {
    const world = buildIslandWorld(
      input({
        subagents: [{ key: 'child-1', status: 'completed' }],
        artifacts: [{ id: 'a1', name: 'report.md', kind: 'document' }],
        strategy: 'EXPERT_TEAM',
        ensembleMembers: [{ label: 'p1', done: true }],
        planSteps: [
          { id: 's1', title: '勘察', status: 'completed' },
          { id: 's2', title: '施工', status: 'in_progress' },
        ],
      }),
      SPEC,
    )
    expect(world.entities.find((e) => e.kind === 'outpost')?.meta).toEqual({ childKey: 'child-1', rawStatus: 'completed' })
    expect(world.entities.find((e) => e.kind === 'depot')?.meta).toEqual({ artifactId: 'a1', artifactKind: 'document' })
    expect(world.entities.find((e) => e.kind === 'beacon')?.meta).toEqual({
      strategy: 'EXPERT_TEAM',
      ensembleMembers: [{ label: 'p1', done: true }],
    })
    const pier = world.entities.find((e) => e.kind === 'pier')
    expect(pier?.meta?.steps).toHaveLength(2)
    expect(pier?.meta?.rawStatus).toBe('in_progress')
  })

  it('只有策略没有融合成员时，灯塔不伪造成员列表', () => {
    const beacon = buildIslandWorld(input({ strategy: 'SOLO_AGENT' }), SPEC).entities.find((e) => e.kind === 'beacon')
    expect(beacon?.meta).toEqual({ strategy: 'SOLO_AGENT', ensembleMembers: undefined })
  })
})

describe('islandWorld — 布局确定性与合法性', () => {
  it('同输入两次构建得到完全一致的坐标（可做增量 diff）', () => {
    const src = input({
      tools: [
        { id: 't1', name: 'read_file', isRunning: true, isError: false },
        { id: 't2', name: 'write_file', isRunning: false, isError: false },
      ],
      subagents: [{ key: 'c1', status: 'running' }],
      artifacts: [{ id: 'a1', name: 'x' }],
      strategy: 'SOLO_AGENT',
      planSteps: [{ id: 's1', title: 'step', status: 'running' }],
      recall: RECALL,
    })
    const a = buildIslandWorld(src, SPEC)
    const b = buildIslandWorld(src, SPEC)
    expect(JSON.stringify(a)).toBe(JSON.stringify(b))
  })

  it('陆地建筑全部落在草地半径内，同类建筑彼此不重叠', () => {
    const world = buildIslandWorld(
      input({
        tools: Array.from({ length: 8 }, (_, i) => ({
          id: `t${i}`,
          name: 'read_file',
          isRunning: false,
          isError: false,
        })),
        subagents: Array.from({ length: 6 }, (_, i) => ({ key: `c${i}`, status: 'running' })),
        artifacts: Array.from({ length: 5 }, (_, i) => ({ id: `a${i}`, name: `f${i}` })),
        strategy: 'SOLO_AGENT',
        planSteps: [{ id: 's1', title: 'step', status: 'pending' }],
        recall: RECALL,
      }),
      SPEC,
    )
    // 记忆塔是陆地建筑：必须落在草地上，且与其它陆地建筑互不重叠（含塔）
    const onLand = world.entities.filter((e) => !['pier', 'depot'].includes(e.kind))
    expect(onLand.some((e) => e.kind === 'athenaeum')).toBe(true)
    for (const entity of onLand) {
      const radius = Math.hypot(entity.plot.x, entity.plot.z)
      expect(radius, `${entity.id} 掉进海里`).toBeLessThan(SPEC.terrain.radius)
    }
    const radii = Math.hypot
    for (let i = 0; i < onLand.length; i++) {
      for (let j = i + 1; j < onLand.length; j++) {
        const a = onLand[i]
        const b = onLand[j]
        const distance = radii(a.plot.x - b.plot.x, a.plot.z - b.plot.z)
        expect(distance, `${a.id} 与 ${b.id} 重叠`).toBeGreaterThan(1.8)
      }
    }
  })

  it('码头仓库位于浅滩（岸线之外）', () => {
    const world = buildIslandWorld(
      input({ artifacts: [{ id: 'a1', name: 'f' }, { id: 'a2', name: 'g' }] }),
      SPEC,
    )
    for (const depot of world.entities.filter((e) => e.kind === 'depot')) {
      expect(Math.hypot(depot.plot.x, depot.plot.z)).toBeGreaterThan(SPEC.terrain.radius)
    }
  })

  it('plotFor 对同一 (kind,index) 稳定，且 index 不同则位置不同', () => {
    const first = plotFor('workshop', 0, SPEC)
    expect(plotFor('workshop', 0, SPEC)).toEqual(first)
    expect(plotFor('workshop', 1, SPEC)).not.toEqual(first)
  })

  it('记忆塔落点确定（固定岬角）且与灯塔、主堡不重叠', () => {
    const a = plotFor('athenaeum', 0, SPEC)
    expect(plotFor('athenaeum', 0, SPEC)).toEqual(a)
    const beacon = plotFor('beacon', 0, SPEC)
    expect(Math.hypot(a.x - beacon.x, a.z - beacon.z)).toBeGreaterThan(1.8)
    expect(Math.hypot(a.x, a.z)).toBeGreaterThan(SPEC.layout.coreRadius + 1.8)
  })

  it('新增建筑不会让已有建筑瞬移（位置与同类实体数量无关）', () => {
    const two = buildIslandWorld(
      input({
        tools: [
          { id: 't1', name: 'read_file', isRunning: false, isError: false },
          { id: 't2', name: 'read_file', isRunning: false, isError: false },
        ],
      }),
      SPEC,
    )
    const three = buildIslandWorld(
      input({
        tools: [
          { id: 't1', name: 'read_file', isRunning: false, isError: false },
          { id: 't2', name: 'read_file', isRunning: false, isError: false },
          { id: 't3', name: 'read_file', isRunning: false, isError: false },
        ],
      }),
      SPEC,
    )
    const plotOf = (world: typeof two, id: string) => world.entities.find((e) => e.id === id)?.plot
    expect(plotOf(three, 'workshop:t1')).toEqual(plotOf(two, 'workshop:t1'))
    expect(plotOf(three, 'workshop:t2')).toEqual(plotOf(two, 'workshop:t2'))
  })
})