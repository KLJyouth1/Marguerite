<template>
  <div class="control-stage cockpit-stage" :class="{ 'cockpit-stage--focus': focusMode }">
    <header class="control-stage__header">
      <div class="control-stage__title-block">
        <h1 class="control-stage__title">{{ t('cockpit.title') }}</h1>
        <p class="control-stage__subtitle">{{ t('cockpit.subtitle') }}</p>
      </div>
      <div class="control-stage__actions">
        <!-- 与 TerminalView 同一款专注模式：工作面打开后，标题头是死垂直空间 -->
        <button
          class="btn btn--ghost"
          :aria-pressed="focusMode"
          :title="focusMode ? t('chrome.focusModeOff') : t('chrome.focusMode')"
          @click="focusMode = !focusMode"
        >
          <Icon :name="focusMode ? 'collapse' : 'expand'" :size="16" />
          <span>{{ focusMode ? t('chrome.focusModeOff') : t('chrome.focusMode') }}</span>
        </button>
        <button class="btn btn--ghost" :disabled="busy" @click="loadStatus">
          <Icon name="refresh" :size="16" /><span>{{ t('cockpit.detect') }}</span>
        </button>
        <button v-if="sessionId" class="btn btn--ghost" :disabled="busy" @click="closeSession">
          <Icon name="x" :size="16" /><span>{{ t('cockpit.closeSession') }}</span>
        </button>
      </div>
    </header>

    <section v-if="error" class="cockpit-error">{{ error }}</section>
    <section v-if="status && !status.available" class="cockpit-error">{{ status.reason }}</section>

    <template v-if="status && status.available">
      <section class="cockpit-bar">
        <input v-model="url" class="cockpit-url" placeholder="https://example.com" @keyup.enter="openPage" />
        <button class="btn btn--primary" :disabled="busy || !sessionId" @click="navigate">{{ t('cockpit.navigate') }}</button>
        <button class="btn btn--ghost" :disabled="busy" @click="openPage">{{ t('cockpit.open') }}</button>
        <span class="cockpit-meta" v-if="title">{{ t('cockpit.pageTitle', { title }) }}</span>
      </section>

      <div class="cockpit-layout">
        <section class="cockpit-view">
          <div v-if="!sessionId" class="cockpit-empty">{{ t('cockpit.emptyCdp') }}</div>
          <div v-else class="cockpit-shot-wrap" @click="onShotClick">
            <img v-if="screenshot" :src="screenshot" class="cockpit-shot" alt="screenshot" />
            <div v-else class="cockpit-empty">{{ t('cockpit.emptyShot') }}</div>
            <div v-if="screenshot" class="cockpit-shot-overlay">{{ t('cockpit.shotOverlay') }}</div>
          </div>
          <div class="cockpit-actions">
            <button class="btn btn--sm btn--ghost" :disabled="busy || !sessionId" @click="refreshShot">{{ t('cockpit.refreshShot') }}</button>
            <button class="btn btn--sm btn--ghost" :disabled="busy || !sessionId" @click="dumpDom">{{ t('cockpit.dumpDom') }}</button>
          </div>
          <pre v-if="domDump" class="cockpit-dom">{{ domDump }}</pre>
        </section>

        <aside class="cockpit-side">
          <section class="cockpit-panel">
            <div class="cockpit-panel__title">{{ t('cockpit.consoleTitle') }}</div>
            <textarea v-model="expression" class="cockpit-console" spellcheck="false" placeholder="document.title"></textarea>
            <button class="btn btn--sm btn--primary" :disabled="busy || !sessionId" @click="evaluate">{{ t('cockpit.run') }}</button>
            <pre v-if="consoleOut" class="cockpit-out">{{ consoleOut || t('cockpit.consoleEmpty') }}</pre>
          </section>
          <section class="cockpit-panel">
            <div class="cockpit-panel__title">{{ t('cockpit.a11yTitle') }}</div>
            <button class="btn btn--sm btn--ghost" :disabled="busy || !sessionId" @click="a11y">{{ t('cockpit.a11yRead') }}</button>
            <ul class="cockpit-a11y">
              <li v-for="n in a11yNodes" :key="n.id"><code>{{ n.role }}</code> — {{ n.name }}</li>
            </ul>
          </section>
        </aside>
      </div>

      <section class="cockpit-panel cockpit-network">
        <div class="cockpit-panel__title">{{ t('cockpit.networkTitle') }}</div>
        <div class="cockpit-network__bar">
          <button class="btn btn--sm btn--ghost" :disabled="busy || !sessionId" @click="captureNetwork">{{ t('cockpit.capture') }}</button>
          <button class="btn btn--sm btn--ghost" :disabled="busy || !sessionId" @click="clearNetwork">{{ t('cockpit.clear') }}</button>
          <span class="cockpit-network__meta">{{ t('cockpit.networkMeta', { n: networkRows.length }) }}</span>
        </div>
        <table v-if="networkRows.length" class="cockpit-network__table">
          <thead><tr><th>{{ t('cockpit.thResource') }}</th><th>{{ t('cockpit.thType') }}</th><th>{{ t('cockpit.thDuration') }}</th><th>{{ t('cockpit.thSize') }}</th></tr></thead>
          <tbody>
            <tr v-for="(n, i) in networkRows" :key="i">
              <td><code :title="n.name">{{ truncate(n.name) }}</code></td>
              <td>{{ n.type }}</td>
              <td>{{ n.duration }} ms</td>
              <td>{{ fmtSize(n.size) }}</td>
            </tr>
          </tbody>
        </table>
        <div v-else class="cockpit-network__empty">{{ t('cockpit.networkEmpty') }}</div>
      </section>

      <section class="cockpit-panel cockpit-iframe">
        <div class="cockpit-panel__title">{{ t('cockpit.iframeTitle') }}</div>
        <div class="cockpit-bar">
          <input v-model="iframeUrl" class="cockpit-url" placeholder="https://example.com" @keyup.enter="goIframe" />
          <button class="btn btn--primary" :disabled="busy || !iframeUrl.trim()" @click="goIframe">{{ t('cockpit.load') }}</button>
        </div>
        <div class="cockpit-iframe-wrap">
          <iframe v-if="iframeActive" :src="iframeActive" class="cockpit-iframe" title="iframe" sandbox="allow-scripts allow-same-origin allow-forms allow-popups" />
          <div v-else class="cockpit-empty">{{ t('cockpit.iframeEmpty') }}</div>
        </div>
        <p class="cockpit-iframe__note">{{ t('cockpit.iframeNote') }}</p>
      </section>
    </template>
  </div>
