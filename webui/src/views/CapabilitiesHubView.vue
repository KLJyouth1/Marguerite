<template>
  <div class="control-stage hub-stage">
    <header class="control-stage__header">
      <div class="control-stage__title-block">
        <p class="hub-kicker">Capabilities</p>
        <h1 class="control-stage__title">{{ t('capabilitiesHub.title') }}</h1>
        <p class="control-stage__subtitle">
          {{ t('capabilitiesHub.subtitle') }}
        </p>
      </div>
      <div class="control-stage__actions">
        <button class="btn btn--ghost" @click="load">
          <Icon name="refresh" :size="16" /><span>{{ t('capabilitiesHub.refresh') }}</span>
        </button>
      </div>
    </header>

    <section v-if="stats" class="hub-meter" :aria-label="t('capabilitiesHub.meterAria')">
      <div class="hub-meter__bar">
        <span class="hub-meter__seg hub-meter__seg--real" :style="{ width: pct(stats.real) }" />
        <span class="hub-meter__seg hub-meter__seg--partial" :style="{ width: pct(stats.partial) }" />
        <span class="hub-meter__seg hub-meter__seg--narrative" :style="{ width: pct(stats.narrative) }" />
      </div>
      <div class="hub-pills">
        <i18n-t keypath="capabilitiesHub.pill.implemented" tag="span" class="hub-pill">
          <template #done><b>{{ stats.implemented }}</b></template>
          <template #total>{{ stats.total }}</template>
        </i18n-t>
        <span class="hub-pill hub-pill--accent">{{ t('capabilitiesHub.pill.real', { n: stats.real }) }}</span>
        <span class="hub-pill">{{ t('capabilitiesHub.pill.partial', { n: stats.partial }) }}</span>
        <span class="hub-pill hub-pill--dim">{{ t('capabilitiesHub.pill.narrative', { n: stats.narrative }) }}</span>
      </div>
    </section>

    <!-- P3 打通：能力注册表（capabilities.groups 单一事实源）——与对话编排面板 /
         舰队页探测 / 权限矩阵共享；点击组卡片展开该组能力明细。 -->
    <section v-if="registry" class="hub-registry" aria-label="capability registry">
      <div class="hub-registry__head">
        <b>{{ t('capabilitiesHub.registryTitle') }}</b>
        <span>{{ t('capabilitiesHub.registrySubtitle') }}</span>
      </div>
      <div class="hub-registry__grid">
        <button
          v-for="(g, i) in registry"
          :key="g.id"
          type="button"
          class="hub-reg-group"
          :class="{ 'is-open': expandedGroup === g.id }"
          :aria-expanded="expandedGroup === g.id ? 'true' : 'false'"
          :style="{ '--i': String(i) }"
          @click="toggleGroup(g.id)"
        >
          <div class="hub-reg-group__row">
            <b>{{ g.label }}</b>
            <span class="hub-reg-group__meta">
              {{ t('capabilitiesHub.registryImplemented', { n: implementedCount(g.id) }) }}
              · {{ t('capabilitiesHub.registryGroupCount', { n: g.capabilities.length }) }}
            </span>
          </div>
          <div v-if="expandedGroup === g.id" class="hub-reg-group__items">
            <span
              v-for="cap in g.capabilities"
              :key="cap.id"
              class="hub-reg-cap"
              :class="`hub-reg-cap--${normMaturity(cap.maturity)}`"
              :title="cap.description"
            >
              <b>{{ cap.name || cap.id }}</b>
              <i>{{ cap.maturity }}</i>
            </span>
          </div>
        </button>
      </div>
    </section>
    <p v-else-if="registryFailed" class="hub-registry-empty">{{ t('capabilitiesHub.registryEmpty') }}</p>

    <div class="hub-grid">
      <RouterLink
        v-for="(s, i) in sections"
        :key="s.nameKey"
        class="hub-card"
        :to="s.href"
        :style="{ '--i': String(i) }"
      >
        <div class="hub-card__icon"><Icon :name="s.icon" :size="18" /></div>
        <div class="hub-card__body">
          <b>{{ t(s.nameKey) }}</b>
          <span>{{ t(s.descKey) }}</span>
          <i>{{ t(s.statusKey) }}</i>
        </div>
        <span class="hub-card__arrow" aria-hidden="true">→</span>
      </RouterLink>
    </div>
  </div>
