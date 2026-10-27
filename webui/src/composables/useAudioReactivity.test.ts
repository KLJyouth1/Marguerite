// @vitest-environment happy-dom
import { describe, it, expect, beforeEach, vi } from 'vitest'

async function fresh() {
  vi.resetModules()
  return Promise.all([import('./useAudioReactivity'), import('./audioEngine')])
}

beforeEach(() => {
  vi.restoreAllMocks()
})

describe('useAudioReactivity', () => {
  it('audioTick() copies the engine frame into reactive refs', async () => {
    const [mod, engineMod] = await fresh()
    const readFrame = vi.spyOn(engineMod.audioEngine, 'readFrame').mockReturnValue({
      bass: 0.5,
      mid: 0.25,
      treble: 0.1,
      energy: 0.4,
      beat: true,
    })
    const spec = new Uint8Array(16).fill(200)
    vi.spyOn(engineMod.audioEngine, 'spectrum').mockReturnValue(spec)
    const wave = new Uint8Array(32).fill(128)
    vi.spyOn(engineMod.audioEngine, 'waveform').mockReturnValue(wave)

    mod.audioTick()

    const { bass, mid, treble, energy, beat, spectrum, waveform } = mod.useAudioReactivity()
    expect(bass.value).toBe(0.5)
    expect(mid.value).toBe(0.25)
    expect(treble.value).toBe(0.1)
    expect(energy.value).toBe(0.4)
    expect(beat.value).toBe(true)
    // Snapshot semantics: the ref holds a copy, not the engine's live buffer.
    expect(spectrum.value).not.toBe(spec)
    expect([...spectrum.value]).toEqual([...spec])
    expect(waveform.value.length).toBe(32)
    expect(readFrame).toHaveBeenCalledTimes(1)
  })

  it('start/stop are idempotent and no-op when unsupported', async () => {
    vi.stubGlobal('requestAnimationFrame', vi.fn(() => 1))
    vi.stubGlobal('cancelAnimationFrame', vi.fn())
    const [mod, engineMod] = await fresh()
    vi.spyOn(engineMod.audioEngine, 'supported').mockReturnValue(false)

    mod.startAudioReactivity()
    mod.startAudioReactivity()
    expect(requestAnimationFrame).not.toHaveBeenCalled()
    mod.stopAudioReactivity() // also safe to stop when never started
  })
})
