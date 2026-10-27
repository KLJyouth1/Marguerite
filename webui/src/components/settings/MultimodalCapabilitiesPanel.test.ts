// @vitest-environment happy-dom
import { afterEach, describe, expect, it, vi } from 'vitest'
import type { App } from 'vue'

const mounted: App[] = []

const PROFILE = {
  id: 'gpt-image-primary',
  label: 'OpenAI',
  provider: 'openai',
  model: 'gpt-image-1',
  active: true,
  capabilities: { text: true, declared: false },
  effectiveCapabilities: { text: true, image_output: true, declared: false },
}

async function settle() {
  for (let index = 0; index < 10; index++) await Promise.resolve()
}

async function mountPanel(options: { calls?: Array<{ method: string, params?: Record<string, unknown> }> } = {}) {
  vi.resetModules()
  document.body.innerHTML = ''
  const calls: Array<{ method: string, params?: Record<string, unknown> }> = []
  const call = vi.fn(async (method: string, params?: Record<string, unknown>) => {
    calls.push({ method, params })
    if (method === 'models.profiles.list') {
      return { ok: true, active: PROFILE.id, profiles: [PROFILE] }
    }
    if (method === 'models.profiles.set_capabilities') {
      return { ok: true, active: PROFILE.id, profiles: [{ ...PROFILE, capabilities: params?.capabilities as never }] }
    }
    throw new Error(`unexpected method: ${method}`)
  })
  vi.doMock('@/stores/rpc', () => ({
    useRpcStore: () => ({
      methods: [] as string[],
      supportsMethod: () => true,
      waitForConnection: vi.fn(async () => {}),
      call,
    }),
  }))

  const { createApp } = await import('vue')
  const Component = (await import('./MultimodalCapabilitiesPanel.vue')).default
  const el = document.createElement('div')
  document.body.appendChild(el)
  const app = createApp(Component)
  app.mount(el)
  mounted.push(app)
  await settle()
  return { el, calls: calls.length ? calls : options.calls || [] }
}

afterEach(() => {
  while (mounted.length) mounted.pop()!.unmount()
  vi.doUnmock('@/stores/rpc')
  vi.restoreAllMocks()
  document.body.innerHTML = ''
})

describe('MultimodalCapabilitiesPanel', () => {
  it('shows the auto-recognised capabilities so the user knows what was guessed', async () => {
    const { el } = await mountPanel()
    const auto = el.querySelector('[data-testid="multimodal-auto"]')
    expect(auto?.textContent || '').toContain('图像输出')
    const checkbox = el.querySelector('[data-testid="multimodal-flag-image_output"]') as HTMLInputElement
    // 未声明 → 勾选框反映"用户声明"，而不是自动识别结果（否则一保存就把猜测固化了）
    expect(checkbox.checked).toBe(false)
  })

  it('saves an explicit declaration with the checked flags', async () => {
    const { el, calls } = await mountPanel()
    const checkbox = el.querySelector('[data-testid="multimodal-flag-image_output"]') as HTMLInputElement
    checkbox.checked = true
    checkbox.dispatchEvent(new Event('change'))
    await settle()
    ;(el.querySelector('[data-testid="multimodal-save"]') as HTMLButtonElement).click()
    await settle()
    const save = calls.find(c => c.method === 'models.profiles.set_capabilities')
    expect(save).toBeTruthy()
    const caps = save!.params!.capabilities as Record<string, unknown>
    expect(caps.declared).toBe(true)
    expect(caps.text).toBe(true)
    expect(caps.image_output).toBe(true)
    expect(caps.image_input).toBe(false)
    expect(save!.params!.id).toBe(PROFILE.id)
  })

  it('blocks the save when the params textarea is not valid JSON', async () => {
    const { el, calls } = await mountPanel()
    const params = el.querySelector('[data-testid="multimodal-params-image_params"]') as HTMLTextAreaElement
    params.value = '{ size: 1024 '
    params.dispatchEvent(new Event('input'))
    await settle()
    ;(el.querySelector('[data-testid="multimodal-save"]') as HTMLButtonElement).click()
    await settle()
    expect(calls.some(c => c.method === 'models.profiles.set_capabilities')).toBe(false)
    expect(el.querySelector('[data-testid="multimodal-status"]')?.textContent || '').toContain('JSON')
  })

  it('sends reset=true to fall back to name-based recognition', async () => {
    const { el, calls } = await mountPanel()
    // 让"重置"按钮可用：先声明一次
    const checkbox = el.querySelector('[data-testid="multimodal-flag-audio_input"]') as HTMLInputElement
    checkbox.checked = true
    checkbox.dispatchEvent(new Event('change'))
    await settle()
    ;(el.querySelector('[data-testid="multimodal-save"]') as HTMLButtonElement).click()
    await settle()
    ;(el.querySelector('[data-testid="multimodal-reset"]') as HTMLButtonElement).click()
    await settle()
    const reset = calls.filter(c => c.method === 'models.profiles.set_capabilities').pop()
    expect(reset?.params?.reset).toBe(true)
  })
})
