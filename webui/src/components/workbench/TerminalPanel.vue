<!--
  沙箱终端面板（真实 PTY）。

  2026-10-25 重做要点：
  · 真实 PTY：shell 可选（PowerShell 7 / Windows PowerShell / cmd / Git Bash / WSL / bash…）；
  · Ctrl+C 走 `sandbox.terminal.key`，能真正中断前台命令（旧实现只能杀整个会话）；
  · 面板尺寸变化同步给 PTY（cols/rows），全屏程序与折行才正确；
  · 输出累积 + 游标读取，断线重连不丢内容；服务端缓冲淘汰时如实提示丢弃字节数；
  · 多工作区：spawn 传 `roots`，顶部展示工作根数量，悬停列出路径。
-->
<template>
  <section class="terminal-panel">
    <div class="terminal-panel__status" role="status">
      <span
        class="terminal-panel__dot"
        :class="{ 'is-running': running }"
        aria-hidden="true"
      />
      <span class="terminal-panel__state">
        <template v-if="connected && running">{{ t('workbench.terminal.running') }}</template>
        <template v-else-if="connected">{{ endedLabel }}</template>
        <template v-else>{{ t('workbench.terminal.disconnected') }}</template>
      </span>
      <label v-if="availableShells.length > 1" class="terminal-panel__shell">
        <span class="terminal-panel__shell-label">{{ t('workbench.terminal.shell') }}</span>
        <select
          class="terminal-panel__select"
          :value="shell"
          :disabled="!connected"
          data-testid="terminal-shell"
          @change="onShellChange"
        >
          <option
            v-for="item in availableShells"
            :key="item.id"
            :value="item.id"
          >{{ item.label }}</option>
        </select>
      </label>
      <span v-else-if="shellLabel" class="terminal-panel__shell-label">{{ shellLabel }}</span>
      <span
        v-if="rootCount > 1"
        class="terminal-panel__roots"
        :title="rootsHint"
        :aria-label="rootsHint"
        data-testid="terminal-roots"
      >{{ t('workbench.terminal.roots.count', { n: rootCount }) }}</span>
      <span v-if="cwd" class="terminal-panel__cwd" :title="cwd">{{ cwd }}</span>
      <span class="terminal-panel__actions">
        <button
          type="button"
          class="terminal-panel__icon"
          :title="t('workbench.terminal.interrupt')"
          :aria-label="t('workbench.terminal.interrupt')"
          :disabled="!connected || !running"
          data-testid="terminal-interrupt"
          @click="emitEvent('terminal-interrupt')"
        >
          <Icon name="stop" :size="13" />
        </button>
        <button
          type="button"
          class="terminal-panel__icon"
          :title="t('workbench.terminal.clear')"
          :aria-label="t('workbench.terminal.clear')"
          @click="clearScreen"
        >
          <Icon name="trash" :size="13" />
        </button>
        <button
          type="button"
          class="terminal-panel__icon"
          :title="t('workbench.terminal.restart')"
          :aria-label="t('workbench.terminal.restart')"
          @click="emitEvent('terminal-restart')"
        >
          <Icon name="refresh" :size="13" />
        </button>
        <button
          type="button"
          class="terminal-panel__icon"
          :title="t('workbench.terminal.terminate')"
          :aria-label="t('workbench.terminal.terminate')"
          :disabled="!connected"
          @click="emitEvent('terminal-kill')"
        >
          <Icon name="x" :size="13" />
        </button>
      </span>
    </div>

    <div v-if="warning" class="terminal-panel__warning" role="status" data-testid="terminal-warning">
      <Icon name="info" :size="15" />
      <span>{{ warning }}</span>
    </div>

    <div v-if="errorMessage" class="terminal-panel__error" role="alert">
      <Icon name="info" :size="15" />
      <span>{{ errorMessage }}</span>
      <button type="button" class="btn btn--ghost" @click="emitEvent('terminal-restart')">
        {{ t('workbench.terminal.restart') }}
      </button>
    </div>

    <div
      ref="outputRef"
      class="terminal-panel__output"
      aria-live="polite"
      data-testid="terminal-output"
      @click="focusInput"
    >
      <pre v-if="output" class="terminal-panel__pre" v-html="rendered"></pre>
      <p v-else class="terminal-panel__hint">{{ t('workbench.terminal.emptyHint') }}</p>
    </div>

    <p v-if="(lostBytes ?? 0) > 0" class="terminal-panel__lost">
      {{ t('workbench.terminal.lostOutput', { n: lostBytes ?? 0 }) }}
    </p>

    <form class="terminal-panel__input-row" @submit.prevent="submit">
      <span class="terminal-panel__prompt" aria-hidden="true">&gt;</span>
      <input
        ref="inputRef"
        v-model="command"
        class="terminal-panel__input"
        type="text"
        autocomplete="off"
        autocapitalize="off"
        spellcheck="false"
        :disabled="!connected || !running"
        :placeholder="t('workbench.terminal.placeholder')"
        :aria-label="t('workbench.terminal.placeholder')"
        data-testid="terminal-input"
        @keydown.up.prevent="historyStep(-1)"
        @keydown.down.prevent="historyStep(1)"
        @keydown.ctrl.c.prevent="interrupt"
        @keydown.ctrl.l.prevent="clearScreen"
      >
      <button
        type="submit"
        class="btn btn--ghost"
        :disabled="!connected || !running || !command.trim()"
      >
        {{ t('workbench.terminal.send') }}
      </button>
    </form>
  </section>
