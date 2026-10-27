<template>
  <div class="tools-stage">
    <header class="tools-header">
      <div>
        <p class="tools-kicker">TOOLS</p>
        <h1 class="tools-title">{{ t('tools.page.title') }}</h1>
        <p class="tools-subtitle">{{ t('tools.page.subtitle') }}</p>
      </div>
      <div class="tools-actions">
        <button class="btn btn--ghost" :disabled="loading" @click="loadAll">
          <Icon name="refresh" :size="16" /><span>{{ t('tools.page.refresh') }}</span>
        </button>
        <button class="btn btn--primary" :disabled="probing" @click="probeMcp">
          <Icon name="gauge" :size="16" /><span>{{ probing ? t('tools.page.probing') : t('tools.page.probeMcp') }}</span>
        </button>
      </div>
    </header>

    <section class="tools-stats" :aria-label="t('tools.page.statsLabel')">
      <span class="tools-stat"><b>{{ stats.total }}</b><span>{{ t('tools.page.statTotal') }}</span></span>
      <span class="tools-stat tools-stat--ok"><b>{{ stats.enabled }}</b><span>{{ t('tools.page.statEnabled') }}</span></span>
      <span class="tools-stat tools-stat--warn"><b>{{ stats.disabled }}</b><span>{{ t('tools.page.statDisabled') }}</span></span>
      <span class="tools-stat"><b>{{ stats.mcp }}</b><span>{{ t('tools.page.statMcp') }}</span></span>
      <span v-if="patch?.appliesTo" class="tools-stat tools-stat--hint">{{ t('tools.page.appliesNextTurn') }}</span>
    </section>

    <section v-if="error" class="tools-error" role="alert">{{ error }}</section>
    <section v-if="patchError" class="tools-error" role="alert">{{ patchError }}</section>
    <section v-if="patchInactive.length" class="tools-warn" role="status">
      {{ t('tools.page.inactivePatch', { names: patchInactive.join(', ') }) }}
    </section>

    <div class="tools-toolbar">
      <label class="tools-search">
        <Icon name="search" :size="15" />
        <input v-model="search" type="search" :placeholder="t('tools.page.searchPlaceholder')" />
      </label>
      <div class="tools-chips" role="group" :aria-label="t('tools.page.filterSource')">
        <button
          v-for="f in sourceFilters"
          :key="f.id"
          type="button"
          class="tools-chip"
          :class="{ 'is-active': sourceFilter === f.id }"
          @click="sourceFilter = f.id"
        >{{ t(f.labelKey) }}</button>
      </div>
      <div class="tools-chips" role="group" :aria-label="t('tools.page.filterStatus')">
        <button
          v-for="f in statusFilters"
          :key="f.id"
          type="button"
          class="tools-chip"
          :class="{ 'is-active': statusFilter === f.id }"
          @click="statusFilter = f.id"
        >{{ t(f.labelKey) }}</button>
      </div>
    </div>

    <section class="tools-card">
      <table v-if="filtered.length" class="tools-table">
        <thead>
          <tr>
            <th>{{ t('tools.page.colName') }}</th>
            <th>{{ t('tools.page.colSource') }}</th>
            <th>{{ t('tools.page.colDescription') }}</th>
            <th>{{ t('tools.page.colState') }}</th>
            <th>{{ t('tools.page.colAction') }}</th>
          </tr>
        </thead>
        <tbody>
          <tr v-for="tool in filtered" :key="tool.key" :class="{ 'is-off': tool.enabled === false }">
            <td>
              <code class="tools-name">{{ tool.name }}</code>
              <span v-if="tool.server" class="tools-server">{{ tool.server }}</span>
            </td>
            <td>
              <span class="tools-source" :class="'tools-source--' + tool.origin">{{ originLabel(tool.origin) }}</span>
            </td>
            <td class="tools-desc" :title="tool.description">{{ tool.description || '—' }}</td>
            <td>
              <ControlSwitch
                v-if="tool.toggleable"
                :checked="tool.enabled !== false"
                :busy="bulkBusy"
                :aria-label="t('tools.page.toggleAria', { name: tool.name })"
                @change="value => toggleTool(tool, value)"
              />
              <span v-else class="tools-readonly">{{ t('tools.page.managedElsewhere') }}</span>
            </td>
            <td class="tools-row-actions">
              <button type="button" class="btn btn--sm btn--ghost" @click="openDetail(tool)">
                {{ t('tools.page.detail') }}
              </button>
              <RouterLink v-if="tool.origin === 'mcp'" class="btn btn--sm btn--ghost" to="/mcp">
                {{ t('tools.page.openMcp') }}
              </RouterLink>
            </td>
          </tr>
        </tbody>
      </table>
      <div v-else class="tools-empty">{{ loading ? t('tools.page.loading') : t('tools.page.empty') }}</div>
    </section>

    <Teleport to="body">
      <div v-if="activeTool" class="tools-drawer-scrim" @click.self="activeTool = null">
        <aside class="tools-drawer" role="dialog" aria-modal="true" :aria-label="t('tools.page.detailTitle', { name: activeTool.name })">
          <header class="tools-drawer__head">
            <div>
              <p class="tools-kicker">{{ t('tools.page.detailKicker') }}</p>
              <h2>{{ activeTool.name }}</h2>
            </div>
            <button type="button" class="tools-iconbtn" :aria-label="t('common.close')" @click="activeTool = null">
              <Icon name="x" :size="18" />
            </button>
          </header>
          <div class="tools-drawer__body">
            <p class="tools-drawer__desc">{{ activeTool.description || '—' }}</p>
            <dl class="tools-meta">
              <div><dt>{{ t('tools.page.colSource') }}</dt><dd>{{ originLabel(activeTool.origin) }}</dd></div>
              <div><dt>{{ t('tools.page.colState') }}</dt><dd>{{ activeTool.enabled === false ? t('tools.page.stateDisabled') : t('tools.page.stateEnabled') }}</dd></div>
              <div v-if="activeTool.server"><dt>{{ t('tools.page.server') }}</dt><dd>{{ activeTool.server }}</dd></div>
              <div v-if="patch?.path"><dt>{{ t('tools.page.patchPath') }}</dt><dd><code>{{ patch.path }}</code></dd></div>
            </dl>
            <div v-if="activeTool.toggleable" class="tools-drawer__switch">
              <ControlSwitch
                :checked="activeTool.enabled !== false"
                :busy="bulkBusy"
                :label="t('tools.page.enableLabel')"
                :caption="t('tools.page.enableHint')"
                @change="toggleActiveTool"
              />
            </div>
            <p v-else class="tools-hint">{{ t('tools.page.mcpManaged') }}</p>
            <p class="tools-hint">{{ t('tools.page.boundary') }}</p>
          </div>
        </aside>
      </div>
    </Teleport>
  </div>
