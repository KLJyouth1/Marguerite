import { ref, computed } from 'vue'
import { defineStore } from 'pinia'
import {
  RpcClient,
  type RpcCallOptions,
  type RpcConnectionWaitOptions,
  type RpcEventHandler,
} from '@/lib/rpc'

const WS_URL_KEY = 'marguerite.wsUrl'
const WS_TOKEN_KEY = 'marguerite.wsToken'
const CACHED_AUTH_KEY = 'marguerite.cachedAuth'
const CHAT_DRAFT_PREFIX = 'marguerite.chat.draft:'

function getDefaultRpcUrl(): string {
  const proto = location.protocol === 'https:' ? 'wss:' : 'ws:'
  return `${proto}//${location.host}/ws`
}

/**
 * 网关随同源 index.html 注入的 bootstrap WS token（`#Marguerite-data`
 * 的 `data-ws-token`，见 core/src/gateway/server.rs maybe_inject_index）。
 *
 * F-T4-01 修复后，网关在 token 已配置时强制校验握手：仅回环同源页面
 * 能读到该属性（远端网页跨源读不到，其 WS 也会被 Origin 白名单拒绝），
 * 桌面 WebView 因此无需用户手工粘贴 token 即可完成握手。
 * `?token=` 深链写入的 sessionStorage 值仍优先（指向其它实例时以它为准）。
 */
function injectedWsToken(): string {
  try {
    const el = document.getElementById('Marguerite-data')
    return el?.getAttribute('data-ws-token') || ''
  } catch {
    return ''
  }
}

function clearStoragePrefix(storage: Storage, prefix: string): void {
  try {
    for (const key of Object.keys(storage)) {
      if (key.startsWith(prefix)) storage.removeItem(key)
    }
  } catch {}
}

function clearLinkTokenBrowserState(): void {
  try {
    localStorage.removeItem(WS_URL_KEY)
    clearStoragePrefix(localStorage, CHAT_DRAFT_PREFIX)
  } catch {}
  try {
    sessionStorage.removeItem(WS_TOKEN_KEY)
    sessionStorage.removeItem(CACHED_AUTH_KEY)
  } catch {}
}

function consumeLinkTokenFromUrl(): { url: string; token: string } | null {
  let url: URL
  try {
    url = new URL(window.location.href)
  } catch {
    return null
  }
  const token = (url.searchParams.get('token') || '').trim()
  if (!token) return null

  clearLinkTokenBrowserState()
  const rpcUrl = getDefaultRpcUrl()
  saveConnectionSettings(rpcUrl, token)

  try {
    url.searchParams.delete('token')
    const cleaned = `${url.pathname}${url.search}${url.hash}`
    window.history.replaceState(null, '', cleaned)
  } catch {}

  return { url: rpcUrl, token }
}

function loadConnectionSettings(): { url: string; token: string } {
  let url = getDefaultRpcUrl()
  let token = ''
  try { url = localStorage.getItem(WS_URL_KEY) || url } catch {}
  try { token = sessionStorage.getItem(WS_TOKEN_KEY) || injectedWsToken() } catch {}
  return { url, token }
}

function saveConnectionSettings(url: string, token: string): void {
  try { localStorage.setItem(WS_URL_KEY, url || getDefaultRpcUrl()) } catch {}
  try {
    if (token) sessionStorage.setItem(WS_TOKEN_KEY, token)
    else sessionStorage.removeItem(WS_TOKEN_KEY)
  } catch {}
}

