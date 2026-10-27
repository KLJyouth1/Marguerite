/**
 * Pure audio-analysis primitives shared by the Web Audio engine
 * (`audioEngine.ts`) and the reactivity loop (`useAudioReactivity.ts`).
 * No Vue, no Web Audio, no module state here — everything is a plain function
 * so it can be unit-tested in isolation with hand-crafted buffers.
 */

export function clamp(v: number, min = 0, max = 1): number {
  return Math.min(max, Math.max(min, v))
}

/** Wrap an index into [0, length) — mirrors modulo but tolerant of negatives. */
export function wrapIndex(i: number, length: number): number {
  if (length <= 0) return 0
  return ((i % length) + length) % length
}

export interface BandEnergy {
  bass: number
  mid: number
  treble: number
}

// Perceptual bands used for the particle-field / visualisation source data.
// Frequencies above 8 kHz add little to a desktop ambient track, so treble
// tops out there instead of up to the Nyquist limit.
const BASS_RANGE: [number, number] = [20, 250]
const MID_RANGE: [number, number] = [250, 2000]
const TREBLE_RANGE: [number, number] = [2000, 8000]

/**
 * Average a byte-frequency buffer (0..255) over a frequency band, returning a
 * perceptual 0..1 level. `analyser.frequencyBinCount` entries map linearly to
 * 0..(sampleRate / 2); bin `i` covers `i * (sampleRate / fftSize)` Hz.
 */
export function bandEnergies(
  spectrum: Uint8Array,
  sampleRate: number,
  fftSize: number,
): BandEnergy {
  if (!spectrum.length || sampleRate <= 0 || fftSize <= 0) {
    return { bass: 0, mid: 0, treble: 0 }
  }
  const hzPerBin = sampleRate / fftSize
  const over = (range: [number, number]): number => {
    const lo = Math.max(0, Math.floor(range[0] / hzPerBin))
    const hi = Math.min(spectrum.length - 1, Math.ceil(range[1] / hzPerBin))
    if (hi < lo) return 0
    let sum = 0
    for (let i = lo; i <= hi; i++) sum += spectrum[i]
    const avg = sum / (hi - lo + 1) / 255
    // sqrt curve: byte magnitudes are already near-log, but a light gamma makes
    // quiet pads breathe instead of hugging zero.
    return Math.sqrt(clamp(avg))
  }
  return { bass: over(BASS_RANGE), mid: over(MID_RANGE), treble: over(TREBLE_RANGE) }
}

/**
 * RMS level of a time-domain byte buffer where 128 is silence, scaled to 0..1.
 */
export function waveformRms(wave: Uint8Array): number {
  if (!wave.length) return 0
  let sum = 0
  for (let i = 0; i < wave.length; i++) {
    const v = (wave[i] - 128) / 128
    sum += v * v
  }
  return Math.sqrt(sum / wave.length)
}

/** Exponential one-pole smoothing: `current` chases `target` by `alpha` per frame. */
export function smooth(current: number, target: number, alpha: number): number {
  const k = clamp(alpha, 0, 1)
  return current + (target - current) * k
}

/**
 * Fisher–Yates shuffle of `[0, length)` — the deterministic ordering used by
 * the player's shuffle mode. Injectable RNG keeps tests reproducible.
 */
export function shuffledIndices(length: number, rnd: () => number = Math.random): number[] {
  const order = Array.from({ length }, (_, i) => i)
  for (let i = order.length - 1; i > 0; i--) {
    const j = Math.floor(rnd() * (i + 1))
    ;[order[i], order[j]] = [order[j], order[i]]
  }
  return order
}

/**
 * Rotate the track list for next/prev or natural end-of-track advance.
 * `order` is the shuffle ordering (its length must equal `ids.length`, empty =
 * linear order). Returns the target id, or null when walking past a boundary
 * without wrap. A currentId that is not in the list (e.g. a session-only local
 * file) resolves to the first/last entry in the requested direction.
 */
export function rotateTrackId(
  ids: string[],
  currentId: string,
  direction: 1 | -1,
  order: string[],
  wrap: boolean,
): string | null {
  if (!ids.length) return null
  const pool = order.length === ids.length ? order : ids
  const idx = pool.indexOf(currentId)
  // Unknown current id (e.g. a session-only local file): forward starts from
  // the top, backward from the bottom.
  let next = idx < 0 ? (direction === 1 ? 0 : pool.length - 1) : idx + direction
  if (next < 0 || next >= pool.length) {
    if (!wrap) return null
    next = wrapIndex(next, pool.length)
  }
  return pool[next]
}

export interface BeatDetectorOptions {
  /** Frames of bass history compared against the current frame. */
  historySize?: number
  /** Current bass energy must exceed the history average by this factor. */
  sensitivity?: number
  /** Minimum frames between two accepted beats (prevents double-triggers). */
  cooldown?: number
  /** Ignore frames quieter than this floor (silence is never a beat). */
  minLevel?: number
}

export interface BeatDetector {
  /** Feed one bass-energy sample (0..1); returns true exactly on a detected beat. */
  update: (bass: number) => boolean
}

/**
 * Simple energy-flux beat detector: a frame counts as a beat when its bass
 * energy meaningfully exceeds the recent running average (the classic
 * "fast energy localisation" heuristic), gated by a cooldown and a noise
 * floor. Stateful on purpose — kept in a closure so it stays easy to test.
 */
export function createBeatDetector(options: BeatDetectorOptions = {}): BeatDetector {
  const historySize = clamp(options.historySize ?? 43, 4, 256)
  const sensitivity = clamp(options.sensitivity ?? 1.35, 1, 3)
  const cooldown = Math.max(0, Math.floor(options.cooldown ?? 8))
  const minLevel = clamp(options.minLevel ?? 0.08, 0, 1)

  const history: number[] = []
  let sinceBeat = cooldown

  return {
    update(bass: number): boolean {
      const v = clamp(bass)
      let beat = false
      if (history.length >= historySize) {
        const avg = history.reduce((a, b) => a + b, 0) / history.length
        if (sinceBeat >= cooldown && v >= minLevel && v > avg * sensitivity) {
          beat = true
          sinceBeat = 0
        }
      }
      history.push(v)
      while (history.length > historySize) history.shift()
      sinceBeat++
      return beat
    },
  }
}
