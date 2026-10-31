/**
 * Adapters: existing console state → `AttentionItem[]`.
 *
 * Kept apart from `attentionQueue.ts` so the ranking module stays free of store
 * imports (and therefore trivially testable), while this file is the *only* place
 * that knows which store fields feed the shelf. Adding a second source later
 * (failed tasks, pending choices) means adding a function here — never touching
 * the ranking invariant.
 *
 * ## Why approvals carry a "first seen" clock
 *
 * The gateway reports an approval as a tuple of ids; it does not report *when*
 * the user started being blocked. The honest substitute is "when this renderer
 * first observed it" — it under-reports age after a reload, which is the safe
 * direction: an item that looks *younger* than it is never gets buried, and a
 * reload must not fabricate two hours of waiting.
 */

import type { PendingApproval } from '@/stores/app'

import type { AttentionItem } from './attentionQueue'

/**
 * Tracks first-observation time per approval id.
 *
 * Deliberately a plain object wrapper rather than a module-level `Map`, so a
 * caller can reset it per session and tests can inject a fixed clock.
 */
export interface FirstSeenTracker {
  /** Returns the stored timestamp, stamping `now` the first time an id is seen. */
  stamp(id: string, now: number): number
  /** Forget ids that are no longer pending, so the map cannot grow without bound. */
  retain(liveIds: Iterable<string>): void
  /** Current size — exposed for the leak test. */
  size(): number
}

export function createFirstSeenTracker(): FirstSeenTracker {
  const seen = new Map<string, number>()
  return {
    stamp(id: string, now: number): number {
      const existing = seen.get(id)
      if (existing !== undefined) return existing
      seen.set(id, now)
      return now
    },
    retain(liveIds: Iterable<string>): void {
      const live = new Set(liveIds)
      for (const id of [...seen.keys()]) {
        if (!live.has(id)) seen.delete(id)
      }
    },
    size(): number {
      return seen.size
    },
  }
}

/**
 * One pending approval → one attention item.
 *
 * `kind` is always `approval`: from the console's point of view every pending
 * approval blocks a live turn. Finer distinctions (destructive command, network
 * egress) belong to the approval card itself, which has the full command text
 * and the permission matrix — the shelf must not claim to know them.
 *
 * The title is the tool name plus the command's first line, hard-truncated.
 * Truncation happens here rather than in CSS so the accessible name matches what
 * is on screen; a title that only *looks* short but reads long is worse for
 * screen readers than a genuinely short one.
 */
export const ATTENTION_TITLE_MAX = 80

export function truncateAttentionTitle(text: string, max = ATTENTION_TITLE_MAX): string {
  const oneLine = text.replace(/\s+/g, ' ').trim()
  if (oneLine.length <= max) return oneLine
  // Keep the head: a command's discriminating part is its verb + path prefix,
  // and the tail is usually flags the user will read in the card anyway.
  return `${oneLine.slice(0, Math.max(0, max - 1))}…`
}

export function approvalToAttention(
  approval: PendingApproval,
  firstSeen: FirstSeenTracker,
  now: number,
): AttentionItem | null {
  const approvalId = String(approval.approvalId || '').trim()
  const sessionKey = String(approval.sessionKey || '').trim()
  // An approval with no session cannot be routed to, and one with no id cannot
  // be de-duplicated. Dropping it here is honest: the topbar pill still counts
  // it, so the user is not silently losing a blocker.
  if (!approvalId || !sessionKey) return null

  const tool = String(approval.tool || '').trim()
  const command = truncateAttentionTitle(String(approval.command || ''))
  const title = tool && command ? `${tool} · ${command}` : tool || command || approvalId

  return {
    id: `approval:${approvalId}`,
    kind: 'approval',
    at: firstSeen.stamp(approvalId, now),
    title: truncateAttentionTitle(title),
    target: { kind: 'approval', approvalId, sessionKey },
  }
}

/** Drop stale ids and map the live list. Null-safe: anything unroutable is skipped. */
export function approvalsToAttention(
  approvals: readonly PendingApproval[],
  firstSeen: FirstSeenTracker,
  now: number,
): AttentionItem[] {
  const live = approvals.filter((a) => String(a.approvalId || '').trim())
  firstSeen.retain(live.map((a) => String(a.approvalId).trim()))
  return live
    .map((approval) => approvalToAttention(approval, firstSeen, now))
    .filter((item): item is AttentionItem => item !== null)
}
