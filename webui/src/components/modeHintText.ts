/**
 * 去掉 hint 文案里重复的模式别名。
 *
 * 事故（真机截图）：`mode.hint.omni-transcend` 曾写成「全知超验 · 跨模态综合推理」，
 * 而 `ModeHintBar` 已经在左侧渲染别名 → 顶栏显示成「全知超验 · 全知超验 · 跨模态…」。
 * 六语言文案已修正；这里再补一层防御：任何语言若又把别名写进 hint 开头，
 * 渲染前也会被剥掉，不会再次出现重复 chip。
 */
const SEPARATORS = [' · ', ' — ', ' – ', ' - ', '：', ': ']

export function stripDuplicateLabel(hint: string, label: string): string {
  const raw = (hint ?? '').trim()
  const l = (label ?? '').trim()
  if (!raw) return ''
  if (!l) return raw
  for (const sep of SEPARATORS) {
    if (raw.startsWith(l + sep)) return raw.slice((l + sep).length).trim()
  }
  return raw === l ? '' : raw
}
