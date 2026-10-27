<template>
  <div class="control-stage lm-stage">
    <header class="control-stage__header">
      <div class="control-stage__title-block">
        <h1 class="control-stage__title">{{ t('localModels.title') }}</h1>
        <p class="control-stage__subtitle">{{ t('localModels.subtitle') }}</p>
      </div>
      <div class="control-stage__actions">
        <button class="btn btn--ghost" @click="refreshAll"><Icon name="refresh" :size="16" /><span>{{ t('localModels.refresh') }}</span></button>
      </div>
    </header>

    <section v-if="error" class="lm-error">{{ error }}</section>

    <!-- ── 系统概览 + 成本时段 ── -->
    <div class="lm-system">
      <div class="lm-sys-card">
        <div class="lm-sys-title">💻 {{ t('localModels.sysTitle') }}</div>
        <div class="lm-sys-rows">
          <div><span>{{ t('localModels.sysOs') }}</span><b>{{ sys.os || '—' }}</b></div>
          <div><span>{{ t('localModels.sysRam') }}</span><b>{{ ramLabel }}</b></div>
          <div><span>{{ t('localModels.sysCores') }}</span><b>{{ sys.cores }}</b></div>
          <div>
            <span>{{ t('localModels.sysGpu') }}</span>
            <b :class="gpuKnown ? 'lm-ok' : 'lm-muted'" :title="gpuTitle">
              {{ gpuLabel }}<span v-if="!gpuKnown" class="lm-gpu-hint"> · {{ gpuHint }}</span>
            </b>
          </div>
          <div><span>{{ t('localModels.sysTier') }}</span><b class="lm-tier" :title="tierRaw" :data-tier="tierRaw">{{ tierLabel }}</b></div>
          <div><span>{{ t('localModels.sysOllama') }}</span><b :class="ollamaStatus.tone === 'on' ? 'lm-ok' : (ollamaStatus.tone === 'warn' ? 'lm-warn' : 'lm-muted')" :title="ollamaStatus.hint">{{ ollamaStatus.label }}</b></div>
        </div>
      </div>
      <div class="lm-sys-card">
        <div class="lm-sys-title">💸 {{ t('localModels.costTitle') }}</div>
        <template v-if="cost">
          <div class="lm-cost-big">≈ ${{ cost.estimated_cost_usd }}</div>
          <div class="lm-cost-rows">
            <div><span>{{ t('localModels.currentPeriod') }}</span><b :class="cost.period === 'off-peak' ? 'lm-ok' : 'lm-warn'">{{ cost.period === 'off-peak' ? t('localModels.offPeak') : t('localModels.peak') }}</b></div>
            <div><span>{{ t('localModels.input') }}</span><b>${{ cost.price_input_per_mtok }}/M</b></div>
            <div><span>{{ t('localModels.output') }}</span><b>${{ cost.price_output_per_mtok }}/M</b></div>
            <div class="lm-cost-advice">{{ cost.advice }}</div>
          </div>
        </template>
        <div v-else class="lm-muted">{{ t('localModels.loading') }}</div>
      </div>
      <div class="lm-sys-card">
        <div class="lm-sys-title">🧭 {{ t('localModels.deployedTitle') }}</div>
        <div v-if="localProfiles.length === 0" class="lm-muted">{{ t('localModels.deployedEmpty') }}</div>
        <div v-for="p in localProfiles" :key="p.id" class="lm-profile">
          <span class="lm-profile__tag">{{ t('localModels.localTag') }}</span>
          <span>{{ p.label }}</span>
          <span class="lm-muted">{{ p.model }}</span>
          <b v-if="p.active" class="lm-ok">{{ t('localModels.currentPrimary') }}</b>
        </div>
        <div class="lm-candidates">
          <div class="lm-candidates__title">{{ t('localModels.routingCandidatesTitle') }}</div>
          <div v-if="candidateStore.ids.length === 0" class="lm-muted">{{ t('localModels.routingCandidatesEmpty') }}</div>
          <div v-else class="lm-candidates__chips">
            <span v-for="id in candidateStore.ids" :key="id" class="lm-candidates__chip" :title="id">{{ id }}</span>
          </div>
        </div>
      </div>
    </div>

    <!-- ── 本机 Ollama 在线模型（live tags，一键加入路由） ── -->
    <section class="lm-section">
      <div class="lm-section__head">
        <h3>🦙 {{ t('localModels.ollamaLiveTitle') }}</h3>
        <button class="btn btn--ghost btn--sm" :disabled="ollamaLoading" @click="loadOllamaLive">
          <Icon name="refresh" :size="14" /><span>{{ ollamaLoading ? t('localModels.ollamaLiveScanning') : t('localModels.refresh') }}</span>
        </button>
      </div>
      <div class="lm-ollama-bar">
        <span
          class="lm-ollama-badge"
          :class="`lm-ollama-badge--${ollamaStatus.tone}`"
        >{{ ollamaStatus.label }}</span>
        <span class="lm-muted">{{ ollamaLive?.base_url || '—' }}</span>
      </div>
      <p v-if="ollamaStatus.hint" class="lm-muted">{{ ollamaStatus.hint }}</p>
      <div v-if="ollamaLoading && !ollamaLive" class="lm-muted">{{ t('localModels.ollamaLiveScanning') }}</div>
      <div v-else-if="ollamaLive && ollamaLive.online && ollamaLive.models.length === 0" class="lm-muted">
        {{ t('localModels.ollamaLiveEmpty') }}
      </div>
      <div class="lm-ollama-grid">
        <div v-for="m in ollamaLive?.models ?? []" :key="m.name" class="lm-ollama-card">
          <div class="lm-ollama-name"><b>{{ m.name }}</b></div>
          <div class="lm-ollama-meta lm-muted">
            <span>{{ m.family || '—' }}</span>
            <span>{{ m.parameter_size || '—' }}</span>
            <span>{{ m.quantization || '—' }}</span>
            <span>{{ formatGb(m.size_gb) }}</span>
          </div>
          <div class="lm-ollama-actions">
            <button v-if="activeOllamaModel === m.name" class="btn btn--sm" disabled>
              {{ t('localModels.ollamaLiveActive') }}
            </button>
            <button v-else-if="deployedOllamaModels.has(m.name)" class="btn btn--secondary btn--sm" disabled>
              {{ t('localModels.ollamaLiveAdded') }}
            </button>
            <button
              v-else
              class="btn btn--primary btn--sm"
              :disabled="ollamaDeploying === m.name"
              @click="deployOllama(m)"
            >{{ ollamaDeploying === m.name ? t('localModels.ollamaLiveDeploying') : t('localModels.ollamaLiveAdd') }}</button>
          </div>
        </div>
      </div>
      <p v-if="ollamaMsg" class="lm-ollama-msg">{{ ollamaMsg }}</p>
    </section>

    <!-- ── 智能推荐 ── -->
    <section class="lm-section">
      <div class="lm-section__head">
        <h3>🤖 {{ t('localModels.recommendTitle') }}</h3>
        <label class="lm-intent">
          {{ t('localModels.intentLabel') }}
          <select v-model="intent" @change="runRecommend">
            <option value="general">{{ t('localModels.intentGeneral') }}</option>
            <option value="coding">{{ t('localModels.intentCoding') }}</option>
            <option value="reasoning">{{ t('localModels.intentReasoning') }}</option>
            <option value="chat">{{ t('localModels.intentChat') }}</option>
            <option value="multimodal">{{ t('localModels.intentMultimodal') }}</option>
          </select>
        </label>
      </div>
      <p v-if="recommend?.reason" class="lm-reason">{{ recommend.reason }}</p>
      <div class="lm-picks">
        <div v-for="p in recommend?.picks || []" :key="p.id" class="lm-pick" :class="{ 'is-managed': managedIds.has(p.id) }">
          <div class="lm-pick__name">
            <b>{{ p.label }}</b>
            <span class="lm-pick__intent">{{ p.intent }}</span>
          </div>
          <div class="lm-pick__meta">{{ p.size_gb }}GB · {{ p.context }} · {{ p.shipped_gate.license }}</div>
          <div class="lm-pick__actions">
            <button class="btn btn--primary btn--sm" @click="download(p.id)">{{ t('localModels.download') }}</button>
            <button class="btn btn--secondary btn--sm" @click="deploy(p.id)">{{ t('localModels.deployToRouter') }}</button>
          </div>
        </div>
      </div>
    </section>

    <!-- ── 目录 ── -->
    <section class="lm-section">
      <div class="lm-section__head">
        <h3>📦 {{ t('localModels.catalogTitle', { count: catalog.length }) }}</h3>
        <button class="btn btn--ghost btn--sm" :disabled="!!gateBusy" @click="checkAllGates">{{ t('localModels.checkAllGates') }}</button>
      </div>
      <div class="lm-gate-opts">
        <label><input type="checkbox" v-model="gateWeights" /> {{ t('localModels.gateWeights') }}</label>
        <label><input type="checkbox" v-model="gateRuntime" /> {{ t('localModels.gateRuntime') }}</label>
        <span class="lm-muted">{{ t('localModels.gateHint') }}</span>
      </div>
      <div class="lm-catalog">
        <div v-for="e in catalog" :key="e.id" class="lm-entry" :class="{ 'is-managed': managedIds.has(e.id) }">
          <div class="lm-entry__head">
            <b>{{ e.label }}</b>
            <span class="lm-entry__intent">{{ e.intent }}</span>
          </div>
          <div class="lm-entry__meta">
            <span>{{ e.family }}</span><span>{{ e.size_gb }}GB</span><span>{{ e.context }}</span>
            <span>{{ t('localModels.minRam', { ram: e.min_ram_gb }) }}</span>
          </div>
          <div class="lm-gate">
            <span v-for="(v, k) in gateBadges(e)" :key="k" :class="v.ok ? 'lm-gate__ok' : 'lm-gate__bad'">{{ v.label }}</span>
          </div>
          <div class="lm-entry__eval">📊 {{ e.shipped_gate.eval }}</div>
          <div class="lm-entry__src lm-muted">Ollama: {{ e.ollama_tag || '—' }} · HF: {{ e.hf_repo }}</div>
          <div class="lm-entry__actions">
            <button class="btn btn--primary btn--sm" @click="download(e.id)">{{ t('localModels.download') }}</button>
            <button class="btn btn--secondary btn--sm" @click="deploy(e.id)">{{ t('localModels.deploy') }}</button>
            <button
              class="btn btn--ghost btn--sm"
              :disabled="gateBusy === e.id"
              @click="checkGate(e.id)"
            >{{ gateBusy === e.id ? t('localModels.gateChecking') : t('localModels.gateCheck') }}</button>
            <button
              v-if="managedIds.has(e.id)"
              class="btn btn--ghost btn--sm"
              @click="toggleStart(e.id)"
            >{{ startedIds.has(e.id) ? t('localModels.stop') : t('localModels.start') }}</button>
            <button v-if="managedIds.has(e.id)" class="btn btn--ghost btn--sm lm-danger" @click="remove(e.id)">{{ t('localModels.remove') }}</button>
          </div>
          <!-- 未校验时必须显式传 null：GateReport 的 prop 是 `GateReport | null`，
               传 undefined 会在每次渲染时刷 Vue 的 prop 类型告警。 -->
          <GateReport :report="gateReports[e.id] ?? null" />
        </div>
      </div>
    </section>

    <!-- ── 训练与微调 ── -->
    <section class="lm-section">
      <div class="lm-section__head">
        <h3>🧪 {{ t('localModels.finetuneTitle') }}</h3>
        <span class="lm-muted">{{ t('localModels.finetuneHint') }}</span>
      </div>
      <div class="lm-finetune">
        <div class="lm-ft-form">
          <label>{{ t('localModels.baseModel') }}
            <select v-model="ftForm.baseId">
              <option value="" disabled>{{ t('localModels.selectBase') }}</option>
              <option v-for="b in baseOptions" :key="b.id" :value="b.id">{{ b.label }}</option>
            </select>
          </label>
          <label>{{ t('localModels.datasetPath') }}
            <input v-model="ftForm.dataset" placeholder="/path/to/dataset.jsonl" />
          </label>
          <label>{{ t('localModels.method') }}
            <select v-model="ftForm.method">
              <option value="lora">LoRA</option>
              <option value="qlora">QLoRA</option>
              <option value="full">Full</option>
            </select>
          </label>
          <label>{{ t('localModels.epochs') }}
            <input v-model.number="ftForm.epochs" type="number" min="1" max="50" />
          </label>
          <label>{{ t('localModels.learningRate') }}
            <input v-model.number="ftForm.lr" type="number" step="0.0001" min="0" />
          </label>
          <button class="btn btn--primary btn--sm" :disabled="ftBusy" @click="startFinetune">{{ t('localModels.startFinetune') }}</button>
        </div>
        <p v-if="ftMsg" class="lm-ft-msg">{{ ftMsg }}</p>
        <div v-if="ftJobs.length === 0" class="lm-muted">{{ t('localModels.noFinetuneJobs') }}</div>
        <div v-for="j in ftJobs" :key="j.id" class="lm-ft-job">
          <span class="lm-ft-job__id">{{ j.id }}</span>
          <span class="lm-muted">{{ j.base_label }} · {{ j.method }} · {{ j.epochs }}ep</span>
          <span :class="j.needs_backend ? 'lm-warn' : (j.status === 'running' ? 'lm-ok' : 'lm-muted')">
            {{ j.needs_backend ? t('localModels.needsBackend') : j.status }}
          </span>
          <span v-if="j.needs_backend" class="lm-muted">{{ t('localModels.needsBackendEnv') }}</span>
          <button
            v-if="['queued', 'running'].includes(j.status)"
            class="btn btn--ghost btn--sm lm-danger"
            @click="cancelFinetune(j.id)"
          >{{ t('localModels.cancel') }}</button>
        </div>
      </div>
    </section>
  </div>
