<template>
  <div class="control-stage mcp-stage">
    <header class="control-stage__header">
      <div class="control-stage__title-block">
        <h1 class="control-stage__title">{{ t('mcp.page.title') }}</h1>
        <p class="control-stage__subtitle">{{ t('mcp.page.subtitle') }}</p>
      </div>
      <div class="control-stage__actions">
        <button class="btn btn--ghost" :disabled="busy" @click="loadAll">
          <Icon name="refresh" :size="16" /><span>{{ t('mcp.page.refresh') }}</span>
        </button>
        <button class="btn btn--primary" @click="openAdd">
          <Icon name="plus" :size="16" /><span>{{ t('mcp.page.addServer') }}</span>
        </button>
      </div>
    </header>

    <section v-if="serversError" class="mcp-error">{{ serversError }}</section>

    <div class="mcp-toolbar">
      <label class="mcp-search">
        <Icon name="search" :size="15" />
        <input v-model="serverSearch" type="search" :placeholder="t('mcp.page.searchPlaceholder')" />
      </label>
      <div class="mcp-chips" role="group" :aria-label="t('mcp.page.filterLabel')">
        <button
          v-for="f in serverFilters"
          :key="f.id"
          type="button"
          class="mcp-chip"
          :class="{ 'is-active': serverFilter === f.id }"
          @click="serverFilter = f.id"
        >{{ t(f.labelKey) }}</button>
      </div>
      <span v-if="enabledCount !== null" class="mcp-count">{{ t('mcp.page.enabledCount', { enabled: enabledCount, total: servers.length }) }}</span>
    </div>

    <!-- ① 服务器列表 -->
    <section class="mcp-card">
      <div class="mcp-card__head">
        <div class="mcp-card__title">{{ t('mcp.page.serversTitle') }}</div>
        <button class="btn btn--sm btn--ghost" :disabled="toolsBusy || !servers.length" @click="probeAll">
          {{ toolsBusy ? t('mcp.page.probing') : t('mcp.page.probeAll') }}
        </button>
      </div>
      <div v-if="toolsError" class="mcp-note mcp-note--err">{{ toolsError }}</div>
      <table v-if="servers.length" class="mcp-table">
        <thead>
          <tr>
            <th>{{ t('mcp.page.colName') }}</th>
            <th>{{ t('mcp.page.colTransport') }}</th>
            <th>{{ t('mcp.page.colEnabled') }}</th>
            <th>{{ t('mcp.page.colArgs') }}</th>
            <th>{{ t('mcp.page.colEnv') }}</th>
            <th>{{ t('mcp.page.colProbe') }}</th>
            <th>{{ t('mcp.page.colActions') }}</th>
          </tr>
        </thead>
        <tbody>
          <tr v-for="s in filteredServers" :key="s.name">
            <td><b>{{ s.name }}</b></td>
            <td>
              <span class="mcp-kind">{{ s.url ? t('mcp.page.transportUrl') : t('mcp.page.transportCommand') }}</span>
              <code class="mcp-code">{{ s.url || s.command }}</code>
            </td>
            <td>
              <ControlSwitch
                :checked="s.enabled !== false"
                :aria-label="t('mcp.page.toggleAria', { name: s.name })"
                @change="value => toggleServer(s.name, value)"
              />
            </td>
            <td><code class="mcp-code">{{ (s.args || []).join(' ') || '—' }}</code></td>
            <td>
              <span v-if="(s.env || []).length" class="mcp-env">{{ (s.env || []).join(', ') }}</span>
              <span v-else class="mcp-dim">—</span>
            </td>
            <td>
              <template v-if="probeState(s.name)">
                <span
                  v-if="probeState(s.name)!.ok"
                  class="mcp-badge mcp-badge--ok"
                >{{ t('mcp.page.toolCount', { n: probeState(s.name)!.count ?? probeState(s.name)!.tools.length }) }}</span>
                <span
                  v-else
                  class="mcp-badge mcp-badge--err"
                  :title="probeState(s.name)!.error || ''"
                >{{ probeState(s.name)!.code || t('mcp.page.probeFailed') }}</span>
              </template>
              <span v-else class="mcp-badge">{{ t('mcp.page.notProbed') }}</span>
            </td>
            <td class="mcp-actions">
              <button class="btn btn--sm btn--ghost" :disabled="probing[s.name]" @click="probeOne(s.name)">
                {{ probing[s.name] ? t('mcp.page.probing') : t('mcp.page.probe') }}
              </button>
              <button class="btn btn--sm btn--ghost" @click="openEdit(s)">{{ t('mcp.page.edit') }}</button>
              <button class="btn btn--sm btn--ghost" @click="duplicateServer(s)">{{ t('mcp.page.duplicate') }}</button>
              <button class="btn btn--sm btn--ghost" @click="copyServer(s)">{{ t('mcp.page.copyJson') }}</button>
              <button class="btn btn--sm btn--ghost mcp-danger" @click="removeServer(s.name)">{{ t('mcp.page.delete') }}</button>
            </td>
          </tr>
        </tbody>
      </table>
      <EmptyState
        v-else-if="servers.length"
        icon="search"
        :icon-size="28"
        :title="t('mcp.page.filterEmptyTitle')"
        :description="t('mcp.page.filterEmptyHint')"
        test-id="mcp-empty-filter"
      />
      <EmptyState
        v-else
        icon="plugin"
        :icon-size="28"
        :title="t('mcp.page.emptyTitle')"
        :description="t('mcp.page.emptyHint')"
        :action-label="t('mcp.page.addServer')"
        test-id="mcp-empty"
        @action="openAdd"
      />
      <div v-if="deleteNotice" class="mcp-note" :class="{ 'mcp-note--err': !deleteOk }">{{ deleteNotice }}</div>
    </section>

    <!-- ② 新增 / 编辑表单 -->
    <section v-if="formOpen" class="mcp-card">
      <div class="mcp-card__head">
        <div class="mcp-card__title">{{ editing ? t('mcp.page.formTitleEdit') : t('mcp.page.formTitleAdd') }}</div>
        <button class="btn btn--sm btn--ghost" @click="formOpen = false">{{ t('mcp.page.cancel') }}</button>
      </div>
      <div class="mcp-form">
        <label class="mcp-field">
          <span>{{ t('mcp.page.name') }}</span>
          <input v-model="form.name" class="mcp-input" :disabled="!!editing" :placeholder="t('mcp.page.namePlaceholder')" />
        </label>
        <label class="mcp-field">
          <span>{{ t('mcp.page.url') }}</span>
          <input v-model="form.url" class="mcp-input" :placeholder="t('mcp.page.urlPlaceholder')" />
        </label>
        <label class="mcp-field">
          <span>{{ t('mcp.page.command') }}</span>
          <input v-model="form.command" class="mcp-input" :placeholder="t('mcp.page.commandPlaceholder')" />
        </label>
        <label class="mcp-field mcp-field--full">
          <span>{{ t('mcp.page.args') }}</span>
          <textarea v-model="form.args" class="mcp-textarea" rows="3" :placeholder="t('mcp.page.argsPlaceholder')"></textarea>
          <span class="mcp-field__hint">{{ t('mcp.page.argsHint') }}</span>
        </label>
        <label class="mcp-field mcp-field--full">
          <span>{{ t('mcp.page.env') }}</span>
          <textarea
            v-model="form.env"
            class="mcp-textarea"
            rows="3"
            :placeholder="editing ? t('mcp.page.envKeepPlaceholder') : t('mcp.page.envPlaceholder')"
          ></textarea>
          <span class="mcp-field__hint">{{ t('mcp.page.envHint') }}</span>
        </label>
        <div class="mcp-form__actions">
          <button class="btn btn--ghost" @click="formOpen = false">{{ t('mcp.page.cancel') }}</button>
          <button class="btn btn--primary" :disabled="saving" @click="saveServer">
            {{ saving ? t('mcp.page.saving') : t('mcp.page.save') }}
          </button>
        </div>
        <div v-if="formResult" class="mcp-form__result" :class="{ 'is-error': !formOk }">{{ formResult }}</div>
      </div>
    </section>

    <!-- ③ 插件声明的 MCP 进程 -->
    <section class="mcp-card">
      <div class="mcp-card__head">
        <div class="mcp-card__title">{{ t('mcp.page.sectionPluginsTitle') }}</div>
        <button class="btn btn--sm btn--ghost" :disabled="pluginBusy" @click="loadPlugins">{{ t('mcp.page.refresh') }}</button>
      </div>
      <div class="mcp-note">{{ t('mcp.page.pluginsHint') }}</div>
      <div v-if="pluginsError" class="mcp-note mcp-note--err">{{ pluginsError }}</div>
      <div v-if="pluginNotice" class="mcp-note" :class="{ 'mcp-note--err': !pluginNoticeOk }">{{ pluginNotice }}</div>
      <table v-if="pluginRows.length" class="mcp-table">
        <thead>
          <tr>
            <th>{{ t('mcp.page.colPlugin') }}</th>
            <th>{{ t('mcp.page.colDeclared') }}</th>
            <th>{{ t('mcp.page.colCommand') }}</th>
            <th>{{ t('mcp.page.colTimeout') }}</th>
            <th>{{ t('mcp.page.colRunning') }}</th>
            <th>{{ t('mcp.page.colTools') }}</th>
            <th>{{ t('mcp.page.colActions') }}</th>
          </tr>
        </thead>
        <tbody>
          <tr v-for="p in pluginRows" :key="p.name">
            <td><b>{{ p.name }}</b></td>
            <td>{{ p.declared ? t('mcp.page.declaredYes') : t('mcp.page.declaredNo') }}</td>
            <td>
              <code class="mcp-code">{{ [p.command, ...(p.args || [])].filter(Boolean).join(' ') || '—' }}</code>
            </td>
            <td>{{ typeof p.timeoutMs === 'number' ? `${p.timeoutMs} ms` : '—' }}</td>
            <td>
              <span class="mcp-badge" :class="p.running ? 'mcp-badge--ok' : ''">
                {{ p.running ? t('mcp.page.running') : t('mcp.page.stopped') }}
              </span>
            </td>
            <td>
              <span v-if="p.error" class="mcp-badge mcp-badge--err" :title="p.error">{{ p.error }}</span>
              <span v-else-if="(p.tools || []).length" class="mcp-env">{{ p.tools.join(', ') }}</span>
              <span v-else class="mcp-dim">{{ t('mcp.page.noTools') }}</span>
            </td>
            <td class="mcp-actions">
              <button v-if="!p.running" class="btn btn--sm btn--primary" :disabled="pluginBusy" @click="startPlugin(p.name)">
                {{ t('mcp.page.start') }}
              </button>
              <button v-else class="btn btn--sm btn--ghost" :disabled="pluginBusy" @click="stopPlugin(p.name)">
                {{ t('mcp.page.stop') }}
              </button>
            </td>
          </tr>
        </tbody>
      </table>
      <div v-else-if="!pluginsError" class="mcp-note">{{ t('mcp.page.noPluginMcp') }}</div>
    </section>

    <!-- ④ 工具清单 -->
    <section class="mcp-card">
      <div class="mcp-card__head">
        <div class="mcp-card__title">{{ t('mcp.page.sectionToolsTitle') }}</div>
        <button class="btn btn--sm btn--ghost" :disabled="toolsBusy || !servers.length" @click="probeAll">
          {{ toolsBusy ? t('mcp.page.probing') : t('mcp.page.probeAll') }}
        </button>
      </div>
      <div class="mcp-note">{{ t('mcp.page.toolsHint') }}</div>
      <div v-if="bulkTools?.stale" class="mcp-note">{{ t('mcp.page.toolsStale') }}</div>
      <div v-if="!servers.length" class="mcp-note">{{ t('mcp.page.toolsEmpty') }}</div>
      <div v-for="s in servers" :key="'tools-' + s.name" class="mcp-tools">
        <div class="mcp-tools__head">
          <b>{{ s.name }}</b>
          <template v-if="probeState(s.name)">
            <span v-if="probeState(s.name)!.ok" class="mcp-badge mcp-badge--ok">
              {{ t('mcp.page.toolsCount', { n: probeState(s.name)!.count ?? probeState(s.name)!.tools.length }) }}
            </span>
            <span v-else class="mcp-badge mcp-badge--err" :title="probeState(s.name)!.error || ''">
              {{ probeState(s.name)!.code || t('mcp.page.probeFailed') }}
            </span>
          </template>
          <span v-else class="mcp-badge">{{ t('mcp.page.toolsNotProbed') }}</span>
        </div>
        <ul v-if="probeState(s.name)?.ok && toolsForServer(s.name).length" class="mcp-tools__list">
          <li v-for="tool in toolsForServer(s.name)" :key="tool.name" class="mcp-tools__row">
            <code class="mcp-code">{{ tool.name }}</code>
            <span class="mcp-tools__desc">{{ tool.description || '—' }}</span>
          </li>
        </ul>
        <div v-else-if="probeState(s.name)?.ok" class="mcp-note">{{ t('mcp.page.toolsNone') }}</div>
      </div>
    </section>
  </div>
