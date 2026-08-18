import { afterEach, describe, expect, it, vi } from 'vitest'

// useSandboxSettings 依赖的模块统一 mock（按方法分流 RPC）
const mocks = vi.hoisted(() => ({
  rpcCall: vi.fn(),
  waitForConnection: vi.fn(async () => {}),
  pushToast: vi.fn(),
}))

vi.mock('@/stores/rpc', () => ({
  useRpcStore: () => ({
    call: mocks.rpcCall,
    waitForConnection: mocks.waitForConnection,
    supportsMethod: () => true,
  }),
}))

vi.mock('@/composables/useToasts', () => ({
  useToasts: () => ({ pushToast: mocks.pushToast }),
}))

vi.mock('@/platform', () => ({
  usePlatform: () => ({
    capabilities: { isDesktop: false },
    settings: {},
  }),
}))

vi.mock('@/i18n', () => ({
  default: { global: { t: (key: string) => key } },
}))

import { effectScope } from 'vue'
import { useSandboxSettings } from './useSandboxSettings'

const DEFAULT_POLICY = {
  schemaVersion: 2,
  policyVersion: 1,
  files: {
    customDenyWritePaths: [],
    recursiveDeleteBackupEnabled: true,
    backupQuotaBytes: 3 * 1024 ** 3,
  },
  commands: {
    requireApprovalPrefixes: [],
    autoAllowPrefixes: [],
    systemTools: 'auto',
  },
  network: { blockAllNetwork: false, allowDomains: [], denyDomains: [] },
  runtimes: { enabled: true, python: true, node: true, gitBash: true },
}

function mockReads(): void {
  mocks.rpcCall.mockImplementation((method: string) => {
    switch (method) {
      case 'sandbox.policy.get':
        return Promise.resolve(JSON.parse(JSON.stringify(DEFAULT_POLICY)))
      case 'sandbox.policy.defaults':
        return Promise.resolve({ builtinDenyWritePaths: [], runtimeTarget: null, runtimeVersions: {} })
      case 'sandbox.run_mode.preference.get':
        return Promise.resolve({ runMode: 'full' })
      case 'sandbox.capability.status':
        return Promise.resolve({ available: false, backend: 'policy-only' })
      default:
        return Promise.resolve({})
    }
  })
}

function makeHarness() {
  const scope = effectScope()
  let api!: ReturnType<typeof useSandboxSettings>
  scope.run(() => {
    api = useSandboxSettings()
  })
  return { api, scope }
}

afterEach(() => {
  vi.clearAllMocks()
})

describe('useSandboxSettings 开关持久化', () => {
  it('load 后开关反映服务端策略', async () => {
    mockReads()
    const { api, scope } = makeHarness()
    await api.load()
    expect(api.draft.value?.runtimes.enabled).toBe(true)
    expect(api.loadError.value).toBe('')
    scope.stop()
  })

  it('保存成功后开关状态保持（不回弹）', async () => {
    mockReads()
    mocks.rpcCall.mockImplementation((method: string, params?: { policy?: typeof DEFAULT_POLICY }) => {
      if (method === 'sandbox.policy.update') {
        const saved = JSON.parse(JSON.stringify(params!.policy))
        saved.policyVersion = 2
        return Promise.resolve(saved)
      }
      return mockReadsAndGet(method)
    })
    const { api, scope } = makeHarness()
    await api.load()
    // 用户关闭「运行环境」开关
    api.draft.value!.runtimes.enabled = false
    const ok = await api.flushSectionSave('runtimes')
    expect(ok).toBe(true)
    expect(api.draft.value!.runtimes.enabled).toBe(false)
    expect(api.draft.value!.policyVersion).toBe(2)
    scope.stop()
  })

  it('保存失败时开关不回弹（核心修复：保留用户意图 + 错误提示）', async () => {
    mockReads()
    mocks.rpcCall.mockImplementation((method: string) => {
      if (method === 'sandbox.policy.update') {
        return Promise.reject(new Error('network down'))
      }
      return mockReadsAndGet(method)
    })
    const { api, scope } = makeHarness()
    await api.load()
    expect(api.draft.value!.runtimes.enabled).toBe(true)
    // 用户关闭开关 → 保存失败
    api.draft.value!.runtimes.enabled = false
    const ok = await api.flushSectionSave('runtimes')
    expect(ok).toBe(false)
    // 关键断言：开关保持用户设置（旧实现会回滚为 true）
    expect(api.draft.value!.runtimes.enabled).toBe(false)
    expect(api.sectionError.runtimes).toContain('network down')
    expect(mocks.pushToast).toHaveBeenCalled()
    scope.stop()
  })

  it('版本冲突时同步服务端 baseline 但保留用户开关状态', async () => {
    mockReads()
    const conflict = new Error('conflict')
    Object.assign(conflict, { code: 'POLICY_VERSION_CONFLICT', details: { currentPolicy: { ...DEFAULT_POLICY, policyVersion: 3 } } })
    mocks.rpcCall.mockImplementation((method: string) => {
      if (method === 'sandbox.policy.update') {
        return Promise.reject(conflict)
      }
      return mockReadsAndGet(method)
    })
    const { api, scope } = makeHarness()
    await api.load()
    api.draft.value!.runtimes.enabled = false
    const ok = await api.flushSectionSave('runtimes')
    expect(ok).toBe(false)
    // baseline 同步为服务端最新版本，但 draft 保留用户改动（开关不回弹）
    expect(api.baseline.value!.policyVersion).toBe(3)
    expect(api.draft.value!.runtimes.enabled).toBe(false)
    scope.stop()
  })
})

// 与 mockReads 相同的只读分流（load 阶段使用）
function mockReadsAndGet(method: string): Promise<unknown> {
  switch (method) {
    case 'sandbox.policy.get':
      return Promise.resolve(JSON.parse(JSON.stringify(DEFAULT_POLICY)))
    case 'sandbox.policy.defaults':
      return Promise.resolve({ builtinDenyWritePaths: [], runtimeTarget: null, runtimeVersions: {} })
    case 'sandbox.run_mode.preference.get':
      return Promise.resolve({ runMode: 'full' })
    case 'sandbox.capability.status':
      return Promise.resolve({ available: false, backend: 'policy-only' })
    default:
      return Promise.resolve({})
  }
}
