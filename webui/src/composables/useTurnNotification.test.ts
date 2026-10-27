// @vitest-environment happy-dom

// 回合结束桌面通知契约测试（2026-10-21）。
//
// 锁四条判据 —— 它们都是"发错了比不发更糟"的情形：
//   1. 浏览器形态（无 Tauri）→ 绝不调用任何 command；
//   2. 用户关闭开关 → 不发；
//   3. 用户就在看（聚焦 / 页面可见）→ 不发（否则等于噪音）；
//   4. 用户主动中止 → 不发（他就在旁边）。
// 另锁：正文截断、空正文回退、发送失败不抛。

import { describe, expect, it, vi, beforeEach, afterEach } from 'vitest'
import {
  buildTurnNotificationBody,
  loadTurnNotifyEnabled,
  notifyTurnEnded,
  saveTurnNotifyEnabled,
  shouldNotifyTurnEnded,
} from '@/composables/useTurnNotification'
import * as bridge from '@/utils/tauriBridge'

describe('shouldNotifyTurnEnded —— 判据矩阵', () => {
  const base = {
    enabled: true,
    isDesktop: true,
    hasFocus: false,
    visibility: 'hidden',
    reason: undefined as string | undefined,
  }

  it('桌面壳 + 未聚焦 + 已开启 + 非中止 → 发', () => {
    expect(shouldNotifyTurnEnded(base)).toEqual({ ok: true, why: 'notify' })
  })

  it('浏览器形态一律不发（不发起必然失败的调用）', () => {
    expect(shouldNotifyTurnEnded({ ...base, isDesktop: false })).toEqual({
      ok: false,
      why: 'not-desktop',
    })
  })

  it('用户关闭开关后不发', () => {
    expect(shouldNotifyTurnEnded({ ...base, enabled: false })).toEqual({
      ok: false,
      why: 'disabled',
    })
  })

  it('用户主动中止不发（他就坐在旁边）', () => {
    expect(shouldNotifyTurnEnded({ ...base, reason: 'aborted' })).toEqual({
      ok: false,
      why: 'aborted',
    })
  })

  it('窗口有焦点时不发（正看着屏幕，弹通知是噪音）', () => {
    expect(shouldNotifyTurnEnded({ ...base, hasFocus: true, visibility: 'visible' })).toEqual({
      ok: false,
      why: 'user-present',
    })
  })

  it('失焦但未最小化要发（最常见场景：切去回微信，窗口还开着）', () => {
    // 这是修过的一个真 bug：旧判据 `hasFocus || visible` 会在这里判成"在看"，
    // 导致功能只在最小化时生效，几乎等于没用。
    expect(shouldNotifyTurnEnded({ ...base, hasFocus: false, visibility: 'visible' })).toEqual({
      ok: true,
      why: 'notify',
    })
  })

  it('最小化（visibility=hidden）时即使 hasFocus 残留 true 也要发', () => {
    // 某些 WebView 在窗口最小化后 hasFocus() 仍返回 true，只看 hasFocus 会漏发
    expect(shouldNotifyTurnEnded({ ...base, hasFocus: true, visibility: 'hidden' })).toEqual({
      ok: true,
      why: 'notify',
    })
  })

  it('优先级：非桌面 > 关闭 > 中止 > 在看（顺序稳定，便于排查）', () => {
    // 全部条件都"不该发"时，返回最先命中的那一条，便于日志定位
    expect(
      shouldNotifyTurnEnded({
        enabled: false,
        isDesktop: false,
        hasFocus: true,
        visibility: 'visible',
        reason: 'aborted',
      }).why,
    ).toBe('not-desktop')
  })
})

describe('buildTurnNotificationBody —— 正文', () => {
  it('折叠空白并截断到 120 字', () => {
    const long = 'A'.repeat(300)
    const out = buildTurnNotificationBody(long)
    // 120 字 + 省略号
    expect(out).toBe(`${'A'.repeat(120)}…`)
  })

  it('多行与多余空白被压成一行', () => {
    expect(buildTurnNotificationBody('第一行\n\n  第二行   ')).toBe('第一行 第二行')
  })

  it('空正文给出状态说明，不发空通知', () => {
    expect(buildTurnNotificationBody('')).toBe('回合已结束（无文本输出）')
    expect(buildTurnNotificationBody('', 'aborted')).toBe('回合已中止')
  })
})

