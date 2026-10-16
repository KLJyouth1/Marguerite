<template>
  <div class="control-stage lm-stage">
    <header class="control-stage__header">
      <div class="control-stage__title-block">
        <h1 class="control-stage__title">本地模型中心</h1>
        <p class="control-stage__subtitle">聚合下载源 · 系统智能推荐 · 一键下载 / 部署 / 启停 · 本地/云路由标记</p>
      </div>
      <div class="control-stage__actions">
        <button class="btn btn--ghost" @click="refreshAll"><Icon name="refresh" :size="16" /><span>刷新</span></button>
      </div>
    </header>

    <section v-if="error" class="lm-error">{{ error }}</section>

    <!-- ── 系统概览 + 成本时段 ── -->
    <div class="lm-system">
      <div class="lm-sys-card">
        <div class="lm-sys-title">💻 系统配置</div>
        <div class="lm-sys-rows">
          <div><span>系统</span><b>{{ sys.os || '—' }}</b></div>
          <div><span>内存</span><b>{{ sys.ramGB }} GB</b></div>
          <div><span>核心数</span><b>{{ sys.cores }}</b></div>
          <div><span>GPU</span><b :class="sys.gpu ? 'lm-ok' : 'lm-muted'">{{ sys.gpu ? (sys.gpuName || '检测到') : '未检测到' }}</b></div>
          <div><span>推荐档位</span><b class="lm-tier">{{ recommend?.tier || '—' }}</b></div>
          <div><span>Ollama</span><b :class="status?.ollama_present ? 'lm-ok' : 'lm-muted'">{{ status?.ollama_present ? '已安装' : '未安装' }}</b></div>
        </div>
      </div>
      <div class="lm-sys-card">
        <div class="lm-sys-title">💸 成本与时段</div>
        <template v-if="cost">
          <div class="lm-cost-big">≈ ${{ cost.estimated_cost_usd }}</div>
          <div class="lm-cost-rows">
            <div><span>当前时段</span><b :class="cost.period === 'off-peak' ? 'lm-ok' : 'lm-warn'">{{ cost.period === 'off-peak' ? '低谷 (off-peak)' : '峰值 (peak)' }}</b></div>
            <div><span>输入</span><b>${{ cost.price_input_per_mtok }}/M</b></div>
            <div><span>输出</span><b>${{ cost.price_output_per_mtok }}/M</b></div>
            <div class="lm-cost-advice">{{ cost.advice }}</div>
          </div>
        </template>
        <div v-else class="lm-muted">加载中…</div>
      </div>
      <div class="lm-sys-card">
        <div class="lm-sys-title">🧭 已部署本地模型（路由可见）</div>
        <div v-if="localProfiles.length === 0" class="lm-muted">尚未部署本地模型到路由</div>
        <div v-for="p in localProfiles" :key="p.id" class="lm-profile">
          <span class="lm-profile__tag">本地</span>
          <span>{{ p.label }}</span>
          <span class="lm-muted">{{ p.model }}</span>
          <b v-if="p.active" class="lm-ok">· 当前主模型</b>
        </div>
      </div>
    </div>

    <!-- ── 智能推荐 ── -->
    <section class="lm-section">
      <div class="lm-section__head">
        <h3>🤖 智能推荐</h3>
        <label class="lm-intent">
          用途
          <select v-model="intent" @change="runRecommend">
            <option value="general">通用</option>
            <option value="coding">编码</option>
            <option value="reasoning">推理</option>
            <option value="chat">对话</option>
            <option value="multimodal">多模态</option>
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
            <button class="btn btn--primary btn--sm" @click="download(p.id)">下载</button>
            <button class="btn btn--secondary btn--sm" @click="deploy(p.id)">部署到路由</button>
          </div>
        </div>
      </div>
    </section>

    <!-- ── 目录 ── -->
    <section class="lm-section">
      <div class="lm-section__head">
        <h3>📦 聚合下载目录（{{ catalog.length }}）</h3>
        <button class="btn btn--ghost btn--sm" :disabled="!!gateBusy" @click="checkAllGates">校验全部 gate</button>
      </div>
      <div class="lm-gate-opts">
        <label><input type="checkbox" v-model="gateWeights" /> 权重已下载 / 就位</label>
        <label><input type="checkbox" v-model="gateRuntime" /> 本机运行时支持该格式</label>
        <span class="lm-muted">勾选「权重已下载」再校验；否则权重按「宣布 ≠ 可部署」判为阻断</span>
      </div>
      <div class="lm-catalog">
        <div v-for="e in catalog" :key="e.id" class="lm-entry" :class="{ 'is-managed': managedIds.has(e.id) }">
          <div class="lm-entry__head">
            <b>{{ e.label }}</b>
            <span class="lm-entry__intent">{{ e.intent }}</span>
          </div>
          <div class="lm-entry__meta">
            <span>{{ e.family }}</span><span>{{ e.size_gb }}GB</span><span>{{ e.context }}</span>
            <span>{{ e.min_ram_gb }}GB RAM 起</span>
          </div>
          <div class="lm-gate">
            <span v-for="(v, k) in gateBadges(e)" :key="k" :class="v.ok ? 'lm-gate__ok' : 'lm-gate__bad'">{{ v.label }}</span>
          </div>
          <div class="lm-entry__eval">📊 {{ e.shipped_gate.eval }}</div>
          <div class="lm-entry__src lm-muted">Ollama: {{ e.ollama_tag || '—' }} · HF: {{ e.hf_repo }}</div>
          <div class="lm-entry__actions">
            <button class="btn btn--primary btn--sm" @click="download(e.id)">下载</button>
            <button class="btn btn--secondary btn--sm" @click="deploy(e.id)">部署</button>
            <button
              class="btn btn--ghost btn--sm"
              :disabled="gateBusy === e.id"
              @click="checkGate(e.id)"
            >{{ gateBusy === e.id ? '校验中…' : '校验 shipped gate' }}</button>
            <button
              v-if="managedIds.has(e.id)"
              class="btn btn--ghost btn--sm"
              @click="toggleStart(e.id)"
            >{{ startedIds.has(e.id) ? '停止' : '启动' }}</button>
            <button v-if="managedIds.has(e.id)" class="btn btn--ghost btn--sm lm-danger" @click="remove(e.id)">移除</button>
          </div>
          <GateReport :report="gateReports[e.id]" />
        </div>
      </div>
    </section>

    <!-- ── 训练与微调 ── -->
    <section class="lm-section">
      <div class="lm-section__head">
        <h3>🧪 训练与微调（Finetune）</h3>
        <span class="lm-muted">诚实编排：有训练后端才真正派发，否则标记「待配置后端」</span>
      </div>
      <div class="lm-finetune">
        <div class="lm-ft-form">
          <label>基底模型
            <select v-model="ftForm.baseId">
              <option value="" disabled>选择基底</option>
              <option v-for="b in baseOptions" :key="b.id" :value="b.id">{{ b.label }}</option>
            </select>
          </label>
          <label>数据集路径
            <input v-model="ftForm.dataset" placeholder="/path/to/dataset.jsonl" />
          </label>
          <label>方法
            <select v-model="ftForm.method">
              <option value="lora">LoRA</option>
              <option value="qlora">QLoRA</option>
              <option value="full">Full</option>
            </select>
          </label>
          <label>轮数
            <input v-model.number="ftForm.epochs" type="number" min="1" max="50" />
          </label>
          <label>学习率
            <input v-model.number="ftForm.lr" type="number" step="0.0001" min="0" />
          </label>
          <button class="btn btn--primary btn--sm" :disabled="ftBusy" @click="startFinetune">发起微调</button>
        </div>
        <p v-if="ftMsg" class="lm-ft-msg">{{ ftMsg }}</p>
        <div v-if="ftJobs.length === 0" class="lm-muted">尚无微调任务</div>
        <div v-for="j in ftJobs" :key="j.id" class="lm-ft-job">
          <span class="lm-ft-job__id">{{ j.id }}</span>
          <span class="lm-muted">{{ j.base_label }} · {{ j.method }} · {{ j.epochs }}ep</span>
          <span :class="j.needs_backend ? 'lm-warn' : (j.status === 'running' ? 'lm-ok' : 'lm-muted')">
            {{ j.needs_backend ? '待配置后端' : j.status }}
          </span>
          <span v-if="j.needs_backend" class="lm-muted">需 MGT_FINETUNE_COMMAND</span>
          <button
            v-if="['queued', 'running'].includes(j.status)"
            class="btn btn--ghost btn--sm lm-danger"
            @click="cancelFinetune(j.id)"
          >取消</button>
        </div>
      </div>
    </section>
  </div>
