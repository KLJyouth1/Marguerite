import { describe, expect, it } from 'vitest'
import type { ChatRenderedMessage } from '@/types/chat'
import { parseSubagentPayload, subagentPanelEntries, subagentTone } from './subagent'

function message(text: string, ts: string | number = '2026-10-26T10:00:00Z'): ChatRenderedMessage {
  return { text, displayRole: 'subagent', ts } as ChatRenderedMessage
}

describe('subagentTone', () => {
  it('maps completion states to the same tones the card uses', () => {
    expect(subagentTone('completed')).toBe('ok')
    expect(subagentTone('FAILED')).toBe('err')
    expect(subagentTone('in_progress')).toBe('run')
    expect(subagentTone('')).toBe('idle')
  })
})

describe('subagentPanelEntries', () => {
  it('dedupes by child session and keeps the latest completion, newest first', () => {
    const entries = subagentPanelEntries([
      message(JSON.stringify({
        child_session_key: 'child-a',
        status: 'running',
        result: { text: 'working' },
      }), '2026-10-26T10:00:00Z'),
      message(JSON.stringify({
        child_session_key: 'child-b',
        status: 'completed',
        result: { summary: 'b done' },
      }), '2026-10-26T10:01:00Z'),
      message(JSON.stringify({
        child_session_key: 'child-a',
        status: 'completed',
        result: { text: 'a done' },
      }), '2026-10-26T10:02:00Z'),
    ])

    expect(entries.map(entry => entry.key)).toEqual(['child-a', 'child-b'])
    expect(entries[0]).toMatchObject({ tone: 'ok', summary: 'a done' })
    expect(entries[1]).toMatchObject({ tone: 'ok', summary: 'b done' })
  })

  it('falls back to terminal_reason and session_key, ignoring unusable rows', () => {
    const entries = subagentPanelEntries([
      message(JSON.stringify({ session_key: 'child-c', terminal_reason: 'failed' })),
      message(JSON.stringify({ status: 'completed' })),
      message('not json'),
      { text: '', displayRole: 'assistant', ts: '' } as ChatRenderedMessage,
    ])

    expect(entries).toHaveLength(1)
    expect(entries[0]).toMatchObject({ key: 'child-c', tone: 'err', summary: '' })
  })
})

describe('parseSubagentPayload', () => {
  it('returns null for malformed JSON', () => {
    expect(parseSubagentPayload('{')).toBeNull()
    expect(parseSubagentPayload('null')).toBeNull()
  })
})
