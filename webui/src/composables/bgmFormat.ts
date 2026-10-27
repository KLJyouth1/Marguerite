/**
 * Pure presentation helpers for the background-music player UI.
 * No Vue, no module state — every function here is plain math/string work so it
 * can be unit-tested without mounting anything.
 */

/**
 * Format a possibly-NaN time in seconds as `m:ss` (clamped to 0 when missing).
 * Negative or non-finite input renders as `0:00`; hours are folded into the
 * minutes field (a 90-minute ambient set reads `90:00`, never `1:30:00`).
 */
export function formatClock(seconds: number | undefined | null): string {
  if (typeof seconds !== 'number' || !Number.isFinite(seconds) || seconds < 0) return '0:00'
  const total = Math.floor(seconds)
  const m = Math.floor(total / 60)
  const s = total % 60
  return `${m}:${String(s).padStart(2, '0')}`
}

/**
 * Clamp a 0..1 fraction into a seek position within the known duration.
 * Returns null when there is no timeline yet (e.g. the synth pad) — the UI
 * then keeps the progress bar inert instead of seeking a phantom position.
 */
export function fractionToSeek(fraction: number, duration: number): number | null {
  if (!Number.isFinite(duration) || duration <= 0) return null
  const clamped = Math.min(1, Math.max(0, fraction))
  return clamped * duration
}
