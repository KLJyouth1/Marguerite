<template>
  <div class="control-stage tactics-stage">
    <header class="control-stage__header">
      <div class="control-stage__title-block">
        <h1 class="control-stage__title">{{ t('tactics.title') }}</h1>
        <p class="control-stage__subtitle">{{ t('tactics.subtitle') }}</p>
      </div>
      <div class="control-stage__actions">
        <button class="btn btn--ghost" :disabled="busy" data-testid="tactics-reset" @click="resetForm">
          <Icon name="refresh" :size="16" /><span>{{ t('tactics.reset') }}</span>
        </button>
        <button class="btn btn--primary" :disabled="busy" data-testid="tactics-run" @click="run">
          <Icon name="play" :size="16" /><span>{{ busy ? t('tactics.running') : t('tactics.run') }}</span>
        </button>
      </div>
    </header>

    <div v-if="note" class="tactics-note tactics-note--warn" data-testid="tactics-note">{{ note }}</div>

    <!-- 场景：DAG + 预算 + 先验。留空 = 用后端声明先验（后端会标 n=0）。 -->
    <section class="tactics-card">
      <div class="tactics-card__head">
        <div class="tactics-card__title">{{ t('tactics.scenario.title') }}</div>
      </div>

      <div class="tactics-card__subtitle">{{ t('tactics.scenario.tasks') }}</div>
      <table class="tactics-table tactics-dag" data-testid="tactics-dag">
        <thead>
          <tr>
            <th>{{ t('tactics.scenario.taskId') }}</th>
            <th>{{ t('tactics.scenario.work') }}</th>
            <th>{{ t('tactics.scenario.dependsOn') }}</th>
            <th class="tactics-col-narrow" aria-hidden="true"></th>
          </tr>
        </thead>
        <tbody>
          <tr v-for="(task, index) in form.tasks" :key="index">
            <td><input v-model="task.id" class="tactics-input" :aria-label="t('tactics.scenario.taskId')" /></td>
            <td>
              <input
                v-model="task.workK"
                class="tactics-input tactics-input--num"
                inputmode="decimal"
                :aria-label="t('tactics.scenario.work')"
              />
            </td>
            <td>
              <input
                v-model="task.dependsOn"
                class="tactics-input"
                :placeholder="t('tactics.scenario.dependsHint')"
                :aria-label="t('tactics.scenario.dependsOn')"
              />
            </td>
            <td class="tactics-col-narrow">
              <button
                class="btn btn--ghost tactics-icon-btn"
                :disabled="form.tasks.length <= 1"
                :title="t('tactics.scenario.remove')"
                :aria-label="t('tactics.scenario.remove')"
                @click="removeTask(index)"
              >
                <Icon name="trash" :size="15" />
              </button>
            </td>
          </tr>
        </tbody>
      </table>
      <button class="btn btn--ghost" data-testid="tactics-add-task" @click="addTask">
        <Icon name="plus" :size="15" /><span>{{ t('tactics.scenario.addTask') }}</span>
      </button>

      <div class="tactics-grid">
        <label class="tactics-field">
          <span>{{ t('tactics.scenario.budget') }}</span>
          <input v-model="form.budgetTokens" class="tactics-input" inputmode="decimal" placeholder="—" />
        </label>
        <label class="tactics-field">
          <span>{{ t('tactics.scenario.deadline') }}</span>
          <input v-model="form.deadlineMs" class="tactics-input" inputmode="decimal" placeholder="—" />
        </label>
        <label class="tactics-field">
          <span>{{ t('tactics.scenario.trials') }}</span>
          <input v-model="form.trials" class="tactics-input" inputmode="numeric" />
        </label>
        <label class="tactics-field">
          <span>{{ t('tactics.scenario.maxFail') }}</span>
          <input v-model="form.maxFailProbability" class="tactics-input" inputmode="decimal" placeholder="0.15" />
        </label>
        <label class="tactics-field">
          <span>{{ t('tactics.scenario.seed') }}</span>
          <input v-model="form.seed" class="tactics-input" inputmode="numeric" />
        </label>
      </div>

      <div class="tactics-card__subtitle">{{ t('tactics.scenario.priorsTitle') }}</div>
      <p class="tactics-hint">{{ t('tactics.scenario.priorsHint') }}</p>
      <div class="tactics-grid">
        <label v-for="field in PRIOR_FIELDS" :key="field" class="tactics-field">
          <span>{{ t(`tactics.scenario.priors.${field}`) }}</span>
          <input v-model="form.priors[field]" class="tactics-input" inputmode="decimal" placeholder="—" />
        </label>
      </div>

      <ul v-if="issues.length" class="tactics-issues" data-testid="tactics-issues">
        <li v-for="(issue, index) in issues" :key="`${issue.code}-${issue.field ?? index}`">
          {{ issueText(issue) }}
        </li>
      </ul>
    </section>

    <!-- 目录：与结果分离，目录读不到也不影响推演（simulate 自带全部字段）。 -->
    <section v-if="catalog?.tactics?.length" class="tactics-card">
      <div class="tactics-card__head">
        <div class="tactics-card__title">{{ t('tactics.catalog.title') }}</div>
      </div>
      <table class="tactics-table" data-testid="tactics-catalog">
        <thead>
          <tr>
            <th>{{ t('tactics.catalog.tactic') }}</th>
            <th>{{ t('tactics.catalog.topology') }}</th>
            <th>{{ t('tactics.catalog.slots') }}</th>
            <th>{{ t('tactics.catalog.gate') }}</th>
            <th>{{ t('tactics.catalog.bet') }}</th>
          </tr>
        </thead>
        <tbody>
          <tr v-for="entry in catalog.tactics" :key="entry.id">
            <td><code class="tactics-mono">{{ entry.id }}</code></td>
            <td>{{ topologyLabel(entry.topology) }}</td>
            <td class="tactics-num">{{ entry.width ?? 1 }}</td>
            <td>
              <span v-if="entry.gate" class="tactics-pill tactics-pill--on">
                {{ t('tactics.catalog.gateDetect', { percent: formatPct(entry.gate.detect) }) }}
              </span>
              <span v-else class="tactics-pill">{{ t('tactics.catalog.gateNone') }}</span>
            </td>
            <td>{{ entry.bet ?? '—' }}</td>
          </tr>
        </tbody>
      </table>
    </section>

    <!-- 裁决 -->
    <section v-if="plan" class="tactics-card" data-testid="tactics-result">
      <div class="tactics-card__head">
        <div class="tactics-card__title">{{ t('tactics.result.title') }}</div>
      </div>

      <div
        class="tactics-verdict"
        :class="{ 'tactics-verdict--warn': plan.rationale?.noViableTactic }"
        data-testid="tactics-recommendation"
      >
        <div class="tactics-verdict__headline">
          <template v-if="plan.rationale?.recommended">
            {{ t('tactics.result.recommended', { tactic: plan.rationale.recommended }) }}
          </template>
          <template v-else>{{ t('tactics.result.noViable') }}</template>
        </div>
        <div class="tactics-verdict__reason">{{ plan.rationale?.reason ?? '—' }}</div>
      </div>

      <!-- 未知 ≠ 0：没有实测样本的先验必须紧挨着推荐一起出现 -->
      <div class="tactics-card__subtitle">{{ t('tactics.result.unknownsTitle') }}</div>
      <div v-if="plan.unknowns?.length" class="tactics-unknowns" data-testid="tactics-unknowns">
        <div v-for="unknown in plan.unknowns" :key="unknown.field" class="tactics-unknown">
          <code class="tactics-mono">{{ unknown.field }}</code>
          <span>{{ unknown.reason }}</span>
        </div>
      </div>
      <p v-else class="tactics-hint">{{ t('tactics.result.unknownNone') }}</p>

      <table class="tactics-table" data-testid="tactics-results">
        <thead>
          <tr>
            <th>{{ t('tactics.result.table.tactic') }}</th>
            <th>{{ t('tactics.result.table.success') }}</th>
            <th>{{ t('tactics.result.table.p50') }}</th>
            <th>{{ t('tactics.result.table.p90') }}</th>
            <th>{{ t('tactics.result.table.tokens') }}</th>
            <th>{{ t('tactics.result.table.cost') }}</th>
            <th>{{ t('tactics.result.table.coord') }}</th>
            <th>{{ t('tactics.result.table.bottleneck') }}</th>
            <th>{{ t('tactics.result.table.verdict') }}</th>
          </tr>
        </thead>
        <tbody>
          <tr
            v-for="row in plan.results ?? []"
            :key="row.tactic"
            :class="{ 'tactics-row--winner': row.tactic === plan.rationale?.recommended }"
          >
            <td><code class="tactics-mono">{{ row.tactic }}</code></td>
            <td class="tactics-num">{{ formatPct(row.successRate, 1) }}</td>
            <td class="tactics-num">{{ formatMs(row.p50Ms) }}</td>
            <td class="tactics-num">{{ formatMs(row.p90Ms) }}</td>
            <td class="tactics-num">{{ formatTokens(row.p90Tokens) }}</td>
            <td class="tactics-num">{{ formatUsd(row.p90CostUsd) }}</td>
            <td class="tactics-num">{{ formatPct(row.coordinationShare) }}</td>
            <td>{{ row.bottleneck || '—' }}</td>
            <td>
              <span
                class="tactics-pill"
                :class="viable(row.tactic) ? 'tactics-pill--on' : 'tactics-pill--off'"
                :title="verdictDetail(row.tactic)"
              >
                {{ viable(row.tactic) ? t('tactics.result.verdict.viable') : t('tactics.result.verdict.killed') }}
              </span>
            </td>
          </tr>
        </tbody>
      </table>

      <template v-if="plan.rationale?.whyNot?.length">
        <div class="tactics-card__subtitle">{{ t('tactics.result.whyNotTitle') }}</div>
        <ul class="tactics-why-not" data-testid="tactics-why-not">
          <li v-for="item in plan.rationale.whyNot" :key="`${item.tactic}-${item.reason}`">
            <code class="tactics-mono">{{ item.tactic }}</code>
            <span class="tactics-why-not__reason">{{ whyReasonText(item.reason) }}</span>
            <span class="tactics-why-not__detail">{{ item.detail }}</span>
          </li>
        </ul>
      </template>

      <div class="tactics-card__subtitle">{{ t('tactics.result.premortemTitle') }}</div>
      <div v-if="plan.premortem?.length" class="tactics-premortem" data-testid="tactics-premortem">
        <div v-for="item in plan.premortem" :key="item.id" class="tactics-premortem__item">
          <div class="tactics-premortem__head">
            <span class="tactics-pill" :class="`tactics-pill--${item.severity === 'high' ? 'off' : 'on'}`">
              {{ severityText(item.severity) }}
            </span>
            <span class="tactics-premortem__freq">{{ formatPct(item.frequency) }}</span>
          </div>
          <div class="tactics-premortem__evidence">{{ item.evidence }}</div>
          <div class="tactics-premortem__mitigation">{{ item.mitigation }}</div>
        </div>
      </div>
      <p v-else class="tactics-hint">{{ t('tactics.result.noPremortem') }}</p>
    </section>

    <div v-else class="tactics-empty">{{ t('tactics.result.empty') }}</div>

    <!-- 反事实回放 -->
    <section class="tactics-card">
      <div class="tactics-card__head">
        <div class="tactics-card__title">{{ t('tactics.replay.title') }}</div>
      </div>
      <p class="tactics-hint">{{ t('tactics.replay.hint') }}</p>
      <div class="tactics-replay-bar">
        <input
          v-model="runId"
          class="tactics-input"
          :placeholder="t('tactics.replay.placeholder')"
          :aria-label="t('tactics.replay.runId')"
          data-testid="tactics-run-id"
        />
        <button class="btn btn--ghost" :disabled="replayBusy || !runId.trim()" data-testid="tactics-replay" @click="runReplay">
          <Icon name="play" :size="15" /><span>{{ replayBusy ? t('tactics.replay.running') : t('tactics.replay.run') }}</span>
        </button>
      </div>
      <div v-if="replayError" class="tactics-note tactics-note--warn" data-testid="tactics-replay-error">{{ replayError }}</div>

      <template v-if="replay">
        <div v-if="replay.insufficientEvidence" class="tactics-note" data-testid="tactics-replay-insufficient">
          {{ t('tactics.replay.insufficient') }}
        </div>
        <p class="tactics-hint">
          {{ t('tactics.replay.source', { source: replay.scenarioSource ?? '—' }) }}
        </p>

        <template v-if="replay.deltas?.length">
          <div class="tactics-card__subtitle">{{ t('tactics.replay.delta.title') }}</div>
          <table class="tactics-table" data-testid="tactics-replay-deltas">
            <thead>
              <tr>
                <th>{{ t('tactics.replay.delta.tactic') }}</th>
                <th>{{ t('tactics.replay.delta.success') }}</th>
                <th>{{ t('tactics.replay.delta.time') }}</th>
                <th>{{ t('tactics.replay.delta.tokens') }}</th>
              </tr>
            </thead>
            <tbody>
              <tr v-for="row in replay.deltas" :key="row.tactic">
                <td><code class="tactics-mono">{{ row.tactic }}</code></td>
                <td class="tactics-num">{{ formatPct(row.successRate, 1) }} <span class="tactics-dim">({{ formatDeltaPct(row.successDelta) }})</span></td>
                <td class="tactics-num">{{ formatMs(row.p50Ms) }} <span class="tactics-dim">({{ formatDeltaPct(row.timeDeltaPct) }})</span></td>
                <td class="tactics-num">{{ formatTokens(row.p90Tokens) }} <span class="tactics-dim">({{ formatDeltaPct(row.tokenDeltaPct) }})</span></td>
              </tr>
            </tbody>
          </table>
        </template>

        <template v-if="replay.whyChain?.length">
          <div class="tactics-card__subtitle">{{ t('tactics.replay.whyTitle') }}</div>
          <ol class="tactics-why" data-testid="tactics-replay-why">
            <li v-for="(link, index) in replay.whyChain" :key="index" class="tactics-why__item">
              <div class="tactics-why__question">{{ link.question }}</div>
              <div class="tactics-why__answer">{{ link.answer }}</div>
              <div class="tactics-why__meta">
                <span class="tactics-pill" :class="whyKindClass(link.kind)">{{ whyKindText(link.kind) }}</span>
                <code v-if="link.evidence" class="tactics-mono tactics-dim">{{ link.evidence }}</code>
              </div>
            </li>
          </ol>
        </template>

        <template v-if="replay.gaps?.length">
          <div class="tactics-card__subtitle">{{ t('tactics.replay.gapsTitle') }}</div>
          <ul class="tactics-gaps" data-testid="tactics-replay-gaps">
            <li v-for="gap in replay.gaps" :key="gap.field">
              <code class="tactics-mono">{{ gap.field }}</code>
              <span>{{ gap.reason }}</span>
            </li>
          </ul>
        </template>
      </template>
    </section>
  </div>
