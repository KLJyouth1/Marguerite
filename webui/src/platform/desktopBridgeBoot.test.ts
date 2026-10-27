// @vitest-environment node
import { readFileSync } from 'node:fs'
import { describe, expect, it } from 'vitest'

/**
 * 顺序契约：`router/index.ts` 在模块顶层 `getPlatform()` 并把结果存进常量，
 * 因此桌面桥必须在该模块**被求值之前**安装。ESM 会先求值所有 import 再执行
 * 函数体，唯一可靠的写法是把引导模块放在 `main.ts` 的第一个 import。
 *
 * 这条测试防的是"以后有人整理 import 顺序，把桥降级成语句调用"这类静默回归 ——
 * 它不会报错，只会让整个桌面端悄悄退回 web 平台。
 */
describe('desktop bridge boot ordering', () => {
  const source = readFileSync(new URL('../main.ts', import.meta.url), 'utf8')

  it('keeps the bridge boot module as the very first import', () => {
    const firstImport = source.match(/^import\s.+$/m)?.[0] ?? ''
    expect(firstImport).toContain('./platform/desktopBridgeBoot')
  })

  it('does not also install the bridge as a late statement', () => {
    // 语句区调用发生在所有 import 求值之后，等于无效；出现即说明有人误解了时序。
    expect(source).not.toMatch(/^\s*installDesktopBridge\(\)/m)
  })
})
