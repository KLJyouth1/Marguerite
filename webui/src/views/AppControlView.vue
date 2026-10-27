<template>
  <div class="control-stage ac-stage">
    <header class="control-stage__header">
      <div class="control-stage__title-block">
        <h1 class="control-stage__title">应用自控</h1>
        <p class="control-stage__subtitle">
          语义化驱动本应用界面：枚举可交互元素 → 点击 / 填写 / 按键 / 读取；一键巡检全部路由。
          与「电脑操控」（像素级键鼠）和「浏览器驾驶舱」（外部 CDP）互补，这一层直接操作 DOM，稳定可断言。
        </p>
      </div>
      <div class="control-stage__actions">
        <button class="btn btn--ghost" type="button" data-testid="ac-refresh" @click="refresh">
          <Icon name="refresh" :size="16" /><span>刷新元素清单</span>
        </button>
        <button class="btn btn--ghost" type="button" data-testid="ac-copy" @click="copyInventory">
          <Icon name="copy" :size="16" /><span>复制清单 JSON</span>
        </button>
      </div>
    </header>

    <section class="ac-meta">
      <span class="ac-pill">当前路由 <code>{{ route.path }}</code></span>
      <span class="ac-pill">可交互元素 <strong>{{ elements.length }}</strong></span>
      <span class="ac-pill">外部入口 <code>window.MargueriteAppControl</code></span>
    </section>

    <div class="ac-grid">
      <section class="ac-panel">
        <div class="ac-panel__head">
          <h2 class="ac-panel__title">元素清单</h2>
          <input
            v-model="filter"
            class="ac-input"
            type="search"
            placeholder="按名称 / 角色 / testid 过滤"
            data-testid="ac-filter"
          />
        </div>
        <div class="ac-table-wrap">
          <table class="ac-table" data-testid="ac-elements">
            <thead>
              <tr><th>角色</th><th>名称</th><th>状态</th><th></th></tr>
            </thead>
            <tbody>
              <tr
                v-for="el in visibleElements"
                :key="el.selector"
                :class="{ 'is-selected': el.selector === selected }"
              >
                <td><code>{{ el.role }}</code></td>
                <td class="ac-name" :title="el.selector">{{ el.name || '（无名称）' }}</td>
                <td>
                  <span v-if="el.disabled" class="ac-badge ac-badge--warn">禁用</span>
                  <span v-else class="ac-badge">可点</span>
                </td>
                <td class="ac-row-actions">
                  <button class="btn btn--sm btn--ghost" type="button" @click="select(el)">详情</button>
                  <button class="btn btn--sm btn--ghost" type="button" @click="doClick(el)">点击</button>
                </td>
              </tr>
              <tr v-if="!visibleElements.length">
                <td colspan="4" class="ac-empty">没有匹配的可交互元素（页面可能还在加载，点右上角刷新）</td>
              </tr>
            </tbody>
          </table>
        </div>
      </section>

      <section class="ac-panel">
        <h2 class="ac-panel__title">选中元素</h2>
        <div v-if="selectedElement" class="ac-detail">
          <div class="ac-detail__row"><span>角色</span><code>{{ selectedElement.role }}</code></div>
          <div class="ac-detail__row"><span>名称</span><span>{{ selectedElement.name || '（无名称）' }}</span></div>
          <div class="ac-detail__row"><span>选择器</span><code class="ac-ellipsis">{{ selectedElement.selector }}</code></div>
          <div class="ac-detail__row"><span>位置</span><span>{{ selectedElement.rect.x }}, {{ selectedElement.rect.y }} · {{ selectedElement.rect.width }}×{{ selectedElement.rect.height }}</span></div>
          <div class="ac-detail__row" v-if="selectedElement.value !== undefined">
            <span>当前值</span><span class="ac-ellipsis">{{ selectedElement.value }}</span>
          </div>
          <div class="ac-detail__actions">
            <button class="btn btn--sm btn--ghost" type="button" data-testid="ac-click" @click="doClick(selectedElement)">点击</button>
            <button class="btn btn--sm btn--ghost" type="button" data-testid="ac-read" @click="doRead(selectedElement)">读取</button>
          </div>
          <div class="ac-fill">
            <input v-model="fillValue" class="ac-input" type="text" placeholder="要填入的值" data-testid="ac-fill-value" />
            <button class="btn btn--sm btn--primary" type="button" data-testid="ac-fill" @click="doFill">填入</button>
          </div>
        </div>
        <p v-else class="ac-empty">在左侧清单里选一个元素，即可点击 / 读取 / 填入。</p>

        <h2 class="ac-panel__title ac-panel__title--spaced">按键</h2>
        <div class="ac-fill">
          <input v-model="keyValue" class="ac-input" type="text" placeholder="如 Enter / Escape / k" data-testid="ac-key" />
          <label class="ac-check"><input v-model="keyCtrl" type="checkbox" /> Ctrl</label>
          <button class="btn btn--sm btn--ghost" type="button" data-testid="ac-press" @click="doPress">发送</button>
        </div>
      </section>
    </div>

    <section class="ac-panel ac-scan">
      <div class="ac-panel__head">
        <h2 class="ac-panel__title">全路由巡检</h2>
        <div class="ac-scan__actions">
          <span class="ac-pill">路由 <strong>{{ routes.length }}</strong></span>
          <button
            class="btn btn--primary"
            type="button"
            data-testid="ac-scan-start"
            :disabled="scan.running"
            @click="startScan"
          >
            <Icon :name="scan.running ? 'pause' : 'play'" :size="16" />
            <span>{{ scan.running ? `巡检中 ${scan.entries.length}/${scan.total}` : '开始巡检' }}</span>
          </button>
        </div>
      </div>
      <p class="ac-hint">
        判据：真实导航 → 等待渲染 → 采集「可交互元素数 / role=alert 文案 / 未捕获错误 / console.error」。
        有 alert 或错误即判失败。巡检会短暂切页，结束后自动回到本页。
      </p>
      <div v-if="scan.entries.length" class="ac-scan__summary">
        <span class="ac-pill">通过 <strong>{{ passedCount }}</strong></span>
        <span class="ac-pill">失败 <strong>{{ scan.entries.length - passedCount }}</strong></span>
        <span class="ac-pill">耗时 <strong>{{ scanDuration }}</strong></span>
        <button class="btn btn--sm btn--ghost" type="button" data-testid="ac-scan-copy" @click="copyScan">
          <Icon name="copy" :size="14" /><span>复制结果 JSON</span>
        </button>
      </div>
      <!-- 失败路由置顶：巡检会跑几十个路由，失败行埋在长表里等于没报告 -->
      <div v-if="failedEntries.length" class="ac-failures" data-testid="ac-scan-failures">
        <div class="ac-failures__title">失败路由（{{ failedEntries.length }}）</div>
        <div v-for="entry in failedEntries" :key="entry.path" class="ac-failures__row">
          <code>{{ entry.path }}</code>
          <span>{{ [...entry.alerts, ...entry.errors][0] || '未知原因' }}</span>
        </div>
      </div>
      <div v-if="scan.entries.length" class="ac-table-wrap ac-table-wrap--full">
        <table class="ac-table" data-testid="ac-scan-results">
          <thead>
            <tr><th>路由</th><th>结果</th><th>元素</th><th>告警 / 错误</th><th>耗时</th></tr>
          </thead>
          <tbody>
            <tr v-for="entry in scan.entries" :key="entry.path">
              <td><code>{{ entry.path }}</code></td>
              <td>
                <span class="ac-badge" :class="entry.ok ? 'ac-badge--ok' : 'ac-badge--danger'">
                  {{ entry.ok ? '通过' : '失败' }}
                </span>
              </td>
              <td>{{ entry.elements }}</td>
              <td class="ac-name">
                <span v-if="!entry.alerts.length && !entry.errors.length">—</span>
                <span v-for="(msg, i) in [...entry.alerts, ...entry.errors]" :key="i" class="ac-issue">{{ msg }}</span>
              </td>
              <td>{{ entry.ms }} ms</td>
            </tr>
          </tbody>
        </table>
      </div>
      <p v-else class="ac-empty">还没有巡检结果。点「开始巡检」逐页走一遍。</p>
    </section>

    <section class="ac-panel">
      <div class="ac-panel__head">
        <h2 class="ac-panel__title">操作日志</h2>
        <button class="btn btn--sm btn--ghost" type="button" @click="actionLog.splice(0)">清空</button>
      </div>
      <pre class="ac-log" data-testid="ac-log">{{ actionLog.length ? actionLog.join('\n') : '（暂无操作）' }}</pre>
    </section>
  </div>
