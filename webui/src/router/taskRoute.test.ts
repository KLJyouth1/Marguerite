// @vitest-environment happy-dom
import { createApp } from 'vue'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { createPinia } from 'pinia'
import type { RouteLocationNormalized } from 'vue-router'
import i18n, { loadLocaleMessages } from '@/i18n'
import { defaultRootRedirect } from './sharedRoutes'
import { routeTitle, routes } from './index'
import TaskWorkspaceView from '@/views/TaskWorkspaceView.vue'

beforeEach(() => {
  localStorage.clear()
  i18n.global.locale.value = 'en'
  delete window.MargueriteDesktop
  window.matchMedia = vi.fn().mockImplementation((query: string) => ({
    matches: false,
    media: query,
    onchange: null,
    addEventListener: vi.fn(),
    removeEventListener: vi.fn(),
    addListener: vi.fn(),
    removeListener: vi.fn(),
    dispatchEvent: vi.fn(),
  }))
})

describe('/task route skeleton', () => {
  it('registers the task workspace route lazily and keeps /chat as the landing page', () => {
    const task = routes.find(route => route.path === '/task')

    expect(task).toBeDefined()
    expect(task?.name).toBe('task')
    expect(task?.meta?.titleKey).toBe('task.title')
    expect(task?.meta?.icon).toBe('listChecks')
    expect(typeof task?.component).toBe('function')
    expect(defaultRootRedirect()).toBe('/chat')
  })

  // Gap4: /task graduated from a URL-only route to a first-class Work
  // destination, so it carries the primary rail metadata. The flat rail order
  // itself is guarded by nav.test.ts.
  it('carries the primary Work rail metadata', () => {
    const task = routes.find(route => route.path === '/task')

    expect(task?.meta?.nav).toBe('primary')
    expect(task?.meta?.navLabelKey).toBe('task.title')
  })

  it('localizes the task document title from task.title', async () => {
    await loadLocaleMessages('zh-Hans')
    i18n.global.locale.value = 'zh-Hans'
    const task = routes.find(route => route.path === '/task')

    expect(routeTitle({
      name: task?.name,
      meta: task?.meta,
    } as unknown as RouteLocationNormalized)).toBe('任务工作区')

    i18n.global.locale.value = 'en'
  })

  it('renders the live task workspace shell for /task', async () => {
    const host = document.createElement('div')
    document.body.appendChild(host)
    const app = createApp(TaskWorkspaceView)
    // The view reads the shared useTaskWorkspace + activeMode stores, so it needs
    // a real Pinia (T05 wired the placeholder to live task state).
    app.use(createPinia())
    app.use(i18n)
    app.mount(host)

    expect(host.textContent).toContain('Task workspace')
    expect(host.textContent).toContain('No task is running yet.')

    app.unmount()
    document.body.innerHTML = ''
  })
})
