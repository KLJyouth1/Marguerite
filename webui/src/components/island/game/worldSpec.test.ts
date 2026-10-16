import { describe, it, expect } from 'vitest'
import {
  defaultWorldSpec,
  validateWorldSpec,
  specFromDescription,
  hashSeed,
  WEATHER_KINDS,
} from './worldSpec'

describe('validateWorldSpec', () => {
  it('accepts the default world (MVP baseline has 3 core regions)', () => {
    const errors = validateWorldSpec(defaultWorldSpec())
    expect(errors).toEqual([])
  })

  it('rejects a non-object world', () => {
    expect(validateWorldSpec(null).length).toBeGreaterThan(0)
    expect(validateWorldSpec('nope').length).toBeGreaterThan(0)
  })

  it('rejects invalid enum values and bad names', () => {
    const w = {
      ...defaultWorldSpec(),
      weather: 'sandstorm',
      theme: 'neon',
      islandName: 'a'.repeat(80),
    }
    const errors = validateWorldSpec(w)
    expect(errors.join('\n')).toMatch(/weather/)
    expect(errors.join('\n')).toMatch(/theme/)
    expect(errors.join('\n')).toMatch(/islandName/)
  })

  it('requires at least 3 regions and unique ids', () => {
    const w = { ...defaultWorldSpec(), regions: defaultWorldSpec().regions.slice(0, 2) }
    expect(validateWorldSpec(w).join('\n')).toMatch(/regions/)
  })

  it('detects duplicate quest ids', () => {
    const w = defaultWorldSpec()
    w.initialQuests = [
      { id: 'x', label: 'a', difficulty: 1, expReward: 10 },
      { id: 'x', label: 'b', difficulty: 2, expReward: 20 },
    ]
    expect(validateWorldSpec(w).join('\n')).toMatch(/duplicate/)
  })
})

describe('specFromDescription (interactive generation closed loop)', () => {
  it('is deterministic for the same text', () => {
    const a = specFromDescription('rainy blossom hard dungeon')
    const b = specFromDescription('rainy blossom hard dungeon')
    expect(a).toEqual(b)
    expect(a.seed).toBe(hashSeed('rainy blossom hard dungeon'))
  })

  it('derives weather/theme/difficulty from keywords', () => {
    const rain = specFromDescription('a rainy night fog island hard')
    expect(WEATHER_KINDS).toContain(rain.weather)
    const blossom = specFromDescription('樱花花见 blossom')
    expect(blossom.theme).toBe('blossom')
    const hard = specFromDescription('大师 hard 3星 dungeon')
    expect(hard.initialQuests[1].difficulty).toBe(3)
  })

  it('always produces a valid spec (validation passes)', () => {
    for (const text of ['', '  ', '花之岛', 'a rainy blossom hard dungeon', 'night spire']) {
      expect(validateWorldSpec(specFromDescription(text))).toEqual([])
    }
  })
})
