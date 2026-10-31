// LineRunPanel 契约测试：回执卡 + 复盘单的装配行为——
// 尤其是"复盘失败也必须开单并显示原因"这条诚实性契约（空白回执 = 缺陷）。
// @vitest-environment happy-dom
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { nextTick, ref, type App, type Ref } from 'vue'

const mocks = vi.hoisted(() => ({
  messages: {
    'chat.line.run.title': 'Run receipt',
    'chat.line.run.retroButton': 'Retro',
    'chat.line.run.retroLoading': 'Loading retro…',
    'chat.line.retro.title': '5-Why retro',
    'chat.line.retro.error': 'Retro failed: {error}',
    'chat.line.retro.errorUnknown': 'Unknown retro failure',
    'chat.line.retro.idle': 'No retro requested yet.',
    'chat.line.retro.loading': 'Loading retro…',
    'chat.line.retro.done': 'Done',
    'chat.line.code.retroUnavailable': 'The retro could not be produced.',
    'chat.line.code.runNotPersisted': 'The run record was never persisted.',
    'common.close': 'Close',
  } as Record<string, string>,
}))

vi.mock('vue-i18n', () => ({
  useI18n: () => ({
    t: (key: string, values?: Record<string, unknown>) => {
      let value = mocks.messages[key] ?? key
      for (const [name, replacement] of Object.entries(values ?? {})) {
        value = value.replace(`{${name}}`, String(replacement))
      }
      return value
    },
  }),
}))

vi.mock('@/components/Icon.vue', async () => {
  const { defineComponent, h } = await import('vue')
  return {
    default: defineComponent({
      name: 'IconStub',
      props: { name: { type: String, default: '' } },
      setup(props) {
        return () => h('span', { 'data-icon': props.name })
      },
    }),
  }
})

import LineRunPanel from './LineRunPanel.vue'
import type { LineWorkflow } from '@/composables/line/useLineWorkflow'
import type { LineCatalog, LineRetro, LineRunResult } from '@/composables/line/types'

/* ── 夹具：假工作流，只喂面板契约声明会用到的成员 ─────────────────────────── */

const RUN_ID = 'line-run-1'

function soloCatalog(): LineCatalog {
  return {
    loaded: true,
    patterns: [
      {
        id: 'solo',
        label: 'Solo',
        description: 'single worker',
        rosterKind: 'none',
        needsSynthesis: false,
        topology: 'solo',
      },
    ],
    recipes: [],
    legacyForms: [],
    roles: [],
    teams: [],
    facets: [],
    limits: null,
    defaults: null,
  } as unknown as LineCatalog
}

function fixtureRun(runId = RUN_ID): LineRunResult {
  return {
    runId,
    sessionKey: 'agent:main:webchat:line',
    at: '2026-10-30T00:00:00Z',
    shape: '1 Solo',
    spec: {
      specId: 'ls-test',
      goal: 'fixture goal',
      acceptance: [],
      stages: [
        { id: 's-1', pattern: 'solo', width: 1, roster: [], facets: [], synthesize: false, note: '' },
      ],
      criteria: { maxFailProbability: 0.25, budgetTokens: null, deadlineMs: null },
    },
    stages: [],
    verdict: {
      ok: true,
      failures: [],
      elapsedMsActual: 1200,
      elapsedMsPredicted: 1000,
      tokensPredicted: 200,
      successRatePredicted: 0.9,
    },
    preflight: { ok: true, total: { p90CostUsd: null } },
    gate: { autonomy: 'standard_doit', decision: 'allow', reason: '' },
    storage: { persisted: true, path: '/tmp/line/line-run-1.json', bytes: 512, pruned: 0 },
  } as unknown as LineRunResult
}

function fixtureRetro(runId = RUN_ID): LineRetro {
  return {
    ok: true,
    runId,
    whyChain: [],
    measuredVsPredicted: {
      elapsedMsActual: 1200,
      elapsedMsPredicted: 1000,
      successRatePredicted: 0.9,
      comparable: true,
    },
    claims: [],
    nextPreflightHint: '',
    gaps: [],
    deltas: [],
  } as unknown as LineRetro
}

interface FakeLine {
  catalog: Ref<LineCatalog>
  lastRun: Ref<LineRunResult | null>
  lastRunError: Ref<string | null>
  retro: Ref<LineRetro | null>
  retroLoading: Ref<boolean>
  loadRetro: ReturnType<typeof vi.fn>
}

function fakeLine(run: LineRunResult | null): FakeLine {
  return {
    catalog: ref(soloCatalog()),
    lastRun: ref(run),
    lastRunError: ref<string | null>(null),
    retro: ref<LineRetro | null>(null),
    retroLoading: ref(false),
    loadRetro: vi.fn(async () => {}),
  }
}

/* ── 挂载 ─────────────────────────────────────────────────────────────────── */

const mountedApps: Array<{ app: App; el: HTMLElement }> = []

async function flush() {
  for (let i = 0; i < 8; i += 1) await Promise.resolve()
  await nextTick()
  await nextTick()
}