</template>

<script setup lang="ts">
import { onMounted, reactive, ref, computed } from 'vue'
import { useI18n } from 'vue-i18n'
import Icon from '@/components/Icon.vue'
import EmptyState from '@/components/EmptyState.vue'
import ControlSwitch from '@/components/ControlSwitch.vue'
import { useRpcStore } from '@/stores/rpc'
import { useConfirm } from '@/composables/useConfirm'

interface McpServer {
  name: string
  command: string
  args: string[]
  env: string[]
  url?: string | null
  enabled?: boolean
}

interface ProbeTool {
  name: string
  description?: string
}

interface ProbeState {
  ok: boolean
  tools: ProbeTool[]
  count?: number | null
  error?: string
  code?: string
}

interface ToolsServerState {
  name: string
  ok: boolean
  count?: number | null
  error?: string | null
}

interface ToolsPayload {
  ok: boolean
  tools?: Array<{ name: string; server: string }>
  servers?: ToolsServerState[]
  total?: number
  cached?: boolean
  stale?: boolean
  note?: string
  error?: string
}

interface PluginMcpRow {
  name: string
  declared: boolean
  command?: string
  args?: string[]
  timeoutMs?: number | null
  running: boolean
  tools: string[]
  error?: string | null
}

const { t } = useI18n()
const rpc = useRpcStore()

