// 对话产线工作流的契约测试：只验"声明的契约"，不验内部 watcher 顺序。
// @vitest-environment happy-dom
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { effectScope, nextTick, ref } from 'vue'
import { createPinia, setActivePinia } from 'pinia'

const rpcCall = vi.hoisted(() => vi.fn())
/**
 * 网关声明的方法表。真实 rpc store 的 `supportsMethod` 读的是 `methods` 这个 **ref**，
 * 所以目录可用性是响应式的；mock 必须同形状 —— 用普通函数会让 computed 永久缓存第一次
 * 结果，「hello 帧晚到」这类冷启动竞态就根本测不到。ref 在 mock 工厂里建（那一刻 vue
 * 已经求值），用例通过 `setDeclared()` 翻转它。
 */
const ALL_METHODS = vi.hoisted(() => [
  'line.catalog',
  'line.compile',
  'line.plan',
  'line.preflight',
  'line.retro',
  'line.run',
])
const declared = vi.hoisted(() => ({ ref: null as null | { value: string[] } }))

vi.mock('@/stores/rpc', async () => {
  const { ref } = await import('vue')
  declared.ref = ref([...ALL_METHODS])
  return {
    useRpcStore: () => ({
      isConnected: true,
      supportsMethod: (method: string) => (declared.ref?.value ?? []).includes(method),
      call: rpcCall,
      on: () => () => {},
    }),
  }
})

function setDeclared(methods: string[]): void {
  if (!declared.ref) throw new Error('rpc mock 尚未初始化')
  declared.ref.value = methods
}

let workflowScope: ReturnType<typeof effectScope> | null = null

import { toLegacyForm } from './lineSpec'
import type { LegacyOrchestrationForm, LinePattern, LineSpec, LineStage } from './types'
import { useLineWorkflow } from './useLineWorkflow'

const SESSION = 'agent:main:webchat:line'
const MODE = 'standard_doit'
const SPEC_KEY = 'marguerite.line.spec.v1'
const ARMED_KEY = 'marguerite.line.armed.v1'

/* ── 夹具 ─────────────────────────────────────────────────────────────────── */

function stageOf(pattern: LinePattern, width = 1, roster: string[] = []): LineStage {
  return { id: `s-${pattern}`, pattern, width, roster, facets: [], synthesize: false, note: '' }
}

function specOf(goal: string, stages: LineStage[]): LineSpec {
  return {
    specId: 'ls-fixture',
    goal,
    acceptance: [],
    stages,
    criteria: { maxFailProbability: 0.25, budgetTokens: null, deadlineMs: null },
  }
}

/** 目录载荷：标签、名册口径与上限全部由"后端"给出，夹具不复制常量语义。 */
function catalogPayload(): Record<string, unknown> {
  return {
    ok: true,
    patterns: [
      { id: 'solo', label: '单体', description: '', rosterKind: 'none', needsSynthesis: false, topology: 'solo' },
      { id: 'fanout', label: '并联', description: '', rosterKind: 'roles', needsSynthesis: false, topology: 'star' },
      { id: 'swarm', label: '蜂群', description: '', rosterKind: 'none', needsSynthesis: false, topology: 'swarm' },
      { id: 'panel', label: '专家组', description: '', rosterKind: 'teams', needsSynthesis: true, topology: 'panel' },
      { id: 'relay', label: '接力', description: '', rosterKind: 'roles', needsSynthesis: true, topology: 'relay' },
    ],
    recipes: [
      { id: 'research-then-write', label: '先研后写', stages: [stageOf('swarm', 3), stageOf('solo')] },
      { id: 'empty-recipe', label: '空配方', stages: [] },
    ],
    legacyForms: [
      { id: 'single', label: '单体', stages: ['solo'] },
      { id: 'subagent', label: '子智能体', stages: ['fanout'] },
      { id: 'team', label: '团队', stages: ['panel'] },
      { id: 'expert', label: '专家', stages: ['panel'] },
      { id: 'swarm', label: '蜂群', stages: ['swarm'] },
    ],
    teams: [{ id: 'team-a', label: 'A 团', roles: ['researcher'] }],
    roles: [{ id: 'researcher', label: '研究员', agentKey: 'researcher', personaFallback: false }],
    facets: [{ id: 'fact-check', instruction: '核查事实' }],
    limits: { maxStages: 3, maxWidth: 8, subagentMaxConcurrent: 4, runKeep: 20, dirBudgetBytes: 4096 },
    defaults: {
      criteria: { maxFailProbability: 0.25, budgetTokens: null, deadlineMs: null },
      trials: 200,
      maxTrials: 2000,
    },
  }
}

