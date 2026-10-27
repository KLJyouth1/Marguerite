/** Marguerite Web UI — WebSocket RPC client (TypeScript port). */

export interface RpcErrorDetail {
  code?: string;
  message?: string;
  details?: unknown;
  retryable?: boolean;
  retry_after_ms?: number;
  accepted?: boolean;
}

export interface RpcClientError extends Error {
  code?: string;
  details?: unknown;
  retryable?: boolean;
  retry_after_ms?: number;
  accepted?: boolean;
}

export type RpcTerminationAction = 'reject' | 'reconnect';

export interface RpcCallOptions {
  timeoutMs?: number;
  signal?: AbortSignal;
  timeoutAction?: RpcTerminationAction;
  abortAction?: RpcTerminationAction;
  /** Called synchronously only after the request frame is accepted by send(). */
  onSent?: (socketGeneration: number) => void;
  /**
   * 断线时是否进入 outbox 排队、重连成功后按序重放。
   * 默认 true（幂等的读型调用安全）；写型/有副作用调用可传 false 沿用
   * 立即 reject 语义（由上层 durable outbox 自行重放，避免重复副作用）。
   */
  queue?: boolean;
  /**
   * 内部字段：本请求因连接回收被重放过的次数（由客户端自增，调用方不要传）。
   * 用于给「回收→重放」设上界，避免病态循环。
   */
  replayCount?: number;
}

export interface RpcConnectionWaitOptions {
  timeoutAction?: RpcTerminationAction;
  abortAction?: RpcTerminationAction;
}

export class RpcTimeoutError extends Error implements RpcClientError {
  readonly code = 'RPC_TIMEOUT';

  constructor(
    readonly method: string,
    readonly timeoutMs: number
  ) {
    super(`${method} timed out after ${timeoutMs}ms`);
    this.name = 'RpcTimeoutError';
  }
}

export class RpcAbortError extends Error implements RpcClientError {
  readonly code = 'RPC_ABORTED';

  constructor(readonly method: string) {
    super(`${method} was aborted`);
    this.name = 'RpcAbortError';
  }
}

export interface RpcFrame {
  type?: string;
  id?: string;
  method?: string;
  params?: Record<string, unknown>;
  event?: string;
  payload?: unknown;
  meta?: Record<string, unknown>;
  ok?: boolean;
  error?: string | RpcErrorDetail;
  protocol?: number;
  policy?: Record<string, unknown>;
  features?: {
    methods?: string[];
    events?: string[];
  };
  auth?: Record<string, unknown>;
  seq?: number;
}

export type ConnectionState = 'disconnected' | 'connecting' | 'connected';
export type RpcEventHandler = {
  bivarianceHack(...args: unknown[]): void;
}['bivarianceHack'];

interface PendingRequest {
  resolve: (value: unknown) => void;
  reject: (error: Error) => void;
  method: string;
  generation: number;
  timeoutTimer: ReturnType<typeof setTimeout> | null;
  signal: AbortSignal | null;
  abortHandler: (() => void) | null;
  /** 回收重放需要原始参数（此前只在 call() 局部可见）。 */
  params: Record<string, unknown>;
  options: RpcCallOptions;
  replayCount: number;
}

/**
 * 断线期间的排队请求：连接恢复后按 FIFO 重放。
 * 仅承载幂等调用（调用方未显式传 queue:false）；写型调用由上层 durable outbox 负责。
 */
interface OutboxEntry {
  method: string;
  params: Record<string, unknown>;
  options: RpcCallOptions;
  resolve: (value: unknown) => void;
  reject: (error: Error) => void;
  queueTimer: ReturnType<typeof setTimeout> | null;
}

const GUEST_SESSION_STORAGE_KEY = 'marguerite.guestSessionKey';
const GUEST_SESSION_KEY_PATTERN = /^osqg_[A-Za-z0-9_-]{43}$/;

function persistGuestSessionKey(value: string): void {
  if (!GUEST_SESSION_KEY_PATTERN.test(value)) return;
  try {
    globalThis.localStorage?.setItem(GUEST_SESSION_STORAGE_KEY, value);
  } catch {
    // Storage can be disabled; the in-memory key still protects this connection.
  }
}