</template>

<script setup lang="ts">
import { computed, onMounted, ref, watch } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import Icon from '@/components/Icon.vue'
import {
  actionLog,
  clickTarget,
  fillTarget,
  listInteractiveElements,
  logAction,
  pressKey,
  readTarget,
  scanRoutes,
  scanState as scan,
  scannableRoutes,
  type AppControlElement,
} from '@/utils/appControl'

const route = useRoute()
const router = useRouter()

const elements = ref<AppControlElement[]>([])
const filter = ref('')
const selected = ref('')
const fillValue = ref('')
const keyValue = ref('Enter')
const keyCtrl = ref(false)

const routes = computed(() => scannableRoutes(router))

const visibleElements = computed(() => {
  const needle = filter.value.trim().toLowerCase()
  if (!needle) return elements.value
  return elements.value.filter(el => (
    el.name.toLowerCase().includes(needle)
    || el.role.toLowerCase().includes(needle)
    || (el.testid || '').toLowerCase().includes(needle)
  ))
})

const selectedElement = computed(
  () => elements.value.find(el => el.selector === selected.value) || null,
)

const passedCount = computed(() => scan.entries.filter(entry => entry.ok).length)
const failedEntries = computed(() => scan.entries.filter(entry => !entry.ok))
const scanDuration = computed(() => {
  if (!scan.startedAt) return '—'
  const end = scan.finishedAt ?? Date.now()
  return `${((end - scan.startedAt) / 1000).toFixed(1)}s`
})

