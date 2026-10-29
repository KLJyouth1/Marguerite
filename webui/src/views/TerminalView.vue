<template>
  <div class="control-stage terminal-stage" :class="{ 'terminal-stage--focus': focusMode }">
    <header class="control-stage__header">
      <div class="control-stage__title-block">
        <h1 class="control-stage__title">{{ t('terminal.title') }}</h1>
        <p class="control-stage__subtitle">{{ t('terminal.subtitle') }}</p>
      </div>
      <div class="control-stage__actions">
        <span class="terminal-badge" :class="{ 'is-running': running }">
          <span class="terminal-dot" aria-hidden="true"></span>
          {{ connected ? (running ? t('terminal.running') : t('terminal.ended')) : t('terminal.disconnected') }}
        </span>
        <!-- Focus mode: the terminal is a work surface, not a dashboard — once
             the session is running the big title block is dead vertical space.
             The toggle keeps state per-view (not persisted): it reads as a
             temporary zoom, so reopening the page resets to the titled view. -->
        <button
          class="btn btn--ghost"
          :aria-pressed="focusMode"
          :title="focusMode ? t('chrome.focusModeOff') : t('chrome.focusMode')"
          @click="focusMode = !focusMode"
        >
          <Icon :name="focusMode ? 'collapse' : 'expand'" :size="16" />
          <span>{{ focusMode ? t('chrome.focusModeOff') : t('chrome.focusMode') }}</span>
        </button>
        <button class="btn btn--ghost" :disabled="busy || !connected || !running" @click="interrupt">
          <Icon name="stop" :size="16" /><span>{{ t('terminal.interrupt') }}</span>
        </button>
        <button class="btn btn--ghost" :disabled="busy" @click="restart">
          <Icon name="refresh" :size="16" /><span>{{ t('terminal.restart') }}</span>
        </button>
        <button class="btn btn--ghost" :disabled="busy || !connected" @click="kill">
          <Icon name="x" :size="16" /><span>{{ t('terminal.kill') }}</span>
        </button>
      </div>
    </header>

    <div v-if="warning" class="terminal-warning" role="status" data-testid="terminal-warning">
      <Icon name="info" :size="15" /><span>{{ warning }}</span>
    </div>

    <ErrorState v-if="error" :message="error" :on-retry="restart" />

    <section class="terminal-shell">
      <div class="terminal-shell__head">
        <code v-if="cwd">{{ cwd }}</code>
        <code v-else>sandbox shell</code>
        <span
          v-if="roots.length > 1"
          class="terminal-shell__roots"
          :title="rootsHint"
          data-testid="terminal-roots"
        >{{ t('terminal.roots.count', { n: roots.length }) }}</span>
        <span v-if="shellLabel" class="terminal-shell__shell">{{ shellLabel }}</span>
        <span class="terminal-shell__hint">{{ t('terminal.hint') }}</span>
      </div>
      <pre ref="outputRef" class="terminal-output" data-testid="terminal-output">{{ output || t('terminal.waitingOutput') }}</pre>
      <p v-if="lostBytes > 0" class="terminal-lost">{{ t('terminal.lostOutput', { n: lostBytes }) }}</p>
      <form class="terminal-input-row" @submit.prevent="send">
        <span class="terminal-prompt" aria-hidden="true">&gt;</span>
        <input
          v-model="command"
          class="terminal-input"
          type="text"
          autocomplete="off"
          spellcheck="false"
          :disabled="!connected || !running"
          :placeholder="t('terminal.inputPlaceholder')"
          data-testid="terminal-input"
          @keydown.ctrl.c.prevent="interrupt"
        />
        <button class="btn btn--primary" type="submit" :disabled="!connected || !running || !command.trim()">
          <Icon name="send" :size="14" /><span>{{ t('terminal.send') }}</span>
        </button>
      </form>
    </section>

    <p class="terminal-note">{{ t('terminal.note') }}</p>
  </div>
</template>

