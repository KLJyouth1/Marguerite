<template>
  <div class="control-stage cockpit-stage">
    <header class="control-stage__header">
      <div class="control-stage__title-block">
        <h1 class="control-stage__title">浏览器驾驶舱</h1>
        <p class="control-stage__subtitle">零依赖 CDP 操控：导航 / JS 控制台 / 截图点击 / 无障碍树 / Resource Timing 网络面板（browser.* RPC）</p>
      </div>
      <div class="control-stage__actions">
        <button class="btn btn--ghost" :disabled="busy" @click="loadStatus">
          <Icon name="refresh" :size="16" /><span>检测</span>
        </button>
        <button v-if="sessionId" class="btn btn--ghost" :disabled="busy" @click="closeSession">
          <Icon name="x" :size="16" /><span>关闭会话</span>
        </button>
      </div>
    </header>

    <section v-if="error" class="cockpit-error">{{ error }}</section>
    <section v-if="status && !status.available" class="cockpit-error">{{ status.reason }}</section>

    <template v-if="status && status.available">
      <section class="cockpit-bar">
        <input v-model="url" class="cockpit-url" placeholder="https://example.com" @keyup.enter="openPage" />
        <button class="btn btn--primary" :disabled="busy || !sessionId" @click="navigate">导航</button>
        <button class="btn btn--ghost" :disabled="busy" @click="openPage">打开</button>
        <span class="cockpit-meta" v-if="title">标题：{{ title }}</span>
      </section>

      <div class="cockpit-layout">
        <section class="cockpit-view">
          <div v-if="!sessionId" class="cockpit-empty">输入 URL 打开页面（真实 Chromium CDP，需本机 Chrome/Edge）</div>
          <div v-else class="cockpit-shot-wrap" @click="onShotClick">
            <img v-if="screenshot" :src="screenshot" class="cockpit-shot" alt="页面截图" />
            <div v-else class="cockpit-empty">点击「刷新截图」查看页面</div>
            <div v-if="screenshot" class="cockpit-shot-overlay">点击截图任意位置 = 真实鼠标点击</div>
          </div>
          <div class="cockpit-actions">
            <button class="btn btn--sm btn--ghost" :disabled="busy || !sessionId" @click="refreshShot">刷新截图</button>
            <button class="btn btn--sm btn--ghost" :disabled="busy || !sessionId" @click="dumpDom">DOM 快照</button>
          </div>
          <pre v-if="domDump" class="cockpit-dom">{{ domDump }}</pre>
        </section>

        <aside class="cockpit-side">
          <section class="cockpit-panel">
            <div class="cockpit-panel__title">JS 控制台（evaluate）</div>
            <textarea v-model="expression" class="cockpit-console" spellcheck="false" placeholder="document.title"></textarea>
            <button class="btn btn--sm btn--primary" :disabled="busy || !sessionId" @click="evaluate">运行</button>
            <pre v-if="consoleOut" class="cockpit-out">{{ consoleOut }}</pre>
          </section>
          <section class="cockpit-panel">
            <div class="cockpit-panel__title">无障碍节点（a11y）</div>
            <button class="btn btn--sm btn--ghost" :disabled="busy || !sessionId" @click="a11y">读取</button>
            <ul class="cockpit-a11y">
              <li v-for="n in a11yNodes" :key="n.id"><code>{{ n.role }}</code> — {{ n.name }}</li>
            </ul>
          </section>
        </aside>
      </div>

      <section class="cockpit-panel cockpit-network">
        <div class="cockpit-panel__title">DevTools 网络面板（Resource Timing · 近似段）</div>
        <div class="cockpit-network__bar">
          <button class="btn btn--sm btn--ghost" :disabled="busy || !sessionId" @click="captureNetwork">捕获</button>
          <button class="btn btn--sm btn--ghost" :disabled="busy || !sessionId" @click="clearNetwork">清空</button>
          <span class="cockpit-network__meta">{{ networkRows.length }} 条资源（近 100 条 · 按加载倒序）</span>
        </div>
        <table v-if="networkRows.length" class="cockpit-network__table">
          <thead><tr><th>资源</th><th>类型</th><th>耗时</th><th>大小</th></tr></thead>
          <tbody>
            <tr v-for="(n, i) in networkRows" :key="i">
              <td><code :title="n.name">{{ truncate(n.name) }}</code></td>
              <td>{{ n.type }}</td>
              <td>{{ n.duration }} ms</td>
              <td>{{ fmtSize(n.size) }}</td>
            </tr>
          </tbody>
        </table>
        <div v-else class="cockpit-network__empty">暂无捕获（打开页面后点「捕获」；数据来自浏览器 Resource Timing API，不含请求头/响应体）</div>
      </section>

      <section class="cockpit-panel cockpit-iframe">
        <div class="cockpit-panel__title">内置浏览器（iframe 直载 · 轻量预览）</div>
        <div class="cockpit-bar">
          <input v-model="iframeUrl" class="cockpit-url" placeholder="https://example.com" @keyup.enter="goIframe" />
          <button class="btn btn--primary" :disabled="busy || !iframeUrl.trim()" @click="goIframe">载入</button>
        </div>
        <div class="cockpit-iframe-wrap">
          <iframe v-if="iframeActive" :src="iframeActive" class="cockpit-iframe" title="内置浏览器" sandbox="allow-scripts allow-same-origin allow-forms allow-popups" />
          <div v-else class="cockpit-empty">输入 URL 载入。跨域站点若拒绝被嵌套（X-Frame-Options / CSP），iframe 会留白 —— 请改用上方 CDP 驾驶舱（真实 Chromium）。</div>
        </div>
        <p class="cockpit-iframe__note">iframe 预览不携带你的会话凭据；登录型站点请走 CDP 会话。若站点拒绝嵌套，属于站点自身安全策略，非本工具缺陷。</p>
      </section>
    </template>
  </div>