function preflightPayload(
  p90Ms: number,
  overrides: Record<string, unknown> = {},
): Record<string, unknown> {
  return {
    ok: true,
    shape: '1 单体',
    stages: [],
    total: {
      p50Ms: 1000,
      p90Ms,
      p50Tokens: 100,
      p90Tokens: 200,
      p90CostUsd: 0.1,
      successRate: 0.9,
      failProbability: 0.1,
    },
    rationale: {},
    premortem: [],
    unknowns: [],
    chosen: 'solo',
    chosenVsRecommended: null,
    validation: { valid: true, errors: [], warnings: [], maxConcurrent: 4 },
    gate: { autonomy: MODE, decision: 'allow', reason: '' },
    ...overrides,
  }
}

/**
 * `line.plan` 的载荷：字段名逐条对着 `core/src/gateway/line.rs::line_plan` 的 `json!` 写，
 * 连 `note` 那两条诚实说明（swarm 实为同步会诊、solo 实为单个编排子智能体）也是后端给的原文。
 */
function planPayload(
  overrides: Record<string, unknown> = {},
  stages: Record<string, unknown>[] = [
    {
      index: 0,
      pattern: 'swarm',
      width: 3,
      form: 'team',
      backend: 'team.run',
      args: { goal: '产线目标', swarm_on: true },
      roster: [],
      note: '产线里的 swarm 走同步会诊（team.run），不走后台蜂群',
      autonomy: 'high_autonomy',
    },
  ],
): Record<string, unknown> {
  return {
    ok: true,
    shape: '3 蜂群',
    spec: specOf('产线目标', [stageOf('swarm', 3)]),
    stages,
    validation: { valid: true, errors: [], warnings: [], maxConcurrent: 4 },
    gate: { autonomy: MODE, decision: 'allow', reason: '' },
    ...overrides,
  }
}

function soloPlanStages(): Record<string, unknown>[] {
  return [
    {
      index: 0,
      pattern: 'solo',
      width: 1,
      form: 'subagent',
      backend: 'SubAgentManager.spawn_with_spec',
      args: { roles: ['orchestrator'], sequential: false },
      roster: [],
      note: 'solo 阶段用单个 orchestrator 子智能体同步执行',
      autonomy: 'mutating',
    },
  ]
}

function runPayload(overrides: Record<string, unknown> = {}): Record<string, unknown> {
  return {
    runId: 'line-run-1',
    sessionKey: SESSION,
    at: '2026-10-30T00:00:00Z',
    shape: '1 单体',
    spec: specOf('产线目标', [stageOf('solo')]),
    stages: [],
    verdict: {
      ok: true,
      failures: [],
      elapsedMsActual: 1200,
      elapsedMsPredicted: 1000,
      tokensPredicted: 200,
      successRatePredicted: 0.9,
    },
    preflight: preflightPayload(1000),
    gate: { autonomy: MODE, decision: 'allow', reason: '' },
    storage: { persisted: true, path: '/tmp/line/line-run-1.json', bytes: 512, pruned: 0 },
    ...overrides,
  }
}

function rpcError(code: string, message: string): Error {
  return Object.assign(new Error(message), { code })
}

function neverCalled(method: string): never {
  throw new Error(`不该发生的 RPC 调用: ${method}`)
}

type RpcHandler = (params: Record<string, unknown>) => unknown

function calls(method: string): Array<[string, Record<string, unknown>, Record<string, unknown>]> {
  return rpcCall.mock.calls
    .filter((call: unknown[]) => call[0] === method)
    .map(
      (call: unknown[]): [string, Record<string, unknown>, Record<string, unknown>] => [
        String(call[0]),
        (call[1] ?? {}) as Record<string, unknown>,
        (call[2] ?? {}) as Record<string, unknown>,
      ],
    )
}

/** 缺省只声明目录 + 预演；`line.run` / `line.retro` 一被调到就抛（先拒后花的证据）。 */
function armRpc(handlers: Record<string, RpcHandler> = {}) {
  rpcCall.mockImplementation(async (method: string, params?: Record<string, unknown>) => {
    const handler = handlers[method]
    if (handler) return handler(params ?? {})
    if (method === 'line.catalog') return catalogPayload()
    if (method === 'line.preflight') return preflightPayload(500)
    return neverCalled(method)
  })
}

