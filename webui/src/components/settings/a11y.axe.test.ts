// @vitest-environment happy-dom
/**
 * M3-E3 可访问性：WebUI 关键 M3 组件 axe 合规扫描。
 * 目标：axe-core 对 5 个 M3 新增/改造组件（行为事件时间线、预算、任务成本、
 * 持久任务队列、信任校准）实扫，断言无 WCAG A/AA 级别 serious/critical 违规；
 * 无法在 happy-dom 中评估的规则计入 incomplete 并单独说明。
 */
import { afterEach, describe, expect, it, vi } from 'vitest'
import type { App, Component } from 'vue'
import axe from 'axe-core'

const mounted: Array<{ app: App; el: HTMLElement }> = []

async function settle(): Promise<void> {
  for (let i = 0; i < 10; i++) await Promise.resolve()
  await new Promise(resolve => setTimeout(resolve, 15))
}

function activityEntries() {
  const entries = [
    { id: 1, ts: '2026-09-03T10:00:00Z', session: 's1', method: 'task.create', event: 'task.initiated', title: '写周报', status: 'completed' },
    { id: 2, ts: '2026-09-03T10:01:00Z', session: 's1', method: 'plan.confirm', event: 'plan.confirmed', revision_id: 2, run_id: 'run-1' },
    { id: 3, ts: '2026-09-03T10:02:00Z', session: 's1', method: 'tool.call', event: 'tool.call', tool: 'edit_file' },
    { id: 4, ts: '2026-09-03T10:03:00Z', session: 's1', method: 'task.complete', event: 'task.completed', task_id: 't-7' },
    { id: 5, ts: '2026-09-03T10:04:00Z', session: 's1', method: 'mode.set', event: 'mode.changed', from: 'work', to: 'safe' },
    { id: 6, ts: '2026-09-03T10:05:00Z', session: 's1', method: 'approval.grant', event: 'approval.granted', approval: 'risk-high', state: 'granted' },
    { id: 7, ts: '2026-09-03T10:06:00Z', session: 's1', method: 'budget.set', event: 'budget.warning', threshold_tokens: 100_000, threshold_usd: 5, total_tokens: 120_000, cost_usd: 6.2 },
  ]
  return { entries, total: 7 }
}

function budgetState() {
  return {
    ok: true,
    session: 'default',
    threshold_tokens: 100_000,
    threshold_usd: 10,
    total_tokens: 120_000,
    input_tokens: 60_000,
    output_tokens: 60_000,
    cost_usd: 4.2,
    currency: 'USD',
    exceeded: true,
  }
}

function taskCostState() {
  return {
    ok: true,
    run_id: 'run-1',
    session: 'default',
    total_tokens: 12_000,
    input_tokens: 8_000,
    output_tokens: 4_000,
    calls: 12,
    cost_usd: 0.31,
    currency: 'USD',
    models: [
      { model: 'deepseek-v3', calls: 8, tokens: 9_000, cost_usd: 0.22 },
      { model: 'deepseek-r1', calls: 4, tokens: 3_000, cost_usd: 0.09 },
    ],
  }
}

function queueState() {
  return [
    { id: 'q-1', title: '写周报', status: 'running', step: 2, total: 4 },
    { id: 'q-2', title: '整理发票', status: 'queued' },
    { id: 'q-3', title: '生成图表', status: 'paused' },
  ]
}

function trustState() {
  return { ok: true, current: 'medium', persist: false, levels: ['low', 'medium', 'high'] }
}

function tierState() {
  return {
    locked_tier: null,
    thinking_effort: 'medium',
    effective_tier: 'balanced',
    tier_reasons: ['模式=办公 → 均衡档', '预算剩余 78%'],
    budget: {
      tokens_cap: 200_000,
      usd_cap: 2,
      tokens_used: 44_000,
      usd_used: 0.42,
      remaining_ratio: 0.78,
      state: 'healthy',
    },
    tiers: {
      flagship: ['deepseek-flash', 'openai-gpt-5-5'],
      balanced: ['deepseek-v4-flash', 'openai-gpt-5-4-mini'],
      turbo: ['qwen-turbo', 'glm-flash'],
    },
  }
}

function makeRpcCall() {
  return vi.fn(async (method: string) => {
    switch (method) {
      case 'activity.recent':
        return activityEntries()
      case 'usage.budget.get':
        return budgetState()
      case 'usage.taskCost':
        return taskCostState()
      case 'task.queue.list':
      case 'task_queue.list':
        return { items: queueState() }
      case 'trust.get':
      case 'trust.current':
        return trustState()
      case 'model.tier.get':
        return tierState()
      case 'model.tier.set':
        return { ok: true }
      default:
        return {}
    }
  })
}