</template>

<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import Icon from '@/components/Icon.vue'
import GateReport from '@/components/model/GateReport.vue'
import type { GateReport as GateReportData } from '@/components/model/gateTypes'
import { useRpcStore } from '@/stores/rpc'
import { useCandidateModelsStore } from '@/stores/candidateModels'

const rpc = useRpcStore()
const { t } = useI18n()
const candidateStore = useCandidateModelsStore()
let candidateUnsub: (() => void) | null = null

interface ShippedGate { weights: boolean; license: string; eval: string; pricing: string; quantized: boolean; reproducible: boolean }
interface CatalogEntry {
  id: string; label: string; family: string; intent: string
  ollama_tag: string; hf_repo: string; msc_repo: string
  size_gb: number; min_ram_gb: number; context: string
  shipped_gate: ShippedGate
}
interface Recommend { tier: string; max_billions: number; reason: string; picks: CatalogEntry[] }
interface Status { ollama_present: boolean; managed: Managed[]; local_profiles: LocalProfile[] }
interface Managed { id: string; label: string; status: string; deployed: boolean; profile_id: string; started: boolean }
interface LocalProfile { id: string; label: string; model: string; base_url: string; active: boolean }
/** `models.local.system` 的原始返回（Rust 端为 snake_case）。 */
interface HostSpec { os: string; ram_gb: number; cores: number; gpu: boolean; gpu_name: string }

