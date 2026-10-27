<!--
  内置浏览器（工作台面板）：真实 Chromium / CDP 会话的镜像视图。

  两条真实边界（都刻意写在界面上，不粉饰）：
  · 本面板驱动的是 `browser.*` RPC 的 CDP 会话；`tool.browser` 用的是另一次
    无头实例 —— 因此这里显示的是**你自己的**页面，不是智能体的画面。
  · 「接管」是**硬锁**：browser.control.set owner=user 后，智能体侧任何页面动作
    都会被后端拒绝（USER_IN_CONTROL），直到你交还。交还时会尝试把接管摘要
    作为插话（sessions.steer.v2）送进运行中的回合；回合已结束时如实告知未送达。
-->
<template>
  <div class="pd-browser" :class="{ 'is-agent': agentOwns && agentActive, 'is-user': userOwns }">
    <div class="pd-browser__bar">
      <input
        v-model="url"
        class="pd-browser__addr"
        :placeholder="t('workbench.browserCdp.address')"
        :aria-label="t('workbench.browserCdp.address')"
        spellcheck="false"
        @keyup.enter="openPage()"
      />
      <button type="button" class="pd-browser__btn" :disabled="busy" @click="openPage()">
        <Icon name="languages" :size="14" aria-hidden="true" />
        <span>{{ sessionId ? t('workbench.browserCdp.go') : t('workbench.browserCdp.open') }}</span>
      </button>
      <button type="button" class="pd-browser__btn" :disabled="busy || !sessionId" @click="refreshShot()">
        <Icon name="refresh" :size="14" aria-hidden="true" />
        <span>{{ t('workbench.browserCdp.refresh') }}</span>
      </button>
      <button
        v-if="controlReady"
        type="button"
        class="pd-browser__btn"
        :class="{ 'is-active': userOwns }"
        :disabled="busy"
        :title="userOwns ? t('workbench.browserCdp.handbackHint') : t('workbench.browserCdp.takeoverHint')"
        @click="toggleOwnership()"
      >
        <Icon :name="userOwns ? 'share' : 'cursor'" :size="14" aria-hidden="true" />
        <span>{{ userOwns ? t('workbench.browserCdp.handback') : t('workbench.browserCdp.takeover') }}</span>
      </button>
    </div>

    <div class="pd-browser__status">
      <span class="pd-browser__pill" :class="ownerClass">
        {{ userOwns ? t('workbench.browserCdp.youOwn') : t('workbench.browserCdp.agentOwns') }}
      </span>
      <span v-if="agentActive" class="pd-browser__pill pd-browser__pill--live">
        {{ t('workbench.browserCdp.agentActive', { n: agentSteps }) }}
      </span>
      <span class="pd-browser__meta">{{ t('workbench.browserCdp.separateSession') }}</span>
    </div>

    <p v-if="status === null" class="pd-browser__note">
      {{ t('workbench.browserCdp.detecting') }}
      <button type="button" class="pd-browser__link" @click="loadStatus()">{{ t('workbench.browserCdp.detect') }}</button>
    </p>
    <p v-else-if="!status.available" class="pd-browser__error">
      {{ status.reason || t('workbench.browserCdp.unavailable') }}
    </p>
    <p v-if="error" class="pd-browser__error">{{ error }}</p>
    <p v-if="note" class="pd-browser__note">{{ note }}</p>

    <div v-if="status && status.available" class="pd-browser__view" :class="{ 'is-locked': !userOwns }">
      <img
        v-if="shot"
        :src="shot"
        class="pd-browser__shot"
        :alt="t('workbench.browserCdp.shotAlt')"
        @click="onShotClick"
      />
      <div v-else class="pd-browser__empty">{{ t('workbench.browserCdp.noShot') }}</div>
      <div v-if="shot" class="pd-browser__overlay">
        <span v-if="userOwns">{{ t('workbench.browserCdp.clickHint') }}</span>
        <span v-else>{{ t('workbench.browserCdp.lockedHint') }}</span>
      </div>
    </div>

    <section class="pd-browser__log" :aria-label="t('workbench.browserCdp.logTitle')">
      <header>{{ t('workbench.browserCdp.logTitle') }}</header>
      <ul>
        <li v-for="(row, i) in log" :key="i">
          <span class="pd-browser__log-time">{{ row.at }}</span>
          <span class="pd-browser__log-kind">{{ row.kind }}</span>
          <span class="pd-browser__log-text">{{ row.text }}</span>
        </li>
        <li v-if="!log.length" class="pd-browser__log-empty">{{ t('workbench.browserCdp.logEmpty') }}</li>
      </ul>
    </section>
  </div>