const servers = ref<McpServer[]>([])
const serversError = ref('')
const busy = ref(false)

const probes = ref<Record<string, ProbeState>>({})
const probing = reactive<Record<string, boolean>>({})
const bulkTools = ref<ToolsPayload | null>(null)
const toolsBusy = ref(false)
const toolsError = ref('')

const pluginRows = ref<PluginMcpRow[]>([])
const pluginsError = ref('')
const pluginBusy = ref(false)
const pluginNotice = ref('')
const pluginNoticeOk = ref(true)

const formOpen = ref(false)
const editing = ref('')
const saving = ref(false)
const formResult = ref('')
const formOk = ref(true)
const form = reactive({ name: '', url: '', command: '', args: '', env: '' })
const deleteNotice = ref('')
const deleteOk = ref(true)

const serverSearch = ref('')
type ServerFilterId = 'all' | 'enabled' | 'disabled'
const serverFilter = ref<ServerFilterId>('all')
const serverFilters: Array<{ id: ServerFilterId; labelKey: string }> = [
  { id: 'all', labelKey: 'mcp.page.filterAll' },
  { id: 'enabled', labelKey: 'mcp.page.filterEnabled' },
  { id: 'disabled', labelKey: 'mcp.page.filterDisabled' },
]
const { confirm } = useConfirm()

