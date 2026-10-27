// @vitest-environment happy-dom
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import {
  RpcAbortError,
  RpcClient,
  type RpcClientError,
  RpcTimeoutError,
} from '@/lib/rpc'

class MockWebSocket {
  static readonly OPEN = 1
  static readonly CLOSED = 3
  static instances: MockWebSocket[] = []

  readonly sent: string[] = []
  throwOnSend = false
  readyState = MockWebSocket.OPEN
  onopen: (() => void) | null = null
  onmessage: ((event: MessageEvent) => void) | null = null
  onclose: (() => void) | null = null
  onerror: (() => void) | null = null

  constructor(readonly url: string) {
    MockWebSocket.instances.push(this)
  }

  send(data: string): void {
    if (this.throwOnSend) throw new Error('send failed')
    this.sent.push(data)
  }

  close(): void {
    this.readyState = MockWebSocket.CLOSED
    this.onclose?.()
  }

  receive(frame: unknown): void {
    this.onmessage?.({ data: JSON.stringify(frame) } as MessageEvent)
  }
}

function pendingCount(client: RpcClient): number {
  return (
    client as unknown as {
      _pending: Map<string, unknown>
    }
  )._pending.size
}

function establishConnection(socket: MockWebSocket): void {
  socket.receive({ type: 'event', event: 'connect.challenge' })
  socket.receive({ protocol: 3, policy: { tick_interval_ms: 30_000 } })
}

