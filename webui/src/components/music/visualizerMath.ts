/**
 * Pure visualisation math for the music workbench's constellation/aurora canvas.
 *
 * Everything here is deliberately Vue-free and Web-Audio-free: particle motion,
 * ring geometry and energy mapping are plain functions of numbers, so the
 * draw loop in `MusicVisualizer.vue` stays thin and the tuned parameters can be
 * unit-tested with hand-crafted audio frames.
 */

import { clamp } from '../../composables/audioMath'

/** A single dust-mote in the constellation field. Coordinates are 0..1
 * normalised (x across, y down); the draw layer multiplies by the canvas size. */
export interface VizParticle {
  x: number
  y: number
  /** base upward speed, in 0..1 height-fractions per second. */
  speed: number
  /** horizontal sway amplitude (fraction of width). */
  sway: number
  /** sway phase offset (radians) so motes don't bob in lockstep. */
  phase: number
  /** base point radius in CSS pixels. */
  size: number
  /** base opacity 0..1. */
  alpha: number
}

export interface VizFrame {
  bass: number
  mid: number
  treble: number
  energy: number
  /** edge-triggered beat: true only on the frame the beat fires. */
  beat: boolean
}

/** Spawn `count` motes scattered across the viewport. Injectable RNG keeps the
 *  test suite deterministic. */
export function spawnParticles(count: number, rnd: () => number = Math.random): VizParticle[] {
  const out: VizParticle[] = []
  for (let i = 0; i < count; i++) {
    out.push({
      x: rnd(),
      // Distribute motes across the full height; they wrap from bottom to top
      // as they drift upward, so no column ever looks empty on startup.
      y: rnd(),
      speed: 0.012 + rnd() * 0.03,
      sway: 0.01 + rnd() * 0.03,
      phase: rnd() * Math.PI * 2,
      size: 0.8 + rnd() * 1.8,
      alpha: 0.25 + rnd() * 0.5,
    })
  }
  return out
}

/**
 * Advance one mote by `dt` seconds. Rising speed scales with `level`
 * (mid+treble energy); a fresh `beatPulse` (0..1, see decayBeatPulse) drags the
 * mote toward the screen centre — the Hero-MV "particles converge" cue — and it
 * relaxes outward again as the pulse decays.
 */
export function stepParticle(
  p: VizParticle,
  dt: number,
  time: number,
  level: number,
  beatPulse: number,
): VizParticle {
  const speed = p.speed * (1 + level * 1.6)
  let y = p.y - speed * dt
  // Wrap: a mote that floats off the top re-enters at the bottom.
  if (y < -0.02) y = 1.02
  // Beat contraction: pull x toward the horizontal centre, eased by pulse.
  const contract = beatPulse * 0.35
  const centrePull = 0.5 - p.x
  const x = p.x + Math.sin(time * 0.6 + p.phase) * p.sway * dt + centrePull * contract * dt * 4
  return { ...p, x: clamp(x, -0.05, 1.05), y }
}

/**
 * Exponentially decay a beat pulse (1 the frame after a beat, 0 at rest).
 * `pulse` should be set to 1 when a beat is detected and eased here per frame
 * so the visual sting outlives the detection frame.
 */
export function decayBeatPulse(pulse: number, dt: number): number {
  return Math.max(0, pulse - dt * 2.2)
}

export interface RingSpec {
  /** radius in pixels. */
  radius: number
  /** stroke opacity 0..1. */
  alpha: number
  /** stroke width in pixels. */
  width: number
}

/**
 * Concentric radar tunnel rings for `ringCount` rings inside `maxRadius` px.
 * Bass breathing expands the radii and lifts the glow; a live beat pulse adds
 * an outer-ring flash.
 */
export function ringSpecs(
  ringCount: number,
  bass: number,
  beatPulse: number,
  maxRadius: number,
): RingSpec[] {
  const rings: RingSpec[] = []
  const spread = maxRadius / ringCount
  for (let i = 0; i < ringCount; i++) {
    const t = (i + 1) / ringCount
    const expand = 1 + bass * 0.06
    const radius = spread * i * expand + 8
    const outer = i === ringCount - 1 ? beatPulse : 0
    rings.push({
      radius,
      alpha: clamp(0.10 + t * 0.22 + bass * 0.18 + outer * 0.5),
      width: 1 + outer * 1.2,
    })
  }
  return rings
}

/**
 * Map the analyser's byte spectrum (0..255) to `bars` radial ticks placed
 * around the outermost ring. Ticks read left-to-right as low→high energy.
 * Returns 0..1 lengths in px units.
 */
export function spectrumBars(spectrum: Uint8Array, bars: number): number[] {
  const out: number[] = new Array(bars).fill(0)
  if (!spectrum.length || bars <= 0) return out
  const usable = Math.max(1, Math.floor(spectrum.length * 0.7))
  for (let i = 0; i < bars; i++) {
    const idx = Math.floor((i / bars) * usable)
    out[i] = clamp(spectrum[idx] / 255)
  }
  return out
}
