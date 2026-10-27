// @vitest-environment happy-dom
import { createApp, nextTick } from 'vue'
import { afterEach, describe, expect, it } from 'vitest'
import i18n from '@/i18n'
import TerminalPanel from './TerminalPanel.vue'

function mountPanel(props: Record<string, unknown>) {
  const host = document.createElement('div')
  document.body.appendChild(host)
  const app = createApp(TerminalPanel, props)
  app.use(i18n)
  app.mount(host)
  return { app, host }
}

afterEach(() => {
  document.body.innerHTML = ''
})

describe('TerminalPanel（多工作区）', () => {
  it('多根时顶部显示工作根数量，悬停标题列出全部路径', async () => {
    const { app, host } = mountPanel({
      cwd: 'D:/repo/main',
      connected: true,
      running: true,
      roots: ['D:/repo/main', 'D:/repo/lib', 'D:/repo/docs'],
    })
    await nextTick()

    const badge = host.querySelector('[data-testid="terminal-roots"]')
    expect(badge).toBeTruthy()
    expect(badge?.textContent).toContain('3 workspaces')
    expect(badge?.getAttribute('title')).toContain('D:/repo/main')
    expect(badge?.getAttribute('title')).toContain('D:/repo/lib')
    expect(badge?.getAttribute('title')).toContain('D:/repo/docs')
    app.unmount()
  })

  it('单根时保持原有展示（不显示工作根数量）', async () => {
    const { app, host } = mountPanel({
      cwd: 'D:/repo/main',
      connected: true,
      running: true,
      roots: ['D:/repo/main'],
    })
    await nextTick()

    expect(host.querySelector('[data-testid="terminal-roots"]')).toBeNull()
    expect(host.textContent).toContain('D:/repo/main')
    app.unmount()
  })

  it('后端回落的 warning 以提示样式展示，不静默吞掉', async () => {
    const { app, host } = mountPanel({
      cwd: 'D:/repo/main',
      connected: true,
      running: true,
      warning: '请求的目录不在允许的工作区内，已回落',
    })
    await nextTick()

    const warning = host.querySelector('[data-testid="terminal-warning"]')
    expect(warning).toBeTruthy()
    expect(warning?.textContent).toContain('已回落')
    app.unmount()
  })
})
