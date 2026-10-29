// @vitest-environment happy-dom

import { afterEach, describe, expect, it, vi } from 'vitest'
import { createApp, nextTick, type App } from 'vue'
import i18n from '@/i18n'
import SidebarConversations, { type SidebarSectionRow } from './SidebarConversations.vue'

const mounted: App[] = []

afterEach(() => {
  mounted.splice(0).forEach(app => app.unmount())
  document.body.innerHTML = ''
})

function chatRow(overrides: Partial<SidebarSectionRow> = {}): SidebarSectionRow {
  return {
    rowKind: 'session',
    key: 'agent:main:webchat:reset-me',
    title: 'Reset me',
    effectiveAgentId: 'main',
    agentName: 'Main',
    sessionKind: 'chat',
    depth: 0,
    runStatus: 'idle',
    runLabel: 'Idle',
    taskAttention: 'none',
    updatedAt: Date.now(),
    hasContractGaps: false,
    ...overrides,
  }
}

async function mountSidebar(row: SidebarSectionRow) {
  i18n.global.locale.value = 'en'
  const onReset = vi.fn()
  const root = document.createElement('div')
  document.body.appendChild(root)
  const app = createApp(SidebarConversations, {
    sections: [{ family: 'chats', label: 'Tasks', rows: [row] }],
    error: false,
    loading: false,
    currentKey: '',
    contractDebugEnabled: false,
    searchHint: 'Ctrl+K',
    onReset,
  })
  app.use(i18n)
  app.mount(root)
  mounted.push(app)
  await nextTick()
  return { root, onReset }
}

function menuItems(): HTMLButtonElement[] {
  return [...document.body.querySelectorAll<HTMLButtonElement>('.sidebar-row-menu [role="menuitem"]')]
}

async function openRowMenu(root: HTMLElement) {
  root.querySelector<HTMLButtonElement>('.sidebar-row-menu-btn')?.click()
  await nextTick()
}

describe('SidebarConversations clear conversation', () => {
  it('offers clear between rename and delete and only hands the intent to the owner', async () => {
    const { root, onReset } = await mountSidebar(chatRow())
    await openRowMenu(root)

    const labels = menuItems().map(item => item.textContent?.trim())
    expect(labels).toEqual(['Pin task', 'Rename', 'Clear conversation', 'Delete'])

    // 清空保留会话壳，所以不做成与删除同色的危险项。
    const clear = menuItems().find(item => item.textContent?.trim() === 'Clear conversation')
    expect(clear?.classList.contains('sidebar-row-menu__item--danger')).toBe(false)

    clear?.click()
    await nextTick()

    expect(onReset).toHaveBeenCalledWith('agent:main:webchat:reset-me')
    expect(menuItems()).toHaveLength(0)
    // 行仍在列表里：清空不是删除。
    expect(root.textContent).toContain('Reset me')
  })

  it('keeps the row menu (and therefore clear) away from provisional drafts', async () => {
    const { root } = await mountSidebar(chatRow({ provisional: true }))
    expect(root.querySelector('.sidebar-row-menu-btn')).toBeNull()
  })
})
