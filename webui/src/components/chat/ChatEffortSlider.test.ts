// @vitest-environment happy-dom
// 推理等级拖动条：连续拖动 → 松手吸附；键盘逐档；锁定态不可改。
//
// 这组用例锁的是**真实交互契约**，不是渲染快照：
//   1. 拖动过程中填充条跟手（连续值），但**不提前 emit**（避免每像素都写盘）；
//   2. 松手才吸附到最近档位并 emit 一次；
//   3. 键盘 ←/→/Home/End 与点刻度都能改档；
//   4. disabled 时任何路径都不 emit（会话锁定不产生「点了不生效」的假反馈）。
import { createApp, nextTick, type App } from 'vue'
import { afterEach, beforeAll, describe, expect, it } from 'vitest'

import i18n, { loadLocaleMessages } from '@/i18n'
import ChatEffortSlider from '@/components/chat/ChatEffortSlider.vue'
import type { EffortLevel } from '@/types/effort'

let app: App<Element> | null = null

afterEach(() => {
  app?.unmount()
  app = null
  document.body.innerHTML = ''
})

beforeAll(async () => {
  await loadLocaleMessages('zh-Hans')
  i18n.global.locale.value = 'zh-Hans'
})

interface MountOpts {
  level?: EffortLevel
  disabled?: boolean
}

async function mount(opts: MountOpts = {}) {
  const el = document.createElement('div')
  document.body.appendChild(el)
  const Comp = {
    components: { ChatEffortSlider },
    setup() {
      return { level: opts.level ?? 'off', disabled: opts.disabled ?? false }
    },
    template: `<ChatEffortSlider :model-value="level" :disabled="disabled" @update:model-value="onPick" />`,
    data() {
      return { events: [] as EffortLevel[] }
    },
    methods: {
      onPick(level: EffortLevel) {
        ;(this as unknown as { events: EffortLevel[] }).events.push(level)
      },
    },
  }
  app = createApp(Comp)
  app.use(i18n)
  const vm = app.mount(el) as unknown as { events: EffortLevel[] }
  await nextTick()
  return { el, vm }
}

function trigger(el: Element): HTMLButtonElement {
  return el.querySelector('.effort-picker__trigger') as HTMLButtonElement
}

async function openPanel(el: Element) {
  trigger(el).click()
  await nextTick()
  return el.querySelector('.effort-track') as HTMLElement
}

/** happy-dom 不做布局：给滑轨钉死一个 300px 宽、left=0 的矩形，坐标才可预期。 */
function stubTrackRect(track: HTMLElement, width = 300) {
  track.getBoundingClientRect = () =>
    ({ left: 0, top: 0, width, height: 26, right: width, bottom: 26, x: 0, y: 0, toJSON: () => ({}) }) as DOMRect
}

/** happy-dom 未必实现 PointerEvent；处理器只读 clientX/pointerId，用 MouseEvent 冒充即可。 */
function pointer(type: string, clientX: number): Event {
  const ev = new MouseEvent(type, { clientX, bubbles: true })
  return ev
}

