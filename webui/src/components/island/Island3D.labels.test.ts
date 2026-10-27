// @vitest-environment happy-dom
import { describe, expect, it } from 'vitest'
import {
  anchorFitsCanvas,
  estimateLabelBox,
  separateLabelAnchors,
  type IslandLabelAnchor,
} from './Island3D'

function anchor(id: string, x: number, y: number, visible = true): IslandLabelAnchor {
  return { id, x, y, visible, selected: false }
}

/**
 * 真机缺陷回归（2026-10-23，Chromium 实测）：
 * 焦点建筑后，建筑顶部投影到画布上沿之外，标签仍被标记为 visible，
 * 结果 HTML 胶囊被 `.island-stage` 的 overflow 裁掉一半（截图里只剩半截药丸），
 * 完全在画布外的锚点则变成看不见却仍占 DOM 的「幽灵按钮」。
 *
 * 画布实测尺寸 898×672；被截断的标签底边距画布顶 8px、高 22px。
 */
describe('island label fitting', () => {
  it('hides a label whose top edge would be clipped by the canvas top', () => {
    const box = estimateLabelBox('construction:workshop-tower')
    // 底边只离画布顶 8px，胶囊高 22px → 上沿在画布外
    expect(anchorFitsCanvas(379, 8, 898, 672, box)).toBe(false)
  })

  it('hides anchors that project entirely above the canvas after focusing', () => {
    const box = estimateLabelBox('session')
    expect(anchorFitsCanvas(422, -77, 898, 672, box)).toBe(false)
  })

  it('keeps labels that fit comfortably', () => {
    const box = estimateLabelBox('session')
    expect(anchorFitsCanvas(449, 400, 898, 672, box)).toBe(true)
    // 恰好贴住上沿（上沿 = 0）仍算可见
    expect(anchorFitsCanvas(449, box.height, 898, 672, box)).toBe(true)
  })

  it('hides labels that would overflow the left or right edge', () => {
    const box = estimateLabelBox('construction:finished-kiosk')
    const half = box.width / 2
    expect(anchorFitsCanvas(half - 1, 300, 898, 672, box)).toBe(false)
    expect(anchorFitsCanvas(898 - half + 1, 300, 898, 672, box)).toBe(false)
    expect(anchorFitsCanvas(half + 1, 300, 898, 672, box)).toBe(true)
  })

  it('never shows labels on an unmeasured canvas', () => {
    expect(anchorFitsCanvas(0, 0, 0, 0, estimateLabelBox('core'))).toBe(false)
  })

  it('estimates wider boxes for CJK and clamps very long titles', () => {
    // 同字数下 CJK 更宽（11px/字 vs 6.2px/字）
    const twoCjk = estimateLabelBox('主堡').width
    const twoLatin = estimateLabelBox('ab').width
    expect(twoCjk).toBeGreaterThan(twoLatin)
    // 长 id 不产生无限宽的框（否则任何锚点都判为越界）
    expect(estimateLabelBox('x'.repeat(400)).width).toBe(220)
    expect(estimateLabelBox('').width).toBe(16)
  })
})

/**
 * 真机缺陷回归：默认视角下 `session`（y=345.3）与 `construction:workshop-tower`
 * （y=329.3）的胶囊重叠 5.3px，主堡名字被遮掉约四分之一。
 */
describe('island label separation', () => {
  const boxes = new Map([
    ['session', estimateLabelBox('session')],
    ['tower', estimateLabelBox('construction:workshop-tower')],
    ['kiosk', estimateLabelBox('construction:finished-kiosk')],
  ])

  it('pushes the lower label down so capsules no longer overlap', () => {
    // 实测：tower 底边 329.3、session 底边 345.3，胶囊高 21.4
    const out = separateLabelAnchors(
      [anchor('session', 766, 345.3), anchor('tower', 778, 329.3)],
      boxes,
      898,
      672,
    )
    const session = out.find((a) => a.id === 'session')!
    const tower = out.find((a) => a.id === 'tower')!
    expect(tower.y).toBeCloseTo(329.3, 3) // 上面那枚不动
    const gap = session.y - sessionBoxHeight()
    expect(gap).toBeGreaterThanOrEqual(tower.y + 2 - 1e-6)
    // x 不变：只做竖向避让
    expect(session.x).toBe(766)
  })

  it('leaves well-separated labels untouched', () => {
    const out = separateLabelAnchors(
      [anchor('session', 400, 200), anchor('kiosk', 800, 500)],
      boxes,
      898,
      672,
    )
    expect(out.find((a) => a.id === 'session')!.y).toBe(200)
    expect(out.find((a) => a.id === 'kiosk')!.y).toBe(500)
  })

  it('hides a label that would be pushed past the canvas bottom', () => {
    // 两枚都贴着画布底部：下面那枚没有空间，只能退回隐藏
    const out = separateLabelAnchors(
      [anchor('session', 400, 670), anchor('kiosk', 700, 671)],
      boxes,
      898,
      672,
    )
    expect(out.filter((a) => a.visible)).toHaveLength(1)
  })

  it('keeps already-hidden anchors in the result', () => {
    const out = separateLabelAnchors(
      [anchor('session', 400, 300), anchor('tower', 500, 200, false)],
      boxes,
      898,
      672,
    )
    const tunnel = out.find((a) => a.id === 'tower')!
    expect(tunnel.visible).toBe(false)
    expect(tunnel.y).toBe(200)
  })
})

function sessionBoxHeight(): number {
  return estimateLabelBox('session').height
}