</template>

<script setup lang="ts">
import { computed, onActivated, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import Icon from '@/components/Icon.vue'
import ControlSwitch from '@/components/ControlSwitch.vue'
import { useRpcStore } from '@/stores/rpc'
import {
  applyDisabled,
  computeNextDisabled,
  disabledNames,
} from '@/utils/tools/toolPatch'

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error)
}

interface CatalogTool {
  name: string
  description: string
  source?: string
  server?: string
  enabled?: boolean
}

interface McpToolRow {
  name: string
  server: string
}

interface PatchState {
  ok: boolean
  disabled: string[]
  inactive: string[]
  path: string
  appliesTo: string
  error?: string | null
}

interface ToolRow {
  key: string
  name: string
  description: string
  server: string
  origin: 'builtin' | 'mcp' | 'other'
  enabled?: boolean
  toggleable: boolean
}

type SourceFilter = 'all' | 'builtin' | 'mcp'
type StatusFilter = 'all' | 'enabled' | 'disabled'

const { t } = useI18n()
const rpc = useRpcStore()

const loading = ref(false)
const probing = ref(false)
const bulkBusy = ref(false)
const error = ref('')
const patchError = ref('')
const search = ref('')
const sourceFilter = ref<SourceFilter>('all')
const statusFilter = ref<StatusFilter>('all')
const catalog = ref<CatalogTool[]>([])
const mcpTools = ref<McpToolRow[]>([])
const patch = ref<PatchState | null>(null)
const patchInactive = ref<string[]>([])
const activeTool = ref<ToolRow | null>(null)

