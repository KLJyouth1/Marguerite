// @vitest-environment happy-dom
import { describe, it, expect } from 'vitest'
import {
  clamp,
  wrapIndex,
  bandEnergies,
  waveformRms,
  smooth,
  shuffledIndices,
  rotateTrackId,
  createBeatDetector,
} from './audioMath'

describe('audioMath — clamp / wrapIndex', () => {
  it('clamps to the default 0..1 range', () => {
    expect(clamp(-0.5)).toBe(0)
    expect(clamp(1.4)).toBe(1)
    expect(clamp(0.3)).toBe(0.3)
  })

  it('clamps to a custom range', () => {
    expect(clamp(-5, -2, 4)).toBe(-2)
    expect(clamp(9, -2, 4)).toBe(4)
    expect(clamp(1, -2, 4)).toBe(1)
  })

  it('wraps indices, including negatives', () => {
    expect(wrapIndex(0, 5)).toBe(0)
    expect(wrapIndex(5, 5)).toBe(0)
    expect(wrapIndex(-1, 5)).toBe(4)
    expect(wrapIndex(-6, 5)).toBe(4)
    expect(wrapIndex(12, 5)).toBe(2)
    expect(wrapIndex(3, 0)).toBe(0)
  })
})

describe('audioMath — bandEnergies', () => {
  const SAMPLE_RATE = 44100
  const FFT_SIZE = 2048 // → 1024 bins, bin width ≈ 21.5 Hz

  function spectrumWith(binValues: Map<number, number>): Uint8Array {
    const bins = new Uint8Array(1024)
    for (const [bin, value] of binValues) bins[bin] = value
    return bins
  }

  it('returns zeros for empty input', () => {
    expect(bandEnergies(new Uint8Array(0), SAMPLE_RATE, FFT_SIZE)).toEqual({
      bass: 0,
      mid: 0,
      treble: 0,
    })
  })

  it('maps a low-frequency bin into bass only', () => {
    // bin 5 ≈ 107 Hz → inside 20..250
    const bands = bandEnergies(spectrumWith(new Map([[5, 255]])), SAMPLE_RATE, FFT_SIZE)
    expect(bands.bass).toBeGreaterThan(0.25)
    expect(bands.mid).toBe(0)
    expect(bands.treble).toBe(0)
  })

  it('maps a 440 Hz bin into mid only', () => {
    // bin 20 ≈ 430 Hz → inside 250..2000
    const bands = bandEnergies(spectrumWith(new Map([[20, 200]])), SAMPLE_RATE, FFT_SIZE)
    expect(bands.mid).toBeGreaterThan(0.03)
    expect(bands.bass).toBe(0)
    expect(bands.treble).toBe(0)
  })

  it('maps a 4 kHz bin into treble only', () => {
    // bin 186 ≈ 4000 Hz → inside 2000..8000
    const bands = bandEnergies(spectrumWith(new Map([[186, 220]])), SAMPLE_RATE, FFT_SIZE)
    expect(bands.treble).toBeGreaterThan(0.02)
    expect(bands.bass).toBe(0)
    expect(bands.mid).toBe(0)
  })

  it('treats content above 8 kHz as out of the treble band', () => {
    // bin 500 ≈ 10.7 kHz → above the treble ceiling
    const bands = bandEnergies(spectrumWith(new Map([[500, 255]])), SAMPLE_RATE, FFT_SIZE)
    expect(bands.treble).toBe(0)
  })
})

describe('audioMath — waveformRms / smooth', () => {
  it('reads silence as zero', () => {
    const flat = new Uint8Array(64).fill(128)
    expect(waveformRms(flat)).toBe(0)
  })

  it('measures a full-scale square-ish wave', () => {
    const wave = new Uint8Array(64)
    for (let i = 0; i < 64; i++) wave[i] = i % 2 === 0 ? 255 : 0
    // ±1 samples at equal energy → RMS ≈ 1.
    expect(waveformRms(wave)).toBeCloseTo(1, 1)
  })

  it('smooth() chases the target by alpha', () => {
    expect(smooth(0, 1, 0.5)).toBeCloseTo(0.5)
    expect(smooth(0, 1, 0.25)).toBeCloseTo(0.25)
    // Repeated application approaches without overshooting.
    let v = 0
    for (let i = 0; i < 10; i++) v = smooth(v, 1, 0.5)
    expect(v).toBeGreaterThan(0.99)
  })
})

describe('audioMath — shuffledIndices', () => {
  it('is a permutation of 0..n-1', () => {
    const order = shuffledIndices(6, () => 0.5)
    expect([...order].sort()).toEqual([0, 1, 2, 3, 4, 5])
  })

  it('is deterministic with an injected RNG', () => {
    const makeRnd = () => {
      let seed = 42
      return () => {
        seed = (seed * 16807) % 2147483647
        return seed / 2147483647
      }
    }
    expect(shuffledIndices(8, makeRnd())).toEqual(shuffledIndices(8, makeRnd()))
  })

  it('handles tiny lengths', () => {
    expect(shuffledIndices(0)).toEqual([])
    expect(shuffledIndices(1)).toEqual([0])
  })
})

describe('audioMath — rotateTrackId', () => {
  const IDS = ['a', 'b', 'c']

  it('walks linearly forward and backward', () => {
    expect(rotateTrackId(IDS, 'a', 1, [], true)).toBe('b')
    expect(rotateTrackId(IDS, 'b', -1, [], true)).toBe('a')
  })

  it('wraps only when asked', () => {
    expect(rotateTrackId(IDS, 'c', 1, [], false)).toBeNull()
    expect(rotateTrackId(IDS, 'c', 1, [], true)).toBe('a')
    expect(rotateTrackId(IDS, 'a', -1, [], false)).toBeNull()
    expect(rotateTrackId(IDS, 'a', -1, [], true)).toBe('c')
  })

  it('follows the shuffle order when one is supplied', () => {
    const order = ['c', 'a', 'b']
    expect(rotateTrackId(IDS, 'c', 1, order, false)).toBe('a')
    expect(rotateTrackId(IDS, 'b', 1, order, false)).toBeNull() // end of order, no wrap
    expect(rotateTrackId(IDS, 'b', 1, order, true)).toBe('c')
  })

  it('resolves an unknown current id to first/last in direction', () => {
    expect(rotateTrackId(IDS, '__local__', 1, [], true)).toBe('a')
    expect(rotateTrackId(IDS, '__local__', -1, [], true)).toBe('c')
  })
})

describe('audioMath — createBeatDetector', () => {
  it('never beats on silence', () => {
    const beat = createBeatDetector({ historySize: 8, cooldown: 2 })
    let sawBeat = false
    for (let i = 0; i < 100; i++) sawBeat = beat.update(0) || sawBeat
    expect(sawBeat).toBe(false)
  })

  it('fires on a sudden bass spike after the history fills', () => {
    const beat = createBeatDetector({ historySize: 10, sensitivity: 1.4, cooldown: 3 })
    // Quiet baseline.
    for (let i = 0; i < 15; i++) beat.update(0.05)
    // Spike well above average.
    expect(beat.update(0.9)).toBe(true)
  })

  it('cooldown blocks immediate re-triggers', () => {
    const beat = createBeatDetector({ historySize: 10, sensitivity: 1.1, cooldown: 5 })
    for (let i = 0; i < 15; i++) beat.update(0.1)
    expect(beat.update(0.9)).toBe(true)
    for (let i = 0; i < 4; i++) expect(beat.update(0.9)).toBe(false)
  })
})