const filteredServers = computed(() => {
  const q = serverSearch.value.trim().toLowerCase()
  return servers.value.filter(server => {
    if (serverFilter.value === 'enabled' && server.enabled === false) return false
    if (serverFilter.value === 'disabled' && server.enabled !== false) return false
    if (!q) return true
    return server.name.toLowerCase().includes(q)
      || String(server.url || '').toLowerCase().includes(q)
      || server.command.toLowerCase().includes(q)
      || (server.args || []).some(arg => arg.toLowerCase().includes(q))
  })
})

const enabledCount = computed<number | null>(() => {
  if (!servers.value.length) return null
  return servers.value.filter(server => server.enabled !== false).length
})

function errText(e: unknown): string {
  return e instanceof Error ? e.message : String(e)
}

function errCode(e: unknown): string {
  const code = (e as { code?: unknown })?.code
  return typeof code === 'string' ? code : ''
}

async function loadServers() {
  serversError.value = ''
  // 冷加载竞态：视图挂载时 WS 可能仍在 connecting；先等连接就绪再判定能力位，
  // 否则会把「还在握手」误报成「网关不支持 MCP」。
  try {
    await rpc.waitForConnection()
  } catch {
    // 连接失败由后续 call 给出可读错误
  }
  if (!rpc.supportsMethod('mcp.servers.list')) {
    serversError.value = t('mcp.page.unsupported')
    return
  }
  try {
    const res = await rpc.call<{ ok?: boolean; servers?: McpServer[]; error?: string }>('mcp.servers.list')
    if (res?.ok === false) {
      serversError.value = res.error || t('mcp.page.loadFailed')
      return
    }
    servers.value = res?.servers || []
    // 掉线/删除后清掉过期探测结果，避免展示已不存在服务器的状态。
    const names = new Set(servers.value.map((s) => s.name))
    for (const name of Object.keys(probes.value)) {
      if (!names.has(name)) delete probes.value[name]
    }
  } catch (e) {
    serversError.value = t('mcp.page.loadFailedDetail', { error: errText(e) })
  }
}

