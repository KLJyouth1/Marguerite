// @vitest-environment happy-dom
// A2/A8 时间态守卫（R65 复核）：A8 的「真实区分信息」只应在**当天**用裸 HH:MM；
// 跨天必须退回与其余会话行一致的绝对日期，否则三天前的 `01:28` 会被误读成「今天」。
//
// 变异自检：把 `distinctTimeText` 的跨天分支改回 `formatClockTime(ts)` → 本用例必红。
import { afterEach, describe, expect, it, vi } from 'vitest'
import { createApp, defineComponent, h, nextTick, type App } from 'vue'
import { createI18n } from 'vue-i18n'
import SidebarConversations from './SidebarConversations.vue'
import { sessionRelativeTime } from '@/utils/messageTime'
import type { SidebarSection, SidebarSectionRow } from '@/composables/useSessions'

vi.mock('@/composables/useConfirm', () => ({
  useConfirm: () => ({ confirm: vi.fn(async () => true) }),
}))

const mountedApps: App<Element>[] = []

// Title length 2 (<= AMBIGUOUS_TITLE_MAX) => needsDistinctMeta() is always true,
// which is exactly the A8 path that switches to a real timestamp.
function ambiguousRow(key: string, updatedAt: number): SidebarSectionRow {
  return {
    rowKind: 'session',
    key,
    title: 'hi',
    effectiveAgentId: 'main',
    agentName: 'Main',
    sessionKind: 'chat',
    depth: 0,
    runStatus: 'idle',
    runLabel: 'Idle',
    taskAttention: 'none',
    updatedAt,
    hasContractGaps: false,
  }
}

function i18n() {
  return createI18n({
    legacy: false,
    locale: 'en',
    messages: {
      en: {
        chrome: { searchChats: 'Search tasks' },
        sessions: { filter: { chats: 'Tasks' } },
        shared: {
          sidebar: {
            recentConversations: 'Recent tasks',
            recents: 'Recents',
            noConversations: 'No tasks yet.',
            refresh: 'Refresh',
            enterSelectionMode: 'Select tasks',
            rowActions: 'Actions for {title}',
            pinTask: 'Pin task',
            unpinTask: 'Unpin task',
            statusLabel: '{status}',
            rename: 'Rename',
            delete: 'Delete',
            showMore: 'Show {count} more',
            showLess: 'Show less',
            collapse: 'Collapse',
            expand: 'Expand',
          },
        },
        workspaces: { projects: 'Projects', moreActions: 'Project actions' },
      },
    },
  })
}

async function mountSidebar(rows: SidebarSectionRow[]) {
  const sections: SidebarSection[] = [{ family: 'chats', label: 'Tasks', rows }]
  const noop = vi.fn()
  const host = document.createElement('div')
  document.body.appendChild(host)
  const Root = defineComponent(() => () => h(SidebarConversations, {
    sections,
    error: false,
    loading: false,
    currentKey: '',
    contractDebugEnabled: false,
    searchHint: 'Ctrl+K',
    canManageProjects: false,
    canCreateProjects: false,
    onSelect: noop,
    onNewProject: noop,
    onNewProjectTask: noop,
    onProjectPin: noop,
    onProjectEdit: noop,
    onProjectDeleteHistory: noop,
    onProjectRemove: noop,
    onReorder: noop,
    onSessionPin: noop,
  }))
  const app = createApp(Root)
  app.use(i18n())
  app.mount(host)
  mountedApps.push(app)
  await nextTick()
  return { host }
}

function metaFor(host: HTMLElement, key: string): string {
  const row = host.querySelector(`.sidebar-history-row[data-session-key="${key}"]`)
  expect(row, `row ${key} must render`).not.toBeNull()
  return row!.querySelector('.sidebar-idle-time')?.textContent?.trim() ?? ''
}

afterEach(() => {
  mountedApps.splice(0).forEach((app) => app.unmount())
  document.body.innerHTML = ''
  localStorage.clear()
})

describe('SidebarConversations ambiguous-row time (A2/A8)', () => {
  it('uses a bare HH:MM only for same-day rows; falls back to the absolute date past 24h', async () => {
    const today = new Date()
    today.setHours(1, 28, 0, 0)
    const sameDayTs = today.getTime()

    const older = new Date()
    older.setDate(older.getDate() - 3)
    older.setHours(9, 15, 0, 0)
    const olderTs = older.getTime()

    const { host } = await mountSidebar([
      ambiguousRow('agent:main:webchat:today', sameDayTs),
      ambiguousRow('agent:main:webchat:older', olderTs),
    ])

    // Same day -> real clock time, used to tell two same-titled rows apart.
    expect(metaFor(host, 'agent:main:webchat:today')).toBe('01:28')

    // Older than 24h -> never a bare clock; must match the shared session-row label.
    const olderMeta = metaFor(host, 'agent:main:webchat:older')
    expect(olderMeta, '跨天不得再是裸 HH:MM').not.toMatch(/^\d{1,2}:\d{2}$/)
    expect(olderMeta).toBe(sessionRelativeTime(olderTs))
  })
})