async function mountWorkflow(handlers: Record<string, RpcHandler> = {}) {
  armRpc(handlers)
  // 这些用例在组件外直接装配组合器，Vue 不会替它卸载 ⇒ watcher 会活到下一个用例里。
  // 包一层 effectScope 并在 afterEach 停掉：否则上一条用例遗留的 watcher 会回应本次
  // `setDeclared()` 的翻转，把 RPC 计数搅浑（用例单独跑过、整文件跑就红，正是这个症状）。
  workflowScope = effectScope()
  const workflow = workflowScope.run(() => useLineWorkflow(ref(SESSION), ref(MODE)))
  if (!workflow) throw new Error('effectScope.run 未返回工作流')
  for (let i = 0; i < 8; i += 1) await Promise.resolve()
  await nextTick()
  return workflow
}

function deferred<T>() {
  let resolve!: (value: T) => void
  const promise = new Promise<T>((done) => {
    resolve = done
  })
  return { promise, resolve }
}

beforeEach(() => {
  setActivePinia(createPinia())
  try {
    localStorage.clear()
  } catch {
    // 无 localStorage 的环境：持久化用例自己会跳过
  }
  rpcCall.mockReset()
  setDeclared([...ALL_METHODS])
})

afterEach(() => {
  workflowScope?.stop()
  workflowScope = null
  vi.useRealTimers()
  vi.restoreAllMocks()
})

/* ── 契约 ─────────────────────────────────────────────────────────────────── */