/**
 * 本次页面加载中「已经用哪个 token 建立过连接」。
 *
 * 为什么必须有它（2026-10-21 实测根因）：`main.ts` 的顺序是
 * `app.use(router)`（第 27 行）→ `rpcStore.init()`（第 34 行）。vue-router 在
 * `createWebHistory()` 时就捕获了**当前 location**——此时 URL 里还带着 `?token=`。
 * `init()` 消费 token 并 `replaceState` 把它从地址栏抹掉，但紧接着 router 的首次
 * 导航 finalize 又把**它自己捕获的旧 URL（含 token）** 写回地址栏。
 * 于是 `router.afterEach → applyLinkTokenFromUrl()` 再次读到同一个 token，
 * 无条件执行 `disconnect()` + `connect()`：
 *
 *   · 每次带 token 进入页面 = **多一次完整 WS 握手**（实测 2 次 open / 1 次 close）；
 *   · `disconnect()` 会把所有 in-flight 请求 reject 成 `Disconnected`
 *     —— `/channels` 首载 `sessions.list` 偶发失败即此因（4 次复现命中 1~2 次，
 *     命中与否取决于该请求是否正好落在 24ms 的重连窗口内）；
 *   · token 被重新写回 URL，等于把已清理的凭据又暴露在地址栏与历史记录里。
 *
 * 判据是**语义等价**而非"只要 URL 有 token 就重连"：同一个 token 已经不重连，
 * 只有 token 真的变了（换了访问链接）才切连接。这样既修掉双重连接，也保留
 * "不同 share link 切换身份"的能力。
 */
let connectedToken: string | null = null

