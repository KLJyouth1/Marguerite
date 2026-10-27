// 编排时间线聚合契约：后端 phase 序列 → 前端泳道状态机。
//
// 这些断言对应「对话页不能黑盒」的产品要求：成员、工具、结果、终态任何一环
// 解析错，用户看到的就是空卡片或永远转圈的假运行态。
// @vitest-environment happy-dom
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { nextTick, ref } from 'vue'
import { __resetOrchestrationRunsCache, useOrchestrationRuns } from './useOrchestrationRuns'

const notifyReadFailure = vi.hoisted(() => vi.fn())
vi.mock('@/utils/readFailureNotice', () => ({ notifyReadFailure }))

const SESSION = 'agent:main:webchat:test'

// 编排快照写在 localStorage 里（刷新/重启后仍可见），用例之间必须互相隔离，
// 否则前一个用例落下的终态会让后一个用例「凭空多出一条泳道」。
beforeEach(() => {
  try { localStorage.clear() } catch { /* node 环境无 localStorage */ }
  notifyReadFailure.mockClear()
  __resetOrchestrationRunsCache()
})

function frame(phase: string, extra: Record<string, unknown> = {}, session = SESSION) {
  return {
    payload: { kind: 'orchestration', phase, runId: 'orch-1', ...extra },
    meta: { session },
  }
}