</template>

<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import Icon from '@/components/Icon.vue'
import { useRpcStore } from '@/stores/rpc'

const props = defineProps<{
  scopeId: string
  initialUrl?: string
  active?: boolean
}>()

const { t } = useI18n()
const rpc = useRpcStore()

interface BrowserStatus { available?: boolean; reason?: string }
interface OpenResult { sessionId?: string; title?: string; url?: string }
interface ShotResult { dataUrl?: string }
interface ControlState { owner?: string; sinceLocal?: string | null; reason?: string }
interface LogRow { at: string; kind: string; text: string }

const status = ref<BrowserStatus | null>(null)
const url = ref(props.initialUrl || 'https://example.com')
const sessionId = ref('')
const shot = ref('')
const busy = ref(false)
const error = ref('')
const note = ref('')
const log = ref<LogRow[]>([])
const control = ref<ControlState>({ owner: 'agent' })
const controlReady = ref(false)
const agentActive = ref(false)
const agentSteps = ref(0)
let unsubAgent: (() => void) | null = null
let pollTimer: ReturnType<typeof setInterval> | null = null
let agentIdleTimer: ReturnType<typeof setTimeout> | null = null

const userOwns = computed(() => control.value.owner === 'user')
const agentOwns = computed(() => control.value.owner !== 'user')
const ownerClass = computed(() => (userOwns.value ? 'pd-browser__pill--user' : 'pd-browser__pill--agent'))

function stamp(): string {
  const d = new Date()
  return `${String(d.getHours()).padStart(2, '0')}:${String(d.getMinutes()).padStart(2, '0')}:${String(d.getSeconds()).padStart(2, '0')}`
}

function pushLog(kind: string, text: string): void {
  log.value = [{ at: stamp(), kind, text }, ...log.value].slice(0, 30)
}

async function ensureConnected(): Promise<boolean> {
  if (rpc.isConnected) return true
  try {
    await rpc.waitForConnection(10_000)
    return rpc.isConnected
  } catch {
    return false
  }
}

/** 能力探测：冷加载（面板刚挂载、WS 仍在 connecting）要重试几次，
 *  否则 `status` 恒为 null、面板只剩一个空框 —— 真机上就是这样。 */
async function loadStatus(retries = 3): Promise<void> {
  if (!(await ensureConnected())) {
    if (retries > 0) setTimeout(() => void loadStatus(retries - 1), 1500)
    return
  }
  try {
    status.value = await rpc.call<BrowserStatus>('browser.status', {})
  } catch (err) {
    error.value = err instanceof Error ? err.message : String(err)
    status.value = { available: false, reason: error.value }
  }
}

async function loadControl(): Promise<void> {
  if (!rpc.isConnected) return
  try {
    const res = await rpc.call<ControlState>('browser.control.get', {})
    control.value = { owner: res?.owner || 'agent', sinceLocal: res?.sinceLocal ?? null, reason: res?.reason || '' }
    controlReady.value = true
  } catch {
    // 老网关没有该方法：保持「智能体侧」并隐藏接管按钮（controlReady=false）
    controlReady.value = false
  }
}

