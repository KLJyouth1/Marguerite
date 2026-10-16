<template>
  <div class="control-stage ops-stage">
    <header class="control-stage__header">
      <div class="control-stage__title-block">
        <h1 class="control-stage__title">Ops 运维</h1>
        <p class="control-stage__subtitle">只读运维面板：已配置 Ops 主机 / 主机状态 / 文件列表 / 日志尾部 / 进程检查</p>
      </div>
      <div class="control-stage__actions">
        <button class="btn btn--ghost" :disabled="busy" @click="loadAll">
          <Icon name="refresh" :size="16" /><span>刷新</span>
        </button>
        <button class="btn btn--ghost" :disabled="busy" @click="exportDiagnostics">
          <Icon name="copy" :size="16" /><span>{{ t('setup.ops.exportDiagnostics') }}</span>
        </button>
      </div>
    </header>

    <section v-if="error" class="ops-error">{{ error }}</section>

    <!-- 主机列表 -->
    <section class="ops-card">
      <div class="ops-card__title">Ops 主机（ops.list_hosts）</div>
      <div v-if="!hosts.length" class="ops-note">未配置 Ops 主机</div>
      <div v-else class="ops-hosts">
        <button
          v-for="h in hosts"
          :key="h.id"
          class="ops-host"
          :class="{ 'ops-host--active': selectedHost === h.id }"
          @click="selectHost(h.id)"
        >
          <span class="ops-host__id">{{ h.id }}</span>
          <span class="ops-host__addr">{{ h.address }}</span>
        </button>
      </div>
    </section>

    <!-- 主机状态 -->
    <section class="ops-card">
      <div class="ops-card__title">主机状态（ops.status）</div>
      <div v-if="!selectedHost" class="ops-note">请先选择一台 Ops 主机</div>
      <template v-else>
        <div v-if="!status" class="ops-note">尚未拉取状态，点击「刷新」或选中主机更新</div>
        <div v-else class="ops-status">
          <span class="ops-badge" :class="status.online ? 'ops-badge--online' : 'ops-badge--offline'">
            {{ status.online ? '在线' : '离线' }}
          </span>
          <code class="ops-mono">{{ status.detail }}</code>
        </div>
        <div class="ops-jump">
          <button class="btn btn--sm btn--ghost" :title="t('setup.ops.sshSession')" @click="goSshCollab">
            <Icon name="collab" :size="14" /><span>{{ t('setup.ops.sshSession') }}</span>
          </button>
          <button class="btn btn--sm btn--ghost" :title="t('setup.ops.terminal')" @click="goTerminal">
            <Icon name="terminal" :size="14" /><span>{{ t('setup.ops.terminal') }}</span>
          </button>
        </div>
      </template>
    </section>

    <!-- 文件列表 -->
    <section class="ops-card">
      <div class="ops-card__title">文件列表（ops.list_files）</div>
      <div class="ops-form">
        <input v-model="filesPath" class="ops-input" placeholder="路径（如 /home）" :disabled="!selectedHost" />
        <button class="btn btn--sm btn--primary" :disabled="busy || !selectedHost || !filesPath" @click="loadFiles">列出</button>
      </div>
      <div v-if="filesError" class="ops-note">{{ filesError }}</div>
      <div v-else-if="!files.length" class="ops-note">无文件条目</div>
      <table v-else class="ops-table">
        <thead><tr><th>路径</th><th class="ops-table__size">大小</th></tr></thead>
        <tbody>
          <tr v-for="f in files" :key="f.path">
            <td><code class="ops-mono">{{ f.path }}</code></td>
            <td class="ops-table__num">{{ humanSize(f.size_bytes) }}</td>
          </tr>
        </tbody>
      </table>
    </section>

    <!-- 日志尾部 -->
    <section class="ops-card">
      <div class="ops-card__title">日志尾部（ops.tail_logs）</div>
      <div class="ops-form">
        <input v-model="logsPath" class="ops-input" placeholder="路径（日志文件）" :disabled="!selectedHost" />
        <input v-model.number="logsMaxLines" class="ops-input ops-input--sm" type="number" placeholder="行数（默认 200）" :disabled="!selectedHost" />
        <button class="btn btn--sm btn--primary" :disabled="busy || !selectedHost || !logsPath" @click="loadLogs">查看日志</button>
      </div>
      <pre v-if="logs" class="ops-pre">{{ logs }}</pre>
    </section>

    <!-- 进程检查 -->
    <section class="ops-card">
      <div class="ops-card__title">进程检查（ops.inspect_process）</div>
      <div class="ops-form">
        <input v-model="processName" class="ops-input" placeholder="进程名（如 python）" :disabled="!selectedHost" />
        <button class="btn btn--sm btn--primary" :disabled="busy || !selectedHost || !processName" @click="loadProcess">检查进程</button>
      </div>
      <pre v-if="process" class="ops-pre">{{ process }}</pre>
    </section>
  </div>
