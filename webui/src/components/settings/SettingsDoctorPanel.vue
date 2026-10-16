<template>
  <section class="doctor-settings" aria-labelledby="doctor-settings-title">
    <header class="doctor-settings__header">
      <div>
        <h3 id="doctor-settings-title">{{ t('setup.doctor.title') }}</h3>
        <p>{{ t('setup.doctor.desc') }}</p>
      </div>
      <!-- Read-only client panel: no readiness/dirty bar, no save. -->
      <span v-if="report?.agentId" class="doctor-settings__agent" aria-label="agent">{{ report.agentId }}</span>
    </header>

    <!-- Run control: status row + Refresh + Run deep check. -->
    <div class="doctor-controls">
      <div class="doctor-status" role="status">
        <Icon :name="statusGlyphIcon" :size="15" aria-hidden="true" />
        <strong class="doctor-status__word">{{ statusLabelText }}</strong>
        <span v-if="report?.summary" class="doctor-status__summary">{{ report.summary }}</span>
      </div>
      <div class="doctor-controls__actions">
        <button type="button" class="btn" :disabled="busy" @click="void load(false)">
          <Icon name="refresh" :size="14" />
          <span>{{ t('setup.doctor.refresh') }}</span>
        </button>
        <button type="button" class="btn btn--primary" :disabled="busy" @click="void load(true)">
          <Icon name="gauge" :size="14" />
          <span>{{ t('setup.doctor.runDeep') }}</span>
        </button>
      </div>
    </div>
    <!-- Three states: loading / error / findings. -->
    <div v-if="loading" class="doctor-state" role="status">{{ t('setup.doctor.loading') }}</div>
    <div v-else-if="error" class="doctor-state doctor-state--error" role="alert">
      <strong>{{ t('setup.doctor.errorTitle') }}</strong>
      <span>{{ error }}</span>
      <button type="button" class="btn" @click="void load(false)">{{ t('setup.doctor.retry') }}</button>
    </div>
    <template v-else>
      <article v-if="groupedFindings.length === 0" class="doctor-empty">{{ t('setup.doctor.noFindings') }}</article>
      <section v-else class="doctor-findings">
        <section v-for="group in groupedFindings" :key="group.title" class="doctor-group">
          <header class="doctor-group__header">
            <div><h4>{{ group.title }}</h4><p>{{ group.note }}</p></div>
            <span>{{ group.findings.length }}</span>
          </header>
          <article
            v-for="(finding, fIdx) in group.findings"
            :key="finding.id || fIdx"
            class="doctor-finding"
            :class="`is-${findingTone(findingGroupKind(finding))}`"
          >
            <div class="doctor-finding__meta">
              <span class="doctor-chip">{{ impactLabel(impactValue(finding)) }}</span>
              <span class="doctor-chip doctor-chip--surface">{{ finding.surface || t('sessions.overview.impact.reference') }}</span>
              <span v-if="findingBadges(finding)" class="doctor-chip" :class="findingBadgeClass(finding)">{{ findingBadgeText(finding) }}</span>
              <button v-if="findingSettingsLink(finding)" type="button" class="doctor-link" @click="openFindingSettings(finding)">{{ t('sessions.overview.openSettings') }}</button>
            </div>
            <div class="doctor-finding__title">{{ finding.title || finding.id || t('sessions.overview.findingFallback', { n: fIdx + 1 }) }}</div>
            <div v-if="finding.detail" class="doctor-finding__detail">{{ finding.detail }}</div>
            <details v-if="hasFindingExtras(finding)" class="doctor-finding__more" :open="!isMinorFinding(finding)">
              <summary>{{ t('sessions.overview.findingDetails') }}</summary>
              <div v-if="visibleEvidenceEntries(finding.evidence).length" class="doctor-evidence" aria-label="Finding evidence">
                <span v-for="([key, value], eIdx) in visibleEvidenceEntries(finding.evidence).slice(0, 6)" :key="eIdx">
                  <b>{{ evidenceLabel(key) }}</b>{{ evidenceValue(value) }}
                </span>
              </div>
              <AdvancedCliSteps v-if="normalizedFixSteps(finding).length" :steps="normalizedFixSteps(finding)" />
            </details>
          </article>
        </section>
      </section>
    </template>
  </section>
</template>

<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { useRouter } from 'vue-router'
import { useRpcStore } from '@/stores/rpc'
import { usePlatform } from '@/platform'
import { isOwnedGatewayConnection } from '@/composables/useCliInvocation'
import { withoutLegacyMigrationFinding, settingsLinkForFinding } from '@/utils/overviewDiagnostics'
import Icon from '@/components/Icon.vue'
import AdvancedCliSteps from '@/components/overview/AdvancedCliSteps.vue'

