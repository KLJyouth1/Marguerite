/**
 * Hardcoded boot-orchestra script — the visual shell sequence shown after
 * mount when splash tier is `full`. Pure presentation data; not gateway truth.
 */

export type BootNodeKind = 'agent' | 'subagent' | 'tool' | 'memory' | 'retrieval' | 'knowledge'

export interface BootNode {
  id: string
  kind: BootNodeKind
  label: string
  detail: string
  /** polar placement: angle degrees, radius 0..1 */
  angle: number
  radius: number
}

export interface BootLogLine {
  id: string
  text: string
  delayMs: number
}

export const BOOT_NODES: BootNode[] = [
  { id: 'lead', kind: 'agent', label: 'Lead Agent', detail: 'orchestrating', angle: -90, radius: 0.78 },
  { id: 'arch', kind: 'subagent', label: 'Architect', detail: 'planning', angle: -30, radius: 0.82 },
  { id: 'build', kind: 'subagent', label: 'Builder', detail: 'coding', angle: 30, radius: 0.82 },
  { id: 'review', kind: 'subagent', label: 'Reviewer', detail: 'verifying', angle: 90, radius: 0.78 },
  { id: 'tools', kind: 'tool', label: 'Tool Runtime', detail: '12 adapters', angle: 150, radius: 0.84 },
  { id: 'mem', kind: 'memory', label: 'Memory', detail: 'episodic + semantic', angle: 210, radius: 0.8 },
  { id: 'retr', kind: 'retrieval', label: 'Retrieval', detail: 'hybrid search', angle: 250, radius: 0.86 },
  { id: 'kb', kind: 'knowledge', label: 'Knowledge', detail: 'graph online', angle: 310, radius: 0.82 },
]

export const BOOT_LOG: BootLogLine[] = [
  { id: 'l1', text: 'dual-engine core online', delayMs: 120 },
  { id: 'l2', text: 'agent fleet + subagents linked', delayMs: 380 },
  { id: 'l3', text: 'memory · retrieval · knowledge warm', delayMs: 640 },
  { id: 'l4', text: 'tool runtime attached · token-efficient path', delayMs: 900 },
  { id: 'l5', text: 'workspace standing by', delayMs: 1160 },
]

/** Minimum hold so a fast mount never strobes the orchestra away. */
export const BOOT_ORCHESTRA_MIN_MS = 2400

/** Auto-enter after ready + min hold (full fanfare). */
export const BOOT_ORCHESTRA_AUTO_MS = 2800

export const BOOT_ORCHESTRA_SEEN_KEY = 'marguerite-boot-orchestra-seen'