// ── 本机 Ollama 在线模型（models.local.ollama 枚举 live tags）──
interface OllamaLiveModel {
  name: string
  model?: string
  digest?: string
  size_bytes?: number
  size_gb: number
  family?: string
  families?: string[]
  parameter_size?: string
  quantization?: string
  modified_at?: string
}
interface OllamaLiveResult {
  ok: boolean
  online: boolean
  base_url: string
  models: OllamaLiveModel[]
  error?: string
}

const error = ref('')
const catalog = ref<CatalogEntry[]>([])
const recommend = ref<Recommend | null>(null)
const status = ref<Status | null>(null)
const cost = ref<any>(null)
const intent = ref('general')

// ── 训练与微调（Finetune）状态 ──
const ftJobs = ref<any[]>([])
const ftForm = ref({ baseId: '', dataset: '', method: 'lora', epochs: 3, lr: 0.0002 })
const ftBusy = ref(false)
const ftMsg = ref('')
const baseOptions = computed(() => {
  const cats = catalog.value.map(e => ({ id: e.id, label: e.label }))
  const mgd = (status.value?.managed ?? []).map(m => ({ id: m.id, label: m.label }))
  const seen = new Set<string>()
  const out: { id: string; label: string }[] = []
  for (const x of [...cats, ...mgd]) {
    if (!seen.has(x.id)) { seen.add(x.id); out.push(x) }
  }
  return out
})

