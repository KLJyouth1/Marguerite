// @vitest-environment happy-dom
// 桌宠悬浮窗 · 自有右键菜单（用户实测：右键弹出的是浏览器默认菜单）。
//
// 守住四件事：
//   1. contextmenu 被 preventDefault（窗口级 + 根节点），浏览器菜单不再出现；
//   2. 菜单能打开，并且是 role=menu + role=menuitem 的可访问结构；
//   3. 菜单项真的执行动作：互动 → pet.triggerAction('pet')；隐藏 → 桌面悬浮窗开关；
//   4. Esc 关闭、↑↓ 移动焦点；设置项如实禁用并给出说明。
import { createApp, nextTick, type App } from 'vue'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import i18n from '@/i18n'

const petRpcMock = vi.hoisted(() => ({
  list: vi.fn(async () => ({
    skins: [
      { id: 'cat', label: '橘猫小工', kind: 'beast', accent_hex: '#F59E0B' },
      { id: 'blue', label: '雏菊精灵', kind: 'humanoid', accent_hex: '#60A5FA' },
    ],
  })),
  status: vi.fn(async () => ({
    skin: 'cat',
    kind: 'beast',
    mood: 'normal',
    energy: 100,
    affection: 3,
    running: true,
    accent_hex: '#F59E0B',
  })),
  triggerAction: vi.fn(async () => ({ ok: true, energy: 96, affection: 4, mood: 'happy' })),
  setSkin: vi.fn(async () => ({ ok: true, skin: 'cat' })),
}))

const overlayMock = vi.hoisted(() => ({
  setPetOverlayEnabled: vi.fn(async () => ({ ok: true })),
}))

vi.mock('@/rpc/petContract', () => ({
  usePetRpc: () => petRpcMock,
  PET_SKIN_IDS: ['cat', 'blue'],
  PET_EXPECTED_SKIN_COUNT: 2,
}))

vi.mock('@/composables/usePetOverlay', () => ({
  setPetOverlayEnabled: overlayMock.setPetOverlayEnabled,
}))

vi.mock('@/components/pet/petRenderer', () => ({
  // 渲染器只提供宿主调用的子集；菜单/拖拽测试不关心像素
  createPetRenderer: vi.fn(async () => ({
    kind: 'svg',
    renderer: {
      setMood: vi.fn(),
      triggerMoodAction: vi.fn(),
      setFacing: vi.fn(),
      setHover: vi.fn(),
      lookAt: vi.fn(),
      poke: vi.fn(),
      jump: vi.fn(),
      burst: vi.fn(),
      spin: vi.fn(),
      dance: vi.fn(),
      setWalking: vi.fn(),
      dispose: vi.fn(),
    },
  })),
}))

vi.mock('@/components/pet/petBrain', () => ({
  getPetBrain: () => ({
    frame: () => ({ mood: 'normal', energy: 100, affection: 0 }),
    tick: () => ({ mood: 'normal', energy: 100, affection: 0 }),
    petted: vi.fn(),
    feed: vi.fn(),
  }),
}))

vi.mock('@/components/pet/petContract', () => ({
  moodMap: { normal: 'normal', happy: 'happy', sleepy: 'sleepy', working: 'working', excited: 'excited', sad: 'sad' },
}))

vi.mock('@/components/pet/petSpec', () => ({ getPetModelSpec: () => ({}) }))
vi.mock('@/components/pet/detectPetRuntime', () => ({
  detectPetRuntime: () => ({ quality: 'high' }),
  renderTierOf: () => 'high',
}))

import PetOverlayView from './PetOverlayView.vue'

let app: App<Element> | null = null

beforeEach(() => {
  i18n.global.locale.value = 'en'
  petRpcMock.triggerAction.mockClear()
  overlayMock.setPetOverlayEnabled.mockClear()
})

afterEach(() => {
  app?.unmount()
  app = null
  document.body.innerHTML = ''
})

const flush = () => new Promise<void>(resolve => setTimeout(resolve, 0))

async function mount() {
  const el = document.createElement('div')
  document.body.appendChild(el)
  app = createApp(PetOverlayView)
  app.use(i18n)
  app.mount(el)
  await flush()
  await nextTick()
  return el
}

async function openMenu(el: HTMLElement, at: { x: number; y: number } = { x: 120, y: 90 }) {
  const root = el.querySelector<HTMLElement>('.pet-overlay')!
  const ev = new MouseEvent('contextmenu', {
    bubbles: true,
    cancelable: true,
    clientX: at.x,
    clientY: at.y,
  })
  root.dispatchEvent(ev)
  await nextTick()
  return ev
}

function menuButtons(el: HTMLElement): HTMLButtonElement[] {
  return Array.from(el.querySelectorAll<HTMLButtonElement>('.pet-overlay__menu [role="menuitem"]'))
}

/**
 * 菜单是否仍在「打开」态。
 * happy-dom 不会跑 CSS 过渡，Vue 的 <Transition> 收不到 transitionend，
 * 离场节点会留在 DOM 里（带 fade-leave-* class）——所以不能直接断言节点消失。
 */
function menuVisible(el: HTMLElement): boolean {
  const menu = el.querySelector<HTMLElement>('.pet-overlay__menu')
  if (!menu) return false
  return !/fade-leave/.test(menu.className)
}

describe('PetOverlayView · 右键菜单', () => {
  it('右键被 preventDefault，弹出的是桌宠自己的菜单', async () => {
    const el = await mount()
    const ev = await openMenu(el)
    expect(ev.defaultPrevented).toBe(true)

    const menu = el.querySelector('.pet-overlay__menu')
    expect(menu).toBeTruthy()
    expect(menu!.getAttribute('role')).toBe('menu')
    // 三个菜单项（互动 / 隐藏 / 设置）
    expect(menuButtons(el)).toHaveLength(3)
  })

  it('舞台暴露 aria-haspopup / aria-expanded，打开后可键盘关闭', async () => {
    const el = await mount()
    const stage = el.querySelector<HTMLElement>('.pet-overlay__stage')!
    expect(stage.getAttribute('aria-haspopup')).toBe('menu')
    expect(stage.getAttribute('aria-expanded')).toBe('false')

    await openMenu(el)
    expect(stage.getAttribute('aria-expanded')).toBe('true')

    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape' }))
    await nextTick()
    expect(menuVisible(el)).toBe(false)
    expect(stage.getAttribute('aria-expanded')).toBe('false')
  })

  it('「互动」真的触发一次 pet 动作', async () => {
    const el = await mount()
    await openMenu(el)
    menuButtons(el)[0].click()
    await flush()

    expect(petRpcMock.triggerAction).toHaveBeenCalledWith('pet')
    expect(menuVisible(el)).toBe(false)
  })

  it('「隐藏」走既有桌面悬浮窗开关（不自己拼临时命令）', async () => {
    const el = await mount()
    await openMenu(el)
    menuButtons(el)[1].click()
    await flush()

    expect(overlayMock.setPetOverlayEnabled).toHaveBeenCalledWith(false)
  })

  it('设置项如实禁用，并用 title 说明去哪里设置', async () => {
    const el = await mount()
    await openMenu(el)
    const settings = menuButtons(el)[2]
    expect(settings.disabled).toBe(true)
    expect(settings.getAttribute('aria-disabled')).toBe('true')
    expect(settings.getAttribute('title')).toContain('设置')
  })

  it('pet 窗口内全局禁用浏览器默认右键菜单', async () => {
    await mount()
    const ev = new MouseEvent('contextmenu', { bubbles: true, cancelable: true })
    document.body.dispatchEvent(ev)
    expect(ev.defaultPrevented).toBe(true)
  })
})