</template>

<script setup lang="ts">
import { ref, onMounted } from 'vue'
import { useI18n } from 'vue-i18n'
import { useRouter } from 'vue-router'
import { copyTextWithFallback } from '@/utils/browser'
import { useToasts } from '@/composables/useToasts'
import Icon from '@/components/Icon.vue'
import { useRpcStore } from '@/stores/rpc'

const rpc = useRpcStore()
const { t } = useI18n()
const router = useRouter()
const { pushToast } = useToasts()

interface Host { id: string; address: string }
interface HostStatus { host_id: string; online: boolean; detail: string }
interface FileEntry { path: string; size_bytes: number }

const error = ref('')
const busy = ref(false)
const hosts = ref<Host[]>([])
const selectedHost = ref('')
const status = ref<HostStatus | null>(null)
const filesPath = ref('/home')
const files = ref<FileEntry[]>([])
const filesError = ref('')
const logsPath = ref('')
const logsMaxLines = ref(200)
const logs = ref('')
const processName = ref('')
const process = ref('')

async function safe<T>(fn: () => Promise<T>): Promise<T | null> {
  try { return await fn() } catch (e) { error.value = e instanceof Error ? e.message : String(e); return null }
}

async function loadAll() {
  error.value = ''
  busy.value = true
  const list = await safe(() => rpc.call<{ kind: string; data: Host[] }>('ops.list_hosts', {}))
  if (list) hosts.value = Array.isArray(list.data) ? list.data : []
  if (selectedHost.value) await refreshStatus()
  busy.value = false
}

function selectHost(id: string) {
  selectedHost.value = id
  status.value = null
  files.value = []
  logs.value = ''
  process.value = ''
  void refreshStatus()
}

async function refreshStatus() {
  busy.value = true
  const s = await safe(() => rpc.call<{ kind: string; data: HostStatus }>('ops.status', { host: selectedHost.value }))
  if (s) status.value = s.data
  busy.value = false
}

async function loadFiles() {
  filesError.value = ''
  busy.value = true
  const r = await safe(() =>
    rpc.call<{ kind: string; data: FileEntry[] }>('ops.list_files', { host: selectedHost.value, path: filesPath.value }),
  )
  if (r) files.value = Array.isArray(r.data) ? r.data : []
  busy.value = false
}

async function loadLogs() {
  busy.value = true
  const r = await safe(() =>
    rpc.call<{ kind: string; data: string }>('ops.tail_logs', {
      host: selectedHost.value,
      path: logsPath.value,
      max_lines: logsMaxLines.value || 200,
    }),
  )
  if (r) logs.value = r.data ?? ''
  busy.value = false
}

async function loadProcess() {
  busy.value = true
  const r = await safe(() =>
    rpc.call<{ kind: string; data: string }>('ops.inspect_process', { host: selectedHost.value, process: processName.value }),
  )
  if (r) process.value = r.data ?? ''
  busy.value = false
}

function goSshCollab() {
  router.push({ path: '/ssh-collab' }).catch(() => {})
}

function goTerminal() {
  router.push({ path: '/terminal' }).catch(() => {})
}

