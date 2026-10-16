<template>
  <div class="control-stage engine-stage">
    <header class="control-stage__header">
      <div class="control-stage__title-block">
        <h1 class="control-stage__title">引擎健康 · 模型 · 模式</h1>
        <p class="control-stage__subtitle">能力透明化：能力健康度 / Rust 模型档案 / 重型运行时 / 运行时模式</p>
      </div>
      <div class="control-stage__actions">
        <button class="btn btn--ghost" @click="loadAll">
          <Icon name="refresh" :size="16" /><span>刷新</span>
        </button>
      </div>
    </header>

    <section v-if="error" class="engine-error">{{ error }}</section>

    <!-- 能力健康度 -->
    <section v-if="stats" class="engine-card">
      <div class="engine-card__title">Agent 能力健康度（plans.capabilities）</div>
      <div class="engine-pills">
        <span class="engine-pill">已实装 <b>{{ stats.implemented }}</b>/{{ stats.total }}</span>
        <span class="engine-pill engine-pill--accent">真货 {{ stats.real }}</span>
        <span class="engine-pill">部分 {{ stats.partial }}</span>
        <span class="engine-pill engine-pill--dim">纯叙事 {{ stats.narrative }}</span>
      </div>
      <div v-if="narrative.length" class="engine-note">规划中（用户安装后翻转）：{{ narrative.join(' / ') }}</div>
    </section>

    <!-- 模型档案 -->
    <section v-if="models" class="engine-card">
      <div class="engine-card__title">Rust 模型档案（models.profiles.list）</div>
      <div v-if="!models.ok" class="engine-note">{{ models.error }}</div>
      <table v-else class="engine-table">
        <thead>
          <tr><th>档案</th><th>provider / model</th><th>类别</th><th>主模型</th><th>副模型</th><th></th></tr>
        </thead>
        <tbody>
          <tr v-for="p in models.profiles" :key="p.id">
            <td><b>{{ p.label }}</b><code class="engine-code">{{ p.id }}</code></td>
            <td>{{ p.provider }} / {{ p.model }}</td>
            <td>{{ p.kind }}{{ p.builtin ? ' · 内置' : '' }}</td>
            <td>{{ p.active ? '★ 当前' : '' }}</td>
            <td>{{ p.copilot ? '◇ 当前' : '' }}</td>
            <td class="engine-actions">
              <button class="btn btn--sm btn--primary" :disabled="busy || p.active" @click="setActive(p.id)">设为主模型</button>
              <button class="btn btn--sm btn--ghost" :disabled="busy || p.copilot" @click="setCopilot(p.id)">设为副模型</button>
              <button class="btn btn--sm btn--ghost" :disabled="busy || p.active" @click="removeModel(p.id)">删除</button>
            </td>
          </tr>
        </tbody>
      </table>
      <div v-if="models && models.ok" class="engine-actions engine-actions--footer">
        <button class="btn btn--sm btn--ghost" :disabled="busy || !models.copilot" @click="setCopilot('')">关闭副模型</button>
        <button class="btn btn--sm btn--ghost" :disabled="busy" @click="openModelConfig">打开模型配置文件</button>
      </div>
      <div class="engine-form">
        <div class="engine-form__title">新增自定义模型档案</div>
        <div class="engine-form__row">
          <input v-model="newModel.id" class="engine-input" placeholder="id（如 my-llama）" />
          <select v-model="newModel.provider" class="engine-input">
            <option value="custom">custom</option>
            <option value="openai">openai</option>
            <option value="deepseek">deepseek</option>
            <option value="anthropic">anthropic</option>
            <option value="ollama">ollama</option>
          </select>
          <input v-model="newModel.model" class="engine-input" placeholder="model（如 gpt-4o-mini / qwen3.8:27b）" />
          <input v-model="newModel.baseUrl" class="engine-input" placeholder="baseUrl（可选，内置 provider 可留空）" />
          <select v-model="newModel.kind" class="engine-input">
            <option value="cloud">cloud</option>
            <option value="custom">custom</option>
            <option value="local">local（过 shipped gate）</option>
          </select>
          <button class="btn btn--sm btn--primary" :disabled="busy || !newModel.id || !newModel.model" @click="addModel">新增</button>
        </div>
        <div v-if="newModel.kind === 'local'" class="engine-form__checks">
          <label class="engine-check"><input type="checkbox" v-model="newModel.weightsPresent" /> 权重已实际下载</label>
          <label class="engine-check"><input type="checkbox" v-model="newModel.runtimeSupports" /> 本机运行时支持</label>
          <label class="engine-check"><input type="checkbox" v-model="newModel.requireGate" /> 强制 gate 校验</label>
          <label class="engine-check"><input type="checkbox" v-model="newModel.force" /> 强制跳过 gate</label>
          <input v-model="newModel.catalogId" class="engine-input engine-input--sm" placeholder="catalogId（可选，精确命中聚合目录）" />
        </div>
        <div v-if="modelError" class="engine-note">{{ modelError }}</div>
        <GateReport v-if="gateResult" :report="gateResult" />
      </div>
    </section>

    <!-- 重型运行时 -->
    <section v-if="runtimes.length" class="engine-card">
      <div class="engine-card__title">重型运行时（runtime.status）</div>
      <div v-for="r in runtimes" :key="r.kind" class="runtime-row">
        <div class="runtime-row__head">
          <span class="runtime-badge">{{ r.status }}</span>
          <b>{{ r.label }}</b>
          <span class="runtime-engine">{{ r.engine }}</span>
        </div>
        <p class="runtime-download">下载源：<a class="runtime-link" :href="firstUrl(r.download)" target="_blank" rel="noopener">{{ r.download }}</a></p>
        <p class="runtime-note">{{ r.guidance }}</p>
      </div>
    </section>

    <!-- MCP 服务器管理 -->
    <section v-if="mcp" class="engine-card">
      <div class="engine-card__title">MCP 服务器（mcp.servers.list / add / remove / probe）</div>
      <div v-if="!mcp.ok" class="engine-note">{{ mcp.error }}</div>
      <table v-if="mcp.servers.length" class="engine-table">
        <thead><tr><th>名称</th><th>url / command</th><th>动作</th></tr></thead>
        <tbody>
          <tr v-for="s in mcp.servers" :key="s.name">
            <td><b>{{ s.name }}</b></td>
            <td>{{ s.url || (s.command + ' ' + s.args.join(' ')) }}</td>
            <td class="engine-actions">
              <button class="btn btn--sm btn--ghost" :disabled="busy" @click="probeMcp(s.name)">探测</button>
              <button class="btn btn--sm btn--ghost" :disabled="busy" @click="removeMcp(s.name)">删除</button>
            </td>
          </tr>
        </tbody>
      </table>
      <div v-else class="engine-note">未配置 MCP 服务器（支持 URL 型 HTTP MCP 与 command 型 stdio MCP）</div>
      <div v-for="(text, name) in probeResults" :key="name" class="engine-note"><b>{{ name }}</b>：{{ text }}</div>
      <div class="engine-form">
        <div class="engine-form__title">新增 MCP 服务器（URL 或 command）</div>
        <div class="engine-form__row">
          <input v-model="newMcp.name" class="engine-input" placeholder="名称" />
          <input v-model="newMcp.url" class="engine-input" placeholder="http://127.0.0.1:18799/mcp（可选）" />
          <input v-model="newMcp.command" class="engine-input" placeholder="command（如 npx -y @modelcontextprotocol/server-filesystem）" />
          <button class="btn btn--sm btn--primary" :disabled="busy || !newMcp.name || (!newMcp.url && !newMcp.command)" @click="addMcp">新增</button>
        </div>
      </div>
    </section>

    <!-- 任务执行质量（长时任务台账） -->
    <section v-if="longTasks.length" class="engine-card">
      <div class="engine-card__title">Omni-Agent 质量评分卡（long_running.list）</div>
      <div class="engine-pills">
        <span class="engine-pill">总数 {{ qualityStats.total }}</span>
        <span class="engine-pill engine-pill--accent">进行中 {{ qualityStats.running }}</span>
        <span class="engine-pill">完成 {{ qualityStats.completed }}（{{ qualityStats.completionRate }}%）</span>
        <span class="engine-pill">失败 {{ qualityStats.failed }}</span>
        <span class="engine-pill">暂停 {{ qualityStats.paused }}</span>
        <span class="engine-pill engine-pill--dim">中断 {{ qualityStats.interrupted }}</span>
        <span class="engine-pill">检查点 {{ qualityStats.checkpoints }}</span>
        <span class="engine-pill">重试 {{ qualityStats.retries }}</span>
      </div>
      <div class="engine-pills engine-pills--score">
        <span class="engine-pill engine-pill--accent">Omni 质量评分 {{ qualityScore }}</span>
        <span class="engine-pill">{{ qualityGrade }}</span>
        <span class="engine-pill engine-pill--dim">评分 = 完成率×0.9 + 检查点奖励 − 失败/中断惩罚 − 重试惩罚（0–100）</span>
      </div>
      <table class="engine-table">
        <thead><tr><th>任务</th><th>状态</th><th>重试</th><th>检查点</th></tr></thead>
        <tbody>
          <tr v-for="t in recentTasks" :key="t.taskId">
            <td><b>{{ t.goal || t.taskId }}</b></td>
            <td>{{ t.status }}</td>
            <td>{{ t.retryCount }}/{{ t.maxRetries }}</td>
            <td>{{ t.checkpoints?.length ?? 0 }}</td>
          </tr>
        </tbody>
      </table>
    </section>

    <!-- 运行时模式 -->
    <section v-if="mode" class="engine-card">
      <div class="engine-card__title">运行时模式（mode.get / mode.switch）</div>
      <div class="engine-pills">
        <span class="engine-pill engine-pill--accent">当前：{{ modeLabel }}</span>
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
import Icon from '@/components/Icon.vue'
import GateReport from '@/components/model/GateReport.vue'
import type { GateReport as GateReportData } from '@/components/model/gateTypes'
import { useRpcStore } from '@/stores/rpc'
import { useActiveMode, switchActiveMode } from '@/composables/useActiveMode'