</template>

<script setup lang="ts">
import { ref, onMounted } from 'vue'
import Icon from '@/components/Icon.vue'
import { useRpcStore } from '@/stores/rpc'

const rpc = useRpcStore()

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
    consoleOut.value = r?.result ?? '(空)'
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
.cockpit-dom { margin-top: 10px; max-height: 260px; overflow: auto; padding: 10px; background: var(--bg); border: 1px solid var(--hairline); border-radius: var(--radius-sm); font-family: var(--font-mono); font-size: 11px; white-space: pre-wrap; }
.cockpit-side { display: flex; flex-direction: column; gap: 14px; }
.cockpit-panel { background: var(--card); border: 1px solid var(--hairline); border-radius: var(--radius-md); padding: 12px; }
.cockpit-panel__title { font-size: 12px; font-weight: 600; margin-bottom: 8px; }
.cockpit-console { width: 100%; min-height: 90px; padding: 8px; border: 1px solid var(--hairline); border-radius: var(--radius-sm); background: var(--bg); color: var(--text); font-family: var(--font-mono); font-size: 12px; }
.cockpit-out { margin-top: 8px; max-height: 220px; overflow: auto; padding: 8px; background: var(--bg); border: 1px solid var(--hairline); border-radius: var(--radius-sm); font-family: var(--font-mono); font-size: 11px; white-space: pre-wrap; }
.cockpit-a11y { margin: 8px 0 0; padding: 0; list-style: none; font-size: 12px; max-height: 240px; overflow: auto; }
.cockpit-a11y li { padding: 5px 0; border-bottom: 1px solid var(--hairline); }
.cockpit-a11y code { color: var(--accent); }
.cockpit-empty { padding: 20px; text-align: center; color: var(--text-dim); font-size: 13px; }
.cockpit-iframe { margin-top: 14px; }
.cockpit-iframe-wrap { margin-top: 10px; min-height: 320px; border: 1px solid var(--hairline); border-radius: var(--radius-sm); overflow: hidden; background: var(--bg); }
.cockpit-iframe { width: 100%; height: 480px; border: 0; display: block; }
.cockpit-iframe__note { margin: 8px 0 0; font-size: 11px; color: var(--text-dim); }
.cockpit-network { margin-top: 14px; }
.cockpit-network__bar { display: flex; align-items: center; gap: 8px; margin-bottom: 8px; }
.cockpit-network__meta { font-size: 11px; color: var(--text-dim); }
.cockpit-network__table { width: 100%; border-collapse: collapse; font-size: 12px; }
.cockpit-network__table th, .cockpit-network__table td { text-align: left; padding: 6px 8px; border-bottom: 1px solid var(--hairline); }
.cockpit-network__table th { color: var(--text-dim); font-weight: 600; }
.cockpit-network__table code { font-family: var(--font-mono); font-size: 11px; color: var(--accent); }
.cockpit-network__empty { padding: 14px; font-size: 12px; color: var(--text-dim); }
</style>