</template>

<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import Icon from '@/components/Icon.vue'
import { ansiToHtml } from '@/utils/workbench/ansi'

interface ShellOption {
  id: string
  label: string
  available: boolean
  default?: boolean
}

const props = defineProps<{
  cwd?: string
  /** runtime 累积后的完整输出（不是增量） */
  output?: string
  running?: boolean
  connected?: boolean
  errorMessage?: string
  /** 非致命告警（如 cwd 越界后回落），不能当错误渲染 */
  warning?: string
  shell?: string
  shellLabel?: string
  shells?: ShellOption[]
  /** 本次会话的工作根（首项为主根）；多根时在顶部展示数量 */
  roots?: string[]
  /** 服务端输出缓冲淘汰导致丢失的字节数 */
  lostBytes?: number
  exitCode?: number | null
}>()

const emit = defineEmits<{
  (e: 'workbench-event', event: { type: string; payload?: Record<string, unknown> }): void
}>()

const { t } = useI18n()
const command = ref('')
const outputRef = ref<HTMLElement | null>(null)
const inputRef = ref<HTMLInputElement | null>(null)
const history = ref<string[]>([])
const historyIndex = ref(-1)
let draftBeforeHistory = ''
let resizeTimer: number | null = null
let observer: ResizeObserver | null = null
let lastGrid = { cols: 0, rows: 0 }

const availableShells = computed(() =>
  (props.shells || []).filter(item => item.available !== false),
)

const rootCount = computed(() => (props.roots || []).length)

const rootsHint = computed(() => {
  const roots = props.roots || []
  if (!roots.length) return ''
  return `${t('workbench.terminal.roots.hint')}\n${roots.join('\n')}`
})

const rendered = computed(() => ansiToHtml(props.output || ''))

const endedLabel = computed(() => {
  const code = props.exitCode
  if (typeof code === 'number' && code >= 0) {
    return `${t('workbench.terminal.ended')} · exit ${code}`
  }
  return t('workbench.terminal.ended')
})

function emitEvent(type: string, payload?: Record<string, unknown>): void {
  emit('workbench-event', { type, payload })
}

function focusInput(): void {
  inputRef.value?.focus({ preventScroll: true })
}

function clearScreen(): void {
  emitEvent('terminal-clear')
}

function interrupt(): void {
  emitEvent('terminal-interrupt')
  command.value = ''
}

function onShellChange(event: Event): void {
  const value = (event.target as HTMLSelectElement | null)?.value || ''
  if (!value) return
  emitEvent('terminal-set-shell', { shell: value })
}

function submit(): void {
  const data = command.value.trim()
  if (!data || !props.connected || !props.running) return
  history.value = [...history.value.filter(item => item !== data), data].slice(-100)
  historyIndex.value = -1
  draftBeforeHistory = ''
  emitEvent('terminal-input', { data })
  command.value = ''
}

function historyStep(delta: number): void {
  if (!history.value.length) return
  if (historyIndex.value === -1) {
    if (delta > 0) return
    draftBeforeHistory = command.value
    historyIndex.value = history.value.length - 1
  } else {
    const next = historyIndex.value + delta
    if (next >= history.value.length) {
      historyIndex.value = -1
      command.value = draftBeforeHistory
      return
    }
    if (next < 0) return
    historyIndex.value = next
  }
  command.value = history.value[historyIndex.value] || ''
  void nextTick(() => {
    const el = inputRef.value
    if (!el) return
    el.setSelectionRange(el.value.length, el.value.length)
  })
}