const rpc = useRpcStore()

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
  const t = longTasks.value
  const count = (s: string) => t.filter(x => x.status === s).length
  return {
    total: t.length,
    running: count('running') + count('resumed'),
    paused: count('paused'),
    completed: count('completed'),
    failed: count('failed'),
    interrupted: count('interrupted'),
    checkpoints: t.reduce((n, x) => n + (x.checkpoints?.length ?? 0), 0),
    retries: t.reduce((n, x) => n + (x.retryCount ?? 0), 0),
    completionRate: t.length ? Math.round((count('completed') / t.length) * 100) : 0,
  }
})

const qualityScore = computed(() => {
  const t = longTasks.value
  if (!t.length) return 0
  const done = t.filter(x => x.status === 'completed').length
  const fail = t.filter(x => x.status === 'failed').length
  const intr = t.filter(x => x.status === 'interrupted').length
  const retries = t.reduce((n, x) => n + (x.retryCount ?? 0), 0)
  const cps = t.reduce((n, x) => n + (x.checkpoints?.length ?? 0), 0)
  const completion = (done / t.length) * 100
  const penalty = ((fail + intr) / t.length) * 45
  const checkpointBonus = Math.min((cps / t.length) * 8, 12)
  const retryPenalty = Math.min(retries * 6, 18)
  const raw = completion * 0.9 + checkpointBonus - penalty * 0.6 - retryPenalty
  return Math.max(0, Math.min(100, Math.round(raw)))
})

