import { describe, expect, it, vi } from 'vitest'

import {
  aggregateTurnChanges,
  earliestSnapshots,
  summarizeTurnChanges,
  toEpochSeconds,
  useTurnFileChanges,
} from './useTurnFileChanges'
import type { SessionFileChange } from '@/utils/chat/toolPayload'

function change(overrides: Partial<SessionFileChange> = {}): SessionFileChange {
  return {
    path: 'src/a.ts',
    kind: 'modify',
    tool: 'edit_file',
    added: 1,
    removed: 1,
    diff: '',
    truncated: false,
    ts: 1000,
    ...overrides,
  }
}

function makeRpc(supported: string[] | null = null) {
  return {
    call: vi.fn(),
    supportsMethod: vi.fn((method: string) =>
      supported === null ? true : supported.includes(method)),
  }
}

describe('toEpochSeconds', () => {
  it('归一化秒 / 毫秒 / 数字串 / ISO，无法解析时返回 null', () => {
    expect(toEpochSeconds(1_700_000_000)).toBe(1_700_000_000)
    expect(toEpochSeconds(1_700_000_000_123)).toBe(1_700_000_000)
    expect(toEpochSeconds('1700000000')).toBe(1_700_000_000)
    expect(toEpochSeconds('2026-10-25T10:00:00Z')).toBe(1_792_922_400)
    expect(toEpochSeconds(null)).toBeNull()
    expect(toEpochSeconds('')).toBeNull()
    expect(toEpochSeconds('not-a-time')).toBeNull()
    expect(toEpochSeconds(Number.NaN)).toBeNull()
  })
})

describe('aggregateTurnChanges', () => {
  it('只保留时间窗内的条目，5 秒时钟偏移内仍然计入', () => {
    const rows = aggregateTurnChanges([
      change({ path: 'new.ts', ts: 1000 }),
      change({ path: 'skew.ts', ts: 996 }),
      change({ path: 'old.ts', ts: 900 }),
    ], 1000)

    expect(rows.map(row => row.path).sort()).toEqual(['new.ts', 'skew.ts'])
  })

  it('缺少时间戳的条目直接丢弃（不猜它是本回合的）', () => {
    const rows = aggregateTurnChanges([
      change({ path: 'no-ts.ts', ts: null }),
      change({ path: 'ok.ts', ts: 1001 }),
    ], 1000)

    expect(rows).toHaveLength(1)
    expect(rows[0].path).toBe('ok.ts')
  })

  it('同一路径合并行数、保留最早的 kind 与最新的 ts，并按最新在前排序', () => {
    // 后端契约：最新在前
    const rows = aggregateTurnChanges([
      change({ path: 'src/a.ts', kind: 'modify', added: 2, removed: 0, ts: 1010 }),
      change({ path: 'src/b.ts', kind: 'modify', added: 1, removed: 3, ts: 1005 }),
      change({ path: 'src/a.ts', kind: 'create', added: 5, removed: 4, ts: 1002 }),
    ], 1000)

    expect(rows).toHaveLength(2)
    // a.ts 最早一次是 create → 相对回合开始是「新建」
    expect(rows[0]).toMatchObject({ path: 'src/a.ts', kind: 'create', added: 7, removed: 4, ts: 1010 })
    expect(rows[1]).toMatchObject({ path: 'src/b.ts', kind: 'modify', added: 1, removed: 3 })
  })

  it('缺 added/removed 时按 diff 正文统计', () => {
    const rows = aggregateTurnChanges([
      change({ added: null, removed: null, diff: '--- a\n+++ b\n@@\n-old\n+new\n+extra' }),
    ], 1000)

    expect(rows[0]).toMatchObject({ added: 2, removed: 1 })
  })

  it('非有限的时间窗返回空集', () => {
    expect(aggregateTurnChanges([change()], Number.NaN)).toEqual([])
  })
})