async function openPage(): Promise<void> {
  if (busy.value) return
  const target = url.value.trim()
  if (!target) return
  if (!(await ensureConnected())) return
  busy.value = true
  error.value = ''
  note.value = ''
  try {
    const res = await rpc.call<OpenResult>('browser.open', { url: target })
    sessionId.value = res?.sessionId || ''
    if (res?.url) url.value = res.url
    pushLog(t('workbench.browserCdp.logOpen'), res?.url || target)
    await refreshShot()
  } catch (err) {
    error.value = err instanceof Error ? err.message : String(err)
  } finally {
    busy.value = false
  }
}

async function refreshShot(): Promise<void> {
  if (!sessionId.value || !rpc.isConnected) return
  try {
    const res = await rpc.call<ShotResult>('browser.screenshot', { sessionId: sessionId.value })
    if (res?.dataUrl) shot.value = res.dataUrl
  } catch (err) {
    error.value = err instanceof Error ? err.message : String(err)
  }
}

async function onShotClick(ev: MouseEvent): Promise<void> {
  // 只读镜像态不接管用户点击：那是「看到的画面」，不是可交互页面
  if (!userOwns.value || !sessionId.value || busy.value) {
    note.value = t('workbench.browserCdp.takeoverRequired')
    return
  }
  const img = ev.target as HTMLImageElement
  const rect = img.getBoundingClientRect()
  if (!rect.width || !rect.height) return
  const scaleX = (img.naturalWidth || rect.width) / rect.width
  const scaleY = (img.naturalHeight || rect.height) / rect.height
  const x = (ev.clientX - rect.left) * scaleX
  const y = (ev.clientY - rect.top) * scaleY
  busy.value = true
  try {
    await rpc.call('browser.click', { sessionId: sessionId.value, x, y })
    pushLog(t('workbench.browserCdp.logClick'), `(${Math.round(x)}, ${Math.round(y)})`)
    await refreshShot()
  } catch (err) {
    error.value = err instanceof Error ? err.message : String(err)
  } finally {
    busy.value = false
  }
}

/** 接管 / 交还：真正切换后端控制权租约（不是本地样式）。 */
async function toggleOwnership(): Promise<void> {
  if (busy.value || !(await ensureConnected())) return
  busy.value = true
  note.value = ''
  error.value = ''
  try {
    if (!userOwns.value) {
      const res = await rpc.call<ControlState>('browser.control.set', {
        owner: 'user',
        reason: t('workbench.browserCdp.takeoverReason'),
        session: props.scopeId || '',
      })
      control.value = { owner: res?.owner || 'user', sinceLocal: res?.sinceLocal ?? null, reason: res?.reason || '' }
      pushLog(t('workbench.browserCdp.logTakeover'), t('workbench.browserCdp.youOwn'))
      note.value = t('workbench.browserCdp.takeoverNote')
      return
    }
    const res = await rpc.call<ControlState>('browser.control.set', { owner: 'agent', session: props.scopeId || '' })
    control.value = { owner: res?.owner || 'agent', sinceLocal: res?.sinceLocal ?? null, reason: res?.reason || '' }
    pushLog(t('workbench.browserCdp.logHandback'), t('workbench.browserCdp.agentOwns'))
    // 「接着任务」：把接管摘要作为运行中插话送给智能体（回合已结束则如实告知）
    const summary = log.value.slice(0, 8).map(r => `${r.kind} ${r.text}`).join('\n')
    const steered = await steerAgent(summary)
    note.value = steered ? t('workbench.browserCdp.handbackSent') : t('workbench.browserCdp.handbackFailed')
  } catch (err) {
    error.value = err instanceof Error ? err.message : String(err)
  } finally {
    busy.value = false
  }
}