function log(message: string) {
  logAction(message)
}

function refresh() {
  elements.value = listInteractiveElements()
  if (selected.value && !elements.value.some(el => el.selector === selected.value)) {
    selected.value = ''
  }
}

function select(el: AppControlElement) {
  selected.value = el.selector
  fillValue.value = el.value ?? ''
}

function doClick(el: AppControlElement) {
  const result = clickTarget(el.selector)
  log(result.ok ? `点击成功：${result.name || el.selector}` : `点击失败：${result.error}`)
  window.setTimeout(refresh, 180)
}

function doRead(el: AppControlElement) {
  const result = readTarget(el.selector)
  log(result.ok ? `读取 ${result.name || el.selector} = ${JSON.stringify(result.value ?? '')}` : `读取失败：${result.error}`)
}

function doFill() {
  if (!selectedElement.value) return
  const result = fillTarget(selectedElement.value.selector, fillValue.value)
  log(result.ok
    ? `填入成功：${selectedElement.value.name || selectedElement.value.selector} = ${JSON.stringify(result.value ?? '')}`
    : `填入失败：${result.error}`)
  window.setTimeout(refresh, 120)
}

function doPress() {
  const result = pressKey(keyValue.value, keyCtrl.value ? ['ctrl'] : [])
  log(result.ok ? `按键已发送：${keyCtrl.value ? 'Ctrl+' : ''}${keyValue.value}` : `按键失败：${result.error}`)
}

async function copyInventory() {
  const payload = JSON.stringify({ route: route.path, elements: elements.value }, null, 2)
  try {
    await navigator.clipboard.writeText(payload)
    log(`已复制 ${elements.value.length} 个元素到剪贴板`)
  } catch (error) {
    log(`复制失败：${error instanceof Error ? error.message : String(error)}`)
  }
}

async function copyScan() {
  const payload = JSON.stringify({
    scannedAt: scan.finishedAt ? new Date(scan.finishedAt).toISOString() : null,
    total: scan.entries.length,
    passed: passedCount.value,
    failed: failedEntries.value.map(entry => ({
      path: entry.path,
      alerts: entry.alerts,
      errors: entry.errors,
    })),
    entries: scan.entries,
  }, null, 2)
  try {
    await navigator.clipboard.writeText(payload)
    log(`已复制巡检结果（${scan.entries.length} 条）到剪贴板`)
  } catch (error) {
    log(`复制失败：${error instanceof Error ? error.message : String(error)}`)
  }
}

async function startScan() {
  if (scan.running) return
  const entries = await scanRoutes(routes.value, path => router.push(path))
  const failed = entries.filter(entry => !entry.ok)
  // 巡检会停在最后一个路由，回到本页展示结果。
  await router.push('/app-control')
  refresh()
  if (failed.length) log(`建议先看「失败路由」列表：${failed.map(entry => entry.path).join('、')}`)
}

onMounted(refresh)
watch(() => route.fullPath, () => window.setTimeout(refresh, 260))
</script>

