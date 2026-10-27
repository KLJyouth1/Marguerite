// 布局契约（源码级）：happy-dom 不做布局计算，滚动/裁剪这类缺陷只能在
// 组件源码里锁住"必须存在的 CSS 不变量"。每一条都对应一次真机事故。
import { readFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import { dirname, resolve } from 'node:path'
import { describe, expect, it } from 'vitest'

const here = dirname(fileURLToPath(import.meta.url))
/** 去掉注释再断言：注释里会提到被禁用的旧写法（例如"之前 left: 0 会被压住"）。 */
const read = (file: string) => readFileSync(resolve(here, file), 'utf8')
  .replace(/\/\*[\s\S]*?\*\//g, '')

/** 取某个选择器的第一条规则块（含嵌套声明），用于断言关键声明存在。 */
function block(css: string, selector: string): string {
  const start = css.indexOf(selector)
  if (start < 0) return ''
  const open = css.indexOf('{', start)
  if (open < 0) return ''
  let depth = 0
  for (let i = open; i < css.length; i += 1) {
    if (css[i] === '{') depth += 1
    else if (css[i] === '}') {
      depth -= 1
      if (depth === 0) return css.slice(open + 1, i)
    }
  }
  return ''
}

describe('选择模型：列表必须可滚、面板不得溢出', () => {
  const css = read('./ChatModelPicker.vue')

  it('面板是 flex 列并裁掉溢出', () => {
    const panel = block(css, '.model-picker__panel {')
    expect(panel).toContain('display: flex')
    expect(panel).toContain('flex-direction: column')
    expect(panel).toContain('overflow: hidden')
  })

  it('滚动区允许收缩（min-height:0）并出现滚动条', () => {
    const scroll = block(css, '.model-picker__scroll {')
    expect(scroll).toContain('min-height: 0')
    expect(scroll).toContain('overflow-y: auto')
    expect(scroll).toContain('flex: 1 1 auto')
  })
})

describe('运行模式与路由：面板有高度上限且可滚', () => {
  const css = read('./RunModePanel.vue')

  it('popover 限制高度并允许滚动', () => {
    const popover = block(css, '.run-mode-panel__popover {')
    expect(popover).toContain('max-height')
    expect(popover).toContain('overflow-y: auto')
    expect(popover).toContain('overscroll-behavior: contain')
  })

  it('内联子面板脱离绝对定位（否则会逃出父滚动盒）', () => {
    const runMode = read('./ChatComposerRunMode.vue')
    const routing = read('./ChatComposerModelRouting.vue')
    expect(block(runMode, '.composer-run-mode.is-inline {')).toContain('position: static')
    expect(block(routing, '.composer-model-routing.is-inline {')).toContain('position: static')
  })
})

describe('推理等级浮层：向左展开，避免被右侧栏压住', () => {
  const css = read('./ChatEffortSlider.vue')

  it('锚定 right 而不是 left', () => {
    const pop = block(css, '.effort-pop {')
    expect(pop).toContain('right: 0')
    expect(pop).not.toContain('left: 0')
  })

  it('不再有光晕/火星这类装饰动效', () => {
    expect(css).not.toContain('effort-track__halo')
    expect(css).not.toContain('effort-track__sparks')
    expect(css).not.toContain('@keyframes effort-halo')
  })
})

describe('能力面板：不再重复渲染 3×3 档位矩阵', () => {
  const css = read('./ChatModeCapabilityPanel.vue')
  const tpl = css.slice(0, css.indexOf('<style'))

  it('模板与样式都不再包含矩阵单元', () => {
    expect(tpl).not.toContain('orch-mode__matrix')
    expect(tpl).not.toContain('USER_MODE_INTENSITY')
    expect(css).not.toContain('.orch-mode__cell')
  })
})