// 字段名以后端 `models.local.system` 的真实返回为准：Rust 端输出 snake_case
// （ram_gb / gpu_name）。此前这里按 camelCase 读取，取到 undefined，
// 导致内存只剩单位、GPU 只显示「检测到」。
const sys = ref<HostSpec>({
  os: '', ram_gb: 0, cores: 0, gpu: false, gpu_name: '',
})
const managedIds = computed(() => new Set(status.value?.managed?.map(m => m.id) ?? []))
const startedIds = computed(() => new Set(status.value?.managed?.filter(m => m.started).map(m => m.id) ?? []))
const localProfiles = computed(() => status.value?.local_profiles ?? [])

/** 内存：无有效数值时显示占位符，绝不输出孤零零的单位。 */
function formatRamGb(v: number | undefined): string {
  return typeof v === 'number' && Number.isFinite(v) && v > 0 ? `${Math.round(v)} GB` : '—'
}
const ramLabel = computed(() => formatRamGb(sys.value.ram_gb))

const gpuKnown = computed(() => sys.value.gpu && sys.value.gpu_name.trim().length > 0)
const gpuLabel = computed(() => (gpuKnown.value ? sys.value.gpu_name.trim() : t('localModels.sysGpuNotFound')))
const gpuHint = computed(() => t('localModels.sysGpuNotFoundHint'))
const gpuTitle = computed(() => (gpuKnown.value ? sys.value.gpu_name.trim() : gpuHint.value))

// 后端 tier 是稳定枚举（edge/laptop/workstation/server），UI 必须本地化展示；
// 原始值保留在 title/data-tier 上，便于排查与自动化断言。
const TIER_KEYS: Record<string, string> = {
  edge: 'tierEdge',
  laptop: 'tierLaptop',
  workstation: 'tierWorkstation',
  server: 'tierServer',
}
const tierRaw = computed(() => recommend.value?.tier || '')
const tierLabel = computed(() => {
  const tier = tierRaw.value
  if (!tier) return '—'
  const key = TIER_KEYS[tier]
  return key ? t(`localModels.${key}`) : t('localModels.tierUnknown', { tier })
})

// ── Ollama live 模型：部署档案按模型名对账，决定按钮是「加入 / 已加入 / 使用中」──
const ollamaLive = ref<OllamaLiveResult | null>(null)
const ollamaLoading = ref(false)
const ollamaDeploying = ref<string | null>(null)
const ollamaMsg = ref('')
const deployedOllamaModels = computed(() => new Set(localProfiles.value.map(p => p.model)))
const activeOllamaModel = computed(() => localProfiles.value.find(p => p.active)?.model ?? '')

