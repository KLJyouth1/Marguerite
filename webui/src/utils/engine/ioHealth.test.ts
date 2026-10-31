import { describe, expect, it } from 'vitest'
import { ioAvailable, ioRows, type IoHealth } from './ioHealth'

// 假 t 回显「键 + 参数」：断言只关心选了哪条键、参数对不对，
// 不把 6 个 locale 的措辞写进单测（措辞会改，语义不该改）。
const t = (key: string, params?: Record<string, unknown>) => {
  if (!params) return key
  return `${key}(${Object.entries(params)
    .map(([k, v]) => `${k}=${String(v)}`)
    .join(',')})`
}

const row = (io: IoHealth, id: string) => ioRows(io, t).find((r) => r.id === id)

describe('ioRows（W7 写入量自观测）', () => {
  it('旧网关没有 io 段时整块不渲染，而不是渲染一片「—」', () => {
    expect(ioRows(undefined, t)).toEqual([])
    expect(ioRows(null, t)).toEqual([])
    expect(ioRows({}, t)).toEqual([])
    expect(ioAvailable(undefined)).toBe(false)
    expect(ioAvailable({})).toBe(false)
  })

  it('写入行给出次数 + 字节 + 口径（口径机器键要翻译成人话）', () => {
    const io: IoHealth = { process_writes: { count: 12, bytes: 2048, scope: 'json_append_exits' } }
    expect(ioAvailable(io)).toBe(true)
    expect(row(io, 'writes')?.value).toBe(
      '12 · 2.0 KB · engineHealth.io.scopeJsonExits',
    )
  })

  it('未知口径键如实标「未知」并保留原键，绝不编一个听起来对的解释', () => {
    const io: IoHealth = { process_writes: { count: 1, bytes: 5, scope: 'sqlite_rows' } }
    expect(row(io, 'writes')?.value).toContain('engineHealth.io.unknown sqlite_rows')
    const missing: IoHealth = { process_writes: { count: 1, bytes: 5 } }
    expect(row(missing, 'writes')?.value).toContain('engineHealth.io.unknown')
  })

  it('journal_mode 读不到时说「未知」且不报警 —— 报警会说成「WAL 没开」，那是撒谎', () => {
    const io: IoHealth = {
      process_writes: { count: 0, bytes: 0 },
      databases: { sessions: { present: true, bytes: 100, journal_mode: null } },
    }
    const r = row(io, 'db:sessions.db')
    expect(r?.value).toContain('engineHealth.io.journalUnknown')
    expect(r?.warn ?? false).toBe(false)
  })

  it('journal_mode 回读成非 wal 时报警（Windows 上 -wal/-shm 被占用会静默失败）', () => {
    const io: IoHealth = {
      process_writes: { count: 0, bytes: 0 },
      databases: { usage: { present: true, bytes: 100, journal_mode: 'delete' } },
    }
    const r = row(io, 'db:usage.db')
    expect(r?.warn).toBe(true)
    expect(r?.value).toContain('delete')
    expect(r?.value).toContain('engineHealth.io.journalNotWal')
  })

  it('wal 生效时把 -wal 的待合并量并排显示；没有该文件时不补 0', () => {
    const withWal: IoHealth = {
      process_writes: { count: 0, bytes: 0 },
      databases: { audit: { present: true, bytes: 4096, journal_mode: 'wal', wal_bytes: 24 } },
    }
    expect(row(withWal, 'db:audit.db')?.value).toContain('(+wal 24 B)')
    const noWal: IoHealth = {
      process_writes: { count: 0, bytes: 0 },
      databases: { audit: { present: true, bytes: 4096, journal_mode: 'wal' } },
    }
    expect(row(noWal, 'db:audit.db')?.value).not.toContain('+wal')
  })

  it('库文件不存在时如实标「不存在」，不是 0 字节', () => {
    const io: IoHealth = { process_writes: { count: 0, bytes: 0 }, databases: { sessions: {} } }
    expect(row(io, 'db:sessions.db')?.value).toBe('engineHealth.io.absent')
  })

  it('目录超预算才报警；预算内静默', () => {
    const over: IoHealth = {
      process_writes: { count: 0, bytes: 0 },
      context_ledger: { present: true, files: 5, bytes: 300 * 1024 * 1024, budget_bytes: 256 * 1024 * 1024 },
    }
    const r = row(over, 'context')
    expect(r?.warn).toBe(true)
    expect(r?.value).toBe('engineHealth.io.files(count=5) · 300 MB / 256 MB')

    const ok: IoHealth = {
      process_writes: { count: 0, bytes: 0 },
      context_ledger: { present: true, files: 5, bytes: 1024, budget_bytes: 256 * 1024 * 1024 },
    }
    expect(row(ok, 'context')?.warn ?? false).toBe(false)
  })

  it('目录还不存在（全新数据目录）时标「不存在」，不得显示成 0 文件 0 字节', () => {
    const io: IoHealth = { process_writes: { count: 0, bytes: 0 }, tool_spool: { present: false } }
    const r = row(io, 'spool')
    expect(r?.value).toBe('engineHealth.io.absent')
    expect(r?.warn ?? false).toBe(false)
  })

  it('决策日志把阈值一起显示，并把历史代数量带出来', () => {
    const io: IoHealth = {
      process_writes: { count: 0, bytes: 0 },
      decision_log: {
        present: true,
        bytes: 12,
        rotate_at_bytes: 10485760,
        generations: [
          { generation: 0, bytes: 8 },
          { generation: 1, bytes: 4 },
          { generation: 2, bytes: null },
        ],
      },
    }
    const r = row(io, 'decision_log')
    expect(r?.value).toBe('12 B / 10.0 MB +1')
  })

  it('侧车日志单独一行：它是子进程写的，不在本进程写入口径里', () => {
    const io: IoHealth = {
      process_writes: { count: 3, bytes: 100 },
      sidecar_logs: { present: true, files: 2, bytes: 20 * 1024 * 1024, budget_bytes: 200 * 1024 * 1024 },
    }
    const r = row(io, 'sidecar_logs')
    expect(r?.value).toBe('engineHealth.io.files(count=2) · 20.0 MB / 200 MB')
    expect(r?.warn ?? false).toBe(false)
    // 写入行的数字绝不能顺手把侧车日志算进去 —— 口径说了不算，行分开才算。
    expect(row(io, 'writes')?.value).toBe('3 · 100 B · engineHealth.io.unknown')

    const never: IoHealth = { process_writes: { count: 0, bytes: 0 }, sidecar_logs: { present: false } }
    expect(row(never, 'sidecar_logs')?.value).toBe('engineHealth.io.absent')
  })

  it('每个数据源都必须有一行，缺哪一块就露出「不存在」而不是少一行', () => {
    const io: IoHealth = { process_writes: { count: 0, bytes: 0 } }
    const ids = ioRows(io, t).map((r) => r.id)
    expect(ids).toEqual([
      'writes',
      'db:sessions.db',
      'db:usage.db',
      'db:audit.db',
      'context',
      'plans',
      'decision_log',
      'spool',
      'sidecar_logs',
    ])
  })
})