describe('RpcClient', () => {
  beforeEach(() => {
    MockWebSocket.instances = []
    localStorage.clear()
    vi.stubGlobal('WebSocket', MockWebSocket)
    vi.useFakeTimers()
  })

  afterEach(() => {
    vi.clearAllTimers()
    vi.useRealTimers()
    vi.unstubAllGlobals()
  })

  it('persists one random guest session key and sends it in every handshake', () => {
    const first = new RpcClient()
    first.connect('ws://rpc.test')
    const firstSocket = MockWebSocket.instances[0]
    firstSocket.receive({ type: 'event', event: 'connect.challenge' })

    const firstFrame = JSON.parse(firstSocket.sent[0]) as {
      params: { auth: { guestSessionKey: string } }
    }
    const guestSessionKey = firstFrame.params.auth.guestSessionKey
    expect(guestSessionKey).toMatch(/^osqg_[A-Za-z0-9_-]{43}$/)
    expect(localStorage.getItem('marguerite.guestSessionKey')).toBe(guestSessionKey)

    const second = new RpcClient()
    second.connect('ws://rpc.test')
    const secondSocket = MockWebSocket.instances[1]
    secondSocket.receive({ type: 'event', event: 'connect.challenge' })
    const secondFrame = JSON.parse(secondSocket.sent[0]) as {
      params: { auth: { guestSessionKey: string } }
    }
    expect(secondFrame.params.auth.guestSessionKey).toBe(guestSessionKey)

    first.disconnect()
    second.disconnect()
  })

  it('persists a server-generated compatibility guest key from hello', () => {
    const client = new RpcClient()
    client.connect('ws://rpc.test')
    const socket = MockWebSocket.instances[0]
    socket.receive({ type: 'event', event: 'connect.challenge' })
    const serverKey = 'osqg_BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB'

    socket.receive({
      protocol: 3,
      policy: { tick_interval_ms: 30_000 },
      auth: { guestSessionKey: serverKey },
    })

    expect(localStorage.getItem('marguerite.guestSessionKey')).toBe(serverKey)
    client.disconnect()
  })

  it('sends the guest session key alongside a named token', () => {
    localStorage.setItem(
      'marguerite.guestSessionKey',
      'osqg_AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA'
    )
    const client = new RpcClient()
    client.connect('ws://rpc.test', 'osq_named_token')
    const socket = MockWebSocket.instances[0]
    socket.receive({ type: 'event', event: 'connect.challenge' })

    const frame = JSON.parse(socket.sent[0]) as {
      params: { auth: { token: string; guestSessionKey: string } }
    }
    expect(frame.params.auth).toEqual({
      token: 'osq_named_token',
      guestSessionKey: 'osqg_AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA',
    })
    client.disconnect()
  })

  it('preserves structured retry and acceptance metadata on the rejected error', async () => {
    const client = new RpcClient()
    client.connect('ws://rpc.test')
    const socket = MockWebSocket.instances[0]

    const result = client.call(
      'chat.send',
      { message: 'hello' },
      { timeoutMs: 100, timeoutAction: 'reconnect' }
    )
    const request = JSON.parse(socket.sent[0]) as { id: string }
    socket.receive({
      type: 'res',
      id: request.id,
      ok: false,
      error: {
        code: 'STORAGE_BUSY',
        message: 'Storage is temporarily busy',
        retryable: true,
        retry_after_ms: 250,
        accepted: false,
        details: { operation: 'upsert_session', waited_ms: 2000 },
      },
    })

    let caught: RpcClientError | undefined
    try {
      await result
    } catch (error) {
      caught = error as RpcClientError
    }

    expect(caught).toBeInstanceOf(Error)
    expect(caught).toMatchObject({
      message: 'Storage is temporarily busy',
      code: 'STORAGE_BUSY',
      retryable: true,
      retry_after_ms: 250,
      accepted: false,
      details: { operation: 'upsert_session', waited_ms: 2000 },
    })
    expect(pendingCount(client)).toBe(0)
    await vi.advanceTimersByTimeAsync(100)
    expect(socket.readyState).toBe(MockWebSocket.OPEN)
    expect(MockWebSocket.instances).toHaveLength(1)
    client.disconnect()
  })

  it('preserves the wire frame and leaves calls unbounded by default', async () => {
    const client = new RpcClient()
    client.connect('ws://rpc.test')
    const socket = MockWebSocket.instances[0]

    const result = client.call('chat.history', { sessionKey: 'session-1' })
    const request = JSON.parse(socket.sent[0]) as {
      type: string
      id: string
      method: string
      params: Record<string, unknown>
    }

    expect(request).toEqual({
      type: 'req',
      id: request.id,
      method: 'chat.history',
      params: { sessionKey: 'session-1' },
    })
    await vi.advanceTimersByTimeAsync(120_000)
    expect(pendingCount(client)).toBe(1)

    socket.receive({ type: 'res', id: request.id, ok: true, payload: { messages: [] } })
    await expect(result).resolves.toEqual({ messages: [] })
    expect(pendingCount(client)).toBe(0)
    client.disconnect()
  })

  it('reports the socket generation only after a request frame is sent', async () => {
    const client = new RpcClient()
    const onSent = vi.fn()
    client.connect('ws://rpc.test')
    const socket = MockWebSocket.instances[0]

    const result = client.call('chat.history', {}, { onSent })
    const request = JSON.parse(socket.sent[0]) as { id: string }

    expect(onSent).toHaveBeenCalledOnce()
    expect(onSent).toHaveBeenCalledWith(expect.any(Number))

    socket.receive({ type: 'res', id: request.id, ok: true, payload: {} })
    await expect(result).resolves.toEqual({})
    client.disconnect()
  })

  it('rejects a bounded call with a typed timeout and ignores a late response', async () => {
    const client = new RpcClient()
    client.connect('ws://rpc.test')
    const socket = MockWebSocket.instances[0]

    const result = client.call(
      'chat.history',
      { sessionKey: 'session-1' },
      { timeoutMs: 25 }
    )
    const request = JSON.parse(socket.sent[0]) as {
      type: string
      id: string
      method: string
      params: Record<string, unknown>
    }
    const caught = result.catch((error: unknown) => error)

    expect(request).toEqual({
      type: 'req',
      id: request.id,
      method: 'chat.history',
      params: { sessionKey: 'session-1' },
    })
    await vi.advanceTimersByTimeAsync(25)

    const error = await caught
    expect(error).toBeInstanceOf(RpcTimeoutError)
    expect(error).toMatchObject({
      name: 'RpcTimeoutError',
      code: 'RPC_TIMEOUT',
      method: 'chat.history',
      timeoutMs: 25,
    })
    expect(pendingCount(client)).toBe(0)

    socket.receive({ type: 'res', id: request.id, ok: true, payload: 'late' })
    expect(pendingCount(client)).toBe(0)
    expect(socket.readyState).toBe(MockWebSocket.OPEN)
    client.disconnect()
  })

  it('rejects an in-flight call with a typed abort and removes its listener', async () => {
    const client = new RpcClient()
    const controller = new AbortController()
    client.connect('ws://rpc.test')
    const socket = MockWebSocket.instances[0]

    const result = client.call('sessions.messages.snapshot', {}, { signal: controller.signal })
    const request = JSON.parse(socket.sent[0]) as { id: string }
    const caught = result.catch((error: unknown) => error)
    controller.abort()

    const error = await caught
    expect(error).toBeInstanceOf(RpcAbortError)
    expect(error).toMatchObject({
      name: 'RpcAbortError',
      code: 'RPC_ABORTED',
      method: 'sessions.messages.snapshot',
    })
    expect(pendingCount(client)).toBe(0)

    socket.receive({ type: 'res', id: request.id, ok: true, payload: 'late' })
    expect(pendingCount(client)).toBe(0)
    client.disconnect()
  })

  it('can recycle the current socket when an abort requests reconnect', async () => {
    const client = new RpcClient()
    const controller = new AbortController()
    client.connect('ws://rpc.test')
    const socket = MockWebSocket.instances[0]

    const result = client.call(
      'sessions.messages.subscribe',
      {},
      { signal: controller.signal, abortAction: 'reconnect' }
    )
    const caught = result.catch((error: unknown) => error)
    controller.abort()

    expect(await caught).toBeInstanceOf(RpcAbortError)
    expect(socket.readyState).toBe(MockWebSocket.CLOSED)
    expect(pendingCount(client)).toBe(0)

    await vi.advanceTimersByTimeAsync(0)
    expect(MockWebSocket.instances).toHaveLength(2)
    client.disconnect()
  })

  it('replays collateral read-only calls instead of failing them on recycle', async () => {
    // 真机缺陷回归：会话订阅被中止会回收整条 socket，连带把同连接上正在飞的
    // `doctor.status` 打成 "Connection recycled after sessions.messages.subscribe
    // terminated"。只读请求必须被透明重放，而不是以别人的方法名报错。
    const client = new RpcClient()
    const controller = new AbortController()
    client.connect('ws://rpc.test')
    const socket = MockWebSocket.instances[0]
    establishConnection(socket)

    const doctor = client.call('doctor.status', { agentId: 'main' })
    const doctorFrames = socket.sent.map((raw) => JSON.parse(raw) as { id: string; method?: string })
    const doctorId = doctorFrames.find((frame) => frame.method === 'doctor.status')?.id
    expect(doctorId).toBeDefined()

    const subscribe = client
      .call('sessions.messages.subscribe', {}, { signal: controller.signal, abortAction: 'reconnect' })
      .catch((error: unknown) => error)
    controller.abort()
    expect(await subscribe).toBeInstanceOf(RpcAbortError)

    // 被牵连的 doctor.status 不能被判失败；它应留在等待重放的队列里。
    let settled: unknown = 'pending'
    void doctor.then(
      (value) => { settled = value },
      (error) => { settled = error },
    )
    await vi.advanceTimersByTimeAsync(0)
    expect(settled).toBe('pending')

    const nextSocket = MockWebSocket.instances[1]
    establishConnection(nextSocket)
    const replay = nextSocket.sent
      .map((raw) => JSON.parse(raw) as { id: string; method?: string })
      .find((frame) => frame.method === 'doctor.status')
    expect(replay).toBeDefined()

    nextSocket.receive({ type: 'res', id: replay!.id, ok: true, payload: { status: 'DEGRADED' } })
    await expect(doctor).resolves.toEqual({ status: 'DEGRADED' })
    client.disconnect()
  })

  it('does not replay write calls when the socket is recycled', async () => {
    // 写型调用绝不透明重放：重复副作用比一次失败更糟。
    const client = new RpcClient()
    const controller = new AbortController()
    client.connect('ws://rpc.test')
    const socket = MockWebSocket.instances[0]
    establishConnection(socket)

    const write = client.call('config.patch.safe', {}).catch((error: unknown) => error)
    const subscribe = client
      .call('sessions.messages.subscribe', {}, { signal: controller.signal, abortAction: 'reconnect' })
      .catch((error: unknown) => error)
    controller.abort()
    await subscribe

    await expect(write).resolves.toMatchObject({
      message: expect.stringContaining('Connection recycled'),
    })
    client.disconnect()
  })

  it('cleans pending calls synchronously on disconnect and socket close', async () => {
    const disconnectedClient = new RpcClient()
    disconnectedClient.connect('ws://rpc.test')
    const disconnectResult = disconnectedClient.call('chat.history')
    const disconnectError = disconnectResult.catch((error: unknown) => error)

    disconnectedClient.disconnect()

    await expect(disconnectError).resolves.toMatchObject({ message: 'Disconnected' })
    expect(pendingCount(disconnectedClient)).toBe(0)

    const closedClient = new RpcClient()
    closedClient.connect('ws://rpc.test')
    const closedSocket = MockWebSocket.instances[MockWebSocket.instances.length - 1]
    const closeResult = closedClient.call('chat.history')
    const closeError = closeResult.catch((error: unknown) => error)

    closedSocket.close()

    await expect(closeError).resolves.toMatchObject({ message: 'Connection closed' })
    expect(pendingCount(closedClient)).toBe(0)
    closedClient.disconnect()
  })

  it('cleans a call when send throws and recycles the failed socket', async () => {
    const client = new RpcClient()
    const onSent = vi.fn()
    client.connect('ws://rpc.test')
    const socket = MockWebSocket.instances[0]
    socket.throwOnSend = true

    const error = await client.call(
      'chat.history',
      {},
      { onSent },
    ).catch((caught: unknown) => caught)

    expect(error).toMatchObject({ message: 'send failed' })
    expect(onSent).not.toHaveBeenCalled()
    expect(pendingCount(client)).toBe(0)
    expect(socket.readyState).toBe(MockWebSocket.CLOSED)

    await vi.advanceTimersByTimeAsync(0)
    expect(MockWebSocket.instances).toHaveLength(2)
    client.disconnect()
  })

  it('recycles on timeout without letting stale socket events close the replacement', async () => {
    const client = new RpcClient()
    client.connect('ws://rpc.test')
    const firstSocket = MockWebSocket.instances[0]
    establishConnection(firstSocket)
    expect(client.state).toBe('connected')

    const sibling = client.call('sessions.messages.snapshot')
    // 只读旁证请求不再被判死：它应进入重放队列（旧行为是 reject 成
    // "Connection recycled after chat.history terminated"，那正是概览页
    // doctor.status 报错的成因）。
    let siblingSettled: unknown = 'pending'
    void sibling.then(
      (value) => { siblingSettled = value },
      (error) => { siblingSettled = error },
    )
    const result = client.call('chat.history', {}, {
      timeoutMs: 25,
      timeoutAction: 'reconnect',
    })
    const request = JSON.parse(firstSocket.sent[firstSocket.sent.length - 1]) as { id: string }
    const staleClose = firstSocket.onclose
    const caught = result.catch((error: unknown) => error)

    await vi.advanceTimersByTimeAsync(25)
    expect(await caught).toBeInstanceOf(RpcTimeoutError)
    expect(siblingSettled).toBe('pending')
    await vi.runOnlyPendingTimersAsync()

    const secondSocket = MockWebSocket.instances[1]
    expect(secondSocket).toBeDefined()
    establishConnection(secondSocket)
    expect(client.state).toBe('connected')

    // 旁证请求必须在新连接上重放，而不是消失。
    const replay = secondSocket.sent
      .map((raw) => JSON.parse(raw) as { id: string; method?: string })
      .find((frame) => frame.method === 'sessions.messages.snapshot')
    expect(replay).toBeDefined()

    staleClose?.()
    firstSocket.receive({ type: 'res', id: request.id, ok: true, payload: 'late' })

    expect(client.state).toBe('connected')
    secondSocket.receive({ type: 'res', id: replay!.id, ok: true, payload: { messages: [] } })
    expect(pendingCount(client)).toBe(0)
    client.disconnect()
  })

  it('supports typed timeout and abort termination while waiting for a connection', async () => {
    const timeoutClient = new RpcClient()
    timeoutClient.connect('ws://rpc.test')
    const timedWait = timeoutClient
      .waitForConnection(25)
      .catch((error: unknown) => error)

    await vi.advanceTimersByTimeAsync(25)
    const timeoutError = await timedWait
    expect(timeoutError).toBeInstanceOf(RpcTimeoutError)
    expect(timeoutError).toMatchObject({
      code: 'RPC_TIMEOUT',
      method: 'waitForConnection',
      timeoutMs: 25,
    })
    timeoutClient.disconnect()

    const abortClient = new RpcClient()
    const controller = new AbortController()
    abortClient.connect('ws://rpc.test')
    const abortedWait = abortClient
      .waitForConnection(30_000, controller.signal)
      .catch((error: unknown) => error)
    controller.abort()

    const abortError = await abortedWait
    expect(abortError).toBeInstanceOf(RpcAbortError)
    expect(abortError).toMatchObject({
      code: 'RPC_ABORTED',
      method: 'waitForConnection',
    })
    abortClient.disconnect()
  })

  it('rejects an already-aborted connection wait even when the socket is connected', async () => {
    const client = new RpcClient()
    client.connect('ws://rpc.test')
    establishConnection(MockWebSocket.instances[0])
    const controller = new AbortController()
    controller.abort()

    await expect(
      client.waitForConnection(
        30_000,
        controller.signal,
        { abortAction: 'reconnect' },
      ),
    ).rejects.toBeInstanceOf(RpcAbortError)
    expect(client.state).toBe('connected')
    expect(MockWebSocket.instances).toHaveLength(1)
    client.disconnect()
  })

  it('recycles the replacement socket when a wait spans a disconnected gap', async () => {
    const client = new RpcClient()
    client.connect('ws://rpc.test')
    const firstSocket = MockWebSocket.instances[0]
    establishConnection(firstSocket)
    firstSocket.close()

    const timedWait = client.waitForConnection(
      825,
      undefined,
      { timeoutAction: 'reconnect' },
    ).catch((error: unknown) => error)

    await vi.advanceTimersByTimeAsync(800)
    const replacement = MockWebSocket.instances[1]
    expect(replacement).toBeDefined()
    expect(client.state).toBe('connecting')

    await vi.advanceTimersByTimeAsync(25)
    await expect(timedWait).resolves.toBeInstanceOf(RpcTimeoutError)
    expect(replacement.readyState).toBe(MockWebSocket.CLOSED)

    await vi.advanceTimersByTimeAsync(1)
    const retrySocket = MockWebSocket.instances[2]
    expect(retrySocket).toBeDefined()
    establishConnection(retrySocket)
    await expect(client.waitForConnection(25)).resolves.toBeUndefined()
    client.disconnect()
  })

  it('retires a half-connected socket when the handshake wait times out', async () => {
    const client = new RpcClient()
    client.connect('ws://rpc.test')
    const firstSocket = MockWebSocket.instances[0]
    const timedWait = client
      .waitForConnection(
        25,
        undefined,
        { timeoutAction: 'reconnect' },
      )
      .catch((error: unknown) => error)

    await vi.advanceTimersByTimeAsync(25)
    await vi.advanceTimersByTimeAsync(1)

    await expect(timedWait).resolves.toBeInstanceOf(RpcTimeoutError)
    expect(firstSocket.readyState).toBe(MockWebSocket.CLOSED)
    expect(MockWebSocket.instances).toHaveLength(2)

    const secondSocket = MockWebSocket.instances[1]
    establishConnection(secondSocket)
    await expect(client.waitForConnection(25)).resolves.toBeUndefined()
    expect(client.state).toBe('connected')
    client.disconnect()
  })

  it('recycles the socket when a ping goes unanswered past the pong timeout', () => {
    const client = new RpcClient()
    client.connect('ws://rpc.test')
    const socket = MockWebSocket.instances[0]
    establishConnection(socket)

    const gaps: unknown[] = []
    client.on('_gap', (detail: unknown) => gaps.push(detail))

    // 55s: the heartbeat ping is sent.
    vi.advanceTimersByTime(55_000)
    expect(socket.sent.some(frame => frame.includes('"type":"ping"'))).toBe(true)

    // Server stays silent (no pong, no tick). After the 20s pong timeout the
    // watchdog closes the stale socket → onclose schedules a reconnect.
    vi.advanceTimersByTime(25_000)
    expect(gaps.length).toBeGreaterThan(0)
    expect(gaps[0]).toMatchObject({ reason: 'pong_timeout' })
    expect(socket.readyState).toBe(MockWebSocket.CLOSED)

    vi.advanceTimersByTime(800)
    expect(MockWebSocket.instances.length).toBeGreaterThanOrEqual(2)
    client.disconnect()
  })

  it('keeps the socket alive when the pong arrives within the heartbeat window', () => {
    const client = new RpcClient()
    client.connect('ws://rpc.test')
    const socket = MockWebSocket.instances[0]
    establishConnection(socket)

    const gaps: unknown[] = []
    client.on('_gap', (detail: unknown) => gaps.push(detail))

    vi.advanceTimersByTime(55_000)
    expect(socket.sent.some(frame => frame.includes('"type":"ping"'))).toBe(true)

    // Server answers the ping promptly; the watchdog must not close the socket.
    socket.receive({ type: 'pong' })
    vi.advanceTimersByTime(25_000)
    expect(gaps).toHaveLength(0)
    expect(socket.readyState).toBe(MockWebSocket.OPEN)
    client.disconnect()
  })

  // 真机事故（2026-10-24）：一个监听器抛异常曾能吃掉同帧其余监听器（含 '*' 看门狗），
  // 页面表现为「事件停摆但无报错」。分发必须逐监听器隔离。
  it('isolates a throwing listener: the rest of the frame and wildcard still dispatch', () => {
    const errorSpy = vi.spyOn(console, 'error').mockImplementation(() => {})
    const client = new RpcClient()
    client.connect('ws://rpc.test')
    const socket = MockWebSocket.instances[0]
    establishConnection(socket)

    const seen: unknown[] = []
    client.on('session.event.progress', () => {
      throw new Error('boom')
    })
    client.on('session.event.progress', (payload: unknown) => seen.push(payload))
    const wild: string[] = []
    client.on('*', (event: string) => wild.push(event))

    socket.receive({
      type: 'event',
      event: 'session.event.progress',
      payload: { kind: 'orchestration' },
      seq: 1,
    })
    expect(seen).toEqual([{ kind: 'orchestration' }])
    expect(wild).toEqual(['session.event.progress'])
    expect(errorSpy).toHaveBeenCalled()
    errorSpy.mockRestore()
    client.disconnect()
  })

  // 断档后必须推进 seq 水位：原实现只在正常帧推进，一旦 close 没走到 onclose，
  // 之后每一帧都会被判断档而永久丢弃（连接却仍 OPEN、状态仍 connected）。
  it('advances the seq watermark on a gap so later frames are not all discarded', () => {
    const client = new RpcClient()
    client.connect('ws://rpc.test')
    const socket = MockWebSocket.instances[0]
    establishConnection(socket)
    const gaps: unknown[] = []
    client.on('_gap', (detail: unknown) => gaps.push(detail))
    const seen: unknown[] = []
    client.on('session.event.progress', (payload: unknown) => seen.push(payload))

    socket.receive({ type: 'event', event: 'session.event.progress', payload: { n: 1 }, seq: 1 })
    // 2 丢失、直接来 3：丢掉这一帧并关连接触发对账
    socket.receive({ type: 'event', event: 'session.event.progress', payload: { n: 3 }, seq: 3 })
    expect(gaps).toHaveLength(1)
    expect(seen).toHaveLength(1)
    expect((client as unknown as { _lastSeq: number })._lastSeq).toBe(3)
    client.disconnect()
  })

  it('queues a call while disconnected and replays it after reconnect', async () => {
    const client = new RpcClient()
    client.connect('ws://rpc.test')
    const first = MockWebSocket.instances[0]
    establishConnection(first)

    // 断线（服务端 close → onclose → disconnected + 调度重连）
    first.close()
    expect(client.state).toBe('disconnected')

    const result = client.call('sessions.messages.snapshot', { sessionKey: 's1' })
    // 未连接 → 进入 outbox，promise 保持 pending 而不是立即 reject
    let settled = false
    void result.finally(() => { settled = true })
    await vi.advanceTimersByTimeAsync(100)
    expect(settled).toBe(false)

    // 重连成功后按 FIFO 重放
    await vi.advanceTimersByTimeAsync(800)
    const second = MockWebSocket.instances[1]
    expect(second).toBeDefined()
    establishConnection(second)

    const replayed = second.sent.find((frame) => {
      try {
        const parsed = JSON.parse(frame) as { method?: string }
        return parsed.method === 'sessions.messages.snapshot'
      } catch {
        return false
      }
    })
    expect(replayed).toBeDefined()

    const id = (JSON.parse(replayed!) as { id: string }).id
    second.receive({ type: 'res', id, ok: true, payload: { ok: true } })
    await expect(result).resolves.toEqual({ ok: true })
    client.disconnect()
  })

  it('rejects immediately when queue is disabled while disconnected', async () => {
    const client = new RpcClient()
    client.connect('ws://rpc.test')
    const first = MockWebSocket.instances[0]
    establishConnection(first)
    first.close()

    await expect(
      client.call('chat.send', { text: 'x' }, { queue: false })
    ).rejects.toThrow('Not connected')
    client.disconnect()
  })
})

