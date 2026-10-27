// @vitest-environment happy-dom

import { createApp, nextTick } from 'vue'
import { afterEach, describe, expect, it } from 'vitest'
import i18n from '@/i18n'
import AppStatusBar from './AppStatusBar.vue'

const mounted: Array<ReturnType<typeof createApp>> = []

afterEach(() => {
  mounted.splice(0).forEach(app => app.unmount())
  document.body.innerHTML = ''
})

function mount(overrides: Record<string, unknown> = {}) {
  i18n.global.locale.value = 'en'
  const root = document.createElement('div')
  document.body.appendChild(root)
  const app = createApp(AppStatusBar, {
    project: 'marguerite',
    projectPath: 'D:/work/marguerite',
    mode: 'Local mode',
    branch: 'master',
    detached: false,
    connectionLabel: 'Ready',
    connectionTone: 'ready',
    ...overrides,
  })
  app.use(i18n)
  app.mount(root)
  mounted.push(app)
  return root
}

describe('AppStatusBar', () => {
  it('shows project, mode, branch, connection and version', async () => {
    const root = mount()
    await nextTick()
    const text = root.querySelector('.app-status-bar')?.textContent?.replace(/\s+/g, ' ')
    expect(text).toContain('marguerite')
    expect(text).toContain('Local mode')
    expect(text).toContain('master')
    expect(text).toContain('Ready')
    expect(text).toMatch(/v\d+\.\d+\.\d+/)
    expect(root.querySelector('.app-status-bar__project')?.getAttribute('title')).toBe('D:/work/marguerite')
  })

  it('marks the connection state without inventing a branch', async () => {
    const root = mount({ branch: null, detached: true, connectionLabel: 'Disconnected', connectionTone: 'disconnected' })
    await nextTick()
    expect(root.querySelector('.app-status-bar')?.classList.contains('app-status-bar--disconnected')).toBe(true)
    expect(root.querySelector('.app-status-bar')?.textContent).toContain('Detached HEAD')
    expect(root.querySelector('.app-status-bar__branch')).toBeNull()
  })

  it('does not render a fake project path when none is bound', async () => {
    const root = mount({ project: 'No project', projectPath: '' })
    await nextTick()
    const project = root.querySelector<HTMLElement>('.app-status-bar__project')
    expect(project?.textContent).toContain('No project')
    expect(project?.getAttribute('title')).toBe('No project')
  })
})
