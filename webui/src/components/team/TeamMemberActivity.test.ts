// @vitest-environment happy-dom
/**
 * 成员动向区（一张卡的内容区）的诚实性测试。
 *
 * 核心断言只有一句话：**载荷缺失 = 空态，不是 0、不是猜测、不是上一位成员的数据**。
 * 这里的 activity 全部走 `reduceProgressFrame` 的真实产物形状（见 useTeamBoard），
 * 所以断言同时锁住「前端消费的字段名 == 后端事件载荷字段名」。
 */
import { createApp, h, nextTick } from 'vue'
import { createI18n } from 'vue-i18n'
import { afterEach, describe, expect, it } from 'vitest'

import TeamMemberActivity from './TeamMemberActivity.vue'
import {
  emptyMemberActivity,
  reduceProgressFrame,
  type MemberActivityMap,
  type MemberLiveActivity,
  type TeamMember,
} from '@/composables/useTeamBoard'

const mountedApps: ReturnType<typeof createApp>[] = []

const i18n = createI18n({
  legacy: false,
  locale: 'en',
  messages: {
    en: {
      console: { channels: { notReported: 'Not reported' } },
      harness: { runs: { started: 'Started', duration: 'Duration' } },
      team: { board: { toolCalls: 'tool calls' } },
      chat: {
        activityDurationSeconds: '{seconds}s',
        activityDurationMinutes: '{minutes}m {seconds}s',
        orchestration: {
          runMemberThinking: 'Member reasoning',
          runMemberOutcome: 'Outcome',
          runMemberNoTools: 'No tool calls yet',
          runSynthesizing: 'Lead is synthesizing member results',
        },
      },
    },
  },
})

function member(overrides: Partial<TeamMember> = {}): TeamMember {
  return {
    id: 'sub-1',
    label: 'Bolt',
    role: 'coder',
    kind: 'subagent',
    status: 'running',
    currentStep: 'reading tests',
    toolCalls: 2,
    lastActivityMs: null,
    tokensUsed: null,
    ...overrides,
  }
}

function mountActivity(activity: MemberLiveActivity | null, target = member()) {
  const host = document.createElement('div')
  document.body.appendChild(host)
  const app = createApp({ render: () => h(TeamMemberActivity, { member: target, activity }) })
  mountedApps.push(app)
  app.use(i18n)
  app.mount(host)
  return host
}

async function settle() {
  for (let index = 0; index < 4; index += 1) await Promise.resolve()
  await nextTick()
}

function dd(host: HTMLElement, testid: string): string {
  return host.querySelector<HTMLElement>(`[data-testid="${testid}"] dd`)?.textContent?.trim() ?? ''
}

/** 用真实 reducer 造 activity —— 避免测试与实现各写一份字段名。 */
function activityOf(frames: Array<Record<string, unknown>>, memberId = 'sub-1', startAt = 1_000): MemberLiveActivity | null {
  let map: MemberActivityMap = {}
  frames.forEach((frame, index) => {
    const next = reduceProgressFrame(map, { kind: 'orchestration', runId: 'run-1', memberId, ...frame }, startAt + index * 1_000)
    if (next) map = next
  })
  return map[memberId] ?? null
}

afterEach(() => {
  while (mountedApps.length) mountedApps.pop()?.unmount()
  document.body.innerHTML = ''
})

describe('TeamMemberActivity empty states (no fabricated data)', () => {
  it('renders Not reported on every row when no frame was ever observed', async () => {
    const host = mountActivity(null)
    await settle()

    expect(dd(host, 'member-thinking')).toBe('Not reported')
    expect(dd(host, 'member-outcome')).toBe('Not reported')
    expect(dd(host, 'member-started')).toBe('Not reported')
    expect(dd(host, 'member-duration')).toBe('Not reported')
    // 没观察到起点 → 不能替成员断言「还没调用工具」
    expect(dd(host, 'member-tool')).toBe('Not reported')
    expect(host.textContent).not.toContain('No tool calls yet')
    expect(host.textContent).not.toContain('0s')
  })

  it('keeps an empty reducer record empty instead of inventing a start time', async () => {
    const host = mountActivity(emptyMemberActivity('sub-1', 'run-1', 5_000))
    await settle()

    expect(dd(host, 'member-started')).toBe('Not reported')
    expect(dd(host, 'member-duration')).toBe('Not reported')
    expect(dd(host, 'member-thinking')).toBe('Not reported')
    expect(host.querySelector('[data-testid="member-synthesizing"]')).toBeNull()
    expect(host.querySelector('[data-testid="member-error"]')).toBeNull()
  })

  it('says "no tool calls yet" only for a member watched from its own member_started', async () => {
    const watched = activityOf([{ phase: 'member_started' }])
    const host = mountActivity(watched)
    await settle()

    expect(dd(host, 'member-started')).not.toBe('Not reported')
    expect(dd(host, 'member-tool')).toBe('No tool calls yet')
  })
})