// 2026-10-22：侧车启动期（约 6 秒窗口）返回的 SIDECAR_STARTING 必须被当作
// 「加载中」自动重试，而不是弹错误。否则用户看到的就是
// 「一启动先报错，点重试才连上」。
describe('RpcClient — 侧车启动期自动重试', () => {
  beforeEach(() => {
    MockWebSocket.instances = []
    localStorage.clear()
    vi.stubGlobal('WebSocket', MockWebSocket)
    vi.useFakeTimers()
  })

  afterEach(() => {
    vi.clearAllTimers()
    vi.useRealTimers()
    vi.unstubAllGlobals()
  })

  function connected(): { client: RpcClient; socket: MockWebSocket } {
    const client = new RpcClient()
    client.connect('ws://rpc.test')
    const socket = MockWebSocket.instances[0]
    socket.receive({ type: 'event', event: 'connect.challenge' })
    socket.receive({ type: 'res', id: '1', ok: true, payload: {} })
    return { client, socket }
  }

  it('SIDECAR_STARTING 会自动重发同一请求，而不是立刻失败', async () => {
    const { client, socket } = connected()
    const pending = client.call('chat.send', { text: 'hi' })
    const firstId = (JSON.parse(socket.sent[socket.sent.length - 1]) as { id: string }).id

    socket.receive({
      type: 'res',
      id: firstId,
      ok: false,
      error: { code: 'SIDECAR_STARTING', message: '引擎启动中' },
    })

    // 首次失败后不应立刻 reject，而是等待退避后重发
    await vi.advanceTimersByTimeAsync(900)
    const sentAfterRetry = socket.sent.map((f) => JSON.parse(f) as { method?: string; id: string })
      .filter((f) => f.method === 'chat.send')
    expect(sentAfterRetry.length).toBe(2)
    expect(sentAfterRetry[1].id).not.toBe(firstId)

    // 第二次成功 → 整体 resolve
    socket.receive({ type: 'res', id: sentAfterRetry[1].id, ok: true, payload: { ok: true } })
    await expect(pending).resolves.toEqual({ ok: true })
    client.disconnect()
  })

  it('非 SIDECAR_STARTING 的错误不会被重试（避免掩盖真问题）', async () => {
    const { client, socket } = connected()
    const pending = client.call('chat.send', { text: 'hi' })
    const id = (JSON.parse(socket.sent[socket.sent.length - 1]) as { id: string }).id
    socket.receive({
      type: 'res',
      id,
      ok: false,
      error: { code: 'INVALID_REQUEST', message: '参数错误' },
    })
    await expect(pending).rejects.toMatchObject({ code: 'INVALID_REQUEST' })
    const count = socket.sent.map((f) => JSON.parse(f) as { method?: string })
      .filter((f) => f.method === 'chat.send').length
    expect(count).toBe(1)
    client.disconnect()
  })
})
