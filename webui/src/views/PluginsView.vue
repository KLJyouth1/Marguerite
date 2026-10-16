<template>
  <div class="control-stage plugins-stage">
    <header class="control-stage__header">
      <div class="control-stage__title-block">
        <h1 class="control-stage__title">{{ t('plugins.title') }}</h1>
        <p class="control-stage__subtitle">{{ t('plugins.subtitle') }}</p>
      </div>
      <div class="control-stage__actions">
        <div class="plugins-search">
          <Icon name="search" :size="16" />
          <input v-model="search" type="search" :placeholder="t('plugins.searchPlaceholder')" />
        </div>
        <button class="btn btn--ghost" @click="load">
          <Icon name="refresh" :size="16" /><span>{{ t('plugins.refresh') }}</span>
        </button>
        <button class="btn btn--ghost" @click="dockInWorkbench">
          <Icon name="panel-right-open" :size="16" /><span>{{ t('plugins.openInWorkbench') }}</span>
        </button>
        <button class="btn btn--primary" @click="showCreate = !showCreate">
          <Icon name="plus" :size="16" /><span>{{ t('plugins.createSkill') }}</span>
        </button>
      </div>
    </header>

    <section v-if="showCreate" class="skill-create">
      <h2 class="skill-create__title">{{ t('plugins.skill.formTitle') }}</h2>
      <div class="skill-create__form">
        <label class="skill-field">
          <span>{{ t('plugins.skill.nameLabel') }}</span>
          <input v-model="form.name" class="skill-input" :placeholder="t('plugins.skill.namePlaceholder')" />
        </label>
        <label class="skill-field">
          <span>{{ t('plugins.skill.descriptionLabel') }}</span>
          <input v-model="form.description" class="skill-input" :placeholder="t('plugins.skill.descriptionPlaceholder')" />
        </label>
        <label class="skill-field">
          <span>{{ t('plugins.skill.triggersLabel') }}</span>
          <input v-model="form.triggers" class="skill-input" :placeholder="t('plugins.skill.triggersPlaceholder')" />
        </label>
        <label class="skill-field skill-field--full">
          <span>{{ t('plugins.skill.promptLabel') }}</span>
          <textarea v-model="form.prompt_template" class="skill-textarea" rows="5" :placeholder="t('plugins.skill.promptPlaceholder')"></textarea>
        </label>
        <div class="skill-create__actions">
          <button class="btn btn--ghost" @click="showCreate = false">{{ t('plugins.skill.cancel') }}</button>
          <button class="btn btn--primary" :disabled="creating" @click="createSkill">
            {{ creating ? t('plugins.skill.saving') : t('plugins.skill.save') }}
          </button>
        </div>
        <div v-if="createResult" class="skill-create__result" :class="{ 'is-error': !createOk }">{{ createResult }}</div>
      </div>
    </section>

    <section v-if="error" class="plugins-error">{{ error }}</section>

    <section class="plugins-grid">
      <div v-for="p in filtered" :key="p.name" class="plugin-card">
        <div class="plugin-card__header">
          <span class="plugin-card__name">{{ p.name }}</span>
          <span class="plugin-card__maturity" :class="maturityClass(p.maturity)">{{ p.maturity }}</span>
        </div>
        <p class="plugin-card__desc">{{ p.description }}</p>
        <div class="plugin-card__meta">
          <span class="plugin-card__group">{{ p.group }}</span>
          <span class="plugin-card__tier">{{ p.tier }}</span>
        </div>
      </div>
    </section>
    <div v-if="!filtered.length && !error" class="plugins-empty">{{ t('plugins.empty') }}</div>

    <!-- 插件装配状态（P0 S4b：boot 装配 patch → PluginSlot，经 plugins.status 可见） -->
    <section class="market">
      <div class="market__head">
        <div class="market__title">{{ t('plugins.assembly.title') }}</div>
        <button class="btn btn--sm btn--ghost" :disabled="asmBusy" @click="loadAssembly">{{ t('plugins.assembly.refresh') }}</button>
      </div>
      <div v-if="asmError" class="market__note market__note--err">{{ asmError }}</div>
      <div v-if="asm.summary" class="market__note">
        {{ t('plugins.assembly.summary', {
          assembled: asm.summary.assembled_count,
          installed: asm.summary.manifests_total,
          patches: asm.summary.patches_total,
          applied: asm.summary.applied,
          skipped: asm.summary.skipped,
          errors: asm.summary.errors,
        }) }}
      </div>
      <table v-if="asm.manifests && asm.manifests.length" class="market__table">
        <thead><tr><th>{{ t('plugins.assembly.columnManifest') }}</th><th>{{ t('plugins.assembly.columnMode') }}</th><th>{{ t('plugins.assembly.columnStatus') }}</th><th>{{ t('plugins.assembly.columnSlot') }}</th></tr></thead>
        <tbody>
          <tr v-for="m in asm.manifests" :key="m.name">
            <td><b>{{ m.name }}</b></td>
            <td>{{ m.mode === 'declarative-only' ? t('plugins.assembly.modeDeclarative') : t('plugins.assembly.modeNative') }}</td>
            <td>
              <span v-if="m.assembled" class="market__mode" style="border-color: var(--ok); color: var(--ok)">{{ t('plugins.assembly.assembled') }}</span>
              <span v-else class="market__mode">{{ t('plugins.assembly.notAssembled') }}</span>
            </td>
            <td>{{ (m.slots || []).join(', ') }}</td>
          </tr>
        </tbody>
      </table>
      <div v-if="asm.report" class="market__catalog">
        <div class="market__sub">{{ t('plugins.assembly.patchReport') }}</div>
        <div class="market__row"><span class="market__kind">applied</span><span class="market__desc">{{ (asm.report.applied || []).join(' · ') || '—' }}</span></div>
        <div class="market__row"><span class="market__kind">skipped</span><span class="market__desc">{{ (asm.report.skipped || []).map(x => x.join(' · ')).join(' · ') || '—' }}</span></div>
        <div class="market__row"><span class="market__kind">errors</span><span class="market__desc">{{ (asm.report.errors || []).map(x => x.join(' · ')).join(' · ') || '—' }}</span></div>
      </div>
    </section>

    <!-- DSH 插件市场：安全审计 / 导入 / 导出 / 批量审计 -->
    <section class="market">
      <div class="market__head">
        <div class="market__title">{{ t('plugins.market.title') }}</div>
        <button class="btn btn--sm btn--ghost" :disabled="busy" @click="runAudit">{{ t('plugins.market.auditAll') }}</button>
        <button class="btn btn--sm btn--primary" :disabled="busy || !marketItems.length" @click="batchInstall">{{ t('plugins.market.installAll') }}</button>
      </div>
      <div v-if="marketError" class="market__note market__note--err">{{ marketError }}</div>
      <div v-if="auditItems.length" class="market__audit">
        <table class="market__table">
          <thead><tr><th>{{ t('plugins.market.columnPlugin') }}</th><th>{{ t('plugins.market.columnVersion') }}</th><th>{{ t('plugins.market.columnVerdict') }}</th><th>{{ t('plugins.market.columnCapabilities') }}</th><th>{{ t('plugins.market.columnFindings') }}</th></tr></thead>
          <tbody>
            <tr v-for="a in auditItems" :key="a.name">
              <td><b>{{ a.name }}</b></td>
              <td>{{ a.version || "—" }}</td>
              <td><span class="market__verdict" :data-v="a.verdict">{{ verdictLabel(a.verdict) }}</span></td>
              <td>{{ a.capabilities }} / {{ a.hooks }}</td>
              <td><span v-for="(s, i) in [...a.suspects, ...a.risks]" :key="i" class="market__finding">{{ s }}</span><span v-if="!a.suspects.length && !a.risks.length" class="market__finding market__finding--ok">{{ t('plugins.market.noFindings') }}</span></td>
            </tr>
          </tbody>
        </table>
      </div>
      <!-- 过滤标签页：全部 / 插件 / 技能 / Harness 生态 -->
      <div class="market__tabs">
        <button
          v-for="tab in marketTabs"
          :key="tab.id"
          class="market__tab"
          :class="{ 'is-active': marketTab === tab.id }"
          @click="marketTab = tab.id"
        >
          {{ tab.label }}
          <span class="market__tab-count">{{ tabCount(tab.id) }}</span>
        </button>
      </div>
      <div class="market__catalog">
        <div class="market__sub">{{ t('plugins.market.catalogTitle', { count: pluginSkillItems.length }) }}</div>
        <div v-for="m in pluginSkillItems" :key="m.kind + m.name" class="market__row">
          <code>{{ m.name }}</code>
          <span class="market__kind">{{ m.kind === 'plugin' ? t('plugins.market.kindPlugin') : t('plugins.market.kindSkill') }}</span>
          <!-- Makes "installed but inert" visible: a plugin manifest with no
               `entry` is declared, not assembled. Previously this was silent. -->
          <span
            v-if="m.kind === 'plugin' && m.mode === 'declarative-only'"
            class="market__mode"
            :title="t('plugins.market.declarativeOnlyHint')"
          >{{ t('plugins.market.declarativeOnly') }}</span>
          <span class="market__desc">{{ m.description }}</span>
          <button class="btn btn--sm btn--primary" :disabled="busy || m.kind !== 'plugin'" @click="installManifest(m.name)">{{ t('plugins.market.install') }}</button>
          <button class="btn btn--sm btn--ghost" :disabled="busy || m.kind !== 'plugin'" @click="exportManifest(m.name)">{{ t('plugins.market.export') }}</button>
        </div>
      </div>

      <!-- DeepSeek Harness 生态：上游仓库蒸馏目录（market.harness） -->
      <div v-if="marketTab === 'harness' || marketTab === 'all'" class="market__harness">
        <div class="market__sub">
          {{ t('plugins.harness.catalogTitle', { count: harnessItems.length }) }}
        </div>
        <div class="market__harness-grid">
          <div v-for="h in harnessItems" :key="h.name" class="harness-card" :class="{ 'harness-card--aligned': h.deepseek_aligned }">
            <div class="harness-card__head">
              <code class="harness-card__name">{{ h.name }}</code>
              <span class="harness-card__eco" :class="'eco--' + h.ecosystem">{{ ecoLabel(h.ecosystem) }}</span>
            </div>
            <p class="harness-card__desc">{{ h.description }}</p>
            <div class="harness-card__tags">
              <span v-for="c in h.capabilities" :key="c" class="harness-card__tag">{{ c }}</span>
            </div>
            <div class="harness-card__distill">
              <span class="harness-card__distill-label">{{ t('plugins.harness.distillTarget') }}</span>
              <code>{{ h.distilled_via }}</code>
            </div>
            <div v-if="h.deepseek_aligned" class="harness-card__aligned">{{ t('plugins.harness.aligned') }}</div>
          </div>
        </div>
      </div>
      <div v-if="batchResult" class="market__note">{{ batchResult }}</div>
      <div class="market__sources">
        <div class="market__sub">{{ t('plugins.sources.title') }}</div>
        <button class="btn btn--sm btn--ghost" :disabled="busy" @click="refreshSources">{{ t('plugins.sources.refresh') }}</button>
        <div v-for="s in sources" :key="s.kind + s.name" class="market__row">
          <code>{{ s.name }}</code>
          <span class="market__kind">{{ s.kind === 'external_repo' ? t('plugins.sources.kindExternalRepo') : t('plugins.sources.kindCatalog') }}</span>
          <a v-if="s.url" class="market__desc market__link" :href="s.url" target="_blank" rel="noopener">{{ s.url }}</a>
          <span v-else class="market__desc">{{ s.path }}</span>
        </div>
        <p v-if="sourcesHint" class="market__note">{{ sourcesHint }}</p>
      </div>
      <div class="market__form">
        <div class="market__sub">{{ t('plugins.sources.importTitle') }}</div>
        <textarea v-model="importText" class="market__textarea" :placeholder="t('plugins.sources.importPlaceholder')" />
        <div class="market__form-row">
          <label class="market__ack"><input v-model="importAck" type="checkbox" /> {{ t('plugins.sources.importAck') }}</label>
          <button class="btn btn--sm btn--primary" :disabled="busy || !importText.trim()" @click="importManifest">{{ t('plugins.sources.importAction') }}</button>
          <button class="btn btn--sm btn--ghost" :disabled="busy" @click="refreshMarket">{{ t('plugins.sources.refreshMarket') }}</button>
        </div>
        <div v-if="importResult" class="market__note">{{ importResult }}</div>
      </div>
    </section>
  </div>
