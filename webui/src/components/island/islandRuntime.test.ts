import { describe, expect, it } from 'vitest'
import {
  applyIslandEvent,
  createIslandRuntimeState,
  ISLAND_STATE_LIMITS,
  ISLAND_TRACKED_EVENTS,
  sessionKeyOf,
} from './islandRuntime'

const KEY = 'sess-abc'
const OPTIONS = { sessionKey: KEY, now: 1_700_000_000_000 }

function fresh() {
  return createIslandRuntimeState(KEY)
}

describe('islandRuntime — 事件订阅面', () => {
  it('跟踪的事件名无重复且覆盖工具/会话/子代理/产物/路由/计划/记忆', () => {
    expect(new Set(ISLAND_TRACKED_EVENTS).size).toBe(ISLAND_TRACKED_EVENTS.length)
    const joined = ISLAND_TRACKED_EVENTS.join(',')
    for (const needle of ['tool_use_start', 'tool_result', 'state_change', 'subagent_completion', 'artifact', 'router_decision', 'ensemble_progress', 'plan_run', 'progress', 'memory_recall']) {
      expect(joined).toContain(needle)
    }
  })

  it('会话归属字段的别名都能解析', () => {
    expect(sessionKeyOf({ session_key: 'a' })).toBe('a')
    expect(sessionKeyOf({ sessionKey: 'b' })).toBe('b')
    expect(sessionKeyOf({ key: 'c' })).toBe('c')
    expect(sessionKeyOf(null)).toBe('')
  })
})

describe('islandRuntime — 工具归约', () => {
  it('tool_use_start 归一化别名并记录服务端起始时间', () => {
    const state = applyIslandEvent(
      fresh(),
      'session.event.tool_use_start',
      { session_key: KEY, tool_use_id: 't1', tool_name: 'read_file', started_at: 123 },
      OPTIONS,
    )
    expect(state.tools).toEqual([{ id: 't1', name: 'read_file', isRunning: true, isError: false, startedAt: 123, endedAt: undefined }])
    expect(state.eventCount).toBe(1)
    expect(state.runStatus).toBe('idle')
  })

  it('tool_result 收口状态，失败标记为 error', () => {
    let state = applyIslandEvent(fresh(), 'session.event.tool_use_start', { session_key: KEY, tool_id: 't1', name: 'run_command' }, OPTIONS)
    state = applyIslandEvent(state, 'session.event.tool_result', { session_key: KEY, toolId: 't1', is_error: true }, OPTIONS)
    expect(state.tools[0].isRunning).toBe(false)
    expect(state.tools[0].isError).toBe(true)
    expect(state.tools[0].endedAt).toBe(OPTIONS.now)
    expect(state.tools[0].name).toBe('run_command')
  })

  it('乱序结果（先来 result）也能建出工位，不丢事件', () => {
    const state = applyIslandEvent(fresh(), 'session.event.tool_result', { session_key: KEY, id: 't9' }, OPTIONS)
    expect(state.tools[0]).toMatchObject({ id: 't9', name: 'tool', isRunning: false })
  })

  it('工具列表有上限，保留最近的一批', () => {
    let state = fresh()
    for (let i = 0; i < ISLAND_STATE_LIMITS.tools + 5; i++) {
      state = applyIslandEvent(state, 'session.event.tool_use_start', { session_key: KEY, tool_id: `t${i}`, name: 'read_file' }, OPTIONS)
    }
    expect(state.tools).toHaveLength(ISLAND_STATE_LIMITS.tools)
    expect(state.tools[state.tools.length - 1].id).toBe(`t${ISLAND_STATE_LIMITS.tools + 4}`)
    expect(state.tools[0].id).toBe('t5')
  })
})

