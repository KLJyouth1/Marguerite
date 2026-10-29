<template>
  <div class="control-stage engine-stage">
    <header class="control-stage__header">
      <div class="control-stage__title-block">
        <h1 class="control-stage__title">{{ t('engineHealth.title') }}</h1>
        <p class="control-stage__subtitle">{{ t('engineHealth.subtitle') }}</p>
      </div>
      <div class="control-stage__actions">
        <button class="btn btn--ghost" @click="loadAll">
          <Icon name="refresh" :size="16" /><span>{{ t('engineHealth.refresh') }}</span>
        </button>
      </div>
    </header>

    <section v-if="error" class="engine-error">{{ error }}</section>

    <!-- 侧车能力降级：用户唯一会看的地方，必须具体到「丢了哪个能力」 -->
    <section v-if="sidecar?.degraded" class="engine-card engine-card--warn">
      <div class="engine-card__title">{{ t('engineHealth.sidecarDegradedTitle') }}</div>
      <div class="engine-note">{{ sidecarDegraded }}</div>
      <div v-if="sidecar.python" class="engine-note">
        {{ t('engineHealth.sidecarInterpreter', { path: sidecar.python }) }}
      </div>
    </section>

    <!-- 能力健康度 -->
    <section v-if="stats" class="engine-card">
      <div class="engine-card__title">{{ t('engineHealth.capabilitiesTitle') }}</div>
      <div class="engine-pills">
        <span class="engine-pill">{{ t('engineHealth.implemented', { done: stats.implemented, total: stats.total }) }}</span>
        <span class="engine-pill engine-pill--accent">{{ t('engineHealth.real', { count: stats.real }) }}</span>
        <span class="engine-pill">{{ t('engineHealth.partial', { count: stats.partial }) }}</span>
        <span class="engine-pill engine-pill--dim">{{ t('engineHealth.narrative', { count: stats.narrative }) }}</span>
      </div>
      <div v-if="narrative.length" class="engine-note">{{ t('engineHealth.planned', { list: narrative.join(' / ') }) }}</div>
    </section>

    <!-- 模型档案 -->
    <section v-if="models" class="engine-card">
      <div class="engine-card__title">{{ t('engineHealth.profilesTitle') }}</div>
      <div v-if="!models.ok" class="engine-note">{{ models.error }}</div>
      <table v-else class="engine-table">
        <thead>
          <tr>
            <th>{{ t('engineHealth.colProfile') }}</th>
            <th>provider / model</th>
            <th>{{ t('engineHealth.colKind') }}</th>
            <th>{{ t('engineHealth.colPrimary') }}</th>
            <th>{{ t('engineHealth.colCopilot') }}</th>
            <th></th>
          </tr>
        </thead>
        <tbody>
          <tr v-for="p in models.profiles" :key="p.id">
            <td><b>{{ p.label }}</b><code class="engine-code">{{ p.id }}</code></td>
            <td>{{ p.provider }} / {{ p.model }}</td>
            <td>{{ p.kind }}{{ p.builtin ? t('engineHealth.builtinSuffix') : '' }}</td>
            <td>{{ p.active ? t('engineHealth.currentMark') : '' }}</td>
            <td>{{ p.copilot ? t('engineHealth.copilotMark') : '' }}</td>
            <td class="engine-actions">
              <button class="btn btn--sm btn--primary" :disabled="busy || p.active" @click="setActive(p.id)">{{ t('engineHealth.setPrimary') }}</button>
              <button class="btn btn--sm btn--ghost" :disabled="busy || p.copilot" @click="setCopilot(p.id)">{{ t('engineHealth.setCopilot') }}</button>
              <button class="btn btn--sm btn--ghost" :disabled="busy || p.active" @click="removeModel(p.id)">{{ t('common.delete') }}</button>
            </td>
          </tr>
        </tbody>
      </table>
      <div v-if="models && models.ok" class="engine-actions engine-actions--footer">
        <button class="btn btn--sm btn--ghost" :disabled="busy || !models.copilot" @click="setCopilot('')">{{ t('engineHealth.disableCopilot') }}</button>
        <button class="btn btn--sm btn--ghost" :disabled="busy" @click="openModelConfig">{{ t('engineHealth.openConfig') }}</button>
      </div>
      <div class="engine-form">
        <div class="engine-form__title">{{ t('engineHealth.addProfileTitle') }}</div>
        <div class="engine-form__row">
          <input v-model="newModel.id" class="engine-input" :placeholder="t('engineHealth.idPlaceholder')" />
          <select v-model="newModel.provider" class="engine-input">
            <option value="custom">custom</option>
            <option value="openai">openai</option>
            <option value="deepseek">deepseek</option>
            <option value="anthropic">anthropic</option>
            <option value="ollama">ollama</option>
          </select>
          <input v-model="newModel.model" class="engine-input" :placeholder="t('engineHealth.modelPlaceholder')" />
          <input v-model="newModel.baseUrl" class="engine-input" :placeholder="t('engineHealth.baseUrlPlaceholder')" />
          <select v-model="newModel.kind" class="engine-input">
            <option value="cloud">cloud</option>
            <option value="custom">custom</option>
            <option value="local">{{ t('engineHealth.kindLocal') }}</option>
          </select>
          <button class="btn btn--sm btn--primary" :disabled="busy || !newModel.id || !newModel.model" @click="addModel">{{ t('engineHealth.add') }}</button>
        </div>
        <div v-if="newModel.kind === 'local'" class="engine-form__checks">
          <label class="engine-check"><input type="checkbox" v-model="newModel.weightsPresent" /> {{ t('engineHealth.weightsPresent') }}</label>
          <label class="engine-check"><input type="checkbox" v-model="newModel.runtimeSupports" /> {{ t('engineHealth.runtimeSupports') }}</label>
          <label class="engine-check"><input type="checkbox" v-model="newModel.requireGate" /> {{ t('engineHealth.requireGate') }}</label>
          <label class="engine-check"><input type="checkbox" v-model="newModel.force" /> {{ t('engineHealth.forceSkipGate') }}</label>
          <input v-model="newModel.catalogId" class="engine-input engine-input--sm" :placeholder="t('engineHealth.catalogIdPlaceholder')" />
        </div>
        <div v-if="modelError" class="engine-note">{{ modelError }}</div>
        <GateReport v-if="gateResult" :report="gateResult" />
      </div>
    </section>

    <!-- 重型运行时 -->
    <section v-if="runtimes.length" class="engine-card">
      <div class="engine-card__title">{{ t('engineHealth.runtimesTitle') }}</div>
      <div v-for="r in runtimes" :key="r.kind" class="runtime-row">
        <div class="runtime-row__head">
          <span class="runtime-badge">{{ r.status }}</span>
          <b>{{ r.label }}</b>
          <span class="runtime-engine">{{ r.engine }}</span>
        </div>
        <p class="runtime-download">{{ t('engineHealth.downloadSource') }}<a class="runtime-link" :href="firstUrl(r.download)" target="_blank" rel="noopener">{{ r.download }}</a></p>
        <p class="runtime-note">{{ r.guidance }}</p>
      </div>
    </section>

    <!-- MCP 服务器管理 -->
    <section v-if="mcp" class="engine-card">
      <div class="engine-card__title">{{ t('engineHealth.mcpTitle') }}</div>
      <div v-if="!mcp.ok" class="engine-note">{{ mcp.error }}</div>
      <table v-if="mcp.servers.length" class="engine-table">
        <thead>
          <tr>
            <th>{{ t('engineHealth.colName') }}</th>
            <th>url / command</th>
            <th>{{ t('engineHealth.colActions') }}</th>
          </tr>
        </thead>
        <tbody>
          <tr v-for="s in mcp.servers" :key="s.name">
            <td><b>{{ s.name }}</b></td>
            <td>{{ s.url || (s.command + ' ' + s.args.join(' ')) }}</td>
            <td class="engine-actions">
              <button class="btn btn--sm btn--ghost" :disabled="busy" @click="probeMcp(s.name)">{{ t('engineHealth.probe') }}</button>
              <button class="btn btn--sm btn--ghost" :disabled="busy" @click="removeMcp(s.name)">{{ t('common.delete') }}</button>
            </td>
          </tr>
        </tbody>
      </table>
      <div v-else class="engine-note">{{ t('engineHealth.mcpEmpty') }}</div>
      <div v-for="(text, name) in probeResults" :key="name" class="engine-note"><b>{{ name }}</b>：{{ text }}</div>
      <div class="engine-form">
        <div class="engine-form__title">{{ t('engineHealth.addMcpTitle') }}</div>
        <div class="engine-form__row">
          <input v-model="newMcp.name" class="engine-input" :placeholder="t('engineHealth.mcpNamePlaceholder')" />
          <input v-model="newMcp.url" class="engine-input" :placeholder="t('engineHealth.mcpUrlPlaceholder')" />
          <input v-model="newMcp.command" class="engine-input" :placeholder="t('engineHealth.mcpCommandPlaceholder')" />
          <button class="btn btn--sm btn--primary" :disabled="busy || !newMcp.name || (!newMcp.url && !newMcp.command)" @click="addMcp">{{ t('engineHealth.add') }}</button>
        </div>
      </div>
    </section>

    <!-- 任务执行质量（长时任务台账） -->
    <section v-if="longTasks.length" class="engine-card">
      <div class="engine-card__title">{{ t('engineHealth.qualityTitle') }}</div>
      <div class="engine-pills">
        <span class="engine-pill">{{ t('engineHealth.qTotal', { count: qualityStats.total }) }}</span>
        <span class="engine-pill engine-pill--accent">{{ t('engineHealth.qRunning', { count: qualityStats.running }) }}</span>
        <span class="engine-pill">{{ t('engineHealth.qCompleted', { count: qualityStats.completed, rate: qualityStats.completionRate }) }}</span>
        <span class="engine-pill">{{ t('engineHealth.qFailed', { count: qualityStats.failed }) }}</span>
        <span class="engine-pill">{{ t('engineHealth.qPaused', { count: qualityStats.paused }) }}</span>
        <span class="engine-pill engine-pill--dim">{{ t('engineHealth.qInterrupted', { count: qualityStats.interrupted }) }}</span>
        <span class="engine-pill">{{ t('engineHealth.qCheckpoints', { count: qualityStats.checkpoints }) }}</span>
        <span class="engine-pill">{{ t('engineHealth.qRetries', { count: qualityStats.retries }) }}</span>
      </div>
      <div class="engine-pills engine-pills--score">
        <span class="engine-pill engine-pill--accent">{{ t('engineHealth.qScore', { score: qualityScore }) }}</span>
        <span class="engine-pill">{{ qualityGrade }}</span>
        <span class="engine-pill engine-pill--dim">{{ t('engineHealth.qFormula') }}</span>
      </div>
      <table class="engine-table">
        <thead>
          <tr>
            <th>{{ t('engineHealth.colTask') }}</th>
            <th>{{ t('engineHealth.colStatus') }}</th>
            <th>{{ t('engineHealth.colRetries') }}</th>
            <th>{{ t('engineHealth.colCheckpoints') }}</th>
          </tr>
        </thead>
        <tbody>
          <tr v-for="task in recentTasks" :key="task.taskId">
            <td><b>{{ task.goal || task.taskId }}</b></td>
            <td>{{ task.status }}</td>
            <td>{{ task.retryCount }}/{{ task.maxRetries }}</td>
            <td>{{ task.checkpoints?.length ?? 0 }}</td>
          </tr>
        </tbody>
      </table>
    </section>

    <!-- 运行时模式 -->
    <section v-if="mode" class="engine-card">
      <div class="engine-card__title">{{ t('engineHealth.modeTitle') }}</div>
      <div class="engine-pills">
        <span class="engine-pill engine-pill--accent">{{ t('engineHealth.current', { mode: modeLabel }) }}</span>
      </div>
      <div class="engine-actions engine-actions--footer">
        <button
          v-for="k in modeKinds"
          :key="k.kind"
          class="btn btn--sm"
          :class="mode.kind === k.kind || mode.mode === k.kind ? 'btn--primary' : 'btn--ghost'"
          :disabled="busy"
          @click="switchMode(k.kind)"
        >{{ k.label }}</button>
      </div>
    </section>
  </div>