const qualityGrade = computed(() => {
  const s = qualityScore.value
  if (s >= 90) return '优'
  if (s >= 75) return '良'
  if (s >= 60) return '合格'
  return '待改进'
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

async function loadAll() {
  error.value = ''
  busy.value = true
  try {
    // 冷加载竞态守卫：WS 仍处于 connecting 时先等连接就绪，避免把「未就绪」
    // 当成加载失败，把 `not connected (state: connecting)` 裸串渲染给用户。
    if (!rpc.isConnected) await rpc.waitForConnection(10_000)
    const [rt, mp, caps, mc, lr] = await Promise.all([
      safe(() => rpc.call<RuntimeItem[]>('runtime.status', {}), '加载失败'),
      safe(() => rpc.call<ModelsPayload>('models.profiles.list', {}), '加载失败'),
      safe(() => rpc.call<CapabilitiesPayload>('plans.capabilities', {}), '加载失败'),
      safe(() => rpc.call<McpPayload>('mcp.servers.list', {}), '加载失败'),
      safe(() => rpc.call<LongTasksPayload>('long_running.list', {}), '加载失败'),
    ])
    if (rt) runtimes.value = Array.isArray(rt) ? rt : []
    if (mp) models.value = mp
    // `mode` is the shared activeMode ref; hydration is owned by useActiveMode
    // (App.vue initActiveMode). Do NOT overwrite it here — that would race the
    // live subscription and mask a real-time switch.
    if (caps) { stats.value = caps.stats ?? null; narrative.value = caps.narrativeIds ?? [] }
    if (mc) mcp.value = mc
    if (lr) longTasks.value = Array.isArray(lr.tasks) ? lr.tasks : []
  } catch (e) {
    error.value = `加载失败: ${e instanceof Error ? e.message : String(e)}`
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
      '取模型配置文件路径失败',
    )
    if (!info?.path) return
    if (!info.exists) {
      error.value = `模型配置文件尚不存在（首次修改模型档案后才会生成）：${info.path}`
      return
    }
    const opened = await safe(
      () => rpc.call('artifacts.open_in_system', { path: info.path }),
      '打开模型配置文件失败',
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
    modelError.value = (r.error ?? '新增失败') + (r.code ? `（${r.code}）` : '')
    gateResult.value = r.gate ?? null
  } else {
    modelError.value = '新增失败'
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
    const names = (r.tools ?? []).map(t => t.name).join(', ')
    probeResults.value[name] = r.ok ? '连通：' + ((r.tools ?? []).length) + ' 个工具（' + names + '）' : (r.error ?? '探测失败')
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
