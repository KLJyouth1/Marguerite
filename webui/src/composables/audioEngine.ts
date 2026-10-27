import {
  bandEnergies,
  clamp,
  smooth,
  waveformRms,
  createBeatDetector,
  type BandEnergy,
} from './audioMath'

/**
 * Shared audio-analysis engine: the single Web Audio graph that taps both the
 * player's <audio> element (files / https streams / local blobs) and the
 * built-in synth pad, and exposes the resulting spectrum / waveform / band
 * energies to the reactivity loop.
 *
 * Design rules:
 * - The AudioContext is created lazily on the first unlock() — browsers block
 *   audio until a user gesture, so we never touch it at module import.
 * - A media element may only ever feed ONE MediaElementAudioSourceNode, so the
 *   attachment is memoised and survives repeated calls.
 * - Wherever Web Audio is unavailable (happy-dom in tests, locked-down
 *   WebViews) everything degrades to a silent no-op: callers read zeros and
 *   never throw.
 */

export interface AudioFrame {
  bass: number
  mid: number
  treble: number
  energy: number
  beat: boolean
}

const FFT_SIZE = 2048

function audioCtor(): typeof AudioContext | null {
  if (typeof window === 'undefined') return null
  const w = window as unknown as {
    AudioContext?: typeof AudioContext
    webkitAudioContext?: typeof AudioContext
  }
  return w.AudioContext || w.webkitAudioContext || null
}

class AudioEngine {
  private ctx: AudioContext | null = null
  private analyser: AnalyserNode | null = null
  private mediaSource: MediaElementAudioSourceNode | null = null
  private attachedElement: HTMLAudioElement | null = null
  private fftBuf: Uint8Array<ArrayBuffer> | null = null
  private waveBuf: Uint8Array<ArrayBuffer> | null = null
  private beatDetector = createBeatDetector()
  // Smoothed band levels kept on the engine so repeated readFrame() calls
  // between rAF ticks stay coherent.
  private smoothed: BandEnergy = { bass: 0, mid: 0, treble: 0 }
  private energy = 0

  /** False until Web Audio is confirmed working; callers may branch on it. */
  supported(): boolean {
    return !!this.ctx
  }

  /** The shared context, or null when Web Audio is unavailable. */
  context(): AudioContext | null {
    return this.ctx
  }

  private ensureContext(): boolean {
    if (this.ctx) return true
    const Ctor = audioCtor()
    if (!Ctor) return false
    try {
      this.ctx = new Ctor()
    } catch {
      this.ctx = null
      return false
    }
    try {
      this.analyser = this.ctx.createAnalyser()
      this.analyser.fftSize = FFT_SIZE
      this.analyser.smoothingTimeConstant = 0.5
      this.fftBuf = new Uint8Array(new ArrayBuffer(this.analyser.frequencyBinCount))
      this.waveBuf = new Uint8Array(new ArrayBuffer(FFT_SIZE))
      this.analyser.connect(this.ctx.destination)
    } catch {
      this.ctx = null
      this.analyser = null
      return false
    }
    return true
  }

  /**
   * Create/resume the context. Browsers require this to run from a user
   * gesture; calling it earlier or again is harmless. Resolves when the
   * context is running (or unavailable).
   */
  async unlock(): Promise<void> {
    if (!this.ensureContext()) return
    if (this.ctx!.state === 'suspended') {
      try {
        await this.ctx!.resume()
      } catch {
        /* stays suspended; reads keep returning zeros */
      }
    }
  }

  /** Tap the player's shared <audio> element. Idempotent per element. */
  attachMediaElement(el: HTMLAudioElement): void {
    if (!this.ensureContext() || !this.analyser) return
    if (this.attachedElement === el && this.mediaSource) return
    // A different element asking to be attached means the previous one went
    // away — only one source node can exist per element, so the old tap simply
    // stops receiving data; re-tapping the same element is illegal.
    if (this.attachedElement && this.attachedElement !== el) return
    try {
      this.mediaSource = this.ctx!.createMediaElementSource(el)
      this.mediaSource.connect(this.analyser)
      this.attachedElement = el
    } catch {
      /* already tapped by someone else — leave graph untouched */
    }
  }

  /** Route an external source (the synth pad) through the shared analyser. */
  connectExternalSource(node: AudioNode): void {
    if (!this.analyser) return
    try {
      node.connect(this.analyser)
    } catch {
      /* noop */
    }
  }

  /** Live byte-frequency spectrum, or null when unsupported. */
  spectrum(): Uint8Array | null {
    if (!this.analyser || !this.fftBuf) return null
    this.analyser.getByteFrequencyData(this.fftBuf)
    return this.fftBuf
  }

  /** Live time-domain waveform, or null when unsupported. */
  waveform(): Uint8Array | null {
    if (!this.analyser || !this.waveBuf) return null
    this.analyser.getByteTimeDomainData(this.waveBuf)
    return this.waveBuf
  }

  /**
   * Sample the graph once: band energies (smoothed), total RMS energy, and the
   * beat flag for this frame. Cheap enough to call from a single rAF loop.
   */
  readFrame(): AudioFrame {
    if (!this.analyser || !this.ctx || !this.fftBuf || !this.waveBuf) {
      return { bass: 0, mid: 0, treble: 0, energy: 0, beat: false }
    }
    this.analyser.getByteFrequencyData(this.fftBuf)
    this.analyser.getByteTimeDomainData(this.waveBuf)
    const raw = bandEnergies(this.fftBuf, this.ctx.sampleRate, this.analyser.fftSize)
    this.smoothed.bass = smooth(this.smoothed.bass, raw.bass, 0.45)
    this.smoothed.mid = smooth(this.smoothed.mid, raw.mid, 0.45)
    this.smoothed.treble = smooth(this.smoothed.treble, raw.treble, 0.45)
    this.energy = smooth(this.energy, waveformRms(this.waveBuf), 0.5)
    const beat = this.beatDetector.update(this.smoothed.bass)
    return {
      bass: clamp(this.smoothed.bass),
      mid: clamp(this.smoothed.mid),
      treble: clamp(this.smoothed.treble),
      energy: clamp(this.energy),
      beat,
    }
  }
}

/** App-wide singleton — exactly one graph, however many components mount. */
export const audioEngine = new AudioEngine()