function newGuestSessionKey(): string {
  const bytes = new Uint8Array(32);
  globalThis.crypto.getRandomValues(bytes);
  let binary = '';
  for (const byte of bytes) binary += String.fromCharCode(byte);
  const encoded = globalThis.btoa(binary)
    .replace(/\+/g, '-')
    .replace(/\//g, '_')
    .replace(/=+$/g, '');
  return `osqg_${encoded}`;
}

function loadGuestSessionKey(): string {
  try {
    const stored = globalThis.localStorage?.getItem(GUEST_SESSION_STORAGE_KEY) || '';
    if (GUEST_SESSION_KEY_PATTERN.test(stored)) return stored;
  } catch {
    // Fall through to an in-memory credential.
  }
  const generated = newGuestSessionKey();
  persistGuestSessionKey(generated);
  return generated;
}

export class RpcClient {
  private _ws: WebSocket | null = null;
  private _socketGeneration = 0;
  private _reqId = 0;
  private _pending = new Map<string, PendingRequest>();
  private _listeners = new Map<string, Set<RpcEventHandler>>();
  private _state: ConnectionState = 'disconnected';
  private _url = '';
  private _token: string | null = null;
  private _guestSessionKey: string | null = null;
  private _reconnectTimer: ReturnType<typeof setTimeout> | null = null;
  private _reconnectDelay = 800;
  private _maxReconnectDelay = 15000;
  private _reconnectFactor = 1.7;
  private _autoReconnect = true;
  private _pingTimer: ReturnType<typeof setInterval> | null = null;
  private _pingInterval = 55000;
  private _policy: Record<string, unknown> | null = null;
  private _lastSeq = 0;
  private _lastFrameAt = 0;
  private _lastPingAt = 0;
  private _lastPongAt = 0;
  private _pongWatchTimer: ReturnType<typeof setInterval> | null = null;
  /** 心跳超时阈值：发出 ping 后若既无 pong 也无任何帧，判为连接失效并触发重连。 */
  private _pongTimeoutMs = 20000;
  private _tickWatchTimer: ReturnType<typeof setInterval> | null = null;
  private _tickTimeoutMs = 60000;
  private _outbox: OutboxEntry[] = [];
  private static readonly _MAX_OUTBOX = 200;
  /**
   * 同一请求因「连接回收」被重放的最大次数。
   *
   * 背景（真机缺陷）：`sessions.messages.subscribe` 的超时/中止被显式配置为
   * `abortAction: 'reconnect'`（见 `sessionBootstrapContract.phaseCallOptions`），
   * 目的是让被放弃的会话订阅不再占住网关的串行调度队列。但回收是**整条 socket**
   * 级别的，于是同连接上恰好正在飞的其他请求（典型：概览页的 `doctor.status`）
   * 会被一起 reject，用户看到
   * `无法从 ws://… 加载 doctor.status。Connection recycled after
   *  sessions.messages.subscribe terminated` —— 一个与 doctor 毫无关系的失败。
   *
   * 这些被牵连的请求**从未被服务端确认失败**，只是 socket 换代了；对只读调用
   * 重放是安全且必须的。上限 2 次防止病态循环（真断网时仍会走正常失败路径）。
   */
  private static readonly _MAX_REPLAYS = 2;

  connect(url: string, token?: string): void {
    this._url = url;
    this._token = token || null;
    this._guestSessionKey = this._guestSessionKey || loadGuestSessionKey();
    this._autoReconnect = true;
    this._clearReconnectTimer();
    if (this._ws) {
      this._retireCurrentSocket(new Error('Connection replaced'), false);
    }
    this._doConnect();
  }

  disconnect(): void {
    this._autoReconnect = false;
    this._clearReconnectTimer();
    this._retireCurrentSocket(new Error('Disconnected'), false);
    this._rejectAllPending(new Error('Disconnected'));
    this._rejectOutbox(new Error('Disconnected'));
    this._setState('disconnected');
  }

  /**
   * 对外调用入口：在 `_callOnce` 之上加一层**侧车启动期退避重试**。
   *
   * # 为什么需要（2026-10-22 实测的「一启动先报错、点重试才连上」）
   *
   * 启动顺序：Tauri 拉起 Rust 网关（**立刻**监听 18790）→ 网关**异步**启动 Python
   * 侧车（冷启动约 6 秒）→ 侧车就绪。WebView 在网关一监听上就加载并连 WS，
   * 于是这 6 秒内每个 Python 方法都返回 `SIDECAR_STARTING`。
   *
   * 旧行为：直接把它当错误弹出来，用户看到"报错"，点重试时侧车已就绪 → "重试就好了"。
   * 新行为：把它当**加载中**，退避重试（0.8s→1.6→3.2→6→6→6，总计约 24s 上限），
   * 期间不打扰用户。超过上限仍失败才如实抛错。
   */
  call(
    method: string,
    params: Record<string, unknown> = {},
    options: RpcCallOptions = {},
  ): Promise<unknown> {
    return this._callWithSidecarRetry(method, params, options, 0)
  }

  private _callWithSidecarRetry(
    method: string,
    params: Record<string, unknown>,
    options: RpcCallOptions,
    attempt: number,
  ): Promise<unknown> {
    return this._callOnce(method, params, options).catch((err: unknown) => {
      const code = (err as RpcClientError | undefined)?.code
      if (code === 'SIDECAR_STARTING' && attempt < 6) {
        const delay = Math.min(800 * 2 ** attempt, 6_000)
        return new Promise<void>((resolve) => setTimeout(resolve, delay)).then(() =>
          this._callWithSidecarRetry(method, params, options, attempt + 1),
        )
      }
      throw err
    })
  }

  private _callOnce(
    method: string,
    params: Record<string, unknown> = {},
    options: RpcCallOptions = {}
  ): Promise<unknown> {
    return new Promise((resolve, reject) => {
      const socket = this._ws;
      if (!socket || socket.readyState !== WebSocket.OPEN) {
        // 断线/重连窗口内：幂等调用进 outbox，连接恢复后按 FIFO 重放，
        // 消息不再因瞬时断连丢失（「请重新连接后再发送」的根因之一）。
        if (options.queue !== false) {
          this._enqueueOutbox({
            method,
            params,
            options,
            resolve,
            reject,
            queueTimer: null,
          });
          return;
        }
        reject(new Error('Not connected'));
        return;
      }
      const generation = this._socketGeneration;
      this._sendFrame(socket, generation, method, params, options, resolve, reject);
    });
  }

  /**
   * 底层发送：建立 pending、注册超时/中止、序列化并写帧。
   * 被 call() 与 outbox 重放共用，保证两者语义一致。
   */
  private _sendFrame(
    socket: WebSocket,
    generation: number,
    method: string,
    params: Record<string, unknown>,
    options: RpcCallOptions,
    resolve: (value: unknown) => void,
    reject: (error: Error) => void
  ): void {
    if (options.signal?.aborted) {
      reject(new RpcAbortError(method));
      return;
    }

    const id = String(++this._reqId);
    const pending: PendingRequest = {
      resolve,
      reject,
      method,
      generation,
      timeoutTimer: null,
      signal: options.signal || null,
      abortHandler: null,
      params,
      options,
      replayCount: options.replayCount ?? 0,
    };
    this._pending.set(id, pending);

    const terminate = (error: Error, action: RpcTerminationAction): void => {
      if (!this._rejectPending(id, error, generation)) return;
      if (action === 'reconnect') {
        this._recycleConnection(
          generation,
          new Error(`Connection recycled after ${method} terminated`)
        );
      }
    };

    if (options.signal) {
      pending.abortHandler = () => {
        terminate(new RpcAbortError(method), options.abortAction || 'reject');
      };
      options.signal.addEventListener('abort', pending.abortHandler, { once: true });
    }

    if (
      options.timeoutMs !== undefined &&
      options.timeoutMs > 0 &&
      Number.isFinite(options.timeoutMs)
    ) {
      pending.timeoutTimer = setTimeout(() => {
        terminate(
          new RpcTimeoutError(method, options.timeoutMs!),
          options.timeoutAction || 'reject'
        );
      }, options.timeoutMs);
    }

    let frame: string;
    try {
      frame = JSON.stringify({ type: 'req', id, method, params });
    } catch (error) {
      this._rejectPending(
        id,
        error instanceof Error ? error : new Error('Failed to serialize RPC request'),
        generation
      );
      return;
    }

    try {
      socket.send(frame);
    } catch (error) {
      const sendError =
        error instanceof Error ? error : new Error('Failed to send RPC request');
      this._rejectPending(id, sendError, generation);
      this._recycleConnection(generation, sendError);
      return;
    }
    try {
      options.onSent?.(generation);
    } catch {
      // A send receipt is observational. It must never fail a request whose
      // frame is already on the wire.
    }
  }

  private _enqueueOutbox(entry: OutboxEntry): void {
    // 竞态兜底：排队瞬间连接已恢复则直接发送。
    if (this._ws && this._ws.readyState === WebSocket.OPEN) {
      this._sendFrame(
        this._ws,
        this._socketGeneration,
        entry.method,
        entry.params,
        entry.options,
        entry.resolve,
        entry.reject
      );
      return;
    }
    if (this._outbox.length >= RpcClient._MAX_OUTBOX) {
      entry.reject(new Error('RPC outbox full'));
      return;
    }
    if (
      entry.options.timeoutMs !== undefined &&
      entry.options.timeoutMs > 0 &&
      Number.isFinite(entry.options.timeoutMs)
    ) {
      entry.queueTimer = setTimeout(() => {
        const idx = this._outbox.indexOf(entry);
        if (idx >= 0) this._outbox.splice(idx, 1);
        entry.reject(new RpcTimeoutError(entry.method, entry.options.timeoutMs!));
      }, entry.options.timeoutMs);
    }
    this._outbox.push(entry);
  }

  /** 连接恢复（hello 成功）后按 FIFO 重放排队请求。 */
  private _flushOutbox(): void {
    if (!this._outbox.length) return;
    const socket = this._ws;
    const generation = this._socketGeneration;
    if (!socket || socket.readyState !== WebSocket.OPEN) return;
    const entries = this._outbox;
    this._outbox = [];
    for (const entry of entries) {
      if (entry.queueTimer !== null) {
        clearTimeout(entry.queueTimer);
        entry.queueTimer = null;
      }
      this._sendFrame(
        socket,
        generation,
        entry.method,
        entry.params,
        entry.options,
        entry.resolve,
        entry.reject
      );
    }
  }

  on(event: string, handler: RpcEventHandler): () => void {
    if (!this._listeners.has(event)) this._listeners.set(event, new Set());
    this._listeners.get(event)!.add(handler);
    return () => this._listeners.get(event)?.delete(handler);
  }

  get state(): ConnectionState {
    return this._state;
  }

  get policy(): Record<string, unknown> {
    return this._policy || {};
  }

  waitForConnection(
    timeoutMs: number = 30000,
    signal?: AbortSignal,
    actions: RpcConnectionWaitOptions = {}
  ): Promise<void> {
    if (signal?.aborted) {
      // No wait and no request ever started, so this caller owns no socket to
      // recycle. Retiring the current connection here could kill a newer
      // session's healthy generation.
      return Promise.reject(new RpcAbortError('waitForConnection'));
    }
    if (this._state === 'connected') return Promise.resolve();

    return new Promise((resolve, reject) => {
      let timer: ReturnType<typeof setTimeout> | null = null;
      let settled = false;
      let off: () => void = () => {};

      const cleanup = (): void => {
        if (timer !== null) {
          clearTimeout(timer);
          timer = null;
        }
        off();
        signal?.removeEventListener('abort', onAbort);
      };
      const finish = (
        error?: Error,
        action: RpcTerminationAction = 'reject'
      ): void => {
        if (settled) return;
        settled = true;
        cleanup();
        if (!error) {
          resolve();
          return;
        }
        reject(error);
        if (action === 'reconnect') {
          // A disconnected waiter spans the reconnect gap. Its originally
          // observed generation may already have been retired before the
          // replacement handshake started; recycle the current still-
          // unconnected generation so the next attempt gets a fresh socket.
          if (this._state !== 'connected') {
            this._recycleConnection(
              this._socketGeneration,
              new Error('Connection recycled after waitForConnection terminated')
            );
          }
        }
      };
      const onAbort = (): void => {
        finish(
          new RpcAbortError('waitForConnection'),
          actions.abortAction || 'reject'
        );
      };

      off = this.on('_state', (s: ConnectionState) => {
        if (s === 'connected') {
          finish();
        }
      });
      signal?.addEventListener('abort', onAbort, { once: true });
      if (timeoutMs > 0 && Number.isFinite(timeoutMs)) {
        timer = setTimeout(() => {
          finish(
            new RpcTimeoutError('waitForConnection', timeoutMs),
            actions.timeoutAction || 'reject'
          );
        }, timeoutMs);
      }
    });
  }

  private _doConnect(): void {
    if (this._ws) return;
    this._setState('connecting');
    this._lastSeq = 0;
    this._lastFrameAt = Date.now();
    this._lastPingAt = 0;
    this._lastPongAt = 0;
    this._stopTickWatch();
    this._stopPongWatch();
    const generation = ++this._socketGeneration;
    let socket: WebSocket;
    try {
      socket = new WebSocket(this._url);
    } catch {
      if (generation !== this._socketGeneration) return;
      this._setState('disconnected');
      this._scheduleReconnect();
      return;
    }
    this._ws = socket;
    let handshakeRequestId: string | null = null;

    socket.onopen = () => {
      if (!this._isCurrentSocket(socket, generation)) return;
      this._reconnectDelay = 800;
      // Don't send connect yet — wait for connect.challenge from server
    };

    socket.onmessage = (ev: MessageEvent) => {
      if (!this._isCurrentSocket(socket, generation)) return;
      let data: RpcFrame;
      try {
        data = JSON.parse(ev.data);
      } catch {
        return;
      }
      if (!this._noteIncomingFrame(data)) return;

      // Handshake: server sends connect.challenge, we reply with connect request
      if (data.type === 'event' && data.event === 'connect.challenge') {
        const authParams = {
          auth: {
            ...(this._token ? { token: this._token } : {}),
            guestSessionKey: this._guestSessionKey || loadGuestSessionKey(),
          },
        };
        const id = String(++this._reqId);
        if (handshakeRequestId) return;
        handshakeRequestId = id;
        this._pending.set(id, {
          resolve: () => {},
          reject: (_err: Error) => {
            this._recycleConnection(generation, new Error('Connect handshake failed'));
          },
          method: 'connect',
          generation,
          timeoutTimer: null,
          signal: null,
          abortHandler: null,
          params: {},
          options: {},
          replayCount: 0,
        });
        try {
          socket.send(
            JSON.stringify({
              type: 'req',
              id,
              method: 'connect',
              params: {
                minProtocol: 3,
                maxProtocol: 3,
                client: { name: 'Marguerite-web' },
                ...authParams,
              },
            })
          );
        } catch (error) {
          const sendError =
            error instanceof Error ? error : new Error('Failed to send connect request');
          this._rejectPending(id, sendError, generation);
          this._recycleConnection(generation, sendError);
        }
        return;
      }

      // Handshake: HelloOk frame
      if (data.protocol !== undefined && this._state === 'connecting') {
        this._policy = data.policy || null;
        const serverGuestSessionKey = data.auth?.guestSessionKey;
        if (
          typeof serverGuestSessionKey === 'string'
          && GUEST_SESSION_KEY_PATTERN.test(serverGuestSessionKey)
        ) {
          this._guestSessionKey = serverGuestSessionKey;
          persistGuestSessionKey(serverGuestSessionKey);
        }
        if (handshakeRequestId) {
          this._resolvePending(handshakeRequestId, data, generation);
          handshakeRequestId = null;
        }
        this._setState('connected');
        this._emit('_hello', [data]);
        this._startPing();
        this._startTickWatch();
        // 连接恢复：重放断线期间排队的幂等请求。
        this._flushOutbox();
        return;
      }

      if (data.type === 'res') {
        const id = data.id ?? '';
        if (data.ok) {
          this._resolvePending(id, data.payload, generation);
        } else {
          const err = data.error;
          const message =
            typeof err === 'string'
              ? err
              : (err && (err.message || err.code)) || 'RPC error';
          const error = new Error(message) as RpcClientError;
          if (err && typeof err === 'object') {
            error.code = err.code;
            error.details = err.details;
            error.retryable = err.retryable;
            error.retry_after_ms = err.retry_after_ms;
            error.accepted = err.accepted;
          }
          this._rejectPending(id, error, generation);
        }
      } else if (data.type === 'event') {
        const meta = data.meta || {};
        this._emit(data.event ?? '', [data.payload, meta]);
        this._emit('*', [data.event, data.payload, meta]);
      }
    };

    socket.onclose = () => {
      if (!this._isCurrentSocket(socket, generation)) return;
      this._ws = null;
      ++this._socketGeneration;
      this._stopPing();
      this._stopTickWatch();
      this._rejectPendingForGeneration(generation, new Error('Connection closed'));
      this._setState('disconnected');
      this._scheduleReconnect();
    };

    // 偶发 error-only 场景：部分实现只触发 error 而不保证 close。
    // 若连接仍处于 OPEN，主动关闭以进入 onclose → 统一重连路径，避免悬挂。
    socket.onerror = () => {
      if (!this._isCurrentSocket(socket, generation)) return;
      if (socket.readyState === WebSocket.OPEN) {
        try {
          socket.close();
        } catch {
          // 关闭失败由 onclose 兜底或等待底层断开
        }
      }
    };
  }

  private _isCurrentSocket(socket: WebSocket, generation: number): boolean {
    return this._ws === socket && this._socketGeneration === generation;
  }

  private _takePending(id: string, generation?: number): PendingRequest | undefined {
    const pending = this._pending.get(id);
    if (!pending || (generation !== undefined && pending.generation !== generation)) {
      return undefined;
    }
    this._pending.delete(id);
    if (pending.timeoutTimer !== null) {
      clearTimeout(pending.timeoutTimer);
      pending.timeoutTimer = null;
    }
    if (pending.signal && pending.abortHandler) {
      pending.signal.removeEventListener('abort', pending.abortHandler);
      pending.abortHandler = null;
    }
    return pending;
  }

  private _resolvePending(id: string, value: unknown, generation?: number): boolean {
    const pending = this._takePending(id, generation);
    if (!pending) return false;
    pending.resolve(value);
    return true;
  }

  private _rejectPending(id: string, error: Error, generation?: number): boolean {
    const pending = this._takePending(id, generation);
    if (!pending) return false;
    pending.reject(error);
    return true;
  }

  private _rejectPendingForGeneration(generation: number, error: Error): void {
    for (const [id, pending] of [...this._pending]) {
      if (pending.generation === generation) {
        this._rejectPending(id, error, generation);
      }
    }
  }

  private _rejectAllPending(error: Error): void {
    for (const id of [...this._pending.keys()]) {
      this._rejectPending(id, error);
    }
  }

  private _rejectOutbox(error: Error): void {
    const entries = this._outbox;
    this._outbox = [];
    for (const entry of entries) {
      if (entry.queueTimer !== null) {
        clearTimeout(entry.queueTimer);
        entry.queueTimer = null;
      }
      entry.reject(error);
    }
  }

  /**
   * 连接即将换代时，把**被牵连的只读请求**转进 outbox 等新连接重放，
   * 避免它们以 `Connection recycled after <别人的方法> terminated` 失败。
   *
   * 只赎救同时满足以下条件的请求（保守，宁可失败也不重复副作用）：
   *   · 未主动中止（`signal` 未 abort）—— 主动中止是调用方自己的意愿；
   *   · 未显式声明 `queue: false` —— 写型调用必须保持"立即失败"语义；
   *   · 方法名属于只读集合（后缀白名单）—— 不靠猜测，避免 `.send/.patch/.delete`
   *     这类有副作用的调用被重放；
   *   · 重放次数未达上限。
   */
  private _salvagePendingForGeneration(generation: number): void {
    const survivors: OutboxEntry[] = [];
    for (const [id, pending] of [...this._pending]) {
      if (pending.generation !== generation) continue;
      if (pending.signal?.aborted) continue;
      if (pending.options.queue === false) continue;
      if (!RpcClient._isReplaySafeMethod(pending.method)) continue;
      if (pending.replayCount >= RpcClient._MAX_REPLAYS) continue;

      if (pending.timeoutTimer !== null) {
        clearTimeout(pending.timeoutTimer);
        pending.timeoutTimer = null;
      }
      if (pending.signal && pending.abortHandler) {
        pending.signal.removeEventListener('abort', pending.abortHandler);
        pending.abortHandler = null;
      }
      this._pending.delete(id);
      survivors.push({
        method: pending.method,
        params: pending.params,
        options: { ...pending.options, replayCount: pending.replayCount + 1 },
        resolve: pending.resolve,
        reject: pending.reject,
        queueTimer: null,
      });
    }
    for (const entry of survivors) this._enqueueOutbox(entry);
  }

  /** 只读方法白名单：只有这些后缀允许在连接回收后被透明重放。 */
  private static _isReplaySafeMethod(method: string): boolean {
    return (
      method.endsWith('.status') ||
      method.endsWith('.list') ||
      method.endsWith('.snapshot') ||
      method.endsWith('.history') ||
      method.endsWith('.get') ||
      method.endsWith('.effective') ||
      method.endsWith('.preview')
    );
  }

  private _retireCurrentSocket(error: Error, reconnect: boolean): void {
    const socket = this._ws;
    const generation = this._socketGeneration;
    if (!socket) {
      this._stopPing();
      this._stopTickWatch();
      this._setState('disconnected');
      if (reconnect) this._scheduleReconnect(true);
      return;
    }

    this._ws = null;
    ++this._socketGeneration;
    this._stopPing();
    this._stopTickWatch();
    // 先赎救被牵连的只读请求（仅重连场景；主动断开时一并失败）。
    if (reconnect) this._salvagePendingForGeneration(generation);
    this._rejectPendingForGeneration(generation, error);
    this._setState('disconnected');
    try {
      socket.close();
    } catch {}
    if (reconnect) this._scheduleReconnect(true);
  }

  private _recycleConnection(generation: number, error: Error): void {
    if (generation !== this._socketGeneration) return;
    this._retireCurrentSocket(error, true);
  }

  private _clearReconnectTimer(): void {
    if (this._reconnectTimer !== null) {
      clearTimeout(this._reconnectTimer);
      this._reconnectTimer = null;
    }
  }

  private _startPing(): void {
    this._stopPing();
    this._pingTimer = setInterval(() => {
      if (this._ws && this._ws.readyState === WebSocket.OPEN) {
        this._lastPingAt = Date.now();
        this._ws.send('{"type":"ping"}');
      }
    }, this._pingInterval);
    this._startPongWatch();
  }

  private _stopPing(): void {
    if (this._pingTimer !== null) {
      clearInterval(this._pingTimer);
      this._pingTimer = null;
    }
    this._stopPongWatch();
  }

  /**
   * 心跳超时检测：server 可能 TCP 层仍开着但事件循环已停摆（不响应 ping、
   * 不推 tick）。若最新一次 ping 超过 `_pongTimeoutMs` 仍无 pong 且无任何帧，
   * 主动关闭连接 → 触发重连。阈值 20s 远高于服务端最坏调度窗口（py 代理 3s），
   * 低于 ping 周期 55s，避免误判。
   */
  private _startPongWatch(): void {
    this._stopPongWatch();
    this._pongWatchTimer = setInterval(() => {
      if (!this._ws || this._ws.readyState !== WebSocket.OPEN) return;
      if (this._lastPingAt <= this._lastPongAt) return;
      const unansweredMs = Date.now() - this._lastPingAt;
      if (unansweredMs <= this._pongTimeoutMs) return;
      this._emit('_gap', [{ reason: 'pong_timeout', idleMs: unansweredMs }]);
      try {
        this._ws.close();
      } catch {}
    }, Math.min(5000, this._pongTimeoutMs));
  }

  private _stopPongWatch(): void {
    if (this._pongWatchTimer !== null) {
      clearInterval(this._pongWatchTimer);
      this._pongWatchTimer = null;
    }
  }

  private _noteIncomingFrame(data: RpcFrame): boolean {
    this._lastFrameAt = Date.now();
    if (data?.type === 'pong') {
      this._lastPongAt = Date.now();
      return true;
    }
    if (!data || data.type !== 'event' || typeof data.seq !== 'number') return true;

    const seq = data.seq;
    if (this._lastSeq > 0 && seq !== this._lastSeq + 1) {
      const detail = { expected: this._lastSeq + 1, actual: seq, event: data.event };
      // 先推进水位、再通知、再关连接。
      //
      // 原实现只在「正常帧」分支推进 `_lastSeq`，断档分支只 close()：
      // 一旦 close 没能走到 onclose（被监听器异常打断、或 readyState 已是 CLOSING），
      // 之后每一帧都会被判成新的断档而永久丢弃 —— 连接却仍是 OPEN、状态仍是 connected，
      // 页面表现为「事件全部停摆但没有任何报错」。推进水位后最坏也只是丢掉断档那一帧。
      this._lastSeq = Math.max(this._lastSeq, seq);
      this._emit('_gap', [detail]);
      try {
        this._ws?.close();
      } catch {}
      return false;
    }
    this._lastSeq = seq;
    return true;
  }

  private _startTickWatch(): void {
    this._stopTickWatch();
    const tickMs = (this._policy?.tick_interval_ms as number) || 30000;
    this._tickTimeoutMs = Math.max(10000, tickMs * 2.5);
    this._lastFrameAt = Date.now();
    this._tickWatchTimer = setInterval(() => {
      if (!this._ws || this._ws.readyState !== WebSocket.OPEN) return;
      const idleMs = Date.now() - this._lastFrameAt;
      if (idleMs <= this._tickTimeoutMs) return;
      this._emit('_gap', [{ reason: 'tick_timeout', idleMs }]);
      try {
        this._ws.close();
      } catch {}
    }, Math.min(tickMs, 10000));
  }

  private _stopTickWatch(): void {
    if (this._tickWatchTimer !== null) {
      clearInterval(this._tickWatchTimer);
      this._tickWatchTimer = null;
    }
  }

  private _scheduleReconnect(immediate: boolean = false): void {
    if (!this._autoReconnect) return;
    this._clearReconnectTimer();
    // 指数退避 + 抖动（缩至 0.8–1.0 倍）：多端同时断连时错开重连时刻，
    // 且不越过退避上限，避免对网关的瞬时重连风暴。
    const base = immediate ? 0 : this._reconnectDelay;
    const jitter = 0.8 + Math.random() * 0.2;
    const delay = immediate ? 0 : Math.max(0, Math.round(base * jitter));
    this._reconnectTimer = setTimeout(() => {
      this._reconnectTimer = null;
      if (!this._autoReconnect || this._ws) return;
      this._doConnect();
    }, delay);
    if (immediate) return;
    this._reconnectDelay = Math.min(
      this._reconnectDelay * this._reconnectFactor,
      this._maxReconnectDelay
    );
  }

  private _setState(s: ConnectionState): void {
    if (this._state === s) return;
    this._state = s;
    this._emit('_state', [s]);
  }

  /**
   * 安全分发：单个监听器抛异常不得影响同帧其它监听器，也不得打断连接状态机。
   *
   * 真机事故（2026-10-24）：编排卡片与聊天等待指示同时停在 31s，页面却没有任何报错。
   * 当时所有分发点都是裸 `forEach` —— 任一监听器抛异常都会让**同一帧**剩余的监听器
   * （包括 `*` 上的看门狗、`_gap` 断档回调）静默失联，而且异常发生在 onmessage 内。
   * 现在逐监听器 try/catch：异常只上报，不影响本帧其余订阅者与后续帧。
   */
  private _emit(event: string, args: unknown[]): void {
    const handlers = this._listeners.get(event);
    if (!handlers || handlers.size === 0) return;
    for (const handler of [...handlers]) {
      try {
        (handler as (...params: unknown[]) => void)(...args);
      } catch (error) {
        console.error(`[RPC] listener for "${event}" threw; dispatch continues`, error);
      }
    }
  }
}