</template>

<script setup lang="ts">
/**
 * 战术推演 —— 把「蜂群 / 专家团 / 子智能体 / 主智能体怎么组合」交给可推翻的数据。
 *
 * 三条界面纪律（与后端契约同源）：
 * 1. **未知 ≠ 0**：没有实测样本的先验紧挨着推荐展示，不折叠、不淡化。
 * 2. **落选必须有理由**：每条非推荐战术的 `whyNot` 原样展示（含被哪条判据否掉）。
 * 3. **回放不补零**：台账没有的字段进 gaps；没有条目就明说「无法比较」，
 *    不渲染一张看起来像结论的空表。
 */
import { computed, onMounted, reactive, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import Icon from '@/components/Icon.vue'
import { useRpcStore } from '@/stores/rpc'
import {
  PRIOR_FIELDS,
  TACTICS_CATALOG_METHOD,
  TACTICS_REPLAY_METHOD,
  TACTICS_SIMULATE_METHOD,
  buildSimulateParams,
  defaultScenarioForm,
  formatDeltaPct,
  formatMs,
  formatPct,
  formatTokens,
  formatUsd,
  topologyKey,
  validateScenarioForm,
  whyKindKey,
  type CatalogPayload,
  type PlanView,
  type ReplayReportView,
  type ScenarioForm,
  type ScenarioIssue,
} from '@/utils/tactics/model'

const { t } = useI18n()
const rpc = useRpcStore()

const form = reactive<ScenarioForm>(defaultScenarioForm())
const catalog = ref<CatalogPayload | null>(null)
const plan = ref<PlanView | null>(null)
const replay = ref<ReplayReportView | null>(null)
const runId = ref('')
const busy = ref(false)
const replayBusy = ref(false)
const catalogError = ref('')
const runError = ref('')
const replayError = ref('')

const supported = computed(() => rpc.supportsMethod(TACTICS_SIMULATE_METHOD))
const issues = computed<ScenarioIssue[]>(() => validateScenarioForm(form))

const note = computed(() => {
  if (catalogError.value) return catalogError.value
  if (rpc.methods.length === 0) return t('tactics.note.connecting')
  if (!supported.value) return t('tactics.note.unsupported')
  if (runError.value) return runError.value
  return ''
})

function resetForm(): void {
  Object.assign(form, defaultScenarioForm())
  plan.value = null
  runError.value = ''
}

function addTask(): void {
  form.tasks.push({ id: '', workK: '', dependsOn: '' })
}

function removeTask(index: number): void {
  if (form.tasks.length <= 1) return
  form.tasks.splice(index, 1)
}

function issueText(issue: ScenarioIssue): string {
  const label = t(`tactics.scenario.issue.${issue.code.replace(/-([a-z])/g, (_, c: string) => c.toUpperCase())}`)
  return issue.field ? `${label} (${issue.field})` : label
}

async function loadCatalog(): Promise<void> {
  try {
    const payload = await rpc.call<CatalogPayload>(TACTICS_CATALOG_METHOD, {})
    catalog.value = payload
    catalogError.value = payload?.ok === false ? (payload as { error?: string }).error ?? '' : ''
  } catch (err) {
    catalogError.value = `${t('tactics.note.catalogFailed')}：${errorText(err)}`
  }
}

async function run(): Promise<void> {
  if (issues.value.length || busy.value) return
  busy.value = true
  runError.value = ''
  try {
    const payload = await rpc.call<PlanView>(TACTICS_SIMULATE_METHOD, buildSimulateParams(form))
    if (payload?.ok === false) {
      plan.value = null
      runError.value = `${t('tactics.note.simulateFailed')}：${payload.error ?? payload.code ?? '—'}`
      return
    }
    plan.value = payload
  } catch (err) {
    plan.value = null
    runError.value = `${t('tactics.note.simulateFailed')}：${errorText(err)}`
  } finally {
    busy.value = false
  }
}

async function runReplay(): Promise<void> {
  const id = runId.value.trim()
  if (!id || replayBusy.value) return
  replayBusy.value = true
  replayError.value = ''
  try {
    const payload = await rpc.call<ReplayReportView>(TACTICS_REPLAY_METHOD, { runId: id })
    if (payload?.ok === false) {
      replay.value = null
      replayError.value = `${t('tactics.note.replayFailed')}：${payload.error ?? payload.code ?? '—'}`
      return
    }
    replay.value = payload
  } catch (err) {
    replay.value = null
    replayError.value = `${t('tactics.note.replayFailed')}：${errorText(err)}`
  } finally {
    replayBusy.value = false
  }
}

function errorText(err: unknown): string {
  return err instanceof Error ? err.message : String(err)
}

function topologyLabel(topology: string | undefined): string {
  const key = topologyKey(topology)
  return key ? t(key) : topology ?? '—'
}

function viable(tactic: string): boolean {
  return plan.value?.rationale?.verdicts?.find((v) => v.tactic === tactic)?.viable ?? false
}

/** 悬停显示被哪条判据否掉，而不是只说「不可行」。 */
function verdictDetail(tactic: string): string {
  const verdict = plan.value?.rationale?.verdicts?.find((v) => v.tactic === tactic)
  const violated = verdict?.checks?.filter((check) => check.violated) ?? []
  if (!violated.length) return ''
  return violated
    .map((check) => `${check.id}: ${check.actual ?? '—'} > ${check.threshold ?? '—'}`)
    .join('\n')
}

const WHY_REASONS = ['killed', 'lower-success', 'costlier', 'slower'] as const

function whyReasonText(reason: string | undefined): string {
  if (!reason) return '—'
  return (WHY_REASONS as readonly string[]).includes(reason)
    ? t(`tactics.result.whyReason.${reason}`)
    : reason
}

function severityText(severity: string | undefined): string {
  if (severity === 'high') return t('tactics.result.severity.high')
  if (severity === 'medium') return t('tactics.result.severity.medium')
  return severity ?? '—'
}

function whyKindText(kind: string | undefined): string {
  const key = whyKindKey(kind)
  return key ? t(key) : kind ?? '—'
}

function whyKindClass(kind: string | undefined): string {
  if (kind === 'evidence') return 'tactics-pill--on'
  if (kind === 'gap') return 'tactics-pill--off'
  return ''
}

onMounted(() => {
  void loadCatalog()
})
</script>

<style scoped>
.tactics-stage {
  padding: 24px 28px;
  max-width: 1180px;
  margin: 0 auto;
  display: flex;
  flex-direction: column;
  gap: 14px;
}

.tactics-card {
  border: 1px solid var(--hairline);
  border-radius: var(--radius-sm);
  background: var(--card);
  padding: 14px 16px;
}

.tactics-card__head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 10px;
}