function openAdd() {
  editing.value = ''
  form.name = ''
  form.url = ''
  form.command = ''
  form.args = ''
  form.env = ''
  formResult.value = ''
  formOpen.value = true
}

function openEdit(server: McpServer) {
  editing.value = server.name
  form.name = server.name
  form.url = server.url || ''
  form.command = server.command || ''
  form.args = (server.args || []).join('\n')
  // env 值只写不回显：编辑时留空 = 不改；已设置的键在列表的 env 列可见。
  form.env = ''
  formResult.value = ''
  formOpen.value = true
}

/** 复制一个新条目：名称加 -copy，传输与参数预填，env 留空（避免复制密钥）。 */
function duplicateServer(server: McpServer) {
  editing.value = ''
  form.name = `${server.name}-copy`
  form.url = server.url || ''
  form.command = server.command || ''
  form.args = (server.args || []).join('\n')
  form.env = ''
  formResult.value = ''
  formOk.value = true
  formOpen.value = true
}

/** 复制服务器配置为 JSON（不含 env 值；只给可分享的结构）。 */
async function copyServer(server: McpServer) {
  const payload = {
    name: server.name,
    url: server.url || '',
    command: server.command,
    args: server.args || [],
    envKeys: server.env || [],
    enabled: server.enabled !== false,
  }
  try {
    await navigator.clipboard.writeText(JSON.stringify(payload, null, 2))
    deleteOk.value = true
    deleteNotice.value = t('mcp.page.copied', { name: server.name })
  } catch {
    deleteOk.value = false
    deleteNotice.value = t('mcp.page.copyFailed')
  }
}

/** 启用/停用：写入 config.yaml，运行期加载会跳过停用项。 */
async function toggleServer(name: string, enabled: boolean) {
  try {
    const res = await rpc.call<{ ok?: boolean; error?: string; code?: string }>(
      'mcp.servers.setEnabled',
      { name, enabled },
    )
    if (res?.ok === false) {
      deleteOk.value = false
      deleteNotice.value = res.error || res.code || t('mcp.page.toggleFailed')
      return
    }
    deleteOk.value = true
    deleteNotice.value = t(enabled ? 'mcp.page.enabledNotice' : 'mcp.page.disabledNotice', { name })
    await loadServers()
  } catch (e) {
    deleteOk.value = false
    deleteNotice.value = t('mcp.page.toggleFailedDetail', { error: errText(e) })
  }
}

function parseArgs(raw: string): string[] {
  return raw
    .split(/\r?\n/)
    .map((line) => line.trim())
    .filter(Boolean)
}

function parseEnv(raw: string): Record<string, string> {
  const env: Record<string, string> = {}
  for (const line of raw.split(/\r?\n/)) {
    const text = line.trim()
    if (!text) continue
    const idx = text.indexOf('=')
    if (idx <= 0) throw new Error(t('mcp.page.envInvalid', { line: text }))
    env[text.slice(0, idx).trim()] = text.slice(idx + 1).trim()
  }
  return env
}