describe('ChatEffortSlider', () => {
  it('默认收起，触发器显示当前档位文案', async () => {
    const { el } = await mount({ level: 'deep' })
    expect(el.querySelector('.effort-pop')).toBeNull()
    expect(trigger(el).textContent).toContain('深度')
  })

  it('点开面板后刻度标签反映四档', async () => {
    const { el } = await mount()
    await openPanel(el)
    const labels = [...el.querySelectorAll('.effort-scale__item')].map((n) => n.textContent?.trim())
    expect(labels).toEqual(['关闭', '常规', '深度', '极限'])
  })

  it('拖动过程中不 emit，松手才吸附到最近档位', async () => {
    const { el, vm } = await mount({ level: 'off' })
    const track = await openPanel(el)
    stubTrackRect(track)

    track.dispatchEvent(pointer('pointerdown', 10))
    await nextTick()
    track.dispatchEvent(pointer('pointermove', 155)) // 300px 轨道的 51.6% → 1.55 档
    await nextTick()
    expect(vm.events).toEqual([])

    track.dispatchEvent(pointer('pointerup', 155))
    await nextTick()
    // 1.55 吸附到 2（round），即 deep
    expect(vm.events).toEqual(['deep'])
  })

  it('拖到最右端吸附到极限档', async () => {
    const { el, vm } = await mount({ level: 'off' })
    const track = await openPanel(el)
    stubTrackRect(track)

    track.dispatchEvent(pointer('pointerdown', 300))
    track.dispatchEvent(pointer('pointerup', 300))
    await nextTick()
    expect(vm.events).toEqual(['max'])
  })

  it('拖到最左端回到关闭档', async () => {
    const { el, vm } = await mount({ level: 'max' })
    const track = await openPanel(el)
    stubTrackRect(track)

    track.dispatchEvent(pointer('pointerdown', 0))
    track.dispatchEvent(pointer('pointerup', 0))
    await nextTick()
    expect(vm.events).toEqual(['off'])
  })

  it('点击刻度标签直接切档', async () => {
    const { el, vm } = await mount({ level: 'off' })
    await openPanel(el)
    const items = [...el.querySelectorAll('.effort-scale__item')] as HTMLButtonElement[]
    items[1].click()
    await nextTick()
    expect(vm.events).toEqual(['normal'])
  })

  it('键盘 ←/→/Home/End 逐档移动', async () => {
    const { el, vm } = await mount({ level: 'normal' })
    await openPanel(el)
    const thumb = el.querySelector('.effort-track__thumb') as HTMLButtonElement

    thumb.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowRight', bubbles: true }))
    await nextTick()
    thumb.dispatchEvent(new KeyboardEvent('keydown', { key: 'End', bubbles: true }))
    await nextTick()
    thumb.dispatchEvent(new KeyboardEvent('keydown', { key: 'Home', bubbles: true }))
    await nextTick()

    // normal(1) → deep(2) → max(3) → off(0)
    expect(vm.events).toEqual(['deep', 'max', 'off'])
  })

  it('已是最高档时按 → 不产生重复事件', async () => {
    const { el, vm } = await mount({ level: 'max' })
    await openPanel(el)
    const thumb = el.querySelector('.effort-track__thumb') as HTMLButtonElement
    thumb.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowRight', bubbles: true }))
    await nextTick()
    expect(vm.events).toEqual([])
  })

  it('disabled 时拖动/点击/键盘都不 emit', async () => {
    const { el, vm } = await mount({ level: 'off', disabled: true })
    const track = await openPanel(el)
    stubTrackRect(track)

    track.dispatchEvent(pointer('pointerdown', 300))
    track.dispatchEvent(pointer('pointerup', 300))
    await nextTick()

    const items = [...el.querySelectorAll('.effort-scale__item')] as HTMLButtonElement[]
    items[3].click()
    await nextTick()

    const thumb = el.querySelector('.effort-track__thumb') as HTMLButtonElement
    thumb.dispatchEvent(new KeyboardEvent('keydown', { key: 'End', bubbles: true }))
    await nextTick()

    expect(vm.events).toEqual([])
  })

  it('ARIA：滑轨暴露档位值与档位文案', async () => {
    const { el } = await mount({ level: 'deep' })
    await openPanel(el)
    const thumb = el.querySelector('.effort-track__thumb') as HTMLElement
    expect(thumb.getAttribute('role')).toBe('slider')
    expect(thumb.getAttribute('aria-valuemin')).toBe('0')
    expect(thumb.getAttribute('aria-valuemax')).toBe('3')
    expect(thumb.getAttribute('aria-valuenow')).toBe('2')
    expect(thumb.getAttribute('aria-valuetext')).toBe('深度')
  })
})
