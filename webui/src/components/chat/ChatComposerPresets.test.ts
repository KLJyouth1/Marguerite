// @vitest-environment happy-dom
// 输入框模式胶囊（WorkBuddy / Codex 风格）：
//   - 默认**收起**成一枚小胶囊，不再常驻铺 3×3 网格；
//   - 点开才在上方弹出面板：3×3 网格 + 自动模型破甲开关 + 高级抽屉入口；
//   - 点选格子下发 `{ userMode }` 键并自动收起；锁定态仍可查看但不可切换。
import { createApp, nextTick, ref, type App } from 'vue'
import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest'

const switchUserMode = vi.fn(async () => null)
const switching = ref(false)
const switchError = ref<string | null>(null)
const activeUserMode = ref<{
  key: string
  intensity: string
  working: string
  label: string
} | null>({
  key: 'standard_doit',
  intensity: 'standard',
  working: 'doit',
  label: '标准·执行',
})

vi.mock('@/composables/useActiveMode', () => ({
  useActiveMode: () => ({
    activeUserMode,
    switching,
    switchError,
    switchUserMode,
  }),
}))

import i18n, { loadLocaleMessages } from '@/i18n'
import ChatComposerPresets from '@/components/chat/ChatComposerPresets.vue'

let app: App<Element> | null = null

afterEach(() => {
  app?.unmount()
  app = null
  document.body.innerHTML = ''
  switchUserMode.mockClear()
  switching.value = false
  switchError.value = null
  activeUserMode.value = {
    key: 'standard_doit',
    intensity: 'standard',
    working: 'doit',
    label: '标准·执行',
  }
})

beforeAll(async () => {
  await loadLocaleMessages('zh-Hans')
  i18n.global.locale.value = 'zh-Hans'
})

interface MountOpts {
  locked?: boolean
  routing?: string
  busy?: boolean
}

async function mount(opts: MountOpts = {}) {
  const el = document.createElement('div')
  document.body.appendChild(el)
  const Comp = {
    components: { ChatComposerPresets },
    setup() {
      return {
        locked: opts.locked ?? false,
        // ref 才能被 onRoute 回写后驱动 props 变化（setup 返回的裸字符串不会）。
        routing: ref(opts.routing ?? 'omni_agent'),
        busy: opts.busy ?? false,
      }
    },
    template: `<ChatComposerPresets
      :run-mode-locked="locked"
      :model-routing-mode="routing"
      :model-routing-busy="busy"
      @open-advanced="onAdv"
      @set-model-routing-mode="onRoute"
    />`,
    data() {
      return { events: [] as unknown[] }
    },
    methods: {
      onAdv() {
        ;(this as unknown as { events: unknown[] }).events.push(['adv'])
      },
      onRoute(mode: string) {
        const self = this as unknown as { events: unknown[]; routing: string }
        self.events.push(['route', mode])
        // 回写 props：真实 ChatView 收到 RPC 回执后会更新 modelRoutingMode，
        // 破甲开关「关→开」的档位记忆必须在这种真实回写下才可验证。
        self.routing = mode
      },
    },
  }
  app = createApp(Comp)
  app.use(i18n)
  const vm = app.mount(el) as unknown as { events: unknown[] }
  await nextTick()
  return { el, vm }
}

function trigger(el: Element): HTMLButtonElement {
  return el.querySelector('.composer-mode-picker__trigger') as HTMLButtonElement
}
function cells(el: Element): HTMLButtonElement[] {
  return [...el.querySelectorAll('.mode-matrix__cell')] as HTMLButtonElement[]
}
async function openPanel(el: Element) {
  trigger(el).click()
  await nextTick()
}
/** `<Transition>` 的离场在 happy-dom 里要等计时器超时才真正摘除节点 → 轮询等待。 */
async function waitGone(el: Element, selector: string, ms = 600) {
  const deadline = Date.now() + ms
  while (Date.now() < deadline) {
    if (!el.querySelector(selector)) return
    await new Promise((resolve) => setTimeout(resolve, 20))
  }
}