// 「是否安装」与「服务是否在跑」来自两个后端事实：
// - `models.local.status.ollama_present`：PATH 上是否存在 ollama(.exe)（安装事实）
// - `models.local.ollama.online`：/api/tags 是否可达（运行事实）
// 二者必须合成同一个状态再渲染，避免同一页一处「已安装」、一处「连不上」。
type OllamaTone = 'on' | 'warn' | 'off'
interface OllamaStatusView { label: string; tone: OllamaTone; hint: string }
const ollamaStatus = computed<OllamaStatusView>(() => {
  const live = ollamaLive.value
  if (!live) {
    // 首次探测尚未返回：不臆测安装/运行状态。
    return { label: t('localModels.loading'), tone: 'off', hint: '' }
  }
  const present = status.value?.ollama_present
  const base = live.base_url || 'http://localhost:11434'
  if (live.online) {
    // 服务可达但 PATH 上找不到命令：如实说明「在跑但没找到可执行文件」。
    return present === false
      ? { label: t('localModels.ollamaRunningNoCli'), tone: 'on', hint: '' }
      : { label: t('localModels.ollamaInstalledRunning'), tone: 'on', hint: '' }
  }
  if (present === undefined) {
    // 安装事实（status）尚未返回：不臆测「未安装」，避免与在线探测结果互相打脸。
    return { label: '—', tone: 'off', hint: '' }
  }
  if (present) {
    // 已安装但端口探测失败：统一为「已安装 · 未运行」并给启动指引。
    return {
      label: t('localModels.ollamaInstalledStopped'),
      tone: 'warn',
      hint: t('localModels.ollamaStartHint', { base }),
    }
  }
  return { label: t('localModels.ollamaNotInstalled'), tone: 'off', hint: t('localModels.ollamaInstallHint') }
})

/** 模型体积：与内存同口径 —— 没有有效数值时显示占位符，不输出「0.00 GB」误导用户。 */
function formatGb(v: number | undefined): string {
  const n = typeof v === 'number' && Number.isFinite(v) ? v : 0
  return n > 0 ? `${n.toFixed(2)} GB` : '—'
}

async function loadOllamaLive() {
  ollamaLoading.value = true
  try {
    const r = await rpc.call<OllamaLiveResult>('models.local.ollama', {})
    ollamaLive.value = r
  } catch (e) {
    // 旧网关可能尚未注册该 RPC：降级为「离线」展示，不向用户抛 RPC 裸错。
    ollamaLive.value = {
      ok: false,
      online: false,
      base_url: '',
      models: [],
      error: e instanceof Error ? e.message : String(e),
    }
  } finally {
    ollamaLoading.value = false
  }
}

async function deployOllama(m: OllamaLiveModel) {
  ollamaDeploying.value = m.name
  ollamaMsg.value = ''
  try {
    const r = await rpc.call<{ ok: boolean; error?: string }>('models.local.ollama.deploy', {
      model: m.name,
      activate: true,
    })
    if (r && r.ok) {
      ollamaMsg.value = t('localModels.ollamaLiveDeployedOk')
      // 部署成功后刷新已部署档案（让新 profile 出现在「Deployed」区）、live 列表
      // 与路由候选真源（config.get candidateModels）。
      await Promise.all([loadStatus(), loadOllamaLive(), candidateStore.refresh()])
    } else {
      ollamaMsg.value = r?.error || t('localModels.ollamaLiveDeployFailed')
    }
  } catch (e) {
    ollamaMsg.value = e instanceof Error ? e.message : String(e)
  } finally {
    ollamaDeploying.value = null
  }
}

// ── shipped gate 实时校验（调用 models.gate.check）──
// 目录徽章只是「模型本身放行属性」；这里按本机宿主事实（权重是否就位、运行时是否支持）
// 实时计算是否真能放行。host 事实是全局开关，对所有目录项一致。
const gateReports = ref<Record<string, GateReportData>>({})
const gateBusy = ref<string | null>(null)
const gateWeights = ref(false)
const gateRuntime = ref(true)

async function checkGate(id: string) {
  gateBusy.value = id
  try {
    const r = await rpc.call<GateReportData>('models.gate.check', {
      id,
      weightsPresent: gateWeights.value,
      runtimeSupports: gateRuntime.value,
    })
    if (r && typeof r.passed === 'boolean') {
      gateReports.value[id] = r
    } else {
      gateReports.value[id] = { model: id, passed: false, blocking: [], findings: [] }
    }
  } catch (e) {
    gateReports.value[id] = {
      model: id,
      passed: false,
      blocking: ['RPC'],
      findings: [
        { element: 'RPC', verdict: 'fail', detail: e instanceof Error ? e.message : String(e) },
      ],
    }
  } finally {
    gateBusy.value = null
  }
}

async function checkAllGates() {
  for (const e of catalog.value) {
    await checkGate(e.id)
  }
}

async function loadSystem() {
  try {
    const s = await rpc.call<HostSpec>('models.local.system', {})
    sys.value = {
      os: s.os ?? '',
      ram_gb: typeof s.ram_gb === 'number' ? s.ram_gb : 0,
      cores: typeof s.cores === 'number' ? s.cores : 0,
      gpu: !!s.gpu,
      gpu_name: typeof s.gpu_name === 'string' ? s.gpu_name.trim() : '',
    }
  } catch {
    // 探测失败不阻断：保留默认（Rust 端兜底宿主真实规格）
  }
}

