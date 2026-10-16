/**
 * WP4 accessibility primitives — pure, DOM-free logic.
 *
 * Every export here is a plain function over plain data so the whole surface
 * can be unit-tested under the default node environment (no component mount,
 * no store/router). Components wire these props into `StatusLiveRegion` and
 * the override stylesheet; the rules themselves live here and stay
 * framework-free.
 */

/**
 * Event role that decides how a status change should be announced.
 * - `approval` : a user-gated action is waiting on a decision.
 * - `error`    : a hard failure the user must act on.
 * - `alert`    : an immediate system warning.
 * - `success`  : an action completed successfully.
 * - `info`     : neutral progress / informational update.
 * - `status`   : passive background status.
 */
export type AnnounceRole =
  | 'approval'
  | 'error'
  | 'alert'
  | 'success'
  | 'info'
  | 'status'

/** Valid values for the `aria-live` attribute. */
export type AriaLiveMode = 'assertive' | 'polite' | 'off'

/**
 * Map an event role to an `aria-live` politeness mode.
 *
 * `assertive` interrupts the user and is reserved for things that demand
 * immediate attention (approvals waiting on a decision, hard errors, urgent
 * alerts). Everything else uses `polite` so the update is announced when the
 * user pauses instead of yanking focus mid-task. Unknown roles degrade to
 * `polite` — never accidentally blast assertive.
 */
export function announceLevel(role: AnnounceRole): AriaLiveMode {
  switch (role) {
    case 'approval':
    case 'error':
    case 'alert':
      return 'assertive'
    case 'success':
    case 'info':
    case 'status':
      return 'polite'
    default:
      return 'polite'
  }
}

/** WCAG 2.5.8 (Minimum Target Size, Level AA) floor: 24×24 CSS pixels. */
export const MIN_TARGET_SIZE = 24

/**
 * True when a hit-target of `w`×`h` CSS pixels clears WCAG 2.5.8.
 * Boundary-inclusive: exactly 24×24 passes; anything below in either
 * dimension fails. Non-finite or non-positive dimensions also fail.
 */
export function targetSizeOk(w: number, h: number): boolean {
  if (!Number.isFinite(w) || !Number.isFinite(h)) return false
  if (w <= 0 || h <= 0) return false
  return w >= MIN_TARGET_SIZE && h >= MIN_TARGET_SIZE
}

/** Hazard severity bands. L0 = none, L3 = immediate danger. */
export type HazardLevel = 'L0' | 'L1' | 'L2' | 'L3'

export interface HazardGlyph {
  /** Icon token name — the renderer decides the glyph; this stays framework-free. */
  icon: string
  /**
   * Short human-readable label. Status must never be conveyed by colour alone,
   * so every band ships an icon AND a text label.
   */
  label: string
}

const HAZARD_TABLE: Record<HazardLevel, HazardGlyph> = {
  L0: { icon: 'check-circle', label: 'No hazard' },
  L1: { icon: 'info', label: 'Low risk' },
  L2: { icon: 'alert-triangle', label: 'Caution' },
  L3: { icon: 'alert-octagon', label: 'Danger' },
}

/**
 * Resolve a hazard band to its icon token + text label.
 * The pair means screen-reader and low-vision users get the same signal as
 * sighted users who rely on colour coding. An unknown band falls back to the
 * safe L0 glyph rather than throwing.
 */
export function hazardIcon(level: HazardLevel): HazardGlyph {
  return HAZARD_TABLE[level] ?? HAZARD_TABLE.L0
}
