// 运行流投影（buildSidebarTasks）：右侧「任务」列要能看见
// 思考正文 / 实时工具 / 历史步骤 / 工具结果 / 本轮总结。
//
// 锁的契约：
//   1. 顺序稳定：思考 → 实时工具 → 历史 → 总结；
//   2. 每条都带可展开正文（input/output），而不是只有一行标题；
//   3. 流式期间不把半截正文当"执行总结"；
//   4. 超长正文截断到 4000 字并标 `…`，避免侧栏被一条命令输出撑爆。
import { describe, expect, it } from 'vitest'

import { buildSidebarTasks } from './useChatSidebarPanels'
import type { ChatMessage, ChatToolCall } from '@/types/chat'

function toolCall(overrides: Partial<ChatToolCall> = {}): ChatToolCall {
  return {
    toolId: 't1',
    name: 'run_command',
    displayName: '运行命令',
    inputPreview: 'npm test',
    inputRaw: '{"cmd":"npm test"}',
    isRunning: true,
    status: '',
    isError: false,
    result: '',
    resultPreview: '',
    isOpen: false,
    ...overrides,
  }
}

function assistant(overrides: Partial<ChatMessage> = {}): ChatMessage {
  return { role: 'assistant', text: '', ts: 1, ...overrides }
}

describe('buildSidebarTasks', () => {
  it('puts live thinking first and keeps it expandable', () => {
    const tasks = buildSidebarTasks({
      thinking: '先读一下配置，再决定改哪个文件',
      streamToolCalls: [],
      messages: [],
      streaming: true,
    })
    expect(tasks).toHaveLength(1)
    expect(tasks[0]).toMatchObject({
      kind: 'thinking',
      status: 'running',
      output: '先读一下配置，再决定改哪个文件',
    })
    // 数据层不写死任何语言的标题，由展示层本地化
    expect(tasks[0].title).toBe('')
  })

  it('maps a live tool call to kind=tool with its raw input and result', () => {
    const tasks = buildSidebarTasks({
      thinking: '',
      streamToolCalls: [toolCall()],
      messages: [],
      streaming: true,
    })
    expect(tasks).toHaveLength(1)
    expect(tasks[0]).toMatchObject({
      id: 'tool:t1',
      kind: 'tool',
      title: '运行命令',
      status: 'running',
      tools: ['run_command'],
      input: '{"cmd":"npm test"}',
    })
  })

  it('marks an errored tool call as error and surfaces the error text as output', () => {
    const tasks = buildSidebarTasks({
      thinking: '',
      streamToolCalls: [toolCall({ isRunning: false, isError: true, status: 'error', result: 'boom' })],
      messages: [],
      streaming: false,
    })
    expect(tasks[0].status).toBe('error')
    expect(tasks[0].output).toBe('boom')
  })

  it('projects history: reasoning, timeline steps and tool calls (in that order)', () => {
    const msg = assistant({
      turnId: 'turn-1',
      text: '已完成',
      reasoning: { text: '我看了目录', seconds: 6 },
      timeline: [{ kind: 'status', label: '读取文件', status: 'done', duration: '1s' }],
      tool_calls: [
        { name: 'read_file', input: { path: 'a.ts' }, result: 'export {}' },
      ] as ChatMessage['tool_calls'],
    })
    const tasks = buildSidebarTasks({
      thinking: '',
      streamToolCalls: [],
      messages: [msg],
      streaming: false,
    })
    expect(tasks.map(t => t.kind)).toEqual(['thinking', 'step', 'tool', 'summary'])
    expect(tasks[0].output).toBe('我看了目录')
    expect(tasks[1]).toMatchObject({ title: '读取文件', duration: '1s' })
    expect(tasks[2]).toMatchObject({ title: 'read_file', output: 'export {}' })
    // 对象入参按 JSON 缩进展开，便于直接读
    expect(tasks[2].input).toContain('"path": "a.ts"')
    expect(tasks[3]).toMatchObject({ kind: 'summary', output: '已完成' })
  })

  it('does not treat the in-progress answer as a finished summary', () => {
    const tasks = buildSidebarTasks({
      thinking: '',
      streamToolCalls: [],
      messages: [assistant({ text: '半截答案' })],
      streaming: true,
    })
    expect(tasks.some(t => t.kind === 'summary')).toBe(false)
  })

  it('clips very long payloads so one command cannot flood the panel', () => {
    const huge = 'x'.repeat(5000)
    const tasks = buildSidebarTasks({
      thinking: '',
      streamToolCalls: [toolCall({ isRunning: false, result: huge })],
      messages: [],
      streaming: false,
    })
    expect(tasks[0].output!.endsWith('\n…')).toBe(true)
    expect(tasks[0].output!.length).toBeLessThan(4100)
  })

  it('ignores non-assistant messages entirely', () => {
    const tasks = buildSidebarTasks({
      thinking: '',
      streamToolCalls: [],
      messages: [
        { role: 'user', text: '帮我改一下', ts: 1 },
        { role: 'system', text: 'resume', ts: 2 },
      ],
      streaming: false,
    })
    expect(tasks).toEqual([])
  })
})
