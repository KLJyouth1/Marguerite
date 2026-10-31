// 停靠条（LineDock）的行为契约。
//
// 锁的都是"用户会真实撞到"的那几件事：目录 RPC 不存在时整条隐身（不给假控件）；
// 收起态必须把形状 + 闸门 + 挂线状态摆在脸上；挂线却被阻塞时不许装作能用；
// "从当前挡位生成"失败必须说出来；展开/收起是视口偏好、要过刷新还活着。
//
// 挂载方式沿用仓内既有范式（`createApp(...).mount(el)` + querySelector），
// **不引入 @vue/test-utils** —— 项目没装它，加依赖会撞 `--webui` 的 lockfile 自洽闸。
// 装配台（LineComposer）整体 mock：它自己有编排层职责，这里只验"该挂时挂上、该转交时转交"。
// @vitest-environment happy-dom

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { computed, createApp, nextTick, ref, type App } from 'vue'

vi.mock('./LineComposer.vue', async () => {
  const { defineComponent, h } = await import('vue')
  return {
    default: defineComponent({
      name: 'LineComposerStub',
      props: {
        armed: { type: Boolean, default: false },
        blockingErrors: { type: Array, default: () => [] },
        gateDecision: { type: String, default: 'unknown' },
        spec: { type: Object, default: null },
        draftGoal: { type: String, default: '' },
      },
      emits: ['update:spec', 'update:armed', 'preflight-refresh'],
      setup(props, { emit }) {
        return () =>
          h('div', { 'data-testid': 'line-composer-stub' }, [
            h('span', { 'data-testid': 'stub-armed' }, String(props.armed)),
            h('span', { 'data-testid': 'stub-blocking' }, String((props.blockingErrors as string[]).length)),
            h('span', { 'data-testid': 'stub-draft' }, String(props.draftGoal)),
            h('button', { 'data-testid': 'stub-emit-armed', onClick: () => emit('update:armed', true) }, 'arm'),
            h(
              'button',
              {
                'data-testid': 'stub-emit-spec',
                onClick: () => emit('update:spec', { ...(props.spec as LineSpec), goal: '装配台改写目标' }),
              },
              'spec',
            ),
            h(
              'button',
              { 'data-testid': 'stub-emit-refresh', onClick: () => emit('preflight-refresh') },
              'refresh',
            ),
          ])
      },
    }),
  }
})

const messages = vi.hoisted((): Record<string, string> => ({
  'chat.line.dock.title': 'Line dock',
  'chat.line.dock.expand': 'Expand line dock',
  'chat.line.dock.collapse': 'Collapse line dock',
  'chat.line.dock.fromGear': 'From current gear',
  'chat.line.dock.fromGearHint': 'Compile the live gear into the line',
  'chat.line.dock.fromGearFailed': 'This gear cannot be compiled into a line',
  'chat.line.dock.ladderAria': 'Line shape and gate',
  'chat.line.compose.gate.allow': 'Allowed',
  'chat.line.compose.gate.confirm': 'Confirm',
  'chat.line.compose.gate.deny': 'Denied',
  'chat.line.compose.gate.unknown': 'Gate unknown',
  'chat.line.compose.gateUnknownReason': 'No preflight verdict yet',
  'chat.line.compose.armSwitchLabel': 'Arm the line',
  'chat.line.compose.armExplain': 'An armed line replaces the next send',
  'chat.line.compose.armOn': 'Armed',
  'chat.line.compose.armOff': 'Off',
  'chat.line.compose.armRefused': "Can't arm yet: {error}",
  'chat.line.code.preflightEmpty': 'preflight came back empty',
  'chat.line.compose.armButBlocked': 'Armed, but dispatch is still blocked ({count})',
}))

vi.mock('vue-i18n', () => ({
  useI18n: () => ({
    t: (key: string, values?: Record<string, unknown>) => {
      let value = messages[key] ?? key
      for (const [name, replacement] of Object.entries(values ?? {})) {
        value = value.replace(`{${name}}`, String(replacement))
      }
      return value
    },
    te: (key: string) => key in messages,
  }),
}))

import LineDock from './LineDock.vue'
import { summarizeShape } from '@/composables/line/lineSpec'
import type { LineDispatchOutcome, LineWorkflow } from '@/composables/line/useLineWorkflow'
import type {
  GateDecision,
  LegacyOrchestrationForm,
  LineCatalog,
  LinePattern,
  LinePlan,
  LinePreflight,
  LineRetro,
  LineRunResult,
  LineSpec,
  LineStage,
} from '@/composables/line/types'