async function steerAgent(summary: string): Promise<boolean> {
  const key = props.scopeId.trim()
  if (!key || !rpc.supportsMethod('sessions.steer.v2')) return false
  try {
    const res = await rpc.call<{ accepted?: boolean }>('sessions.steer.v2', {
      key,
      message: t('workbench.browserCdp.steerMessage', { actions: summary || t('workbench.browserCdp.steerNoActions') }),
      client_request_id: `browser-handback-${Date.now()}`,
    })
    return res?.accepted === true
  } catch {
    return false
  }
}

/** 智能体侧浏览器活动：只认 tool_use_start 里的工具名，不猜。 */
function subscribeAgentActivity(): void {
  try {
    unsubAgent = rpc.on('session.event.tool_use_start', (payload: unknown) => {
      const raw = (payload || {}) as Record<string, unknown>
      const name = String(raw.name ?? raw.tool ?? raw.tool_name ?? '')
      if (!/browser/i.test(name)) return
      agentActive.value = true
      agentSteps.value += 1
      pushLog(t('workbench.browserCdp.logAgent'), name)
      if (agentIdleTimer) clearTimeout(agentIdleTimer)
      // 12s 无新动作视为这一轮结束（工具回路的最小可用判据）
      agentIdleTimer = setTimeout(() => { agentActive.value = false }, 12_000)
    })
  } catch {
    unsubAgent = null
  }
}

watch(() => props.active, (on) => {
  if (on) {
    if (status.value === null) void loadStatus()
    void loadControl()
    void refreshShot()
  }
})

onMounted(async () => {
  await loadStatus()
  void loadControl()
  subscribeAgentActivity()
  // 带初始地址打开的面板（点消息里的链接 / 「+ → 内置浏览器」）直接加载页面，
  // 否则用户会看到一个空框，还得再点一次「打开」。
  if (status.value?.available && !sessionId.value && url.value.trim()) void openPage()
  pollTimer = setInterval(() => {
    if (props.active === false) return
    void loadControl()
    if (agentActive.value) void refreshShot()
  }, 3000)
})

onBeforeUnmount(() => {
  if (pollTimer) clearInterval(pollTimer)
  if (agentIdleTimer) clearTimeout(agentIdleTimer)
  pollTimer = null
  agentIdleTimer = null
  unsubAgent?.()
  unsubAgent = null
  // 面板关闭时把控制权还给智能体：残留的 user 锁会让后续任务永久被拦
  if (userOwns.value) void rpc.call('browser.control.set', { owner: 'agent' }).catch(() => {})
  if (sessionId.value) void rpc.call('browser.close', { sessionId: sessionId.value }).catch(() => {})
})
</script>

<style scoped>
.pd-browser {
  display: flex;
  height: 100%;
  min-height: 0;
  flex-direction: column;
  background: var(--bg-surface);
  color: var(--text);
  font-size: var(--fs-sm);
  transition: box-shadow var(--dur-standard, 200ms) var(--ease-standard, ease);
}

/* 智能体占用时：边框泛荧光（呼吸）。
   这不是装饰——它回答「现在谁在动这个页面」，与工具栏里的控制胶囊同源。 */
.pd-browser.is-agent {
  box-shadow: inset 0 0 0 2px color-mix(in srgb, var(--accent) 70%, transparent),
    0 0 22px -6px color-mix(in srgb, var(--accent) 55%, transparent);
  animation: pdBrowserGlow 2.2s var(--ease-in-out, ease-in-out) infinite;
}

@keyframes pdBrowserGlow {
  0%, 100% { box-shadow: inset 0 0 0 2px color-mix(in srgb, var(--accent) 45%, transparent), 0 0 12px -6px color-mix(in srgb, var(--accent) 40%, transparent); }
  50% { box-shadow: inset 0 0 0 2px color-mix(in srgb, var(--accent) 85%, transparent), 0 0 26px -4px color-mix(in srgb, var(--accent) 65%, transparent); }
}

@media (prefers-reduced-motion: reduce) {
  .pd-browser.is-agent { animation: none; }
}