async function saveServer() {
  if (!form.name.trim()) {
    formOk.value = false
    formResult.value = t('mcp.page.requiredName')
    return
  }
  if (!form.url.trim() && !form.command.trim()) {
    formOk.value = false
    formResult.value = t('mcp.page.requiredTransport')
    return
  }
  let env: Record<string, string> | null = null
  try {
    if (form.env.trim()) env = parseEnv(form.env)
  } catch (e) {
    formOk.value = false
    formResult.value = errText(e)
    return
  }
  saving.value = true
  formResult.value = ''
  try {
    const params: Record<string, unknown> = {
      name: form.name.trim(),
      url: form.url.trim(),
      command: form.command.trim(),
      args: parseArgs(form.args),
    }
    // 编辑时 env 留空 = 保留原值（不回显、不覆盖）；新增时留空 = 空环境。
    if (env) params.env = env
    const method = editing.value ? 'mcp.servers.update' : 'mcp.servers.add'
    const res = await rpc.call<{ ok?: boolean; error?: string; code?: string }>(method, params)
    if (res?.ok === false) {
      formOk.value = false
      formResult.value = res.code ? `${res.code}: ${res.error || ''}` : (res.error || t('mcp.page.saveFailed'))
      return
    }
    formOk.value = true
    formResult.value = t('mcp.page.saved', { name: form.name.trim() })
    formOpen.value = false
    editing.value = ''
    await loadServers()
  } catch (e) {
    formOk.value = false
    formResult.value = t('mcp.page.saveFailedDetail', { error: errText(e) })
  } finally {
    saving.value = false
  }
}

async function removeServer(name: string) {
  deleteNotice.value = ''
  const approved = await confirm({
    title: t('mcp.page.deleteTitle', { name }),
    body: t('mcp.page.deleteBody', { name }),
    primaryLabel: t('mcp.page.deleteConfirm'),
  })
  if (!approved) return
  try {
    const res = await rpc.call<{ ok?: boolean; error?: string }>('mcp.servers.remove', { name })
    if (res?.ok === false) {
      deleteOk.value = false
      deleteNotice.value = res.error || t('mcp.page.deleteFailed')
      return
    }
    deleteOk.value = true
    deleteNotice.value = t('mcp.page.deleted', { name })
    delete probes.value[name]
    await loadServers()
  } catch (e) {
    deleteOk.value = false
    deleteNotice.value = t('mcp.page.deleteFailedDetail', { error: errText(e) })
  }
}

async function probeOne(name: string) {
  probing[name] = true
  try {
    const res = await rpc.call<{ ok?: boolean; tools?: ProbeTool[] }>('mcp.servers.probe', { name })
    probes.value[name] = { ok: true, tools: res?.tools || [] }
  } catch (e) {
    probes.value[name] = {
      ok: false,
      tools: [],
      error: errText(e),
      code: errCode(e) || undefined,
    }
  } finally {
    probing[name] = false
  }
}

async function probeAll() {
  if (toolsBusy.value) return
  toolsBusy.value = true
  toolsError.value = ''
  try {
    await rpc.waitForConnection()
  } catch {
    // 交后续调用报错
  }
  if (!rpc.supportsMethod('mcp.tools.list')) {
    toolsError.value = t('mcp.page.unsupported')
    toolsBusy.value = false
    return
  }
  try {
    const res = await rpc.call<ToolsPayload>('mcp.tools.list')
    bulkTools.value = res
    if (res?.ok === false) toolsError.value = res.error || t('mcp.page.probeFailed')
  } catch (e) {
    bulkTools.value = null
    toolsError.value = errCode(e) || errText(e)
  } finally {
    toolsBusy.value = false
  }
}

/** 单服务器探测优先，其次 mcp.tools.list 的逐服务器聚合结果。 */
function probeState(name: string): ProbeState | null {
  const single = probes.value[name]
  if (single) return single
  const bulk = bulkTools.value?.servers?.find((s) => s.name === name)
  if (!bulk) return null
  return {
    ok: bulk.ok,
    tools: [],
    count: bulk.count,
    error: bulk.error || undefined,
    code: bulk.ok ? undefined : 'MCP_PROBE_FAILED',
  }
}

function toolsForServer(name: string): ProbeTool[] {
  const single = probes.value[name]
  if (single?.ok) return single.tools
  return (bulkTools.value?.tools || [])
    .filter((tool) => tool.server === name)
    .map((tool) => ({ name: tool.name }))
}