const OPEN_KEY = 'marguerite.line.dock.open.v1'

/* ── 夹具 ─────────────────────────────────────────────────────────────────── */

function stageOf(pattern: LinePattern, width = 1, roster: string[] = []): LineStage {
  return { id: `s-${pattern}`, pattern, width, roster, facets: [], synthesize: false, note: '' }
}

function specOf(goal: string, stages: LineStage[]): LineSpec {
  return {
    specId: 'ls-dock-fixture',
    goal,
    acceptance: [],
    stages,
    criteria: { maxFailProbability: 0.25, budgetTokens: null, deadlineMs: null },
  }
}

/** 与 `useLineWorkflow.test.ts` 同一份目录口径：形状标签只有后端一份，夹具不复制语义。 */
const CATALOG: LineCatalog = {
  loaded: true,
  patterns: [
    { id: 'solo', label: '单体', description: '', rosterKind: 'none', needsSynthesis: false, topology: 'solo' },
    { id: 'fanout', label: '并联', description: '', rosterKind: 'roles', needsSynthesis: false, topology: 'star' },
    { id: 'panel', label: '专家组', description: '', rosterKind: 'teams', needsSynthesis: true, topology: 'panel' },
    { id: 'swarm', label: '蜂群', description: '', rosterKind: 'none', needsSynthesis: false, topology: 'swarm' },
    { id: 'relay', label: '接力', description: '', rosterKind: 'roles', needsSynthesis: true, topology: 'relay' },
  ],
  recipes: [],
  legacyForms: [{ id: 'swarm', label: '蜂群', stages: ['swarm'] }],
  roles: [],
  teams: [],
  facets: [],
  limits: { maxStages: 3, maxWidth: 8, subagentMaxConcurrent: 4, runKeep: 20, dirBudgetBytes: 4096 },
  defaults: {
    criteria: { maxFailProbability: 0.25, budgetTokens: null, deadlineMs: null },
    trials: 200,
    maxTrials: 2000,
  },
}

interface FakeLineOptions {
  available?: boolean
  armed?: boolean
  blocking?: string[]
  gate?: GateDecision | 'unknown'
  spec?: LineSpec
  draftGoal?: string
}

/** 假工作流：dock 只消费 `LineWorkflow`，所以按接口逐字段给全，不做局部替身。 */
function fakeLine(options: FakeLineOptions = {}) {
  const spec = ref<LineSpec>(
    options.spec ?? specOf('产线目标', [stageOf('swarm', 3), stageOf('solo')]),
  )
  const armed = ref(options.armed ?? false)
  const running = ref(false)
  const available = ref(options.available ?? true)
  const blocking = ref<string[]>(options.blocking ?? [])
  const gate = ref<GateDecision | 'unknown'>(options.gate ?? 'allow')
  const draftGoal = ref(options.draftGoal ?? '')
  const preflight = ref<LinePreflight | null>(null)
  const preflightLoading = ref(false)
  const preflightError = ref<string | null>(null)
  const plan = ref<LinePlan | null>(null)
  const planLoading = ref(false)
  const planError = ref<string | null>(null)

  const setSpec = vi.fn((next: LineSpec) => {
    spec.value = next
  })
  const armNextFromLegacy = vi.fn(() => true)
  const refreshPreflight = vi.fn(async () => {})

  // 接口里这几个字段是 ComputedRef，夹具也给同一形态。
  const catalog = computed<LineCatalog>(() => CATALOG)
  const catalogLoading = computed(() => false)
  const catalogError = computed<string | null>(() =>
    available.value ? null : 'LINE_CATALOG_UNAVAILABLE',
  )
  const catalogAvailable = computed(() => available.value)
  const warnings = computed<string[]>(() => [])
  const blockingErrors = computed(() => blocking.value)
  const gateDecision = computed(() => gate.value)
  const shapeText = computed(() =>
    summarizeShape(
      spec.value,
      (pattern) => CATALOG.patterns.find((p) => p.id === pattern)?.label ?? pattern,
    ),
  )
  const canRun = computed(
    () => armed.value && catalogAvailable.value && !running.value && blockingErrors.value.length === 0,
  )

  const workflow = {
    spec,
    armed,
    running,
    catalog,
    catalogLoading,
    catalogError,
    catalogAvailable,
    preflight,
    preflightLoading,
    preflightError,
    plan,
    planLoading,
    planError,
    errors: blockingErrors,
    warnings,
    blockingErrors,
    gateDecision,
    canRun,
    shapeText,
    draftGoal,
    setDraftGoal: vi.fn(),
    lastRun: ref<LineRunResult | null>(null),
    lastRunError: ref<string | null>(null),
    retro: ref<LineRetro | null>(null),
    retroLoading: ref(false),
    setSpec,
    applyRecipeById: vi.fn(() => true),
    armNextFromLegacy,
    refreshPreflight,
    dispatch: vi.fn(async (): Promise<LineDispatchOutcome> => ({
      status: 'error',
      message: 'NOT_USED',
    })),
    loadRetro: vi.fn(async () => {}),
  } satisfies LineWorkflow

  return {
    workflow,
    spec,
    armed,
    available,
    blocking,
    gate,
    running,
    canRun,
    draftGoal,
    setSpec,
    armNextFromLegacy,
    refreshPreflight,
  }
}

