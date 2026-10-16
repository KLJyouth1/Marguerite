import { describe, expect, it } from 'vitest'

import {
  announceLevel,
  hazardIcon,
  targetSizeOk,
  MIN_TARGET_SIZE,
} from './useA11y'

describe('announceLevel (aria-live politeness)', () => {
  it('routes approvals, errors and alerts to assertive', () => {
    expect(announceLevel('approval')).toBe('assertive')
    expect(announceLevel('error')).toBe('assertive')
    expect(announceLevel('alert')).toBe('assertive')
  })

  it('routes ordinary completions and neutral updates to polite', () => {
    expect(announceLevel('success')).toBe('polite')
    expect(announceLevel('info')).toBe('polite')
    expect(announceLevel('status')).toBe('polite')
  })
})

describe('targetSizeOk (WCAG 2.5.8, 24×24 floor)', () => {
  it('pins the floor constant to 24', () => {
    expect(MIN_TARGET_SIZE).toBe(24)
  })

  it('passes exactly at the 24×24 boundary', () => {
    expect(targetSizeOk(24, 24)).toBe(true)
  })

  it('rejects anything below 24 in either dimension', () => {
    expect(targetSizeOk(23, 24)).toBe(false)
    expect(targetSizeOk(24, 23)).toBe(false)
    expect(targetSizeOk(20, 20)).toBe(false)
  })

  it('accepts targets larger than the floor', () => {
    expect(targetSizeOk(44, 44)).toBe(true)
    expect(targetSizeOk(24, 48)).toBe(true)
    expect(targetSizeOk(48, 24)).toBe(true)
  })

  it('rejects non-positive and non-finite dimensions', () => {
    expect(targetSizeOk(0, 0)).toBe(false)
    expect(targetSizeOk(-8, 30)).toBe(false)
    expect(targetSizeOk(Number.NaN, 24)).toBe(false)
    expect(targetSizeOk(24, Number.POSITIVE_INFINITY)).toBe(false)
  })
})

describe('hazardIcon (icon + text, never colour alone)', () => {
  it('maps L0–L3 to distinct icon tokens', () => {
    expect(hazardIcon('L0').icon).toBe('check-circle')
    expect(hazardIcon('L1').icon).toBe('info')
    expect(hazardIcon('L2').icon).toBe('alert-triangle')
    expect(hazardIcon('L3').icon).toBe('alert-octagon')
  })

  it('gives every band a non-empty human-readable label', () => {
    for (const level of ['L0', 'L1', 'L2', 'L3'] as const) {
      const glyph = hazardIcon(level)
      expect(glyph.label.length).toBeGreaterThan(0)
    }
  })

  it('uses a unique icon per band so status never collapses to colour', () => {
    const icons = (['L0', 'L1', 'L2', 'L3'] as const).map((l) => hazardIcon(l).icon)
    expect(new Set(icons).size).toBe(4)
  })
})
