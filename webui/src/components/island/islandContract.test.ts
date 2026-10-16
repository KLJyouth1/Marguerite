import { describe, expect, it } from 'vitest'
import {
  DEFAULT_WORKSHOP_FLAVOR,
  formatDuration,
  ISLAND_KINDS,
  ISLAND_KIND_META,
  ISLAND_STATUS_ROLE,
  kindForSource,
  statusFromRecall,
  statusFromRun,
  statusFromStep,
  statusFromSubagent,
  statusFromTool,
  toolDisplayName,
  workshopFlavor,
  type IslandActivityStatus,
  type IslandSource,
} from './islandContract'

const ALL_SOURCES: IslandSource[] = ['session', 'tool', 'subagent', 'artifact', 'router', 'plan', 'memory', 'construction']

describe('islandContract — 建筑类型与来源的唯一映射', () => {
  it('每种建筑都有元数据，且 labelKey 遵循 island.kind.<kind> 约定', () => {
    for (const kind of ISLAND_KINDS) {
      const meta = ISLAND_KIND_META[kind]
      expect(meta, `缺少 ${kind} 的元数据`).toBeTruthy()
      expect(meta.labelKey).toBe(`island.kind.${kind}`)
      expect(meta.maxInstances).toBeGreaterThan(0)
    }
  })

  it('来源与建筑类型互为逆映射（双向覆盖，无遗漏无重复）', () => {
    const seen = new Set<IslandSource>()
    for (const kind of ISLAND_KINDS) {
      const source = ISLAND_KIND_META[kind].source
      expect(seen.has(source), `来源 ${source} 被多个建筑占用`).toBe(false)
      seen.add(source)
      expect(kindForSource(source)).toBe(kind)
    }
    for (const source of ALL_SOURCES) {
      expect(seen.has(source), `来源 ${source} 没有归属建筑`).toBe(true)
    }
    expect(seen.size).toBe(ALL_SOURCES.length)
  })

  it('未登记的来源直接抛错（防止契约漂移）', () => {
    expect(() => kindForSource('unknown' as IslandSource)).toThrowError(/未映射的实体来源/)
  })

  it('记忆塔（athenaeum）由 memory 来源驱动，且是单向单例、不临水、可发光', () => {
    expect(kindForSource('memory')).toBe('athenaeum')
    const meta = ISLAND_KIND_META.athenaeum
    expect(meta.source).toBe('memory')
    expect(meta.labelKey).toBe('island.kind.athenaeum')
    expect(meta.maxInstances).toBe(1)
    expect(meta.waterborne).toBe(false)
    expect(meta.glow).toBe(true)
  })
})

describe('islandContract — 工具名 → 工坊形态', () => {
  it('执行类工具进熔炉，联网类进观象台，读取类进书库，其余进作坊', () => {
    expect(workshopFlavor('run_command')).toBe('forge')
    expect(workshopFlavor('terminal.exec')).toBe('forge')
    expect(workshopFlavor('ssh_exec')).toBe('forge')
    expect(workshopFlavor('web_search')).toBe('observatory')
    expect(workshopFlavor('browser')).toBe('observatory')
    expect(workshopFlavor('screenshot')).toBe('observatory')
    expect(workshopFlavor('read_file')).toBe('library')
    expect(workshopFlavor('grep')).toBe('library')
    expect(workshopFlavor('kb_search')).toBe('library')
    expect(workshopFlavor('write_file')).toBe('mill')
    expect(workshopFlavor('edit_file')).toBe('mill')
  })

  it('空名与未知工具回落到默认形态，绝不返回 undefined', () => {
    expect(workshopFlavor('')).toBe(DEFAULT_WORKSHOP_FLAVOR)
    expect(workshopFlavor('   ')).toBe(DEFAULT_WORKSHOP_FLAVOR)
    expect(workshopFlavor('some_future_tool')).toBe(DEFAULT_WORKSHOP_FLAVOR)
  })
})

describe('islandContract — 状态归一化', () => {
  it('工具三态：运行中=active，失败=error，成功=success', () => {
    expect(statusFromTool({ isRunning: true, isError: false })).toBe('active')
    expect(statusFromTool({ isRunning: false, isError: true })).toBe('error')
    expect(statusFromTool({ isRunning: false, isError: false })).toBe('success')
  })

  it('会话运行态全量取值都有映射（ChatRunStatusState 键集）', () => {
    const runStates = ['idle', 'queued', 'running', 'approval_pending', 'interrupted', 'failed', 'timeout', 'cancelled']
    for (const state of runStates) {
      expect(statusFromRun(state), `${state} 未映射`).toBeTruthy()
    }
    expect(statusFromRun('running')).toBe('active')
    expect(statusFromRun('approval_pending')).toBe('waiting')
    expect(statusFromRun('failed')).toBe('error')
    expect(statusFromRun('unknown-future-state')).toBe('idle')
  })

  it('子代理/计划步骤状态归一化', () => {
    expect(statusFromSubagent('completed')).toBe('success')
    expect(statusFromSubagent('failed')).toBe('error')
    expect(statusFromSubagent('running')).toBe('active')
    expect(statusFromStep('completed')).toBe('success')
    expect(statusFromStep('in_progress')).toBe('active')
    expect(statusFromStep('failed')).toBe('error')
    expect(statusFromStep('pending')).toBe('queued')
  })

  it('每个活动状态都有取色角色定义（idle 为中性）', () => {
    const statuses: IslandActivityStatus[] = ['idle', 'queued', 'active', 'success', 'error', 'waiting']
    for (const status of statuses) {
      expect(status in ISLAND_STATUS_ROLE, `${status} 缺少取色角色`).toBe(true)
    }
    expect(ISLAND_STATUS_ROLE.idle).toBeNull()
    expect(ISLAND_STATUS_ROLE.error).toBe('danger')
  })

  it('记忆召回状态派生：命中+强化=success，仅命中=active，无命中=idle', () => {
    expect(statusFromRecall({ hits: 3, reinforced: 2 })).toBe('success')
    expect(statusFromRecall({ hits: 3, reinforced: 0 })).toBe('active')
    expect(statusFromRecall({ hits: 3 })).toBe('active')
    expect(statusFromRecall({ hits: 0, reinforced: 0 })).toBe('idle')
    // 防御：非法/负数命中一律视为无召回，绝不伪造「有记忆」
    expect(statusFromRecall({ hits: -5, reinforced: 9 })).toBe('idle')
    expect(statusFromRecall({ hits: Number.NaN })).toBe('idle')
  })
})

describe('islandContract — 耗时格式化', () => {
  it('秒/分/时三档可读，非法输入回退占位符', () => {
    expect(formatDuration(0)).toBe('0s')
    expect(formatDuration(45_400)).toBe('45s')
    expect(formatDuration(65_000)).toBe('1m05s')
    expect(formatDuration(3_600_000 + 2 * 60_000 + 5_000)).toBe('1h02m')
    expect(formatDuration(Number.NaN)).toBe('—')
    expect(formatDuration(-1)).toBe('—')
  })
})

describe('islandContract — 展示名', () => {
  it('超长工具名截断，短名原样保留', () => {
    expect(toolDisplayName('read_file')).toBe('read_file')
    expect(toolDisplayName('')).toBe('tool')
    const long = 'a_very_long_tool_name_exceeding_limit'
    expect(toolDisplayName(long).length).toBeLessThanOrEqual(22)
    expect(toolDisplayName(long).endsWith('…')).toBe(true)
  })
})