function gateBadges(e: CatalogEntry): { label: string; ok: boolean }[] {
  const g = e.shipped_gate
  return [
    { label: t('localModels.gateWeightsBadge'), ok: g.weights },
    { label: t('localModels.gateLicenseBadge'), ok: !!g.license },
    { label: t('localModels.gateEvalBadge'), ok: !!g.eval },
    { label: t('localModels.gatePricingBadge'), ok: !!g.pricing },
    { label: t('localModels.gateQuantBadge'), ok: g.quantized },
    { label: t('localModels.gateReproBadge'), ok: g.reproducible },
  ]
}

async function loadCatalog() {
  try {
    const res = await rpc.call<{ items: CatalogEntry[] }>('models.local.catalog', {})
    catalog.value = res.items ?? []
  } catch (e) { error.value = `${t('localModels.loadFailed')}: ${e instanceof Error ? e.message : String(e)}` }
}
async function loadStatus() {
  try { status.value = await rpc.call<Status>('models.local.status', {}) } catch (e) { error.value = `${t('localModels.loadFailed')}: ${e instanceof Error ? e.message : String(e)}` }
}
async function loadCost() {
  try { cost.value = await rpc.call<any>('usage.cost', {}) } catch { /* 可选 */ }
}
async function runRecommend() {
  try {
    recommend.value = await rpc.call<Recommend>('models.local.recommend', {
      intent: intent.value,
      ramGB: sys.value.ram_gb,
      cores: sys.value.cores,
      gpu: sys.value.gpu,
      os: sys.value.os,
    })
  } catch (e) { error.value = `${t('localModels.loadFailed')}: ${e instanceof Error ? e.message : String(e)}` }
}
async function download(id: string) {
  try { await rpc.call('models.local.download', { id }); await loadStatus() } catch (e) { error.value = e instanceof Error ? e.message : String(e) }
}
async function deploy(id: string) {
  try { await rpc.call('models.local.deploy', { id }); await loadStatus(); await candidateStore.refresh() } catch (e) { error.value = e instanceof Error ? e.message : String(e) }
}
async function toggleStart(id: string) {
  try {
    const m = status.value?.managed?.find(x => x.id === id)
    await rpc.call('models.local.' + (m?.started ? 'stop' : 'start'), { id })
    await loadStatus()
  } catch (e) { error.value = e instanceof Error ? e.message : String(e) }
}
async function remove(id: string) {
  try { await rpc.call('models.local.remove', { id, removeProfile: true }); await loadStatus() } catch (e) { error.value = e instanceof Error ? e.message : String(e) }
}

async function loadFinetunes() {
  try {
    const r = await rpc.call<{ jobs: any[] }>('models.local.finetune.list', {})
    ftJobs.value = r.jobs ?? []
  } catch { /* 可选 */ }
}
async function startFinetune() {
  ftMsg.value = ''
  if (!ftForm.value.baseId || !ftForm.value.dataset) {
    ftMsg.value = t('localModels.missingFields')
    return
  }
  ftBusy.value = true
  try {
    const r = await rpc.call<any>('models.local.finetune', {
      baseId: ftForm.value.baseId,
      dataset: ftForm.value.dataset,
      method: ftForm.value.method,
      epochs: ftForm.value.epochs,
      lr: ftForm.value.lr,
    })
    ftMsg.value = r.note ?? (r.ok ? t('localModels.submitted') : (r.error ?? t('localModels.submitFailed')))
    if (r.needs_backend) ftMsg.value += t('localModels.backendNote')
    await loadFinetunes()
  } catch (e) { ftMsg.value = e instanceof Error ? e.message : String(e) } finally { ftBusy.value = false }
}
async function cancelFinetune(id: string) {
  try { await rpc.call('models.local.finetune.cancel', { id }); await loadFinetunes() } catch (e) { error.value = e instanceof Error ? e.message : String(e) }
}

async function refreshAll() {
  error.value = ''
  try {
    // 冷加载竞态守卫：WS 仍处于 connecting 时先等连接就绪，避免把「未就绪」
    // 当成加载失败，把 `not connected (state: connecting)` 裸串渲染给用户。
    if (!rpc.isConnected) await rpc.waitForConnection(10_000)
    await loadSystem()
    await Promise.all([loadCatalog(), loadStatus(), loadCost(), loadFinetunes(), loadOllamaLive(), candidateStore.refresh()])
    await runRecommend()
  } catch (e) {
    error.value = `${t('localModels.loadFailed')}: ${e instanceof Error ? e.message : String(e)}`
  }
}

let modelsChangedUnsub: (() => void) | null = null
let modelsChangedTimer: ReturnType<typeof setTimeout> | null = null

// ── Ollama 在线状态的自动核实 ──
//
// 只在页面打开时探一次是不够的：Ollama 很可能**在页面打开之后**才启动（或中途被杀），
// 此时徽标会一直停在「离线」，用户必须手点刷新才知道服务已经好了。
// 这里做两件事：① 页面可见时每 15s 复探一次；② 切回前台立刻复探。
// 后台标签页不探（省 RPC/省电），`ollamaLoading` 期间不叠加请求。
const OLLAMA_PROBE_MS = 15_000
let ollamaProbeTimer: ReturnType<typeof setInterval> | null = null

