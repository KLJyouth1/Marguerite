// @vitest-environment happy-dom
// 守卫：任务页此前把当前模式渲染成**枚举名**（真机实测「模式 bloom」），而顶栏同一个
// 模式叫「深度打磨」—— 同一产品里两个名字。D-03 约定模式名一律走 i18n 别名，
// 枚举值只留在 tooltip 里供高级用户核对。这里钉住这条契约。
import { createApp, nextTick, type App } from 'vue'
import { afterEach, beforeAll, describe, expect, it } from 'vitest'

import i18n, { loadLocaleMessages } from '@/i18n'
import ModeImpactCard from '@/components/task/ModeImpactCard.vue'

let app: App<Element> | null = null

afterEach(() => {
  app?.unmount()
  app = null
  document.body.innerHTML = ''
})

beforeAll(async () => {
  await loadLocaleMessages('zh-Hans')
})

function mount(props: { modeKind: string; modeRoute: string | null; strategy: string | null }) {
  const el = document.createElement('div')
  document.body.appendChild(el)
  app = createApp(ModeImpactCard, props)
  app.use(i18n)
  app.mount(el)
  return el
}

function values(el: Element): string[] {
  return Array.from(el.querySelectorAll('.mode-impact__val')).map((n) => n.textContent?.trim() ?? '')
}

describe('ModeImpactCard', () => {
  it('renders the mode through its zh-Hans alias and keeps the enum only in the tooltip', async () => {
    i18n.global.locale.value = 'zh-Hans'
    const el = mount({ modeKind: 'bloom', modeRoute: 'bloom_ensemble', strategy: 'two-stage' })
    await nextTick()

    // 把 modeKindLabel 改回 `normalizeModeKind(props.modeKind)` 时，这一条立刻报 'bloom'。
    expect(values(el)[0]).toBe('深度打磨')
    expect(el.querySelector('.mode-impact__val')?.getAttribute('title')).toBe('bloom')
    // 路由 / 策略是引擎侧的真实值（不是枚举名），照原样展示，缺值用破折号。
    expect(values(el).slice(1)).toEqual(['bloom_ensemble', 'two-stage'])
  })

  it('shows an em dash for a missing mode instead of inventing one', async () => {
    i18n.global.locale.value = 'zh-Hans'
    const el = mount({ modeKind: '', modeRoute: null, strategy: null })
    await nextTick()
    expect(values(el)).toEqual(['—', '—', '—'])
  })
})
