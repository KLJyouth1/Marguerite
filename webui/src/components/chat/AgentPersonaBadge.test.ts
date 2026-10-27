// @vitest-environment happy-dom
// P0-2 consumer side: the header badge renders the shared active persona,
// hides when there is none, and navigates to /agents on click.
import { createApp, nextTick, type App } from 'vue'
import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest'

const push = vi.fn()
vi.mock('vue-router', () => ({
  useRouter: () => ({ push }),
}))

import i18n, { loadLocaleMessages } from '@/i18n'
import AgentPersonaBadge from '@/components/chat/AgentPersonaBadge.vue'
import { useChatRouteFeedback } from '@/composables/chat/useChatRouteFeedback'

let app: App<Element> | null = null

afterEach(() => {
  app?.unmount()
  app = null
  document.body.innerHTML = ''
  push.mockReset()
  useChatRouteFeedback().setActivePersona(null)
})

beforeAll(async () => {
  await loadLocaleMessages('zh-Hans')
  i18n.global.locale.value = 'zh-Hans'
})

async function mountBadge() {
  const el = document.createElement('div')
  document.body.appendChild(el)
  app = createApp(AgentPersonaBadge)
  app.use(i18n)
  app.mount(el)
  await nextTick()
  return el
}

describe('AgentPersonaBadge (P0-2)', () => {
  it('renders the current expert name and a tooltip with description + agent id', async () => {
    useChatRouteFeedback().setActivePersona({
      agentId: 'code-engineer',
      name: '代码工程师',
      description: '编写与调试代码',
      isBuiltin: true,
    })
    const el = await mountBadge()
    const badge = el.querySelector('[data-testid="agent-persona-badge"]')
    expect(badge).not.toBeNull()
    expect(el.textContent).toContain('当前专家：代码工程师')
    expect(badge!.getAttribute('title')).toContain('编写与调试代码')
    expect(badge!.getAttribute('title')).toContain('code-engineer')
  })

  it('does not render when no persona is active', async () => {
    const el = await mountBadge()
    expect(el.querySelector('[data-testid="agent-persona-badge"]')).toBeNull()
  })

  it('labels the default session as 主智能体, not 当前专家', async () => {
    // 主智能体不是「专家」，是默认应答者（严工）。同一事实在两处用不同措辞
    // 会让用户以为自己在跟两个不同的东西说话。
    useChatRouteFeedback().setActivePersona({
      agentId: 'main',
      name: '严工 · 高级开发工程师',
      description: '先诊断根因',
      isBuiltin: true,
    })
    const el = await mountBadge()
    expect(el.textContent).toContain('主智能体：严工 · 高级开发工程师')
    expect(el.textContent).not.toContain('当前专家')
  })

  it('navigates to /agents on click', async () => {
    useChatRouteFeedback().setActivePersona({
      agentId: 'researcher',
      name: '研究员',
      description: '',
      isBuiltin: true,
    })
    const el = await mountBadge()
    ;(el.querySelector('[data-testid="agent-persona-badge"]') as HTMLButtonElement).click()
    expect(push).toHaveBeenCalledWith('/agents')
  })

  it('reacts to persona changes on the shared ref (producer → consumer)', async () => {
    const shared = useChatRouteFeedback()
    const el = await mountBadge()
    expect(el.querySelector('[data-testid="agent-persona-badge"]')).toBeNull()
    shared.setActivePersona({ agentId: 'data-analyst', name: '数据分析师', description: '', isBuiltin: true })
    await nextTick()
    expect(el.querySelector('[data-testid="agent-persona-badge"]')).not.toBeNull()
  })
})