async function exportDiagnostics() {
  const payload = {
    surface: 'ops',
    hosts: hosts.value,
    selectedHost: selectedHost.value,
    status: status.value,
    files: files.value,
    logs: logs.value,
    process: process.value,
    error: error.value,
    collectedAt: new Date().toISOString(),
  }
  try {
    await copyTextWithFallback(JSON.stringify(payload, null, 2))
    pushToast(t('setup.ops.diagnosticsCopied'), { tone: 'ok' })
  } catch (err) {
    const detail = err instanceof Error ? err.message : String(err)
    pushToast(t('setup.ops.diagnosticsCopyFailed', { error: detail }), { tone: 'danger' })
  }
}

function humanSize(bytes: number): string {
  if (!Number.isFinite(bytes)) return '—'
  if (bytes < 1024) return `${bytes} B`
  const units = ['KB', 'MB', 'GB', 'TB']
  let v = bytes
  let i = -1
  do { v /= 1024; i++ } while (v >= 1024 && i < units.length - 1)
  return `${v.toFixed(1)} ${units[i]}`
}

onMounted(loadAll)
</script>

<style scoped>
.ops-stage { padding: 24px 28px; max-width: 1100px; margin: 0 auto; }
.ops-error { margin: 16px 0; padding: 10px 14px; background: color-mix(in srgb, var(--danger) 10%, var(--card)); border: 1px solid color-mix(in srgb, var(--danger) 30%, transparent); border-radius: var(--radius-sm); color: var(--danger); font-size: 12px; }
.ops-card { margin-top: 16px; padding: 14px; background: var(--card); border: 1px solid var(--hairline); border-radius: var(--radius-md); }
.ops-card__title { font-size: 13px; font-weight: 600; margin-bottom: 10px; }
.ops-note { margin-top: 8px; font-size: 12px; color: var(--text-dim); }
.ops-host { display: flex; align-items: center; gap: 10px; width: 100%; padding: 8px 10px; margin-bottom: 6px; border: 1px solid var(--hairline); border-radius: var(--radius-sm); background: var(--bg); cursor: pointer; font: inherit; text-align: left; transition: background var(--transition), border-color var(--transition); }
.ops-host:hover { background: var(--bg-hover); }
.ops-host--active { border-color: var(--accent); background: color-mix(in srgb, var(--accent) 8%, var(--card)); }
.ops-host__id { font-weight: 600; font-size: 12px; }
.ops-host__addr { font-family: var(--font-mono); font-size: 11px; color: var(--text-dim); }
.ops-status { display: flex; align-items: flex-start; gap: 10px; }
.ops-badge { padding: 3px 10px; border-radius: 999px; font-size: 11px; font-weight: 600; font-variant-numeric: tabular-nums; }
.ops-badge--online { background: color-mix(in srgb, var(--ok) 14%, var(--card)); color: var(--ok); }
.ops-badge--offline { background: color-mix(in srgb, var(--danger) 14%, var(--card)); color: var(--danger); }
.ops-mono { font-family: var(--font-mono); font-size: 11px; word-break: break-all; color: var(--text); }
.ops-form { display: flex; gap: 8px; flex-wrap: wrap; align-items: center; }
.ops-input { flex: 1 1 220px; min-width: 180px; padding: 7px 9px; border: 1px solid var(--hairline); border-radius: var(--radius-sm); background: var(--bg); color: var(--text); font-size: 12px; }
.ops-input--sm { flex: 1 1 120px; min-width: 100px; max-width: 160px; }
.ops-table { width: 100%; border-collapse: collapse; font-size: 12px; margin-top: 10px; }
.ops-table th, .ops-table td { text-align: left; padding: 7px 8px; border-bottom: 1px solid var(--hairline); }
.ops-table__size { width: 90px; }
.ops-table__num { font-variant-numeric: tabular-nums; text-align: right; }
.ops-pre { margin-top: 10px; padding: 12px; max-height: 360px; overflow: auto; background: var(--bg); border: 1px solid var(--hairline); border-radius: var(--radius-sm); font-family: var(--font-mono); font-size: 11px; line-height: 1.5; white-space: pre-wrap; word-break: break-all; color: var(--text); }
.ops-jump { display: flex; gap: 8px; margin-top: 10px; }
</style>