interface FixStep { label: string; command?: string; detail?: string }
interface Finding {
  id?: string; severity?: 'error' | 'warn' | 'info' | 'ok'
  readinessImpact?: 'blocks_ready' | 'degrades' | 'optional' | 'none'
  surface?: string; title?: string; detail?: string
  evidence?: Record<string, unknown>; fixSteps?: FixStep[]; restartRequired?: boolean
}
interface HealthReport {
  status?: string; ready?: boolean; summary?: string; gatewayUrl?: string
  configPath?: string; requestedConfigPath?: string; agentId?: string
  counts?: Record<string, number>; impactCounts?: Record<string, number>; findings?: Finding[]
}
interface FindingGroup { title: string; note: string; findings: Finding[] }

const { t } = useI18n()
const router = useRouter()
const rpc = useRpcStore()
const platform = usePlatform()
const HIDDEN_EVIDENCE_KEYS = new Set(['restart_required', 'restartRequired'])

const loading = ref(true)
const busy = ref(false)
const error = ref<string | null>(null)
const report = ref<HealthReport | null>(null)

async function load(deep: boolean) {
  if (busy.value) return
  // Initial mount shows the skeleton; re-runs after a completed load don't.
  if (report.value === null) loading.value = true
  busy.value = true
  error.value = null
  try {
    await rpc.waitForConnection()
    const data = withoutLegacyMigrationFinding(await rpc.call<HealthReport>('doctor.status', { agentId: 'main', deep }))
    report.value = data
  } catch (err) {
    error.value = err instanceof Error ? err.message : String(err)
    report.value = null
  } finally {
    loading.value = false
    busy.value = false
  }
}

const stateToken = computed(() => error.value ? 'unavailable' : loading.value ? 'loading' : classToken(report.value?.status || 'unknown'))
const statusGlyphIcon = computed<'check' | 'shield' | 'x'>(() => {
  const cls = stateToken.value
  if (cls === 'action_required' || cls === 'unavailable') return 'x'
  if (cls === 'degraded') return 'shield'
  return 'check'
})
const statusLabelText = computed(() => {
  if (loading.value) return t('sessions.overview.checking')
  if (error.value) return t('sessions.overview.statusLabel.unavailable')
  return statusLabel(report.value?.status || 'unknown', report.value?.ready)
})

const groupedFindings = computed<FindingGroup[]>(() => {
  const findings = report.value?.findings || []
  if (!findings.length) return []
  const kinds: Array<'action' | 'degraded' | 'optional' | 'ready'> = ['action', 'degraded', 'optional', 'ready']
  return kinds.map((kind) => ({
    title: t(`sessions.overview.group.${kind}.title`),
    note: t(`sessions.overview.group.${kind}.note`),
    findings: findings.filter(f => findingGroupKind(f) === kind),
  })).filter(g => g.findings.length)
})

