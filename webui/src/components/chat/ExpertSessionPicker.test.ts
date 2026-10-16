// @vitest-environment happy-dom
// P0-2 consumer side: the header "new expert session" picker lists agents from
// the shared useAgentOptions singleton and opens a `agent:{id}:webchat:{suffix}`
// session on select. Mid-session persona switching is intentionally unsupported.
import { createApp, nextTick, type App } from 'vue'
import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest'

const push = vi.fn()
vi.mock('vue-router', () => ({
  useRouter: () => ({ push }),
}))

// Keep the shared agents singleton out of the RPC store: a fixed list is enough
// to exercise the picker's trigger → menu → select → navigate path.
vi.mock('@/composables/useAgentOptions', async () => {
  const { ref } = await import('vue')
  return {
    useAgentOptions: () => ({
      agents: ref([
        { id: 'code-engineer', name: '代码工程师' },
        { id: 'researcher', name: '研究员' },
      ]),
      agentListError: ref(false),
      loadAgents: vi.fn(),
    }),
  }
})

import i18n, { loadLocaleMessages } from '@/i18n'
import ExpertSessionPicker from '@/components/chat/ExpertSessionPicker.vue'

let app: App<Element> | null = null

afterEach(() => {
  app?.unmount()
  app = null
  document.body.innerHTML = ''
  push.mockReset()
})

beforeAll(async () => {
  await loadLocaleMessages('zh-Hans')
  i18n.global.locale.value = 'zh-Hans'
})

async function mountPicker() {
  const el = document.createElement('div')
  document.body.appendChild(el)
  app = createApp(ExpertSessionPicker)
  app.use(i18n)
  app.mount(el)
  await nextTick()
  return el
}

function trigger(el: Element): HTMLButtonElement {
  return el.querySelector('[data-testid="expert-session-trigger"]') as HTMLButtonElement
}

describe('ExpertSessionPicker (P0-2)', () => {
  it('renders the localized "new expert session" trigger', async () => {
    const el = await mountPicker()
    const button = trigger(el)
    expect(button).not.toBeNull()
    expect(button.getAttribute('aria-haspopup')).toBe('menu')
    expect(button.getAttribute('aria-expanded')).toBe('false')
    expect(el.textContent).toContain('新专家会话')
  })

  it('lists agents only after the trigger is opened', async () => {
    const el = await mountPicker()
    expect(el.querySelector('[data-testid="expert-session-menu"]')).toBeNull()
    trigger(el).click()
    await nextTick()
    expect(trigger(el).getAttribute('aria-expanded')).toBe('true')
    expect(el.querySelector('[data-testid="expert-session-menu"]')).not.toBeNull()
    expect(el.querySelector('[data-testid="expert-session-code-engineer"]')).not.toBeNull()
    expect(el.querySelector('[data-testid="expert-session-researcher"]')).not.toBeNull()
  })

  it('opens agent:{id}:webchat:{suffix} and closes the menu on select', async () => {
    const el = await mountPicker()
    trigger(el).click()
    await nextTick()
    ;(el.querySelector('[data-testid="expert-session-code-engineer"]') as HTMLButtonElement).click()
    await nextTick()
    expect(push).toHaveBeenCalledTimes(1)
    const arg = push.mock.calls[0][0] as { path: string; query: { session: string } }
    expect(arg.path).toBe('/chat')
    expect(arg.query.session).toMatch(/^agent:code-engineer:webchat:[a-z0-9]+$/)
    expect(el.querySelector('[data-testid="expert-session-menu"]')).toBeNull()
  })
})
