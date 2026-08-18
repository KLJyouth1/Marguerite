import { describe, expect, it } from 'vitest'

import { diffLines, diffStats } from './fileDiff'

describe('fileDiff.diffLines', () => {
  it('aligns a single-line replacement as del + add', () => {
    expect(diffLines('old', 'new')).toEqual([
      { type: 'del', text: 'old' },
      { type: 'add', text: 'new' },
    ])
  })

  it('keeps unchanged context lines and marks a mid-file edit', () => {
    const lines = diffLines('a\nb\nc', 'a\nB\nc')
    expect(lines.map(line => `${line.type}:${line.text}`)).toEqual([
      'ctx:a',
      'del:b',
      'add:B',
      'ctx:c',
    ])
  })

  it('degrades oversized inputs to a single delete + add block', () => {
    const oldText = Array.from({ length: 700 }, (_, i) => `old-${i}`).join('\n')
    const newText = Array.from({ length: 700 }, (_, i) => `new-${i}`).join('\n')
    const lines = diffLines(oldText, newText)
    expect(lines.length).toBe(1400)
    expect(lines[0]).toEqual({ type: 'del', text: 'old-0' })
    expect(lines[699]).toEqual({ type: 'del', text: 'old-699' })
    expect(lines[700]).toEqual({ type: 'add', text: 'new-0' })
  })
})

describe('fileDiff.diffStats', () => {
  it('counts added and removed lines', () => {
    expect(diffStats(diffLines('a\nb', 'a\nB\nc'))).toEqual({ added: 2, removed: 1 })
  })
})