export const useRpcStore = defineStore('rpc', () => {
  const client = ref<RpcClient | null>(null)
  const state = ref<'disconnected' | 'connecting' | 'connected'>('disconnected')
  const policy = ref<Record<string, unknown> | null>(null)
  const auth = ref<Record<string, unknown> | null>(null)
  const methods = ref<string[]>([])
  const unavailableMethods = ref<Set<string>>(new Set())
  const error = ref<string | null>(null)

  const isConnected = computed(() => state.value === 'connected')
  const isConnecting = computed(() => state.value === 'connecting')
  const isLocalOwner = computed(() => {
    if (!isConnected.value) return false
    const principal = auth.value?.principal
    return Boolean(
      principal
      && typeof principal === 'object'
      && (principal as Record<string, unknown>).isOwner === true,
    )
  })
  const canManageProjectWorkspaces = computed(() =>
    isLocalOwner.value
    && supportsMethod('workspaces.list'))
  const canChooseProject = computed(() =>
    canManageProjectWorkspaces.value
    && supportsMethod('workspaces.open'))

  function init() {
    const rpc = new RpcClient()
    client.value = rpc

    rpc.on('_state', (s: 'disconnected' | 'connecting' | 'connected') => {
      state.value = s
      if (s !== 'connected') {
        policy.value = null
        auth.value = null
        methods.value = []
        unavailableMethods.value = new Set()
      }
    })

    rpc.on('_hello', (data: {
      policy?: Record<string, unknown>
      auth?: Record<string, unknown>
      features?: { methods?: unknown }
    }) => {
      policy.value = data.policy || null
      auth.value = data.auth || null
      methods.value = Array.isArray(data.features?.methods)
        ? data.features.methods.filter((method): method is string => typeof method === 'string')
        : []
      unavailableMethods.value = new Set()
    })

    rpc.on('_gap', (detail: unknown) => {
      console.warn('[RPC] Sequence gap detected:', detail)
    })

    // Auto-connect on init. Desktop shells use the local gateway serving this UI.
    consumeLinkTokenFromUrl()
    const { url, token } = loadConnectionSettings()
    // 注入 token 落盘到 sessionStorage。
    //
    // 为什么必须做：`loadConnectionSettings` 在桌面壳冷启动时只从
    // `#Marguerite-data[data-ws-token]` 读到 token，**不会**写回 sessionStorage；
    // 而一大堆 REST 辅助函数（`/api/approvals`、`/api/system/update`、
    // `/api/elevated-mode`、附件上传…）只认 `sessionStorage['marguerite.wsToken']`。
    // 真机实证：带 token 的 WS 握手成功，但同一页面的 REST 全军覆没在 401，
    // 控制台刷出「Approvals snapshot failed: HTTP 401」这类死路告警。
    // 写回后，REST 与 WS 共用同一份凭据，401 的根因被消掉。
    if (token) {
      try {
        if (sessionStorage.getItem(WS_TOKEN_KEY) !== token) {
          sessionStorage.setItem(WS_TOKEN_KEY, token)
        }
      } catch {
        // sessionStorage 不可用（隐私模式）：各 REST 调用自带注入 token 兜底。
      }
    }
    if (rpc.state === 'disconnected') {
      connectedToken = token || null
      rpc.connect(url, token || undefined)
    }
  }

  async function connect(url: string, token?: string) {
    if (!client.value) throw new Error('RPC client not initialized')
    error.value = null
    saveConnectionSettings(url, token || '')
    connectedToken = token || null
    client.value.connect(url, token)
  }

  function applyLinkTokenFromUrl(): boolean {
    const settings = consumeLinkTokenFromUrl()
    if (!settings) return false
    // 同一个 token 已连过：`consumeLinkTokenFromUrl` 已顺手清理地址栏，
    // 这里**不再** disconnect + connect。否则每次带 token 进入都会二次握手，
    // 把 in-flight 请求打成 `Disconnected`（见 `connectedToken` 的注释）。
    if (settings.token === connectedToken) return false
    if (client.value) {
      client.value.disconnect()
      error.value = null
      policy.value = null
      auth.value = null
      methods.value = []
      unavailableMethods.value = new Set()
      connectedToken = settings.token
      client.value.connect(settings.url, settings.token)
    }
    return true
  }

  function disconnect() {
    client.value?.disconnect()
    connectedToken = null
    state.value = 'disconnected'
    policy.value = null
    auth.value = null
    methods.value = []
    unavailableMethods.value = new Set()
  }

  function supportsMethod(method: string): boolean {
    return methods.value.includes(method) && !unavailableMethods.value.has(method)
  }

  function markMethodUnavailable(method: string): void {
    if (!method) return
    unavailableMethods.value = new Set([...unavailableMethods.value, method])
  }

  /**
   * 冷启动/重连窗口的统一等待上限。
   *
   * 为什么在这里（而不是每个页面各写一遍）：`state !== 'connected'` 时直接抛
   * `Cannot call X: not connected (state: connecting)` 会把「还在连」当成「失败了」，
   * 于是插件、编辑器、驾驶舱、终端、专家中心、本地模型中心等页面在**第一次打开**
   * 就渲染红色错误条 + 重试按钮 —— 用户看到的是"每次点开都让我重试"。
   * 同源缺陷此前在 8 个视图里用各自的 `waitForConnection` 打了补丁（见各文件
   * "not connected (state: connecting)" 注释），但新页面仍会重犯。
   *
   * 现在把它收敛到唯一入口：未连接时先等（最多 8s，足够一次完整 WS 握手与
   * 网关冷启动），等到就正常调用；等不到才如实失败。
   */
  const CONNECT_WAIT_MS = 8_000

  async function call<T = unknown>(
    method: string,
    params?: Record<string, unknown>,
    options?: RpcCallOptions,
  ): Promise<T> {
    if (!client.value) throw new Error('RPC client not initialized')
    if (state.value !== 'connected') {
      // 写型/显式"排队关闭"的调用保持立即失败语义，绝不替调用方等：
      // 一个已经失效的连接上重放写操作可能造成重复副作用。
      if (options?.queue === false) {
        throw new Error(`Cannot call ${method}: not connected (state: ${state.value})`)
      }
      await client.value.waitForConnection(CONNECT_WAIT_MS, options?.signal, {
        abortAction: 'reject',
        timeoutAction: 'reject',
      })
    }
    return (
      options
        ? client.value.call(method, params, options)
        : client.value.call(method, params)
    ) as Promise<T>
  }

  function on(event: string, handler: RpcEventHandler): () => void {
    if (!client.value) {
      console.warn(`[RPC] No client for event subscription: ${event}`)
      return () => {}
    }
    return client.value.on(event, handler)
  }

  function waitForConnection(
    timeoutMs?: number,
    signal?: AbortSignal,
    actions?: RpcConnectionWaitOptions,
  ): Promise<void> {
    if (!client.value) return Promise.reject(new Error('RPC client not initialized'))
    return client.value.waitForConnection(timeoutMs, signal, actions)
  }

  return {
    client,
    state,
    policy,
    auth,
    methods,
    error,
    isConnected,
    isConnecting,
    isLocalOwner,
    canManageProjectWorkspaces,
    canChooseProject,
    init,
    connect,
    applyLinkTokenFromUrl,
    disconnect,
    supportsMethod,
    markMethodUnavailable,
    call,
    on,
    waitForConnection,
  }
})