</template>

<script setup lang="ts">
import { ref, onMounted } from 'vue'
import { useI18n } from 'vue-i18n'
import Icon from '@/components/Icon.vue'
import { useRpcStore } from '@/stores/rpc'

const rpc = useRpcStore()
const { t } = useI18n()
/** 专注模式：隐藏标题头，给截图/面板让出垂直空间（视图内瞬态，不持久化）。 */
const focusMode = ref(false)

interface Status { available: boolean; reason?: string }
interface OpenResult { sessionId: string; title: string; url: string }
interface EvalResult { result?: string }
interface ShotResult { dataUrl: string }
interface A11yResult { nodes: Array<{ id: string; role: string; name: string }> }

const status = ref<Status | null>(null)
const url = ref('https://example.com')
const sessionId = ref('')
const title = ref('')
const screenshot = ref('')
const expression = ref('document.title')
const consoleOut = ref('')
const domDump = ref('')
const a11yNodes = ref<Array<{ id: string; role: string; name: string }>>([])
interface NetworkEntry { name: string; duration: number; size: number; type: string }
const networkRows = ref<NetworkEntry[]>([])
const busy = ref(false)
const error = ref('')

async function safe<T>(fn: () => Promise<T>): Promise<T | null> {
  try { return await fn() } catch (e) { error.value = e instanceof Error ? e.message : String(e); return null }
}

async function loadStatus() {
  // 冷加载竞态守卫：WS 仍处于 connecting 时先等连接就绪。否则 `safe()` 会把
  // `not connected (state: connecting)` 当成加载失败，直接渲染成红错误条。
  if (!rpc.isConnected) await safe(() => rpc.waitForConnection(10_000))
  const s = await safe(() => rpc.call<Status>('browser.status', {}))
  if (s) status.value = s
}

async function openPage() {
  busy.value = true; error.value = ''
  try {
    const r = await rpc.call<OpenResult>('browser.open', { url: url.value || 'about:blank' })
    sessionId.value = r.sessionId
    title.value = r.title || ''
    url.value = r.url || url.value
    await refreshShot()
  } finally { busy.value = false }
}

