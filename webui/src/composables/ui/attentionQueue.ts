/**
 * Attention queue — the ranking brain behind the Attention Shelf.
 *
 * ## Why this exists (first principles)
 *
 * An agent desktop is not a chat client; it is a *delegation* surface. The user
 * hands over intent and then waits. Everything the UI can do during that wait
 * is: (a) tell the truth about progress, or (b) ask for a decision. A chat
 * transcript does neither — a blocked approval scrolls out of view exactly when
 * it starts costing the user the most time.
 *
 * So screen space is allocated by **urgency**, not by feature category. This
 * module owns that allocation: it turns heterogeneous "things that need a
 * human" into one deterministic order.
 *
 * ## The ordering guarantee (the whole point)
 *
 * Recency is a *tie-breaker inside a kind*, never a cross-kind lever. The base
 * weights are spaced far wider than the entire recency range
 * (`RECENCY_MAX` = 99 < the smallest gap, 300), so:
 *
 *   a freshly appeared `choice` can NEVER outrank an `approval` that has been
 *   sitting for two hours.
 *
 * That invariant is deliberate and is asserted in the test file. Without it the
 * shelf quietly becomes "newest thing wins", which is exactly the failure mode
 * of notification feeds — and the reason people stop reading them.
 */

export type AttentionKind = 'approval' | 'blocked' | 'failed' | 'choice'

/** Where activating an item should take the user. */
export type AttentionTarget =
  | { kind: 'route'; path: string }
  | { kind: 'approval'; approvalId: string; sessionKey: string }

export interface AttentionItem {
  /** Stable within a render pass; two items with the same id are a bug. */
  id: string
  kind: AttentionKind
  /** Epoch ms when the item started needing attention. */
  at: number
  /** Already-localised one-line label. This module never translates. */
  title: string
  /** Optional already-localised second line. */
  detail?: string
  target?: AttentionTarget
}

export interface RankedAttention extends AttentionItem {
  /** Base kind weight + recency bonus. Higher sorts first. */
  score: number
}

/**
 * Base weight per kind. The gaps are intentional:
 * approval(1000) − blocked(700) = 300, blocked − failed = 300, failed − choice = 250.
 * Every gap is > `RECENCY_MAX`, so kind ordering is immune to time.
 *
 * - `approval`  blocks a running tool; the whole turn is stopped. Costs the user
 *               wall-clock time every second it waits.
 * - `blocked`   something cannot continue but nothing is burning (e.g. missing
 *               credential). Still a hard stop.
 * - `failed`    the work already ended badly; no longer burning time, but the
 *               user has to decide whether to retry.
 * - `choice`    an optional fork (pick a model, pick a branch). Lowest.
 */
export const KIND_BASE: Readonly<Record<AttentionKind, number>> = {
  approval: 1000,
  blocked: 700,
  failed: 400,
  choice: 150,
}

/** How long a brand-new item keeps its full recency bonus. */
export const RECENCY_SPAN_MS = 30 * 60 * 1000

/** Recency bonus ceiling. MUST stay below the smallest `KIND_BASE` gap. */
export const RECENCY_MAX = 99

/**
 * Linear decay of the recency bonus over `RECENCY_SPAN_MS`.
 *
 * `at` in the future (clock skew between gateway and renderer) is clamped to
 * full bonus rather than producing a negative age — a skewed clock must not
 * reorder the shelf.
 */
export function recencyBonus(at: number, now: number): number {
  if (!Number.isFinite(at) || !Number.isFinite(now)) return 0
  const age = now - at
  if (age <= 0) return RECENCY_MAX
  if (age >= RECENCY_SPAN_MS) return 0
  return Math.round((1 - age / RECENCY_SPAN_MS) * RECENCY_MAX)
}

/** Composite score. Pure; safe to call in a computed. */
export function attentionScore(item: AttentionItem, now: number): number {
  const base = KIND_BASE[item.kind] ?? 0
  return base + recencyBonus(item.at, now)
}

/**
 * Deterministic ranking.
 *
 * Sort keys, in order: score desc → `at` desc (newer first on a tie) → `id` asc
 * (so an exact tie never depends on input order, which would make the shelf
 * flicker between renders).
 *
 * `limit` <= 0 means "no limit"; the shelf passes a small number because a
 * peripheral surface that needs scrolling is not peripheral any more.
 */
export function rankAttention(
  items: readonly AttentionItem[],
  now: number,
  limit = 0,
): RankedAttention[] {
  const ranked = items
    .map((item) => ({ ...item, score: attentionScore(item, now) }))
    .sort((a, b) => {
      if (b.score !== a.score) return b.score - a.score
      if (b.at !== a.at) return b.at - a.at
      return a.id < b.id ? -1 : a.id > b.id ? 1 : 0
    })
  return limit > 0 ? ranked.slice(0, limit) : ranked
}

/** Counts per kind over *all* items (not just the visible slice). */
export function attentionCounts(
  items: readonly AttentionItem[],
): Record<AttentionKind, number> {
  const counts: Record<AttentionKind, number> = {
    approval: 0,
    blocked: 0,
    failed: 0,
    choice: 0,
  }
  for (const item of items) counts[item.kind] += 1
  return counts
}

/**
 * Total attention *debt*: how many distinct human decisions are outstanding.
 * Distinct from `items.length` only if a caller ever passes duplicates — the
 * shelf uses it for the collapsed pill's badge.
 */
export function attentionDebt(items: readonly AttentionItem[]): number {
  return new Set(items.map((item) => item.id)).size
}

/**
 * Drop ids that appear twice, keeping the *earlier* `at` (the item started
 * needing attention then; a later duplicate is the same debt re-announced).
 * Producers can be sloppy — two RPC surfaces can both report one approval — and
 * double-counting is worse than a stale timestamp.
 */
export function dedupeAttention(items: readonly AttentionItem[]): AttentionItem[] {
  const byId = new Map<string, AttentionItem>()
  for (const item of items) {
    const seen = byId.get(item.id)
    if (!seen || item.at < seen.at) byId.set(item.id, item)
  }
  return [...byId.values()]
}
