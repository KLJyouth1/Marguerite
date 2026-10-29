// 「桌面端右键弹出浏览器菜单」用户投诉的回归锁定（2026-10-26）。
//
// 与 AppContextMenu.test.ts 的分工：那份钉「应用菜单出现」，这份钉**原生菜单被抑制**
// 这个更本质的契约 —— 只有没人接管的原生 `contextmenu` 默认动作才是「浏览器菜单」。
// 三条行为保证 + 一条接线守卫：
//   ① 无任何处理者接管时，document 级必须 preventDefault（原生菜单永不出现）；
//   ② 组件已自行处理（宠物根 `@contextmenu.prevent` 同款）时不叠加、不反转；
//   ③ 输入框保留原生粘贴菜单：既不弹应用菜单，也**不得** preventDefault；
//   ④ 接线守卫：主窗口必须挂载 AppContextMenu，宠物两种形态必须保留自有处理点
//      （删掉任意一条，浏览器菜单就会重新出现——正是本次投诉本身）。
//
// @vitest-environment happy-dom

import { readFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'

import { afterEach, describe, expect, it, vi } from 'vitest'
import { createApp, nextTick, type App } from 'vue'
import { createI18n } from 'vue-i18n'

vi.mock('vue-router', async () => {
  const actual = await vi.importActual<typeof import('vue-router')>('vue-router')
  return { ...actual, useRouter: () => ({ push: vi.fn() }) }
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
}

function menuEl(): HTMLElement | null {
  return document.querySelector('[data-testid="app-context-menu"]')
}

/** 派发一次可取消的右键并返回事件本身（defaultPrevented 是唯一客观判据）。 */
function rightClick(target: Element): MouseEvent {
  const ev = new MouseEvent('contextmenu', {
    bubbles: true,
    cancelable: true,
    clientX: 40,
    clientY: 60,
  })
  target.dispatchEvent(ev)
  return ev
}

afterEach(() => {
  for (const app of mounted.splice(0)) app.unmount()
  document.body.innerHTML = ''
})

describe('主窗口原生右键菜单抑制 —— 只有没人接管时才由应用层兜底', () => {
  it('① 无处理者接管的区域：原生菜单必被 preventDefault（投诉本体）', async () => {
    mountMenu()
    const ev = rightClick(document.body)
    await nextTick()
    expect(ev.defaultPrevented, '未被任何组件接管的右键必须抑制原生浏览器菜单').toBe(true)
    // 且抑制的同时给出应用菜单（不是静默吞掉）。
    expect(menuEl()).toBeTruthy()
  })

  it('② 组件已自行处理（宠物根同款 @contextmenu.prevent）：不叠加应用菜单', async () => {
    mountMenu()
    // 复刻 DesktopPet.vue / PetOverlayView.vue 根节点的处理方式：
    // 自己 preventDefault 并弹自有菜单，document 级监听必须让路。
    const petRoot = document.createElement('div')
    petRoot.addEventListener('contextmenu', (ev: Event) => {
      ev.preventDefault()
      ;(petRoot as HTMLElement).dataset.ownMenu = 'open'
    })
    document.body.appendChild(petRoot)
    const ev = rightClick(petRoot)
    await nextTick()
    expect(ev.defaultPrevented).toBe(true) // 原生菜单仍被抑制（由组件抑制的）
    expect(petRoot.dataset.ownMenu).toBe('open')
    expect(menuEl(), '自有菜单已接管，应用菜单不得叠加').toBeNull()
  })

  it('③ 输入框：既不弹应用菜单，也绝不 preventDefault（原生粘贴菜单必须保留）', async () => {
    mountMenu()
    const input = document.createElement('input')
    document.body.appendChild(input)
    const ev = rightClick(input)
    await nextTick()
    expect(menuEl()).toBeNull()
    expect(ev.defaultPrevented, '输入框的原生菜单（粘贴/拼写）是刻意保留的，不得被抑制').toBe(false)
  })
})

describe('接线守卫 —— 任何一环被删，浏览器菜单就会重新出现', () => {
  const src = (rel: string) =>
    readFileSync(fileURLToPath(new URL(rel, import.meta.url)), 'utf8').replace(/\r\n/g, '\n')

  it('App.vue 必须挂载 AppContextMenu（主窗口唯一的全局抑制点）', () => {
    const app = src('../App.vue').replace(/<!--[\s\S]*?-->/g, '')
    expect(app).toMatch(/<AppContextMenu\s*\/>/)
  })

  it('DesktopPet.vue（页内宠物）根节点必须保留 @contextmenu.prevent 自有菜单', () => {
    const pet = src('./pet/DesktopPet.vue')
    expect(pet).toMatch(/@contextmenu\.prevent="onContextMenu"/)
  })

  it('PetOverlayView.vue（悬浮窗宠物）必须保留根处理 + 窗口级兜底抑制', () => {
    const overlay = src('../views/PetOverlayView.vue')
    expect(overlay).toMatch(/@contextmenu\.prevent="onContextMenu"/)
    expect(overlay).toMatch(/window\.addEventListener\('contextmenu',\s*suppressNativeContextMenu\)/)
  })
})
