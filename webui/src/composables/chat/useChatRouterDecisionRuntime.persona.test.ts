import { describe, expect, it, vi } from 'vitest'
import { ref } from 'vue'
import type { ChatMessage } from '@/types/chat'
import type { RouterDecisionPayload } from '@/types/rpc'
import { useChatRouterDecisionRuntime } from '@/composables/chat/useChatRouterDecisionRuntime'
import { useChatRouteFeedback } from '@/composables/chat/useChatRouteFeedback'

function makeRuntime(messages: ChatMessage[] = []) {
  return useChatRouterDecisionRuntime({
    messages: ref(messages),
    sessionKey: ref('sess'),
    isStreaming: ref(true),
    autoScroll: ref(true),
    modelRoutingMode: ref('squilla_router'),
    streamBubble: ref(true),
    streamHasVisibleOutput: ref(false),
    startStreaming: vi.fn(),
    resetStreamForRouterReplay: vi.fn(),
    resetStreamIdleTimer: vi.fn(),
    setStreamActivity: vi.fn(),
    scrollToBottom: vi.fn(),
  })
}

// P0-2 producer side: the router-decision append path must publish the expert
// persona into the shared single source, and clear it when the frame omits it.
describe('router decision → activePersona (P0-2)', () => {
  it('publishes persona.agentId/name from an expert session payload', () => {
    const runtime = makeRuntime([{ role: 'user', text: 'q', ts: 0 }])
    const { activePersona } = useChatRouteFeedback()
    activePersona.value = null

    const payload = {
      tier: 'c1',
      model: 'gpt-test',
      persona: {
        agent_id: 'code-engineer',
        name: 'Code Engineer',
        description: 'Writes code',
        is_builtin: true,
      },
    } as unknown as RouterDecisionPayload
    runtime.queueRouterDecision(payload)

    expect(activePersona.value).not.toBeNull()
    expect(activePersona.value!.agentId).toBe('code-engineer')
    expect(activePersona.value!.name).toBe('Code Engineer')
    expect(activePersona.value!.isBuiltin).toBe(true)
  })

  it('accepts camelCase persona keys as well', () => {
    const runtime = makeRuntime([{ role: 'user', text: 'q', ts: 0 }])
    const { activePersona } = useChatRouteFeedback()

    runtime.queueRouterDecision({
      tier: 'c1',
      persona: { agentId: 'researcher', name: 'Researcher' },
    } as unknown as RouterDecisionPayload)

    expect(activePersona.value?.agentId).toBe('researcher')
  })

  it('clears the persona when a subsequent frame omits it (non-expert session)', () => {
    const runtime = makeRuntime([{ role: 'user', text: 'q', ts: 0 }])
    const { activePersona } = useChatRouteFeedback()
    activePersona.value = { agentId: 'code-engineer', name: 'Code Engineer', description: '', isBuiltin: true }

    runtime.queueRouterDecision({ tier: 'c1' } as unknown as RouterDecisionPayload)

    expect(activePersona.value).toBeNull()
  })
})