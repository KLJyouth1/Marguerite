// @vitest-environment happy-dom
import { describe, it, expect, beforeEach, vi } from 'vitest'

// The synth pad is a procedurally-generated ambient drone (no audio file is
// ever bundled). It must degrade gracefully where the Web Audio API is missing
// or blocked — older WebViews, locked-down CI runners, and jsdom/happy-dom
// without an AudioContext. In that case play() is a no-op, `available` stays
// false, and `amplitude` stays 0, so callers fall back to a silent state
// instead of throwing. (happy-dom does not implement AudioContext; we also
// stub it away to make the degradation deterministic across environments.)
describe('useAmbientSynth — graceful degradation', () => {
  beforeEach(() => {
    vi.restoreAllMocks()
    vi.resetModules()
    // Force audioCtor() to find no constructor.
    vi.stubGlobal('AudioContext', undefined)
    const w = window as unknown as { AudioContext?: unknown; webkitAudioContext?: unknown }
    try { w.AudioContext = undefined } catch { /* read-only in some envs */ }
    try { w.webkitAudioContext = undefined } catch { /* read-only in some envs */ }
  })

  it('exposes the synth track id and i18n key', async () => {
    const mod = await import('./useAmbientSynth')
    expect(mod.BGM_SYNTH_TRACK_ID).toBe('__synth__')
    expect(mod.BGM_SYNTH_I18N_KEY).toBe('chrome.bgm.synth')
  })

  it('degrades to a no-op when Web Audio is unavailable', async () => {
    const { useAmbientSynth } = await import('./useAmbientSynth')
    const synth = useAmbientSynth()

    expect(synth.available.value).toBe(false)
    const ok = await synth.play()
    expect(ok).toBe(false)
    expect(synth.playing.value).toBe(false)
    expect(synth.amplitude.value).toBe(0)

    // None of these should throw on the degraded path.
    expect(() => synth.setVolume(0.5)).not.toThrow()
    expect(() => synth.pause()).not.toThrow()
    expect(() => synth.dispose()).not.toThrow()
  })

  it('keeps amplitude at zero before any playback', async () => {
    const { useAmbientSynth } = await import('./useAmbientSynth')
    const synth = useAmbientSynth()
    expect(typeof synth.amplitude.value).toBe('number')
    expect(synth.amplitude.value).toBe(0)
    // Volume changes must not surface a non-zero level without a running graph.
    synth.setVolume(0.8)
    expect(synth.amplitude.value).toBe(0)
  })
})
