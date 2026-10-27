// @vitest-environment happy-dom
// 输入框底栏的对话编排入口：四按钮渲染、面板互斥、form 即时推导、updateOrchestration 上抛。
//
// 锁的是「接线」而不是面板内部（内部交互各自有组件测试）：按钮必须真实挂在
// composer 底栏、点开面板后选择必须把补丁抛给父级、面板之间必须互斥 ——
// 否则会出现「能点但无效」的假反馈。
import { afterEach, describe, expect, it, vi } from 'vitest'
import { createApp, nextTick } from 'vue'
import { createPinia } from 'pinia'
import i18n from '@/i18n'
import { createDefaultOrchestration } from '@/composables/chat/orchestration'
import ChatComposer from './ChatComposer.vue'

const BASE_PROPS = {
  // 编排四按钮组有 v-if="props.orchestration"：这里显式传入默认编排状态，
  // 未传时整组不渲染（规避既有测试不装 pinia 的注入问题）。
  orchestration: createDefaultOrchestration(),
  modelValue: '',
  'onUpdate:modelValue': () => {},
  attachments: [],
  busySendMode: 'queue',
  hasSendContent: false,
  isStreaming: false,
  canStop: false,
  runModeLocked: false,
  runModeLockMessage: '',
  isNewLanding: false,
  placeholder: 'Send a message',
  sendButtonTitle: 'Send',
  runMode: 'safe',
  allowedRunModes: ['safe', 'full'],
  modelRoutingMode: 'off',
  modelRoutingSettingsBusy: false,
  voiceBusy: false,
  voiceRecording: false,
  voiceReady: true,
}

const apps: ReturnType<typeof createApp>[] = []

async function mount(overrides: Record<string, unknown> = {}) {
  const el = document.createElement('div')
  document.body.appendChild(el)
  const app = createApp(ChatComposer, { ...BASE_PROPS, ...overrides })
  // ChatSwarmPopover / ChatTeamPicker 的 setup 会 useRpcStore() → 需要 pinia。
  app.use(createPinia())
  app.use(i18n)
  apps.push(app)
  app.mount(el)
  await nextTick()
  return el
}

afterEach(() => {
  while (apps.length) apps.pop()?.unmount()
  document.body.innerHTML = ''
})

