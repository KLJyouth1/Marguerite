// @vitest-environment happy-dom
/**
 * 成员卡片本身：一个成员一张卡，卡上每个字段都能指回 `team.members` 的某一列。
 */
import { createApp, h } from 'vue'
import { createI18n } from 'vue-i18n'
import { afterEach, describe, expect, it } from 'vitest'

import TeamMemberCard from './TeamMemberCard.vue'
import {
  EMPTY_RUN_INFO,
  emptyMemberActivity,
  type MemberLiveActivity,
  type TeamMember,
  type TeamRunInfo,
} from '@/composables/useTeamBoard'

const mountedApps: ReturnType<typeof createApp>[] = []

const i18n = createI18n({
  legacy: false,
  locale: 'en',
  messages: {
    en: {
      console: { channels: { notReported: 'Not reported' } },
      usageLogs: { metrics: { tokens: 'Tokens' } },
      team: {
        board: {
          toolCalls: 'tool calls',
          lastActivity: 'Last activity',
          status: {
            running: 'Running',
            waiting: 'Waiting',
            finished: 'Finished',
            failed: 'Failed',
          },
        },
      },
      chat: {
        thinking: 'Thinking',
        activityDurationSeconds: '{seconds}s',
        activityDurationMinutes: '{minutes}m {seconds}s',
        orchestration: {
          formTeam: 'Team',
          formSwarm: 'Swarm',
          runMemberThinking: 'Member reasoning',
          runMemberOutcome: 'Outcome',
          runMemberNoTools: 'No tool calls yet',
          runSynthesizing: 'Lead is synthesizing member results',
        },
      },
      harness: { runs: { started: 'Started', duration: 'Duration' } },
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
    currentStep: 'patching the guard',
    toolCalls: 7,
    lastActivityMs: null,
    tokensUsed: null,
    ...overrides,
  }
}

function mountCard(props: {
  member: TeamMember
  status: string
  activity?: MemberLiveActivity | null
  run?: TeamRunInfo | null
}) {
  const host = document.createElement('div')
  document.body.appendChild(host)
  const app = createApp({ render: () => h(TeamMemberCard, props) })
  mountedApps.push(app)
  app.use(i18n)
  app.mount(host)
  return host
}

afterEach(() => {
  while (mountedApps.length) mountedApps.pop()?.unmount()
  document.body.innerHTML = ''
})

describe('TeamMemberCard identity + stats', () => {
  it('carries the member id, role, kind and merged status on the card itself', () => {
    const host = mountCard({
      member: member({ kind: 'expert', status: 'finished' }),
      status: 'finished',
      run: { ...EMPTY_RUN_INFO, runId: 'run-0000abcd', form: 'team' },
    })

    const card = host.querySelector<HTMLElement>('.team-card')!
    expect(card.getAttribute('data-member-id')).toBe('sub-1')
    expect(card.getAttribute('data-status')).toBe('finished')
    expect(card.getAttribute('data-live')).toBe('false')
    expect(card.querySelector('.team-card__name')?.textContent?.trim()).toBe('Bolt')
    expect(card.querySelector('.team-card__role')?.textContent?.trim()).toBe('coder')
    expect(card.querySelector('.team-card__avatar')?.textContent?.trim()).toBe('B')
    expect(card.querySelector('.team-card__status')?.className).toBe('team-card__status team-status--finished')
    expect(card.querySelector('[data-testid="member-run-label"]')?.textContent?.trim()).toBe('Team · #0000abcd')
  })

  it('shows an em dash for tokens the backend reports as null', () => {
    const host = mountCard({ member: member({ tokensUsed: null, toolCalls: 0 }), status: 'running' })

    expect(host.querySelector('.team-card__stats')?.textContent).toContain('Tokens —')
    // toolCalls = 0 是真实计数，不走空态
    expect(host.querySelector('.team-card__stats')?.textContent).toContain('0')
    expect(host.querySelector('.team-card__stat')?.textContent?.trim()).toBe('0 tool calls')
  })

  it('falls back to the raw status label for a value the board does not know', () => {
    const host = mountCard({ member: member(), status: 'queued_by_backend' })

    expect(host.querySelector('.team-card__status')?.className).toBe('team-card__status team-status--unknown')
    expect(host.querySelector('.team-card__status')?.textContent?.trim()).toBe('queued_by_backend')
  })

  it('renders the empty-step placeholder instead of an invented step', () => {
    const host = mountCard({ member: member({ currentStep: null }), status: 'waiting' })
    const step = host.querySelector<HTMLElement>('.team-card__step')!

    expect(step.textContent?.trim()).toBe('Not reported')
    expect(step.className).toContain('team-card__step--empty')
    expect(host.querySelector<HTMLElement>('.team-card[data-status="waiting"]')).not.toBeNull()
  })

  it('flags a card that has live observations and maps thinking onto its own badge', () => {
    const activity = emptyMemberActivity('sub-1', 'run-1', 1_000)
    const host = mountCard({ member: member(), status: 'thinking', activity: { ...activity, lastPhase: 'member_thinking' } })

    expect(host.querySelector('.team-card')?.getAttribute('data-live')).toBe('true')
    expect(host.querySelector('.team-card__status')?.className).toBe('team-card__status team-status--thinking')
    expect(host.querySelector('.team-card__status')?.textContent?.trim()).toBe('Thinking')
  })
})
