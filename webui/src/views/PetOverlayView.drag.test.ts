// 桌宠悬浮窗「整卡可拖」契约（2026-10-25）。
//
// 用户原话：「桌宠还无法移动出窗体」。根因是只有顶部 16px 手柄绑了拖拽，
// 按着宠物本体拖什么都不动 —— 观感就是宠物被关在 360×420 的窗里出不来。
//
// 这里用**源码契约**锁住修法（与仓内 detectPetRuntime.test.ts 同范式）：
//   1. 舞台绑定了 pointerdown（不再只有手柄）；
//   2. 位移阈值存在且 ≥3px（区分"拖拽"与"双击抚摸"）；
//   3. 拖动走 moveWindow（改窗口位置，而不是在窗内挪图片）；
//   4. 拖拽后紧随的 dblclick 被吞掉（否则松手会莫名抚摸一次）。

import { describe, expect, it } from 'vitest'
import source from '@/views/PetOverlayView.vue?raw'

describe('PetOverlayView —— 桌宠整卡可拖', () => {
  it('舞台绑定了 pointerdown（拖拽不再只限顶部手柄）', () => {
    expect(source).toMatch(/class="pet-overlay__stage"[\s\S]{0,400}@pointerdown="onStagePointerDown"/)
  })

  it('拖拽阈值 ≥3px，且未达阈值不移动窗口', () => {
    const m = source.match(/const DRAG_THRESHOLD = (\d+)/)
    expect(m, '必须存在 DRAG_THRESHOLD 常量').toBeTruthy()
    expect(Number(m![1])).toBeGreaterThanOrEqual(3)
    expect(source).toMatch(/Math\.hypot\(dx, dy\) < DRAG_THRESHOLD\) return/)
  })

  it('拖动改变的是窗口位置（moveWindow），不是在窗内挪元素', () => {
    expect(source).toMatch(/moveWindow\(stageDrag\.wx \+ dx, stageDrag\.wy \+ dy\)/)
  })

  it('拖拽结束后吞掉尾随的 dblclick，避免误抚摸', () => {
    expect(source).toMatch(/if \(stageDrag\?\.moved\) swallowNextPet = true/)
    expect(source).toMatch(/if \(swallowNextPet\) \{[\s\S]{0,80}return/)
  })

  it('右键不进入拖拽分支（button !== 0 直接返回）', () => {
    expect(source).toMatch(/function onStagePointerDown\(ev: PointerEvent\) \{[\s\S]{0,160}if \(ev\.button !== 0\) return/)
  })

  // ── 2026-10-25 第二轮：真机仍然「拖不动」的根因是窗口级 ACL + Promise 漏接 ──
  it('手柄按下走原生 startDragging（不再只认 mousedown）', () => {
    expect(source).toMatch(/class="pet-overlay__handle"[\s\S]{0,200}@pointerdown="onHandlePointerDown"/)
  })

  it('startDragging 的 Promise reject 必须被 catch 并回退（ACL 拒绝时是 reject 不是 throw）', () => {
    expect(source).toMatch(/\.catch\(\(\) => beginFallbackDrag\(ev\)\)/)
    // 旧写法（try/catch + 直接 return）会让回退永不执行
    expect(source).not.toMatch(/try \{ w\.startDragging\(\); return \} catch/)
  })

  it('回退拖拽补上指针捕获，避免光标跑出窗体后 pointermove 断流', () => {
    expect(source).toMatch(/setPointerCapture\?\.\(ev\.pointerId\)/)
  })

  it('窗口移动命令的失败不能变成未处理拒绝', () => {
    expect(source).toMatch(/Promise\.resolve\(w\.invoke\('pet_overlay_move', \{ x, y \}\)\)\.catch\(\(\) => \{\}\)/)
  })
})
