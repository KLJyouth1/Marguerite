// @vitest-environment happy-dom
// 右侧面板宽度控制器：clamp / 持久化 / 键盘步长 / 拖拽落盘。
//
// 用户实测缺陷：右侧面板是固定 300px 浮层，打开时压住会话窗、不能调宽。
// 这些用例守住三件事：
//   1. 宽度永远落在 [280px, 55vw]，脏数据不炸；
//   2. 用户拖过的宽度要跨会话记住（`marguerite.layout.rightPanelWidth`）；
//   3. 分隔线键盘 ←/→ 每次正好 ±24px，且松手/按键才落盘。
import { beforeEach, describe, expect, it } from 'vitest'

import {
  RIGHT_PANEL_DEFAULT_WIDTH,
  RIGHT_PANEL_KEYBOARD_STEP,
  RIGHT_PANEL_MIN_WIDTH,
  RIGHT_PANEL_WIDTH_KEY,
  clampRightPanelWidth,
  parseRightPanelWidth,
  persistRightPanelWidth,
  readRightPanelWidth,
  rightPanelMaxWidth,
  usePanelResize,
  widthAfterKey,
} from './usePanelResize'

beforeEach(() => {
  window.innerWidth = 1440
  localStorage.clear()
})

describe('usePanelResize · 纯逻辑', () => {
  it('把宽度收敛到 280px .. 55vw', () => {
    expect(clampRightPanelWidth(120, 1440)).toBe(RIGHT_PANEL_MIN_WIDTH)
    expect(clampRightPanelWidth(2000, 1440)).toBe(792)
    expect(rightPanelMaxWidth(1440)).toBe(792)
    // 窄屏时 55vw < 280：下限优先，避免面板窄到不可用
    expect(rightPanelMaxWidth(500)).toBe(RIGHT_PANEL_MIN_WIDTH)
    expect(clampRightPanelWidth(400, 500)).toBe(RIGHT_PANEL_MIN_WIDTH)
  })

  it('缺省 / 脏数据回落到默认宽度，不产生 NaN', () => {
    expect(clampRightPanelWidth(Number.NaN, 1440)).toBe(RIGHT_PANEL_DEFAULT_WIDTH)
    expect(parseRightPanelWidth(null, 1440)).toBe(RIGHT_PANEL_DEFAULT_WIDTH)
    expect(parseRightPanelWidth('', 1440)).toBe(RIGHT_PANEL_DEFAULT_WIDTH)
    expect(parseRightPanelWidth('not-a-number', 1440)).toBe(RIGHT_PANEL_DEFAULT_WIDTH)
    expect(parseRightPanelWidth('120', 1440)).toBe(RIGHT_PANEL_MIN_WIDTH)
    expect(parseRightPanelWidth('9999', 1440)).toBe(792)
  })

  it('键盘 ←/→ 每次 ±24px，其它按键不接管', () => {
    expect(RIGHT_PANEL_KEYBOARD_STEP).toBe(24)
    expect(widthAfterKey(300, 'ArrowLeft', 1440)).toBe(324)
    expect(widthAfterKey(300, 'ArrowRight', 1440)).toBe(280)
    expect(widthAfterKey(RIGHT_PANEL_MIN_WIDTH, 'ArrowRight', 1440)).toBe(280)
    expect(widthAfterKey(790, 'ArrowLeft', 1440)).toBe(792)
    expect(widthAfterKey(300, 'Enter', 1440)).toBeNull()
  })

  it('持久化键固定为 marguerite.layout.rightPanelWidth', () => {
    expect(RIGHT_PANEL_WIDTH_KEY).toBe('marguerite.layout.rightPanelWidth')
    persistRightPanelWidth(444)
    expect(localStorage.getItem(RIGHT_PANEL_WIDTH_KEY)).toBe('444')
    expect(readRightPanelWidth(1440)).toBe(444)
    // 读回时同样要 clamp（用户换到更窄的屏幕）
    localStorage.setItem(RIGHT_PANEL_WIDTH_KEY, '5000')
    expect(readRightPanelWidth(1440)).toBe(792)
  })
})

describe('usePanelResize · 交互', () => {
  it('拖拽分隔线时实时改宽，松手才落盘', () => {
    const { width, resizing, startResize } = usePanelResize()
    width.value = 320

    startResize({ button: 0, clientX: 800, preventDefault: () => {} } as unknown as PointerEvent)
    expect(resizing.value).toBe(true)

    window.dispatchEvent(new MouseEvent('pointermove', { clientX: 700 }))
    expect(width.value).toBe(420)
    // 拖动过程中不写盘
    expect(localStorage.getItem(RIGHT_PANEL_WIDTH_KEY)).toBeNull()

    window.dispatchEvent(new MouseEvent('pointerup', { clientX: 700 }))
    expect(resizing.value).toBe(false)
    expect(localStorage.getItem(RIGHT_PANEL_WIDTH_KEY)).toBe('420')
  })

  it('拖拽同样受 clamp 约束（拖到视口外不会越界）', () => {
    const { width, startResize } = usePanelResize()
    width.value = 400
    startResize({ button: 0, clientX: 800, preventDefault: () => {} } as unknown as PointerEvent)
    window.dispatchEvent(new MouseEvent('pointermove', { clientX: -5000 }))
    expect(width.value).toBe(792)
    window.dispatchEvent(new MouseEvent('pointerup', { clientX: -5000 }))
  })

  it('键盘微调同步落盘', () => {
    const { width, onResizeKeydown } = usePanelResize()
    width.value = 300
    const ev = { key: 'ArrowLeft', preventDefault: () => {} } as unknown as KeyboardEvent
    onResizeKeydown(ev)
    expect(width.value).toBe(324)
    expect(localStorage.getItem(RIGHT_PANEL_WIDTH_KEY)).toBe('324')
  })

  it('右键不进入拖拽', () => {
    const { width, resizing, startResize } = usePanelResize()
    width.value = 300
    startResize({ button: 2, clientX: 800, preventDefault: () => {} } as unknown as PointerEvent)
    expect(resizing.value).toBe(false)
    window.dispatchEvent(new MouseEvent('pointermove', { clientX: 100 }))
    expect(width.value).toBe(300)
  })
})
