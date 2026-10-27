// @vitest-environment happy-dom
// 输入框底栏的推理等级拖动条：渲染位置、事件上抛、锁定态。
//
// 锁的是「接线」而不是滑块内部（内部交互在 ChatEffortSlider.test.ts）：
// 滑块必须真的挂在 composer 底栏、点一下必须把档位抛给父级、
// 会话锁定（runModeLocked）时必须禁用 —— 否则会出现「能点但无效」的假反馈。
import { afterEach, describe, expect, it, vi } from 'vitest'
import { createApp, nextTick } from 'vue'
import { createPinia } from 'pinia'
import i18n from '@/i18n'
import ChatComposer from './ChatComposer.vue'

const BASE_PROPS = {
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
  // composer 现在内嵌 ChatModelPicker（依赖 rpc store），挂载必须带 Pinia。
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

describe('ChatComposer reasoning effort wiring', () => {
  it('renders the effort slider in the composer footer', async () => {
    const el = await mount()
    expect(el.querySelector('[data-testid="composer-effort-bar"]')).not.toBeNull()
  })

  it('shows the level passed by the parent', async () => {
    const el = await mount({ effort: 'max' })
    const trigger = el.querySelector('.effort-picker__trigger')
    expect(trigger?.textContent).toContain(i18n.global.t('chat.effort.levelMax'))
  })

  it('emits setEffort when a level is picked', async () => {
    const onSetEffort = vi.fn()
    const el = await mount({ effort: 'off', onSetEffort })
    ;(el.querySelector('.effort-picker__trigger') as HTMLButtonElement).click()
    await nextTick()
    const items = [...el.querySelectorAll('.effort-scale__item')] as HTMLButtonElement[]
    expect(items).toHaveLength(4)
    items[2].click()
    await nextTick()
    expect(onSetEffort).toHaveBeenCalledWith('deep')
  })

  it('disables the slider while the run mode is locked', async () => {
    const onSetEffort = vi.fn()
    const el = await mount({ effort: 'off', runModeLocked: true, onSetEffort })
    ;(el.querySelector('.effort-picker__trigger') as HTMLButtonElement).click()
    await nextTick()
    const items = [...el.querySelectorAll('.effort-scale__item')] as HTMLButtonElement[]
    expect(items.every((i) => i.disabled)).toBe(true)
    items[3].click()
    await nextTick()
    expect(onSetEffort).not.toHaveBeenCalled()
  })
})
