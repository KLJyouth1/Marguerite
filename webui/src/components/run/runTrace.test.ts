import { describe, expect, it } from 'vitest'

import type { ChatStreamTimelineItem } from '@/types/chat'
import type { ChatHistoryMessage } from '@/types/rpc'
import { nodeStepsFromHistoryMessage, nodeStepsFromTimeline } from './runTrace'

describe('run trace tool identity', () => {
  it('preserves the original tool name when flattening chat timeline items', () => {
    const items: ChatStreamTimelineItem[] = [{
      type: 'tool-group',
      key: 'web-fetch-group',
      group: {
        groupId: 'web-fetch-group',
        operationKey: 'web.read',
        label: 'Read web page',
        iconName: 'monitor',
        calls: [{
          toolId: 'fetch-1',
          renderKey: 'fetch-1',
          name: 'web_fetch',
          displayName: 'web_fetch',
          inputPreview: '',
          isRunning: false,
          status: 'success',
          isError: false,
          result: '2026 results',
          resultPreview: '2026 results',
          isOpen: false,
        }],
        secondary: '',
        isRunning: false,
        isError: false,
        status: 'success',
      },
    }]

    expect(nodeStepsFromTimeline(items)[0]).toMatchObject({
      toolName: 'web_fetch',
      operationKey: 'web.read',
    })
  })

  it('preserves the original tool name when flattening persisted history', () => {
    const message = {
      role: 'assistant',
      tool_calls: [{
        tool_use_id: 'search-1',
        name: 'MCPURLSearch',
        result: 'Found 7 results.',
      }],
    } as ChatHistoryMessage

    expect(nodeStepsFromHistoryMessage(message)[0]).toMatchObject({
      toolName: 'MCPURLSearch',
      operationKey: 'tool.mcpurlsearch',
    })
  })

  it('reads the gateway-persisted tool_call rows and their outcome signal', () => {
    const message = {
      role: 'assistant',
      tool_calls: [
        {
          type: 'tool_call',
          tool_use_id: 'hist-tool-3',
          name: 'write_file',
          input: { path: 'a.txt' },
          result: '',
          ok: true,
          execution_status: { status: 'success' },
        },
        {
          type: 'tool_call',
          tool_use_id: 'hist-tool-4',
          name: 'shell',
          input: '{"cmd":"false"}',
          result: 'exit 1',
          ok: false,
          is_error: true,
          execution_status: { status: 'error' },
        },
        {
          type: 'tool_call',
          tool_use_id: 'hist-tool-5',
          name: 'read_file',
          input: '{"path":"b.txt"}',
          result: '',
        },
      ],
    } as ChatHistoryMessage

    const steps = nodeStepsFromHistoryMessage(message)
    expect(steps.map(step => [step.toolName, step.state])).toEqual([
      // 空输出的成功有权威 ok 信号 → 不得降级成「未完成」。
      ['write_file', 'output-available'],
      ['shell', 'output-error'],
      // 无信号（旧数据 / 回合中断）→ 保持未知，不猜成功。
      ['read_file', 'input-streaming'],
    ])
    expect(steps[1].isError).toBe(true)
  })
})
