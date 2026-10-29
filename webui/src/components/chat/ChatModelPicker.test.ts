// @vitest-environment happy-dom
// 输入框旁的自定义端点快添表单。
//
// 锁的是**协议与 provider id 必须一起决定**这条执行侧契约：
// `provider_protocol::provider_for_id` 只按 id 判定 Anthropic 家族（`_anthropic`
// 后缀），只填 baseUrl 的话，Anthropic 兼容端点会被按 OpenAI 协议打到
// /v1/chat/completions；`responses` 则必须落进 `wireApi` 而不是靠猜。
import { createApp, nextTick, type App } from 'vue'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { createI18n } from 'vue-i18n'
import { createPinia } from 'pinia'

const rpcCall = vi.fn()

vi.mock('@/stores/rpc', () => ({
  useRpcStore: () => ({
    state: 'connected',
    call: rpcCall,
    on: () => () => {},
  }),
}))

import ChatModelPicker from '@/components/chat/ChatModelPicker.vue'

let app: App<Element> | null = null

const i18n = createI18n({
  legacy: false,
  locale: 'zh-Hans',
  missingWarn: false,
  fallbackWarn: false,
  messages: {
    'zh-Hans': {
      chat: {
        modelPicker: {
          title: '选择模型',
          cloud: '云端厂商',
          custom: '自定义',
          local: '本地模型',
          count: '{count} 个',
          refreshLocal: '刷新本地模型',
          refreshingLocal: '读取本地模型…',
          activating: '切换中…',
          failed: '切换失败：{error}',
          empty: '无可用档案',
          localHint: '',
          addCustom: '添加',
          customLabel: '名称',
          customBaseUrl: '端点',
          customModel: '模型',
          customApiKey: 'API Key',
          customProtocol: '请求协议',
          customProtocolChat: 'OpenAI 兼容',
          customProtocolResponses: 'OpenAI Responses',
          customProtocolAnthropic: 'Anthropic Messages',
          customSubmit: '保存并启用',
          customCancel: '收起',
          customRequired: '端点与模型名为必填项。',
          close: '关闭',
        },
        modelIndicator: { tooltip: '{model}' },
      },
    },
  },
})

afterEach(() => {
  app?.unmount()
  app = null
  document.body.innerHTML = ''
  rpcCall.mockReset()
})

async function mountPicker() {
  const el = document.createElement('div')
  document.body.appendChild(el)
  app = createApp(ChatModelPicker)
  app.use(i18n)
  app.use(createPinia())
  app.mount(el)
  await nextTick()
  await nextTick()
  return el
}

async function openCustomForm(el: HTMLElement) {
  const trigger = el.querySelector('.model-picker__trigger') as HTMLButtonElement
  trigger.click()
  await nextTick()
  const addButton = Array.from(el.querySelectorAll<HTMLButtonElement>('.model-picker__refresh'))
    .find(button => button.textContent?.includes('添加'))
  expect(addButton, '自定义分档必须有添加入口').toBeTruthy()
  addButton!.click()
  await nextTick()
  const form = el.querySelector('.model-picker__form') as HTMLFormElement
  expect(form, '点击「添加」后表单必须出现').toBeTruthy()
  return form
}

async function fillAndSubmit(form: HTMLFormElement, protocol: string) {
  const inputs = form.querySelectorAll<HTMLInputElement>('.model-picker__input')
  const [label, baseUrl, model, apiKey] = inputs
  const set = (input: HTMLInputElement, value: string) => {
    input.value = value
    input.dispatchEvent(new Event('input'))
  }
  set(label!, 'Company gateway')
  set(baseUrl!, 'https://gw.example.com/v1')
  set(model!, 'claude-sonnet-4')
  set(apiKey!, 'sk-test')
  const select = form.querySelector<HTMLSelectElement>('select.model-picker__input')
  expect(select, '协议必须可选').toBeTruthy()
  select!.value = protocol
  select!.dispatchEvent(new Event('change'))
  await nextTick()
  form.dispatchEvent(new Event('submit'))
  await flush()
}

async function flush() {
  for (let i = 0; i < 6; i += 1) {
    await Promise.resolve()
    await nextTick()
  }
}

function upsertPayload() {
  const call = rpcCall.mock.calls.find(c => c[0] === 'onboarding.llmProfile.upsert')
  expect(call, '提交必须调用 onboarding.llmProfile.upsert').toBeTruthy()
  return call![1] as Record<string, unknown>
}

describe('ChatModelPicker 自定义端点：协议决定 provider id 与 wireApi', () => {
  it('Anthropic 协议：provider id 以 _anthropic 结尾，且不发送 wireApi', async () => {
    rpcCall.mockResolvedValue({ ok: true })
    const el = await mountPicker()
    const form = await openCustomForm(el)
    await fillAndSubmit(form, 'anthropic')

    const payload = upsertPayload()
    expect(String(payload.providerId)).toMatch(/_anthropic$/)
    expect(payload.wireApi).toBeUndefined()
    expect(payload.kind).toBe('custom')
  })

  it('Responses 协议：wireApi=responses，provider id 不带 _anthropic 后缀', async () => {
    rpcCall.mockResolvedValue({ ok: true })
    const el = await mountPicker()
    const form = await openCustomForm(el)
    await fillAndSubmit(form, 'responses')

    const payload = upsertPayload()
    expect(payload.wireApi).toBe('responses')
    expect(String(payload.providerId)).not.toMatch(/_anthropic$/)
  })

  it('Chat 协议（默认）：不发送 wireApi，由后端默认 chat', async () => {
    rpcCall.mockResolvedValue({ ok: true })
    const el = await mountPicker()
    const form = await openCustomForm(el)
    await fillAndSubmit(form, 'chat')

    const payload = upsertPayload()
    expect(payload.wireApi).toBeUndefined()
    expect(String(payload.providerId)).not.toMatch(/_anthropic$/)
  })

  it('保存失败时把后端错误带回来，而不是静默成功', async () => {
    rpcCall.mockResolvedValue({ ok: false, error: 'baseUrl 不可达' })
    const el = await mountPicker()
    const form = await openCustomForm(el)
    await fillAndSubmit(form, 'chat')

    const alert = el.querySelector('.model-picker__error')
    expect(alert?.textContent).toContain('baseUrl 不可达')
  })
})
