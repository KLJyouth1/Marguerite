import { describe, expect, it } from 'vitest'
import {
  changedLineNumbers,
  diffLines,
  diffSummary,
  DIFF_MAX_LINES,
} from './lineDiff'

/**
 * 行级差异是编辑器「智能体改了哪几行」的唯一数据源，必须逐条锁住：
 * 统计口径（新增/修改/删除）与行号映射错一处，用户看到的就是「标记在错误的行上」。
 */
describe('lineDiff', () => {
  it('内容相同 → 全部 same，统计为 0', () => {
    const rows = diffLines('a\nb\nc', 'a\nb\nc')!
    expect(rows.map(r => r.status)).toEqual(['same', 'same', 'same'])
    expect(diffSummary(rows)).toEqual({ added: 0, removed: 0, modified: 0 })
    expect([...changedLineNumbers(rows)]).toEqual([])
  })

  it('新增行 → added，并映射到新文件行号', () => {
    const rows = diffLines('a\nc', 'a\nb\nc')!
    expect(rows.map(r => r.status)).toEqual(['same', 'added', 'same'])
    expect(diffSummary(rows)).toEqual({ added: 1, removed: 0, modified: 0 })
    // 新文件第 2 行是新增
    expect([...changedLineNumbers(rows)]).toEqual([2])
  })

  it('修改一行 → 配成 modified（而不是一删一增）', () => {
    const rows = diffLines('a\nold\nc', 'a\nnew\nc')!
    expect(rows.map(r => r.status)).toEqual(['same', 'modified', 'same'])
    expect(rows[1]).toMatchObject({ before: 'old', after: 'new' })
    expect(diffSummary(rows)).toEqual({ added: 0, removed: 0, modified: 1 })
    expect([...changedLineNumbers(rows)]).toEqual([2])
  })

  it('删除行 → removed，且不占用新文件行号', () => {
    const rows = diffLines('a\ngone\nb', 'a\nb')!
    expect(rows.map(r => r.status)).toEqual(['same', 'removed', 'same'])
    expect(diffSummary(rows)).toEqual({ added: 0, removed: 1, modified: 0 })
    // 被删行在新文件里没有位置
    expect([...changedLineNumbers(rows)]).toEqual([])
  })

  it('删除多于新增时，多出来的仍算 removed', () => {
    const rows = diffLines('x\ny\nz', 'q')!
    expect(diffSummary(rows)).toEqual({ added: 0, removed: 2, modified: 1 })
  })

  it('CRLF 与 LF 视为同一份内容（Windows 来回读写不该全篇标红）', () => {
    const rows = diffLines('a\r\nb\r\n', 'a\nb\n')!
    expect(rows.every(r => r.status === 'same')).toBe(true)
  })

  it('任一侧超过上限 → 返回 null（调用方必须如实说明「未计算」）', () => {
    const big = Array.from({ length: DIFF_MAX_LINES + 1 }, (_, i) => `l${i}`).join('\n')
    expect(diffLines(big, 'a')).toBeNull()
    expect(diffLines('a', big)).toBeNull()
  })

  it('空文件 ↔ 有内容', () => {
    const added = diffLines('', 'a\nb')!
    expect(diffSummary(added)).toMatchObject({ added: 2 })
    const removed = diffLines('a\nb', '')!
    expect(diffSummary(removed)).toMatchObject({ removed: 2 })
  })
})