const sourceFilters: Array<{ id: SourceFilter; labelKey: string }> = [
  { id: 'all', labelKey: 'tools.page.filterAll' },
  { id: 'builtin', labelKey: 'tools.page.filterBuiltin' },
  { id: 'mcp', labelKey: 'tools.page.filterMcp' },
]
const statusFilters: Array<{ id: StatusFilter; labelKey: string }> = [
  { id: 'all', labelKey: 'tools.page.statusAll' },
  { id: 'enabled', labelKey: 'tools.page.statusEnabled' },
  { id: 'disabled', labelKey: 'tools.page.statusDisabled' },
]

function originOf(tool: CatalogTool): ToolRow['origin'] {
  // tools.catalog 只返回本地注册表里的内置工具；MCP 工具由 mcp.tools.list 单独返回。
  // 因此没有 server 字段的一律是内置工具（source 字段在不同版本里命名不一，不能拿它判）。
  return tool.server ? 'mcp' : 'builtin'
}

const rows = computed<ToolRow[]>(() => {
  const local = catalog.value.map(tool => {
    const origin = originOf(tool)
    return {
      key: `local:${tool.name}`,
      name: tool.name,
      description: tool.description,
      server: tool.server || '',
      origin,
      enabled: tool.enabled !== false,
      toggleable: origin !== 'mcp',
    } satisfies ToolRow
  })
  const mcp = mcpTools.value.map(tool => ({
    key: `mcp:${tool.server}:${tool.name}`,
    name: tool.name,
    description: '',
    server: tool.server,
    origin: 'mcp' as const,
    enabled: true,
    toggleable: false,
  } satisfies ToolRow))
  return [...local, ...mcp]
})

const filtered = computed(() => {
  const q = search.value.trim().toLowerCase()
  return rows.value.filter(tool => {
    if (sourceFilter.value !== 'all' && tool.origin !== sourceFilter.value) return false
    if (statusFilter.value === 'enabled' && tool.enabled === false) return false
    if (statusFilter.value === 'disabled' && tool.enabled !== false) return false
    if (!q) return true
    return tool.name.toLowerCase().includes(q)
      || tool.description.toLowerCase().includes(q)
      || tool.server.toLowerCase().includes(q)
  })
})

const stats = computed(() => {
  const total = rows.value.length
  const mcp = rows.value.filter(tool => tool.origin === 'mcp').length
  const disabled = catalog.value.filter(tool => tool.enabled === false).length
  return { total, mcp, disabled, enabled: Math.max(0, total - disabled) }
})

function originLabel(origin: ToolRow['origin']): string {
  if (origin === 'mcp') return t('tools.page.originMcp')
  if (origin === 'builtin') return t('tools.page.originBuiltin')
  return t('tools.page.originOther')
}

async function loadCatalog() {
  const res = await rpc.call<{ items?: CatalogTool[] }>('tools.catalog')
  catalog.value = res?.items || []
}

async function loadPatch() {
  patchError.value = ''
  try {
    const res = await rpc.call<PatchState>('tools.patch.get')
    patch.value = res
    patchInactive.value = res?.inactive || []
    if (res?.ok === false) patchError.value = String(res.error || '')
  } catch (e) {
    patch.value = null
    patchError.value = errorMessage(e)
  }
}

async function loadCachedMcp() {
  try {
    const res = await rpc.call<{ tools?: McpToolRow[] }>('mcp.tools.list', { cachedOnly: true })
    mcpTools.value = res?.tools || []
  } catch {
    mcpTools.value = []
  }
}

async function loadAll() {
  loading.value = true
  error.value = ''
  try {
    await rpc.waitForConnection()
    await Promise.all([loadCatalog(), loadPatch(), loadCachedMcp()])
  } catch (e) {
    error.value = t('tools.page.loadFailed', { error: errorMessage(e) })
  } finally {
    loading.value = false
  }
}