.tactics-card__title {
  font-size: 14px;
  font-weight: 600;
  color: var(--text);
}

.tactics-card__subtitle {
  margin: 14px 0 6px;
  font-size: 12px;
  font-weight: 600;
  letter-spacing: 0.02em;
  color: var(--text-dim);
}

.tactics-hint {
  margin: 4px 0 6px;
  font-size: 12px;
  color: var(--text-dim);
}

.tactics-note {
  padding: 8px 10px;
  border: 1px solid var(--hairline);
  border-radius: var(--radius-sm);
  font-size: 12px;
  color: var(--text-dim);
}

.tactics-note--warn {
  border-color: color-mix(in srgb, var(--accent) 40%, var(--hairline));
}

.tactics-table {
  width: 100%;
  border-collapse: collapse;
  margin-top: 6px;
  font-size: 12px;
}

.tactics-table th,
.tactics-table td {
  text-align: left;
  padding: 6px 8px;
  border-bottom: 1px solid var(--hairline);
  vertical-align: middle;
}

.tactics-table th {
  color: var(--text-dim);
  font-weight: 500;
}

.tactics-num {
  text-align: right;
  font-variant-numeric: tabular-nums;
}

.tactics-col-narrow {
  width: 44px;
}

.tactics-mono {
  font-family: var(--font-mono, monospace);
  font-size: 11.5px;
}