</template>

<script setup lang="ts">
import { ref, onMounted } from 'vue'
import { useI18n } from 'vue-i18n'
import Icon from '@/components/Icon.vue'
import { useRpcStore } from '@/stores/rpc'
import type { IconName } from '@/utils/icons'

const rpc = useRpcStore()
const { t } = useI18n()
interface CapStats { total: number; real: number; partial: number; implemented: number; narrative: number }
interface Capabilities { stats?: CapStats }
const stats = ref<CapStats | null>(null)

/** P3 单一事实源：capabilities.groups 返回的全量能力注册表（12 组）。 */
interface BackendCapability {
  id: string
  name?: string
  group?: string
  tier?: string
  maturity?: 'Real' | 'Partial' | 'Narrative' | string
  description?: string
}
interface CapabilityGroupRow {
  id: string
  label: string
  capabilities: BackendCapability[]
}
const registry = ref<CapabilityGroupRow[] | null>(null)
const registryFailed = ref(false)
const expandedGroup = ref<string | null>(null)

function toggleGroup(id: string) {
  expandedGroup.value = expandedGroup.value === id ? null : id
}
function normMaturity(m?: string): string {
  return String(m ?? '').toLowerCase()
}
function implementedCount(groupId: string): number {
  const g = registry.value?.find(r => r.id === groupId)
  if (!g) return 0
  return g.capabilities.filter(c => normMaturity(c.maturity) !== 'narrative').length
}

const sections: Array<{ nameKey: string; icon: IconName; descKey: string; statusKey: string; href: string }> = [
  { nameKey: 'capabilitiesHub.sections.modelService.name', icon: 'model', descKey: 'capabilitiesHub.sections.modelService.desc', statusKey: 'capabilitiesHub.sections.status.built', href: '/engine' },
  { nameKey: 'capabilitiesHub.sections.capability.name', icon: 'capabilities', descKey: 'capabilitiesHub.sections.capability.desc', statusKey: 'capabilitiesHub.sections.status.live', href: '/engine' },
  { nameKey: 'capabilitiesHub.sections.settings.name', icon: 'settings', descKey: 'capabilitiesHub.sections.settings.desc', statusKey: 'capabilitiesHub.sections.status.withinSettings', href: '/settings' },
  { nameKey: 'capabilitiesHub.sections.extensions.name', icon: 'boxes', descKey: 'capabilitiesHub.sections.extensions.desc', statusKey: 'capabilitiesHub.sections.status.built', href: '/plugins' },
  { nameKey: 'capabilitiesHub.sections.agentPresets.name', icon: 'agents', descKey: 'capabilitiesHub.sections.agentPresets.desc', statusKey: 'capabilitiesHub.sections.status.built', href: '/expert-center' },
  { nameKey: 'capabilitiesHub.sections.pluginMarket.name', icon: 'plugin', descKey: 'capabilitiesHub.sections.pluginMarket.desc', statusKey: 'capabilitiesHub.sections.status.built', href: '/plugins' },
  { nameKey: 'capabilitiesHub.sections.webuiPlugins.name', icon: 'appWindow', descKey: 'capabilitiesHub.sections.webuiPlugins.desc', statusKey: 'capabilitiesHub.sections.status.built', href: '/browser-cockpit' },
  { nameKey: 'capabilitiesHub.sections.pet.name', icon: 'pet', descKey: 'capabilitiesHub.sections.pet.desc', statusKey: 'capabilitiesHub.sections.status.built', href: '/pet' },
  { nameKey: 'capabilitiesHub.sections.communityPlugins.name', icon: 'boxes', descKey: 'capabilitiesHub.sections.communityPlugins.desc', statusKey: 'capabilitiesHub.sections.status.built', href: '/plugins' },
  { nameKey: 'capabilitiesHub.sections.graph.name', icon: 'gitBranch', descKey: 'capabilitiesHub.sections.graph.desc', statusKey: 'capabilitiesHub.sections.status.built', href: '/graph' },
  { nameKey: 'capabilitiesHub.sections.collabSsh.name', icon: 'collab', descKey: 'capabilitiesHub.sections.collabSsh.desc', statusKey: 'capabilitiesHub.sections.status.built', href: '/ssh-collab' },
  { nameKey: 'capabilitiesHub.sections.terminalSandbox.name', icon: 'command', descKey: 'capabilitiesHub.sections.terminalSandbox.desc', statusKey: 'capabilitiesHub.sections.status.built', href: '/terminal' },
  { nameKey: 'capabilitiesHub.sections.taskQuality.name', icon: 'gauge', descKey: 'capabilitiesHub.sections.taskQuality.desc', statusKey: 'capabilitiesHub.sections.status.built', href: '/engine' },
]

