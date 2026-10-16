import { describe, expect, it } from 'vitest'
import {
  buildComputerUseParams,
  CAPTURE_MODES,
  isVisionUnavailable,
  needsCapture,
  parseGrounding,
  supportsGrounding,
  type ComputerUseDraft,
} from './computerUse'

function draft(overrides: Partial<ComputerUseDraft> = {}): ComputerUseDraft {
  return {
    action: 'screenshot',
    x: 10,
    y: 20,
    dx: 30,
    dy: 40,
    text: 'hello',
    key: 'return',
    description: '右上角搜索框',
    vision: true,
    autoClick: false,
    patchGrounding: false,
    mode: 'primary',
    maxEdge: 0,
    ...overrides,
  }
}

describe('computer_use.action 请求契约（纯函数）', () => {
  it('locate 必须带上描述与 vision（否则网关只回「截图已就绪」的降级文案）', () => {
    const params = buildComputerUseParams(draft({ action: 'locate' }))
    expect(params.action).toBe('locate')
    expect(params.description).toBe('右上角搜索框')
    expect(params.vision).toBe(true)
    // 默认不自动点击（不可逆操作须显式开启）
    expect(params.auto_click).toBeUndefined()
    expect(params.patch_grounding).toBeUndefined()
  })

  it('三个开关只在显式开启时才进载荷，且不带无关参数', () => {
    const params = buildComputerUseParams(draft({
      action: 'locate',
      autoClick: true,
      patchGrounding: true,
      vision: false,
    }))
    expect(params.auto_click).toBe(true)
    expect(params.patch_grounding).toBe(true)
    // vision 关闭时不发该键（网关据此走诚实降级路径）
    expect('vision' in params).toBe(false)
    // locate 不消费坐标/文本/按键 → 不得混入
    expect('x' in params).toBe(false)
    expect('text' in params).toBe(false)
    expect('key' in params).toBe(false)
  })

  it('多屏与分辨率参数只发给真正会采集屏幕的动作', () => {
    expect(needsCapture('screenshot')).toBe(true)
    expect(needsCapture('locate')).toBe(true)
    expect(needsCapture('click')).toBe(false)

    const shot = buildComputerUseParams(draft({ action: 'screenshot', mode: 'all', maxEdge: 1600 }))
    expect(shot.mode).toBe('all')
    expect(shot.max_edge).toBe(1600)

    // maxEdge=0 表示原始分辨率 → 不发该键（避免网关把 0 当成「缩到 0」的歧义）
    const native = buildComputerUseParams(draft({ action: 'screenshot', maxEdge: 0 }))
    expect('max_edge' in native).toBe(false)

    // 非采集动作不带 mode/max_edge（否则界面显示的参数与实际生效的不一致）
    const click = buildComputerUseParams(draft({ action: 'click' }))
    expect('mode' in click).toBe(false)
    expect('max_edge' in click).toBe(false)
    expect(click.x).toBe(10)
    expect(click.y).toBe(20)
  })

  it('既有动作的载荷形状保持不变（drag/scroll/type/key 各自复用正确的键）', () => {
    expect(buildComputerUseParams(draft({ action: 'drag' }))).toMatchObject({
      action: 'drag', x: 10, y: 20, x2: 30, y2: 40,
    })
    expect(buildComputerUseParams(draft({ action: 'scroll' }))).toMatchObject({
      action: 'scroll', x: 10, y: 20, dx: 30, dy: 40,
    })
    expect(buildComputerUseParams(draft({ action: 'type' }))).toMatchObject({
      action: 'type', text: 'hello',
    })
    expect(buildComputerUseParams(draft({ action: 'key_combo' }))).toMatchObject({
      action: 'key_combo', key: 'return',
    })
    expect(CAPTURE_MODES).toEqual(['primary', 'all', 'window'])
    expect(supportsGrounding('locate')).toBe(true)
    expect(supportsGrounding('screenshot')).toBe(false)
  })
})

describe('grounding 响应解析（诚实呈现，不编造坐标）', () => {
  it('解析网关透出的 grounding 坐标/置信度/描述', () => {
    const view = parseGrounding({
      ok: true,
      data: '定位成功: x=1234.0, y=567.0, confidence=0.91\n截图: /tmp/s.png',
      grounding: { x: 1234, y: 567, confidence: 0.91, description: '右上角搜索按钮' },
    })
    expect(view).toEqual({
      x: 1234,
      y: 567,
      confidence: 0.91,
      description: '右上角搜索按钮',
      lowConfidence: false,
    })
  })

  it('低置信度由引擎的 data 前缀如实透传（不在前端自造阈值判定）', () => {
    const view = parseGrounding({
      ok: true,
      data: '[低置信度] 定位成功: x=1.0, y=2.0, confidence=0.20',
      grounding: { x: 1, y: 2, confidence: 0.2 },
    })
    expect(view?.lowConfidence).toBe(true)
    expect(view?.description).toBeUndefined()
  })

  it('缺坐标/畸形载荷 → 无结果（不显示占位坐标）', () => {
    expect(parseGrounding({ ok: true, data: '截图已就绪' })).toBeNull()
    expect(parseGrounding({ ok: true, grounding: { x: 1 } })).toBeNull()
    expect(parseGrounding({ ok: true, grounding: { x: 'abc', y: 2 } })).toBeNull()
    expect(parseGrounding(null)).toBeNull()
    expect(parseGrounding('nope')).toBeNull()
  })

  it('未配置视觉模型的软失败码被单独识别（UI 给可执行提示而非泛化失败）', () => {
    expect(isVisionUnavailable({
      ok: false,
      code: 'VISION_UNAVAILABLE',
      error: '请求了视觉 grounding，但当前未配置可用视觉模型',
    })).toBe(true)
    expect(isVisionUnavailable({ ok: false, error: 'boom' })).toBe(false)
    expect(isVisionUnavailable(null)).toBe(false)
  })
})
