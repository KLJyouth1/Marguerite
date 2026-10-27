import { computed, ref } from 'vue'
import {
  countDiffLines,
  normalizeSessionFileChanges,
  type SessionFileChange,
} from '@/utils/chat/toolPayload'
import type { FileChangeListPayload, FileChangePayload } from '@/types/rpc'

type TurnChangesRpc = {
  call: <T = unknown>(method: string, params?: Record<string, unknown>) => Promise<T>
  supportsMethod?: (method: string) => boolean
}

export interface UseTurnFileChangesOptions {
  rpc: TurnChangesRpc
}

/** 一条展示用改动：同一路径在一个回合内的多次改动已合并。 */
export interface TurnFileChange {
  path: string
  /** 本回合第一次改动时的类型，反映「相对回合开始时」的净效果 */
  kind: SessionFileChange['kind']
  tool: string
  added: number
  removed: number
  /** 本回合最后一次改动时间（unix 秒） */
  ts: number
}

export interface TurnChangeSummary {
  files: number
  added: number
  removed: number
}

export type TurnRevertFailureCode = 'no_snapshot' | 'ledger_unavailable' | 'revert_failed'

export interface TurnRevertFailure {
  path: string
  code: TurnRevertFailureCode
  error: string
}

export interface TurnRevertResult {
  reverted: number
  failures: TurnRevertFailure[]
}

/**
 * 消息时间戳可能是 epoch 秒 / epoch 毫秒 / ISO 串；台账统一用 epoch 秒。
 * 无法解析时返回 null —— 调用方不得把它当成 0 参与时间窗过滤。
 */
export function toEpochSeconds(value: string | number | null | undefined): number | null {
  if (value === null || value === undefined || value === '') return null
  if (typeof value === 'number') {
    if (!Number.isFinite(value) || value <= 0) return null
    return value >= 1e12 ? Math.floor(value / 1000) : Math.floor(value)
  }
  const raw = value.trim()
  if (!raw) return null
  if (/^\d+$/.test(raw)) return toEpochSeconds(Number(raw))
  const ms = Date.parse(raw)
  return Number.isFinite(ms) ? Math.floor(ms / 1000) : null
}

/** 时钟偏移容忍：前端消息时间与后端台账时间来自不同时钟。 */
const CLOCK_SKEW_SECONDS = 5

function statsOf(change: SessionFileChange): { added: number, removed: number } {
  if (change.added !== null || change.removed !== null) {
    return { added: change.added ?? 0, removed: change.removed ?? 0 }
  }
  return countDiffLines(change.diff)
}

/**
 * 把会话改动收敛成「本回合、按路径去重」的行。
 *
 * - 只保留 `ts >= since` 的条目：台账是会话级，不按回合切分；
 * - 缺少 `ts` 的条目无法归因，直接丢弃（不猜「大概是本回合的」）；
 * - 同一路径多次改动合并，`kind` 取最早那次（= 相对回合开始时的净效果）。
 */
export function aggregateTurnChanges(
  changes: SessionFileChange[],
  sinceSeconds: number,
): TurnFileChange[] {
  if (!Number.isFinite(sinceSeconds)) return []
  const byPath = new Map<string, TurnFileChange>()
  // 后端按最新在前返回；反过来遍历才能让每个路径保留最早那次改动。
  for (const change of [...changes].reverse()) {
    const ts = change.ts
    if (typeof ts !== 'number' || !Number.isFinite(ts)) continue
    if (ts < sinceSeconds - CLOCK_SKEW_SECONDS) continue
    const stats = statsOf(change)
    const existing = byPath.get(change.path)
    if (existing) {
      existing.added += stats.added
      existing.removed += stats.removed
      existing.ts = Math.max(existing.ts, ts)
      continue
    }
    byPath.set(change.path, {
      path: change.path,
      kind: change.kind,
      tool: change.tool,
      added: stats.added,
      removed: stats.removed,
      ts,
    })
  }
  return [...byPath.values()].sort((a, b) => b.ts - a.ts)
}

/**
 * 全局台账（最新在前）→ 每个路径在本时间窗内的**最早**快照。
 *
 * 回滚要用「回合开始前的内容」，所以不能取最新一条的 `before`；
 * 而台账没有会话维度，只能按时间窗 + 路径匹配。匹配不到时调用方必须
 * 如实报「缺原始快照」，绝不用空串覆盖磁盘文件。
 */
export function earliestSnapshots(
  items: FileChangePayload[],
  sinceSeconds: number,
): Map<string, FileChangePayload> {
  const byPath = new Map<string, FileChangePayload>()
  for (const item of [...items].reverse()) {
    if (!item || typeof item !== 'object') continue
    const path = typeof item.path === 'string' ? item.path.trim() : ''
    if (!path) continue
    if (typeof item.ts !== 'number' || !Number.isFinite(item.ts)) continue
    if (item.ts < sinceSeconds - CLOCK_SKEW_SECONDS) continue
    if (byPath.has(path)) continue
    byPath.set(path, item)
  }
  return byPath
}