</template>

<script setup lang="ts">
import { ref, computed, onMounted } from 'vue'
import { useI18n } from 'vue-i18n'
import { sidecarDegradedText, type SidecarHealth } from '@/utils/engine/sidecarHealth'
import Icon from '@/components/Icon.vue'
import GateReport from '@/components/model/GateReport.vue'
import type { GateReport as GateReportData } from '@/components/model/gateTypes'
import { useRpcStore } from '@/stores/rpc'
import { useActiveMode, switchActiveMode } from '@/composables/useActiveMode'

const rpc = useRpcStore()
const { t } = useI18n()

interface RuntimeItem { kind: string; label: string; bundled: boolean; status: string; engine: string; download: string; guidance: string }
interface ModelProfile { id: string; label: string; provider: string; model: string; kind: string; builtin: boolean; active: boolean; copilot: boolean }
interface ModelsPayload { ok: boolean; active?: string; copilot?: string | null; profiles: ModelProfile[]; error?: string }
interface ProfilesAddResult extends ModelsPayload { code?: string; gate?: GateReportData }
interface CapStats { total: number; real: number; partial: number; implemented: number; narrative: number }
interface CapabilitiesPayload { stats?: CapStats; narrativeIds?: string[] }
interface McpServer { name: string; command: string; args: string[]; env: string[]; url?: string | null }
interface McpPayload { ok: boolean; servers: McpServer[]; error?: string }
interface ProbePayload { ok: boolean; server: string; url: string; tools?: Array<{ name: string; description: string }>; error?: string }
interface LongTask { taskId: string; goal: string; status: string; startedAtMs: number; lastHeartbeatMs: number; retryCount: number; maxRetries: number; checkpoints: Array<{ stage: string; state: unknown; timestampMs: number }> }
interface LongTasksPayload { tasks: LongTask[] }

