// activeMode Pinia store — 「跨 chunk 共享」不变量守卫。
//
// 背景：`useActiveMode` 原先用**模块级** `export const activeMode = ref(null)` 做单例。
// `webui/` 是路由懒加载 + 多 chunk 打包，该模块被复制进多个 chunk（`webui/dist` 实测：
// `session.event.mode.changed` 的 hydrate+订阅只出现在 `index-*.js`，
// 而 `MODE_KINDS` 的 `omni_transcend` 同时出现在 `index-*.js` 与 `ChatView-*.js`）。
// 于是 `App.vue`（主 chunk）的 `initActiveMode()` 只 hydrate 主 chunk 那份 ref，
// 而 `ModeHintBar`（懒加载 ChatView chunk）读到自己那份、永远为 null，
// 界面稳定显示 "Initializing · The gateway is loading its runtime configuration..."，
// 而顶栏（读已 hydrate 的那份）显示正确状态 —— 同一页面两个组件互相矛盾。
//
// 修法：状态搬进 Pinia store（挂在 app 级实例上，跨 chunk 天然共享）。
//
// ⚠️ 测试写法注意：本模块导出的 `activeMode` 是**模块级 computed**，它在首次求值时
// 绑定当时的 pinia/store 实例。因此这里 **只创建一次 pinia**，用例之间用
// 「重置 store 状态」而不是「换 pinia」来隔离 —— 换 pinia 会让 computed 仍指向旧 store，
// 表现为读到的值永远是上一个用例留下的（本文件最初就踩了这个坑）。
import { beforeEach, describe, expect, it } from 'vitest'
import { createPinia, setActivePinia } from 'pinia'
import { activeMode, useActiveMode } from './useActiveMode'
import { useActiveModeStore } from '@/stores/activeMode'

const pinia = createPinia()
setActivePinia(pinia)

describe('activeMode cross-chunk sharing', () => {
  beforeEach(() => {
    useActiveModeStore().mode = null
  })

  it('exposes the same value to every consumer (store is the single source)', () => {
    // 「两个不同消费方」= 两次独立取用，模拟两个 chunk 各自的组件。
    const a = useActiveMode().activeMode
    const b = useActiveMode().activeMode

    useActiveModeStore().mode = { kind: 'bloom', mode: 'bloom', label: 'Bloom' }

    // 回归到「模块级 ref」时，两个消费方会各自持有一份 ref，此断言失败。
    expect(a.value?.kind).toBe('bloom')
    expect(b.value?.kind).toBe('bloom')
    expect(a.value).toBe(b.value)
  })

  it('reflects store mutations made from a different import site', () => {
    // 模拟「另一个 chunk 里的组件改了模式」：直接写 store，本处必须看见。
    useActiveModeStore().mode = { kind: 'plan', mode: 'plan' }
    expect(activeMode.value?.kind).toBe('plan')

    useActiveModeStore().mode = { kind: 'omni_transcend', mode: 'omni_transcend' }
    expect(activeMode.value?.kind).toBe('omni_transcend')
  })
})