/**
 * 按真实字体宽度估算终端网格：PTY 收到错误的 cols 会让 `ls` / 进度条 / 折行全部错位。
 * 用隐藏探针量等宽字符宽度，避免依赖硬编码 8px（不同主题字号不同）。
 */
function measureGrid(el: HTMLElement): { cols: number; rows: number } {
  const style = window.getComputedStyle(el)
  const probe = document.createElement('span')
  probe.textContent = 'M'.repeat(40)
  probe.style.position = 'absolute'
  probe.style.visibility = 'hidden'
  probe.style.whiteSpace = 'pre'
  // `style.font` 简写在部分浏览器里是空串；逐项复制才量得准。
  probe.style.fontFamily = style.fontFamily
  probe.style.fontSize = style.fontSize
  probe.style.fontWeight = style.fontWeight
  probe.style.fontStyle = style.fontStyle
  probe.style.letterSpacing = style.letterSpacing
  el.appendChild(probe)
  const measured = probe.getBoundingClientRect().width
  probe.remove()
  const charWidth = measured > 0 ? measured / 40 : 8
  const fontSize = Number.parseFloat(style.fontSize) || 13
  const lineHeight = Number.parseFloat(style.lineHeight) || fontSize * 1.5
  const paddingX = (Number.parseFloat(style.paddingLeft) || 0) + (Number.parseFloat(style.paddingRight) || 0)
  const paddingY = (Number.parseFloat(style.paddingTop) || 0) + (Number.parseFloat(style.paddingBottom) || 0)
  const cols = Math.max(20, Math.floor((el.clientWidth - paddingX) / Math.max(charWidth, 1)))
  const rows = Math.max(4, Math.floor((el.clientHeight - paddingY) / Math.max(lineHeight, 1)))
  return { cols, rows }
}

function scheduleResize(): void {
  if (resizeTimer !== null) window.clearTimeout(resizeTimer)
  resizeTimer = window.setTimeout(() => {
    resizeTimer = null
    const el = outputRef.value
    if (!el) return
    const grid = measureGrid(el)
    if (grid.cols === lastGrid.cols && grid.rows === lastGrid.rows) return
    lastGrid = grid
    emitEvent('terminal-resize', grid)
  }, 160)
}

watch(
  () => props.output,
  () => {
    void nextTick(() => {
      const el = outputRef.value
      if (el) el.scrollTop = el.scrollHeight
    })
  },
)

watch(
  () => props.connected,
  (connected) => {
    if (connected) {
      focusInput()
      scheduleResize()
    }
  },
)

onMounted(() => {
  focusInput()
  void nextTick(scheduleResize)
  if (typeof ResizeObserver !== 'undefined' && outputRef.value) {
    observer = new ResizeObserver(() => scheduleResize())
    observer.observe(outputRef.value)
  }
})

onBeforeUnmount(() => {
  observer?.disconnect()
  observer = null
  if (resizeTimer !== null) window.clearTimeout(resizeTimer)
})
</script>

<style scoped>
.terminal-panel {
  /* flex 而非固定 grid 行：告警/错误/丢失提示都是条件渲染，
     固定行模板会让输出区在缺行时落到 auto 行、把 1fr 空在底部。 */
  display: flex;
  flex-direction: column;
  height: 100%;
  min-height: 0;
  gap: var(--sp-2);
  padding: var(--sp-3);
  background: var(--bg-surface);
  font-family: var(--font-mono, ui-monospace, monospace);
  font-size: var(--font-size-sm, 13px);
}

.terminal-panel__status {
  display: flex;
  align-items: center;
  gap: var(--sp-2);
  color: var(--text-dim);
  font-size: 12px;
}

.terminal-panel__state { flex: 0 0 auto; }

.terminal-panel__shell {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  flex: 0 0 auto;
}

.terminal-panel__shell-label {
  color: var(--text-dim);
  font-size: 11px;
}

.terminal-panel__select {
  max-width: 200px;
  padding: 2px 4px;
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  background: var(--bg);
  color: var(--text);
  font: inherit;
  font-size: 11px;
}

.terminal-panel__roots {
  flex: 0 0 auto;
  padding: 0 6px;
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  color: var(--text-muted);
  font-size: 11px;
  white-space: nowrap;
  cursor: help;
}