describe('islandRuntime — 会话状态与编排', () => {
  it('state_change 支持 to_state / run_status 别名', () => {
    const a = applyIslandEvent(fresh(), 'session.event.state_change', { session_key: KEY, to_state: 'running' }, OPTIONS)
    expect(a.runStatus).toBe('running')
    const b = applyIslandEvent(fresh(), 'session.event.state_change', { session_key: KEY, run_status: 'failed' }, OPTIONS)
    expect(b.runStatus).toBe('failed')
  })

  it('task.queued / task.running 推进运行态', () => {
    expect(applyIslandEvent(fresh(), 'task.queued', { session_key: KEY }, OPTIONS).runStatus).toBe('queued')
    expect(applyIslandEvent(fresh(), 'task.running', { session_key: KEY }, OPTIONS).runStatus).toBe('running')
  })

  it('router_decision 记录编排策略（决定灯塔是否出现）', () => {
    const state = applyIslandEvent(fresh(), 'session.event.router_decision', { session_key: KEY, strategy: 'MAX' }, OPTIONS)
    expect(state.strategy).toBe('MAX')
  })

  it('ensemble_progress 收敛成员完成态并去重', () => {
    let state = applyIslandEvent(fresh(), 'session.event.ensemble_progress', { session_key: KEY, event_type: 'proposer_start', proposer_label: 'p1' }, OPTIONS)
    state = applyIslandEvent(state, 'session.event.ensemble_progress', { session_key: KEY, event_type: 'proposer_finish', proposer_label: 'p1' }, OPTIONS)
    expect(state.ensembleMembers).toEqual([{ label: 'p1', done: true }])
    state = applyIslandEvent(state, 'session.event.ensemble_progress', { session_key: KEY, event_type: 'proposer_start', proposer_index: 2 }, OPTIONS)
    expect(state.ensembleMembers.map((m) => m.label)).toEqual(['p1', 'proposer-2'])
  })
})

describe('islandRuntime — 子代理 / 产物 / 计划', () => {
  it('子代理按 child key 去重', () => {
    let state = applyIslandEvent(fresh(), 'session.event.subagent_completion', { session_key: KEY, child_session_key: 'c1', status: 'running' }, OPTIONS)
    state = applyIslandEvent(state, 'session.event.subagent_completion', { session_key: KEY, child_session_key: 'c1', status: 'completed' }, OPTIONS)
    expect(state.subagents).toEqual([{ key: 'c1', status: 'completed' }])
  })

  it('产物支持 name/filename 别名并去重', () => {
    let state = applyIslandEvent(fresh(), 'session.event.artifact', { session_key: KEY, id: 'a1', filename: 'report.md', kind: 'document' }, OPTIONS)
    state = applyIslandEvent(state, 'session.event.artifact', { session_key: KEY, id: 'a1', name: 'report.md' }, OPTIONS)
    expect(state.artifacts).toHaveLength(1)
    expect(state.artifacts[0].name).toBe('report.md')
  })

  it('plan_run 全量替换步骤；缺失状态视为 pending', () => {
    const state = applyIslandEvent(
      fresh(),
      'session.event.plan_run',
      {
        session_key: KEY,
        steps: [
          { stepId: 's1', title: '勘察', status: 'completed' },
          { step_id: 's2', step_title: '施工', status: 'in_progress' },
          { stepId: 's3', title: '验收' },
        ],
      },
      OPTIONS,
    )
    expect(state.planSteps).toEqual([
      { id: 's1', title: '勘察', status: 'completed' },
      { id: 's2', title: '施工', status: 'in_progress' },
      { id: 's3', title: '验收', status: 'pending' },
    ])
  })

  it('plan_revision 的嵌套 snapshot.steps 同样可解析', () => {
    const state = applyIslandEvent(
      fresh(),
      'session.event.plan_revision',
      { session_key: KEY, snapshot: { steps: [{ stepId: 's1', title: 'a' }] } },
      OPTIONS,
    )
    expect(state.planSteps).toEqual([{ id: 's1', title: 'a', status: 'pending' }])
  })

  it('step_progress 单步推进：started → in_progress，finished → completed', () => {
    let state = applyIslandEvent(
      fresh(),
      'session.event.progress',
      { session_key: KEY, kind: 'step_progress', stepId: 's1', stepTitle: '编码', phase: 'started' },
      OPTIONS,
    )
    expect(state.planSteps).toEqual([{ id: 's1', title: '编码', status: 'in_progress' }])
    state = applyIslandEvent(
      state,
      'session.event.progress',
      { session_key: KEY, kind: 'step_progress', stepId: 's1', stepTitle: '编码', phase: 'finished' },
      OPTIONS,
    )
    expect(state.planSteps).toEqual([{ id: 's1', title: '编码', status: 'completed' }])
  })

  it('无关主题的 progress 事件不污染计划', () => {
    const state = applyIslandEvent(fresh(), 'session.event.progress', { session_key: KEY, kind: 'other' }, OPTIONS)
    expect(state.planSteps).toEqual([])
  })
})