describe('偏好持久化', () => {
  beforeEach(() => localStorage.clear())
  afterEach(() => localStorage.clear())

  it('从未设置时默认开启', () => {
    expect(loadTurnNotifyEnabled()).toBe(true)
  })

  it('显式关闭后读回 false', () => {
    saveTurnNotifyEnabled(false)
    expect(loadTurnNotifyEnabled()).toBe(false)
  })

  it('显式开启后读回 true', () => {
    saveTurnNotifyEnabled(true)
    expect(loadTurnNotifyEnabled()).toBe(true)
  })

  it('localStorage 抛错时不崩（隐私模式）', () => {
    const spy = vi.spyOn(Storage.prototype, 'getItem').mockImplementation(() => {
      throw new Error('denied')
    })
    expect(loadTurnNotifyEnabled()).toBe(true) // 宁可多一条通知
    spy.mockRestore()
  })
})

describe('notifyTurnEnded —— 实际调用', () => {
  beforeEach(() => {
    localStorage.clear()
    vi.restoreAllMocks()
  })
  afterEach(() => localStorage.clear())

  it('非桌面壳时不调用 invoke', async () => {
    vi.spyOn(bridge, 'getTauriInvoke').mockReturnValue(undefined)
    const ok = await notifyTurnEnded({ text: '完成' })
    expect(ok).toBe(false)
  })

  it('桌面壳未聚焦时用正确 command 与参数调用', async () => {
    const invoke = vi.fn().mockResolvedValue(true)
    vi.spyOn(bridge, 'getTauriInvoke').mockReturnValue(invoke as never)
    // happy-dom 的 document.hasFocus() 恒返回 true，必须显式模拟"人不在"
    const focusSpy = vi.spyOn(document, 'hasFocus').mockReturnValue(false)
    const visSpy = vi.spyOn(document, 'visibilityState', 'get').mockReturnValue('hidden')

    const ok = await notifyTurnEnded({ text: '任务完成，共 3 个文件' })
    expect(ok).toBe(true)
    expect(invoke).toHaveBeenCalledWith('notify_turn_ended', {
      title: 'Marguerite · 回合完成',
      body: '任务完成，共 3 个文件',
    })
    focusSpy.mockRestore()
    visSpy.mockRestore()
  })

  it('invoke 抛错时返回 false 而不冒泡（通知失败不该影响主流程）', async () => {
    const invoke = vi.fn().mockRejectedValue(new Error('permission denied'))
    vi.spyOn(bridge, 'getTauriInvoke').mockReturnValue(invoke as never)
    const focusSpy = vi.spyOn(document, 'hasFocus').mockReturnValue(false)
    const visSpy = vi.spyOn(document, 'visibilityState', 'get').mockReturnValue('hidden')

    await expect(notifyTurnEnded({ text: 'x' })).resolves.toBe(false)
    focusSpy.mockRestore()
    visSpy.mockRestore()
  })

  it('关闭开关后即使桌面壳也不调用', async () => {
    saveTurnNotifyEnabled(false)
    const invoke = vi.fn().mockResolvedValue(true)
    vi.spyOn(bridge, 'getTauriInvoke').mockReturnValue(invoke as never)
    const focusSpy = vi.spyOn(document, 'hasFocus').mockReturnValue(false)
    const visSpy = vi.spyOn(document, 'visibilityState', 'get').mockReturnValue('hidden')

    expect(await notifyTurnEnded({ text: 'x' })).toBe(false)
    expect(invoke).not.toHaveBeenCalled()
    focusSpy.mockRestore()
    visSpy.mockRestore()
  })

  it('用户就在看（聚焦且可见）时不调用 invoke', async () => {
    const invoke = vi.fn().mockResolvedValue(true)
    vi.spyOn(bridge, 'getTauriInvoke').mockReturnValue(invoke as never)
    const focusSpy = vi.spyOn(document, 'hasFocus').mockReturnValue(true)
    const visSpy = vi.spyOn(document, 'visibilityState', 'get').mockReturnValue('visible')

    expect(await notifyTurnEnded({ text: 'x' })).toBe(false)
    expect(invoke).not.toHaveBeenCalled()
    focusSpy.mockRestore()
    visSpy.mockRestore()
  })
})