</template>

<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import Icon from '@/components/Icon.vue'
import GateReport from '@/components/model/GateReport.vue'
import type { GateReport as GateReportData } from '@/components/model/gateTypes'
import { useRpcStore } from '@/stores/rpc'

const rpc = useRpcStore()

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

const sys = ref<{ os: string; ramGB: number; cores: number; gpu: boolean; gpuName: string }>({
  os: '', ramGB: 0, cores: 0, gpu: false, gpuName: '',
})
const managedIds = computed(() => new Set(status.value?.managed?.map(m => m.id) ?? []))
const startedIds = computed(() => new Set(status.value?.managed?.filter(m => m.started).map(m => m.id) ?? []))
const localProfiles = computed(() => status.value?.local_profiles ?? [])

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
    const s = await rpc.call<{ os: string; ramGB: number; cores: number; gpu: boolean; gpuName: string }>(
      'models.local.system',
      {},
    )
    sys.value = { os: s.os, ramGB: s.ramGB, cores: s.cores, gpu: s.gpu, gpuName: s.gpuName }
  } catch {
    // 探测失败不阻断：保留默认（Rust 端兜底宿主真实规格）
  }
}

function gateBadges(e: CatalogEntry): { label: string; ok: boolean }[] {
  const g = e.shipped_gate
  return [
    { label: '权重', ok: g.weights },
    { label: '许可', ok: !!g.license },
    { label: '评测', ok: !!g.eval },
    { label: '定价', ok: !!g.pricing },
    { label: '量化', ok: g.quantized },
    { label: '可复现', ok: g.reproducible },
  ]
}