async function probeMcp() {
  if (probing.value) return
  probing.value = true
  error.value = ''
  try {
    const res = await rpc.call<{ tools?: McpToolRow[]; servers?: Array<{ name: string; ok: boolean; error?: string | null }>; disabled?: string[] }>('mcp.tools.list')
    mcpTools.value = res?.tools || []
    const bad = (res?.servers || []).filter(server => !server.ok)
    if (bad.length) {
      error.value = bad.map(server => `${server.name}: ${server.error || ''}`).join('；')
    }
  } catch (e) {
    error.value = t('tools.page.probeFailed', { error: errorMessage(e) })
  } finally {
    probing.value = false
  }
}

async function toggleTool(tool: ToolRow, enabled: boolean) {
  if (bulkBusy.value || !tool.toggleable) return
  bulkBusy.value = true
  patchError.value = ''
  try {
    const disabled = computeNextDisabled(disabledNames(catalog.value), tool.name, enabled)
    const res = await rpc.call<{ ok?: boolean; disabled?: string[]; inactive?: string[]; error?: string }>(
      'tools.patch.set',
      { disabled },
    )
    if (res?.ok === false) {
      patchError.value = String(res.error || t('tools.page.toggleFailed'))
      return
    }
    catalog.value = applyDisabled(catalog.value, res?.disabled || disabled)
    patchInactive.value = res?.inactive || []
    if (activeTool.value?.key === tool.key) {
      activeTool.value = { ...activeTool.value, enabled }
    }
  } catch (e) {
    patchError.value = t('tools.page.toggleFailedDetail', { error: errorMessage(e) })
  } finally {
    bulkBusy.value = false
  }
}

function openDetail(tool: ToolRow) {
  activeTool.value = tool
}

function toggleActiveTool(enabled: boolean) {
  if (activeTool.value) void toggleTool(activeTool.value, enabled)
}

// keepAlive 页面：首次显示与回到前台都会触发 onActivated（补丁/目录可能已被别处改动）。
onActivated(() => { void loadAll() })
</script>

