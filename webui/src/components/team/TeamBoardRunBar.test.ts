// @vitest-environment happy-dom
/**
 * 运行条：把「这几张卡属于哪一次编排 / 这份读数有多可信」如实说出来。
 *
 * 重点是 `dropped` 与 `fromDisk` —— 后端一直在发，此前前端整段丢掉，
 * 于是被缓冲挤掉的成员和「上一次运行」都伪装成当前事实。
 */
import { createApp, h } from 'vue'
import { createI18n } from 'vue-i18n'
import { afterEach, describe, expect, it } from 'vitest'

import TeamBoardRunBar from './TeamBoardRunBar.vue'
import { EMPTY_RUN_INFO, type TeamRunInfo } from '@/composables/useTeamBoard'

const mountedApps: ReturnType<typeof createApp>[] = []

const i18n = createI18n({
  legacy: false,
  locale: 'en',
  messages: {
    en: {
      team: { board: { lastActivity: 'Last activity' } },
      chat: {
        orchestration: {
          memberCount: '{n} members',
          runFilesMore: '{n} more not listed',
          formTeam: 'Team',
          formSwarm: 'Swarm',
        },
      },
      cronSkills: { list: { metaLastRun: 'Last run' } },
    },
  },
})

function runInfo(overrides: Partial<TeamRunInfo> = {}): TeamRunInfo {
  return { ...EMPTY_RUN_INFO, runId: 'run-0000abcd', form: 'team', ...overrides }
}

function mountBar(run: TeamRunInfo, memberCount: number, updatedAt: number | null = null) {
  const host = document.createElement('div')
  document.body.appendChild(host)
  const app = createApp({ render: () => h(TeamBoardRunBar, { run, memberCount, updatedAt }) })
  mountedApps.push(app)
  app.use(i18n)
  app.mount(host)
  return host
}

const text = (host: HTMLElement, testid: string) =>
  host.querySelector<HTMLElement>(`[data-testid="${testid}"]`)?.textContent?.trim() ?? ''

afterEach(() => {
  while (mountedApps.length) mountedApps.pop()?.unmount()
  document.body.innerHTML = ''
})

describe('TeamBoardRunBar', () => {
  it('names the run the cards belong to', () => {
    const host = mountBar(runInfo({ form: 'swarm', runId: 'run-0000abcd' }), 2)

    expect(host.querySelector('[data-testid="team-run-bar-form"]')?.getAttribute('data-form')).toBe('swarm')
    expect(host.querySelector('[data-testid="team-run-bar-form"]')?.textContent?.trim()).toBe('Swarm')
    expect(text(host, 'team-run-bar-run-id')).toBe('#0000abcd')
    expect(text(host, 'team-run-bar-members')).toBe('2 members')
  })

  it('keeps the member count as a defined numerator: cards minus the synthesized lead row', () => {
    // 视图传进来的就是「卡数 − lead 行」，这里锁住调用方给的 3 而不是 4
    const host = mountBar(runInfo({ memberLimit: 512 }), 3)
    expect(text(host, 'team-run-bar-members')).toBe('3 members')
    expect(host.querySelector('[data-testid="team-run-bar-dropped"]')).toBeNull()
  })

  it('announces dropped members instead of hiding them behind the buffer', () => {
    const host = mountBar(runInfo({ dropped: 37, memberLimit: 512 }), 512)

    expect(text(host, 'team-run-bar-dropped')).toBe('37 more not listed')
    expect(host.querySelector('[data-testid="team-run-bar-dropped"]')?.getAttribute('title')).toBe('512/512')
  })

  it('marks a disk snapshot as the previous run', () => {
    const host = mountBar(runInfo({ fromDisk: true, note: 'no orchestration run yet' }), 0)

    expect(text(host, 'team-run-bar-from-disk')).toBe('Last run')
    expect(text(host, 'team-run-bar-note')).toBe('no orchestration run yet')
  })

  it('renders no timestamp when the backend did not provide one', () => {
    const host = mountBar(runInfo(), 1, null)

    expect(host.querySelector('[data-testid="team-run-bar-updated"]')).toBeNull()
    expect(host.textContent).not.toContain('Last activity')
  })

  it('relativizes the updatedAt the backend already sends', () => {
    const host = mountBar(runInfo(), 1, Date.now() - 180_000)

    expect(text(host, 'team-run-bar-updated')).toContain('Last activity')
    expect(host.querySelector('[data-testid="team-run-bar-updated"]')?.textContent).toContain('minute')
  })

  it('shows an unknown form verbatim instead of guessing a shape', () => {
    const host = mountBar(runInfo({ form: 'council' }), 1)
    expect(host.querySelector('[data-testid="team-run-bar-form"]')?.textContent?.trim()).toBe('council')
  })

  it('renders no form chip when the run has no form', () => {
    const host = mountBar(runInfo({ form: '' }), 1)
    expect(host.querySelector('[data-testid="team-run-bar-form"]')).toBeNull()
  })
})
