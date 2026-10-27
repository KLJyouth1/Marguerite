import { computed, ref } from 'vue'
import type { RpcCallOptions, RpcConnectionWaitOptions } from '@/lib/rpc'

const activeHolds = ref(0)
let primedRelease: (() => void) | null = null

/**
 * Optional, mount-time RPCs must not enter the Gateway's serialized dispatch
 * queue ahead of chat session recovery. ChatView acquires a hold synchronously
 * during setup (before child mounted hooks run) and releases it as soon as the
 * critical request frames have been queued.
 */
export const optionalSessionRpcAllowed = computed(() => activeHolds.value === 0)

export const OPTIONAL_SESSION_RPC_TIMEOUT_MS = 10_000

export const optionalSessionRpcCallOptions: RpcCallOptions = {
  timeoutMs: OPTIONAL_SESSION_RPC_TIMEOUT_MS,
  // Give ordinary metadata latency enough room to avoid interrupting an
  // active stream. If a request is genuinely stuck, the Gateway's serialized
  // dispatcher cannot serve later frames, so reconnect as a last resort.
  timeoutAction: 'reconnect',
  abortAction: 'reconnect',
}

// The first setup-status read can queue behind the live Windows capability
// canary, whose own bounded probe may take up to 30 seconds. Treat this as a
// slow diagnostic read: give it enough time to finish and never recycle the
// shared chat socket merely because the read was abandoned.
export const sandboxSetupRpcCallOptions: RpcCallOptions = {
  timeoutMs: 45_000,
  timeoutAction: 'reject',
  abortAction: 'reject',
}

/**
 * `onboarding.status` 的调用选项 —— **它决定 readiness 显示，必须用与
 * sandboxSetup 相同的宽限**。
 *
 * # 为什么必须有它（2026-10-22 实测的「一直连接中」机制）
 *
 * `useGatewayReadiness` 用 `optionalSessionRpcCallOptions`（**10 秒** +
 * `timeoutAction: 'reconnect'`）调 `onboarding.status`。而上面那段注释已经写明：
 * **首次 setup-status 读取可能排在 Windows 能力 canary 后面，该探针最长 30 秒**。
 *
 * 于是形成一个自我维持的死循环：
 *
 * ```
 * 10s 超时 → reconnect（砸掉整条 WS）→ 重连后第一帧又排在 canary 后
 *          → 再次 10s 超时 → 再 reconnect → …
 * ```
 *
 * 期间 `useRpcCall` 的 `loading` 恒为 true、`status` 恒为 null，
 * `useGatewayReadiness` 因此恒返回 `'connecting'` —— 用户看到的就是
 * **顶栏一直「连接中」**，与真实的网关健康状态无关。
 *
 * 同一份文件里，`sandboxSetupRpcCallOptions` 已经为**同一类**读取给出了正确判据
 * （45 秒 + `reject` 而不断连），只是这个调用点被漏掉了。本常量就是把那条判据
 * 补到漏掉的调用点 ——「半修」的补齐，不是新设计。
 *
 * 为什么不直接改 `optionalSessionRpcCallOptions`：它同时服务 chat 会话恢复等
 * 关键路径，那里「真卡住就重连」是**正确**的语义。只有这个诊断型读取不该继承它。
 */
export const readinessRpcCallOptions: RpcCallOptions = {
  timeoutMs: 45_000,
  timeoutAction: 'reject',
  abortAction: 'reject',
}

// A mode click is an interactive control, so it must never look frozen behind
// an unrelated slow RPC on the connection. The composable updates the visible
// selection optimistically; this bound only governs persistence/rollback.
export const runModeWriteRpcCallOptions: RpcCallOptions = {
  timeoutMs: 5_000,
  timeoutAction: 'reject',
  abortAction: 'reject',
}

type OptionalSessionRpcClient = {
  waitForConnection: (
    timeoutMs?: number,
    signal?: AbortSignal,
    actions?: RpcConnectionWaitOptions,
  ) => Promise<unknown>
}

export function waitForSessionRpcConnection(
  rpc: OptionalSessionRpcClient,
  callOptions?: RpcCallOptions,
): Promise<unknown> {
  if (!callOptions) return rpc.waitForConnection()
  return rpc.waitForConnection(
    callOptions.timeoutMs,
    callOptions.signal,
    {
      timeoutAction: callOptions.timeoutAction,
      abortAction: callOptions.abortAction,
    },
  )
}

function createSessionBootstrapAdmission(): () => void {
  activeHolds.value += 1
  let released = false
  return () => {
    if (released) return
    released = true
    activeHolds.value = Math.max(0, activeHolds.value - 1)
  }
}

export function acquireSessionBootstrapAdmission(): () => void {
  return createSessionBootstrapAdmission()
}

/**
 * Hold optional traffic while a lazy ChatView chunk is still resolving.
 *
 * Router navigation starts before App/Sidebar mounted hooks, so this closes
 * the otherwise-unavoidable gap where global metadata RPCs could enter the
 * Gateway's serial dispatcher before ChatView setup has a chance to run.
 * Priming is singleton/idempotent: query-only chat navigation reuses the
 * mounted ChatView and must not accumulate an owner nobody will claim.
 */
export function primeSessionBootstrapAdmission(): void {
  if (primedRelease) return
  primedRelease = createSessionBootstrapAdmission()
}

/**
 * Atomically transfers the router's pre-mount hold to ChatView.
 *
 * Returning the existing release function instead of releasing and acquiring
 * a new hold prevents optional watchers from observing a transient open gate.
 */
export function claimSessionBootstrapAdmission(): () => void {
  if (!primedRelease) return createSessionBootstrapAdmission()
  const release = primedRelease
  primedRelease = null
  return release
}

/** Release a navigation hold when the chat route is aborted or abandoned. */
export function clearPrimedSessionBootstrapAdmission(): void {
  const release = primedRelease
  primedRelease = null
  release?.()
}
