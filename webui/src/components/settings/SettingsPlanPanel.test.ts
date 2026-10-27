// @vitest-environment happy-dom
import { afterEach, describe, expect, it, vi } from 'vitest'

const mounted: Array<{ app: { unmount: () => void }; el: HTMLElement }> = []

async function settle(): Promise<void> {
  for (let i = 0; i < 8; i++) await Promise.resolve()
  await new Promise(resolve => setTimeout(resolve, 10))
}

const BASE_PLAN = {
  auto_decompose: true,
  require_approval: false,
  max_steps: 12,
  checkpoint_interval_steps: 1,
  heartbeat_interval_s: 30,
  step_timeout_secs: 0,
}

async function mountPanel(
  options: {
    methods?: string[]
    call?: (method: string, params: unknown) => Promise<unknown>
  } = {},
) {
  vi.resetModules()
  document.body.innerHTML = ''
  const methods = options.methods ?? ['config.plan.get', 'config.plan.set']
  const call =
    options.call ??
    vi.fn(async (method: string) => {
      if (method === 'config.plan.get') {
        return { ok: true, source: 'config.yaml', plan: { ...BASE_PLAN } }
      }
      if (method === 'config.plan.set') {
        return {
          ok: true,
          changed: true,
          restartRequired: true,
          plan: { ...BASE_PLAN, require_approval: true },
        }
      }
      return {}
    })
  const rpc = {
    waitForConnection: vi.fn(async () => {}),
    supportsMethod: vi.fn((method: string) => methods.includes(method)),
    markMethodUnavailable: vi.fn(),
    call,
  }
  vi.doMock('@/stores/rpc', () => ({ useRpcStore: () => rpc }))

  const { createApp, nextTick } = await import('vue')
  const i18n = (await import('@/i18n')).default
  i18n.global.locale.value = 'en'
  const Component = (await import('./SettingsPlanPanel.vue')).default
  const el = document.createElement('div')
  document.body.appendChild(el)
  const app = createApp(Component)
  app.use(i18n)
  app.mount(el)
  mounted.push({ app, el })
  await settle()
  await nextTick()
  return { el, call }
}

afterEach(() => {
  while (mounted.length) mounted.pop()?.app.unmount()
})

describe('SettingsPlanPanel', () => {
  /// 面板必须从**引擎自己的** config.yaml 读（Rust-first `config.plan.get`），
  /// 而不是复用 `config.get`（PY_FIRST，侧车在线时读的是 Python 的 config.toml）。
  it('reads plan values from the engine config source', async () => {
    const { el, call } = await mountPanel()
    expect(call).toHaveBeenCalledWith('config.plan.get', {})
    expect(el.textContent).toContain('Source: config.yaml')
    const approval = el.querySelector<HTMLInputElement>('[name="plan_require_approval"]')
    expect(approval).toBeTruthy()
    expect(approval!.getAttribute('aria-checked')).toBe('false')
    expect(el.querySelector<HTMLInputElement>('[name="plan_max_steps"]')!.value).toBe('12')
  })

  /// 写入必须走 `config.plan.set`，且**如实转述后端的 restartRequired**：
  /// 计划执行器的配置是进程级缓存，只说「已保存」就是假承诺。
  it('writes via config.plan.set and reports restartRequired honestly', async () => {
    const { el, call } = await mountPanel()
    const approval = el.querySelector<HTMLInputElement>('[name="plan_require_approval"]')!
    approval.checked = true
    approval.dispatchEvent(new Event('change'))
    await settle()
    expect(call).toHaveBeenCalledWith('config.plan.set', { require_approval: true })
    expect(el.textContent).toContain('takes effect after the gateway restarts')
  })

  /// 网关没有这对方法时（旧版/浏览器直连）**不许假装能改**，也不许发请求。
  it('does not pretend to be editable when the gateway lacks the interface', async () => {
    const { el, call } = await mountPanel({ methods: [] })
    expect(call).not.toHaveBeenCalled()
    expect(el.textContent).toContain('does not expose the plan-configuration interface')
  })
})