export function summarizeTurnChanges(changes: TurnFileChange[]): TurnChangeSummary {
  return changes.reduce<TurnChangeSummary>((acc, change) => ({
    files: acc.files + 1,
    added: acc.added + change.added,
    removed: acc.removed + change.removed,
  }), { files: 0, added: 0, removed: 0 })
}

/**
 * 本回合的文件改动汇总 + 批量裁决。
 *
 * 数据源刻意只用既有契约：`sessions.fileChanges` 提供改动与时间戳，
 * `workspace.file_changes.list` 提供「回合开始前的原文」，回滚走
 * `workspace.file_changes.revert`。台账不回删条目，所以回滚成功后由前端
 * 本地移除对应行，而不是重新拉取（否则已撤销的文件会一直显示）。
 */
export function useTurnFileChanges(options: UseTurnFileChangesOptions) {
  const changes = ref<TurnFileChange[]>([])
  const loading = ref(false)
  const reverting = ref(false)
  /** 后端没有 `sessions.fileChanges`：整条汇总静默隐藏，不把版本差异当故障。 */
  const unsupported = ref(false)
  const canRevert = ref(false)

  let query: { sessionKey: string; since: number | null } = { sessionKey: '', since: null }

  const summary = computed(() => summarizeTurnChanges(changes.value))
  const visible = computed(() => !unsupported.value && changes.value.length > 0)

  async function load(sessionKey: string, sinceRaw: string | number | null | undefined): Promise<boolean> {
    const since = toEpochSeconds(sinceRaw)
    query = { sessionKey: sessionKey.trim(), since }
    changes.value = []
    if (!query.sessionKey || since === null) return false
    if (options.rpc.supportsMethod) {
      if (!options.rpc.supportsMethod('sessions.fileChanges')) {
        unsupported.value = true
        canRevert.value = false
        return false
      }
      canRevert.value = options.rpc.supportsMethod('workspace.file_changes.list')
    } else {
      canRevert.value = true
    }
    unsupported.value = false
    loading.value = true
    try {
      const response = await options.rpc.call<Record<string, unknown>>('sessions.fileChanges', {
        session: query.sessionKey,
      })
      if (response && response.ok === false) return false
      changes.value = aggregateTurnChanges(normalizeSessionFileChanges(response), since)
      return true
    } catch {
      return false
    } finally {
      loading.value = false
    }
  }

  function reset() {
    query = { sessionKey: '', since: null }
    changes.value = []
    loading.value = false
    reverting.value = false
    unsupported.value = false
    canRevert.value = false
  }

  /** 成功返回 null；失败返回可展示的原因。 */
  async function revertPath(path: string, kind: string, before: string): Promise<string | null> {
    try {
      const result = await options.rpc.call<{ ok?: boolean, reverted?: boolean, error?: string }>(
        'workspace.file_changes.revert',
        { path, kind, before },
      )
      if (result?.ok === true || result?.reverted === true) return null
      return result?.error || ''
    } catch (error) {
      return error instanceof Error ? error.message : String(error)
    }
  }

  /**
   * 逐个回滚本回合的改动。串行执行，部分失败不影响其余文件。
   *
   * - 本回合新建的文件直接删除（不需要原始内容）；
   * - 修改/删除的文件必须拿到「回合开始前」的快照，拿不到就报 `no_snapshot`；
   * - 返回真实的成功/失败清单，调用方据此更新界面，绝不谎报全部成功。
   */
  async function revertAll(): Promise<TurnRevertResult | null> {
    if (reverting.value) return null
    const { sessionKey, since } = query
    if (!sessionKey || since === null || changes.value.length === 0) return null
    reverting.value = true
    const failures: TurnRevertFailure[] = []
    const revertedPaths: string[] = []
    try {
      let snapshots = new Map<string, FileChangePayload>()
      let ledgerUnavailable = false
      try {
        const ledger = await options.rpc.call<FileChangeListPayload>('workspace.file_changes.list', {
          limit: 200,
        })
        snapshots = earliestSnapshots(
          Array.isArray(ledger?.items) ? ledger.items : [],
          since,
        )
      } catch {
        ledgerUnavailable = true
      }

      for (const change of changes.value) {
        if (change.kind === 'create') {
          const error = await revertPath(change.path, 'create', '')
          if (error === null) revertedPaths.push(change.path)
          else failures.push({ path: change.path, code: 'revert_failed', error })
          continue
        }
        const snapshot = snapshots.get(change.path)
        if (!snapshot) {
          failures.push({
            path: change.path,
            code: ledgerUnavailable ? 'ledger_unavailable' : 'no_snapshot',
            error: '',
          })
          continue
        }
        const error = await revertPath(change.path, snapshot.kind, snapshot.before)
        if (error === null) revertedPaths.push(change.path)
        else failures.push({ path: change.path, code: 'revert_failed', error })
      }

      changes.value = changes.value.filter(change => !revertedPaths.includes(change.path))
      return { reverted: revertedPaths.length, failures }
    } finally {
      reverting.value = false
    }
  }

  return {
    changes,
    loading,
    reverting,
    unsupported,
    canRevert,
    summary,
    visible,
    load,
    reset,
    revertAll,
  }
}
