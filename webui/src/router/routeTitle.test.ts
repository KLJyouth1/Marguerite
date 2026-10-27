// @vitest-environment happy-dom
import { beforeEach, describe, expect, it } from 'vitest'
import type { RouteLocationNormalized } from 'vue-router'
import i18n, { isI18nReady, loadLocaleMessages, whenI18nReady } from '@/i18n'
import { applyRouteTitle, routeTitle } from './index'

// 首屏标题缺陷（2026-10-24）：冷启动时路由导航先于语言包就绪，afterEach 用英文
// meta.title 写了标题，随后语言包加载完再改成中文，用户看到「英文 → 中文」跳变。
// applyRouteTitle 就是 afterEach 调用的同一个写标题入口，这里直接验证它的门控。

const petRoute = { name: 'pet', meta: { title: 'Pet' } } as unknown as RouteLocationNormalized

beforeEach(() => {
  document.title = 'Marguerite'
  // 每个用例都把语言恢复成 en；未就绪判定是单向的，用例顺序按 describe 内声明顺序。
  i18n.global.locale.value = 'en'
})

describe('首屏页面标题', () => {
  it('i18n 未就绪时 afterEach 不写标题（哨兵值保持不变）', () => {
    document.title = 'SENTINEL — Marguerite'

    // 默认参数走 isI18nReady()（首屏初始状态：未就绪）
    expect(applyRouteTitle(petRoute)).toBe(false)
    expect(applyRouteTitle(petRoute, false)).toBe(false)
    // 未就绪时既不写中文，也不写英文占位
    expect(document.title).toBe('SENTINEL — Marguerite')
  })

  it('语言包就绪且切到目标语言后，写入中文标题并兑现一次性 ready 信号', async () => {
    const ready = whenI18nReady()
    expect(isI18nReady()).toBe(false)

    // 语言包已注册但 locale 还没切过去 —— 仍不算就绪
    await loadLocaleMessages('zh-Hans')
    expect(isI18nReady()).toBe(false)

    i18n.global.locale.value = 'zh-Hans'
    expect(isI18nReady()).toBe(true)
    await ready

    // 默认参数（afterEach 的实际调用方式）此时才允许写标题
    expect(applyRouteTitle(petRoute)).toBe(true)
    expect(document.title).toBe('鲸鱼娘 — Marguerite')
  })

  it('routeTitle 在缺少 key 时兜底行为不变', () => {
    // 既没有 titleKey 也没有 nav.<name> → meta.title
    expect(routeTitle({
      name: 'not-a-navigation-route',
      meta: { title: 'Mystery Page' },
    } as unknown as RouteLocationNormalized)).toBe('Mystery Page')

    // titleKey 命中不到、name 也为空 → 继续回退 meta.title
    expect(routeTitle({
      name: '',
      meta: { titleKey: 'nav.doesNotExist', title: 'Not Found' },
    } as unknown as RouteLocationNormalized)).toBe('Not Found')

    // 全都没有 → 品牌名兜底
    expect(routeTitle({ name: '', meta: {} } as unknown as RouteLocationNormalized)).toBe('Marguerite')
  })
})
