import { describe, expect, it } from 'vitest'

import type { PendingApproval } from '@/stores/app'

import {
  approvalToAttention,
  approvalsToAttention,
  ATTENTION_TITLE_MAX,
  createFirstSeenTracker,
  truncateAttentionTitle,
} from './attentionSources'

const NOW = 1_800_000_000_000

function approval(over: Partial<PendingApproval> = {}): PendingApproval {
  return {
    approvalId: 'ap-1',
    sessionKey: 'sess-1',
    tool: 'shell.exec',
    command: 'npm run build',
    ...over,
  }
}

describe('attentionSources — 首次观测时钟', () => {
  it('同一个 id 反复上报只记第一次的时间（重渲染不得让等待时间归零）', () => {
    const tracker = createFirstSeenTracker()
    expect(tracker.stamp('ap-1', NOW)).toBe(NOW)
    expect(tracker.stamp('ap-1', NOW + 60_000)).toBe(NOW)
  })

  it('retain 清掉已消失的 id，避免长驻进程里 Map 无限增长', () => {
    const tracker = createFirstSeenTracker()
    tracker.stamp('ap-1', NOW)
    tracker.stamp('ap-2', NOW)
    expect(tracker.size()).toBe(2)

    tracker.retain(['ap-2'])

    expect(tracker.size()).toBe(1)
    expect(tracker.stamp('ap-2', NOW + 1)).toBe(NOW)
  })

  it('已裁决后同一 approvalId 再次出现时重新计时（是新的一笔债）', () => {
    const tracker = createFirstSeenTracker()
    tracker.stamp('ap-1', NOW)
    tracker.retain([])
    expect(tracker.stamp('ap-1', NOW + 5_000)).toBe(NOW + 5_000)
  })
})

describe('attentionSources — 映射', () => {
  it('缺 sessionKey 的审批不进货架（无法路由），但也不会伪造条目', () => {
    expect(approvalToAttention(approval({ sessionKey: '' }), createFirstSeenTracker(), NOW)).toBeNull()
    expect(approvalToAttention(approval({ approvalId: '  ' }), createFirstSeenTracker(), NOW)).toBeNull()
  })

  it('正常审批映射为 approval 类，并带上可路由的 target', () => {
    const item = approvalToAttention(approval(), createFirstSeenTracker(), NOW)

    expect(item).not.toBeNull()
    expect(item?.kind).toBe('approval')
    expect(item?.id).toBe('approval:ap-1')
    expect(item?.at).toBe(NOW)
    expect(item?.target).toEqual({
      kind: 'approval',
      approvalId: 'ap-1',
      sessionKey: 'sess-1',
    })
    expect(item?.title).toBe('shell.exec · npm run build')
  })

  it('标题压成单行并硬截断，且截断后仍在阈值内（无障碍名与屏幕一致）', () => {
    const long = `${'x'.repeat(200)}\n second line`
    const out = truncateAttentionTitle(long)
    expect(out).toHaveLength(ATTENTION_TITLE_MAX)
    expect(out.endsWith('…')).toBe(true)
    expect(out).not.toContain('\n')
  })

  it('缺命令时退回工具名，缺工具时退回命令，两者都缺时退回 approvalId', () => {
    expect(approvalToAttention(approval({ command: '' }), createFirstSeenTracker(), NOW)?.title).toBe(
      'shell.exec',
    )
    expect(approvalToAttention(approval({ tool: '' }), createFirstSeenTracker(), NOW)?.title).toBe(
      'npm run build',
    )
    expect(
      approvalToAttention(approval({ tool: '', command: '' }), createFirstSeenTracker(), NOW)?.title,
    ).toBe('ap-1')
  })

  it('批量映射会顺手 retain，只保留仍在挂起的 id', () => {
    const tracker = createFirstSeenTracker()
    tracker.stamp('ap-gone', NOW)

    const items = approvalsToAttention([approval()], tracker, NOW)

    expect(items).toHaveLength(1)
    expect(tracker.size()).toBe(1)
  })
})
