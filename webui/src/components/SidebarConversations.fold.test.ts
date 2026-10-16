// @vitest-environment happy-dom
// A1 折叠守卫（R65）：单个「项目/最近」区默认只渲染最近 RECENTS_PAGE_SIZE 条会话行，
// 其余收进「显示更多（还有 N 条）」。此前**没有任何自动化覆盖**——真机探针能测出
// rowCount 27→202，但把上限改成无限大时单测全绿。
//
// 本用例锁定三条语义（变异自检：把 `RECENTS_PAGE_SIZE` 改为极大 → 必红）：
//   1) 折叠真的发生（渲染数 < 总数）；
//   2) 上限与总数**无关**（30 条与 120 条渲染同样多 = 固定上限，而非“全部渲染”）；
//   3) 「显示更多」出现、点开后全量渲染并切换为「收起」。
import { afterEach, describe, expect, it, vi } from 'vitest'
import { createApp, defineComponent, h, nextTick, type App } from 'vue'
import { createI18n } from 'vue-i18n'
import SidebarConversations from './SidebarConversations.vue'
import type { SidebarSection, SidebarSectionRow } from '@/composables/useSessions'

vi.mock('@/composables/useConfirm', () => ({
  useConfirm: () => ({ confirm: vi.fn(async () => true) }),
}))

const mountedApps: App<Element>[] = []

function chatRow(i: number): SidebarSectionRow {
  return {
    rowKind: 'session',
    key: `agent:main:webchat:task-${i}`,
    title: `Task ${i}`,
    effectiveAgentId: 'main',
    agentName: 'Main',
    sessionKind: 'chat',
    depth: 0,
    runStatus: 'idle',
    runLabel: 'Idle',
    taskAttention: 'none',
    // 老时间戳 → 走绝对日期分支，避免依赖 sessions.relTime.* 文案键。
    updatedAt: 1,
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

async function mountSidebar(count: number) {
  const rows: SidebarSectionRow[] = Array.from({ length: count }, (_, i) => chatRow(i))
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

function renderedSessionRows(host: HTMLElement): number {
  return host.querySelectorAll('.sidebar-history-row[data-session-key]').length
}

afterEach(() => {
  mountedApps.splice(0).forEach((app) => app.unmount())
  document.body.innerHTML = ''
  localStorage.clear()
})

describe('SidebarConversations Recents folding (A1)', () => {
  it('folds the list and offers 显示更多（还有 N 条）', async () => {
    const { host } = await mountSidebar(30)
    const rendered = renderedSessionRows(host)
    expect(rendered, '必须渲染出会话行').toBeGreaterThan(0)
    expect(rendered, '必须折叠掉一部分，不能把 30 条平铺').toBeLessThan(30)

    const more = host.querySelector('[data-testid="sidebar-recents-more"]')
    expect(more, '应出现「显示更多」按钮').not.toBeNull()
    expect(more!.textContent ?? '').toContain(String(30 - rendered))
  })

  it('the cap is independent of the total (fixed cap, not "render all")', async () => {
    const a = await mountSidebar(30)
    const b = await mountSidebar(120)
    const cap30 = renderedSessionRows(a.host)
    const cap120 = renderedSessionRows(b.host)
    expect(cap30).toBeLessThan(30)
    expect(cap120, '总数翻倍时渲染数不变 = 固定上限').toBe(cap30)
  })

  it('expanding reveals every row and switches the control to 收起', async () => {
    const { host } = await mountSidebar(30)
    const before = renderedSessionRows(host)
    expect(before).toBeLessThan(30)

    ;(host.querySelector('[data-testid="sidebar-recents-more"]') as HTMLButtonElement).click()
    await nextTick()

    expect(renderedSessionRows(host)).toBe(30)
    expect(host.querySelector('[data-testid="sidebar-recents-less"]')).not.toBeNull()
    expect(host.querySelector('[data-testid="sidebar-recents-more"]')).toBeNull()
  })
})
