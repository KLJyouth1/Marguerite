// @vitest-environment happy-dom
//
// 回归锚点（真机缺陷 2026-10-23）：桌面专属路由（/pet-overlay）曾按
// `hasDesktopOnboarding` 注册 —— 那是「桌面桥是否实现 onboarding 三命令」的能力位。
// 桥是部分实现时该能力位为 false，悬浮窗窗口加载 /control/pet-overlay 就落到
// NotFound（360×420 透明窗口里显示「未找到」+ 整条侧栏）。
// 现在改为按 `isDesktop` 注册：只要是桌面壳，悬浮窗路由就必须存在。

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const OVERLAY = '/pet-overlay'

beforeEach(() => {
  vi.resetModules()
})

afterEach(() => {
  delete (window as unknown as { MargueriteDesktop?: unknown }).MargueriteDesktop
})

describe('desktop routes registration', () => {
  it('desktop shell with a PARTIAL bridge still registers /pet-overlay', async () => {
    // 部分实现：没有任何 onboarding 命令 → hasDesktopOnboarding === false
    ;(window as unknown as { MargueriteDesktop?: unknown }).MargueriteDesktop = {}
    const { routes } = await import('./index')
    const overlay = routes.find((r) => r.path === OVERLAY)
    expect(overlay).toBeTruthy()
    expect(overlay?.meta?.platforms).toContain('desktop')
  })

  it('web (no desktop bridge) must NOT register /pet-overlay', async () => {
    delete (window as unknown as { MargueriteDesktop?: unknown }).MargueriteDesktop
    const { routes } = await import('./index')
    expect(routes.some((r) => r.path === OVERLAY)).toBe(false)
  })
})