const error = ref('')
const busy = ref(false)
const stats = ref<CapStats | null>(null)
const narrative = ref<string[]>([])
const models = ref<ModelsPayload | null>(null)
const runtimes = ref<RuntimeItem[]>([])
const mcp = ref<McpPayload | null>(null)
const newMcp = ref({ name: '', url: '', command: '' })
const newModel = ref({ id: '', provider: 'custom', model: '', baseUrl: '', kind: 'cloud', catalogId: '', weightsPresent: false, runtimeSupports: true, requireGate: true, force: false })
const probeResults = ref<Record<string, string>>({})
const modelError = ref('')
// 手写表单的 gate 报告（kind=local 复用前移的 shipped gate）；与 LocalModelsView 共享同一渲染组件。
const gateResult = ref<GateReportData | null>(null)
const longTasks = ref<LongTask[]>([])

// Share the single runtime-mode ref (upgrade-masterplan §6 阶段 D). EngineHealth
// now reflects mode switches made anywhere, instantly — no local polling.
const { activeMode } = useActiveMode()
const mode = activeMode

const modeKinds = [
  { kind: 'plan', label: 'Plan' },
  { kind: 'agent', label: 'Agent' },
  { kind: 'ensemble', label: 'Ensemble' },
  { kind: 'bloom', label: 'Bloom' },
  { kind: 'omni-transcend', label: 'OmniTranscend' },
]

