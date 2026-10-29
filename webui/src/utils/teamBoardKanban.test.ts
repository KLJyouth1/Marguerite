// teamBoardKanban 纯函数：状态→泳道映射 + 视图偏好持久化。
// happy-dom：最后的「真实 localStorage 契约」测试需要全局 storage。
// @vitest-environment happy-dom
import { afterEach, describe, expect, it } from 'vitest'
import {
  TEAM_BOARD_VIEW_STORAGE_KEY,
  TEAM_KANBAN_COLUMN_ORDER,
  kanbanColumnForStatus,
  loadTeamBoardViewMode,
  saveTeamBoardViewMode,
} from '@/utils/teamBoardKanban'

function memoryStorage(initial: Record<string, string> = {}): {
  store: Record<string, string>
  storage: Pick<Storage, 'getItem' | 'setItem'>
} {
  const store: Record<string, string> = { ...initial }
  return {
    store,
    storage: {
      getItem: (key: string) => (key in store ? store[key] : null),
      setItem: (key: string, value: string) => {
        store[key] = value
      },
    },
  }
}

afterEach(() => {
  try {
    localStorage.clear()
  } catch {
    /* 测试环境无 localStorage 时忽略 */
  }
})

describe('kanbanColumnForStatus', () => {
  it('maps the four backend statuses and the frontend thinking state onto their columns', () => {
    expect(kanbanColumnForStatus('waiting')).toBe('waiting')
    expect(kanbanColumnForStatus('running')).toBe('running')
    // thinking 是前端从 member_thinking 帧合成的状态，归「进行中」
    expect(kanbanColumnForStatus('thinking')).toBe('running')
    expect(kanbanColumnForStatus('finished')).toBe('finished')
    expect(kanbanColumnForStatus('failed')).toBe('failed')
  })

  it('routes unknown statuses to the honest "other" column instead of guessing', () => {
    expect(kanbanColumnForStatus('queued')).toBe('other')
    expect(kanbanColumnForStatus('')).toBe('other')
    expect(kanbanColumnForStatus('RUNNING')).toBe('other')
  })

  it('keeps the lane order fixed: 待命 → 进行中 → 已完成 → 异常 → 未报告', () => {
    expect(TEAM_KANBAN_COLUMN_ORDER).toEqual(['waiting', 'running', 'finished', 'failed', 'other'])
  })
})

describe('view-mode persistence', () => {
  it('round-trips the kanban preference', () => {
    const { storage, store } = memoryStorage()
    expect(loadTeamBoardViewMode(storage)).toBe('grid')

    saveTeamBoardViewMode('kanban', storage)
    expect(store[TEAM_BOARD_VIEW_STORAGE_KEY]).toBe('kanban')
    expect(loadTeamBoardViewMode(storage)).toBe('kanban')

    saveTeamBoardViewMode('grid', storage)
    expect(loadTeamBoardViewMode(storage)).toBe('grid')
  })

  it('treats dirty values as grid instead of throwing', () => {
    const { storage } = memoryStorage({ [TEAM_BOARD_VIEW_STORAGE_KEY]: 'board-view' })
    expect(loadTeamBoardViewMode(storage)).toBe('grid')
    expect(loadTeamBoardViewMode(memoryStorage({ [TEAM_BOARD_VIEW_STORAGE_KEY]: '' }).storage)).toBe('grid')
  })

  it('degrades to grid when storage access throws', () => {
    const throwing = {
      getItem: () => {
        throw new Error('blocked')
      },
      setItem: () => {
        throw new Error('blocked')
      },
    }
    expect(loadTeamBoardViewMode(throwing)).toBe('grid')
    expect(() => saveTeamBoardViewMode('kanban', throwing)).not.toThrow()
  })
})

describe('real localStorage contract', () => {
  it('uses the documented storage key', () => {
    saveTeamBoardViewMode('kanban')
    expect(localStorage.getItem('marguerite.teamBoard.view.v1')).toBe('kanban')
    expect(loadTeamBoardViewMode()).toBe('kanban')
  })
})