async function navigate() {
  busy.value = true; error.value = ''
  try {
    const r = await rpc.call<OpenResult>('browser.navigate', { sessionId: sessionId.value, url: url.value })
    title.value = r.title || ''
    url.value = r.url || url.value
    await refreshShot()
  } finally { busy.value = false }
}

async function refreshShot() {
  busy.value = true; error.value = ''
  try {
    const r = await safe(() => rpc.call<ShotResult>('browser.screenshot', { sessionId: sessionId.value }))
    if (r) screenshot.value = r.dataUrl || ''
  } finally { busy.value = false }
}

async function evaluate() {
  busy.value = true; error.value = ''
  try {
    const r = await safe(() => rpc.call<EvalResult>('browser.evaluate', { sessionId: sessionId.value, expression: expression.value, awaitPromise: true }))
    consoleOut.value = r?.result ?? ''
  } finally { busy.value = false }
}

async function dumpDom() {
  busy.value = true; error.value = ''
  try {
    const r = await safe(() => rpc.call<EvalResult>('browser.evaluate', { sessionId: sessionId.value, expression: 'document.documentElement.outerHTML.slice(0, 6000)', awaitPromise: false }))
    domDump.value = String(r?.result ?? '').slice(0, 6000)
  } finally { busy.value = false }
}

async function a11y() {
  busy.value = true; error.value = ''
  try {
    const r = await safe(() => rpc.call<A11yResult>('browser.a11y', { sessionId: sessionId.value, max: 60 }))
    a11yNodes.value = r?.nodes ?? []
  } finally { busy.value = false }
}

async function onShotClick(e: MouseEvent) {
  if (!screenshot.value) return
  const img = e.target as HTMLImageElement
  const rect = img.getBoundingClientRect()
  const scale = img.naturalWidth > 0 ? img.naturalWidth / rect.width : 1
  const x = (e.clientX - rect.left) * scale
  const y = (e.clientY - rect.top) * scale
  busy.value = true; error.value = ''
  try {
    await rpc.call('browser.click', { sessionId: sessionId.value, x, y })
    await refreshShot()
  } catch (err) { error.value = err instanceof Error ? err.message : String(err) } finally { busy.value = false }
}

async function closeSession() {
  busy.value = true
  await safe(() => rpc.call('browser.close', { sessionId: sessionId.value }))
  sessionId.value = ''; screenshot.value = ''; consoleOut.value = ''; domDump.value = ''; a11yNodes.value = []
  busy.value = false
}

const NETWORK_EXPR = 'JSON.stringify(performance.getEntriesByType("resource").slice(-100).map(function(e){return {name:e.name,duration:Math.round(e.duration),size:e.transferSize||e.encodedBodySize||0,type:e.initiatorType}}))'

function truncate(s: string): string {
  return s.length > 72 ? s.slice(0, 72) + "..." : s
}

function fmtSize(n: number): string {
  if (n <= 0) return "—"
  if (n < 1024) return n + " B"
  if (n < 1048576) return (n / 1024).toFixed(1) + " KB"
  return (n / 1048576).toFixed(2) + " MB"
}

async function captureNetwork(): Promise<void> {
  busy.value = true
  error.value = ''
  try {
    const r = await safe(() => rpc.call<EvalResult>('browser.evaluate', { sessionId: sessionId.value, expression: NETWORK_EXPR, awaitPromise: false }))
    if (r) {
      const raw = String(r.result ?? '')
      try { networkRows.value = JSON.parse(raw) as NetworkEntry[] } catch { networkRows.value = [] }
    }
  } finally { busy.value = false }
}

async function clearNetwork(): Promise<void> {
  try {
    await rpc.call('browser.evaluate', { sessionId: sessionId.value, expression: 'performance.clearResourceTimings()', awaitPromise: false })
    networkRows.value = []
  } catch { /* 忽略 */ }
}

// 内置浏览器（iframe 直载）
const iframeUrl = ref('https://example.com')
const iframeActive = ref('')
function goIframe() {
  iframeActive.value = iframeUrl.value.trim()
}

onMounted(loadStatus)
</script>

