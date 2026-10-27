// @vitest-environment happy-dom
import { describe, expect, it } from 'vitest'
import { formatClock, fractionToSeek } from './bgmFormat'

describe('bgmFormat — formatClock', () => {
  it('renders zero for missing / non-finite / negative input', () => {
    expect(formatClock(undefined)).toBe('0:00')
    expect(formatClock(null)).toBe('0:00')
    expect(formatClock(Number.NaN)).toBe('0:00')
    expect(formatClock(-12.4)).toBe('0:00')
    expect(formatClock(Number.POSITIVE_INFINITY)).toBe('0:00')
  })

  it('formats whole seconds as m:ss with zero-padded seconds', () => {
    expect(formatClock(0)).toBe('0:00')
    expect(formatClock(9)).toBe('0:09')
    expect(formatClock(59)).toBe('0:59')
    expect(formatClock(60)).toBe('1:00')
    expect(formatClock(125.9)).toBe('2:05')
    expect(formatClock(213.4)).toBe('3:33')
  })

  it('folds hours into minutes rather than emitting h:mm:ss', () => {
    expect(formatClock(3600)).toBe('60:00')
    expect(formatClock(5400)).toBe('90:00')
  })
})

describe('bgmFormat — fractionToSeek', () => {
  it('returns null when there is no usable timeline (synth pad / unknown duration)', () => {
    expect(fractionToSeek(0.5, 0)).toBeNull()
    expect(fractionToSeek(0.5, Number.NaN)).toBeNull()
    expect(fractionToSeek(0.5, -10)).toBeNull()
  })

  it('maps a 0..1 fraction into an absolute seek position', () => {
    expect(fractionToSeek(0, 200)).toBe(0)
    expect(fractionToSeek(0.5, 200)).toBe(100)
    expect(fractionToSeek(1, 200)).toBe(200)
  })

  it('clamps out-of-range fractions into [0, 1]', () => {
    expect(fractionToSeek(-0.5, 200)).toBe(0)
    expect(fractionToSeek(1.5, 200)).toBe(200)
  })
})
