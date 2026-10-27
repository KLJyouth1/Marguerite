// @vitest-environment happy-dom
import { describe, it, expect, beforeEach, vi } from 'vitest'

// audioEngine is a module-level singleton, so each test re-imports a fresh
// module with its own (fake) AudioContext graph.

class FakeAnalyser {
  fftSize = 2048
  frequencyBinCount = 1024
  smoothingTimeConstant = 0
  connectedTo: AudioNode[] = []
  freqData = new Uint8Array(1024)
  waveData = new Uint8Array(2048)
  connect(node: AudioNode) {
    this.connectedTo.push(node)
  }
  getByteFrequencyData(buf: Uint8Array) {
    for (let i = 0; i < buf.length; i++) buf[i] = this.freqData[i] ?? 0
  }
  getByteTimeDomainData(buf: Uint8Array) {
    for (let i = 0; i < buf.length; i++) buf[i] = this.waveData[i] ?? 128
  }
}

class FakeContext {
  state: 'running' | 'suspended' = 'suspended'
  sampleRate = 44100
  destination = {} as AudioDestinationNode
  analyser = new FakeAnalyser()
  mediaSources: AudioNode[] = []
  resumed = 0
  async resume() {
    this.resumed += 1
    this.state = 'running'
  }
  createAnalyser() {
    return this.analyser as unknown as AnalyserNode
  }
  createMediaElementSource(_el: HTMLAudioElement) {
    const node = { connectedTo: [] as AudioNode[], connect(n: AudioNode) { this.connectedTo.push(n) } }
    this.mediaSources.push(node as unknown as AudioNode)
    return node as unknown as MediaElementAudioSourceNode
  }
}

async function freshEngine() {
  vi.resetModules()
  return import('./audioEngine')
}

beforeEach(() => {
  vi.restoreAllMocks()
  vi.resetModules()
  // Default: no Web Audio anywhere.
  vi.stubGlobal('AudioContext', undefined)
  const w = window as unknown as Record<string, unknown>
  try { w.AudioContext = undefined } catch { /* read-only */ }
  try { w.webkitAudioContext = undefined } catch { /* read-only */ }
})

describe('audioEngine — graceful degradation', () => {
  it('is unsupported and silent when Web Audio is missing', async () => {
    const { audioEngine } = await freshEngine()
    expect(audioEngine.supported()).toBe(false)
    await audioEngine.unlock()
    expect(audioEngine.readFrame()).toEqual({
      bass: 0,
      mid: 0,
      treble: 0,
      energy: 0,
      beat: false,
    })
    expect(audioEngine.spectrum()).toBeNull()
    expect(audioEngine.waveform()).toBeNull()
    // Must not throw on an unsupported path.
    expect(() => audioEngine.attachMediaElement(new Audio())).not.toThrow()
    expect(() =>
      audioEngine.connectExternalSource({ connect() {} } as unknown as AudioNode),
    ).not.toThrow()
  })
})

describe('audioEngine — happy path with a fake graph', () => {
  function installFakeContext() {
    const ctx = new FakeContext()
    vi.stubGlobal('AudioContext', function () {
      return ctx as unknown as AudioContext
    })
    return ctx
  }

  it('builds the graph on unlock and taps the media element once', async () => {
    const ctx = installFakeContext()
    const { audioEngine } = await freshEngine()
    await audioEngine.unlock()
    expect(audioEngine.supported()).toBe(true)
    expect(ctx.resumed).toBeGreaterThanOrEqual(1)

    const el = new Audio()
    audioEngine.attachMediaElement(el)
    expect(ctx.mediaSources).toHaveLength(1)
    // Tapping the same element again must not create a second source.
    audioEngine.attachMediaElement(el)
    expect(ctx.mediaSources).toHaveLength(1)
  })

  it('does not re-tap a different element after the first', async () => {
    const ctx = installFakeContext()
    const { audioEngine } = await freshEngine()
    await audioEngine.unlock()
    audioEngine.attachMediaElement(new Audio())
    const before = ctx.mediaSources.length
    audioEngine.attachMediaElement(new Audio())
    expect(ctx.mediaSources).toHaveLength(before)
  })

  it('readFrame() reports bass energy from the analyser spectrum', async () => {
    const ctx = installFakeContext()
    const { audioEngine } = await freshEngine()
    await audioEngine.unlock()
    // Strong low bins (~100 Hz) → bass band lights up; mid/treble stay dark.
    for (let i = 1; i <= 5; i++) ctx.analyser.freqData[i] = 255
    // Sine-ish waveform: alternating 0/255 around 128.
    for (let i = 0; i < 2048; i++) ctx.analyser.waveData[i] = i % 2 === 0 ? 200 : 60

    // Two frames: smoothing means the first chases the target.
    audioEngine.readFrame()
    const frame = audioEngine.readFrame()
    expect(frame.bass).toBeGreaterThan(0.2)
    expect(frame.mid).toBeLessThan(0.01)
    expect(frame.treble).toBeLessThan(0.01)
    expect(frame.energy).toBeGreaterThan(0.05)
  })
})