describe('ChatComposerPresets (collapsed capsule)', () => {
  it('renders collapsed by default — no 3×3 grid until opened', async () => {
    const { el } = await mount()
    expect(trigger(el)).not.toBeNull()
    expect(cells(el).length).toBe(0)
    expect(el.querySelector('.composer-mode-pop')).toBeNull()
  })

  it('shows the active mode label on the capsule', async () => {
    const { el } = await mount()
    expect(trigger(el).textContent).toContain('标准·执行')
  })

  it('hides the header current-label when it would merely repeat the title', async () => {
    // 未加载到模式时 currentLabel 退化成标题本身（「模式 / Mode」）→ 标题行右侧重复。
    activeUserMode.value = null
    const { el } = await mount()
    expect(trigger(el).textContent).toContain('模式')
    await openPanel(el)
    expect(el.querySelector('.composer-mode-pop__current')).toBeNull()
  })

  it('opens the 3×3 grid in a popover when the capsule is clicked', async () => {
    const { el } = await mount()
    await openPanel(el)
    expect(el.querySelector('.composer-mode-pop')).not.toBeNull()
    expect(cells(el).length).toBe(9)
  })

  it('marks the active cell from the store', async () => {
    const { el } = await mount()
    await openPanel(el)
    const active = el.querySelector('.mode-matrix__cell.is-active')
    expect(active).not.toBeNull()
    expect(active?.textContent).toContain('标准')
  })

  it('switches the user mode and closes the popover when a cell is picked', async () => {
    const { el } = await mount()
    await openPanel(el)
    const extremeDoit = cells(el).find(
      (c) => c.textContent?.includes('极致') && c.textContent?.includes('执行'),
    )
    expect(extremeDoit).toBeTruthy()
    extremeDoit!.click()
    await waitGone(el, '.composer-mode-pop')
    expect(switchUserMode).toHaveBeenCalledWith('extreme_doit')
    expect(trigger(el).getAttribute('aria-expanded')).toBe('false')
    expect(el.querySelector('.composer-mode-pop')).toBeNull()
  })

  it('stays inspectable but non-switchable while the session is locked', async () => {
    const { el } = await mount({ locked: true })
    expect(trigger(el).getAttribute('aria-disabled')).toBe('true')
    await openPanel(el)
    const all = cells(el)
    expect(all.length).toBe(9)
    expect(all.every((c) => c.disabled)).toBe(true)
  })

  it('disables the grid while a mode switch is already in flight', async () => {
    const { el } = await mount()
    switching.value = true
    await openPanel(el)
    expect(cells(el).every((c) => c.disabled)).toBe(true)
  })
})

describe('ChatComposerPresets (auto model unleash)', () => {
  it('reads "on" from any non-off routing mode and toggles it off', async () => {
    const { el, vm } = await mount({ routing: 'omni_agent' })
    await openPanel(el)
    const btn = el.querySelector('.composer-mode-pop__auto') as HTMLButtonElement
    expect(btn.getAttribute('aria-pressed')).toBe('true')
    expect(btn.textContent).toContain('破甲')
    btn.click()
    await nextTick()
    expect(vm.events).toContainEqual(['route', 'off'])
  })

  it('reads "off" from routing=off and re-enables the strongest routing', async () => {
    const { el, vm } = await mount({ routing: 'off' })
    await openPanel(el)
    const btn = el.querySelector('.composer-mode-pop__auto') as HTMLButtonElement
    expect(btn.getAttribute('aria-pressed')).toBe('false')
    expect(btn.textContent).toContain('关闭')
    btn.click()
    await nextTick()
    expect(vm.events).toContainEqual(['route', 'omni_agent'])
  })

  it('re-enables the routing mode the user actually had, not omni_agent', async () => {
    // 回归：关一次破甲再打开，不得把轻量档静默升级成最重的 omni_agent。
    const { el, vm } = await mount({ routing: 'squilla_router' })
    await openPanel(el)
    const btn = el.querySelector('.composer-mode-pop__auto') as HTMLButtonElement
    expect(btn.getAttribute('aria-pressed')).toBe('true')

    btn.click()
    await nextTick()
    expect(btn.getAttribute('aria-pressed')).toBe('false')

    btn.click()
    await nextTick()
    expect(vm.events).toEqual([
      ['route', 'off'],
      ['route', 'squilla_router'],
    ])
  })

  it('ignores clicks while the routing settings are being written', async () => {    const { el, vm } = await mount({ routing: 'omni_agent', busy: true })
    await openPanel(el)
    const btn = el.querySelector('.composer-mode-pop__auto') as HTMLButtonElement
    expect(btn.disabled).toBe(true)
    btn.click()
    await nextTick()
    expect(vm.events).not.toContainEqual(['route', 'off'])
  })
})

describe('ChatComposerPresets (advanced drawer)', () => {
  it('opens the advanced drawer and collapses the popover', async () => {
    const { el, vm } = await mount()
    await openPanel(el)
    ;(el.querySelector('.composer-mode-bar__advanced') as HTMLButtonElement).click()
    await waitGone(el, '.composer-mode-pop')
    expect(vm.events).toContainEqual(['adv'])
    expect(el.querySelector('.composer-mode-pop')).toBeNull()
  })

  it('closes the popover on Escape', async () => {
    const { el } = await mount()
    await openPanel(el)
    document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape' }))
    await waitGone(el, '.composer-mode-pop')
    expect(trigger(el).getAttribute('aria-expanded')).toBe('false')
    expect(el.querySelector('.composer-mode-pop')).toBeNull()
  })
})