type FakeLine = ReturnType<typeof fakeLine>

interface DockProps {
  gearKey?: string
  legacyForm?: LegacyOrchestrationForm
  legacyKnobs?: { agents: string[]; teams: string[]; swarmSize: number }
}

const mounted: App[] = []

function mountDock(line: LineWorkflow, extra: DockProps = {}): HTMLElement {
  const el = document.createElement('div')
  document.body.appendChild(el)
  const app = createApp(LineDock, {
    line,
    gearKey: extra.gearKey ?? 'gear-swarm',
    legacyForm: extra.legacyForm ?? 'swarm',
    legacyKnobs: extra.legacyKnobs ?? { agents: [], teams: [], swarmSize: 5 },
  })
  app.mount(el)
  mounted.push(app)
  return el
}

function byTestId(el: HTMLElement, testId: string): Element | null {
  return el.querySelector(`[data-testid="${testId}"]`)
}

function mustFind(el: HTMLElement, testId: string): Element {
  const node = byTestId(el, testId)
  expect(node, `缺少 [data-testid=${testId}]`).toBeTruthy()
  return node as Element
}

function textOf(el: HTMLElement, testId: string): string {
  return (mustFind(el, testId).textContent ?? '').trim()
}

/** 触发点击并等 Vue 完成一次渲染。 */
async function click(node: Element | null): Promise<void> {
  expect(node).toBeTruthy()
  node!.dispatchEvent(new Event('click'))
  await nextTick()
}

beforeEach(() => {
  try {
    localStorage.clear()
  } catch {
    // 无 localStorage 的环境：持久化用例自己会失败，而不是被静默跳过
  }
})

afterEach(() => {
  while (mounted.length) {
    try {
      mounted.pop()!.unmount()
    } catch {
      /* 已卸载 */
    }
  }
  document.body.innerHTML = ''
  vi.restoreAllMocks()
})

/* ── 用例 ─────────────────────────────────────────────────────────────────── */