describe('useOrchestrationRuns', () => {
  it('按 phase 还原一次完整编排（成员 / 工具 / 结果 / 终态）', () => {
    const sessionKey = ref(SESSION)
    const model = useOrchestrationRuns(sessionKey)

    model.handleFrame(frame('run_started', { form: 'swarm', goal: '做一份 PPT', memberEstimate: 2 }))
    expect(model.runs.value).toHaveLength(1)
    const run = model.runs.value[0]
    expect(run.form).toBe('swarm')
    expect(run.status).toBe('running')

    model.handleFrame(frame('member_started', { memberId: 'm1', role: 'researcher', roleLabel: '研究员', task: '检索学校资料' }))
    model.handleFrame(frame('member_started', { memberId: 'm2', role: 'coder', roleLabel: '工程师', task: '生成 PPT' }))
    expect(run.members).toHaveLength(2)

    model.handleFrame(frame('tool_call', { memberId: 'm2', tool: 'office_automation', argsPreview: '{"action":"create_ppt"}' }))
    expect(run.members[1].tools).toHaveLength(1)
    expect(run.members[1].tools[0].status).toBe('running')

    model.handleFrame(frame('tool_result', { memberId: 'm2', tool: 'office_automation', ok: true, elapsedMs: 812, outputPreview: '已生成 pptx' }))
    expect(run.members[1].tools[0].status).toBe('ok')
    expect(run.members[1].tools[0].elapsedMs).toBe(812)

    model.handleFrame(frame('member_completed', { memberId: 'm2', ok: true, elapsedMs: 1200, summary: 'PPT 已落盘' }))
    expect(run.members[1].status).toBe('ok')
    expect(run.members[1].summary).toBe('PPT 已落盘')

    model.handleFrame(frame('lead_synthesizing', { members: 2 }))
    expect(run.synthesizing).toBe(true)

    model.handleFrame(frame('run_completed', { ok: true, reply: '全部完成', summaries: ['资料已整理', 'PPT 已落盘'] }))
    expect(run.status).toBe('completed')
    expect(run.synthesizing).toBe(false)
    expect(run.reply).toBe('全部完成')
    // 缺少 member_completed 的成员由 run_completed.summaries 补齐
    expect(run.members[0].summary).toBe('资料已整理')
  })

  it('工具结果只回填最近一条同名运行中的行（多轮同名调用不错位）', () => {
    const sessionKey = ref(SESSION)
    const model = useOrchestrationRuns(sessionKey)
    model.handleFrame(frame('run_started', {}))
    model.handleFrame(frame('member_started', { memberId: 'm1', roleLabel: '工程师' }))
    model.handleFrame(frame('tool_call', { memberId: 'm1', tool: 'read_file', argsPreview: 'a.txt' }))
    model.handleFrame(frame('tool_result', { memberId: 'm1', tool: 'read_file', ok: true, elapsedMs: 10 }))
    model.handleFrame(frame('tool_call', { memberId: 'm1', tool: 'read_file', argsPreview: 'b.txt' }))
    model.handleFrame(frame('tool_result', { memberId: 'm1', tool: 'read_file', ok: false, elapsedMs: 20 }))

    const tools = model.runs.value[0].members[0].tools
    expect(tools).toHaveLength(2)
    expect([tools[0].status, tools[0].elapsedMs]).toEqual(['ok', 10])
    expect([tools[1].status, tools[1].elapsedMs]).toEqual(['error', 20])
  })

  it('成员结束会收口仍在运行的工具行，避免永远转圈', () => {
    const sessionKey = ref(SESSION)
    const model = useOrchestrationRuns(sessionKey)
    model.handleFrame(frame('run_started', {}))
    model.handleFrame(frame('member_started', { memberId: 'm1', roleLabel: '研究员' }))
    model.handleFrame(frame('tool_call', { memberId: 'm1', tool: 'web_search' }))
    model.handleFrame(frame('member_completed', { memberId: 'm1', ok: false, summary: '失败' }))
    expect(model.runs.value[0].members[0].tools[0].status).toBe('error')
  })

  it('run_failed 保留错误文案并结束计时', () => {
    const sessionKey = ref(SESSION)
    const model = useOrchestrationRuns(sessionKey)
    model.handleFrame(frame('run_started', {}))
    model.handleFrame(frame('run_failed', { error: '模型鉴权失败' }))
    const run = model.runs.value[0]
    expect(run.status).toBe('failed')
    expect(run.error).toBe('模型鉴权失败')
    expect(run.endedAtMs).not.toBeNull()
  })

  it('忽略其它会话与非编排类帧', () => {
    const sessionKey = ref(SESSION)
    const model = useOrchestrationRuns(sessionKey)
    model.handleFrame(frame('run_started', {}, 'agent:main:webchat:other'))
    expect(model.runs.value).toHaveLength(0)

    model.handleFrame({ payload: { kind: 'heartbeat', phase: 'run_started' }, meta: { session: SESSION } })
    expect(model.runs.value).toHaveLength(0)
  })

  it('新的编排排在前面，且最多保留 6 条', () => {
    const sessionKey = ref(SESSION)
    const model = useOrchestrationRuns(sessionKey)
    for (let i = 0; i < 8; i += 1) {
      model.handleFrame({
        payload: { kind: 'orchestration', phase: 'run_started', runId: `run-${i}`, form: 'team' },
        meta: { session: SESSION },
      })
    }
    expect(model.runs.value).toHaveLength(6)
    expect(model.runs.value[0].runId).toBe('run-7')
  })

  // 刷新/重启后仍能看到执行过程：已结束的编排落 localStorage，按会话分桶。
  it('刷新后恢复已结束的编排，且不串会话', async () => {
    const model = useOrchestrationRuns(ref(SESSION))
    model.handleFrame(frame('run_started', { form: 'swarm', goal: '做 PPT', memberEstimate: 1 }))
    model.handleFrame(frame('member_started', { memberId: 'm1', roleLabel: '研究员', task: '检索' }))
    model.handleFrame(frame('member_completed', { memberId: 'm1', ok: true, elapsedMs: 1200, summary: '已检索' }))
    model.handleFrame(frame('run_completed', { ok: true, reply: '最终答复', summaries: ['已检索'] }))
    await nextTick()

    // 新实例 = 刷新页面后重新挂载
    const restored = useOrchestrationRuns(ref(SESSION))
    expect(restored.runs.value).toHaveLength(1)
    const run = restored.runs.value[0]
    expect(run.status).toBe('completed')
    expect(run.reply).toBe('最终答复')
    expect(run.members[0].summary).toBe('已检索')

    // 另一个会话不继承
    expect(useOrchestrationRuns(ref('ws.another')).runs.value).toHaveLength(0)
  })

  it('仍在运行的编排不落盘（避免重开后永远转圈）', async () => {
    const model = useOrchestrationRuns(ref(SESSION))
    model.handleFrame(frame('run_started', { form: 'team', goal: '还在跑' }))
    await nextTick()
    expect(localStorage.getItem(`marguerite.chat.orchRuns.v1:${SESSION}`)).toBeNull()
  })

  // 真机事故（2026-10-24）：终态帧丢失后，任何重放/迟到帧都不该把状态改回去。
  it('终态是单向的：迟到的 member_started / 重复的 run_started 不得复活已完成的 run', async () => {
    const model = useOrchestrationRuns(ref(SESSION))
    model.handleFrame(frame('run_started', { form: 'swarm', goal: '跑完又复活？' }))
    const startedAt = model.runs.value[0].startedAtMs
    model.handleFrame(frame('member_started', { memberId: 'm1', roleLabel: '研究员', task: '检索' }))
    model.handleFrame(frame('member_completed', { memberId: 'm1', ok: true, elapsedMs: 900, summary: '已检索' }))
    model.handleFrame(frame('run_completed', { ok: true, reply: '完成' }))
    expect(model.runs.value[0].status).toBe('completed')

    // 服务端补发/重放（或对账重放）迟到的帧
    model.handleFrame(frame('member_started', { memberId: 'm2', roleLabel: '工程师', task: '实现' }))
    expect(model.runs.value[0].status).toBe('completed')
    model.handleFrame(frame('run_started', { form: 'swarm', goal: '跑完又复活？' }))
    expect(model.runs.value[0].status).toBe('completed')
    // 重放不得重置起点（否则耗时会被算成 0）
    expect(model.runs.value[0].startedAtMs).toBe(startedAt)
    // 终态之后不再新建成员行：显示出来就只能是「运行中」的假状态（结果帧永远不来）。
    expect(model.runs.value[0].members).toHaveLength(1)
  })

  // 真机症状：卡片头写「已完成」，展开后成员/工具还在转圈（member_completed 丢帧）。
  it('run_completed 收口仍在运行的成员与工具行', () => {
    const model = useOrchestrationRuns(ref(SESSION))
    model.handleFrame(frame('run_started', { form: 'swarm' }))
    model.handleFrame(frame('member_started', { memberId: 'm1', roleLabel: '研究员' }))
    model.handleFrame(frame('tool_call', { memberId: 'm1', tool: 'web_search' }))
    // 直接终态，缺 member_completed / tool_result
    model.handleFrame(frame('run_completed', { ok: true, reply: '完成' }))

    const run = model.runs.value[0]
    expect(run.status).toBe('completed')
    expect(run.members[0].status).toBe('ok')
    expect(run.members[0].tools[0].status).toBe('ok')
    expect(run.members[0].elapsedMs).not.toBeNull()
  })

  it('run_failed 把仍在运行的成员与工具判失败，已成功的保持不变', () => {
    const model = useOrchestrationRuns(ref(SESSION))
    model.handleFrame(frame('run_started', { form: 'team' }))
    model.handleFrame(frame('member_started', { memberId: 'm1', roleLabel: '研究员' }))
    model.handleFrame(frame('member_completed', { memberId: 'm1', ok: true, elapsedMs: 100, summary: '好了' }))
    model.handleFrame(frame('member_started', { memberId: 'm2', roleLabel: '工程师' }))
    model.handleFrame(frame('tool_call', { memberId: 'm2', tool: 'write_file' }))
    model.handleFrame(frame('run_failed', { error: '综合阶段失败' }))

    const run = model.runs.value[0]
    expect(run.status).toBe('failed')
    expect([run.members[0].status, run.members[0].summary]).toEqual(['ok', '好了'])
    expect(run.members[1].status).toBe('error')
    expect(run.members[1].tools[0].status).toBe('error')
  })

  it('对账确认记录已丢失（网关重启）时由 markRunLost 收口且幂等', () => {
    const model = useOrchestrationRuns(ref(SESSION))
    model.handleFrame(frame('run_started', { form: 'subagent' }))
    model.handleFrame(frame('member_started', { memberId: 'm1', roleLabel: '工程师' }))

    model.markRunLost('orch-1', '状态已丢失')
    const run = model.runs.value[0]
    expect(run.status).toBe('failed')
    expect(run.error).toBe('状态已丢失')
    expect(run.endedAtMs).not.toBeNull()
    expect(run.members[0].status).toBe('error')

    // 已终态的 run 不再被改写（迟到的对账结果不能覆盖真实终态）
    model.markRunLost('orch-1', '另一个原因')
    expect(model.runs.value[0].error).toBe('状态已丢失')
    // 不存在的 run：静默忽略
    expect(() => model.markRunLost('run-ghost', 'x')).not.toThrow()
  })

  // 真机缺陷：运行中切走会话，卡片直接从界面上消失；切回来也回不来（running 不落盘），
  // 这期间到达的终态帧又被会话过滤丢掉 —— 这次编排在 UI 上等于从未发生。
  it('运行中切走会话再切回：卡片仍在，离会期间的进度也不丢', () => {
    const sessionKey = ref(SESSION)
    const model = useOrchestrationRuns(sessionKey)
    model.handleFrame(frame('run_started', { form: 'swarm', goal: '跑一半' }))
    model.handleFrame(frame('member_started', { memberId: 'm1', roleLabel: '研究员' }))

    sessionKey.value = 'ws.other'
    expect(model.runs.value).toHaveLength(0)

    // 离会期间本会话继续推进
    model.handleFrame(frame('member_completed', { memberId: 'm1', ok: true, elapsedMs: 120, summary: '好了' }))
    model.handleFrame(frame('run_completed', { ok: true, reply: '结论' }))

    sessionKey.value = SESSION
    const run = model.runs.value[0]
    expect(run.runId).toBe('orch-1')
    expect(run.status).toBe('completed')
    expect(run.reply).toBe('结论')
    expect(run.members[0].summary).toBe('好了')
  })

  it('运行中切走再切回（期间无新帧）时卡片仍是运行中且成员完整', () => {
    const sessionKey = ref(SESSION)
    const model = useOrchestrationRuns(sessionKey)
    model.handleFrame(frame('run_started', { form: 'swarm' }))
    model.handleFrame(frame('member_started', { memberId: 'm1', roleLabel: '工程师' }))

    sessionKey.value = 'ws.other'
    sessionKey.value = SESSION

    const run = model.runs.value[0]
    expect(run.status).toBe('running')
    expect(run.members).toHaveLength(1)
    expect(model.activeCount.value).toBe(1)
  })

  it('快照损坏时提示用户并回到空视图，而不是假装没有编排', () => {
    localStorage.setItem(`marguerite.chat.orchRuns.v1:${SESSION}`, '{broken json')

    const model = useOrchestrationRuns(ref(SESSION))

    expect(model.runs.value).toEqual([])
    expect(notifyReadFailure).toHaveBeenCalledWith(
      'orchestration-runs',
      'shared.storage.orchestrationRunsReadFailed',
    )
  })
})