const modeLabel = computed(() => {
  const m = mode.value
  if (!m) return '—'
  const kinds: Record<string, string> = { plan: 'Plan', agent: 'Agent', ensemble: 'Ensemble', bloom: 'Bloom', 'omni-transcend': 'OmniTranscend' }
  const k = m.kind ?? m.mode ?? ''
  return kinds[k] ?? k ?? '—'
})

const qualityStats = computed(() => {
  const tasks = longTasks.value
  const count = (s: string) => tasks.filter(x => x.status === s).length
  return {
    total: tasks.length,
    running: count('running') + count('resumed'),
    paused: count('paused'),
    completed: count('completed'),
    failed: count('failed'),
    interrupted: count('interrupted'),
    checkpoints: tasks.reduce((n, x) => n + (x.checkpoints?.length ?? 0), 0),
    retries: tasks.reduce((n, x) => n + (x.retryCount ?? 0), 0),
    completionRate: tasks.length ? Math.round((count('completed') / tasks.length) * 100) : 0,
  }
})

const qualityScore = computed(() => {
  const tasks = longTasks.value
  if (!tasks.length) return 0
  const done = tasks.filter(x => x.status === 'completed').length
  const fail = tasks.filter(x => x.status === 'failed').length
  const intr = tasks.filter(x => x.status === 'interrupted').length
  const retries = tasks.reduce((n, x) => n + (x.retryCount ?? 0), 0)
  const cps = tasks.reduce((n, x) => n + (x.checkpoints?.length ?? 0), 0)
  const completion = (done / tasks.length) * 100
  const penalty = ((fail + intr) / tasks.length) * 45
  const checkpointBonus = Math.min((cps / tasks.length) * 8, 12)
  const retryPenalty = Math.min(retries * 6, 18)
  const raw = completion * 0.9 + checkpointBonus - penalty * 0.6 - retryPenalty
  return Math.max(0, Math.min(100, Math.round(raw)))
})