<style scoped>
.tools-stage { display: flex; flex-direction: column; gap: 14px; margin: 0 auto; max-width: 1280px; padding: 24px 28px; }
.tools-header { align-items: flex-start; display: flex; gap: 16px; justify-content: space-between; }
.tools-kicker { color: var(--accent); font-family: var(--font-mono); font-size: 10px; letter-spacing: 0.18em; margin: 0 0 4px; text-transform: uppercase; }
.tools-title { font-family: var(--font-display); font-size: 24px; font-weight: 700; line-height: 1.25; margin: 0; }
.tools-subtitle { color: var(--text-muted); font-size: 13px; line-height: 1.6; margin: 6px 0 0; max-width: 720px; }
.tools-actions { align-items: center; display: flex; flex-wrap: wrap; gap: 8px; }
.tools-stats { align-items: center; display: flex; flex-wrap: wrap; gap: 8px; }
.tools-stat { align-items: baseline; background: var(--bg-elevated); border: 1px solid var(--border); border-radius: var(--radius-full); color: var(--text-muted); display: inline-flex; font-size: 11px; gap: 6px; padding: 4px 11px; }
.tools-stat b { color: var(--text); font-family: var(--font-mono); font-size: 13px; }
.tools-stat--ok b { color: var(--ok); }
.tools-stat--warn b { color: var(--warn); }
.tools-stat--hint { border-style: dashed; color: var(--text-dim); }
.tools-error { background: color-mix(in srgb, var(--danger) 10%, var(--card)); border: 1px solid color-mix(in srgb, var(--danger) 30%, transparent); border-radius: var(--radius-sm); color: var(--danger); font-size: 12px; padding: 10px 14px; }
.tools-warn { background: color-mix(in srgb, var(--warn) 10%, var(--card)); border: 1px solid color-mix(in srgb, var(--warn) 30%, transparent); border-radius: var(--radius-sm); color: var(--warn); font-size: 12px; padding: 10px 14px; }
.tools-toolbar { align-items: center; display: flex; flex-wrap: wrap; gap: 10px; }
.tools-search { align-items: center; background: var(--bg); border: 1px solid var(--border); border-radius: var(--radius-md); display: flex; gap: 8px; min-width: 260px; padding: 7px 11px; }
.tools-search input { background: transparent; border: 0; color: var(--text); font-family: inherit; font-size: 13px; outline: none; width: 100%; }
.tools-chips { display: flex; flex-wrap: wrap; gap: 6px; }
.tools-chip { background: var(--bg); border: 1px solid var(--border); border-radius: var(--radius-full); color: var(--text-dim); cursor: pointer; font: inherit; font-size: 12px; padding: 5px 12px; transition: border-color var(--dur-fast) var(--ease-out), color var(--dur-fast) var(--ease-out); }
.tools-chip:hover { color: var(--text); }
.tools-chip.is-active { background: color-mix(in srgb, var(--accent) 12%, var(--card)); border-color: var(--accent); color: var(--accent); }
.tools-card { background: var(--card); border: 1px solid var(--border); border-radius: var(--radius-lg); overflow: hidden; }
.tools-table { border-collapse: collapse; font-size: 13px; width: 100%; }
.tools-table th, .tools-table td { border-bottom: 1px solid var(--border); padding: 10px 12px; text-align: left; vertical-align: middle; }
.tools-table th { color: var(--text-dim); font-size: 11px; font-weight: 700; letter-spacing: 0.05em; text-transform: uppercase; }
.tools-table tbody tr:hover { background: color-mix(in srgb, var(--accent) 4%, transparent); }
.tools-table tbody tr.is-off { background: color-mix(in srgb, var(--warn) 6%, transparent); }
.tools-name { color: var(--text); font-family: var(--font-mono); font-size: 12px; }
.tools-server { background: var(--bg-hover); border-radius: var(--radius-sm); color: var(--text-dim); font-size: 10px; margin-left: 8px; padding: 1px 6px; }
.tools-source { border: 1px solid var(--border); border-radius: var(--radius-full); color: var(--text-dim); font-size: 10px; padding: 2px 8px; white-space: nowrap; }
.tools-source--mcp { border-color: color-mix(in srgb, var(--accent) 40%, var(--border)); color: var(--accent); }
.tools-desc { color: var(--text-muted); max-width: 560px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.tools-readonly { color: var(--text-dim); font-size: 11px; }
.tools-row-actions { display: flex; gap: 6px; flex-wrap: wrap; }
.tools-empty { color: var(--text-dim); font-size: 13px; padding: 40px; text-align: center; }
.tools-drawer-scrim { align-items: stretch; background: var(--scrim); display: flex; inset: 0; justify-content: flex-end; position: fixed; z-index: 1100; }
.tools-drawer { background: var(--bg-surface); border-left: 1px solid var(--border); box-shadow: var(--elev-3); display: flex; flex-direction: column; max-width: 520px; width: min(92vw, 520px); }
.tools-drawer__head { align-items: flex-start; border-bottom: 1px solid var(--border); display: flex; gap: 12px; justify-content: space-between; padding: 18px 20px; }
.tools-drawer__head h2 { font-family: var(--font-mono); font-size: 16px; margin: 0; }
.tools-iconbtn { background: transparent; border: 1px solid var(--border); border-radius: var(--radius-md); color: var(--text-muted); cursor: pointer; display: inline-flex; padding: 6px; }
.tools-drawer__body { display: flex; flex: 1; flex-direction: column; gap: 14px; overflow: auto; padding: 18px 20px; }
.tools-drawer__desc { color: var(--text); font-size: 13px; line-height: 1.7; margin: 0; }
.tools-meta { display: grid; gap: 8px; margin: 0; }
.tools-meta > div { display: grid; gap: 8px; grid-template-columns: 120px minmax(0, 1fr); }
.tools-meta dt { color: var(--text-dim); font-size: 11px; text-transform: uppercase; }
.tools-meta dd { color: var(--text); font-size: 12px; margin: 0; overflow-wrap: anywhere; }
.tools-meta code { font-family: var(--font-mono); font-size: 11px; }
.tools-drawer__switch { border-top: 1px solid var(--border); padding-top: 12px; }
.tools-hint { color: var(--text-dim); font-size: 12px; line-height: 1.6; margin: 0; }
@media (max-width: 760px) {
  .tools-stage { padding: 18px 14px; }
  .tools-header { flex-direction: column; }
  .tools-desc { max-width: 200px; }
}
</style>
