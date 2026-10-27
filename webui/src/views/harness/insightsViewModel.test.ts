import { describe, expect, it } from 'vitest'
import {
  assistantTurnTotal,
  insightGaps,
  taskTypeRows,
  thinkingShare,
  toolFailureTotal,
  topSessions,
  topTools,
  type InsightsPayload,
} from './insightsViewModel'

const payload: InsightsPayload = {
  ok: true,
  totals: {
    sessions: 2,
    messages: 12,
    assistantTurns: 5,
    toolCalls: 4,
    toolFailures: 1,
    toolUnknown: 1,
    thinkingMessages: 2,
  },
  taskTypes: {
    directAnswer: 1,
    reasoning: 1,
    toolAssisted: 2,
    toolAndReasoning: 1,
  },
  tools: [
    { name: 'shell_exec', calls: 1, failures: 1, unknown: 0, avgMs: 30, p95Ms: 30 },
    { name: 'read_file', calls: 3, failures: 0, unknown: 1, avgMs: 10, p95Ms: 20 },
  ],
  sessions: [
    { key: 'b', messages: 4, toolCalls: 1, thinkingChars: 0 },
    { key: 'a', messages: 8, toolCalls: 1, thinkingChars: 120 },
    { key: 'c', messages: 1, toolCalls: 0, thinkingChars: 0 },
  ],
  gaps: ['部分工具结果缺少成功/失败标记', '  部分工具结果缺少成功/失败标记  ', ''],
}

describe('insightsViewModel', () => {
  it('把四类任务形态固定输出，并按 assistant 轮次算占比', () => {
    const rows = taskTypeRows(payload)
    expect(rows.map(row => row.key)).toEqual([
      'directAnswer',
      'reasoning',
      'toolAssisted',
      'toolAndReasoning',
    ])
    expect(rows.map(row => row.count)).toEqual([1, 1, 2, 1])
    expect(assistantTurnTotal(payload)).toBe(5)
    expect(rows[2].share).toBeCloseTo(0.4)
  })

  it('空数据不除零、不造假', () => {
    expect(taskTypeRows(null)).toEqual([
      { key: 'directAnswer', count: 0, share: 0 },
      { key: 'reasoning', count: 0, share: 0 },
      { key: 'toolAssisted', count: 0, share: 0 },
      { key: 'toolAndReasoning', count: 0, share: 0 },
    ])
    expect(assistantTurnTotal(null)).toBe(0)
    expect(thinkingShare(null)).toBe(0)
    expect(toolFailureTotal(null)).toBe(0)
    expect(topTools(null)).toEqual([])
    expect(topSessions(null)).toEqual([])
    expect(insightGaps(null)).toEqual([])
  })

  it('工具排行按次数降序、同次数按名称升序', () => {
    expect(topTools(payload).map(row => row.name)).toEqual(['read_file', 'shell_exec'])
  })

  it('会话排行按工具调用优先、其次消息数', () => {
    expect(topSessions(payload).map(row => row.key)).toEqual(['a', 'b', 'c'])
  })

  it('思考占比与失败数只读真实字段', () => {
    expect(thinkingShare(payload)).toBeCloseTo(0.4)
    expect(toolFailureTotal(payload)).toBe(1)
  })

  it('缺口提示去空去重', () => {
    expect(insightGaps(payload)).toEqual(['部分工具结果缺少成功/失败标记'])
  })
})