describe('LineDock', () => {
  it('后端没有 line.catalog RPC ⇒ 整条不渲染，也不留任何假控件', () => {
    const line = fakeLine({ available: false })
    const el = mountDock(line.workflow)

    expect(byTestId(el, 'line-dock')).toBe(null)
    expect(byTestId(el, 'line-arm-dock')).toBe(null)
    expect(byTestId(el, 'line-dock-toggle')).toBe(null)
    expect(byTestId(el, 'line-composer-stub')).toBe(null)
    expect((el.textContent ?? '').trim()).toBe('')
  })

  it('收起态一行内给出形状摘要与闸门裁决，且不挂载装配台', () => {
    const el = mountDock(fakeLine().workflow)

    expect(byTestId(el, 'line-dock')).toBeTruthy()
    expect(textOf(el, 'line-dock-shape')).toBe('3 蜂群 → 单体')
    expect(textOf(el, 'line-dock-gate')).toBe('Allowed')
    expect(mustFind(el, 'line-dock-gate').classList.contains('ld__gate--allow')).toBe(true)
    expect(byTestId(el, 'line-composer-stub')).toBe(null)

    const toggle = mustFind(el, 'line-dock-toggle')
    expect(toggle.getAttribute('aria-expanded')).toBe('false')
    expect(toggle.getAttribute('aria-label')).toBe('Expand line dock')
  })

  it('闸门按后端裁决换皮：confirm / deny / unknown 各有自己的类与文案', async () => {
    const line = fakeLine({ gate: 'confirm' })
    const el = mountDock(line.workflow)

    expect(mustFind(el, 'line-dock-gate').classList.contains('ld__gate--confirm')).toBe(true)
    expect(textOf(el, 'line-dock-gate')).toBe('Confirm')

    line.gate.value = 'deny'
    await nextTick()
    expect(mustFind(el, 'line-dock-gate').classList.contains('ld__gate--deny')).toBe(true)
    expect(textOf(el, 'line-dock-gate')).toBe('Denied')

    line.gate.value = 'unknown'
    await nextTick()
    expect(textOf(el, 'line-dock-gate')).toBe('Gate unknown')
    expect(mustFind(el, 'line-dock-gate').getAttribute('title')).toBe('No preflight verdict yet')
  })

  it('展开/收起写进 localStorage，重新挂载沿用存储值', async () => {
    const el = mountDock(fakeLine().workflow)

    await click(byTestId(el, 'line-dock-toggle'))
    expect(byTestId(el, 'line-composer-stub')).toBeTruthy()
    expect(localStorage.getItem(OPEN_KEY)).toBe('true')
    expect(mustFind(el, 'line-dock-toggle').getAttribute('aria-label')).toBe('Collapse line dock')

    await click(byTestId(el, 'line-dock-toggle'))
    expect(byTestId(el, 'line-composer-stub')).toBe(null)
    expect(localStorage.getItem(OPEN_KEY)).toBe('false')

    // 初始值 = 存储值（与 line.armed 无关）
    localStorage.setItem(OPEN_KEY, 'true')
    const reopened = mountDock(fakeLine({ armed: false }).workflow)
    expect(byTestId(reopened, 'line-composer-stub')).toBeTruthy()
  })

  it('没有存储值时按挂载时的 armed 决定初始展开，且首帧不回写存储', () => {
    const armedEl = mountDock(fakeLine({ armed: true }).workflow)
    expect(byTestId(armedEl, 'line-composer-stub')).toBeTruthy()
    expect(localStorage.getItem(OPEN_KEY)).toBe(null)

    localStorage.clear()
    const offEl = mountDock(fakeLine({ armed: false }).workflow)
    expect(byTestId(offEl, 'line-composer-stub')).toBe(null)
  })

  it('localStorage 抛错（隐私模式 / 配额满）只丢偏好，不影响展开', async () => {
    const el = mountDock(fakeLine().workflow)
    const setItem = vi.spyOn(localStorage, 'setItem').mockImplementation(() => {
      throw new Error('QuotaExceededError')
    })

    await click(byTestId(el, 'line-dock-toggle'))
    expect(byTestId(el, 'line-composer-stub')).toBeTruthy()
    expect(setItem).toHaveBeenCalled()
  })

  it('开关翻转的是工作流实例上的 armed 本身（role=switch 与 aria-checked 同步）', async () => {
    const line = fakeLine()
    const el = mountDock(line.workflow)
    const arm = mustFind(el, 'line-arm-dock')

    expect(arm.getAttribute('role')).toBe('switch')
    expect(arm.getAttribute('aria-checked')).toBe('false')
    expect(textOf(el, 'line-arm-dock')).toBe('Off')

    await click(arm)
    expect(line.armed.value).toBe(true)
    expect(textOf(el, 'line-arm-dock')).toBe('Armed')
    expect(mustFind(el, 'line-arm-dock').getAttribute('aria-checked')).toBe('true')

    await click(byTestId(el, 'line-arm-dock'))
    expect(line.armed.value).toBe(false)
  })

  it('形状被拦时开关拒这一次上膛，并把第一条原因说出来（与装配台同一语义）', async () => {
    const line = fakeLine({ blocking: ['LINE_PREFLIGHT_EMPTY'] })
    const el = mountDock(line.workflow)

    await click(byTestId(el, 'line-arm-dock'))
    expect(line.armed.value).toBe(false)
    // 裸机器码不许出现在界面上：与 LineComposer 一样过 lineCodeText 翻译。
    expect(textOf(el, 'line-dock-arm-refused')).toContain('preflight came back empty')

    // 阻塞清空后同一动作立刻生效，且拒绝提示自己消失（不留一条过期告警）。
    line.blocking.value = []
    await nextTick()
    await click(byTestId(el, 'line-arm-dock'))
    expect(line.armed.value).toBe(true)
    expect(byTestId(el, 'line-dock-arm-refused')).toBe(null)
  })

  it('挂线但派不出去时如实报阻塞条数，不让开关看着像能用', async () => {
    const line = fakeLine({
      armed: true,
      blocking: ['s-panel：名册里的团队已不存在', 'goal 不能为空'],
    })
    const el = mountDock(line.workflow)

    expect(line.canRun.value).toBe(false)
    const blocked = mustFind(el, 'line-dock-blocked')
    expect(blocked.getAttribute('role')).toBe('status')
    expect(textOf(el, 'line-dock-blocked')).toContain('blocked (2)')

    // 阻塞清空 ⇒ 这行消失（canRun 恢复）
    line.blocking.value = []
    await nextTick()
    expect(line.canRun.value).toBe(true)
    expect(byTestId(el, 'line-dock-blocked')).toBe(null)
  })

  it('未挂线时即使有阻塞也不说"已挂线但被阻塞"——那句话只描述挂上了却发不出去', () => {
    const el = mountDock(fakeLine({ armed: false, blocking: ['goal 不能为空'] }).workflow)
    expect(byTestId(el, 'line-dock-blocked')).toBe(null)
  })

  it('从当前挡位生成：带 knobs 与 form 调工作流，成功后展开装配台', async () => {
    const line = fakeLine()
    const el = mountDock(line.workflow, {
      legacyForm: 'swarm',
      legacyKnobs: { agents: [], teams: [], swarmSize: 5 },
    })

    await click(byTestId(el, 'line-dock-from-gear'))
    expect(line.armNextFromLegacy.mock.calls).toEqual([
      [{ agents: [], teams: [], swarmSize: 5 }, 'swarm'],
    ])
    expect(byTestId(el, 'line-dock-from-gear-failed')).toBe(null)
    expect(byTestId(el, 'line-composer-stub')).toBeTruthy()
  })

  it('生成失败时给一行 role=status 的原因，绝不静默空转', async () => {
    const line = fakeLine()
    line.armNextFromLegacy.mockReturnValue(false)
    const el = mountDock(line.workflow, { legacyForm: 'team' })

    await click(byTestId(el, 'line-dock-from-gear'))
    const failed = mustFind(el, 'line-dock-from-gear-failed')
    expect(failed.getAttribute('role')).toBe('status')
    expect(textOf(el, 'line-dock-from-gear-failed')).toContain('cannot be compiled')
    // 失败不该把用户带进一个看起来什么都没发生的收起态
    expect(byTestId(el, 'line-composer-stub')).toBe(null)
  })

  it('旧挡位是 single ⇒ 不给出"生成"按钮（那等于把线清成 solo）', () => {
    const line = fakeLine()
    const el = mountDock(line.workflow, { legacyForm: 'single' })
    expect(byTestId(el, 'line-dock-from-gear')).toBe(null)
    expect(line.armNextFromLegacy).not.toHaveBeenCalled()
  })

  it('展开后装配台的 update:armed / preflight-refresh 落回同一个实例，不留第二份状态', async () => {
    const line: FakeLine = fakeLine()
    const el = mountDock(line.workflow)
    await click(byTestId(el, 'line-dock-toggle'))

    await click(byTestId(el, 'stub-emit-armed'))
    expect(line.armed.value).toBe(true)
    expect(textOf(el, 'stub-armed')).toBe('true')

    await click(byTestId(el, 'stub-emit-refresh'))
    expect(line.refreshPreflight).toHaveBeenCalledTimes(1)
    expect(textOf(el, 'stub-blocking')).toBe('0')

    await click(byTestId(el, 'stub-emit-spec'))
    expect(line.setSpec).toHaveBeenCalledTimes(1)
    expect(line.spec.value.goal).toBe('装配台改写目标')
  })

  it('本轮目标（输入框那句话）原样递给装配台，不在停靠条这一层丢掉', async () => {
    const line: FakeLine = fakeLine({ armed: true, draftGoal: '把两份方案比出个结论' })
    const el = mountDock(line.workflow)

    expect(textOf(el, 'stub-draft')).toBe('把两份方案比出个结论')

    line.draftGoal.value = '改了一句'
    await nextTick()
    expect(textOf(el, 'stub-draft')).toBe('改了一句')
  })
})