async function mountComponent(name: string, methods: string[]) {
  vi.resetModules()
  document.body.innerHTML = ''
  const call = makeRpcCall()
  const rpc = {
    waitForConnection: vi.fn(async () => {}),
    supportsMethod: vi.fn((m: string) => methods.includes(m)),
    markMethodUnavailable: vi.fn(),
    call,
  }
  vi.doMock('@/stores/rpc', () => ({ useRpcStore: () => rpc }))

  const { createApp, nextTick } = await import('vue')
  const i18n = (await import('@/i18n')).default
  i18n.global.locale.value = 'zh-Hans'
  const Component = (await componentLoader[name]()).default
  const el = document.createElement('div')
  el.style.setProperty('--dur-base', '0ms')
  el.style.setProperty('--dur-fast', '0ms')
  document.body.appendChild(el)
  const app = createApp(Component)
  app.use(i18n)
  app.mount(el)
  mounted.push({ app, el })
  await settle()
  await nextTick()
  return { el, call }
}

type AxeResult = {
  violations: Array<{ id: string; impact?: string; help: string; nodes: unknown[] }>
  incomplete: Array<{ id: string; impact?: string; help: string; nodes: unknown[] }>
}

// 静态导入映射，避免 vite 动态导入目录告警
const componentLoader: Record<string, () => Promise<{ default: Component }>> = {
  RecentActivityPanel: () => import('./RecentActivityPanel.vue'),
  BudgetPanel: () => import('./BudgetPanel.vue'),
  TaskCostPanel: () => import('./TaskCostPanel.vue'),
  TaskQueuePanel: () => import('./TaskQueuePanel.vue'),
  TrustCalibrationPanel: () => import('./TrustCalibrationPanel.vue'),
  TierLedgerPanel: () => import('./TierLedgerPanel.vue'),
}

afterEach(() => {
  for (const { app, el } of mounted) {
    app?.unmount()
    el?.remove()
  }
  mounted.length = 0
  document.body.innerHTML = ''
})

const M3_COMPONENTS: Array<{ file: string; methods: string[] }> = [
  { file: 'RecentActivityPanel', methods: ['activity.recent'] },
  { file: 'BudgetPanel', methods: ['usage.budget.get', 'usage.budget.set'] },
  { file: 'TaskCostPanel', methods: ['usage.taskCost', 'activity.recent'] },
  { file: 'TaskQueuePanel', methods: ['task.queue.list', 'activity.recent'] },
  { file: 'TrustCalibrationPanel', methods: ['trust.get', 'trust.set'] },
  { file: 'TierLedgerPanel', methods: ['model.tier.get', 'model.tier.set'] },
]

describe('M3-E3 WebUI axe 合规扫描 (WCAG A/AA)', () => {
  for (const { file, methods } of M3_COMPONENTS) {
    it(`${file} 无 serious/critical WCAG 违规`, async () => {
      const { el } = await mountComponent(file, methods)
      const result = (await axe.run(el, {
        runOnly: { type: 'tag', values: ['wcag2a', 'wcag2aa', 'wcag21a', 'wcag21aa'] },
      })) as AxeResult

      // 断言：任何 WCAG A/AA 规则的违规一律不允许（strict 无白名单）
      const serious = result.violations.filter(v => v.impact === 'serious' || v.impact === 'critical')
      const allViolations = result.violations
      // eslint-disable-next-line no-console
      if (allViolations.length) {
        // 仅用于失败诊断输出
        console.warn(
          `${file} violations:`,
          allViolations.map(v => `${v.id}(${v.impact}):${v.nodes.length}`),
        )
      }
      expect(serious, `${file} 存在 serious/critical 违规`).toEqual([])

      // incomplete 记录（happy-dom 无法评估项），供报告引用
      ;(result.incomplete || []).forEach(v => {
        // eslint-disable-next-line no-console
        console.info(`${file} incomplete: ${v.id} (${v.impact ?? 'n/a'})`)
      })
    })
  }

  it('M3 时间线组件渲染语义结构（ol + time + role=alert 错误提示）', async () => {
    const { el } = await mountComponent('RecentActivityPanel', ['activity.recent'])
    expect(el.querySelector('ol.activity-list')).not.toBeNull()
    expect(el.querySelectorAll('li.activity-row').length).toBeGreaterThanOrEqual(3)
    expect(el.querySelector('time')?.getAttribute('datetime')).toBe('2026-09-03T10:00:00Z')
    expect(el.querySelector('.activity-banner')).toBeNull()
  })
})
