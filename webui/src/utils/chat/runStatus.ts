// runStatus — the SINGLE source of truth for the session run-status vocabulary.
//
// History (F-06): the same vocabulary was copy-pasted into `useSessions.ts` and
// `chat/useChatRunStatus.ts`. 0.2.9 had to patch `error → failed` in BOTH, and
// they had already silently drifted on `approval_pending` (the sessions list
// folded it to `idle`, the chat runtime kept it). This module removes the drift:
// both entries delegate here, so a future addition can never again miss a copy.
//
// Gateway vocabulary (`Envelope::sessions_changed`, core/src/events/envelope.rs):
//   running / idle / error  — plus task-level statuses (queued/…).

export const CANONICAL_RUN_STATUS_VALUES = [
  'queued',
  'running',
  'approval_pending',
  'interrupted',
  'failed',
  'timeout',
  'cancelled',
  'idle',
] as const

export type CanonicalRunStatus = (typeof CANONICAL_RUN_STATUS_VALUES)[number]

/**
 * Normalize a raw status string into the canonical run-status value.
 *
 * - `abandoned` → `interrupted`, `killed` → `cancelled`
 * - `error` → `failed` (a failed turn must never read as idle)
 * - `succeeded` / `success` / `complete` → `idle`
 * - anything unknown → `idle`
 */
export function normalizeRunStatus(status: string | undefined | null): CanonicalRunStatus {
  const value = String(status || '').toLowerCase()
  if (value === 'abandoned') return 'interrupted'
  if (value === 'killed') return 'cancelled'
  if (value === 'error') return 'failed'
  if (value === 'succeeded' || value === 'success' || value === 'complete') return 'idle'
  if ((CANONICAL_RUN_STATUS_VALUES as readonly string[]).includes(value)) {
    return value as CanonicalRunStatus
  }
  return 'idle'
}