const qualityGrade = computed(() => {
  const s = qualityScore.value
  if (s >= 90) return t('engineHealth.gradeExcellent')
  if (s >= 75) return t('engineHealth.gradeGood')
  if (s >= 60) return t('engineHealth.gradePass')
  return t('engineHealth.gradeNeedsWork')
})

const recentTasks = computed(() => [...longTasks.value].sort((a, b) => b.startedAtMs - a.startedAtMs).slice(0, 8))

function firstUrl(download: string): string {
  const m = download.match(/https?:\/\/[^\s|]+/)
  return m ? m[0] : 'https://www.google.com'
}

async function safe<T>(fn: () => Promise<T>, label?: string): Promise<T | null> {
  try { return await fn() } catch (e) {
    const msg = e instanceof Error ? e.message : String(e)
    error.value = label ? `${label}: ${msg}` : msg
    return null
  }
}

// 侧车能力预检结论（`health.sidecar`）。字段全部可选：旧网关不带这个键。
const sidecar = ref<SidecarHealth | null>(null)

// 文案逻辑抽到 `utils/engine/sidecarHealth`（纯函数、有单测）：
// 视图只负责"要不要渲染"，不负责"怎么说清影响"。
// `params` 转 `never`：vue-i18n 的 `t` 是按 messages schema 推导的重载签名，
// 这里传的是运行时拼出来的参数对象，类型层无法静态验证 —— 断言在这里，
// 而不是把整个 util 的类型放宽（那会让 6 处调用点都失去检查）。
const sidecarTranslate = (key: string, params?: Record<string, unknown>) => t(key, params as never)
const sidecarDegraded = computed(() => sidecarDegradedText(sidecar.value, sidecarTranslate))

async function loadAll() {
  error.value = ''
  busy.value = true
  try {
    // 冷加载竞态守卫：WS 仍处于 connecting 时先等连接就绪，避免把「未就绪」
    // 当成加载失败，把 `not connected (state: connecting)` 裸串渲染给用户。
    if (!rpc.isConnected) await rpc.waitForConnection(10_000)
    const [rt, mp, caps, mc, lr, hl] = await Promise.all([
      safe(() => rpc.call<RuntimeItem[]>('runtime.status', {}), t('engineHealth.loadFailed')),
      safe(() => rpc.call<ModelsPayload>('models.profiles.list', {}), t('engineHealth.loadFailed')),
      safe(() => rpc.call<CapabilitiesPayload>('plans.capabilities', {}), t('engineHealth.loadFailed')),
      safe(() => rpc.call<McpPayload>('mcp.servers.list', {}), t('engineHealth.loadFailed')),
      safe(() => rpc.call<LongTasksPayload>('long_running.list', {}), t('engineHealth.loadFailed')),
      // 侧车能力预检（缺依赖时 degraded=true）：不属于"加载失败"，取不到就保持 null。
      safe(() => rpc.call<{ sidecar?: SidecarHealth | null }>('health', {}), ''),
    ])
    if (rt) runtimes.value = Array.isArray(rt) ? rt : []
    if (mp) models.value = mp
    // `mode` is the shared activeMode ref; hydration is owned by useActiveMode
    // (App.vue initActiveMode). Do NOT overwrite it here — that would race the
    // live subscription and mask a real-time switch.
    if (caps) { stats.value = caps.stats ?? null; narrative.value = caps.narrativeIds ?? [] }
    if (mc) mcp.value = mc
    if (lr) longTasks.value = Array.isArray(lr.tasks) ? lr.tasks : []
    if (hl) sidecar.value = hl.sidecar ?? null
  } catch (e) {
    error.value = `${t('engineHealth.loadFailed')}: ${e instanceof Error ? e.message : String(e)}`
  } finally {
    busy.value = false
  }
}

