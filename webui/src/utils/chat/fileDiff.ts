// 行级 diff（LCS），用于「文件更改卡片」的 before → after 差异展示。
//
// 与 Rust 后端 `builtin::diff_lines` 同构（最长公共子序列），输出统一为
// 三类行：ctx（未变）/ del（删除）/ add（新增），前端据此着色渲染。
// 对超大输入（行列乘积过大）降级为「整块删除 + 整块新增」，避免 O(n·m)
// 的内存与耗时失控。

export interface FileDiffLine {
  type: 'ctx' | 'del' | 'add'
  text: string
}

/** LCS 降级阈值：old 行数 × new 行数超过此值则不做精确 diff。 */
const DIFF_DEGRADE_CELLS = 400_000

export function diffLines(oldText: string, newText: string): FileDiffLine[] {
  const a = oldText.split('\n')
  const b = newText.split('\n')
  const cells = a.length * b.length

  // 超大输入直接降级：整段 before 删除、整段 after 新增，仍可读但不再精确对齐。
  if (cells > DIFF_DEGRADE_CELLS) {
    return [
      ...a.map(text => ({ type: 'del' as const, text })),
      ...b.map(text => ({ type: 'add' as const, text })),
    ]
  }

  const n = a.length
  const m = b.length
  // dp[i][j] = a[i..] 与 b[j..] 的 LCS 长度
  const dp: number[][] = Array.from({ length: n + 1 }, () => new Array<number>(m + 1).fill(0))
  for (let i = n - 1; i >= 0; i--) {
    for (let j = m - 1; j >= 0; j--) {
      dp[i][j] = a[i] === b[j]
        ? dp[i + 1][j + 1] + 1
        : Math.max(dp[i + 1][j], dp[i][j + 1])
    }
  }

  const out: FileDiffLine[] = []
  let i = 0
  let j = 0
  while (i < n && j < m) {
    if (a[i] === b[j]) {
      out.push({ type: 'ctx', text: a[i] })
      i++
      j++
    } else if (dp[i + 1][j] >= dp[i][j + 1]) {
      out.push({ type: 'del', text: a[i] })
      i++
    } else {
      out.push({ type: 'add', text: b[j] })
      j++
    }
  }
  while (i < n) {
    out.push({ type: 'del', text: a[i] })
    i++
  }
  while (j < m) {
    out.push({ type: 'add', text: b[j] })
    j++
  }
  return out
}

/** 统计 diff 中新增/删除的行数（用于折叠态的摘要，如「+3 -1」）。 */
export function diffStats(lines: FileDiffLine[]): { added: number; removed: number } {
  let added = 0
  let removed = 0
  for (const line of lines) {
    if (line.type === 'add') added++
    else if (line.type === 'del') removed++
  }
  return { added, removed }
}