.tactics-dim {
  color: var(--text-dim);
}

.tactics-input {
  width: 100%;
  box-sizing: border-box;
  padding: 5px 7px;
  border: 1px solid var(--hairline);
  border-radius: var(--radius-sm);
  background: var(--bg);
  color: var(--text);
  font-size: 12px;
}

.tactics-input--num {
  text-align: right;
  font-variant-numeric: tabular-nums;
}

.tactics-icon-btn {
  padding: 4px 6px;
}

.tactics-grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(170px, 1fr));
  gap: 10px;
  margin-top: 10px;
}

.tactics-field {
  display: flex;
  flex-direction: column;
  gap: 4px;
  font-size: 12px;
  color: var(--text-dim);
}

.tactics-issues {
  margin: 10px 0 0;
  padding-left: 18px;
  font-size: 12px;
  color: var(--accent);
}

.tactics-pill {
  display: inline-flex;
  align-items: center;
  padding: 1px 7px;
  border-radius: 999px;
  border: 1px solid var(--hairline);
  font-size: 11px;
  color: var(--text-dim);
}

.tactics-pill--on {
  border-color: color-mix(in srgb, var(--accent) 45%, var(--hairline));
  color: var(--accent);
}

.tactics-pill--off {
  border-color: color-mix(in srgb, var(--text-dim) 45%, var(--hairline));
  color: var(--text-dim);
}

