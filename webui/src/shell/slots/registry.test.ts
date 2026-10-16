// @vitest-environment happy-dom
import { describe, expect, it } from 'vitest'
import { defineComponent, h } from 'vue'
import { registerUiSlotContribution, slotContributions, uiSlotVersion } from './registry'
import type { UiSlotContribution } from './types'

const Stub = defineComponent({ render: () => h('span') })

function contribution(
  id: string,
  order: number,
  slot: UiSlotContribution['slot'] = 'sidebar.footer',
): UiSlotContribution {
  return { id, slot, order, component: Stub }
}

describe('ui slot registry', () => {
  it('renders contributions ordered by `order`', () => {
    const offs = [
      registerUiSlotContribution(contribution('zz.late', 30)),
      registerUiSlotContribution(contribution('zz.early', 10)),
      registerUiSlotContribution(contribution('zz.mid', 20)),
    ]
    try {
      expect(slotContributions('sidebar.footer').map(c => c.id)).toEqual([
        'zz.early',
        'zz.mid',
        'zz.late',
      ])
    } finally {
      offs.forEach(off => off())
    }
  })

  it('breaks ties on id so order never depends on registration sequence', () => {
    const offs = [
      registerUiSlotContribution(contribution('zz.b', 10)),
      registerUiSlotContribution(contribution('zz.a', 10)),
    ]
    try {
      expect(slotContributions('sidebar.footer').map(c => c.id)).toEqual(['zz.a', 'zz.b'])
    } finally {
      offs.forEach(off => off())
    }
  })

  it('refuses a duplicate id in the same slot (no silent shadowing)', () => {
    const off = registerUiSlotContribution(contribution('zz.dup', 10))
    try {
      expect(() => registerUiSlotContribution(contribution('zz.dup', 20))).toThrow(
        /already has a contribution with id "zz\.dup"/,
      )
    } finally {
      off()
    }
  })

  it('refuses the reserved host namespace and empty ids', () => {
    expect(() => registerUiSlotContribution(contribution('core.hijack', 10))).toThrow(
      /reserved "core\." namespace/,
    )
    expect(() => registerUiSlotContribution(contribution('   ', 10))).toThrow(/non-empty id/)
  })

  it('is append-only: registering keeps existing contributions', () => {
    const offA = registerUiSlotContribution(contribution('zz.keep', 10))
    const offB = registerUiSlotContribution(contribution('zz.added', 20))
    try {
      const ids = slotContributions('sidebar.footer').map(c => c.id)
      expect(ids).toContain('zz.keep')
      expect(ids).toContain('zz.added')
    } finally {
      offA()
      offB()
    }
  })

  it('keeps slots isolated and bumps the version on change', () => {
    const before = uiSlotVersion().value
    const off = registerUiSlotContribution(contribution('zz.top', 10, 'topbar.actions'))
    try {
      expect(slotContributions('topbar.actions').map(c => c.id)).toEqual(['zz.top'])
      expect(slotContributions('sidebar.nav').map(c => c.id)).not.toContain('zz.top')
      expect(uiSlotVersion().value).toBeGreaterThan(before)
    } finally {
      off()
    }
    expect(slotContributions('topbar.actions').map(c => c.id)).not.toContain('zz.top')
  })
})