</template>

<script setup lang="ts">
import { ref, computed, onMounted, reactive, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import Icon from '@/components/Icon.vue'
import { useRpcStore } from '@/stores/rpc'
import { openViewPanel } from '@/components/workbench/AppWorkbench.vue'

const { t } = useI18n()
const rpc = useRpcStore()

interface Plugin { name: string; description: string; group: string; tier: string; maturity: string; }

const plugins = ref<Plugin[]>([])
const search = ref('')
const error = ref('')
const showCreate = ref(false)
const creating = ref(false)
const createResult = ref('')
const createOk = ref(false)
const form = reactive({ name: '', description: '', prompt_template: '', triggers: '' })

const filtered = computed(() => {
  const q = search.value.toLowerCase()
  if (!q) return plugins.value
  return plugins.value.filter(p => p.name.toLowerCase().includes(q) || p.description.toLowerCase().includes(q))
})

/**
 * Dock this view into the workbench. Docking is additive — the workbench opens its
 * own instance of the view, so the main area stays free to navigate elsewhere and
 * this route keeps its own copy.
 */
function dockInWorkbench() {
  openViewPanel({ view: 'plugins', title: t('plugins.title') })
}

function maturityClass(m: string) {
  return { 'maturity--real': m === 'Real', 'maturity--partial': m === 'Partial', 'maturity--narrative': m === 'Narrative' }
}

async function load() {
  error.value = ''
  try {
    const res = await rpc.call<{ plugins: Plugin[] }>('plugins.list')
    plugins.value = res.plugins || []
  } catch (e) {
    error.value = t('plugins.loadFailed', { error: e instanceof Error ? e.message : String(e) })
  }
}

async function createSkill() {
  if (!form.name.trim() || !form.description.trim() || !form.prompt_template.trim()) {
    createResult.value = t('plugins.skill.requiredFields')
    createOk.value = false
    return
  }
  creating.value = true
  createResult.value = ''
  try {
    const res = await rpc.call<{ success: boolean; message: string }>('skills.create', {
      name: form.name.trim(),
      description: form.description.trim(),
      prompt_template: form.prompt_template,
      triggers: form.triggers.split(',').map(s => s.trim()).filter(Boolean),
    })
    createOk.value = res.success
    createResult.value = res.message
    if (res.success) {
      form.name = ''; form.description = ''; form.prompt_template = ''; form.triggers = ''
      showCreate.value = false
    }
  } catch (e) {
    createOk.value = false
    createResult.value = t('plugins.skill.createFailed', { error: e instanceof Error ? e.message : String(e) })
  } finally {
    creating.value = false
  }
}

onMounted(load)

// ── 插件装配状态（P0 S4b：boot 装配 patch → PluginSlot，经 plugins.status 可见）──
// 装配快照在网关启动时生成（OnceLock，重启生效）；前端只读渲染 + 手动刷新。
interface AsmReport { applied: string[]; skipped: Array<[string, string]>; errors: Array<[string, string]> }
interface AsmSummary { manifests_total: number; assembled_count: number; patches_total: number; applied: number; skipped: number; errors: number }
interface AsmManifest { name: string; version: string; mode: string; entry: string | null; activation: string; assembled: boolean; slots: string[] }
interface AssemblyStatus { manifests: AsmManifest[]; slots: any[]; report: AsmReport; summary: AsmSummary }
const asm = ref<AssemblyStatus>({
  manifests: [],
  slots: [],
  report: { applied: [], skipped: [], errors: [] },
  summary: { manifests_total: 0, assembled_count: 0, patches_total: 0, applied: 0, skipped: 0, errors: 0 },
})
const asmError = ref('')
const asmBusy = ref(false)

async function loadAssembly() {
  asmBusy.value = true
  asmError.value = ''
  try {
    const r = await rpc.call<AssemblyStatus>('plugins.status')
    asm.value = r
  } catch (e) {
    asmError.value = t('plugins.assembly.statusFailed', { error: e instanceof Error ? e.message : String(e) })
  } finally {
    asmBusy.value = false
  }
}

onMounted(loadAssembly)

// ── DSH 插件市场：审计 / 导入 / 导出 ──
// `mode` / `entry` / `activation` are present only on `kind === 'plugin'` rows
// (gateway `market.list` → core/src/gateway/market.rs::plugin_market_item).
// `mode === 'declarative-only'` means the manifest declares capabilities but has
// no `entry`, so nothing is assembled at runtime — the old "installed but inert"
// state, which used to be completely silent. Skill / harness rows omit them.
interface MarketItem { name: string; version: string; description: string; kind: string; author?: string; source?: string; mode?: string; entry?: string | null; activation?: string }
interface AuditItem { name: string; version: string; capabilities: number; hooks: number; suspects: string[]; risks: string[]; verdict: string }
const marketItems = ref<MarketItem[]>([])
const auditItems = ref<AuditItem[]>([])
const importText = ref('')
const importAck = ref(false)
const importResult = ref('')
const marketError = ref('')
const busy = ref(false)

// 过滤标签页：全部 / 插件 / 技能 / Harness 生态
type MarketTabId = 'all' | 'plugin' | 'skill' | 'harness'
const marketTab = ref<MarketTabId>('all')
const marketTabs = computed<{ id: MarketTabId; label: string }[]>(() => [
  { id: 'all', label: t('plugins.market.tabAll') },
  { id: 'plugin', label: t('plugins.market.tabPlugin') },
  { id: 'skill', label: t('plugins.market.tabSkill') },
  { id: 'harness', label: t('plugins.market.tabHarness') },
])

// DeepSeek Harness 生态蒸馏目录条目
interface HarnessItem {
  name: string
  description: string
  category: string
  ecosystem: string
  capabilities: string[]
  distilled_via: string
  deepseek_aligned: boolean
}
const harnessItems = ref<HarnessItem[]>([])

// 本地清单（插件 + 技能），不含 harness 生态条目
const pluginSkillItems = computed(() => marketItems.value.filter(m => m.kind === 'plugin' || m.kind === 'skill'))

function tabCount(id: MarketTabId): number {
  if (id === 'all') return marketItems.value.length + harnessItems.value.length
  if (id === 'harness') return harnessItems.value.length
  return marketItems.value.filter(m => m.kind === id).length
}

function ecoLabel(eco: string): string {
  return eco === 'computer_use'
    ? t('plugins.ecosystem.computerUse')
    : t('plugins.ecosystem.harness')
}

function verdictLabel(v: string): string {
  if (v === 'clean') return t('plugins.verdict.clean')
  if (v === 'warn') return t('plugins.verdict.warn')
  if (v === 'broken') return t('plugins.verdict.broken')
  return v
}

async function refreshMarket() {
  marketError.value = ''
  try {
    const r = await rpc.call<{ items: MarketItem[]; total: number }>('market.list', {})
    marketItems.value = r.items ?? []
  } catch (e) { marketError.value = e instanceof Error ? e.message : String(e) }
}

async function refreshHarness() {
  marketError.value = ''
  try {
    const r = await rpc.call<{ items: HarnessItem[]; total: number; hint?: string }>('market.harness', {})
    harnessItems.value = r.items ?? []
  } catch (e) { marketError.value = e instanceof Error ? e.message : String(e) }
}

async function runAudit() {
  busy.value = true
  marketError.value = ''
  try {
    const r = await rpc.call<{ ok: boolean; items: AuditItem[]; total_suspects: number; total_risks: number; error?: string }>('market.audit', {})
    if (r.ok === false) { marketError.value = r.error ?? t('plugins.market.auditFailed') }
    else {
      auditItems.value = r.items ?? []
      marketError.value = t('plugins.market.auditDone', {
        count: r.items?.length ?? 0,
        suspects: r.total_suspects ?? 0,
        risks: r.total_risks ?? 0,
      })
    }
  } catch (e) { marketError.value = e instanceof Error ? e.message : String(e) } finally { busy.value = false }
}

async function importManifest() {
  busy.value = true
  importResult.value = ''
  try {
    const r = await rpc.call<{ ok: boolean; name?: string; version?: string; message?: string; error?: string }>('market.import', { content: importText.value, ack: importAck.value })
    importResult.value = r.ok
      ? t('plugins.market.importSuccess', { name: r.name ?? '', version: r.version ?? '', message: r.message ?? '' })
      : t('plugins.market.importFailed', { error: r.error ?? '' })
    if (r.ok) { importText.value = ''; importAck.value = false; await refreshMarket() }
  } catch (e) { importResult.value = e instanceof Error ? e.message : String(e) } finally { busy.value = false }
}

async function exportManifest(name: string) {
  busy.value = true
  importResult.value = ''
  try {
    const r = await rpc.call<{ ok: boolean; name?: string; content?: string; error?: string }>('market.export', { name })
    if (r.ok) { importText.value = r.content ?? ''; importResult.value = t('plugins.market.exportSuccess', { name }) }
    else importResult.value = r.error ?? t('plugins.market.exportFailed')
  } catch (e) { importResult.value = e instanceof Error ? e.message : String(e) } finally { busy.value = false }
}

interface SourceItem { name: string; path: string; url?: string | null; kind: string }
const sources = ref<SourceItem[]>([])
const sourcesHint = ref('')
const batchResult = ref('')

async function refreshSources() {
  marketError.value = ''
  try {
    const r = await rpc.call<{ ok: boolean; items: SourceItem[]; total: number; hint?: string }>('market.sources', {})
    sources.value = r.items ?? []
    sourcesHint.value = r.hint ?? ''
  } catch (e) { marketError.value = e instanceof Error ? e.message : String(e) }
}

async function installManifest(name: string) {
  busy.value = true
  try {
    const r = await rpc.call<{ success: boolean; message: string }>('market.install', { name })
    marketError.value = r.message ?? ''
    await refreshMarket()
  } catch (e) { marketError.value = e instanceof Error ? e.message : String(e) } finally { busy.value = false }
}

async function batchInstall() {
  busy.value = true
  batchResult.value = ''
  const names = marketItems.value.filter(m => m.kind === 'plugin').map(m => m.name)
  if (!names.length) { busy.value = false; return }
  try {
    const r = await rpc.call<{ ok: boolean; results: Array<{ name: string; success: boolean; message: string }>; total: number }>('market.install.batch', { names })
    const okCount = (r.results ?? []).filter(x => x.success).length
    batchResult.value = t('plugins.market.batchResult', { ok: okCount, total: r.results?.length ?? 0 })
  } catch (e) { batchResult.value = e instanceof Error ? e.message : String(e) } finally { busy.value = false }
}

void refreshMarket()

// Cold deep-link / in-page refresh mounts this view before the WS handshake
// finishes, so the first `market.list` is rejected with
// "not connected (state: connecting)". The refresh button used to be disabled
// whenever the list was empty — which locked the user out of the only recovery
// path (empty list ⇒ disabled ⇒ can never refill). The button is now always
// clickable, and this re-runs the load once the connection is actually ready.
watch(
  () => rpc.state,
  (state) => {
    if (state === 'connected' && !marketItems.value.length && !busy.value) {
      void refreshMarket()
    }
  },
)
void refreshHarness()
</script>

<style scoped>
.plugins-stage { padding: 24px 28px; max-width: 1200px; margin: 0 auto; }
.plugins-error { margin: 16px 0; padding: 10px 14px; background: color-mix(in srgb, var(--danger) 10%, var(--card)); border: 1px solid color-mix(in srgb, var(--danger) 30%, transparent); border-radius: var(--radius-sm); color: var(--danger); font-size: 12px; }
.plugins-search { display: flex; align-items: center; gap: 6px; padding: 6px 10px; border-radius: var(--radius-sm); border: 1px solid var(--hairline); background: var(--bg); }
.plugins-search input { border: none; background: transparent; color: var(--text); font-size: 12px; outline: none; width: 180px; }
.plugins-grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(300px, 1fr)); gap: 10px; margin-top: 20px; }
.plugin-card { background: var(--card); border: 1px solid var(--hairline); border-radius: var(--radius-md); padding: 12px 14px; }
.plugin-card__header { display: flex; justify-content: space-between; align-items: center; }
.plugin-card__name { font-family: var(--font-mono, monospace); font-size: 12px; font-weight: 600; }
.plugin-card__maturity { font-size: 9px; font-weight: 700; padding: 2px 7px; border-radius: var(--radius-pill); }
.maturity--real { background: color-mix(in srgb, var(--ok) 15%, transparent); color: var(--ok); }
.maturity--partial { background: color-mix(in srgb, var(--warn) 15%, transparent); color: var(--warn); }
.maturity--narrative { background: color-mix(in srgb, var(--text-dim) 12%, transparent); color: var(--text-dim); }
.plugin-card__desc { font-size: 11px; color: var(--text-muted); margin: 6px 0; line-height: 1.5; }
.plugin-card__meta { display: flex; gap: 8px; font-size: 10px; color: var(--text-dim); }
.plugin-card__group { padding: 1px 6px; background: var(--bg-hover); border-radius: var(--radius-sm); }
.plugins-empty { text-align: center; padding: 40px; color: var(--text-dim); font-size: 13px; }
.skill-create { margin-top: 20px; background: var(--card); border: 1px solid var(--hairline); border-radius: var(--radius-md); padding: 16px; }
.skill-create__title { font-size: 14px; font-weight: 600; margin: 0 0 12px; }
.skill-create__form { display: grid; grid-template-columns: 1fr 1fr; gap: 10px; }
.skill-field { display: flex; flex-direction: column; gap: 4px; font-size: 11px; color: var(--text-dim); }
.skill-field--full { grid-column: 1 / -1; }
.skill-input { padding: 7px 10px; border-radius: var(--radius-sm); border: 1px solid var(--hairline); background: var(--bg); color: var(--text); font-size: 12px; outline: none; }
.skill-input:focus { border-color: var(--accent); }
.skill-textarea { padding: 8px 10px; border-radius: var(--radius-sm); border: 1px solid var(--hairline); background: var(--bg); color: var(--text); font-size: 12px; outline: none; resize: vertical; font-family: var(--font-mono, monospace); }
.skill-textarea:focus { border-color: var(--accent); }
.skill-create__actions { grid-column: 1 / -1; display: flex; justify-content: flex-end; gap: 8px; margin-top: 4px; }
.skill-create__result { grid-column: 1 / -1; font-size: 12px; padding: 8px 12px; border-radius: var(--radius-sm); }
.skill-create__result:not(.is-error) { background: color-mix(in srgb, var(--ok) 10%, var(--card)); color: var(--ok); }
.skill-create__result.is-error { background: color-mix(in srgb, var(--danger) 10%, var(--card)); color: var(--danger); }
.market { margin-top: 18px; padding: 16px; background: var(--card); border: 1px solid var(--hairline); border-radius: var(--radius-md); }
.market__head { display: flex; justify-content: space-between; align-items: center; gap: 10px; }
.market__title { font-size: 14px; font-weight: 700; }
.market__note { margin-top: 8px; font-size: 12px; color: var(--accent); word-break: break-all; }
.market__note--err { color: var(--danger); }
.market__audit { margin-top: 12px; overflow: auto; }
.market__table { width: 100%; border-collapse: collapse; font-size: 12px; }
.market__table th, .market__table td { text-align: left; padding: 7px 8px; border-bottom: 1px solid var(--hairline); }
.market__verdict { padding: 2px 8px; border-radius: 999px; font-size: 11px; }
.market__verdict[data-v="clean"] { color: var(--ok); background: color-mix(in srgb, var(--ok) 12%, var(--card)); }
.market__verdict[data-v="warn"] { color: var(--warn); background: color-mix(in srgb, var(--warn) 12%, var(--card)); }
.market__verdict[data-v="broken"] { color: var(--danger); background: color-mix(in srgb, var(--danger) 12%, var(--card)); }
.market__finding { display: inline-block; margin: 2px 6px 2px 0; padding: 2px 7px; border-radius: 999px; background: var(--bg-hover); border: 1px solid var(--hairline); font-size: 11px; }
.market__finding--ok { color: var(--ok); border-color: color-mix(in srgb, var(--ok) 40%, transparent); }
.market__catalog { margin-top: 14px; }
.market__sub { font-size: 11px; font-weight: 600; color: var(--text-dim); margin-bottom: 8px; }
.market__row { display: flex; gap: 10px; align-items: center; padding: 6px 0; border-bottom: 1px solid var(--hairline); }
.market__row code { font-family: var(--font-mono); font-size: 12px; }
.market__kind { font-size: 10px; padding: 2px 7px; border-radius: 999px; background: var(--bg-hover); color: var(--text-dim); }
.market__mode { font-size: 10px; padding: 2px 7px; border-radius: 999px; background: var(--bg-hover); border: 1px solid var(--warn); color: var(--warn); white-space: nowrap; }
.market__desc { flex: 1; min-width: 0; font-size: 11px; color: var(--text-dim); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.market__link { color: var(--accent); text-decoration: none; }
.market__sources { margin-top: 14px; }
.market__form { margin-top: 14px; border-top: 1px solid var(--hairline); padding-top: 10px; }
.market__textarea { width: 100%; min-height: 72px; padding: 8px 10px; border-radius: var(--radius-sm); border: 1px solid var(--hairline); background: var(--bg); color: var(--text); font-size: 11px; resize: vertical; font-family: var(--font-mono, monospace); }
.market__form-row { display: flex; gap: 10px; align-items: center; margin-top: 8px; }
.market__ack { font-size: 11px; color: var(--text-dim); display: flex; gap: 5px; align-items: center; }
.market__tabs { display: flex; gap: 6px; margin-top: 14px; flex-wrap: wrap; }
.market__tab { display: inline-flex; align-items: center; gap: 6px; padding: 5px 12px; border-radius: 999px; border: 1px solid var(--hairline); background: var(--bg); color: var(--text-dim); font-size: 12px; cursor: pointer; transition: all var(--dur-fast) var(--ease-standard); }
.market__tab:hover { color: var(--text); border-color: color-mix(in srgb, var(--accent) 40%, transparent); }
.market__tab.is-active { color: var(--accent); border-color: var(--accent); background: color-mix(in srgb, var(--accent) 12%, var(--card)); }
.market__tab-count { font-size: 10px; padding: 1px 6px; border-radius: 999px; background: var(--bg-hover); color: var(--text-dim); }
.market__harness { margin-top: 16px; border-top: 1px dashed var(--hairline); padding-top: 14px; }
.market__harness-grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(280px, 1fr)); gap: 10px; }
.harness-card { background: var(--card); border: 1px solid var(--hairline); border-radius: var(--radius-md); padding: 12px 14px; display: flex; flex-direction: column; gap: 8px; }
.harness-card--aligned { border-color: color-mix(in srgb, var(--accent) 40%, transparent); background: color-mix(in srgb, var(--accent) 6%, var(--card)); }
.harness-card__head { display: flex; justify-content: space-between; align-items: center; gap: 8px; }
.harness-card__name { font-family: var(--font-mono, monospace); font-size: 12px; font-weight: 600; }
.harness-card__eco { font-size: 9px; font-weight: 700; padding: 2px 7px; border-radius: var(--radius-pill); white-space: nowrap; }
.eco--harness { background: color-mix(in srgb, var(--accent) 15%, transparent); color: var(--accent); }
.eco--computer_use { background: color-mix(in srgb, var(--text-dim) 18%, transparent); color: var(--text-dim); }
.harness-card__desc { font-size: 11px; color: var(--text-muted); margin: 0; line-height: 1.5; }
.harness-card__tags { display: flex; flex-wrap: wrap; gap: 4px; }
.harness-card__tag { font-size: 10px; padding: 1px 6px; border-radius: var(--radius-sm); background: var(--bg-hover); border: 1px solid var(--hairline); color: var(--text-dim); font-family: var(--font-mono, monospace); }
.harness-card__distill { display: flex; flex-direction: column; gap: 3px; padding: 6px 8px; background: var(--bg); border-radius: var(--radius-sm); border: 1px solid var(--hairline); }
.harness-card__distill-label { font-size: 9px; font-weight: 700; color: var(--text-dim); text-transform: uppercase; letter-spacing: .04em; }
.harness-card__distill code { font-family: var(--font-mono, monospace); font-size: 10px; color: var(--text-muted); word-break: break-all; }
.harness-card__aligned { font-size: 10px; font-weight: 600; color: var(--accent); padding: 2px 8px; border-radius: 999px; background: color-mix(in srgb, var(--accent) 12%, transparent); align-self: flex-start; }
</style>