.terminal-panel__cwd {
  min-width: 0;
  flex: 1;
  overflow: hidden;
  color: var(--text-dim);
  font-size: 11px;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.terminal-panel__actions { display: inline-flex; flex: 0 0 auto; gap: 2px; }
.terminal-panel__icon {
  display: inline-flex;
  width: 24px;
  height: 24px;
  align-items: center;
  justify-content: center;
  padding: 0;
  border: 0;
  border-radius: var(--radius-sm);
  background: transparent;
  color: var(--text-muted);
  cursor: pointer;
}
.terminal-panel__icon:hover:not(:disabled) { background: var(--bg-hover); color: var(--text); }
.terminal-panel__icon:disabled { cursor: not-allowed; opacity: 0.45; }

.terminal-panel__dot {
  width: 8px;
  height: 8px;
  flex: 0 0 auto;
  border-radius: 50%;
  background: var(--text-dim);
}

.terminal-panel__dot.is-running {
  background: var(--ok);
}

.terminal-panel__warning {
  display: flex;
  align-items: center;
  gap: var(--sp-2);
  padding: var(--sp-2);
  border-radius: var(--radius-sm);
  background: color-mix(in srgb, var(--warn) 12%, transparent);
  color: var(--warn);
}

.terminal-panel__error {
  display: flex;
  align-items: center;
  gap: var(--sp-2);
  padding: var(--sp-2);
  border-radius: var(--radius-sm);
  background: color-mix(in srgb, var(--danger) 12%, transparent);
  color: var(--danger);
}

.terminal-panel__lost {
  margin: 0;
  color: var(--warn);
  font-size: 11px;
}

.terminal-panel__output {
  flex: 1 1 auto;
  overflow: auto;
  min-height: 0;
  padding: var(--sp-2);
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  background: var(--bg);
  cursor: text;
}

.terminal-panel__pre {
  margin: 0;
  color: var(--text);
  font: inherit;
  line-height: 1.5;
  white-space: pre-wrap;
  word-break: break-word;
}

.terminal-panel__hint {
  margin: 0;
  color: var(--text-muted);
}

.terminal-panel__input-row {
  display: flex;
  align-items: center;
  gap: var(--sp-2);
}

.terminal-panel__prompt {
  color: var(--accent);
  font-weight: 700;
}

.terminal-panel__input {
  flex: 1;
  min-width: 0;
  padding: var(--sp-1) var(--sp-2);
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  background: var(--bg);
  color: var(--text);
  font-family: inherit;
}

.terminal-panel__input:disabled {
  opacity: 0.55;
}

/* ANSI 颜色：全部走主题变量，深色/浅色皮肤都跟着换 */
.terminal-panel__pre :deep(.t-ansi-black) { color: var(--text-dim); }
.terminal-panel__pre :deep(.t-ansi-red) { color: var(--danger); }
.terminal-panel__pre :deep(.t-ansi-green) { color: var(--ok); }
.terminal-panel__pre :deep(.t-ansi-yellow) { color: var(--warn); }
.terminal-panel__pre :deep(.t-ansi-blue) { color: var(--info, var(--accent)); }
.terminal-panel__pre :deep(.t-ansi-magenta) { color: var(--accent-secondary, var(--accent)); }
.terminal-panel__pre :deep(.t-ansi-cyan) { color: var(--accent); }
.terminal-panel__pre :deep(.t-ansi-white) { color: var(--text); }
.terminal-panel__pre :deep(.t-ansi-bright-black) { color: var(--text-muted); }
.terminal-panel__pre :deep(.t-ansi-bright-red) { color: var(--danger); font-weight: 600; }
.terminal-panel__pre :deep(.t-ansi-bright-green) { color: var(--ok); font-weight: 600; }
.terminal-panel__pre :deep(.t-ansi-bright-yellow) { color: var(--warn); font-weight: 600; }
.terminal-panel__pre :deep(.t-ansi-bright-blue) { color: var(--info, var(--accent)); font-weight: 600; }
.terminal-panel__pre :deep(.t-ansi-bright-magenta) { color: var(--accent-secondary, var(--accent)); font-weight: 600; }
.terminal-panel__pre :deep(.t-ansi-bright-cyan) { color: var(--accent); font-weight: 600; }
.terminal-panel__pre :deep(.t-ansi-bright-white) { color: var(--text); font-weight: 600; }
.terminal-panel__pre :deep(.t-ansi-bold) { font-weight: 600; }
.terminal-panel__pre :deep(.t-ansi-dim) { opacity: 0.7; }
</style>