describe('ChatComposer orchestration wiring', () => {
  it('renders the four orchestration buttons', async () => {
    const el = await mount()
    for (const id of [
      'chat-orchestration-btn-agents',
      'chat-orchestration-btn-team',
      'chat-orchestration-btn-swarm',
      'chat-orchestration-btn-mode',
    ]) {
      expect(el.querySelector(`[data-testid="${id}"]`)).not.toBeNull()
    }
  })

  it('shows the agent count badge when agents are selected', async () => {
    const el = await mount({
      orchestration: { form: 'subagent', agents: ['researcher', 'coder'] },
    })
    const badge = el.querySelector('[data-testid="orch-agents-count"]')
    expect(badge?.textContent).toBe('2')
  })

  it('picking an agent emits updateOrchestration with the subagent form', async () => {
    const onUpdate = vi.fn()
    const el = await mount({ onUpdateOrchestration: onUpdate })
    ;(el.querySelector('[data-testid="chat-orchestration-btn-agents"]') as HTMLButtonElement).click()
    await nextTick()
    const options = [...el.querySelectorAll('.orch-agent')] as HTMLButtonElement[]
    expect(options.length).toBeGreaterThan(0)
    options[0].click()
    await nextTick()
    expect(onUpdate).toHaveBeenCalledWith({ agents: ['orchestrator'], form: 'subagent' })
  })

  it('picking an expert team emits updateOrchestration with the expert form', async () => {
    const onUpdate = vi.fn()
    const el = await mount({ onUpdateOrchestration: onUpdate })
    ;(el.querySelector('[data-testid="chat-orchestration-btn-team"]') as HTMLButtonElement).click()
    await nextTick()
    const teams = [...el.querySelectorAll('.orch-team')] as HTMLButtonElement[]
    expect(teams.length).toBeGreaterThan(0)
    teams[0].click()
    await nextTick()
    expect(onUpdate).toHaveBeenCalledWith(
      expect.objectContaining({ teams: ['deck_craft'], form: 'expert' }),
    )
  })

  it('clicking a selected team again removes it (back to single)', async () => {
    const onUpdate = vi.fn()
    const el = await mount({
      orchestration: { ...createDefaultOrchestration(), teams: ['deck_craft'], form: 'expert' },
      onUpdateOrchestration: onUpdate,
    })
    ;(el.querySelector('[data-testid="chat-orchestration-btn-team"]') as HTMLButtonElement).click()
    await nextTick()
    const teams = [...el.querySelectorAll('.orch-team')] as HTMLButtonElement[]
    const selected = teams.find(btn => btn.getAttribute('aria-pressed') === 'true')
    expect(selected).toBeTruthy()
    selected!.click()
    await nextTick()
    expect(onUpdate).toHaveBeenCalledWith({ teams: [], form: 'single', confirmed: false })
  })

  it('supports selecting multiple expert teams at once', async () => {
    const onUpdate = vi.fn()
    const el = await mount({
      orchestration: { ...createDefaultOrchestration(), teams: ['deck_craft'], form: 'expert' },
      onUpdateOrchestration: onUpdate,
    })
    ;(el.querySelector('[data-testid="chat-orchestration-btn-team"]') as HTMLButtonElement).click()
    await nextTick()
    const teams = [...el.querySelectorAll('.orch-team')] as HTMLButtonElement[]
    const second = teams.find(btn => btn.getAttribute('aria-pressed') !== 'true')
    expect(second).toBeTruthy()
    second!.click()
    await nextTick()
    expect(onUpdate).toHaveBeenCalledWith(
      expect.objectContaining({ teams: ['deck_craft', 'fullstack_craft'], form: 'expert' }),
    )
  })

  it('opens orchestration panels mutually exclusively', async () => {
    const el = await mount()
    ;(el.querySelector('[data-testid="chat-orchestration-btn-agents"]') as HTMLButtonElement).click()
    await nextTick()
    expect(el.querySelector('.orch-agent')).not.toBeNull()
    ;(el.querySelector('[data-testid="chat-orchestration-btn-team"]') as HTMLButtonElement).click()
    await nextTick()
    // agents 面板（.orch-agent 专属）关闭，team 面板（.orch-team 专属）打开。
    expect(el.querySelector('.orch-agent')).toBeNull()
    expect(el.querySelector('.orch-team')).not.toBeNull()
  })

  it('swarm toggle emits updateOrchestration with the swarm form', async () => {
    const onUpdate = vi.fn()
    const el = await mount({ onUpdateOrchestration: onUpdate })
    ;(el.querySelector('[data-testid="chat-orchestration-btn-swarm"]') as HTMLButtonElement).click()
    await nextTick()
    const toggle = el.querySelector('.swarm-popover__toggle') as HTMLButtonElement
    expect(toggle).not.toBeNull()
    toggle.click()
    await nextTick()
    expect(onUpdate).toHaveBeenCalledWith({ swarmOn: true, form: 'swarm' })
  })

  // 2026-10-24：能力面板不再重复渲染 3×3 档位矩阵（与「模式」弹层内容完全重复）。
  // 档位只在「模式」里改；这里守住"不重复"，并确认能力勾选仍然生效。
  it('capability panel toggles capabilities and no longer duplicates the mode matrix', async () => {
    const onUpdate = vi.fn()
    const el = await mount({ onUpdateOrchestration: onUpdate })
    ;(el.querySelector('[data-testid="chat-orchestration-btn-mode"]') as HTMLButtonElement).click()
    await nextTick()
    expect(el.querySelectorAll('.orch-mode__cell').length).toBe(0)
    const cap = el.querySelector('.orch-mode__cap') as HTMLButtonElement
    expect(cap).not.toBeNull()
    cap.click()
    await nextTick()
    expect(onUpdate).toHaveBeenCalledWith(
      expect.objectContaining({ capabilities: expect.any(Array) }),
    )
  })

  // 回归：蜂群浮层此前在发送后仍停留在原地，盖住刚出现的编排泳道卡片右侧，
  // 用户以为「卡片没显示全」。发送动作必须先收起所有浮层。
  it('closes open popovers before submitting on send', async () => {
    const onSend = vi.fn()
    const el = await mount({ onSend, hasSendContent: true })
    ;(el.querySelector('[data-testid="chat-orchestration-btn-swarm"]') as HTMLButtonElement).click()
    await nextTick()
    expect(el.querySelector('.swarm-popover__toggle')).not.toBeNull()
    ;(el.querySelector('.chat-send-btn') as HTMLButtonElement).click()
    await nextTick()
    expect(onSend).toHaveBeenCalledTimes(1)
    expect(el.querySelector('.swarm-popover__toggle')).toBeNull()
  })
})