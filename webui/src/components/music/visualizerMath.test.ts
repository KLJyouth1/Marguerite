// @vitest-environment happy-dom
import { describe, expect, it } from 'vitest'
import {
  decayBeatPulse,
  ringSpecs,
  spawnParticles,
  spectrumBars,
  stepParticle,
} from './visualizerMath'

describe('visualizerMath — spawnParticles', () => {
  it('spawns the requested count within the normalised viewport', () => {
    const rnd = (() => { let i = 0; const xs = [0.1, 0.9, 0.4]; return () => xs[i++ % xs.length] })()
    const parts = spawnParticles(3, rnd)
    expect(parts).toHaveLength(3)
    for (const p of parts) {
      expect(p.x).toBeGreaterThanOrEqual(0)
      expect(p.x).toBeLessThanOrEqual(1)
      expect(p.y).toBeGreaterThanOrEqual(0)
      expect(p.y).toBeLessThanOrEqual(1)
      expect(p.speed).toBeGreaterThan(0)
      expect(p.size).toBeGreaterThan(0)
      expect(p.alpha).toBeGreaterThan(0)
    }
  })
})

describe('visualizerMath — stepParticle', () => {
  it('rises with the track level and wraps back to the bottom', () => {
    const p = { x: 0.5, y: 0.99, speed: 0.02, sway: 0.02, phase: 0, size: 1, alpha: 0.5 }
    const next = stepParticle(p, 1, 0, 0, 0)
    expect(next.y).toBeLessThan(p.y)
    // Floating off the top wraps to the bottom.
    const nearTop = stepParticle({ ...p, y: 0.0 }, 2, 0, 0, 0)
    expect(nearTop.y).toBeGreaterThan(1)
  })

  it('a beat pulse drags the mote horizontally toward centre', () => {
    const offCentre = { x: 0.0, y: 0.5, speed: 0.02, sway: 0, phase: 0, size: 1, alpha: 0.5 }
    const quiet = stepParticle(offCentre, 0.16, 0, 0, 0)
    const thumped = stepParticle(offCentre, 0.16, 0, 0, 1)
    expect(thumped.x).toBeGreaterThan(quiet.x)
  })
})

describe('visualizerMath — decayBeatPulse', () => {
  it('decays toward zero but never below it', () => {
    expect(decayBeatPulse(1, 0.1)).toBeCloseTo(0.78, 2)
    expect(decayBeatPulse(0, 0.1)).toBe(0)
    expect(decayBeatPulse(1, 10)).toBe(0)
  })
})

describe('visualizerMath — ringSpecs', () => {
  it('emits the requested ring count inside maxRadius', () => {
    const rings = ringSpecs(4, 0, 0, 300)
    expect(rings).toHaveLength(4)
    for (const r of rings) {
      expect(r.radius).toBeGreaterThan(0)
      expect(r.radius).toBeLessThanOrEqual(300)
      expect(r.alpha).toBeGreaterThan(0)
      expect(r.alpha).toBeLessThanOrEqual(1)
    }
  })

  it('bass breathing expands the outer ring and a beat flashes it brighter', () => {
    const quiet = ringSpecs(3, 0, 0, 300)
    const thumping = ringSpecs(3, 1, 1, 300)
    expect(thumping[2].radius).toBeGreaterThan(quiet[2].radius)
    expect(thumping[2].alpha).toBeGreaterThan(quiet[2].alpha)
  })
})

describe('visualizerMath — spectrumBars', () => {
  it('maps the analyser bytes to 0..1 bar lengths', () => {
    const spec = new Uint8Array(1024)
    spec[0] = 255
    const bars = spectrumBars(spec, 4)
    expect(bars).toHaveLength(4)
    expect(bars[0]).toBe(1)
    for (const b of bars) expect(b).toBeLessThanOrEqual(1)
  })

  it('handles empty or zero-bar input without throwing', () => {
    expect(spectrumBars(new Uint8Array(0), 4)).toEqual([0, 0, 0, 0])
    expect(spectrumBars(new Uint8Array(64), 0)).toEqual([])
  })
})