function classToken(value: string | undefined | null): string {
  return String(value || 'unknown').toLowerCase().replace(/[^a-z0-9_-]+/g, '-')
}
function impactValue(finding: Finding): string {
  const impact = String(finding?.readinessImpact || '')
  if (['blocks_ready', 'degrades', 'optional', 'none'].includes(impact)) return impact
  const severity = String(finding?.severity || 'info')
  if (severity === 'error') return 'blocks_ready'
  if (severity === 'warn') return 'degrades'
  if (severity === 'info') return 'optional'
  return 'none'
}
function findingGroupKind(finding: Finding): 'action' | 'degraded' | 'optional' | 'ready' {
  const impact = impactValue(finding)
  if (impact === 'blocks_ready') return 'action'
  if (impact === 'degrades') return 'degraded'
  if (impact === 'optional') return 'optional'
  return 'ready'
}
function findingTone(kind: 'action' | 'degraded' | 'optional' | 'ready'): 'error' | 'warn' | 'info' | 'ok' {
  if (kind === 'action') return 'error'
  if (kind === 'degraded') return 'warn'
  if (kind === 'optional') return 'info'
  return 'ok'
}
function impactLabel(impact: string): string {
  const keys: Record<string, string> = {
    blocks_ready: 'sessions.overview.impact.blocksReady',
    degrades: 'sessions.overview.impact.degrades',
    optional: 'sessions.overview.impact.optional',
    none: 'sessions.overview.impact.reference',
  }
  return t(keys[impact] || 'sessions.overview.impact.reference')
}
function statusLabel(status: string, ready: boolean | undefined): string {
  if (ready && status === 'degraded') return t('sessions.overview.statusLabel.readyWithWarnings')
  if (ready) return t('sessions.overview.statusLabel.ready')
  const keys: Record<string, string> = {
    action_required: 'sessions.overview.statusLabel.actionRequired',
    degraded: 'sessions.overview.statusLabel.degraded',
    unavailable: 'sessions.overview.statusLabel.unavailable',
    ready: 'sessions.overview.statusLabel.ready',
  }
  return keys[status] ? t(keys[status]) : status
}
function normalizedFixSteps(finding: Finding): Array<{ label: string; command?: string; detail?: string }> {
  const targetFresh = finding.evidence?.target_fresh ?? finding.evidence?.targetFresh
  const hideMigrationApply = finding.id === 'migration.legacy_home_detected' && targetFresh === false
  return (finding.fixSteps || []).filter((step) => !hideMigrationApply || !/(?:^|\s)--apply(?:\s|$)/.test(String(step.command || '')))
    .map(step => ({ label: step.label || t('sessions.overview.step'), command: step.command, detail: step.detail }))
}
function evidenceLabel(key: string): string {
  const label = String(key || '').replace(/([a-z0-9])([A-Z])/g, '$1 $2').replace(/[_-]+/g, ' ').replace(/\s+/g, ' ').trim()
  return label ? label.charAt(0).toUpperCase() + label.slice(1) : ''
}
function evidenceValue(value: unknown): string {
  if (typeof value === 'string') return value
  if (typeof value === 'number' || typeof value === 'boolean') return String(value)
  try {
    const text = JSON.stringify(value)
    return text.length > 120 ? `${text.slice(0, 117)}...` : text
  } catch { return String(value) }
}
function visibleEvidenceEntries(evidence: Record<string, unknown> | undefined): [string, unknown][] {
  return Object.entries(evidence || {}).filter(([key, value]) => value !== undefined && value !== null && !HIDDEN_EVIDENCE_KEYS.has(key))
}
function isMinorFinding(finding: Finding): boolean {
  return ['optional', 'none'].includes(impactValue(finding))
}
function hasFindingExtras(finding: Finding): boolean {
  return visibleEvidenceEntries(finding.evidence).length > 0 || normalizedFixSteps(finding).length > 0
}
function findingBadges(finding: Finding): boolean {
  const id = String(finding?.id || '')
  return id.endsWith('.diagnostic.incomplete') || id.endsWith('.repair.pending') || id === 'gateway.config.mismatch'
}
function findingBadgeText(finding: Finding): string {
  const id = String(finding?.id || '')
  if (id.endsWith('.diagnostic.incomplete')) return t('sessions.overview.badge.diagnostic')
  if (id.endsWith('.repair.pending')) return t('sessions.overview.badge.repair')
  if (id === 'gateway.config.mismatch') return t('sessions.overview.badge.config')
  return ''
}
function findingBadgeClass(finding: Finding): string {
  const id = String(finding?.id || '')
  if (id.endsWith('.diagnostic.incomplete')) return 'doctor-chip--diagnostic'
  if (id.endsWith('.repair.pending')) return 'doctor-chip--repair'
  if (id === 'gateway.config.mismatch') return 'doctor-chip--config'
  return ''
}
function findingSettingsLink(finding: Finding) {
  return settingsLinkForFinding(finding, {
    isDesktop: platform.capabilities.isDesktop,
    ownsGatewayConnection: platform.capabilities.ownsGateway && isOwnedGatewayConnection(),
  })
}
function openFindingSettings(finding: Finding) {
  const link = findingSettingsLink(finding)
  if (!link) return
  router.push({ path: link.path, hash: link.hash, query: link.query }).catch(() => {})
}

onMounted(() => void load(false))
</script>

