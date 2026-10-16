<template>
  <div class="control-stage terminal-stage">
    <header class="control-stage__header">
      <div class="control-stage__title-block">
        <h1 class="control-stage__title">终端沙箱</h1>
        <p class="control-stage__subtitle">独立终端页：sandbox.terminal.spawn / input / read / kill（与工作台面板同源）</p>
      </div>
      <div class="control-stage__actions">
        <span class="terminal-badge" :class="{ 'is-running': running }">
          <span class="terminal-dot" aria-hidden="true"></span>
          {{ connected ? (running ? '运行中' : '会话已结束') : '未连接' }}
        </span>
        <button class="btn btn--ghost" :disabled="busy" @click="restart">
          <Icon name="refresh" :size="16" /><span>重启</span>
        </button>
        <button class="btn btn--ghost" :disabled="busy || !connected" @click="kill">
          <Icon name="stop" :size="16" /><span>终止</span>
        </button>
      </div>
    </header>

    <section v-if="error" class="terminal-error">{{ error }}</section>

    <section class="terminal-shell">
      <div class="terminal-shell__head">
        <code v-if="cwd">{{ cwd }}</code>
        <code v-else>sandbox shell</code>
        <span class="terminal-shell__hint">回车执行 · Ctrl+C 语义经 input 发送</span>
      </div>
      <pre ref="outputRef" class="terminal-output" data-testid="terminal-output">{{ output || '（等待输出…）' }}</pre>
      <form class="terminal-input-row" @submit.prevent="send">
        <span class="terminal-prompt" aria-hidden="true">&gt;</span>
        <input
          v-model="command"
          class="terminal-input"
          type="text"
          autocomplete="off"
          spellcheck="false"
          :disabled="!connected || !running"
          placeholder="输入命令（如 pwd / dir / echo hello）"
          data-testid="terminal-input"
        />
        <button class="btn btn--primary" type="submit" :disabled="!connected || !running || !command.trim()">
          <Icon name="send" :size="14" /><span>发送</span>
        </button>
      </form>
    </section>

    <p class="terminal-note">终端在工作区沙箱内执行（网关回环托管）；会话随页面销毁而终止。若需长期后台任务请使用 Cron / 任务队列。</p>
  </div>
</template>

<script setup lang="ts">
import { nextTick, onMounted, onUnmounted, ref } from 'vue'
import Icon from '@/components/Icon.vue'
import { useRpcStore } from '@/stores/rpc'

interface SpawnResult { ok?: boolean; terminal?: { id: string; cwd: string; running: boolean }; error?: string }
interface ReadResult { ok?: boolean; output?: string; running?: boolean; error?: string }

const rpc = useRpcStore()
const connected = ref(false)
const running = ref(false)
const cwd = ref('')
const output = ref('')
const command = ref('')
const error = ref('')
const busy = ref(false)
const outputRef = ref<HTMLElement | null>(null)

let terminalId = ''
let pollTimer: number | null = null
let killed = false

function stopPolling(): void {
  if (pollTimer !== null) {
    window.clearInterval(pollTimer)
    pollTimer = null
  }
}

function startPolling(): void {
  stopPolling()
  pollTimer = window.setInterval(() => { void poll() }, 400)
}

async function spawn(): Promise<void> {
  busy.value = true
  error.value = ''
  try {
    const r = await rpc.call<SpawnResult>('sandbox.terminal.spawn')
    if (r.ok && r.terminal) {
      terminalId = r.terminal.id
      cwd.value = r.terminal.cwd || ''
      connected.value = true
      running.value = r.terminal.running !== false
      output.value = ''
      startPolling()
    } else {
      connected.value = false
      running.value = false
      error.value = r.error || '终端启动失败'
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
    const r = await rpc.call<ReadResult>('sandbox.terminal.read', { terminalId })
    if (r.ok) {
      output.value = String(r.output ?? '')
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
    await rpc.call('sandbox.terminal.input', { terminalId, data })
  } catch {
    // 输入失败不中断；read 轮询会反映实际状态
  }
}

async function kill(): Promise<void> {
  const id = terminalId
  terminalId = ''
  stopPolling()
  connected.value = false
  running.value = false
  if (!id || killed) return
  killed = true
  try {
    await rpc.call('sandbox.terminal.kill', { terminalId: id })
  } catch {
    // 忽略终止失败
  }
  killed = false
}

async function restart(): Promise<void> {
  await kill()
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
.terminal-error { margin: 12px 0; padding: 10px 14px; background: color-mix(in srgb, var(--danger) 10%, var(--card)); border: 1px solid color-mix(in srgb, var(--danger) 30%, transparent); border-radius: var(--radius-sm); color: var(--danger); font-size: 12px; }
.terminal-shell { background: var(--card); border: 1px solid var(--hairline); border-radius: var(--radius-md); overflow: hidden; }
.terminal-shell__head { display: flex; justify-content: space-between; align-items: center; gap: 10px; padding: 8px 12px; border-bottom: 1px solid var(--hairline); font-size: 12px; color: var(--text-dim); }
.terminal-shell__head code { font-family: var(--font-mono); }
.terminal-shell__hint { font-size: 11px; }
.terminal-output { min-height: 320px; max-height: 60vh; overflow: auto; margin: 0; padding: 14px 12px; background: var(--bg); font-family: var(--font-mono); font-size: 12px; line-height: 1.6; white-space: pre-wrap; word-break: break-all; color: var(--text); }
.terminal-input-row { display: flex; gap: 8px; align-items: center; padding: 10px 12px; border-top: 1px solid var(--hairline); }
.terminal-prompt { font-family: var(--font-mono); color: var(--accent); font-weight: 700; }
.terminal-input { flex: 1; padding: 9px 12px; border: 1px solid var(--hairline); border-radius: var(--radius-sm); background: var(--bg); color: var(--text); font-family: var(--font-mono); font-size: 12px; }
.terminal-note { margin: 12px 0 0; font-size: 12px; color: var(--text-dim); }
</style>