async function setActive(id: string) {
  busy.value = true
  const r = await safe(() => rpc.call<ModelsPayload>('models.profiles.set_active', { id }))
  if (r) models.value = r
  busy.value = false
}

async function setCopilot(id: string) {
  busy.value = true
  const r = await safe(() => rpc.call<ModelsPayload>('models.profiles.set_copilot', { id: id || null }))
  if (r) models.value = r
  busy.value = false
}

/**
 * 打开本地模型配置文件（`models.json`）。
 *
 * 为什么分两步 RPC 而不是前端拼路径：路径真值在 Rust 侧（`Config::config_dir()`），
 * 它随运行形态变化（用户级 `~/.marguerite` / 便携版 / 测试注入），前端硬编码绝对路径
 * 必然在别人的机器上错。这里先向 Rust 要路径，再用**既有的**
 * `artifacts.open_in_system { path }` 交给系统默认程序打开（该 RPC 支持裸 path）。
 *
 * 文件不存在时**如实提示**路径与原因，不假装打开成功——`models.json` 只在第一次
 * 改动模型档案时才落盘，新装环境本来就可能还没有。
 */
async function openModelConfig() {
  busy.value = true
  try {
    const info = await safe(
      () => rpc.call<{ ok: boolean; path: string; exists: boolean }>('models.config_path', {}),
      t('engineHealth.configPathFailed'),
    )
    if (!info?.path) return
    if (!info.exists) {
      error.value = t('engineHealth.configMissing', { path: info.path })
      return
    }
    const opened = await safe(
      () => rpc.call('artifacts.open_in_system', { path: info.path }),
      t('engineHealth.openConfigFailed'),
    )
    if (opened) error.value = ''
  } finally {
    busy.value = false
  }
}

async function addModel() {
  modelError.value = ''
  busy.value = true
  const nm = newModel.value
  const params: Record<string, unknown> = {
    id: nm.id,
    provider: nm.provider,
    model: nm.model,
    baseUrl: nm.baseUrl,
    label: nm.id,
    kind: nm.kind,
  }
  // kind=local 时复用前移的 shipped gate：透传 catalogId / 权重就位 / 运行时支持 / 强制策略。
  if (nm.kind === 'local') {
    if (nm.catalogId) params.catalogId = nm.catalogId
    params.weightsPresent = nm.weightsPresent
    params.runtimeSupports = nm.runtimeSupports
    params.requireGate = nm.requireGate
    if (nm.force) params.force = true
  }
  const r = await safe(() => rpc.call<ProfilesAddResult>('models.profiles.add', params))
  if (r && r.ok) {
    models.value = r
    gateResult.value = r.gate ?? null
    newModel.value = { id: '', provider: 'custom', model: '', baseUrl: '', kind: 'cloud', catalogId: '', weightsPresent: false, runtimeSupports: true, requireGate: true, force: false }
  } else if (r) {
    // 命中 shipped gate 阻断时透明回显错误与错误码（如 gate.blocked），并渲染六要素报告。
    modelError.value = (r.error ?? t('engineHealth.addFailed')) + (r.code ? ` (${r.code})` : '')
    gateResult.value = r.gate ?? null
  } else {
    modelError.value = t('engineHealth.addFailed')
    gateResult.value = null
  }
  busy.value = false
}

async function removeModel(id: string) {
  busy.value = true
  const r = await safe(() => rpc.call<ModelsPayload>('models.profiles.remove', { id }))
  if (r) models.value = r
  busy.value = false
}

async function addMcp() {
  busy.value = true
  const r = await safe(() => rpc.call<McpPayload>('mcp.servers.add', { name: newMcp.value.name, url: newMcp.value.url, command: newMcp.value.command }))
  if (r) { mcp.value = r; newMcp.value = { name: '', url: '', command: '' } }
  busy.value = false
}

async function removeMcp(name: string) {
  busy.value = true
  const r = await safe(() => rpc.call<McpPayload>('mcp.servers.remove', { name }))
  if (r) mcp.value = r
  busy.value = false
}

