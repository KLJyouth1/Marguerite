import type { ConnectionState, RpcCallOptions, RpcEventHandler } from '@/lib/rpc'
import type { SessionEventPayload } from '@/types/rpc'

type SessionListRpc = {
  call(
    method: string,
    params?: Record<string, unknown>,
    callOptions?: RpcCallOptions,
  ): Promise<unknown>
  on(event: string, handler: RpcEventHandler): () => void
}

export interface UseSessionListSubscriptionOptions {
  rpc: SessionListRpc
  isConnected: () => boolean
  isAdmitted?: () => boolean
  refresh: () => void | Promise<void>
  scheduleRefresh: () => void
  onChanged?: RpcEventHandler
  /**
   * Real-time run-status callback. Fired for `sessions.changed` and every
   * `task.*` lifecycle event so the session store can mark a row running/idle
   * the moment the gateway reports it — independent of the 30s list polling,
   * and surviving route changes because this subscription is owned at app level
   * (never torn down when a view unmounts).
   */
  onRunStatus?: (event: string, payload: SessionEventPayload) => void
  warn?: (message: string, error?: unknown) => void
  callOptions?: RpcCallOptions
}

export function useSessionListSubscription(options: UseSessionListSubscriptionOptions) {
  let active = false
  let subscribed = false
  let connectionGeneration = 0
  let subscribeWork: Promise<void> | null = null
  let subscribeAttempt: symbol | null = null
  let removeListeners: Array<() => void> = []

  const warn = options.warn || ((message: string, error?: unknown) => {
    console.warn(message, error)
  })

  function invalidateConnection() {
    connectionGeneration += 1
    subscribed = false
    subscribeWork = null
    subscribeAttempt = null
  }

  function ensureSubscribed(): Promise<void> | null {
    if (
      !active
      || !options.isConnected()
      || options.isAdmitted?.() === false
      || subscribed
    ) return null
    if (subscribeWork) return subscribeWork

    const generation = connectionGeneration
    const attempt = Symbol('session-list-subscription')
    subscribeAttempt = attempt
    subscribeWork = (async () => {
      try {
        if (options.callOptions) {
          await options.rpc.call(
            'sessions.subscribe',
            undefined,
            options.callOptions,
          )
        } else {
          await options.rpc.call('sessions.subscribe')
        }
        if (!active || !options.isConnected() || generation !== connectionGeneration) return
        subscribed = true
        await options.refresh()
      } catch (error) {
        if (active && generation === connectionGeneration) {
          warn('Session list subscription failed', error)
        }
      } finally {
        if (subscribeAttempt === attempt) {
          subscribeWork = null
          subscribeAttempt = null
        }
      }
    })()
    return subscribeWork
  }

  function subscribe(): () => void {
    if (active) return cleanup
    active = true
    removeListeners = [
      options.rpc.on('sessions.changed', payload => {
        options.scheduleRefresh()
        options.onChanged?.(payload)
        options.onRunStatus?.('sessions.changed', payload as SessionEventPayload)
      }),
      options.rpc.on('task.queued', payload =>
        options.onRunStatus?.('task.queued', payload as SessionEventPayload),
      ),
      options.rpc.on('task.running', payload =>
        options.onRunStatus?.('task.running', payload as SessionEventPayload),
      ),
      options.rpc.on('task.failed', payload =>
        options.onRunStatus?.('task.failed', payload as SessionEventPayload),
      ),
      options.rpc.on('task.cancelled', payload =>
        options.onRunStatus?.('task.cancelled', payload as SessionEventPayload),
      ),
      options.rpc.on('task.timeout', payload =>
        options.onRunStatus?.('task.timeout', payload as SessionEventPayload),
      ),
      options.rpc.on('task.succeeded', payload =>
        options.onRunStatus?.('task.succeeded', payload as SessionEventPayload),
      ),
      options.rpc.on('_state', (state: ConnectionState) => {
        if (state === 'connected') {
          void ensureSubscribed()
          return
        }
        invalidateConnection()
      }),
    ]
    void ensureSubscribed()
    return cleanup
  }

  function cleanup() {
    if (!active) return
    const shouldUnsubscribe = subscribed || subscribeWork !== null
    active = false
    invalidateConnection()
    removeListeners.forEach(remove => remove())
    removeListeners = []

    if (shouldUnsubscribe && options.isConnected()) {
      const unsubscribe = options.callOptions
        ? options.rpc.call(
            'sessions.unsubscribe',
            undefined,
            options.callOptions,
          )
        : options.rpc.call('sessions.unsubscribe')
      void unsubscribe.catch(error => {
        warn('Session list unsubscribe failed', error)
      })
    }
  }

  return {
    subscribe,
    resume: ensureSubscribed,
    cleanup,
  }
}