async function loadCatalog() {
  try {
    const res = await rpc.call<{ items: CatalogEntry[] }>('models.local.catalog', {})
    catalog.value = res.items ?? []
  } catch (e) { error.value = `加载失败: ${e instanceof Error ? e.message : String(e)}` }
}
async function loadStatus() {
  try { status.value = await rpc.call<Status>('models.local.status', {}) } catch (e) { error.value = `加载失败: ${e instanceof Error ? e.message : String(e)}` }
}
async function loadCost() {
  try { cost.value = await rpc.call<any>('usage.cost', {}) } catch { /* 可选 */ }
}
async function runRecommend() {
  try {
    recommend.value = await rpc.call<Recommend>('models.local.recommend', {
      intent: intent.value,
      ramGB: sys.value.ramGB,
      cores: sys.value.cores,
      gpu: sys.value.gpu,
      os: sys.value.os,
    })
  } catch (e) { error.value = `加载失败: ${e instanceof Error ? e.message : String(e)}` }
}
async function download(id: string) {
  try { await rpc.call('models.local.download', { id }); await loadStatus() } catch (e) { error.value = e instanceof Error ? e.message : String(e) }
}
async function deploy(id: string) {
  try { await rpc.call('models.local.deploy', { id }); await loadStatus() } catch (e) { error.value = e instanceof Error ? e.message : String(e) }
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
    ftMsg.value = '请选择基底模型并填写数据集路径'
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
    ftMsg.value = r.note ?? (r.ok ? '已提交微调任务' : (r.error ?? '提交失败'))
    if (r.needs_backend) ftMsg.value += '（需在宿主配置训练后端：MGT_FINETUNE_COMMAND 或安装 llama.cpp/unsloth/axolotl）'
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
    await Promise.all([loadCatalog(), loadStatus(), loadCost(), loadFinetunes()])
    await runRecommend()
  } catch (e) {
    error.value = `加载失败: ${e instanceof Error ? e.message : String(e)}`
  }
}

onMounted(() => { void refreshAll() })
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
.lm-cost-big { font-size: 26px; font-weight: 800; color: var(--accent); margin-bottom: 8px; }
.lm-cost-advice { margin-top: 6px; font-size: 11.5px; color: var(--text-dim); line-height: 1.5; }
.lm-profile { display: flex; align-items: center; gap: 8px; font-size: 12.5px; padding: 3px 0; }
.lm-profile__tag { background: color-mix(in srgb, var(--accent) 15%, transparent); color: var(--accent); padding: 1px 7px; border-radius: var(--radius-pill); font-size: 10.5px; font-weight: 700; }

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
</style>
