import { ref } from 'vue'
import { audioEngine } from './audioEngine'

/**
 * Bridges the engine's imperative reads into Vue reactive refs, behind ONE
 * shared requestAnimationFrame loop (started at most once app-wide).
 *
 * Why a single loop: every visualiser component reads the same audio state;
 * polling the analyser more than once per frame just wastes CPU and makes the
 * numbers disagree. Components subscribe to the refs here instead.
 *
 * The loop is lazy: `startAudioReactivity()` (called from the BGM bootstrap)
 * starts it only when Web Audio is actually available. Without it, refs stay
 * at their silent defaults, which keeps visualisers inert rather than broken.
 */

const emptySpectrum = new Uint8Array(0)
const emptyWaveform = new Uint8Array(0)

const spectrum = ref<Uint8Array>(emptySpectrum)
const waveform = ref<Uint8Array>(emptyWaveform)
const bass = ref(0)
const mid = ref(0)
const treble = ref(0)
const energy = ref(0)
/** True for exactly the frame on which a beat was detected (edge-triggered). */
const beat = ref(false)

let raf = 0
let running = false

/** One engine sample → one reactive update. Exported for unit tests. */
export function audioTick(): void {
  const frame = audioEngine.readFrame()
  bass.value = frame.bass
  mid.value = frame.mid
  treble.value = frame.treble
  energy.value = frame.energy
  beat.value = frame.beat
  const spec = audioEngine.spectrum()
  spectrum.value = spec ? spec.slice() : emptySpectrum
  const wave = audioEngine.waveform()
  waveform.value = wave ? wave.slice() : emptyWaveform
}

function loop(): void {
  if (!running) return
  audioTick()
  raf = requestAnimationFrame(loop)
}

/** Idempotent: starts the single shared loop. No-op when unsupported. */
export function startAudioReactivity(): void {
  if (running || !audioEngine.supported()) return
  running = true
  raf = requestAnimationFrame(loop)
}

/** Stops the loop (idempotent). Mostly for tests / teardown. */
export function stopAudioReactivity(): void {
  running = false
  if (raf) cancelAnimationFrame(raf)
  raf = 0
}

export function useAudioReactivity() {
  return {
    spectrum,
    waveform,
    bass,
    mid,
    treble,
    energy,
    beat,
    start: startAudioReactivity,
    stop: stopAudioReactivity,
  }
}