function stopOllamaProbe(): void {
  if (ollamaProbeTimer) {
    clearInterval(ollamaProbeTimer)
    ollamaProbeTimer = null
  }
}

function startOllamaProbe(): void {
  stopOllamaProbe()
  ollamaProbeTimer = setInterval(() => {
    if (document.visibilityState !== 'visible') return
    if (!rpc.isConnected || ollamaLoading.value) return
    void loadOllamaLive()
  }, OLLAMA_PROBE_MS)
}

function onOllamaVisibility(): void {
  if (document.visibilityState === 'visible') {
    // 与轮询同口径：未连上时探测只会拿到「连不上」，会把在线状态误刷成离线。
    if (rpc.isConnected) void loadOllamaLive()
    startOllamaProbe()
  } else {
    stopOllamaProbe()
  }
}

onMounted(() => {
  candidateUnsub = candidateStore.subscribe()
  // P3 打通：其他页面/CLI 的模型变更（档案激活、路由保存、部署）统一广播
  // models.changed → 本中心防抖被动刷新（不整页重载）。自身部署动作返回后
  // 已主动 refreshAll，这里只处理外部来源，防抖避免连发事件重复全量加载。
  modelsChangedUnsub = rpc.on('models.changed', () => {
    if (modelsChangedTimer) clearTimeout(modelsChangedTimer)
    modelsChangedTimer = setTimeout(() => {
      modelsChangedTimer = null
      void refreshAll()
    }, 150)
  })
  void refreshAll()
  startOllamaProbe()
  document.addEventListener('visibilitychange', onOllamaVisibility)
})

onUnmounted(() => {
  if (modelsChangedTimer) clearTimeout(modelsChangedTimer)
  modelsChangedTimer = null
  modelsChangedUnsub?.()
  modelsChangedUnsub = null
  candidateUnsub?.()
  candidateUnsub = null
  stopOllamaProbe()
  document.removeEventListener('visibilitychange', onOllamaVisibility)
})
</script>

<style scoped>
.lm-stage { padding: 24px 28px; max-width: 1100px; margin: 0 auto; }
.lm-error { margin: 12px 0; padding: 10px 14px; background: color-mix(in srgb, var(--danger) 10%, var(--card)); border: 1px solid color-mix(in srgb, var(--danger) 30%, transparent); border-radius: var(--radius-sm); color: var(--danger); font-size: 12px; }
.lm-muted { color: var(--text-dim); font-size: 12px; }
.lm-ok { color: var(--ok); }
.lm-warn { color: var(--warn); }
.lm-danger { color: var(--danger) !important; }

.lm-system { display: grid; grid-template-columns: repeat(auto-fit, minmax(280px, 1fr)); gap: 14px; margin-bottom: 22px; }
.lm-sys-card { background: var(--card); border: 1px solid var(--hairline); border-radius: var(--radius-md); padding: 14px 16px; }
.lm-sys-title { font-size: 13px; font-weight: 700; margin-bottom: 10px; }
.lm-sys-rows, .lm-cost-rows { display: flex; flex-direction: column; gap: 6px; font-size: 12.5px; }
.lm-sys-rows > div, .lm-cost-rows > div { display: flex; justify-content: space-between; }
.lm-sys-rows span, .lm-cost-rows span { color: var(--text-dim); }
.lm-tier { color: var(--accent); font-weight: 700; }
.lm-gpu-hint { font-weight: 400; font-size: 11px; color: var(--text-dim); }
.lm-cost-big { font-size: 26px; font-weight: 800; color: var(--accent); margin-bottom: 8px; }
.lm-cost-advice { margin-top: 6px; font-size: 11.5px; color: var(--text-dim); line-height: 1.5; }
.lm-profile { display: flex; align-items: center; gap: 8px; font-size: 12.5px; padding: 3px 0; }
.lm-profile__tag { background: color-mix(in srgb, var(--accent) 15%, transparent); color: var(--accent); padding: 1px 7px; border-radius: var(--radius-pill); font-size: 10.5px; font-weight: 700; }

.lm-candidates { margin-top: 10px; border-top: 1px dashed var(--hairline); padding-top: 8px; }
.lm-candidates__title { font-size: 11px; font-weight: 700; color: var(--text-dim); margin-bottom: 6px; }
.lm-candidates__chips { display: flex; flex-wrap: wrap; gap: 5px; }
.lm-candidates__chip { background: color-mix(in srgb, var(--ok) 12%, transparent); color: var(--ok); padding: 1px 8px; border-radius: var(--radius-pill); font-size: 10.5px; font-weight: 600; max-width: 100%; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }

