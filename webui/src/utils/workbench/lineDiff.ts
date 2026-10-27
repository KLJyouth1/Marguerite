/**
 * 行级差异（给编辑器画「智能体改了哪几行」）。
 *
 * 只做**行级**判断：没有字符级 diff、没有语法感知 —— 目的是让用户一眼看到
 * 「新增 / 修改 / 删除」分别落在哪些行，而不是替代 git diff。
 *
 * 为什么自己算而不是引依赖：编辑器的改动标记只需要「同位置行的对应关系」，
 * LCS 在这个规模下（≤2000 行）毫秒级；引 diff 库反而多一份体积与 API 漂移风险。
 */

export type DiffStatus = 'same' | 'added' | 'removed' | 'modified'

export interface DiffRow {
  status: DiffStatus
  /** 旧文件该行内容（added 时为空） */
  before?: string
  /** 新文件该行内容（removed 时为空） */
  after?: string
}

export interface DiffSummary {
  added: number
  removed: number
  modified: number
}

/** 超过任一侧行数上限就不算（避免 O(n·m) 卡死界面）；调用方需如实说明「未计算」。 */
export const DIFF_MAX_LINES = 2000

function toLines(text: string): string[] {
  // 空串是**零行**，不是「一行空内容」：否则「新建文件」会被算成 1 处修改 + n 处新增
  if (text === '') return []
  // 统一 CRLF：Windows 上同一份文件来回读写时不应产生「整篇都改了」
  return text.replace(/\r\n/g, '\n').replace(/\r/g, '\n').split('\n')
}

type Op = { kind: 'same'; line: string } | { kind: 'del'; line: string } | { kind: 'ins'; line: string }

/**
 * LCS 行对齐 → 操作序列。超过上限返回 `null`（调用方据此跳过标记）。
 */
function align(before: string[], after: string[]): Op[] | null {
  if (before.length > DIFF_MAX_LINES || after.length > DIFF_MAX_LINES) return null
  const n = before.length
  const m = after.length
  // dp[i][j] = before[i..] 与 after[j..] 的 LCS 长度
  const dp: Uint32Array[] = Array.from({ length: n + 1 }, () => new Uint32Array(m + 1))
  for (let i = n - 1; i >= 0; i -= 1) {
    for (let j = m - 1; j >= 0; j -= 1) {
      dp[i][j] = before[i] === after[j]
        ? dp[i + 1][j + 1] + 1
        : Math.max(dp[i + 1][j], dp[i][j + 1])
    }
  }
  const ops: Op[] = []
  let i = 0
  let j = 0
  while (i < n && j < m) {
    if (before[i] === after[j]) {
      ops.push({ kind: 'same', line: before[i] })
      i += 1
      j += 1
    } else if (dp[i + 1][j] >= dp[i][j + 1]) {
      ops.push({ kind: 'del', line: before[i] })
      i += 1
    } else {
      ops.push({ kind: 'ins', line: after[j] })
      j += 1
    }
  }
  while (i < n) { ops.push({ kind: 'del', line: before[i] }); i += 1 }
  while (j < m) { ops.push({ kind: 'ins', line: after[j] }); j += 1 }
  return ops
}

/**
 * 计算行级差异。
 *
 * 连续「删除 + 插入」按 `min(删, 插)` 配对成 `modified`，多出来的照旧算
 * `removed` / `added` —— 这与用户对「改了一行」的直觉一致。
 *
 * @returns 行序列；任一侧超过 {@link DIFF_MAX_LINES} 行时返回 `null`
 */
export function diffLines(before: string, after: string): DiffRow[] | null {
  const ops = align(toLines(before), toLines(after))
  if (!ops) return null
  const rows: DiffRow[] = []
  let k = 0
  while (k < ops.length) {
    const op = ops[k]
    if (op.kind === 'same') {
      rows.push({ status: 'same', before: op.line, after: op.line })
      k += 1
      continue
    }
    // 收集相邻的删除组与插入组
    const dels: string[] = []
    const inss: string[] = []
    while (k < ops.length && ops[k].kind === 'del') { dels.push((ops[k] as { line: string }).line); k += 1 }
    while (k < ops.length && ops[k].kind === 'ins') { inss.push((ops[k] as { line: string }).line); k += 1 }
    const paired = Math.min(dels.length, inss.length)
    for (let p = 0; p < paired; p += 1) {
      rows.push({ status: 'modified', before: dels[p], after: inss[p] })
    }
    for (let p = paired; p < dels.length; p += 1) rows.push({ status: 'removed', before: dels[p] })
    for (let p = paired; p < inss.length; p += 1) rows.push({ status: 'added', after: inss[p] })
  }
  return rows
}

export function diffSummary(rows: readonly DiffRow[]): DiffSummary {
  const out: DiffSummary = { added: 0, removed: 0, modified: 0 }
  for (const row of rows) {
    if (row.status === 'added') out.added += 1
    else if (row.status === 'removed') out.removed += 1
    else if (row.status === 'modified') out.modified += 1
  }
  return out
}

/**
 * 新文件里「被智能体动过」的行号集合（1 基，`modified`/`added` 都算）。
 * `removed` 行在新文件里没有位置，因此不在集合里 —— 它只在差异视图里出现。
 */
export function changedLineNumbers(rows: readonly DiffRow[]): Set<number> {
  const out = new Set<number>()
  let cursor = 0
  for (const row of rows) {
    if (row.status === 'removed') continue
    cursor += 1
    if (row.status === 'added' || row.status === 'modified') out.add(cursor)
  }
  return out
}
