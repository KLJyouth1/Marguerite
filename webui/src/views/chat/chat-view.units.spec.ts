import { describe, it, expect, afterEach, vi } from 'vitest'
import { activeSnapshot } from './active-snapshot'
import { readAuthToken } from './read-auth-token'
import { useChatViewState } from './useChatViewState'
import type { ProjectWorkspaceItem } from '@/composables/useProjectWorkspaces'

describe('activeSnapshot (pure projection)', () => {
  it('projects workspace fields and defaults removed=false', () => {
    const ws = {
      id: 'w1',
      name: 'Proj',
      path: '/p',
      available: true,
    } as unknown as ProjectWorkspaceItem
    expect(activeSnapshot(ws)).toEqual({
      id: 'w1',
      name: 'Proj',
      path: '/p',
      available: true,
      removed: false,
    })
  })

  it('carries availabilityReason when present', () => {
    const ws = {
      id: 'w2',
      name: 'P2',
      path: '/q',
      available: false,
      availabilityReason: 'offline',
    } as unknown as ProjectWorkspaceItem
    expect(activeSnapshot(ws).availabilityReason).toBe('offline')
  })
})

describe('readAuthToken (pure, sessionStorage)', () => {
  afterEach(() => {
    vi.unstubAllGlobals()
  })

  it('returns empty string when sessionStorage is unavailable (node env catch path)', () => {
    expect(readAuthToken()).toBe('')
  })

  it('reads marguerite.wsToken when present', () => {
    vi.stubGlobal('sessionStorage', {
      getItem: (k: string) => (k === 'marguerite.wsToken' ? 'tok-123' : null),
    })
    expect(readAuthToken()).toBe('tok-123')
  })

  it('returns empty string on storage access error', () => {
    vi.stubGlobal('sessionStorage', {
      getItem: () => {
        throw new Error('blocked')
      },
    })
    expect(readAuthToken()).toBe('')
  })
})

describe('useChatViewState (shared state layer)', () => {
  it('initializes toolResultModal closed with empty content', () => {
    const state = useChatViewState()
    expect(state.toolResultModal.value).toEqual({
      open: false,
      title: '',
      content: '',
      context: undefined,
    })
  })

  it('initializes inputText as an empty string ref', () => {
    const state = useChatViewState()
    expect(state.inputText.value).toBe('')
    // 写入新值后仍为同一 Ref 实例（保证 ChatView 内所有引用共享同一状态）。
    state.inputText.value = 'hello'
    expect(state.inputText.value).toBe('hello')
  })

  it('initializes composing as a false boolean ref', () => {
    const state = useChatViewState()
    expect(state.composing.value).toBe(false)
    // 写入后仍为同一 Ref 实例（ChatComposer 的 @composition-change 借此共享状态）。
    state.composing.value = true
    expect(state.composing.value).toBe(true)
  })

  it('initializes aborted as a false boolean ref', () => {
    const state = useChatViewState()
    expect(state.aborted.value).toBe(false)
    // 写入后仍为同一 Ref 实例（发送/中断逻辑借此共享中止态）。
    state.aborted.value = true
    expect(state.aborted.value).toBe(true)
  })
})