describe('islandRuntime — 记忆召回归约（记忆塔的数据源）', () => {
  const RECALL = {
    session_key: KEY,
    query: 'deploy checklist',
    hits: 4,
    topScore: 0.83,
    ids: ['m1', 'm2', 'm3'],
    reinforced: 2,
    ts: 1_700_000_000_123,
  }

  it('memory_recall 归约成召回快照（含查询/命中/相似度/强化/条目）', () => {
    const state = applyIslandEvent(fresh(), 'session.event.memory_recall', RECALL, OPTIONS)
    expect(state.recall).toEqual({
      query: 'deploy checklist',
      hits: 4,
      topScore: 0.83,
      ids: ['m1', 'm2', 'm3'],
      reinforced: 2,
      at: 1_700_000_000_123,
    })
    expect(state.eventCount).toBe(1)
  })

  it('0 命中不发塔：视为无召回，引用不变（诚实可视化）', () => {
    const state = fresh()
    const next = applyIslandEvent(state, 'session.event.memory_recall', { ...RECALL, hits: 0 }, OPTIONS)
    expect(next).toBe(state)
    expect(next.recall).toBeNull()
  })

  it('字段别名与缺省：top_score/reinforced 缺省为 0，ids 过滤非字符串', () => {
    const state = applyIslandEvent(
      fresh(),
      'session.event.memory_recall',
      { session_key: KEY, query: 'q', hits: 2, top_score: 0.5, ids: ['a', 7, '', 'b'], ts: 42 },
      OPTIONS,
    )
    expect(state.recall).toEqual({ query: 'q', hits: 2, topScore: 0.5, ids: ['a', 'b'], reinforced: 0, at: 42 })
  })

  it('命中的记忆条目 id 截断到上限（≤8）', () => {
    const ids = Array.from({ length: 20 }, (_, i) => `m${i}`)
    const state = applyIslandEvent(fresh(), 'session.event.memory_recall', { session_key: KEY, query: 'q', hits: 20, ids }, OPTIONS)
    expect(state.recall?.ids).toHaveLength(ISLAND_STATE_LIMITS.memoryIds)
    expect(state.recall?.ids[0]).toBe('m0')
  })

  it('其它会话的召回事件被丢弃（引用不变）', () => {
    const state = fresh()
    expect(applyIslandEvent(state, 'session.event.memory_recall', { session_key: 'other', hits: 3 }, OPTIONS)).toBe(state)
  })
})

describe('islandRuntime — 过滤与幂等', () => {
  it('其它会话的事件被丢弃（引用不变 → Vue 层跳过重渲染）', () => {
    const state = fresh()
    const next = applyIslandEvent(state, 'session.event.tool_use_start', { session_key: 'other', tool_id: 't1', name: 'x' }, OPTIONS)
    expect(next).toBe(state)
  })

  it('未跟踪的事件名直接忽略', () => {
    const state = fresh()
    expect(applyIslandEvent(state, 'session.event.text_delta', { session_key: KEY }, OPTIONS)).toBe(state)
  })

  it('目标会话为空时不过滤（用于未选会话的浏览态）', () => {
    const state = createIslandRuntimeState('')
    const next = applyIslandEvent(
      state,
      'session.event.router_decision',
      { session_key: 'anything', strategy: 'SOLO' },
      { sessionKey: '', now: OPTIONS.now },
    )
    expect(next.strategy).toBe('SOLO')
  })
})