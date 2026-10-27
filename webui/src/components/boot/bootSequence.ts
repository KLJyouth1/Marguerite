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

// `BOOT NN/06` stage names mirror the case-4 "BUILD 01/15" progress narrative:
// each line tells the user the shell is genuinely walking through subsystems,
// at the ~120-260ms staggered log cadence. This is presentation timing, NOT
// gateway readiness truth — the `ready` gate still only opens on its own hold.
export const BOOT_LOG: BootLogLine[] = [
  { id: 'l1', text: 'BOOT 01/06 · dual-engine core online', delayMs: 120 },
  { id: 'l2', text: 'BOOT 02/06 · gateway :18790 attached', delayMs: 380 },
  { id: 'l3', text: 'BOOT 03/06 · agent fleet + subagents linked', delayMs: 640 },
  { id: 'l4', text: 'BOOT 04/06 · tool runtime attached · token-efficient path', delayMs: 900 },
  { id: 'l5', text: 'BOOT 05/06 · memory · retrieval · knowledge warm', delayMs: 1160 },
  { id: 'l6', text: 'BOOT 06/06 · workspace standing by', delayMs: 1420 },
]

/**
 * Boot-orchestra display window — the single source of truth for its timing.
 *
 * 为什么是 10s–30s（2026-10-25 改造）：
 *   · 最短 10s —— 启动动画是用户明确要求的「入场仪式」，不能因为网关抢先就绪
 *     在 2.4s 内闪走（旧值 2400/2800ms，实际观感是一闪而过）；就绪也得等满 10s。
 *   · 最长 30s —— 网关始终不就绪时不能无限挡住界面：到 30s 强制进入并打一条
 *     console.warn（可观测原因），把「卡在启动」变成可诊断问题而不是黑屏。
 *   · 用户显式跳过（Enter / Esc / 点击）不受这两个数字约束，立即进入。
 */
export const BOOT_ORCHESTRA_MIN_MS = 10_000
export const BOOT_ORCHESTRA_MAX_MS = 30_000

export const BOOT_ORCHESTRA_SEEN_KEY = 'marguerite-boot-orchestra-seen'