<script setup lang="ts">
import { computed, nextTick, onMounted, onUnmounted, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { useRoute } from 'vue-router'
import ErrorState from '@/components/ErrorState.vue'
import Icon from '@/components/Icon.vue'
import { useRpcStore } from '@/stores/rpc'
import { useSessions } from '@/composables/useSessions'
import { notifyReadFailure } from '@/utils/readFailureNotice'

interface SpawnResult {
  ok?: boolean
  terminal?: {
    id: string
    cwd: string
    running: boolean
    shell?: string
    shellLabel?: string
    cursor?: number
  }
  /** 后端实际接受的合法根（越界回落时与请求值不同）。 */
  roots?: string[] | null
  error?: string
  warning?: string | null
}
interface ReadResult {
  ok?: boolean
  output?: string
  cursor?: number
  lost?: number
  running?: boolean
  exitCode?: number | null
  error?: string
}

const MAX_BUFFER_CHARS = 400_000

const rpc = useRpcStore()
const route = useRoute()
const sessionStore = useSessions()
const { t } = useI18n()
const connected = ref(false)
const running = ref(false)
const cwd = ref('')
const output = ref('')
const command = ref('')
const error = ref('')
const warning = ref('')
const shellLabel = ref('')
const lostBytes = ref(0)
const busy = ref(false)
const roots = ref<string[]>([])
const outputRef = ref<HTMLElement | null>(null)
/** 专注模式：隐藏标题头，把垂直空间还给输出区（视图内瞬态，不持久化）。 */
const focusMode = ref(false)

let terminalId = ''
let pollTimer: number | null = null
let killed = false
// 游标读取：只追加增量，避免「两次轮询之间有人取走输出」导致丢内容。
let cursor = 0

/**
 * 纯文本回放器的跨分片残留：一个 CSI 序列可能被拆在两次 `sandbox.terminal.read`
 * 之间（例如 `\u001b[` 与 `6n` 分属两片），必须按状态机跨片续接。
 */
let ansiPending = ''

/** CSI 序列里最多容许的残留字符数；超过视为流损坏，直接丢弃，避免无界缓存。 */
const ANSI_PENDING_MAX = 64

/**
 * 剥掉 ANSI 控制序列，只留可读文本。
 *
 * 这个视图是 `<pre>` 纯文本回放（不是 xterm），没有任何光标定位能力：
 * 把 `\u001b[6n`（DSR 光标查询）原样塞进去，用户第一眼看到的就是
 * `□[6n□[6n□[6n`（ESC 是不可打印字符，浏览器渲染成方块）——真机首屏事故。
 * 顺序里同时兼容 OSC（窗口标题）与双字符转义，避免它们把后续正文一起吞掉。
 */
function stripAnsiChunk(chunk: string): string {
  const s = ansiPending + chunk
  ansiPending = ''
  let out = ''
  let i = 0
  while (i < s.length) {
    const ch = s[i]
    if (ch !== '\u001b') {
      out += ch
      i += 1
      continue
    }
    if (i + 1 >= s.length) {
      ansiPending = s.slice(i)
      break
    }
    const next = s[i + 1]
    if (next === '[') {
      // CSI：参数字节 0x30-0x3F、中间字节 0x20-0x2F、终止字节 0x40-0x7E
      let j = i + 2
      while (j < s.length && s[j] >= '\u0030' && s[j] <= '\u003f') j += 1
      while (j < s.length && s[j] >= '\u0020' && s[j] <= '\u002f') j += 1
      if (j >= s.length) {
        ansiPending = s.slice(i)
        break
      }
      i = j + 1
      continue
    }
    if (next === ']') {
      // OSC：以 BEL 或 ESC \ 结束
      let j = i + 2
      let closed = false
      while (j < s.length) {
        if (s[j] === '\u0007') {
          j += 1
          closed = true
          break
        }
        if (s[j] === '\u001b' && s[j + 1] === '\\') {
          j += 2
          closed = true
          break
        }
        j += 1
      }
      if (!closed) {
        ansiPending = s.slice(i)
        break
      }
      i = j
      continue
    }
    // 其余双字符转义（ESC 7 / ESC = / ESC M …）
    i += 2
  }
  if (ansiPending.length > ANSI_PENDING_MAX) ansiPending = ''
  // 兜底：ESC 之外的 C0 控制字符（保留 \n \r \t）也不该出现在回放文本里。
  return out.replace(/[\u0000-\u0008\u000b\u000c\u000e-\u001f\u007f]/g, '')
}

const rootsHint = computed(() =>
  roots.value.length
    ? `${t('terminal.roots.hint')}\n${roots.value.join('\n')}`
    : '',
)

/** 过滤空值、去重（忽略 `\` 与 `/`、尾斜杠差异），保留原有优先级顺序。 */
function normalizeRoots(input: unknown): string[] {
  if (!Array.isArray(input)) return []
  const seen = new Set<string>()
  const list: string[] = []
  for (const entry of input) {
    const path = typeof entry === 'string' ? entry.trim() : ''
    if (!path) continue
    const key = path.replace(/\\/g, '/').replace(/\/+$/, '')
    if (seen.has(key)) continue
    seen.add(key)
    list.push(path)
  }
  return list
}

/**
 * 当前会话的工作根：独立终端页没有工作台 item，用全站约定的 `?session=` 查询参数
 * 定位会话（与 App.vue 的 currentSessionKey 同源），再取 `projectPaths`。
 *
 * 拿不到会话（例如直接从导航进入 /terminal）时返回空数组，spawn 保持旧行为，
 * 由网关决定单一工作根。
 */
async function resolveRoots(): Promise<string[]> {
  const rawSession = route.query.session
  const sessionKey = typeof rawSession === 'string' ? rawSession.trim() : ''
  if (!sessionKey) return []
  try {
    await sessionStore.loadSessions()
  } catch {
    // 工作根读不出来：spawn 会退化成网关默认目录，终端里 `pwd` 与预期不符。
    // 这是终端类失败，必须提示「手动刷新」，不能静默。
    notifyReadFailure('terminal.status', 'terminal.statusReadFailed')
    return []
  }
  const hit = sessionStore.sessionsList.value.find((entry) => {
    if (!entry || typeof entry !== 'object') return false
    const row = entry as { key?: string; session?: string; sessionKey?: string }
    return (row.key || row.session || row.sessionKey) === sessionKey
  }) as { projectPaths?: string[]; projectPath?: string; workspace?: string } | undefined
  const paths = normalizeRoots(hit?.projectPaths)
  if (paths.length) return paths
  const single = typeof hit?.projectPath === 'string' ? hit.projectPath.trim() : ''
  if (single) return [single]
  const legacy = typeof hit?.workspace === 'string' ? hit.workspace.trim() : ''
  return legacy ? [legacy] : []
}

function stopPolling(): void {
  if (pollTimer !== null) {
    window.clearInterval(pollTimer)
    pollTimer = null
  }
}

function startPolling(): void {
  stopPolling()
  pollTimer = window.setInterval(() => { void poll() }, 250)
}

async function spawn(): Promise<void> {
  busy.value = true
  error.value = ''
  warning.value = ''
  try {
    // 冷加载竞态守卫：WS 仍处于 connecting 时先等连接就绪。
    if (!rpc.isConnected) await rpc.waitForConnection(10_000)
    const resolved = await resolveRoots()
    roots.value = resolved
    const payload: Record<string, unknown> = {}
    // 多根传 `roots`（顺序即优先级）；单根仍传 `root`，兼容旧后端。
    if (resolved.length > 1) payload.roots = resolved
    else if (resolved.length === 1) payload.root = resolved[0]
    const r = await rpc.call<SpawnResult>(
      'sandbox.terminal.spawn',
      Object.keys(payload).length ? payload : undefined,
    )
    if (r.ok && r.terminal) {
      terminalId = r.terminal.id
      cwd.value = r.terminal.cwd || ''
      shellLabel.value = r.terminal.shellLabel || ''
      cursor = Number(r.terminal.cursor ?? 0)
      connected.value = true
      running.value = r.terminal.running !== false
      warning.value = r.warning || ''
      roots.value = normalizeRoots(
        Array.isArray(r.roots) && r.roots.length ? r.roots : resolved,
      )
      output.value = ''
      ansiPending = ''
      lostBytes.value = 0
      startPolling()
    } else {
      connected.value = false
      running.value = false
      error.value = r.error || t('terminal.spawnFailed')
      warning.value = r.warning || ''
    }
  } catch (e) {
    connected.value = false
    running.value = false
    error.value = e instanceof Error ? e.message : String(e)
  } finally {
    busy.value = false
  }
}

async function poll(): Promise<void> {
  if (!terminalId) return
  try {
    const r = await rpc.call<ReadResult>('sandbox.terminal.read', { terminalId, cursor })
    if (r.ok) {
      const chunk = stripAnsiChunk(String(r.output ?? ''))
      if (chunk) {
        output.value = (output.value + chunk).slice(-MAX_BUFFER_CHARS)
      }
      if (typeof r.cursor === 'number' && r.cursor >= cursor) cursor = r.cursor
      if (typeof r.lost === 'number' && r.lost > 0) lostBytes.value = r.lost
      running.value = r.running === true
      connected.value = true
      error.value = ''
      void nextTick(() => {
        const el = outputRef.value
        if (el) el.scrollTop = el.scrollHeight
      })
      if (r.running === false) stopPolling()
    } else if (r.error) {
      connected.value = false
      error.value = r.error
      stopPolling()
    }
  } catch {
    // 轮询失败保持现状，下次重试
  }
}

async function send(): Promise<void> {
  const data = command.value.trim()
  if (!data || !terminalId || !running.value) return
  command.value = ''
  try {
    const res = await rpc.call<{ ok?: boolean; error?: string }>('sandbox.terminal.input', {
      terminalId,
      data,
    })
    if (res?.ok === false) {
      output.value = `${output.value}\r\n[拒绝] ${res.error || 'input rejected'}\r\n`.slice(-MAX_BUFFER_CHARS)
    }
  } catch (e) {
    output.value = `${output.value}\r\n[错误] ${e instanceof Error ? e.message : String(e)}\r\n`.slice(-MAX_BUFFER_CHARS)
  }
}

async function interrupt(): Promise<void> {
  if (!terminalId || !running.value) return
  try {
    await rpc.call('sandbox.terminal.key', { terminalId, sequence: 'ctrl-c' })
  } catch {
    // 中断失败不破坏会话
  }
}

/** 返回 `false` 表示服务端未确认已停止——不能谎报「已断开」。 */
async function kill(): Promise<boolean> {
  const id = terminalId
  if (!id || killed) return true
  killed = true
  try {
    const res = await rpc.call<{ ok?: boolean; error?: string }>('sandbox.terminal.kill', {
      terminalId: id,
    })
    if (res?.ok === false) {
      error.value = res.error || '终端未能终止，可能仍在运行；请重试'
      return false
    }
  } catch (e) {
    error.value = e instanceof Error ? e.message : String(e)
    return false
  } finally {
    killed = false
  }
  terminalId = ''
  stopPolling()
  connected.value = false
  running.value = false
  return true
}

async function restart(): Promise<void> {
  // 终止未确认时不重启：避免旧进程留在后台、新进程再占一套资源。
  if (!(await kill())) return
  output.value = ''
  cursor = 0
  ansiPending = ''
  lostBytes.value = 0
  error.value = ''
  await spawn()
}

onMounted(() => { void spawn() })
onUnmounted(() => {
  stopPolling()
  if (terminalId) {
    void rpc.call('sandbox.terminal.kill', { terminalId }).catch(() => {})
  }
})
</script>

<style scoped>
.terminal-stage { padding: 24px 28px; max-width: 1080px; margin: 0 auto; }
.terminal-badge { display: inline-flex; align-items: center; gap: 6px; padding: 5px 10px; border-radius: 999px; background: color-mix(in srgb, var(--text) 8%, var(--card)); font-size: 12px; color: var(--text-dim); }
.terminal-dot { width: 8px; height: 8px; border-radius: 50%; background: var(--text-dim); }
.terminal-badge.is-running .terminal-dot { background: var(--accent); }
.terminal-warning { display: flex; align-items: center; gap: 8px; margin-bottom: 12px; padding: 8px 12px; border-radius: var(--radius-sm); background: color-mix(in srgb, var(--warn) 12%, transparent); color: var(--warn); font-size: 12px; }
.terminal-shell { background: var(--card); border: 1px solid var(--hairline); border-radius: var(--radius-md); overflow: hidden; }
.terminal-shell__head { display: flex; justify-content: space-between; align-items: center; gap: 10px; padding: 8px 12px; border-bottom: 1px solid var(--hairline); font-size: 12px; color: var(--text-dim); }
.terminal-shell__head code { font-family: var(--font-mono); }
.terminal-shell__roots { flex: 0 0 auto; padding: 0 8px; border: 1px solid var(--hairline); border-radius: 999px; font-size: 11px; color: var(--text-dim); cursor: help; white-space: nowrap; }
.terminal-shell__shell { font-size: 11px; color: var(--accent); }
.terminal-shell__hint { font-size: 11px; }
/* 输出区随视口生长：旧实现固定 320px/60vh，高窗下方全是死留白（拥挤感的
   反面——浪费）。高度 = 视口减去标题头/输入行/脚注的实测开销，再夹住上下
   界；专注模式隐藏标题头后，同式给出更大的偏移。 */
.terminal-output {
  height: clamp(280px, calc(100vh - 320px), 1600px);
  overflow: auto;
  margin: 0;
  padding: 14px 12px;
  background: var(--bg);
  font-family: var(--font-mono);
  font-size: 12px;
  line-height: 1.6;
  white-space: pre-wrap;
  word-break: break-all;
  color: var(--text);
}
.terminal-stage--focus .control-stage__header { display: none; }
.terminal-stage--focus .terminal-output { height: clamp(280px, calc(100vh - 200px), 1800px); }
.terminal-lost { margin: 0; padding: 4px 12px; font-size: 11px; color: var(--warn); border-top: 1px solid var(--hairline); }
.terminal-input-row { display: flex; gap: 8px; align-items: center; padding: 10px 12px; border-top: 1px solid var(--hairline); }
.terminal-prompt { font-family: var(--font-mono); color: var(--accent); font-weight: 700; }
.terminal-input { flex: 1; padding: 9px 12px; border: 1px solid var(--hairline); border-radius: var(--radius-sm); background: var(--bg); color: var(--text); font-family: var(--font-mono); font-size: 12px; }
.terminal-note { margin: 12px 0 0; font-size: 12px; color: var(--text-dim); }
</style>
