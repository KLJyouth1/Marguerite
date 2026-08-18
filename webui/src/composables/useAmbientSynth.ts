import { ref, type Ref } from 'vue'

/**
 * Procedurally-synthesised background ambience: a slow "pad" built from a few
 * slightly-detuned oscillators through a low-pass filter, with a slow LFO
 * swelling the cutoff. This replaces shipping an audio file — no asset is
 * bundled, so the official release gate (which forbids personal BGM inside the
 * dist) passes, and the pad works offline with zero copyright exposure.
 *
 * The AudioContext is created lazily on the first play(): browsers block audio
 * until a user gesture, and we never tear the context down on pause (we
 * suspend/resume instead) so resuming is instant and click-free. When the Web
 * Audio API is unavailable (older WebViews, locked-down runners) play() degrades
 * to a no-op and `available` stays false, so callers fall back to a silent state.
 */

export const BGM_SYNTH_TRACK_ID = '__synth__'
export const BGM_SYNTH_I18N_KEY = 'chrome.bgm.synth'

// Voices: a low root plus a fifth and octave, each a few cents off its neighbour
// so the faint beating reads as a living drone rather than a sterile tone.
interface VoiceSpec {
  type: OscillatorType
  freq: number
  detune: number
}
const VOICES: VoiceSpec[] = [
  { type: 'sine', freq: 110, detune: -4 }, // A2 root
  { type: 'triangle', freq: 164.81, detune: 3 }, // E3 fifth
  { type: 'sine', freq: 220, detune: 6 }, // A3 octave
]
const LFO_RATE = 0.07 // Hz -> ~14s swell period
const FILTER_BASE = 700 // Hz low-pass centre
const FILTER_SWING = 320 // Hz LFO modulation depth
const MASTER_LEVEL = 0.18 // headroom: a pad, never a lead

let ctx: AudioContext | null = null
let master: GainNode | null = null
let filter: BiquadFilterNode | null = null
let analyser: AnalyserNode | null = null
let analyserBuf: Uint8Array<ArrayBuffer> | null = null
let oscs: OscillatorNode[] = []
let lfo: OscillatorNode | null = null
let lfoGain: GainNode | null = null
let ampRaf = 0
let synthVolume = 0.6 // mirrors useBgm volume (0..1), scaled to MASTER_LEVEL

const playing = ref(false)
const amplitude = ref(0)
const available = ref(false)

function audioCtor(): typeof AudioContext | null {
  if (typeof window === 'undefined') return null
  const w = window as unknown as {
    AudioContext?: typeof AudioContext
    webkitAudioContext?: typeof AudioContext
  }
  return w.AudioContext || w.webkitAudioContext || null
}

function buildGraph(): boolean {
  const Ctor = audioCtor()
  if (!Ctor) {
    available.value = false
    return false
  }
  if (ctx) return true
  try {
    ctx = new Ctor()
  } catch {
    available.value = false
    return false
  }
  available.value = true

  master = ctx.createGain()
  master.gain.value = 0
  filter = ctx.createBiquadFilter()
  filter.type = 'lowpass'
  filter.frequency.value = FILTER_BASE
  filter.Q.value = 0.7
  analyser = ctx.createAnalyser()
  analyser.fftSize = 1024
  analyserBuf = new Uint8Array(new ArrayBuffer(analyser.fftSize))
  filter.connect(master)
  master.connect(analyser)
  analyser.connect(ctx.destination)

  for (const v of VOICES) {
    const o = ctx.createOscillator()
    o.type = v.type
    o.frequency.value = v.freq
    o.detune.value = v.detune
    const g = ctx.createGain()
    g.gain.value = 1 / VOICES.length
    o.connect(g)
    g.connect(filter)
    oscs.push(o)
  }
  // One LFO modulates the filter cutoff for a slow breathing timbre.
  lfo = ctx.createOscillator()
  lfo.frequency.value = LFO_RATE
  lfoGain = ctx.createGain()
  lfoGain.gain.value = FILTER_SWING
  lfo.connect(lfoGain)
  lfoGain.connect(filter.frequency)
  return true
}

function applyVolume(target: number, immediate = false): void {
  if (!ctx || !master) return
  const level = Math.max(0, Math.min(1, target)) * MASTER_LEVEL
  const now = ctx.currentTime
  if (immediate) master.gain.setValueAtTime(level, now)
  else master.gain.setTargetAtTime(level, now, 0.08)
}

function sampleAmplitude(): void {
  if (!analyser || !analyserBuf) {
    amplitude.value = 0
    return
  }
  analyser.getByteTimeDomainData(analyserBuf)
  let sum = 0
  for (let i = 0; i < analyserBuf.length; i++) {
    const v = (analyserBuf[i] - 128) / 128
    sum += v * v
  }
  const rms = Math.sqrt(sum / analyserBuf.length)
  // Scale + smooth: the pad's quiet RMS reads as a gentle 0..1 swell.
  const targetLevel = Math.min(1, rms * 3.4)
  amplitude.value += (targetLevel - amplitude.value) * 0.25
  ampRaf = requestAnimationFrame(sampleAmplitude)
}

export function useAmbientSynth() {
  function play(): Promise<boolean> {
    if (!buildGraph()) return Promise.resolve(false)
    const resume = ctx!.resume()
    for (const o of oscs) {
      try {
        o.start()
      } catch {
        /* already started — reused across suspend/resume */
      }
    }
    try {
      lfo!.start()
    } catch {
      /* already started */
    }
    applyVolume(synthVolume)
    playing.value = true
    cancelAnimationFrame(ampRaf)
    ampRaf = requestAnimationFrame(sampleAmplitude)
    const settled = resume && typeof (resume as Promise<void>).then === 'function'
      ? (resume as Promise<void>).catch(() => {})
      : Promise.resolve()
    return settled
      .then(() => true)
      .catch(() => {
        playing.value = false
        return false
      })
  }

  function pause(): void {
    playing.value = false
    amplitude.value = 0
    cancelAnimationFrame(ampRaf)
    ampRaf = 0
    if (ctx) {
      try {
        void ctx.suspend()
      } catch {
        /* noop */
      }
    }
  }

  function setVolume(v: number): void {
    synthVolume = Math.max(0, Math.min(1, v))
    if (playing.value) applyVolume(synthVolume)
  }

  function dispose(): void {
    cancelAnimationFrame(ampRaf)
    try {
      oscs.forEach((o) => o.stop())
    } catch {
      /* noop */
    }
    try {
      lfo?.stop()
    } catch {
      /* noop */
    }
    try {
      ctx?.close()
    } catch {
      /* noop */
    }
    ctx = null
    master = null
    filter = null
    analyser = null
    analyserBuf = null
    oscs = []
    lfo = null
    lfoGain = null
    playing.value = false
    amplitude.value = 0
  }

  return {
    playing: playing as Ref<boolean>,
    amplitude: amplitude as Ref<number>,
    available: available as Ref<boolean>,
    play,
    pause,
    setVolume,
    dispose,
  }
}
