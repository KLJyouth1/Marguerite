import { beforeEach, describe, expect, it } from 'vitest'
import type { ProjectWorkspaceItem } from '@/types/rpc'
import {
  projectRootChips,
  projectRootIdsFromPaths,
  useChatProjectRoots,
} from './useChatProjectRoots'

function workspace(
  id: string,
  path: string,
  overrides: Partial<ProjectWorkspaceItem> = {},
): ProjectWorkspaceItem {
  return {
    id,
    name: id.toUpperCase(),
    path,
    taskCount: 0,
    pinned: false,
    available: true,
    ...overrides,
  }
}

const WORKSPACES = [
  workspace('primary', 'D:\\repos\\primary'),
  workspace('extra-a', 'D:\\repos\\extra-a'),
  workspace('extra-b', 'D:\\repos\\extra-b'),
]

describe('projectRootIdsFromPaths', () => {
  it('跳过首项主根，按路径反查附加项目（Windows 大小写/分隔符归一）', () => {
    expect(projectRootIdsFromPaths([
      'D:\\repos\\primary',
      'd:/repos/extra-a/',
      'D:\\repos\\extra-b',
    ], WORKSPACES)).toEqual(['extra-a', 'extra-b'])
  })

  it('去重保序，反查不到的路径直接跳过', () => {
    expect(projectRootIdsFromPaths([
      'D:\\repos\\primary',
      'D:\\repos\\extra-b',
      'D:\\repos\\extra-b\\',
      'D:\\repos\\unknown',
    ], WORKSPACES)).toEqual(['extra-b'])
  })

  it('单根 / 空集合不产生附加项目', () => {
    expect(projectRootIdsFromPaths(['D:\\repos\\primary'], WORKSPACES)).toEqual([])
    expect(projectRootIdsFromPaths(undefined, WORKSPACES)).toEqual([])
  })
})

describe('projectRootChips', () => {
  it('补齐名称/路径/可用性，未知 id 不渲染', () => {
    expect(projectRootChips(['extra-a', 'ghost'], WORKSPACES)).toEqual([
      {
        id: 'extra-a',
        name: 'EXTRA-A',
        path: 'D:\\repos\\extra-a',
        available: true,
      },
    ])
  })
})

describe('useChatProjectRoots', () => {
  beforeEach(() => {
    useChatProjectRoots().resetForSession()
  })

  it('会话 projectPaths 回填到附加项目状态', () => {
    const roots = useChatProjectRoots()

    roots.applyProjectPaths([
      'D:\\repos\\primary',
      'D:\\repos\\extra-a',
      'D:\\repos\\extra-b',
    ], WORKSPACES)

    expect(roots.extraIds.value).toEqual(['extra-a', 'extra-b'])
  })

  it('会话切换重置后不会泄漏上一个会话的附加项目', () => {
    const roots = useChatProjectRoots()
    roots.applyProjectPaths(['D:\\repos\\primary', 'D:\\repos\\extra-a'], WORKSPACES)
    expect(roots.extraIds.value).toEqual(['extra-a'])

    roots.resetForSession()
    expect(roots.extraIds.value).toEqual([])

    // 新会话只有单根 → 元数据回填后仍为空，而不是保留旧选择。
    roots.applyProjectPaths(['D:\\repos\\primary'], WORKSPACES)
    expect(roots.extraIds.value).toEqual([])
  })

  it('多选确认时排除主项目并去重', () => {
    const roots = useChatProjectRoots()

    roots.applySelection(['extra-a', 'primary', 'extra-a', 'extra-b'], 'primary')

    expect(roots.extraIds.value).toEqual(['extra-a', 'extra-b'])
  })

  it('可单独移除附加项目', () => {
    const roots = useChatProjectRoots()
    roots.applySelection(['extra-a', 'extra-b'], 'primary')

    roots.removeExtraId('extra-a')

    expect(roots.extraIds.value).toEqual(['extra-b'])
  })
})