<style scoped>
.ac-stage {
  display: flex;
  flex-direction: column;
  gap: var(--sp-4);
  max-width: 1280px;
  margin: 0 auto;
  padding: 24px 28px;
}
.ac-meta {
  display: flex;
  flex-wrap: wrap;
  gap: var(--sp-2);
}
.ac-pill {
  padding: 4px 10px;
  border: 1px solid var(--hairline);
  border-radius: var(--radius-pill, 999px);
  font-size: var(--fs-xs);
  color: var(--text-dim);
}
.ac-pill code {
  font-size: var(--fs-xs);
  color: var(--text);
}
.ac-grid {
  display: grid;
  grid-template-columns: minmax(0, 1.35fr) minmax(0, 1fr);
  gap: var(--sp-3);
}
.ac-panel {
  background: var(--card);
  border: 1px solid var(--hairline);
  border-radius: var(--radius-md);
  padding: 16px;
  min-width: 0;
}
.ac-panel__head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--sp-2);
  margin-bottom: 10px;
}
.ac-panel__title {
  margin: 0;
  font-size: 13px;
  font-weight: 600;
  color: var(--text-dim);
}
.ac-panel__title--spaced {
  margin-top: 18px;
}
.ac-input {
  flex: 1;
  min-width: 0;
  padding: 7px 10px;
  border: 1px solid var(--hairline);
  border-radius: var(--radius-sm);
  background: var(--bg);
  color: var(--text);
  font-size: 12px;
}
.ac-input:focus {
  border-color: var(--accent);
  outline: none;
}
.ac-table-wrap {
  max-height: 420px;
  overflow: auto;
}
/* 巡检结果不内部滚动：失败行必须能被页面滚动到（表内滚动 + 长表 = 看不见的失败） */
.ac-table-wrap--full {
  max-height: none;
  overflow: visible;
}
.ac-failures {
  margin: 0 0 10px;
  padding: 10px 12px;
  border: 1px solid var(--danger);
  border-radius: var(--radius-sm);
  background: color-mix(in srgb, var(--danger) 8%, transparent);
}
.ac-failures__title {
  font-size: 12px;
  font-weight: 600;
  color: var(--danger);
  margin-bottom: 6px;
}
.ac-failures__row {
  display: flex;
  gap: 10px;
  font-size: 12px;
  padding: 2px 0;
}
.ac-failures__row code {
  flex: none;
  color: var(--text);
}
.ac-table {
  width: 100%;
  border-collapse: collapse;
  font-size: 12px;
}
.ac-table th {
  position: sticky;
  top: 0;
  background: var(--card);
  color: var(--text-dim);
  font-weight: 600;
  text-align: left;
  padding: 6px 8px;
  border-bottom: 1px solid var(--hairline);
}
.ac-table td {
  padding: 5px 8px;
  border-bottom: 1px solid var(--hairline);
  vertical-align: top;
}
.ac-table tr.is-selected {
  background: color-mix(in srgb, var(--accent) 12%, transparent);
}
.ac-name {
  max-width: 320px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.ac-row-actions {
  display: flex;
  gap: 6px;
  white-space: nowrap;
}
.ac-badge {
  display: inline-block;
  padding: 1px 7px;
  border-radius: var(--radius-pill, 999px);
  border: 1px solid var(--hairline);
  font-size: 11px;
  color: var(--text-dim);
}
.ac-badge--ok {
  color: var(--ok);
  border-color: currentColor;
}
.ac-badge--warn {
  color: var(--warn);
  border-color: currentColor;
}
.ac-badge--danger {
  color: var(--danger);
  border-color: currentColor;
}
.ac-detail__row {
  display: flex;
  gap: 10px;
  padding: 3px 0;
  font-size: 12px;
}
.ac-detail__row > span:first-child {
  width: 64px;
  flex: none;
  color: var(--text-dim);
}
.ac-detail__actions {
  display: flex;
  gap: 6px;
  margin: 10px 0;
}
.ac-fill {
  display: flex;
  align-items: center;
  gap: 6px;
}
.ac-check {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  font-size: 11px;
  color: var(--text-dim);
}
.ac-ellipsis {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.ac-empty {
  margin: 8px 0;
  font-size: 12px;
  color: var(--text-dim);
}
.ac-hint {
  margin: 0 0 10px;
  font-size: 11px;
  line-height: 1.6;
  color: var(--text-dim);
}
.ac-scan__actions {
  display: flex;
  align-items: center;
  gap: var(--sp-2);
}
.ac-scan__summary {
  display: flex;
  gap: var(--sp-2);
  margin-bottom: 10px;
}
.ac-issue {
  display: block;
  color: var(--danger);
  font-size: 11px;
}
.ac-log {
  margin: 0;
  max-height: 200px;
  overflow: auto;
  padding: 10px;
  background: var(--bg);
  border-radius: var(--radius-sm);
  font-family: var(--font-mono, monospace);
  font-size: 11px;
  color: var(--text);
  white-space: pre-wrap;
}
@media (max-width: 980px) {
  .ac-grid {
    grid-template-columns: 1fr;
  }
}
</style>