async function loadPlugins() {
  pluginsError.value = ''
  try {
    await rpc.waitForConnection()
  } catch {
    // 同上：连接失败交由 call 报错
  }
  if (!rpc.supportsMethod('plugins.mcp.list')) {
    pluginsError.value = t('mcp.page.unsupported')
    pluginRows.value = []
    return
  }
  pluginBusy.value = true
  try {
    const res = await rpc.call<{ ok?: boolean; plugins?: PluginMcpRow[]; error?: string }>('plugins.mcp.list')
    if (res?.ok === false) {
      pluginsError.value = res.error || t('mcp.page.loadFailed')
      return
    }
    pluginRows.value = res?.plugins || []
  } catch (e) {
    pluginsError.value = t('mcp.page.loadFailedDetail', { error: errText(e) })
  } finally {
    pluginBusy.value = false
  }
}

async function startPlugin(name: string) {
  pluginBusy.value = true
  pluginNotice.value = ''
  try {
    const res = await rpc.call<{ ok?: boolean; tools?: string[]; error?: string; code?: string }>('plugins.mcp.start', { name })
    pluginNoticeOk.value = res?.ok !== false
    pluginNotice.value = pluginNoticeOk.value
      ? t('mcp.page.started', { name, n: (res?.tools || []).length })
      : `${res?.code || ''} ${res?.error || ''}`.trim()
  } catch (e) {
    pluginNoticeOk.value = false
    pluginNotice.value = t('mcp.page.startFailed', { error: errText(e) })
  } finally {
    pluginBusy.value = false
    await loadPlugins()
  }
}

async function stopPlugin(name: string) {
  pluginBusy.value = true
  pluginNotice.value = ''
  try {
    const res = await rpc.call<{ ok?: boolean; error?: string; code?: string }>('plugins.mcp.stop', { name })
    pluginNoticeOk.value = res?.ok !== false
    pluginNotice.value = pluginNoticeOk.value
      ? t('mcp.page.stoppedNotice', { name })
      : `${res?.code || ''} ${res?.error || ''}`.trim()
  } catch (e) {
    pluginNoticeOk.value = false
    pluginNotice.value = t('mcp.page.stopFailed', { error: errText(e) })
  } finally {
    pluginBusy.value = false
    await loadPlugins()
  }
}

async function loadAll() {
  busy.value = true
  try {
    await Promise.all([loadServers(), loadPlugins()])
  } finally {
    busy.value = false
  }
}

onMounted(() => {
  void loadAll()
})
</script>