.pd-browser.is-user {
  box-shadow: inset 0 0 0 2px color-mix(in srgb, var(--ok) 65%, transparent);
}

.pd-browser__bar {
  display: flex;
  flex: 0 0 auto;
  align-items: center;
  gap: 6px;
  padding: 6px 10px;
  border-bottom: 1px solid var(--border);
}

.pd-browser__addr {
  flex: 1 1 auto;
  min-width: 0;
  padding: 4px 8px;
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  background: var(--bg-input, var(--bg-surface));
  color: var(--text);
  font: inherit;
  font-size: var(--fs-xs);
}

.pd-browser__btn {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  padding: 3px 8px;
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  background: transparent;
  color: var(--text-muted);
  cursor: pointer;
  font: inherit;
  font-size: var(--fs-xs);
  white-space: nowrap;
}

.pd-browser__btn:hover:not(:disabled) { background: var(--bg-hover); color: var(--text); }
.pd-browser__btn:disabled { cursor: not-allowed; opacity: 0.5; }
.pd-browser__btn.is-active { border-color: var(--ok); color: var(--ok); }

.pd-browser__status {
  display: flex;
  flex: 0 0 auto;
  flex-wrap: wrap;
  align-items: center;
  gap: 6px;
  padding: 5px 10px;
  border-bottom: 1px solid var(--border);
}

.pd-browser__pill {
  padding: 1px 7px;
  border-radius: var(--radius-pill);
  font-size: 10.5px;
}

.pd-browser__pill--agent { background: color-mix(in srgb, var(--accent) 16%, transparent); color: var(--accent); }
.pd-browser__pill--user { background: color-mix(in srgb, var(--ok) 18%, transparent); color: var(--ok); }
.pd-browser__pill--live { background: color-mix(in srgb, var(--warn) 16%, transparent); color: var(--warn); }
.pd-browser__meta { color: var(--text-dim); font-size: 10.5px; }

.pd-browser__error { margin: 8px 12px; color: var(--danger); font-size: var(--fs-xs); line-height: 1.6; white-space: pre-wrap; }
.pd-browser__note { margin: 8px 12px; color: var(--text-muted); font-size: var(--fs-xs); line-height: 1.6; }
.pd-browser__link {
  padding: 0 2px;
  border: 0;
  background: transparent;
  color: var(--accent);
  cursor: pointer;
  font: inherit;
  text-decoration: underline;
}

.pd-browser__view {
  position: relative;
  flex: 1 1 auto;
  min-height: 0;
  overflow: auto;
  background: var(--bg-elevated, var(--bg-surface));
}

.pd-browser__view.is-locked .pd-browser__shot { cursor: not-allowed; }
.pd-browser__shot { display: block; width: 100%; height: auto; cursor: crosshair; }
.pd-browser__empty { display: grid; height: 100%; place-items: center; padding: 24px; color: var(--text-dim); font-size: var(--fs-xs); }

.pd-browser__overlay {
  position: sticky;
  bottom: 0;
  padding: 3px 10px;
  background: color-mix(in srgb, var(--scrim) 55%, transparent);
  color: var(--text);
  font-size: 10.5px;
}

.pd-browser__log {
  flex: 0 0 auto;
  max-height: 148px;
  overflow: auto;
  border-top: 1px solid var(--border);
  font-size: 10.5px;
}

.pd-browser__log header { padding: 4px 10px; color: var(--text-dim); }
.pd-browser__log ul { margin: 0; padding: 0 10px 6px; list-style: none; }
.pd-browser__log li { display: flex; gap: 8px; padding: 1px 0; }
.pd-browser__log-time { color: var(--text-dim); font-variant-numeric: tabular-nums; }
.pd-browser__log-kind { color: var(--accent); min-width: 52px; }
.pd-browser__log-text { color: var(--text-muted); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.pd-browser__log-empty { color: var(--text-dim); }
</style>
