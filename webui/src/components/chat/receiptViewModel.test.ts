import { describe, it, expect } from 'vitest'
import {
  formatBytes,
  hasReceiptContent,
  missingArtifacts,
  receiptCounts,
  receiptTone,
  sortGaps,
  verificationCounts,
  type TaskReceipt,
} from './receiptViewModel'

function receipt(overrides: Partial<TaskReceipt> = {}): TaskReceipt {
  return {
    ok: true,
    counts: {
      userTurns: 1,
      assistantTurns: 1,
      toolCalls: 3,
      toolFailures: 0,
      toolUnknown: 0,
      changes: 2,
      artifacts: 1,
      artifactsPresent: 1,
    },
    changes: [
      { path: 'src/a.ts', kind: 'modify', tool: 'edit_file', ts: 1 },
      { path: 'src/b.ts', kind: 'create', tool: 'write_file', ts: 2 },
    ],
    artifacts: [
      { id: 'a1', name: 'deck.pptx', path: 'D:/ws/deck.pptx', kind: 'generated', size: 2048, ts: 3, present: true },
    ],
    verification: { status: 'passed', passedChecks: 1, checks: [{ tool: 'shell_exec', command: 'cargo test', ok: true, elapsedMs: 900 }] },
    budget: { available: true, runs: 1, tokensIn: 10, tokensOut: 20, model: 'm', policy: 'standard', runsElapsedMs: 1000 },
    unattested: [],
    ...overrides,
  }
}

describe('receiptTone', () => {
  it('maps verification status to a visual tone without changing meaning', () => {
    expect(receiptTone('passed')).toBe('ok')
    expect(receiptTone('failed')).toBe('danger')
    expect(receiptTone('unknown')).toBe('warn')
    expect(receiptTone('unverified')).toBe('muted')
    // 缺失状态一律按「未验证」处理，绝不默认成通过
    expect(receiptTone(undefined)).toBe('muted')
  })
})

describe('hasReceiptContent', () => {
  it('hides the card for pure conversation', () => {
    const quiet = receipt({
      counts: {
        userTurns: 1,
        assistantTurns: 1,
        toolCalls: 0,
        toolFailures: 0,
        toolUnknown: 0,
        changes: 0,
        artifacts: 0,
        artifactsPresent: 0,
      },
      changes: [],
      artifacts: [],
    })
    expect(hasReceiptContent(quiet)).toBe(false)
  })

  it('shows the card when anything was touched', () => {
    expect(hasReceiptContent(receipt())).toBe(true)
    expect(hasReceiptContent(null)).toBe(false)
    expect(hasReceiptContent({ ok: false })).toBe(false)
  })
})

describe('receiptCounts', () => {
  it('falls back to array length when the backend omits counts', () => {
    const out = receiptCounts({ ok: true, changes: [{ path: 'a', kind: 'create', tool: 'w', ts: 1 }] })
    expect(out.changes).toBe(1)
    expect(out.artifacts).toBe(0)
    expect(out.tools).toBe(0)
  })
})

describe('verificationCounts', () => {
  it('separates passed / failed / unknown checks', () => {
    const out = verificationCounts(receipt({
      verification: {
        status: 'failed',
        checks: [
          { tool: 'shell_exec', command: 'cargo test', ok: true, elapsedMs: 1 },
          { tool: 'shell_exec', command: 'npm run build', ok: false, elapsedMs: 2 },
          { tool: 'shell_exec', command: 'tsc', ok: null, elapsedMs: 3 },
        ],
      },
    }))
    expect(out).toEqual({ checks: 3, passed: 1, failed: 1, unknown: 1 })
  })
})

describe('sortGaps', () => {
  it('puts "no verification" first and unknown codes last', () => {
    const sorted = sortGaps([
      { code: 'usage_ledger_unavailable' },
      { code: 'no_verification' },
      { code: 'something_new' },
      { code: 'changes_without_verification', count: 3 },
    ])
    expect(sorted.map(g => g.code)).toEqual([
      'no_verification',
      'changes_without_verification',
      'usage_ledger_unavailable',
      'something_new',
    ])
  })

  it('tolerates a missing list', () => {
    expect(sortGaps(undefined)).toEqual([])
  })
})

describe('missingArtifacts', () => {
  it('only reports artifacts the backend says are gone', () => {
    const gone = missingArtifacts(receipt({
      artifacts: [
        { id: 'a', name: 'a.pdf', path: 'p', kind: 'generated', size: 1, ts: 1, present: true },
        { id: 'b', name: 'b.pdf', path: 'q', kind: 'generated', size: 1, ts: 2, present: false },
      ],
    }))
    expect(gone.map(a => a.name)).toEqual(['b.pdf'])
  })
})

describe('formatBytes', () => {
  it('formats sizes and never prints NaN', () => {
    expect(formatBytes(0)).toBe('0 B')
    expect(formatBytes(512)).toBe('512 B')
    expect(formatBytes(2048)).toBe('2 KB')
    expect(formatBytes(Number.NaN)).toBe('0 B')
  })
})