<style scoped>
.cockpit-stage { padding: 24px 28px; max-width: 1340px; margin: 0 auto; }
.cockpit-stage--focus .control-stage__header { display: none; }
.cockpit-error { margin: 12px 0; padding: 10px 14px; background: color-mix(in srgb, var(--danger) 10%, var(--card)); border: 1px solid color-mix(in srgb, var(--danger) 30%, transparent); border-radius: var(--radius-sm); color: var(--danger); font-size: 12px; }
.cockpit-bar { display: flex; gap: 8px; align-items: center; margin-bottom: 14px; }
.cockpit-url { flex: 1; min-width: 0; padding: 9px 12px; border: 1px solid var(--hairline); border-radius: var(--radius-sm); background: var(--bg); color: var(--text); font-size: 13px; }
.cockpit-meta { font-size: 12px; color: var(--text-dim); white-space: nowrap; }
.cockpit-layout { display: grid; grid-template-columns: minmax(0, 1.6fr) minmax(280px, 1fr); gap: 14px; }
.cockpit-view { background: var(--card); border: 1px solid var(--hairline); border-radius: var(--radius-md); padding: 12px; }
.cockpit-shot-wrap { position: relative; margin-top: 10px; }
.cockpit-shot { width: 100%; border-radius: var(--radius-sm); border: 1px solid var(--hairline); cursor: crosshair; }
.cockpit-shot-overlay { position: absolute; left: 10px; bottom: 10px; padding: 4px 8px; background: color-mix(in srgb, var(--bg) 70%, transparent); border-radius: 999px; font-size: 11px; color: var(--text-dim); pointer-events: none; }
.cockpit-actions { display: flex; gap: 8px; margin-top: 10px; }
/* 固定 260px 的 DOM 快照是拥挤感的主源之一：改随视口生长并夹住上下界 */
.cockpit-dom { margin-top: 10px; height: clamp(200px, 42vh, 560px); overflow: auto; padding: 10px; background: var(--bg); border: 1px solid var(--hairline); border-radius: var(--radius-sm); font-family: var(--font-mono); font-size: 11px; white-space: pre-wrap; }
.cockpit-side { display: flex; flex-direction: column; gap: 14px; }
.cockpit-panel { background: var(--card); border: 1px solid var(--hairline); border-radius: var(--radius-md); padding: 12px; }
.cockpit-panel__title { font-size: 12px; font-weight: 600; margin-bottom: 8px; }
.cockpit-console { width: 100%; min-height: clamp(90px, 15vh, 200px); padding: 8px; border: 1px solid var(--hairline); border-radius: var(--radius-sm); background: var(--bg); color: var(--text); font-family: var(--font-mono); font-size: 12px; }
.cockpit-out { margin-top: 8px; max-height: clamp(220px, 36vh, 520px); overflow: auto; padding: 8px; background: var(--bg); border: 1px solid var(--hairline); border-radius: var(--radius-sm); font-family: var(--font-mono); font-size: 11px; white-space: pre-wrap; }
.cockpit-a11y { margin: 8px 0 0; padding: 0; list-style: none; font-size: 12px; max-height: clamp(240px, 42vh, 560px); overflow: auto; }
.cockpit-a11y li { padding: 5px 0; border-bottom: 1px solid var(--hairline); }
.cockpit-a11y code { color: var(--accent); }
.cockpit-empty { padding: 20px; text-align: center; color: var(--text-dim); font-size: 13px; }
.cockpit-network { margin-top: 14px; }
.cockpit-iframe-wrap { margin-top: 10px; border: 1px solid var(--hairline); border-radius: var(--radius-sm); overflow: hidden; background: var(--bg); }
.cockpit-iframe { width: 100%; height: clamp(420px, 64vh, 960px); border: 0; display: block; }
.cockpit-iframe__note { margin: 8px 0 0; font-size: 11px; color: var(--text-dim); }
.cockpit-network__bar { display: flex; align-items: center; gap: 8px; margin-bottom: 8px; }
.cockpit-network__meta { font-size: 11px; color: var(--text-dim); }
.cockpit-network__table { width: 100%; border-collapse: collapse; font-size: 12px; }
.cockpit-network__table th, .cockpit-network__table td { text-align: left; padding: 6px 8px; border-bottom: 1px solid var(--hairline); }
.cockpit-network__table th { color: var(--text-dim); font-weight: 600; }
.cockpit-network__table code { font-family: var(--font-mono); font-size: 11px; color: var(--accent); }
.cockpit-network__empty { padding: 14px; font-size: 12px; color: var(--text-dim); }
</style>