describe('useLineWorkflow', () => {
  it('形状摘要用后端目录的标签，solo 不带宽度', async () => {
    const wf = await mountWorkflow()
    expect(wf.catalogAvailable.value).toBe(true)
    expect(wf.catalogError.value).toBe(null)
    wf.setSpec(specOf('把调研报告写成结论', [stageOf('swarm', 3), stageOf('solo')]))
    expect(wf.shapeText.value).toBe('3 蜂群 → 单体')
  })

  it('本地校验错误拦下派单：一次后端调用都不发', async () => {
    const wf = await mountWorkflow()
    wf.armed.value = true
    // goal 为空 + panel 名册为空 ⇒ validateLocal 直接拦下（后端也会拒，不必再花一趟）
    wf.setSpec(specOf('', [stageOf('panel', 2)]))
    expect(wf.errors.value.length).toBeGreaterThan(0)
    expect(wf.canRun.value).toBe(false)

    const outcome = await wf.dispatch('', { confirmed: false })
    expect(outcome.status).toBe('invalid')
    expect(outcome.status === 'invalid' ? outcome.reason : '').toContain('goal')
    expect(calls('line.run')).toHaveLength(0)
  })

  it('本地已拦下时 refreshPreflight 也不打后端', async () => {
    const wf = await mountWorkflow()
    wf.setSpec(specOf('', [stageOf('solo')]))
    await wf.refreshPreflight()
    expect(calls('line.preflight')).toHaveLength(0)
    expect(wf.preflight.value).toBe(null)
    expect(wf.errors.value.length).toBeGreaterThan(0)
  })

  it('后端 validation.errors 并入 blockingErrors 后同样先拒后花', async () => {
    const wf = await mountWorkflow({
      'line.preflight': () =>
        preflightPayload(500, {
          ok: false,
          error: '名册无法解析',
          validation: {
            valid: false,
            errors: ['s-panel：名册里的团队已不存在'],
            warnings: [],
            maxConcurrent: 4,
          },
        }),
      'line.run': () => neverCalled('line.run'),
    })
    wf.armed.value = true
    wf.setSpec(specOf('产线目标', [stageOf('solo')]))
    await wf.refreshPreflight()
    expect(wf.preflight.value?.ok).toBe(false)
    expect(wf.blockingErrors.value).toContain('s-panel：名册里的团队已不存在')

    const outcome = await wf.dispatch('产线目标', { confirmed: false })
    expect(outcome).toEqual({
      status: 'invalid',
      reason: 's-panel：名册里的团队已不存在',
    })
    expect(calls('line.run')).toHaveLength(0)
  })

  it('乱序到达的预演响应不得覆盖更新的一次', async () => {
    vi.useFakeTimers()
    const first = deferred<Record<string, unknown>>()
    const second = deferred<Record<string, unknown>>()
    const queue = [first.promise, second.promise]
    const wf = await mountWorkflow({ 'line.preflight': () => queue.shift() ?? preflightPayload(500) })
    wf.setSpec(specOf('第一轮目标', [stageOf('solo')]))

    const p1 = wf.refreshPreflight()
    const p2 = wf.refreshPreflight()
    await nextTick()
    expect(calls('line.preflight')).toHaveLength(2)

    // 新的先回、旧的晚回：最终留下的必须是新那份
    second.resolve(preflightPayload(9999, { shape: '新形状' }))
    await p2
    expect(wf.preflight.value?.total.p90Ms).toBe(9999)

    first.resolve(preflightPayload(1111, { shape: '旧形状' }))
    await p1
    expect(wf.preflight.value?.total.p90Ms).toBe(9999)
    expect(wf.preflight.value?.shape).toBe('新形状')
    vi.useRealTimers()
  })

  it('形状变化在防抖窗口内只打一次预演', async () => {
    vi.useFakeTimers()
    const wf = await mountWorkflow()
    wf.armed.value = true
    expect(calls('line.preflight')).toHaveLength(0)

    wf.setSpec(specOf('第一次', [stageOf('solo')]))
    wf.setSpec(specOf('第二次', [stageOf('solo'), stageOf('swarm', 2)]))
    await vi.advanceTimersByTimeAsync(500)
    for (let i = 0; i < 6; i += 1) await Promise.resolve()

    expect(calls('line.preflight')).toHaveLength(1)
    expect(wf.preflight.value?.total.p90Ms).toBe(500)
    vi.useRealTimers()
  })

  it('派单把 predictedP90Ms 交给 store：超时由预演算出而不是拍固定值', async () => {
    const wf = await mountWorkflow({
      'line.preflight': () => preflightPayload(300_000),
      'line.run': () => runPayload(),
    })
    wf.armed.value = true
    wf.setSpec(specOf('预演前置目标', [stageOf('solo')]))
    await wf.refreshPreflight()
    expect(wf.preflight.value?.total.p90Ms).toBe(300_000)

    const outcome = await wf.dispatch('跑一条长产线', { confirmed: false })
    expect(outcome.status).toBe('ok')

    const runCalls = calls('line.run')
    expect(runCalls).toHaveLength(1)
    const [, params, options] = runCalls[0]
    // store 里 timeout = min(30min, max(180s, p90×2 + 60s))：300_000 → 660_000
    expect(options.timeoutMs).toBe(660_000)
    expect(params.mode).toBe(MODE)
    expect(params.sessionKey).toBe(SESSION)
    expect(params.confirmed).toBe(false)
    expect((params.spec as LineSpec).goal).toBe('跑一条长产线')
    expect(wf.lastRun.value?.runId).toBe('line-run-1')
    expect(wf.lastRun.value?.storage.persisted).toBe(true)
    // 组合（阶段）一个字节都不改；目标随派单写回 —— 发出去之后输入框会清空，
    // 不写回的话回执刚摆出来就反过来说"还不能派单：goal 不可为空"。
    expect(wf.spec.value.goal).toBe('跑一条长产线')
    expect(wf.spec.value.stages).toEqual([stageOf('solo')])
  })

  it('ORCHESTRATE_CONFIRM_REQUIRED → needs-confirm，且组合一个字节都不变', async () => {
    const wf = await mountWorkflow({
      'line.run': () => {
        throw rpcError('ORCHESTRATE_CONFIRM_REQUIRED', '需要用户确认')
      },
    })
    wf.armed.value = true
    wf.setSpec(specOf('既有的产线目标', [stageOf('swarm', 3), stageOf('solo')]))
    const snapshot = JSON.stringify(wf.spec.value)

    const outcome = await wf.dispatch('既有的产线目标', { confirmed: false })
    expect(outcome).toEqual({ status: 'needs-confirm', reason: '需要用户确认' })
    expect(JSON.stringify(wf.spec.value)).toBe(snapshot)
    expect(wf.armed.value).toBe(true)
    expect(wf.running.value).toBe(false)
    expect(wf.lastRun.value).toBe(null)
  })

  it('ORCHESTRATE_DENIED / INVALID_SPEC 各自映射，失败后组合原样可重试', async () => {
    const wf = await mountWorkflow({
      'line.run': () => {
        throw rpcError('ORCHESTRATE_DENIED', '当前档位不允许该形状')
      },
    })
    wf.armed.value = true
    wf.setSpec(specOf('产线目标', [stageOf('solo')]))
    const snapshot = JSON.stringify(wf.spec.value)

    const denied = await wf.dispatch('产线目标', { confirmed: true })
    expect(denied).toEqual({ status: 'denied', reason: '当前档位不允许该形状' })
    expect(wf.lastRunError.value).toBe('当前档位不允许该形状')

    armRpc({
      'line.run': () => {
        throw rpcError('INVALID_SPEC', '阶段数超过上限')
      },
    })
    const invalid = await wf.dispatch('产线目标', { confirmed: true })
    expect(invalid).toEqual({ status: 'invalid', reason: '阶段数超过上限' })
    expect(JSON.stringify(wf.spec.value)).toBe(snapshot)
    expect(wf.armed.value).toBe(true)
  })

  it('产线未开启（LINE_NOT_ARMED）时不发送任何东西', async () => {
    const wf = await mountWorkflow({ 'line.run': () => runPayload() })
    expect(wf.armed.value).toBe(false)
    expect(await wf.dispatch('产线目标', { confirmed: false })).toEqual({
      status: 'error',
      message: 'LINE_NOT_ARMED',
    })
    expect(calls('line.run')).toHaveLength(0)
  })

  it('后端未声明 line.catalog ⇒ 整套功能隐身，不发任何 RPC', async () => {
    setDeclared([])
    const wf = await mountWorkflow({ 'line.run': () => runPayload() })
    wf.armed.value = true

    expect(wf.catalogAvailable.value).toBe(false)
    expect(wf.catalog.value.loaded).toBe(false)
    expect(wf.catalogError.value).toBe('LINE_CATALOG_UNAVAILABLE')
    expect(wf.canRun.value).toBe(false)
    await wf.refreshPreflight()
    expect(await wf.dispatch('产线目标', { confirmed: false })).toEqual({
      status: 'error',
      message: 'LINE_CATALOG_UNAVAILABLE',
    })
    expect(rpcCall.mock.calls).toHaveLength(0)
  })

  it('hello 帧晚到：方法表随后落地必须自动补加载，不能永久挂着"目录不可用"', async () => {
    // 真机竞态（2026-10-31）：同一条产线刷新两次，一次形状与预设齐全、一次整套目录空掉。
    // 原因是 setup 里那一次 `store.load()` 跑在 WS `hello` 之前 —— 那一刻方法表还是空的，
    // load 直接放弃并把错误锁成"未被网关声明"，随后可用性翻正却没人再叫一次。
    setDeclared([])
    const wf = await mountWorkflow()
    expect(wf.catalogAvailable.value).toBe(false)
    expect(wf.catalog.value.loaded).toBe(false)
    expect(calls('line.catalog')).toHaveLength(0)

    setDeclared([...ALL_METHODS])
    await nextTick()
    for (let i = 0; i < 8; i += 1) await Promise.resolve()

    expect(wf.catalogAvailable.value).toBe(true)
    expect(calls('line.catalog')).toHaveLength(1)
    expect(wf.catalog.value.loaded).toBe(true)
    expect(wf.catalogError.value).toBe(null)
    expect(wf.catalog.value.patterns.map((p) => p.id)).toEqual(['solo', 'fanout', 'swarm', 'panel', 'relay'])
  })

  it('配方：套用成功只换阶段，空配方不得把产线清成 solo', async () => {
    const wf = await mountWorkflow()
    wf.setSpec(specOf('原目标', [stageOf('panel', 2, ['team-a'])]))

    expect(wf.applyRecipeById('research-then-write')).toBe(true)
    expect(wf.spec.value.stages.map((s) => s.pattern)).toEqual(['swarm', 'solo'])
    expect(wf.spec.value.goal).toBe('原目标')
    expect(wf.shapeText.value).toBe('3 蜂群 → 单体')

    expect(wf.applyRecipeById('empty-recipe')).toBe(false)
    expect(wf.applyRecipeById('no-such-recipe')).toBe(false)
    expect(wf.spec.value.stages.map((s) => s.pattern)).toEqual(['swarm', 'solo'])
  })

  it('armNextFromLegacy：swarm 出一条蜂群，relay 形态返回 false 且不改写规格', async () => {
    const wf = await mountWorkflow()
    wf.setSpec(specOf('沿用目标', [stageOf('solo')]))

    expect(wf.armNextFromLegacy({ swarmSize: 5 }, 'swarm')).toBe(true)
    expect(wf.spec.value.stages).toHaveLength(1)
    expect(wf.spec.value.stages[0].pattern).toBe('swarm')
    expect(wf.spec.value.stages[0].width).toBe(5)
    expect(wf.spec.value.goal).toBe('沿用目标')
    expect(wf.armed.value).toBe(true)

    // 接力在旧五挡里不可表示：toLegacyForm 返回 null，relay 也进不了 fromLegacyForm
    const relaySpec = specOf('沿用目标', [stageOf('relay', 3, ['researcher'])])
    wf.setSpec(relaySpec)
    expect(toLegacyForm(relaySpec)).toBe(null)
    const relayForm: string = 'relay'
    expect(wf.armNextFromLegacy({}, relayForm as LegacyOrchestrationForm)).toBe(false)
    expect(wf.spec.value).toEqual(relaySpec)
  })

  it('复盘：运行记录没落盘时不发后端请求', async () => {
    const wf = await mountWorkflow({
      'line.run': () => runPayload({ storage: { persisted: false, error: '写盘失败: disk full' } }),
      'line.retro': () => neverCalled('line.retro'),
    })
    wf.armed.value = true
    const dispatched = await wf.dispatch('产线目标', { confirmed: false })
    expect(dispatched.status).toBe('ok')
    expect(wf.lastRun.value?.storage.persisted).toBe(false)

    await wf.loadRetro('line-run-1')
    expect(calls('line.retro')).toHaveLength(0)
    expect(wf.retro.value).toBe(null)
    expect(wf.lastRunError.value).toBe('写盘失败: disk full')
  })

  it('复盘：ok:false 归零并如实带上后端原因', async () => {
    const wf = await mountWorkflow({
      'line.run': () => runPayload(),
      'line.retro': () => ({ ok: false, error: '运行记录不存在', code: 'LINE_RUN_NOT_FOUND' }),
    })
    wf.armed.value = true
    await wf.dispatch('产线目标', { confirmed: false })

    await wf.loadRetro('line-run-404')
    expect(calls('line.retro')).toHaveLength(1)
    expect(calls('line.retro')[0][1].runId).toBe('line-run-404')
    expect(wf.retro.value).toBe(null)
    expect(wf.lastRunError.value).toBe('运行记录不存在')
    expect(wf.retroLoading.value).toBe(false)
  })

  it('持久化：setSpec 落盘并可被下一个实例读回；setItem 抛错不得打断 setSpec', async () => {
    const wf = await mountWorkflow()
    wf.setSpec(specOf('要留下目标', [stageOf('swarm', 3), stageOf('solo')]))
    wf.armed.value = true
    await nextTick()

    expect(JSON.parse(String(localStorage.getItem(SPEC_KEY))).goal).toBe('要留下目标')
    expect(localStorage.getItem(ARMED_KEY)).toBe('true')

    // 下一个实例 = 刷新页面后重新挂载
    const restored = await mountWorkflow()
    expect(restored.spec.value.goal).toBe('要留下目标')
    expect(restored.spec.value.stages.map((s) => s.pattern)).toEqual(['swarm', 'solo'])
    expect(restored.armed.value).toBe(true)

    //  spy 挂在 localStorage 实例上而不是 Storage.prototype：jsdom 把 setItem 做成
    // own property，原型上的 spy 拦不到，于是"抛错被吞"这条路根本没被走过。
    const setItem = vi.spyOn(localStorage, 'setItem').mockImplementation(() => {
      throw new Error('QuotaExceededError')
    })
    expect(() => restored.setSpec(specOf('隐私模式也要能改', [stageOf('solo')]))).not.toThrow()
    expect(setItem).toHaveBeenCalled()
    expect(restored.spec.value.goal).toBe('隐私模式也要能改')
    expect(restored.canRun.value).toBe(true)
  })

  it('目录没给出 patterns ⇒ 不猜上限，标签回落原 id', async () => {
    const wf = await mountWorkflow({
      'line.catalog': () => ({ ok: true, patterns: [], recipes: [], legacyForms: [], limits: null }),
    })
    expect(wf.catalog.value.loaded).toBe(false)
    expect(wf.catalogError.value).toBe('line.catalog 返回的 patterns 不可用')

    // 两段形状才测得到「→」的连接：标签回落原 id，且只列**声明的**阶段 ——
    // 目录没加载时前端不知道 swarm 要不要合流，绝不能替引擎补一个它不会跑的尾巴。
    wf.setSpec(specOf('目标', [stageOf('swarm', 2, ['researcher']), stageOf('solo')]))
    expect(wf.errors.value).toEqual([])
    expect(wf.shapeText.value).toBe('2 swarm → solo')

    // 单段蜂群（synthesize=false，与 fromLegacyForm('swarm') 的如实一致）不得长出合流尾巴。
    wf.setSpec(specOf('目标', [stageOf('swarm', 2, ['researcher'])]))
    expect(wf.shapeText.value).toBe('2 swarm')
  })

  /* ── 执行映射（line.plan）：与预演同行、但不共享成败 ─────────────────── */

  it('line.plan 与预演一起落地：打的是同一个形状、同一份 mode', async () => {
    const wf = await mountWorkflow({
      'line.preflight': () => preflightPayload(700),
      'line.plan': () => planPayload(),
    })
    wf.armed.value = true
    wf.setSpec(specOf('产线目标', [stageOf('swarm', 3)]))
    await wf.refreshPreflight()
    for (let i = 0; i < 4; i += 1) await Promise.resolve()

    const planCalls = calls('line.plan')
    expect(planCalls).toHaveLength(1)
    expect(planCalls[0][1].mode).toBe(MODE)
    expect((planCalls[0][1].spec as LineSpec).goal).toBe('产线目标')

    expect(wf.preflight.value?.total.p90Ms).toBe(700)
    expect(wf.plan.value?.stages[0].backend).toBe('team.run')
    expect(wf.plan.value?.stages[0].note).toContain('同步会诊')
    expect(wf.plan.value?.stages[0].autonomy).toBe('high_autonomy')
    expect(wf.planLoading.value).toBe(false)
    expect(wf.planError.value).toBe(null)
  })

  it('line.plan 失败：预演一个字都不动，挂线与派单照样走', async () => {
    const wf = await mountWorkflow({
      'line.preflight': () => preflightPayload(700),
      'line.plan': () => {
        throw rpcError('-32601', 'method not found: line.plan')
      },
      'line.run': () => runPayload(),
    })
    wf.armed.value = true
    wf.setSpec(specOf('产线目标', [stageOf('solo')]))
    await wf.refreshPreflight()
    for (let i = 0; i < 4; i += 1) await Promise.resolve()

    expect(wf.preflight.value?.total.p90Ms).toBe(700)
    expect(wf.preflightError.value).toBe(null)
    expect(wf.plan.value).toBe(null)
    expect(wf.planError.value).toBe('method not found: line.plan')
    expect(wf.canRun.value).toBe(true)
    expect((await wf.dispatch('产线目标', { confirmed: false })).status).toBe('ok')
  })

  it('网关没声明 line.plan ⇒ 一趟都不打，预演照常出数', async () => {
    setDeclared(ALL_METHODS.filter((m) => m !== 'line.plan'))
    const wf = await mountWorkflow({ 'line.preflight': () => preflightPayload(700) })
    wf.armed.value = true
    wf.setSpec(specOf('产线目标', [stageOf('solo')]))
    await wf.refreshPreflight()
    for (let i = 0; i < 4; i += 1) await Promise.resolve()

    expect(calls('line.plan')).toHaveLength(0)
    expect(wf.plan.value).toBe(null)
    expect(wf.planError.value).toBe('LINE_PLAN_UNAVAILABLE')
    expect(wf.preflight.value?.total.p90Ms).toBe(700)
    expect(wf.canRun.value).toBe(true)
  })

  it('晚到的计划响应不得覆盖更新的那一份（与预演共用形状代号）', async () => {
    vi.useFakeTimers()
    const first = deferred<Record<string, unknown>>()
    const second = deferred<Record<string, unknown>>()
    const queue = [first.promise, second.promise]
    const wf = await mountWorkflow({
      'line.preflight': () => preflightPayload(500),
      'line.plan': () => queue.shift() ?? planPayload({ shape: '补位' }),
    })
    wf.setSpec(specOf('第一轮目标', [stageOf('solo')]))

    const p1 = wf.refreshPreflight()
    const p2 = wf.refreshPreflight()
    await nextTick()
    expect(calls('line.plan')).toHaveLength(2)

    // 新的先回、旧的晚回：卡上留下的必须是新那个形状的映射
    second.resolve(planPayload({ shape: '新映射' }, soloPlanStages()))
    await p2
    for (let i = 0; i < 4; i += 1) await Promise.resolve()
    expect(wf.plan.value?.shape).toBe('新映射')
    expect(wf.plan.value?.stages[0].backend).toBe('SubAgentManager.spawn_with_spec')

    first.resolve(planPayload({ shape: '旧映射' }))
    await p1
    for (let i = 0; i < 4; i += 1) await Promise.resolve()
    expect(wf.plan.value?.shape).toBe('新映射')
    expect(wf.plan.value?.stages[0].backend).toBe('SubAgentManager.spawn_with_spec')
    vi.useRealTimers()
  })

  it('形状被本地拦下 ⇒ 计划与预演一起失效，不给上一形状留着映射', async () => {
    const wf = await mountWorkflow({
      'line.preflight': () => preflightPayload(700),
      'line.plan': () => planPayload(),
    })
    wf.setSpec(specOf('产线目标', [stageOf('solo')]))
    await wf.refreshPreflight()
    for (let i = 0; i < 4; i += 1) await Promise.resolve()
    expect(wf.plan.value).not.toBe(null)

    wf.setSpec(specOf('', [stageOf('panel', 2)]))
    await wf.refreshPreflight()
    expect(wf.plan.value).toBe(null)
    expect(wf.planLoading.value).toBe(false)
    expect(wf.preflight.value).toBe(null)
  })

  /* ── 本轮目标 = 输入框那句话（2026-10-31 真机实测补的缝） ────────────────── */

  it('草稿喂进校验：goal 不再是界面上解不开的死结', async () => {
    vi.useFakeTimers()
    const wf = await mountWorkflow()
    wf.armed.value = true
    wf.setSpec(specOf('', [stageOf('solo')]))
    await vi.advanceTimersByTimeAsync(500)
    expect(wf.blockingErrors.value.some((row) => row.includes('goal'))).toBe(true)
    expect(wf.canRun.value).toBe(false)

    wf.setDraftGoal('把两份方案比出个结论')
    await vi.advanceTimersByTimeAsync(500)
    for (let i = 0; i < 6; i += 1) await Promise.resolve()

    expect(wf.blockingErrors.value).toEqual([])
    expect(wf.canRun.value).toBe(true)
    const preflights = calls('line.preflight')
    expect((preflights[preflights.length - 1][1].spec as LineSpec).goal).toBe(
      '把两份方案比出个结论',
    )
    // 草稿绝不写进形状本身：落盘的是形状，不是用户正在敲的那句话。
    expect(wf.spec.value.goal).toBe('')
    vi.useRealTimers()
  })

  it('未挂线时敲草稿不打预演；挂线那一刻补一次', async () => {
    vi.useFakeTimers()
    const wf = await mountWorkflow()
    wf.setSpec(specOf('', [stageOf('solo')]))
    await vi.advanceTimersByTimeAsync(500)
    // 形状那趟被本地拦下（goal 还空着）—— 一次 RPC 都没花。
    expect(calls('line.preflight')).toHaveLength(0)

    wf.setDraftGoal('随便一句话')
    wf.setDraftGoal('随便一句话，再改改')
    await vi.advanceTimersByTimeAsync(3000)
    // 未挂线：草稿只是普通对话的输入，敲它不该去敲网关。
    expect(calls('line.preflight')).toHaveLength(0)

    wf.armed.value = true
    await vi.advanceTimersByTimeAsync(500)
    for (let i = 0; i < 6; i += 1) await Promise.resolve()
    expect(calls('line.preflight')).toHaveLength(1)
    vi.useRealTimers()
  })

  it('派出去的目标写回形状：清空输入框后不会反过来说"还不能派单"', async () => {
    const wf = await mountWorkflow({ 'line.run': () => runPayload() })
    wf.armed.value = true
    wf.setSpec(specOf('', [stageOf('solo')]))
    wf.setDraftGoal('这一轮的真实目标')

    const outcome = await wf.dispatch('这一轮的真实目标', { confirmed: false })
    expect(outcome.status).toBe('ok')

    wf.setDraftGoal('')
    expect(wf.spec.value.goal).toBe('这一轮的真实目标')
    expect(wf.blockingErrors.value).toEqual([])
    expect(wf.canRun.value).toBe(true)
  })
})