async function probeMcp(name: string) {
  busy.value = true
  const r = await safe(() => rpc.call<ProbePayload>('mcp.servers.probe', { name }))
  if (r) {
    const names = (r.tools ?? []).map(tool => tool.name).join(', ')
    probeResults.value[name] = r.ok
      ? t('engineHealth.probeOk', { count: (r.tools ?? []).length, names })
      : (r.error ?? t('engineHealth.probeFailed'))
  }
  busy.value = false
}

async function switchMode(kind: string) {
  busy.value = true
  // Optimistic + broadcast-driven via the shared store; the live
  // `session.event.mode.changed` frame keeps `mode` in sync app-wide.
  await switchActiveMode(kind)
  busy.value = false
}

onMounted(loadAll)
</script>

<style scoped>
.engine-stage { padding: 24px 28px; max-width: 1100px; margin: 0 auto; }
.engine-error { margin: 16px 0; padding: 10px 14px; background: color-mix(in srgb, var(--danger) 10%, var(--card)); border: 1px solid color-mix(in srgb, var(--danger) 30%, transparent); border-radius: var(--radius-sm); color: var(--danger); font-size: 12px; }
.engine-card { margin-top: 16px; padding: 14px; background: var(--card); border: 1px solid var(--hairline); border-radius: var(--radius-md); }
/* 降级不是错误：用 warn 色描边而不是 danger 底色，避免与真正的加载失败混淆。 */
.engine-card--warn {
  border-color: color-mix(in srgb, var(--warn) 40%, var(--hairline));
  background: color-mix(in srgb, var(--warn) 6%, var(--card));
}
.engine-card--warn .engine-card__title { color: var(--warn); }
.engine-card__title { font-size: 13px; font-weight: 600; margin-bottom: 10px; }
.engine-pills { display: flex; flex-wrap: wrap; gap: 8px; }
.engine-pill { padding: 4px 10px; border-radius: 999px; background: color-mix(in srgb, var(--text) 8%, var(--card)); font-size: 12px; }
.engine-pill--accent { color: var(--accent); }
.engine-pill--dim { color: var(--text-dim); }
.engine-note { margin-top: 8px; font-size: 12px; color: var(--text-dim); }
.engine-table { width: 100%; border-collapse: collapse; font-size: 12px; }
.engine-table th, .engine-table td { text-align: left; padding: 7px 8px; border-bottom: 1px solid var(--hairline); }
.engine-code { display: block; font-family: var(--font-mono); font-size: 11px; color: var(--text-dim); }
.engine-actions { display: flex; gap: 8px; flex-wrap: wrap; align-items: center; }
.engine-actions--footer { margin-top: 12px; }
.runtime-row { border-top: 1px solid var(--hairline); padding: 10px 0; }
.runtime-row__head { display: flex; gap: 10px; align-items: center; }
.runtime-badge { padding: 2px 8px; border-radius: 999px; background: color-mix(in srgb, var(--accent) 12%, var(--card)); color: var(--accent); font-size: 11px; }
.runtime-engine { font-size: 12px; color: var(--text-dim); }
.runtime-download { margin: 6px 0 4px; font-size: 12px; word-break: break-all; }
.runtime-link { color: var(--accent); text-decoration: none; }
.runtime-note { margin: 0; font-size: 12px; color: var(--text-dim); }
.engine-form { margin-top: 12px; border-top: 1px solid var(--hairline); padding-top: 10px; }
.engine-form__title { font-size: 12px; font-weight: 600; margin-bottom: 8px; }
.engine-form__row { display: flex; gap: 8px; flex-wrap: wrap; align-items: center; }
.engine-form__checks { display: flex; flex-wrap: wrap; gap: 14px; align-items: center; margin-top: 8px; font-size: 12px; color: var(--text-dim); }
.engine-check { display: inline-flex; align-items: center; gap: 6px; cursor: pointer; }
.engine-check input { width: 14px; height: 14px; accent-color: var(--accent); }
.engine-input { flex: 1; min-width: 140px; padding: 7px 9px; border: 1px solid var(--hairline); border-radius: var(--radius-sm); background: var(--bg); color: var(--text); font-size: 12px; }
.engine-input--sm { flex: 1 1 240px; min-width: 200px; }
.engine-pills--score { margin-top: 8px; }
</style>