async function mountPanel(line: FakeLine): Promise<HTMLElement> {
  const { createApp } = await import('vue')
  const el = document.createElement('div')
  document.body.appendChild(el)
  const app = createApp(LineRunPanel, { line: line as unknown as LineWorkflow })
  app.mount(el)
  mountedApps.push({ app, el })
  await flush()
  return el
}

function clickRetro(el: HTMLElement): void {
  const button = el.querySelector<HTMLButtonElement>('[data-testid="line-run-retro"]')
  expect(button).toBeTruthy()
  button!.click()
}

beforeEach(() => {
  document.body.innerHTML = ''
  vi.clearAllMocks()
})

afterEach(() => {
  while (mountedApps.length) {
    const { app, el } = mountedApps.pop()!
    app.unmount()
    el.remove()
  }
})

/* ── 用例 ─────────────────────────────────────────────────────────────────── */

describe('LineRunPanel', () => {
  it('没有运行记录时面板整体不渲染（内部 v-if，不依赖父级）', async () => {
    const el = await mountPanel(fakeLine(null))
    expect(el.querySelector('[data-testid="line-run-panel"]')).toBeNull()
    expect(el.querySelector('[data-testid="line-run-card"]')).toBeNull()
  })

  it('有运行记录 ⇒ 渲染回执卡；点复盘按钮先调 loadRetro(runId) 再开单', async () => {
    const line = fakeLine(fixtureRun())
    line.loadRetro.mockImplementation(async (runId: string) => {
      line.retro.value = fixtureRetro(runId)
    })
    const el = await mountPanel(line)

    expect(el.querySelector('[data-testid="line-run-panel"]')).toBeTruthy()
    expect(el.querySelector('[data-testid="line-run-card"]')).toBeTruthy()
    // 单子未请求前不展开
    expect(el.querySelector('[data-testid="line-five-why"]')).toBeNull()

    clickRetro(el)
    await flush()

    expect(line.loadRetro).toHaveBeenCalledTimes(1)
    expect(line.loadRetro.mock.calls[0][0]).toBe(RUN_ID)
    const sheet = el.querySelector('[data-testid="line-five-why"]')
    expect(sheet).toBeTruthy()
    // 单头 runId 用的是请求时记下的那个值
    expect(el.querySelector('.why__runId')?.textContent).toContain(RUN_ID)
    // 成功路径没有错误条
    expect(el.querySelector('[data-testid="line-retro-error"]')).toBeNull()
  })

  it('复盘失败（retro 归零 + lastRunError）也必须开单，且把原因透传进单内', async () => {
    const line = fakeLine(fixtureRun())
    line.loadRetro.mockImplementation(async () => {
      line.retro.value = null
      line.lastRunError.value = '运行记录不存在'
    })
    const el = await mountPanel(line)

    clickRetro(el)
    await flush()

    expect(line.loadRetro).toHaveBeenCalledWith(RUN_ID)
    expect(el.querySelector('[data-testid="line-five-why"]')).toBeTruthy()
    const error = el.querySelector('[data-testid="line-retro-error"]')
    expect(error).toBeTruthy()
    expect(error?.textContent).toContain('运行记录不存在')
  })

  it('关闭按钮收起复盘单，回执卡仍在', async () => {
    const line = fakeLine(fixtureRun())
    line.loadRetro.mockImplementation(async () => {
      line.retro.value = fixtureRetro(RUN_ID)
    })
    const el = await mountPanel(line)

    clickRetro(el)
    await flush()
    expect(el.querySelector('[data-testid="line-five-why"]')).toBeTruthy()

    const close = el.querySelector<HTMLButtonElement>('[data-testid="line-retro-close"]')
    expect(close).toBeTruthy()
    close!.click()
    await flush()

    expect(el.querySelector('[data-testid="line-five-why"]')).toBeNull()
    expect(el.querySelector('[data-testid="line-run-card"]')).toBeTruthy()
  })

  it('LINE_* 机器码经 lineCodeText 翻译后进单，界面不出现裸码', async () => {
    const line = fakeLine(fixtureRun())
    line.loadRetro.mockImplementation(async () => {
      line.retro.value = null
      line.lastRunError.value = 'LINE_RETRO_UNAVAILABLE'
    })
    const el = await mountPanel(line)

    clickRetro(el)
    await flush()

    const error = el.querySelector('[data-testid="line-retro-error"]')
    expect(error).toBeTruthy()
    expect(error?.textContent).toContain('The retro could not be produced.')
    expect(error?.textContent).not.toContain('LINE_RETRO_UNAVAILABLE')
  })

  it('派单错误写在 lastRunError 上时不得污染复盘单（单子未绑定该次复盘失败）', async () => {
    const line = fakeLine(fixtureRun())
    line.loadRetro.mockImplementation(async () => {
      line.retro.value = fixtureRetro(RUN_ID)
    })
    const el = await mountPanel(line)

    // 复盘成功之后，工作流里再挂一条派单错误（dispatch 复用同一个槽位）
    clickRetro(el)
    await flush()
    line.lastRunError.value = 'LINE_NOT_ARMED'
    await flush()

    expect(el.querySelector('[data-testid="line-retro-error"]')).toBeNull()
    expect(el.querySelector('[data-testid="line-five-why"]')).toBeTruthy()
  })
})