describe('TeamMemberActivity payload mapping', () => {
  it('maps member_thinking / tool frames onto the reasoning and tool rows', async () => {
    const activity = activityOf([
      { phase: 'member_started' },
      { phase: 'member_thinking', round: 3, thinking: 'check the ratchet first' },
      { phase: 'tool_call', tool: 'shell' },
      { phase: 'tool_result', tool: 'shell', ok: false, elapsedMs: 1_200 },
    ])
    const host = mountActivity(activity)
    await settle()

    expect(dd(host, 'member-thinking')).toBe('check the ratchet first')
    const tool = host.querySelector<HTMLElement>('[data-testid="member-tool"] .member-activity__tool')
    expect(tool?.getAttribute('data-tool-state')).toBe('error')
    expect(tool?.textContent).toContain('shell')
    expect(tool?.querySelector('.member-activity__tool-mark')?.textContent).toBe('✗')
    // 没有 member_completed → 结论与耗时仍是空态
    expect(dd(host, 'member-outcome')).toBe('Not reported')
    expect(dd(host, 'member-duration')).toBe('Not reported')
  })

  it('formats the backend elapsedMs and shows the completion summary', async () => {
    const activity = activityOf([
      { phase: 'member_started' },
      { phase: 'member_completed', ok: true, elapsedMs: 45_000, summary: '3 assertions added' },
    ])
    const host = mountActivity(activity)
    await settle()

    expect(dd(host, 'member-duration')).toBe('45s')
    expect(dd(host, 'member-outcome')).toBe('3 assertions added')
  })

  it('switches to the minute format past 60 seconds', async () => {
    const activity = activityOf([
      { phase: 'member_started' },
      { phase: 'member_completed', elapsedMs: 90_500, summary: 'done' },
    ])
    const host = mountActivity(activity)
    await settle()

    expect(dd(host, 'member-duration')).toBe('1m 31s')
  })

  it('tints a timeout outcome as failed instead of a clean success', async () => {
    const activity = activityOf([
      { phase: 'member_started' },
      { phase: 'member_completed', ok: false, timedOut: true, summary: 'gave up' },
    ])
    const host = mountActivity(activity)
    await settle()

    const outcome = host.querySelector<HTMLElement>('[data-testid="member-outcome"] dd')
    expect(outcome?.className).toContain('member-activity__value--failed')
    expect(outcome?.textContent).toContain('gave up')
    expect(dd(host, 'member-duration')).toBe('Not reported')
  })

  it('shows the synthesizing hint only on a lead_synthesizing frame and the error only on run_failed', async () => {
    const synthesizing = activityOf([{ phase: 'lead_synthesizing' }], 'lead:run-1')
    const leadMember = member({ id: 'lead:run-1', kind: 'lead', label: 'Lead' })
    const host = mountActivity(synthesizing, leadMember)
    await settle()

    expect(host.querySelector('[data-testid="member-synthesizing"]')?.textContent).toContain('synthesizing member results')
    expect(host.querySelector('[data-testid="member-error"]')).toBeNull()

    const failed = activityOf([{ phase: 'run_failed', error: 'cancelled by user' }], 'lead:run-1')
    const second = mountActivity(failed, leadMember)
    await settle()

    expect(second.querySelector('[data-testid="member-error"]')?.textContent?.trim()).toBe('cancelled by user')
    expect(second.querySelector('[data-testid="member-synthesizing"]')).toBeNull()
  })
})
