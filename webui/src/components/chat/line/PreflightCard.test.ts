// 预演卡的执行映射段（line.plan 的唯一消费者）。
//
// 锁的是这段功能存在的理由：卡片声明的形状与真正会被调用的东西**同源**，
// 而那两条诚实说明（swarm 实为同步会诊、solo 实为单个编排子智能体）只在 `note` 里 ——
// 不渲染出来就等于继续让"蜂群"这个词骗人。
// 另一半要锁的是"配角不许抢戏"：没有计划时段落整块消失（不留空壳），
// 而预演表必须在计划失败/缺席时照常站着。
//
// 挂载方式沿用仓内既有范式（`createApp(...).mount(el)` + querySelector），
// **不引入 @vue/test-utils** —— 项目没装它，加依赖会撞 `--webui` 的 lockfile 自洽闸。
// @vitest-environment happy-dom

import { afterEach, describe, expect, it, vi } from 'vitest'
import { createApp, nextTick, type App } from 'vue'

const messages = vi.hoisted((): Record<string, string> => ({
  'chat.line.preflightTitle': 'Preflight',
  'chat.line.preflightRunning': 'Running',
  'chat.line.colStage': 'Stage',
  'chat.line.colTactic': 'Tactic',
  'chat.line.colSuccess': 'Success',
  'chat.line.colP90Ms': 'P90 time',
  'chat.line.colP90Tokens': 'P90 tokens',
  'chat.line.totalRow': 'Total',
  'chat.line.productNote': '{stages} stages multiplied -> {rate}',
  'chat.line.plan.title': 'What actually gets called',
  'chat.line.plan.hint': 'Same source as the run.',
  'chat.line.plan.backendLabel': 'Backend',
  'chat.line.plan.castLabel': 'Cast',
  'chat.line.plan.autonomyLabel': 'Autonomy',
  'chat.line.gate.allow': 'Auto-approved at {autonomy} autonomy',
  'chat.line.gate.confirm': 'Needs confirmation at {autonomy} autonomy',
  'chat.line.gate.deny': 'Denied at {autonomy} autonomy',
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

import PreflightCard from './PreflightCard.vue'
import type { LineCatalog, LinePattern, LinePlan, LinePreflight, LineSpec, LineStage } from '@/composables/line/types'

/* ── 夹具 ─────────────────────────────────────────────────────────────────── */

function stageOf(pattern: LinePattern, width = 1, roster: string[] = []): LineStage {
  return { id: `s-${pattern}`, pattern, width, roster, facets: [], synthesize: false, note: '' }
}

const SPEC: LineSpec = {
  specId: 'ls-card-fixture',
  goal: '产线目标',
  acceptance: [],
  stages: [stageOf('swarm', 3), stageOf('panel', 2, ['team-a'])],
  criteria: { maxFailProbability: 0.25, budgetTokens: null, deadlineMs: null },
}

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
  legacyForms: [],
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

function preflightFixture(): LinePreflight {
  return {
    ok: true,
    shape: '3 蜂群 → 专家组',
    stages: [{ tactic: 'swarm', p90Ms: 60_000, p90Tokens: 900, successRate: 0.9 }],
    total: {
      p50Ms: 40_000,
      p90Ms: 90_000,
      p50Tokens: 500,
      p90Tokens: 1_500,
      p90CostUsd: 0.2,
      successRate: 0.81,
      failProbability: 0.19,
    },
    rationale: {},
    premortem: [],
    unknowns: [],
    chosen: 'swarm+panel',
    chosenVsRecommended: null,
    validation: { valid: true, errors: [], warnings: [], maxConcurrent: 4 },
    gate: { autonomy: 'standard_doit', decision: 'allow', reason: '' },
  }
}

function planFixture(overrides: Partial<LinePlan> = {}): LinePlan {
  return {
    ok: true,
    shape: '3 蜂群 → 专家组',
    spec: SPEC,
    stages: [
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
      {
        index: 1,
        pattern: 'panel',
        width: 2,
        form: 'team',
        backend: 'team.run',
        args: { goal: '产线目标' },
        roster: ['team-a'],
        note: '',
        autonomy: 'high_autonomy',
      },
    ],
    validation: { valid: true, errors: [], warnings: [], maxConcurrent: 4 },
    gate: { autonomy: 'standard_doit', decision: 'allow', reason: '' },
    ...overrides,
  }
}

/* ── 挂载 ─────────────────────────────────────────────────────────────────── */

const mounted: App[] = []

function mountCard(plan: LinePlan | null): HTMLElement {
  const el = document.createElement('div')
  document.body.appendChild(el)
  const app = createApp(PreflightCard, {
    preflight: preflightFixture(),
    loading: false,
    spec: SPEC,
    catalog: CATALOG,
    plan,
  })
  app.mount(el)
  mounted.push(app)
  return el
}

function byTestId(el: HTMLElement, testId: string): Element | null {
  return el.querySelector(`[data-testid="${testId}"]`)
}

function rows(el: HTMLElement): Element[] {
  return [...el.querySelectorAll('[data-testid="line-plan-stage"]')]
}

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

describe('PreflightCard 执行映射', () => {
  it('每个阶段一行：backend、出演、自主度与那条诚实说明都摆在界面上', () => {
    const el = mountCard(planFixture())
    const list = byTestId(el, 'line-plan-stages')
    expect(list).toBeTruthy()
    expect((list!.textContent ?? '')).toContain('What actually gets called')

    const cells = rows(el)
    expect(cells).toHaveLength(2)
    // 标签来自目录（不是前端复制的常量），宽度只在 >1 时才占位。
    expect(cells[0].textContent).toContain('蜂群 ×3')
    expect(cells[0].textContent).toContain('team.run')
    expect(cells[0].textContent).toContain('high_autonomy')
    // 这一条是本段存在的核心理由：swarm 其实是同步会诊，不是后台蜂群。
    expect(cells[0].textContent).toContain('同步会诊')
    // 没有名册的形状显示占位，有名册的按 · 连接。
    expect(cells[0].textContent).toContain('—')
    expect(cells[1].textContent).toContain('team-a')
    // note 为空 ⇒ 不渲染空壳行
    expect(cells[1].querySelector('.pf__planNote')).toBe(null)
  })

  it('没有计划 ⇒ 整段不出现，而预演表照常站着（配角不许把主表带下水）', () => {
    const el = mountCard(null)
    expect(byTestId(el, 'line-plan-stages')).toBe(null)
    expect(byTestId(el, 'line-preflight-stage')).toBeTruthy()
    expect(byTestId(el, 'line-preflight-total')).toBeTruthy()
  })

  it('计划阶段为空 / ok:false ⇒ 同样一个字节都不渲染', () => {
    expect(rows(mountCard(planFixture({ stages: [] }))))
      .toHaveLength(0)
    const emptyList = document.body.querySelectorAll('[data-testid="line-plan-stages"]')
    expect(emptyList).toHaveLength(0)

    document.body.innerHTML = ''
    const rejected = mountCard(
      planFixture({ ok: false, stages: [], validation: { valid: false, errors: ['INVALID_SPEC'], warnings: [], maxConcurrent: 0 } }),
    )
    expect(byTestId(rejected, 'line-plan-stages')).toBe(null)
    expect(byTestId(rejected, 'line-preflight-total')).toBeTruthy()
  })

  it('字段漂移时如实留空：没带 autonomy 就不摆一个猜出来的档位', async () => {
    const drifted = planFixture()
    // 模拟后端某天不再给这两个字段（契约镜像之外的字段不得被前端"补"出来）。
    const row = drifted.stages[0] as unknown as Record<string, unknown>
    delete row.autonomy
    delete row.args
    const el = mountCard(drifted)
    await nextTick()

    const first = rows(el)[0]
    expect(first.querySelector('.pf__planAutonomy')).toBe(null)
    expect(first.textContent).toContain('team.run')
    expect(first.textContent).not.toContain('undefined')
  })
})
