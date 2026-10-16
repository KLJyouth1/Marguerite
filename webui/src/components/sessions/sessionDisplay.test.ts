import { describe, expect, it } from 'vitest'
import { sessionAgentIdentity, sessionStatusBadge, workspaceBadge } from './sessionDisplay'
import type { SessionItem } from '@/composables/useSessions'

function sessionItem(overrides: Partial<SessionItem>): SessionItem {
  return {
    key: 'agent:main:webchat:test',
    title: 'Test chat',
    subtitle: '',
    groupLabel: 'main',
    effectiveAgentId: 'main',
    sessionKind: 'chat',
    surface: 'webchat',
    conversationKind: 'direct',
    threadLabel: '',
    channelContext: null,
    status: 'killed',
    visualStatus: 'killed',
    runStatus: 'cancelled',
    runLabel: 'Stopped after 1s',
    messageCount: 1,
    updatedAt: 1000,
    interactive: true,
    forkedFromParent: false,
    contractGaps: [],
    raw: { key: 'agent:main:webchat:test' },
    ...overrides,
  }
}

describe('sessionStatusBadge', () => {
  it('shows the normalized stop label for cancelled sessions', () => {
    const badge = sessionStatusBadge(sessionItem({ runStatus: 'cancelled', runLabel: 'Stopped after 1s' }))

    expect(badge?.label).toBe('Stopped after 1s')
  })

  it('shows the normalized stop label for interrupted sessions', () => {
    const badge = sessionStatusBadge(sessionItem({
      runStatus: 'interrupted',
      runLabel: 'Output interrupted',
    }))

    expect(badge?.label).toBe('Output interrupted')
  })
})

describe('sessionAgentIdentity', () => {
  const agents = new Map([['main', 'Main agent']])

  it('does not report deletion while agents.list has not succeeded', () => {
    expect(sessionAgentIdentity('retired', agents, false)).toEqual({
      kind: 'raw',
      value: 'retired',
    })
  })

  it('reports a missing effective agent only after the catalog loaded', () => {
    expect(sessionAgentIdentity('retired', agents, true)).toEqual({
      kind: 'deleted',
      value: 'retired',
    })
    expect(sessionAgentIdentity('main', agents, true)).toEqual({
      kind: 'known',
      value: 'Main agent',
    })
  })
})

describe('workspaceBadge', () => {
  it('treats a bound project dir as the neutral state', () => {
    const badge = workspaceBadge({ root: 'D:\\proj', source: 'bound' })

    expect(badge.cls).toBe('is-bound')
    expect(badge.icon).toBe('folder')
  })

  it('flags both app-data fallbacks instead of rendering them neutral', () => {
    for (const source of ['app_data', 'process_cwd']) {
      const badge = workspaceBadge({ root: 'C:\\AppData', source })

      expect(badge.cls).toBe('is-unbound')
      expect(badge.icon).toBe('alertTriangle')
    }
  })

  it('flags a declared-but-missing dir distinctly from an unbound one', () => {
    const badge = workspaceBadge({ root: 'D:\\gone', source: 'missing' })

    expect(badge.cls).toBe('is-missing')
    expect(badge.icon).toBe('alertTriangle')
  })

  it('falls back to the unbound chip for an unknown source rather than claiming a binding', () => {
    const badge = workspaceBadge({ root: 'D:\\x', source: 'something_new' })

    expect(badge.cls).toBe('is-unbound')
  })

  /**
   * Guards the i18n wiring: every chip label must resolve to a real translation.
   * A missing locale key renders `sessions.inspect.*` verbatim to the user.
   */
  it('never renders a raw i18n key for any workspace state', () => {
    for (const source of ['bound', 'app_data', 'process_cwd', 'missing']) {
      const { label } = workspaceBadge({ root: 'x', source })

      expect(label.trim()).not.toBe('')
      expect(label).not.toContain('sessions.inspect')
    }
  })
})