.lm-section { margin-bottom: 24px; }
.lm-section__head { display: flex; align-items: center; justify-content: space-between; margin-bottom: 10px; }
.lm-section__head h3 { font-size: 16px; font-weight: 700; margin: 0; }
.lm-intent { font-size: 12.5px; color: var(--text-dim); display: flex; align-items: center; gap: 6px; }
.lm-intent select { background: var(--card); color: var(--text); border: 1px solid var(--hairline); border-radius: var(--radius-xs); padding: 3px 6px; font-size: 12.5px; }
.lm-reason { font-size: 12.5px; color: var(--text-dim); margin: 0 0 10px; }

.lm-picks { display: grid; grid-template-columns: repeat(auto-fill, minmax(240px, 1fr)); gap: 12px; }
.lm-pick, .lm-entry { background: var(--card); border: 1px solid var(--hairline); border-radius: var(--radius-md); padding: 12px 14px; }
.lm-pick.is-managed, .lm-entry.is-managed { border-color: color-mix(in srgb, var(--accent) 40%, var(--hairline)); }
.lm-pick__name, .lm-entry__head { display: flex; align-items: center; gap: 8px; }
.lm-pick__intent, .lm-entry__intent { background: color-mix(in srgb, var(--accent) 14%, transparent); color: var(--accent); padding: 1px 7px; border-radius: var(--radius-pill); font-size: 10.5px; font-weight: 600; }
.lm-pick__meta, .lm-entry__meta { margin-top: 6px; font-size: 11.5px; color: var(--text-dim); display: flex; flex-wrap: wrap; gap: 8px; }
.lm-pick__actions, .lm-entry__actions { margin-top: 10px; display: flex; gap: 6px; flex-wrap: wrap; }

.lm-catalog { display: grid; grid-template-columns: repeat(auto-fill, minmax(320px, 1fr)); gap: 14px; }
.lm-gate { display: flex; flex-wrap: wrap; gap: 5px; margin-top: 8px; }
.lm-gate span { font-size: 10.5px; padding: 1px 7px; border-radius: var(--radius-xs); }
.lm-gate__ok { background: color-mix(in srgb, var(--ok) 14%, transparent); color: var(--ok); }
.lm-gate__bad { background: color-mix(in srgb, var(--danger) 14%, transparent); color: var(--danger); }
.lm-entry__eval { margin-top: 8px; font-size: 12px; color: var(--text-muted); line-height: 1.5; }
.lm-entry__src { margin-top: 6px; font-size: 11px; word-break: break-all; }

.lm-gate-opts { display: flex; flex-wrap: wrap; gap: 14px; align-items: center; font-size: 12px; color: var(--text-dim); margin-bottom: 12px; }
.lm-gate-opts label { display: flex; align-items: center; gap: 5px; cursor: pointer; }

.lm-finetune { background: var(--card); border: 1px solid var(--hairline); border-radius: var(--radius-md); padding: 14px 16px; }
.lm-ft-form { display: flex; flex-wrap: wrap; gap: 12px; align-items: flex-end; }
.lm-ft-form label { display: flex; flex-direction: column; gap: 4px; font-size: 12px; color: var(--text-dim); }
.lm-ft-form select, .lm-ft-form input { background: var(--bg); color: var(--text); border: 1px solid var(--hairline); border-radius: var(--radius-xs); padding: 5px 8px; font-size: 12.5px; min-width: 140px; }
.lm-ft-msg { margin: 10px 0 0; font-size: 12px; color: var(--text-muted); line-height: 1.5; }
.lm-ft-job { display: flex; align-items: center; gap: 10px; font-size: 12.5px; padding: 6px 0; border-top: 1px solid var(--hairline); margin-top: 8px; flex-wrap: wrap; }
.lm-ft-job__id { font-family: ui-monospace, SFMono-Regular, Menlo, monospace; color: var(--accent); font-size: 11.5px; }

.lm-ollama-bar { display: flex; align-items: center; gap: 10px; margin-bottom: 10px; font-size: 12px; }
.lm-ollama-badge { padding: 1px 8px; border-radius: var(--radius-pill); font-size: 10.5px; font-weight: 700; }
.lm-ollama-badge--on { background: color-mix(in srgb, var(--ok) 15%, transparent); color: var(--ok); }
.lm-ollama-badge--off { background: color-mix(in srgb, var(--text-dim) 15%, transparent); color: var(--text-dim); }
.lm-ollama-badge--warn { background: color-mix(in srgb, var(--warn) 15%, transparent); color: var(--warn); }
.lm-ollama-grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(260px, 1fr)); gap: 12px; }
.lm-ollama-card { background: var(--card); border: 1px solid var(--hairline); border-radius: var(--radius-md); padding: 12px 14px; }
.lm-ollama-name { font-size: 13px; word-break: break-all; }
.lm-ollama-meta { margin-top: 6px; display: flex; flex-wrap: wrap; gap: 8px; font-size: 11.5px; }
.lm-ollama-actions { margin-top: 10px; display: flex; gap: 6px; flex-wrap: wrap; }
.lm-ollama-msg { margin-top: 10px; font-size: 12px; color: var(--text-muted); line-height: 1.5; }
</style>