.tactics-verdict {
  margin-top: 10px;
  padding: 10px 12px;
  border: 1px solid color-mix(in srgb, var(--accent) 35%, var(--hairline));
  border-radius: var(--radius-sm);
  background: color-mix(in srgb, var(--accent) 8%, var(--card));
}

.tactics-verdict--warn {
  border-color: color-mix(in srgb, var(--accent) 60%, var(--hairline));
}

.tactics-verdict__headline {
  font-size: 14px;
  font-weight: 600;
  color: var(--text);
}

.tactics-verdict__reason {
  margin-top: 4px;
  font-size: 12px;
  color: var(--text-dim);
}

.tactics-unknowns,
.tactics-gaps {
  display: flex;
  flex-direction: column;
  gap: 6px;
  margin: 6px 0 0;
  padding: 0;
  list-style: none;
}

.tactics-unknown,
.tactics-gaps li {
  display: flex;
  gap: 8px;
  align-items: baseline;
  font-size: 12px;
  color: var(--text-dim);
}

.tactics-row--winner {
  background: color-mix(in srgb, var(--accent) 7%, transparent);
}

.tactics-why-not,
.tactics-why {
  display: flex;
  flex-direction: column;
  gap: 6px;
  margin: 6px 0 0;
  padding-left: 18px;
  font-size: 12px;
}