describe('earliestSnapshots', () => {
  it('每个路径取时间窗内最早的一条（回滚要用回合开始前的原文）', () => {
    const snapshots = earliestSnapshots([
      { path: 'a.ts', kind: 'modify', before: 'v3', after: 'v4', tool: 'edit_file', ts: 1030 },
      { path: 'a.ts', kind: 'modify', before: 'v1', after: 'v2', tool: 'edit_file', ts: 1002 },
      { path: 'b.ts', kind: 'create', before: '', after: 'x', tool: 'write_file', ts: 1001 },
    ], 1000)

    expect(snapshots.get('a.ts')?.before).toBe('v1')
    expect(snapshots.get('b.ts')?.kind).toBe('create')
  })

  it('忽略时间窗外、缺路径或缺时间戳的条目', () => {
    const snapshots = earliestSnapshots([
      { path: 'old.ts', kind: 'modify', before: 'x', after: 'y', tool: '', ts: 900 },
      { path: '  ', kind: 'modify', before: 'x', after: 'y', tool: '', ts: 1010 },
      { path: 'no-ts.ts', kind: 'modify', before: 'x', after: 'y', tool: '', ts: undefined as unknown as number },
      { path: 'ok.ts', kind: 'modify', before: '', after: 'y', tool: '', ts: 1010 },
    ], 1000)

    expect([...snapshots.keys()]).toEqual(['ok.ts'])
  })
})

describe('summarizeTurnChanges', () => {
  it('累加文件数与行数', () => {
    expect(summarizeTurnChanges([
      { path: 'a', kind: 'modify', tool: '', added: 3, removed: 1, ts: 1 },
      { path: 'b', kind: 'create', tool: '', added: 2, removed: 0, ts: 2 },
    ])).toEqual({ files: 2, added: 5, removed: 1 })
  })
})