function pct(n: number): string {
  const total = stats.value?.total || 1
  return `${Math.max(0, Math.round((n / total) * 100))}%`
}

async function load() {
  try {
    const [r, reg] = await Promise.all([
      rpc.call<Capabilities>('plans.capabilities', {}),
      rpc
        .call<{ groups?: CapabilityGroupRow[] }>('capabilities.groups', {})
        .catch(() => null),
    ])
    stats.value = r?.stats ?? null
    // P3 单一事实源：注册表来自 capabilities.groups，取不到时如实降级（不假装）。
    registry.value = reg?.groups?.length ? reg.groups : null
    registryFailed.value = !reg?.groups?.length
  } catch {
    registryFailed.value = true
  }
}
onMounted(load)
</script>

<style scoped>
.hub-stage {
  padding: 24px 28px;
  max-width: 1200px;
  margin: 0 auto;
}

.hub-kicker {
  margin: 0 0 4px;
  font-family: var(--font-mono);
  font-size: 10px;
  letter-spacing: 0.22em;
  text-transform: uppercase;
  color: var(--accent);
}

.hub-meter {
  display: flex;
  flex-direction: column;
  gap: 10px;
}

.hub-meter__bar {
  display: flex;
  height: 8px;
  border-radius: var(--radius-pill);
  overflow: hidden;
  background: color-mix(in srgb, var(--text) 8%, transparent);
}

.hub-meter__seg { display: block; height: 100%; min-width: 0; }
.hub-meter__seg--real { background: var(--ok, var(--accent)); }
.hub-meter__seg--partial { background: var(--warn, var(--accent)); }
.hub-meter__seg--narrative { background: color-mix(in srgb, var(--text) 22%, transparent); }

.hub-pills {
  display: flex;
  flex-wrap: wrap;
  gap: 8px;
}

.hub-pill {
  padding: 4px 10px;
  border-radius: var(--radius-pill);
  background: color-mix(in srgb, var(--text) 8%, var(--card));
  font-size: 12px;
}

.hub-pill--accent { color: var(--accent); }
.hub-pill--dim { color: var(--text-dim); }

.hub-registry {
  margin-top: 18px;
  padding-top: 16px;
  border-top: 1px dashed var(--hairline);
}

.hub-registry__head {
  display: flex;
  flex-direction: column;
  gap: 3px;
  margin-bottom: 10px;
}

.hub-registry__head b { font-family: var(--font-display); font-size: 14px; }
.hub-registry__head span { font-size: 11px; color: var(--text-dim); line-height: 1.5; }

