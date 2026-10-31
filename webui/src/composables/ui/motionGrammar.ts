/**
 * Motion grammar — four verbs, one rule: **no motion without a state change.**
 *
 * ## Why a grammar instead of ad-hoc transitions
 *
 * Decoration is motion that fires because *time* passed. Causality is motion
 * that fires because *state* changed. Users can only learn the second kind, and
 * only the second kind stays trustworthy: once an animation runs while nothing
 * happens, every later animation becomes noise and the user stops reading it.
 *
 * So every cue below is named after a *state transition*, never after an effect:
 *
 * | cue      | transition                        | what the eye should infer      |
 * |----------|-----------------------------------|--------------------------------|
 * | `arrive` | nothing → one new piece of work   | "something landed"             |
 * | `settle` | pending → committed               | "that is now final"            |
 * | `yield`  | idle → waiting on someone else    | "it is not stuck, it is working"|
 * | `refuse` | in-flight → rejected/blocked      | "this needs me"                |
 *
 * There is deliberately **no** `success` cue: success is `settle`, and giving it
 * its own celebratory animation is how products teach users to ignore motion.
 *
 * ## Timing
 *
 * Durations and easings are *tokens* from `src/assets/foundation.css`, never raw
 * literals — the repo's `check-webui-motion.mjs` gate enforces that, and it is
 * right to: "an exit is one tier faster than an entrance" has to be tuneable in
 * one place. This module only decides *which* token applies to which cue.
 *
 * ## Reduced motion
 *
 * `prefers-reduced-motion` does not mean "no feedback". It means "no spatial
 * displacement". Every cue degrades to an opacity-only variant so the user still
 * sees *that* something changed, just not *where it moved from*.
 */

export type MotionCue = 'arrive' | 'settle' | 'yield' | 'refuse'

export interface MotionRecipe {
  /** CSS custom property name holding the duration. */
  durationToken: '--dur-fast' | '--dur-base' | '--dur-enter' | '--dur-pulse'
  /** CSS custom property name holding the easing. */
  easingToken: '--ease-standard' | '--ease-out' | '--ease-in' | '--ease-spring'
  /** Keyframe name defined in `styles/attention-system.css`. */
  keyframe: string
  /** Vertical displacement in px; 0 means opacity-only even in full motion. */
  distancePx: number
  /** True for the one continuous loop (waiting). */
  loop: boolean
}

/**
 * The complete vocabulary. Adding a fifth cue means adding a row here *and* a
 * keyframe in `attention-system.css`; the style-side name is the only coupling.
 */
export const MOTION_RECIPES: Readonly<Record<MotionCue, MotionRecipe>> = {
  arrive: {
    durationToken: '--dur-base',
    easingToken: '--ease-out',
    keyframe: 'attn-arrive',
    distancePx: 6,
    loop: false,
  },
  settle: {
    durationToken: '--dur-enter',
    easingToken: '--ease-spring',
    keyframe: 'attn-settle',
    distancePx: 0,
    loop: false,
  },
  yield: {
    durationToken: '--dur-pulse',
    easingToken: '--ease-standard',
    keyframe: 'attn-yield',
    distancePx: 0,
    loop: true,
  },
  refuse: {
    durationToken: '--dur-fast',
    easingToken: '--ease-in',
    keyframe: 'attn-refuse',
    distancePx: 0,
    loop: false,
  },
}

/** Cue order used only for deterministic tie-breaking in reports. */
export const MOTION_CUES: readonly MotionCue[] = ['arrive', 'settle', 'yield', 'refuse']

/**
 * Derive the cue from two consecutive *counts*. Returns `null` when nothing
 * changed — the caller must then play nothing, not "the default animation".
 *
 * A shrink is `settle` rather than a fifth "dismiss" cue: from the user's point
 * of view an item leaving the shelf means it got resolved, which is a commit.
 */
export function cueFromCounts(prev: number, next: number): MotionCue | null {
  if (next > prev) return 'arrive'
  if (next < prev) return 'settle'
  return null
}

/**
 * Attributes to spread onto the animated element.
 *
 * Returns `data-motion` only when a cue should actually play, plus
 * `data-motion-static="true"` when the cue is opacity-only. CSS keys off these,
 * so no branch of the render path needs to know about media queries.
 */
export function motionAttrs(
  cue: MotionCue | null,
  reducedMotion: boolean,
): Record<string, string> {
  if (!cue) return {}
  const recipe = MOTION_RECIPES[cue]
  const attrs: Record<string, string> = { 'data-motion': cue }
  if (reducedMotion || recipe.distancePx === 0) attrs['data-motion-static'] = 'true'
  if (recipe.loop) attrs['data-motion-loop'] = 'true'
  return attrs
}

/**
 * Read the OS preference. Guarded for SSR / happy-dom, where `matchMedia` may be
 * missing — a missing implementation must be treated as "user has no stated
 * preference", i.e. full motion, not as "reduced".
 */
export function prefersReducedMotion(): boolean {
  if (typeof window === 'undefined' || typeof window.matchMedia !== 'function') return false
  try {
    return window.matchMedia('(prefers-reduced-motion: reduce)').matches
  } catch {
    return false
  }
}

/**
 * Build the `animation` shorthand for a cue using tokens only.
 * Kept here (not in CSS) so the JS-driven and CSS-driven paths cannot drift.
 */
export function animationShorthand(cue: MotionCue, reducedMotion: boolean): string {
  const recipe = MOTION_RECIPES[cue]
  const name = reducedMotion ? `${recipe.keyframe}-static` : recipe.keyframe
  const iteration = recipe.loop && !reducedMotion ? 'infinite' : '1'
  return `${name} var(${recipe.durationToken}) var(${recipe.easingToken}) ${iteration}`
}
