import { describe, expect, it } from 'vitest'
import {
  arrangeSidebarSections,
  normalizeSessionItem,
  type SessionItem,
} from './useSessions'
import type { ProjectWorkspaceItem } from './useProjectWorkspaces'
import type { RawSessionItem } from '@/types/rpc'

function session(raw: RawSessionItem): SessionItem {
  const item = normalizeSessionItem(raw)
  if (!item) throw new Error('fixture did not normalize')
  return item
}

describe('persisted project sidebar arrangement', () => {
  it('keeps fixed projects first, includes empty projects, and leaves ordinary tasks last', () => {
    const projects: ProjectWorkspaceItem[] = [
      { id: 'project-b', name: 'Project B', path: '/repo/b', taskCount: 0, pinned: true, available: true },
      { id: 'project-a', name: 'Project A', path: '/repo/a', taskCount: 2, pinned: false, available: true },
    ]
    const rows = arrangeSidebarSections([
      session({ key: 'agent:main:webchat:a-old', title: 'A old', updatedAt: 100, workspaceId: 'project-a' }),
      session({ key: 'agent:main:webchat:a-new', title: 'A new', updatedAt: 300, workspaceId: 'project-a' }),
      session({ key: 'agent:main:webchat:ordinary', title: 'Ordinary', updatedAt: 400 }),
    ], projects)[0].rows

    expect(rows.map(row => ({ kind: row.rowKind, key: row.key, depth: row.depth }))).toEqual([
      { kind: 'workspace', key: 'workspace:project-b', depth: 0 },
      { kind: 'workspace-empty', key: 'workspace:project-b:empty', depth: 1 },
      { kind: 'workspace', key: 'workspace:project-a', depth: 0 },
      { kind: 'session', key: 'agent:main:webchat:a-new', depth: 1 },
      { kind: 'session', key: 'agent:main:webchat:a-old', depth: 1 },
      { kind: 'session', key: 'agent:main:webchat:ordinary', depth: 0 },
    ])
  })

  it('never reorders projects from child activity', () => {
    const projects: ProjectWorkspaceItem[] = [
      { id: 'b', name: 'B', path: '/b', taskCount: 1, pinned: true, available: true },
      { id: 'a', name: 'A', path: '/a', taskCount: 1, pinned: false, available: true },
    ]
    const rows = arrangeSidebarSections([
      session({ key: 'agent:main:webchat:a', title: 'A task', updatedAt: 9_999, workspaceId: 'a' }),
      session({ key: 'agent:main:webchat:b', title: 'B task', updatedAt: 1, workspaceId: 'b' }),
    ], projects)[0].rows

    expect(rows.filter(row => row.rowKind === 'workspace').map(row => row.key))
      .toEqual(['workspace:b', 'workspace:a'])
  })

  it('shows a provisional draft under its project and includes it in the visible count', () => {
    const projects: ProjectWorkspaceItem[] = [
      { id: 'project-a', name: 'Project A', path: '/repo/a', taskCount: 0, pinned: false, available: true },
    ]
    const draft = session({
      key: 'draft:project:project-a:1',
      title: 'New task',
      updatedAt: 300,
      workspaceId: 'project-a',
    })
    draft.provisional = true
    draft.sessionKind = 'chat'
    draft.surface = 'webchat'
    const rows = arrangeSidebarSections([draft], projects)[0].rows

    expect(rows.map(row => row.rowKind)).toEqual(['workspace', 'session'])
    expect(rows[0].workspaceTaskCount).toBe(1)
    expect(rows[1]).toMatchObject({
      key: 'draft:project:project-a:1',
      workspaceId: 'project-a',
      provisional: true,
      depth: 1,
    })
  })

  it('keeps workspace-bound tasks visible while the canonical project list is unavailable', () => {
    const rows = arrangeSidebarSections([
      session({
        key: 'agent:main:webchat:project-task',
        title: 'Project task',
        updatedAt: 100,
        workspaceId: 'project-a',
      }),
    ], undefined)[0].rows

    expect(rows.map(row => row.key)).toContain('agent:main:webchat:project-task')
  })

  it('groups sessions under the project matching their sessions.db project_path, not "recent"', () => {
    const projects: ProjectWorkspaceItem[] = [
      { id: 'html', name: 'html', path: 'D:\\Downloads\\html', taskCount: 0, pinned: false, available: true },
      { id: 'ws', name: 'workspace', path: 'D:\\Downloads\\workspace', taskCount: 0, pinned: false, available: true },
    ]
    const rows = arrangeSidebarSections([
      session({
        key: 'agent:main:webchat:fps',
        title: '帮我做一个fps射击游戏',
        updatedAt: 300,
        projectPath: 'D:\\Downloads\\html',
      }),
      session({
        key: 'agent:main:webchat:rust',
        title: '从零构建一个 Rust 命令行编译器',
        updatedAt: 200,
        projectPath: 'D:\\Downloads\\workspace',
      }),
      session({ key: 'agent:main:webchat:weather', title: '查一下明天的天气', updatedAt: 100 }),
    ], projects)[0].rows

    // 两个项目头在前，各自收纳对应会话；无项目路径的会话留在最后的「最近」扁平区。
    expect(rows.map(row => ({ kind: row.rowKind, key: row.key }))).toEqual([
      { kind: 'workspace', key: 'workspace:html' },
      { kind: 'session', key: 'agent:main:webchat:fps' },
      { kind: 'workspace', key: 'workspace:ws' },
      { kind: 'session', key: 'agent:main:webchat:rust' },
      { kind: 'session', key: 'agent:main:webchat:weather' },
    ])
    expect(rows.find(row => row.key === 'agent:main:webchat:fps')?.workspaceId).toBe('html')
  })

  it('matches project membership despite path case and separator differences', () => {
    const projects: ProjectWorkspaceItem[] = [
      { id: 'html', name: 'html', path: 'D:/Downloads/HTML', taskCount: 0, pinned: false, available: true },
    ]
    const rows = arrangeSidebarSections([
      session({
        key: 'agent:main:webchat:fps',
        title: '帮我做一个fps射击游戏',
        updatedAt: 300,
        projectPath: 'd:\\downloads\\html',
      }),
    ], projects)[0].rows

    expect(rows.map(row => row.rowKind)).toEqual(['workspace', 'session'])
    expect(rows[1]?.workspaceId).toBe('html')
  })
})