describe('useTurnFileChanges', () => {
  it('加载会话改动并裁到时间窗内', async () => {
    const rpc = makeRpc()
    rpc.call.mockResolvedValue({
      ok: true,
      changes: [
        { path: 'src/a.ts', kind: 'modify', tool: 'edit_file', added: 2, removed: 1, ts: 1005 },
        { path: 'src/old.ts', kind: 'modify', tool: 'edit_file', added: 9, removed: 9, ts: 800 },
      ],
    })
    const store = useTurnFileChanges({ rpc })

    const ok = await store.load('s1', 1000)

    expect(ok).toBe(true)
    expect(rpc.call).toHaveBeenCalledWith('sessions.fileChanges', { session: 's1' })
    expect(store.changes.value).toHaveLength(1)
    expect(store.changes.value[0].path).toBe('src/a.ts')
    expect(store.summary.value).toEqual({ files: 1, added: 2, removed: 1 })
    expect(store.visible.value).toBe(true)
    expect(store.canRevert.value).toBe(true)
  })

  it('后端没有 sessions.fileChanges 时静默隐藏，且不发起调用', async () => {
    const rpc = makeRpc(['workspace.file_changes.list'])
    const store = useTurnFileChanges({ rpc })

    const ok = await store.load('s1', 1000)

    expect(ok).toBe(false)
    expect(rpc.call).not.toHaveBeenCalled()
    expect(store.unsupported.value).toBe(true)
    expect(store.visible.value).toBe(false)
    expect(store.canRevert.value).toBe(false)
  })

  it('会话键或时间戳缺失时不调用后端', async () => {
    const rpc = makeRpc()
    const store = useTurnFileChanges({ rpc })

    expect(await store.load('', 1000)).toBe(false)
    expect(await store.load('s1', null)).toBe(false)
    expect(rpc.call).not.toHaveBeenCalled()
  })

  it('调用失败时返回 false 且不残留旧行', async () => {
    const rpc = makeRpc()
    rpc.call.mockRejectedValueOnce(new Error('boom'))
    const store = useTurnFileChanges({ rpc })

    expect(await store.load('s1', 1000)).toBe(false)
    expect(store.changes.value).toEqual([])
    expect(store.loading.value).toBe(false)
  })

  it('撤销全部：新建文件无需快照，修改文件用回合开始前的原文', async () => {
    const rpc = makeRpc()
    rpc.call.mockImplementation(async (method: string) => {
      if (method === 'sessions.fileChanges') {
        return {
          ok: true,
          changes: [
            { path: 'created.ts', kind: 'create', added: 2, removed: 0, ts: 1002 },
            { path: 'edited.ts', kind: 'modify', added: 1, removed: 1, ts: 1001 },
          ],
        }
      }
      if (method === 'workspace.file_changes.list') {
        return {
          items: [
            { path: 'edited.ts', kind: 'modify', before: 'older', after: 'newer', tool: '', ts: 1030 },
            { path: 'edited.ts', kind: 'modify', before: 'original', after: 'newer', tool: '', ts: 1001 },
          ],
        }
      }
      return { ok: true, reverted: true }
    })
    const store = useTurnFileChanges({ rpc })
    await store.load('s1', 1000)

    const result = await store.revertAll()

    expect(result).toEqual({ reverted: 2, failures: [] })
    expect(rpc.call).toHaveBeenCalledWith('workspace.file_changes.revert', {
      path: 'created.ts',
      kind: 'create',
      before: '',
    })
    expect(rpc.call).toHaveBeenCalledWith('workspace.file_changes.revert', {
      path: 'edited.ts',
      kind: 'modify',
      before: 'original',
    })
    // 台账不回删条目：回滚成功的行由前端本地移除
    expect(store.changes.value).toEqual([])
    expect(store.visible.value).toBe(false)
  })

  it('缺原始快照时如实报 no_snapshot，不写空串覆盖文件', async () => {
    const rpc = makeRpc()
    rpc.call.mockImplementation(async (method: string) => {
      if (method === 'sessions.fileChanges') {
        return { ok: true, changes: [{ path: 'edited.ts', kind: 'modify', added: 1, removed: 1, ts: 1001 }] }
      }
      if (method === 'workspace.file_changes.list') return { items: [] }
      return { ok: true }
    })
    const store = useTurnFileChanges({ rpc })
    await store.load('s1', 1000)

    const result = await store.revertAll()

    expect(result).toEqual({
      reverted: 0,
      failures: [{ path: 'edited.ts', code: 'no_snapshot', error: '' }],
    })
    expect(rpc.call).not.toHaveBeenCalledWith('workspace.file_changes.revert', expect.anything())
    expect(store.changes.value).toHaveLength(1)
  })

  it('全局台账不可用时区分 ledger_unavailable', async () => {
    const rpc = makeRpc()
    rpc.call.mockImplementation(async (method: string) => {
      if (method === 'sessions.fileChanges') {
        return { ok: true, changes: [{ path: 'edited.ts', kind: 'modify', added: 1, removed: 1, ts: 1001 }] }
      }
      throw new Error('ledger down')
    })
    const store = useTurnFileChanges({ rpc })
    await store.load('s1', 1000)

    const result = await store.revertAll()

    expect(result?.failures).toEqual([{ path: 'edited.ts', code: 'ledger_unavailable', error: '' }])
  })

  it('单个文件回滚失败不影响其余文件，失败行保留在列表里', async () => {
    const rpc = makeRpc()
    rpc.call.mockImplementation(async (method: string, params?: Record<string, unknown>) => {
      if (method === 'sessions.fileChanges') {
        return {
          ok: true,
          changes: [
            { path: 'bad.ts', kind: 'create', added: 1, removed: 0, ts: 1002 },
            { path: 'good.ts', kind: 'create', added: 1, removed: 0, ts: 1001 },
          ],
        }
      }
      if (method === 'workspace.file_changes.list') return { items: [] }
      if (params?.path === 'bad.ts') return { ok: false, error: 'permission denied' }
      return { ok: true }
    })
    const store = useTurnFileChanges({ rpc })
    await store.load('s1', 1000)

    const result = await store.revertAll()

    expect(result).toEqual({
      reverted: 1,
      failures: [{ path: 'bad.ts', code: 'revert_failed', error: 'permission denied' }],
    })
    expect(store.changes.value.map(row => row.path)).toEqual(['bad.ts'])
  })
})
