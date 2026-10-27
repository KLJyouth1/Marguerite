// 启动动画契约测试（2026-10-22）。
//
// 锁四件事，每一条都对应一个**用户会真实撞到的失败**：
//   1. 背景视频指向随包分发的 `bg-v2.mp4`（旧值 /boot/splash-cyber.mp4 根本不存在，
//      导致启动画面实际只有极光 —— 这正是"改了没生效"的形态）；
//   2. 跳过按钮是**真实 button**、有 aria-label、点击即进入；
//   3. Esc 可跳过（窗口 decorations:false 且外壳被隐藏时，这是唯一键盘出口）；
//   4. 视频加载失败时回退极光，不出现破图。
//
// 挂载方式沿用仓内既有范式（`vue.createApp(...).mount(el)` + querySelector），
// **不引入 @vue/test-utils** —— 项目没装它，为一条测试加依赖不值得。

// @vitest-environment happy-dom

import { describe, expect, it, vi, beforeEach, afterEach } from 'vitest'
import { createApp, nextTick, type App } from 'vue'

const replaceMock = vi.fn()

vi.mock('vue-router', async () => {
  const actual = await vi.importActual<typeof import('vue-router')>('vue-router')
  return {
    ...actual,
    useRouter: () => ({ replace: replaceMock }),
  }
})

// status RPC 在单测里没有网关 —— 让它稳定返回空，避免 useRequest 发起真实请求
vi.mock('@/composables/useRequest', () => ({
  useRequest: () => ({ data: { value: null } }),
}))

import SplashView from '@/views/SplashView.vue'

const mounted: App[] = []

function mountSplash(): HTMLElement {
  const el = document.createElement('div')
  document.body.appendChild(el)
  const app = createApp(SplashView)
  app.mount(el)
  mounted.push(app)
  return el
}

/** 触发 DOM 事件并等 Vue 完成一次渲染。 */
async function fire(el: Element | null, type: string) {
  expect(el).toBeTruthy()
  el!.dispatchEvent(new Event(type))
  await nextTick()
}

beforeEach(() => {
  replaceMock.mockReset()
  vi.useFakeTimers()
  document.body.innerHTML = ''
  delete document.documentElement.dataset.splash
})

afterEach(() => {
  while (mounted.length) {
    try {
      mounted.pop()!.unmount()
    } catch {
      /* 已卸载 */
    }
  }
  vi.useRealTimers()
  document.body.innerHTML = ''
  delete document.documentElement.dataset.splash
})

describe('SplashView — 背景视频', () => {
  it('视频源用 H.264 + VP9 双源（HEVC 在 WebView2 上不可解）', () => {
    const el = mountSplash()
    const video = el.querySelector('video')
    expect(video).toBeTruthy()
    const sources = Array.from(el.querySelectorAll('video source')) as HTMLSourceElement[]
    expect(sources.length).toBeGreaterThanOrEqual(2)
    const srcs = sources.map((s) => s.getAttribute('src') || '')
    // 主选必须是 H.264（WebView2/Chromium 必解）
    expect(srcs[0]).toBe('/splash/bg-v2-h264.mp4')
    expect(sources[0].getAttribute('type')).toContain('avc1')
    // 备选 WebM/VP9
    expect(srcs.join(' ')).toContain('bg-v2.webm')
    // 绝不能再指向不存在的 /boot/*
    expect(srcs.join(' ')).not.toContain('/boot/')
    // 也不应把 HEVC 原件当主源（那正是"视频不播"的根因）
    expect(srcs[0]).not.toContain('bg-v2.mp4')
  })

  it('初始不显示视频（videoOk=false），loadeddata 后显示，error 后回退隐藏', async () => {
    const el = mountSplash()
    const video = el.querySelector('video') as HTMLVideoElement
    expect(video.style.display).toBe('none')

    await fire(video, 'loadeddata')
    expect((el.querySelector('video') as HTMLVideoElement).style.display).not.toBe('none')

    // 回退：error 之后必须重新隐藏，不能留破图
    await fire(el.querySelector('video'), 'error')
    expect((el.querySelector('video') as HTMLVideoElement).style.display).toBe('none')
  })

  it('视频全源失败时缩短自动进入等待（不让人盯着极光干等）', async () => {
    const el = mountSplash()
    await fire(el.querySelector('video'), 'error')
    // 缩短到 2s 触发 enter()，enter 内部还有 600ms 淡出 → 需要 >2600ms
    vi.advanceTimersByTime(2_800)
    expect(replaceMock).toHaveBeenCalledWith('/chat')
  })

  it('视频正常时不提前进入（默认 10s）', async () => {
    const el = mountSplash()
    await fire(el.querySelector('video'), 'loadeddata')
    vi.advanceTimersByTime(3_000)
    expect(replaceMock).not.toHaveBeenCalled()
    vi.advanceTimersByTime(8_000) // 累计 11s > 10s
    expect(replaceMock).toHaveBeenCalledWith('/chat')
  })
})

describe('SplashView — 跳过按钮（AC3）', () => {
  it('是真实 <button>（键盘可达），带明确 aria-label', () => {
    const el = mountSplash()
    const btn = el.querySelector('[data-testid="splash-skip"]')
    expect(btn).toBeTruthy()
    expect(btn!.tagName).toBe('BUTTON')
    expect(btn!.getAttribute('type')).toBe('button')
    expect(btn!.getAttribute('aria-label')).toBeTruthy()
    // 不是靠 tabindex 硬凑的 div
    expect(btn!.getAttribute('tabindex')).toBeNull()
  })

  it('点击后跳转 /chat', async () => {
    const el = mountSplash()
    el.querySelector<HTMLButtonElement>('[data-testid="splash-skip"]')!.click()
    await nextTick()
    vi.advanceTimersByTime(700)
    expect(replaceMock).toHaveBeenCalledWith('/chat')
  })

  it('点击只触发一次跳转（enter 的 entering 闸门 + click.stop）', async () => {
    const el = mountSplash()
    el.querySelector<HTMLButtonElement>('[data-testid="splash-skip"]')!.click()
    await nextTick()
    vi.advanceTimersByTime(700)
    expect(replaceMock).toHaveBeenCalledTimes(1)
  })
})

describe('SplashView — Esc 跳过（AC4）', () => {
  it('Esc 键触发跳转', async () => {
    mountSplash()
    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape' }))
    await nextTick()
    vi.advanceTimersByTime(700)
    expect(replaceMock).toHaveBeenCalledWith('/chat')
  })

  it('其它键不触发', async () => {
    mountSplash()
    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'a' }))
    await nextTick()
    vi.advanceTimersByTime(700)
    expect(replaceMock).not.toHaveBeenCalled()
  })

  it('卸载后不残留全局 keydown 监听', async () => {
    mountSplash()
    // beforeEach 保证 mounted 里只有本次的一个实例
    while (mounted.length) mounted.pop()!.unmount()
    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape' }))
    await nextTick()
    vi.advanceTimersByTime(700)
    expect(replaceMock).not.toHaveBeenCalled()
  })
})

describe('SplashView — 覆盖整个壳（G1）', () => {
  it('挂载时给 html 打 data-splash，卸载时清除（CSS 靠它隐藏 sidebar/topbar）', () => {
    mountSplash()
    expect(document.documentElement.dataset.splash).toBe('1')
    while (mounted.length) mounted.pop()!.unmount()
    expect(document.documentElement.dataset.splash).toBeUndefined()
  })
})