.hub-registry__grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(300px, 1fr));
  gap: 10px;
}

.hub-reg-group {
  display: flex;
  flex-direction: column;
  gap: 8px;
  padding: 12px 14px;
  background: var(--card);
  border: 1px solid var(--hairline);
  border-radius: var(--radius-md);
  color: inherit;
  font: inherit;
  text-align: left;
  cursor: pointer;
  transition:
    border-color var(--dur-fast) var(--ease-standard),
    transform var(--dur-fast) var(--ease-standard);
}

.hub-reg-group:hover,
.hub-reg-group:focus-visible {
  outline: 0;
  border-color: color-mix(in srgb, var(--accent) 45%, var(--hairline));
}

.hub-reg-group.is-open { border-color: var(--accent); }

.hub-reg-group__row {
  display: flex;
  align-items: baseline;
  justify-content: space-between;
  gap: 10px;
}

.hub-reg-group__row b { font-size: 13px; }
.hub-reg-group__meta { font-size: 11px; color: var(--text-dim); white-space: nowrap; }

.hub-reg-group__items {
  display: flex;
  flex-wrap: wrap;
  gap: 5px;
}

.hub-reg-cap {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  padding: 3px 8px;
  border-radius: var(--radius-pill);
  background: color-mix(in srgb, var(--text) 7%, transparent);
  font-size: 11px;
}

.hub-reg-cap b { font-weight: 600; color: var(--text); }
.hub-reg-cap i { font-style: normal; font-size: 10px; color: var(--text-dim); }

.hub-reg-cap--real { border: 1px solid color-mix(in srgb, var(--ok) 35%, transparent); }
.hub-reg-cap--partial { border: 1px solid color-mix(in srgb, var(--warn) 35%, transparent); }
.hub-reg-cap--narrative { opacity: 0.65; }

.hub-registry-empty {
  margin-top: 14px;
  padding: 10px 14px;
  border: 1px dashed var(--hairline);
  border-radius: var(--radius-sm);
  color: var(--text-dim);
  font-size: 12px;
}

.hub-grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(280px, 1fr));
  gap: 12px;
}

.hub-card {
  display: grid;
  grid-template-columns: auto 1fr auto;
  gap: 12px;
  align-items: start;
  padding: 16px;
  background:
    linear-gradient(145deg, color-mix(in srgb, var(--accent) 7%, transparent), transparent 48%),
    var(--card);
  border: 1px solid var(--hairline);
  border-radius: var(--radius-card);
  text-decoration: none;
  color: inherit;
  animation: hub-rise var(--dur-enter) var(--ease-out) both;
  animation-delay: calc(var(--i, 0) * 35ms);
  transition:
    border-color var(--dur-fast) var(--ease-standard),
    transform var(--dur-fast) var(--ease-spring);
}

.hub-card:hover {
  border-color: var(--accent);
  transform: translateY(-2px);
}

.hub-card__icon { color: var(--accent); margin-top: 2px; }

.hub-card__body {
  display: flex;
  flex-direction: column;
  gap: 4px;
  min-width: 0;
}

.hub-card__body b { font-family: var(--font-display); font-size: 14px; }
.hub-card__body span { font-size: 11px; color: var(--text-dim); line-height: 1.45; }
.hub-card__body i { font-style: normal; font-size: 11px; color: var(--accent); }

.hub-card__arrow {
  color: var(--text-dim);
  font-size: 14px;
  transition: transform var(--dur-fast) var(--ease-spring), color var(--dur-fast) var(--ease-standard);
}

.hub-card:hover .hub-card__arrow {
  color: var(--accent);
  transform: translateX(3px);
}

@keyframes hub-rise {
  from { opacity: 0; transform: translateY(8px); }
  to { opacity: 1; transform: translateY(0); }
}

@media (prefers-reduced-motion: reduce) {
  .hub-card { animation: none; }
}
</style>
