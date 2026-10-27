// 应用右键菜单契约（2026-10-25）。
//
// 锁三件事，每条都对应一个用户真实撞到的失败：
//   1. 桌面端右键曾弹 Chromium 默认菜单（重新加载/另存为/检查）→ 非输入区必须换成应用菜单；
//   2. 输入框/文本域必须保留原生菜单（否则粘贴、拼写检查全没了）；
//   3. Esc 必须能关掉（无边框壳里没有别的键盘出口）。
//
// 挂载沿用仓内范式（createApp + mount），不引入 @vue/test-utils。

// @vitest-environment happy-dom

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { createApp, nextTick, type App } from 'vue'
import { createI18n } from 'vue-i18n'

const pushMock = vi.fn()

vi.mock('vue-router', async () => {
  const actual = await vi.importActual<typeof import('vue-router')>('vue-router')
  return { ...actual, useRouter: () => ({ push: pushMock }) }
})

import AppContextMenu from './AppContextMenu.vue'

const mounted: App<Element>[] = []

function mountMenu() {
  const host = document.createElement('div')
  document.body.appendChild(host)
  const app = createApp(AppContextMenu)
  app.use(
    createI18n({
      legacy: false,
      locale: 'en',
      messages: {
        en: {
          contextMenu: { copy: 'Copy', selectAll: 'Select all', reload: 'Reload interface' },
          nav: { settings: 'Settings' },
        },
      },
    }),
  )
  app.mount(host)
  mounted.push(app)
  return host
}

function menuEl(): HTMLElement | null {
  return document.querySelector('[data-testid="app-context-menu"]')
}

async function rightClick(target: Element) {
  target.dispatchEvent(
    new MouseEvent('contextmenu', { bubbles: true, cancelable: true, clientX: 40, clientY: 60 }),
  )
  await nextTick()
}

describe('AppContextMenu —— 桌面端右键必须是应用菜单', () => {
  beforeEach(() => {
    pushMock.mockClear()
  })

  afterEach(() => {
    for (const app of mounted.splice(0)) app.unmount()
    document.body.innerHTML = ''
  })

  it('在普通区域右键：出现应用菜单，且条目齐全', async () => {
    mountMenu()
    await rightClick(document.body)
    const el = menuEl()
    expect(el, '非输入区右键应出现应用菜单').toBeTruthy()
    expect(el!.querySelectorAll('[role="menuitem"]').length).toBe(4)
    expect(el!.querySelector('[data-testid="ctx-copy"]')).toBeTruthy()
    expect(el!.querySelector('[data-testid="ctx-settings"]')).toBeTruthy()
  })

  it('在输入框里右键：不弹应用菜单（保留原生粘贴菜单）', async () => {
    mountMenu()
    const input = document.createElement('input')
    document.body.appendChild(input)
    await rightClick(input)
    expect(menuEl(), '输入框右键不应被应用菜单接管').toBeNull()
  })

  it('Esc 关闭菜单', async () => {
    mountMenu()
    await rightClick(document.body)
    expect(menuEl()).toBeTruthy()
    document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }))
    await nextTick()
    expect(menuEl()).toBeNull()
  })

  it('组件自己已处理过的右键不再叠一层（defaultPrevented 尊重）', async () => {
    mountMenu()
    const custom = document.createElement('div')
    document.body.appendChild(custom)
    custom.addEventListener('contextmenu', (ev) => ev.preventDefault())
    await rightClick(custom)
    expect(menuEl(), '组件已有自己的菜单时不应叠加').toBeNull()
  })
})
