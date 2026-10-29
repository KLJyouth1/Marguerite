/**
 * 团队看板 A3：看板视图的纯函数层。
 *
 * 两件事，都不碰 RPC、不碰组件：
 *   1. **状态 → 泳道列**的映射。列只来自 `resolveMemberStatus` 真实可能产出的
 *      取值（后端 `member_registry` 全集：waiting / running / finished / failed；
 *      前端合成：thinking），其余一律落「未报告」列 —— 不为不存在的状态造列。
 *   2. **网格 / 看板视图偏好**的持久化（localStorage，坏值自愈回 grid）。
 */

export type TeamBoardViewMode = 'grid' | 'kanban'

/** 泳道列 id；`other` 是「后端没声明过的状态」的诚实收口，不是兜底装饰。 */
export type TeamKanbanColumnId = 'waiting' | 'running' | 'finished' | 'failed' | 'other'

export const TEAM_BOARD_VIEW_STORAGE_KEY = 'marguerite.teamBoard.view.v1'

/** 泳道列的固定顺序（待命 → 进行中 → 已完成 → 异常 → 未报告）。 */
export const TEAM_KANBAN_COLUMN_ORDER: readonly TeamKanbanColumnId[] = [
  'waiting',
  'running',
  'finished',
  'failed',
  'other',
]

/**
 * 状态 → 列。只认 `resolveMemberStatus` / `member_registry` 真实产出的取值：
 * - `thinking` 是前端从 member_thinking 帧还原的状态，归「进行中」；
 * - 其余任何字符串（后端未来扩展、或脏数据）→ `other`，卡片上仍显示原值。
 */
export function kanbanColumnForStatus(status: string): TeamKanbanColumnId {
  switch (status) {
    case 'waiting':
      return 'waiting'
    case 'running':
    case 'thinking':
      return 'running'
    case 'finished':
      return 'finished'
    case 'failed':
      return 'failed'
    default:
      return 'other'
  }
}

type MinimalStorage = Pick<Storage, 'getItem' | 'setItem'>

function defaultStorage(): MinimalStorage | null {
  try {
    if (typeof localStorage === 'undefined') return null
    return localStorage
  } catch {
    return null
  }
}

/** 读取视图偏好：只有精确的 'kanban' 才算数，其余（null / 脏值 / 异常）一律 grid。 */
export function loadTeamBoardViewMode(storage: MinimalStorage | null = defaultStorage()): TeamBoardViewMode {
  try {
    return storage?.getItem(TEAM_BOARD_VIEW_STORAGE_KEY) === 'kanban' ? 'kanban' : 'grid'
  } catch {
    return 'grid'
  }
}

/** 保存视图偏好；写失败（隐私模式等）静默忽略 —— 偏好丢失可接受，功能不可丢。 */
export function saveTeamBoardViewMode(
  mode: TeamBoardViewMode,
  storage: MinimalStorage | null = defaultStorage(),
): void {
  try {
    storage?.setItem(TEAM_BOARD_VIEW_STORAGE_KEY, mode)
  } catch {
    /* 偏好写失败不打断交互 */
  }
}