<style scoped>
.mcp-stage { padding: 24px 28px; max-width: 1200px; margin: 0 auto; }
.mcp-error {
  margin-top: 14px;
  padding: 10px 14px;
  border-radius: var(--radius-sm);
  border: 1px solid color-mix(in srgb, var(--danger) 30%, transparent);
  background: color-mix(in srgb, var(--danger) 10%, var(--card));
  color: var(--danger);
  font-size: 12px;
}
.mcp-card {
  margin-top: 16px;
  padding: 16px;
  border: 1px solid var(--hairline);
  border-radius: var(--radius-md);
  background: var(--card);
}
.mcp-card__head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 10px;
  flex-wrap: wrap;
}
.mcp-card__title { font-size: 14px; font-weight: 700; color: var(--text); }
.mcp-note { margin-top: 8px; font-size: 12px; color: var(--text-dim); word-break: break-word; }
.mcp-note--err { color: var(--danger); }
.mcp-table { width: 100%; border-collapse: collapse; margin-top: 10px; font-size: 12px; }
.mcp-table th,
.mcp-table td {
  text-align: left;
  padding: 7px 8px;
  border-bottom: 1px solid var(--hairline);
  vertical-align: top;
}
.mcp-table th { color: var(--text-dim); font-weight: 600; }
.mcp-code { font-family: var(--font-mono, monospace); font-size: 11px; color: var(--text); word-break: break-all; }
.mcp-env { font-family: var(--font-mono, monospace); font-size: 11px; color: var(--text-muted); word-break: break-all; }
.mcp-dim { color: var(--text-dim); }
.mcp-kind {
  display: inline-block;
  margin-right: 6px;
  padding: 1px 7px;
  border-radius: var(--radius-pill);
  background: var(--bg-hover);
  color: var(--text-dim);
  font-size: 10px;
}
.mcp-badge {
  display: inline-block;
  padding: 1px 8px;
  border-radius: var(--radius-pill);
  border: 1px solid var(--hairline);
  background: var(--bg-hover);
  color: var(--text-dim);
  font-size: 10px;
  white-space: nowrap;
}
.mcp-badge--ok {
  border-color: color-mix(in srgb, var(--ok) 45%, transparent);
  background: color-mix(in srgb, var(--ok) 12%, var(--card));
  color: var(--ok);
}
.mcp-badge--err {
  border-color: color-mix(in srgb, var(--danger) 45%, transparent);
  background: color-mix(in srgb, var(--danger) 12%, var(--card));
  color: var(--danger);
}
.mcp-actions { display: flex; gap: 6px; flex-wrap: wrap; }
.mcp-danger { color: var(--danger); }
.mcp-form { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 10px; margin-top: 12px; }
.mcp-field { display: flex; flex-direction: column; gap: 4px; font-size: 11px; color: var(--text-dim); }
.mcp-field--full { grid-column: 1 / -1; }
.mcp-field__hint { color: var(--text-dim); font-size: 10px; line-height: 1.5; }
.mcp-input,
.mcp-textarea {
  padding: 7px 10px;
  border-radius: var(--radius-sm);
  border: 1px solid var(--hairline);
  background: var(--bg);
  color: var(--text);
  font-size: 12px;
  outline: none;
}
.mcp-input:focus,
.mcp-textarea:focus { border-color: var(--accent); }
.mcp-input:disabled { opacity: 0.6; }
.mcp-textarea { resize: vertical; font-family: var(--font-mono, monospace); }
.mcp-form__actions {
  grid-column: 1 / -1;
  display: flex;
  justify-content: flex-end;
  gap: 8px;
  margin-top: 4px;
}
.mcp-form__result {
  grid-column: 1 / -1;
  padding: 8px 12px;
  border-radius: var(--radius-sm);
  font-size: 12px;
  background: color-mix(in srgb, var(--ok) 10%, var(--card));
  color: var(--ok);
}
.mcp-form__result.is-error {
  background: color-mix(in srgb, var(--danger) 10%, var(--card));
  color: var(--danger);
}
.mcp-tools { margin-top: 12px; padding-top: 10px; border-top: 1px solid var(--hairline); }
.mcp-tools__head { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; font-size: 12px; }
.mcp-tools__list { list-style: none; margin: 8px 0 0; padding: 0; display: flex; flex-direction: column; gap: 4px; }
.mcp-tools__row { display: flex; gap: 10px; align-items: baseline; font-size: 12px; }
.mcp-tools__desc { color: var(--text-muted); font-size: 11px; word-break: break-word; }
@media (max-width: 760px) {
  .mcp-form { grid-template-columns: minmax(0, 1fr); }
  .mcp-stage { padding: 18px 14px; }
}
.mcp-toolbar { align-items: center; display: flex; flex-wrap: wrap; gap: 10px; margin-top: 14px; }
.mcp-search { align-items: center; background: var(--bg); border: 1px solid var(--hairline); border-radius: var(--radius-md); display: flex; gap: 8px; min-width: 260px; padding: 7px 11px; }
.mcp-search input { background: transparent; border: 0; color: var(--text); font-family: inherit; font-size: 13px; outline: none; width: 100%; }
.mcp-chips { display: flex; flex-wrap: wrap; gap: 6px; }
.mcp-chip { background: var(--bg); border: 1px solid var(--hairline); border-radius: var(--radius-pill); color: var(--text-dim); cursor: pointer; font: inherit; font-size: 12px; padding: 5px 12px; }
.mcp-chip:hover { color: var(--text); }
.mcp-chip.is-active { background: color-mix(in srgb, var(--accent) 12%, var(--card)); border-color: var(--accent); color: var(--accent); }
.mcp-count { color: var(--text-dim); font-size: 11px; }
@media (max-width: 760px) {
  .mcp-search { flex: 1 1 100%; }
}
</style>