.tactics-why-not li {
  display: flex;
  flex-wrap: wrap;
  gap: 8px;
}

.tactics-why-not__reason {
  font-weight: 600;
  color: var(--text);
}

.tactics-why-not__detail {
  color: var(--text-dim);
}

.tactics-premortem {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(260px, 1fr));
  gap: 10px;
  margin-top: 6px;
}

.tactics-premortem__item {
  border: 1px solid var(--hairline);
  border-radius: var(--radius-sm);
  padding: 10px 12px;
  font-size: 12px;
}

.tactics-premortem__head {
  display: flex;
  justify-content: space-between;
  align-items: center;
}

.tactics-premortem__freq {
  color: var(--text-dim);
  font-variant-numeric: tabular-nums;
}

.tactics-premortem__evidence {
  margin-top: 6px;
  color: var(--text);
}

.tactics-premortem__mitigation {
  margin-top: 4px;
  color: var(--text-dim);
}

.tactics-replay-bar {
  display: flex;
  gap: 8px;
  align-items: center;
  margin-top: 6px;
}

.tactics-replay-bar .tactics-input {
  flex: 1;
}

.tactics-why {
  padding-left: 20px;
}

.tactics-why__item {
  margin-bottom: 8px;
}

.tactics-why__question {
  font-weight: 600;
  color: var(--text);
}

.tactics-why__answer {
  margin-top: 2px;
  color: var(--text);
}

.tactics-why__meta {
  display: flex;
  gap: 8px;
  align-items: center;
  margin-top: 4px;
  color: var(--text-dim);
}

.tactics-empty {
  padding: 26px;
  text-align: center;
  font-size: 13px;
  color: var(--text-dim);
  border: 1px dashed var(--hairline);
  border-radius: var(--radius-sm);
}
</style>