<style scoped>
.doctor-settings { display: grid; gap: var(--sp-4); max-width: 840px; margin: 0 auto; padding: 0.25rem 0 2rem; }
.doctor-settings__header { display: flex; align-items: flex-start; justify-content: space-between; gap: var(--sp-3); }
.doctor-settings__header h3 { font-size: var(--fs-lg); margin: 0; }
.doctor-settings__header p { color: var(--text-muted); font-size: var(--fs-sm); margin: 4px 0 0; line-height: 1.5; }
.doctor-settings__agent { flex: 0 0 auto; color: var(--text-dim); font-family: var(--font-mono); font-size: var(--fs-xs); padding: 2px 8px; border: 1px solid var(--border); border-radius: var(--radius-full); }
.doctor-controls, .doctor-findings { display: grid; gap: var(--sp-4); }
.doctor-controls { align-items: center; justify-content: space-between; flex-wrap: wrap; }
.doctor-status { align-items: center; color: var(--text-dim); display: inline-flex; gap: 8px; min-width: 0; }
.doctor-status__word { color: var(--text); font-size: var(--fs-md); font-weight: 700; white-space: nowrap; }
.doctor-status__summary { color: var(--text-muted); font-size: var(--fs-sm); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.doctor-controls__actions { display: flex; gap: var(--sp-2); }
.doctor-state, .doctor-empty { color: var(--text-muted); padding: var(--sp-4); border: 1px solid var(--border); border-radius: var(--radius-lg); text-align: center; }
.doctor-state--error { display: grid; gap: var(--sp-2); justify-items: center; color: var(--text); }
.doctor-group { display: grid; gap: var(--sp-3); }
.doctor-group__header { align-items: end; border-bottom: 1px solid var(--border); display: flex; gap: var(--sp-3); justify-content: space-between; padding: 0 2px var(--sp-2); }
.doctor-group__header h4 { font-size: var(--fs-md); letter-spacing: 0; margin: 0; }
.doctor-group__header p { color: var(--text-muted); font-size: var(--fs-sm); margin: 3px 0 0; }
.doctor-group__header > span { color: var(--text-dim); font-family: var(--font-mono); font-size: var(--fs-sm); font-variant-numeric: tabular-nums; }
.doctor-finding { background: var(--bg-surface); border: 1px solid var(--border); border-radius: var(--radius-lg); padding: var(--sp-4); color: var(--text); }
.doctor-finding.is-error { border-inline-start: 3px solid var(--danger); }.doctor-finding.is-warn { border-inline-start: 3px solid var(--warn); }.doctor-finding.is-info { border-inline-start: 3px solid var(--accent); }.doctor-finding.is-ok { border-inline-start: 3px solid var(--ok); }
.doctor-finding__meta { align-items: center; color: var(--text-dim); display: flex; flex-wrap: wrap; gap: 6px; }
.doctor-chip { border: 1px solid var(--border); border-radius: var(--radius-full); color: var(--text-muted); display: inline-flex; font-size: 10.5px; font-weight: 700; letter-spacing: 0.08em; padding: 2px 8px; text-transform: uppercase; }
.doctor-chip--surface { color: var(--text-dim); }.doctor-chip--diagnostic { color: var(--warn); }.doctor-chip--repair { color: var(--accent); }.doctor-chip--config { color: var(--danger); }
.doctor-link { background: none; border: none; color: var(--accent); cursor: pointer; font: inherit; letter-spacing: 0.08em; padding: 2px 0; font-size: 10.5px; font-weight: 700; text-transform: uppercase; }
.doctor-link:hover { color: var(--accent-hover); }
.doctor-finding__title { font-size: var(--fs-lg); font-weight: 700; letter-spacing: 0; margin-top: var(--sp-2); min-width: 0; overflow-wrap: anywhere; }
.doctor-finding__detail { color: var(--text-muted); line-height: 1.5; margin-top: 4px; min-width: 0; overflow-wrap: anywhere; }
.doctor-finding__more { margin-top: var(--sp-3); min-width: 0; }
.doctor-finding__more > summary { color: var(--text-dim); cursor: pointer; font-size: var(--fs-xs); list-style: none; width: fit-content; }
.doctor-finding__more > summary::-webkit-details-marker { display: none; }
.doctor-finding__more > summary::before { content: "\25B8"; display: inline-block; margin-right: 5px; transition: transform var(--dur-fast); }
.doctor-finding__more[open] > summary::before { transform: rotate(90deg); }
.doctor-evidence { column-gap: var(--sp-4); display: flex; flex-wrap: wrap; margin-top: var(--sp-2); min-width: 0; overflow-wrap: anywhere; row-gap: 4px; }
.doctor-evidence span { color: var(--text-muted); display: inline-flex; font-family: var(--font-mono); font-size: 11px; gap: 6px; line-height: 1.5; max-width: 100%; min-width: 0; overflow-wrap: anywhere; }
.doctor-evidence span b { color: var(--text-dim); font-family: inherit; font-weight: 700; }
@media (max-width: 720px) { .doctor-settings__header { flex-direction: column; }.doctor-controls { align-items: flex-start; } }
</style>
