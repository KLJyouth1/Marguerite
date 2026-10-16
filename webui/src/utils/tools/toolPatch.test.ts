import { describe, expect, it } from 'vitest'

import { applyDisabled, computeNextDisabled, disabledNames } from './toolPatch'
import type { ToolCatalogItem } from './toolPatch'

describe('tool patch helpers', () => {
  it('keeps other disabled tools when toggling one off', () => {
    // 已有两个工具被停用，再停用第三个：前两个绝不能丢
    const next = computeNextDisabled(['a', 'b'], 'c', false)
    expect(next).toEqual(['a', 'b', 'c'])
  })

  it('removes only the re-enabled tool', () => {
    const next = computeNextDisabled(['a', 'b', 'c'], 'b', true)
    expect(next).toEqual(['a', 'c'])
  })

  it('is idempotent when the requested state already holds', () => {
    expect(computeNextDisabled(['a'], 'a', false)).toEqual(['a'])
    expect(computeNextDisabled(['a'], 'b', true)).toEqual(['a'])
  })

  it('does not mutate the input array', () => {
    const current = ['a']
    computeNextDisabled(current, 'b', false)
    expect(current).toEqual(['a'])
  })

  it('derives enabled flags from the backend disabled list without mutating items', () => {
    const items = [
      { name: 'read_file', description: 'read' },
      { name: 'shell_exec', description: 'shell' },
    ]
    const out = applyDisabled(items, ['shell_exec'])
    expect(out).toEqual([
      { name: 'read_file', description: 'read', enabled: true },
      { name: 'shell_exec', description: 'shell', enabled: false },
    ])
    // 入参不得被就地改写（Vue 的响应式对象被就地改会让 diff 失真）
    expect('enabled' in items[0]).toBe(false)
  })

  it('treats a missing enabled flag as enabled (old backend compatibility)', () => {
    expect(disabledNames([{ name: 'a' }, { name: 'b', enabled: false }])).toEqual(['b'])
    const legacy: ToolCatalogItem[] = [{ name: 'a' }]
    expect(applyDisabled(legacy, [])[0]?.enabled).toBe(true)
  })
})
