<template>
  <section v-if="run" class="lr" data-testid="line-run-card" :data-run-ok="verdictOk ? 'true' : 'false'">
    <header class="lr__head">
      <span
        class="lr__badge"
        :class="verdictOk ? 'lr__badge--ok' : 'lr__badge--bad'"
        data-testid="line-run-badge"
      >
        <Icon :name="verdictOk ? 'check' : 'alertTriangle'" :size="11" />
        {{ verdictOk ? t('chat.line.run.runOk') : t('chat.line.run.runFailed') }}
      </span>
      <strong class="lr__title">{{ t('chat.line.run.title') }}</strong>
      <span class="lr__shape" :title="shapeText">{{ shapeText }}</span>
      <span class="lr__ident">
        <code class="lr__runId" :title="runIdText">{{ runIdText }}</code>
        <span class="lr__at">{{ atText }}</span>
      </span>
    </header>

    <p class="lr__goal" :title="goalText">{{ goalText }}</p>

    <!-- 后端在第一个失败阶段就 break：少掉的尾巴是"从未尝试"，不是"通过了"。 -->
    <p v-if="tailCount > 0" class="lr__notice" data-testid="line-run-tail">
      <Icon name="info" :size="11" />
      {{ t('chat.line.run.stagesRan', { ran: attemptedCount, declared: declaredCount }) }}
    </p>

    <ol class="lr__ladder" data-testid="line-run-stages">
      <li
        v-for="row in ladder"
        :key="row.key"
        class="lr__stage"
        :class="`lr__stage--${row.tone}`"
        :data-testid="`line-run-stage-${row.ord}`"
      >
        <div class="lr__row">
          <span class="lr__ord">{{ row.ord }}</span>
          <span class="lr__pattern" :title="row.patternHint">{{ row.label }}</span>
          <span class="lr__width">{{ row.widthText }}</span>
          <span class="lr__status" :class="`lr__status--${row.tone}`">
            <Icon :name="row.icon" :size="11" />
            {{ row.statusText }}
          </span>
          <code class="lr__backend" :title="row.backendText">{{ row.backendText }}</code>
          <span class="lr__elapsed">{{ row.elapsedText }}</span>
          <button
            v-if="row.expandable"
            type="button"
            class="lr__expand"
            :aria-expanded="open[row.key] ? 'true' : 'false'"
            :data-testid="`line-run-stage-toggle-${row.ord}`"
            @click="toggle(row.key)"
          >
            <span class="lr__caret" :class="{ 'lr__caret--on': open[row.key] }">
              <Icon name="chevronRight" :size="11" />
            </span>
            {{ row.detailLabel }}
          </button>
        </div>

        <p
          v-if="row.tone === 'skipped'"
          class="lr__skipped"
          :data-testid="`line-run-stage-skipped-${row.ord}`"
        >
          {{ t('chat.line.run.skippedWhy', { stage: row.id }) }}
        </p>

        <div
          v-if="row.expandable && open[row.key]"
          class="lr__detail"
          :data-testid="`line-run-stage-detail-${row.ord}`"
        >
          <p v-if="row.errorText" class="lr__error" data-testid="line-run-stage-error">
            {{ row.errorText }}
          </p>
          <p v-if="row.note" class="lr__kv">
            <span class="lr__kvLabel">{{ t('chat.line.run.noteLabel') }}</span>
            <span class="lr__kvValue">{{ row.note }}</span>
          </p>
          <p v-if="row.rosterText" class="lr__kv">
            <span class="lr__kvLabel">{{ t('chat.line.run.rosterLabel') }}</span>
            <code class="lr__kvValue">{{ row.rosterText }}</code>
          </p>

          <div class="lr__block">
            <span class="lr__kvLabel">{{ t('chat.line.run.replyLabel') }}</span>
            <pre v-if="row.reply" class="lr__reply">{{ row.reply }}</pre>
            <p v-else class="lr__muted">{{ t('chat.line.run.replyEmpty') }}</p>
          </div>

          <div class="lr__block">
            <span class="lr__kvLabel">{{ t('chat.line.run.claimsLabel', { count: row.claims.length }) }}</span>
            <ul v-if="row.claims.length" class="lr__claims">
              <li
                v-for="(claim, ci) in row.claims"
                :key="`${row.key}-claim-${ci}`"
                class="lr__claim"
                :class="{ 'lr__claim--dissent': claim.dissent === true }"
                :data-testid="`line-run-claim-${row.ord}-${ci}`"
              >
                <div class="lr__claimMeta">
                  <span class="lr__kind">{{ kindLabel(claim.kind) }}</span>
                  <!-- 少数派是这套设计的功能，不是噪声：措辞相近却相反的结论必须留在回执上。 -->
                  <span
                    v-if="claim.dissent === true"
                    class="lr__dissent"
                    :title="t('chat.line.run.dissentHint')"
                  >{{ t('chat.line.run.dissent') }}</span>
                  <!-- accepted 恒为 null：裁决属于 harness.ledger.verdict，产线不自造。 -->
                  <span class="lr__pending" :title="t('chat.line.run.verdictPendingHint')">
                    {{ t('chat.line.run.verdictPending') }}
                  </span>
                  <span class="lr__by">{{ t('chat.line.run.byMember', { member: memberOf(claim) }) }}</span>
                </div>
                <p class="lr__claimText">{{ claim.claim ?? '' }}</p>
                <p v-if="evidenceOf(claim).length" class="lr__evidence">
                  <span class="lr__kvLabel">{{ t('chat.line.run.evidenceLabel') }}</span>
                  <code
                    v-for="(ev, ei) in evidenceOf(claim)"
                    :key="`${row.key}-ev-${ci}-${ei}`"
                    class="lr__evItem"
                  >{{ ev }}</code>
                </p>
              </li>
            </ul>
            <p v-else class="lr__muted">{{ t('chat.line.run.claimsEmpty') }}</p>
          </div>
        </div>
      </li>
    </ol>

    <div class="lr__verdict" data-testid="line-run-metrics">
      <div v-for="m in metrics" :key="m.key" class="lr__metric" :data-testid="`line-run-metric-${m.key}`">
        <span class="lr__metricLabel">{{ m.label }}</span>
        <span class="lr__metricValue">{{ m.value }}</span>
        <span class="lr__metricHint">{{ m.hint }}</span>
      </div>
    </div>

    <div v-if="failures.length" class="lr__failures" data-testid="line-run-failures">
      <strong>{{ t('chat.line.run.failuresTitle') }}</strong>
      <ul>
        <li v-for="(item, i) in failures" :key="`${i}-${item}`">{{ item }}</li>
      </ul>
    </div>

    <p
      class="lr__storage"
      :class="{ 'lr__storage--missing': !storage.persisted }"
      data-testid="line-run-storage"
    >
      <Icon :name="storage.persisted ? 'shield' : 'alertTriangle'" :size="11" />
      <span>{{ storage.text }}</span>
    </p>

    <footer class="lr__foot">
      <button
        type="button"
        class="lr__btn"
        :disabled="!retroEnabled"
        :title="retroTitle"
        data-testid="line-run-retro"
        @click="requestRetro"
      >
        <Icon name="gauge" :size="13" />
        {{ retroLoading ? t('chat.line.run.retroLoading') : t('chat.line.run.retroButton') }}
      </button>
      <span v-if="retroLoading" class="lr__footNote">{{ t('chat.line.run.retroLoading') }}</span>
      <span v-else-if="retroFailed" class="lr__footNote lr__footNote--bad">{{ retroErrorText }}</span>
      <span v-else-if="retroReady" class="lr__footNote">{{ t('chat.line.run.retroReady') }}</span>
      <span v-else-if="!storage.persisted" class="lr__footNote lr__footNote--bad">
        {{ t('chat.line.run.retroBlocked', { error: storageErrorText }) }}
      </span>
      <span v-else class="lr__footNote">{{ t('chat.line.run.retroHint') }}</span>
    </footer>
  </section>
</template>

<script setup lang="ts">
// 产线回执（LineRunCard）—— 一条产线跑完之后唯一可对账的那一份记录。
//
// 它与预演卡是一对：预演给的是**预测**（p90 / 相乘的成功率），回执给的是**实测耗时**。
// 两者并排而不折成一个数字，是因为产线的 token 至今没有计量（只有耗时被秒表抓到），
// 而失败会截断阶段序列。这两件事一旦被界面包装成"跑完了、花了 N token"，
// 回执就失去了它存在的意义。
import { computed, ref } from 'vue'
import { useI18n } from 'vue-i18n'

import Icon from '@/components/Icon.vue'
import { formatDeltaPct, formatMs, formatPct, formatTokens, formatUsd } from '@/utils/tactics/model'
import type {
  LineCatalog,
  LineClaim,
  LineRetro,
  LineRunResult,
  LineStageRun,
} from '@/composables/line/types'

const props = defineProps<{
  run: LineRunResult | null
  catalog: LineCatalog
  retro: LineRetro | null
  retroLoading: boolean
}>()

const emit = defineEmits<{
  'load-retro': [runId: string]
}>()

const { t } = useI18n()

type StageTone = 'ok' | 'failed' | 'skipped'

interface LadderRow {
  key: string
  ord: number
  id: string
  label: string
  patternHint: string
  widthText: string
  tone: StageTone
  icon: 'check' | 'alertTriangle' | 'clock'
  statusText: string
  backendText: string
  elapsedText: string
  expandable: boolean
  detailLabel: string
  reply: string
  claims: LineClaim[]
  errorText: string
  note: string
  rosterText: string
}

interface MetricRow {
  key: string
  label: string
  value: string
  hint: string
}

const open = ref<Record<string, boolean>>({})

function toggle(key: string): void {
  open.value = { ...open.value, [key]: !open.value[key] }
}

function finite(value: unknown): number | null {
  return typeof value === 'number' && Number.isFinite(value) ? value : null
}

function patternLabel(pattern: string | undefined): string {
  if (!pattern) return '—'
  return props.catalog.patterns.find((p) => p.id === pattern)?.label ?? pattern
}

function patternHint(pattern: string | undefined): string {
  if (!pattern) return ''
  const meta = props.catalog.patterns.find((p) => p.id === pattern)
  return meta ? `${meta.id} · ${meta.description}` : ''
}

const shapeText = computed(() => props.run?.shape?.trim() || '—')
const runIdText = computed(() => props.run?.runId ?? '—')
const goalText = computed(() => props.run?.spec?.goal?.trim() || '—')
const verdictOk = computed(() => props.run?.verdict?.ok === true)
const failures = computed(() => props.run?.verdict?.failures ?? [])
const declaredCount = computed(() => props.run?.spec?.stages?.length ?? 0)
const attemptedCount = computed(() => props.run?.stages?.length ?? 0)
const tailCount = computed(() => Math.max(0, declaredCount.value - attemptedCount.value))

const atText = computed(() => {
  const raw = props.run?.at
  if (!raw) return '—'
  const date = new Date(raw)
  return Number.isNaN(date.getTime()) ? raw : date.toLocaleString()
})

function rowOf(
  ord: number,
  id: string,
  pattern: string | undefined,
  width: number,
  note: string,
  roster: string[],
  hit: LineStageRun | null,
): LadderRow {
  const tone: StageTone = hit === null ? 'skipped' : hit.status === 'ok' ? 'ok' : 'failed'
  const claims = (hit?.claims ?? []).filter(Boolean) as LineClaim[]
  const reply = hit?.reply ?? ''
  return {
    key: `${ord}-${id}`,
    ord,
    id,
    label: patternLabel(pattern),
    patternHint: patternHint(pattern),
    widthText: width > 0 ? `×${width}` : '—',
    tone,
    icon: tone === 'ok' ? 'check' : tone === 'failed' ? 'alertTriangle' : 'clock',
    statusText:
      tone === 'ok'
        ? t('chat.line.run.statusOk')
        : tone === 'failed'
          ? t('chat.line.run.statusFailed')
          : t('chat.line.run.statusNotRun'),
    backendText: hit?.backend?.trim() || '—',
    elapsedText: tone === 'skipped' ? '—' : formatMs(finite(hit?.elapsedMs) ?? undefined),
    expandable: tone !== 'skipped',
    detailLabel: t('chat.line.run.detailToggle', { count: claims.length }),
    reply,
    claims,
    errorText: hit?.error ?? '',
    note,
    rosterText: roster.join(' · '),
  }
}

const ladder = computed<LadderRow[]>(() => {
  const run = props.run
  if (!run) return []
  const declared = run.spec?.stages ?? []
  const actual = run.stages ?? []
  const rows: LadderRow[] = declared.map((stage, i) =>
    rowOf(
      i + 1,
      stage.id ?? `stage-${i + 1}`,
      stage.pattern,
      stage.width ?? 0,
      stage.note ?? '',
      stage.roster ?? [],
      actual.find((s) => s?.index === i) ?? actual.find((s) => s?.id === stage.id) ?? null,
    ),
  )
  // 声明里对不上的回报条目（旧 run 记录 / 手写 spec）必须照样列出来，否则证据静默丢失。
  actual.forEach((entry, i) => {
    const matched = declared.some((d, di) => d.id === entry?.id || entry?.index === di)
    if (matched) return
    rows.push(
      rowOf(
        declared.length + i + 1,
        entry?.id ?? `extra-${i + 1}`,
        entry?.pattern,
        0,
        '',
        [],
        entry ?? null,
      ),
    )
  })
  return rows
})

const elapsedRatio = computed<number | null>(() => {
  const v = props.run?.verdict
  if (!v) return null
  const actual = finite(v.elapsedMsActual)
  const predicted = finite(v.elapsedMsPredicted)
  if (actual === null || predicted === null || predicted <= 0) return null
  return (actual - predicted) / predicted
})

// 耗时是各阶段**相加**，成功率是各阶段**相乘**：任何"总计"都得带上这个口径。
const metrics = computed<MetricRow[]>(() => {
  const v = props.run?.verdict
  if (!v) return []
  const rows: MetricRow[] = [
    {
      key: 'elapsed-actual',
      label: t('chat.line.run.elapsedActual'),
      value: formatMs(finite(v.elapsedMsActual) ?? undefined),
      hint: t('chat.line.run.elapsedActualHint'),
    },
    {
      key: 'elapsed-predicted',
      label: t('chat.line.run.elapsedPredicted'),
      value: formatMs(finite(v.elapsedMsPredicted) ?? undefined),
      hint: t('chat.line.run.elapsedPredictedHint'),
    },
    {
      key: 'elapsed-delta',
      label: t('chat.line.run.elapsedDelta'),
      value: formatDeltaPct(elapsedRatio.value ?? undefined),
      hint: verdictOk.value
        ? t('chat.line.run.elapsedDeltaHintOk')
        : t('chat.line.run.elapsedDeltaHintShort'),
    },
    {
      // 没有 tokensActual：这一行的数字是预演 p90，标签与提示都必须带着"预演"。
      key: 'tokens-predicted',
      label: t('chat.line.run.tokensPredicted'),
      value: formatTokens(finite(v.tokensPredicted) ?? undefined),
      hint: t('chat.line.run.tokensHint'),
    },
    {
      key: 'success-predicted',
      label: t('chat.line.run.successRatePredicted'),
      value: formatPct(finite(v.successRatePredicted) ?? undefined, 1),
      hint: t('chat.line.run.successHint', { stages: declaredCount.value }),
    },
  ]
  const cost = finite(props.run?.preflight?.total?.p90CostUsd)
  if (cost !== null) {
    rows.splice(3, 0, {
      key: 'cost-predicted',
      label: t('chat.line.run.costPredicted'),
      value: formatUsd(cost),
      hint: t('chat.line.run.costHint'),
    })
  }
  return rows
})

const storageErrorText = computed(() => props.run?.storage?.error?.trim() || '—')

const storage = computed<{ persisted: boolean; text: string }>(() => {
  const s = props.run?.storage
  if (!s) return { persisted: false, text: t('chat.line.run.storageUnknown') }
  if (s.persisted) {
    const measuredBytes = finite(s.bytes)
    const base = t('chat.line.run.storageSaved', {
      path: s.path?.trim() || '—',
      bytes: measuredBytes === null ? '—' : String(measuredBytes),
    })
    const pruned = finite(s.pruned)
    return {
      persisted: true,
      text: pruned && pruned > 0 ? `${base} ${t('chat.line.run.storagePruned', { count: pruned })}` : base,
    }
  }
  return { persisted: false, text: t('chat.line.run.storageMissing', { error: storageErrorText.value }) }
})

const retroForRun = computed<LineRetro | null>(() => {
  const runId = props.run?.runId
  const retro = props.retro
  if (!runId || !retro || retro.runId !== runId) return null
  return retro
})
const retroFailed = computed(() => retroForRun.value?.ok === false)
const retroReady = computed(
  () => retroForRun.value !== null && !retroFailed.value && (retroForRun.value?.whyChain?.length ?? 0) > 0,
)
const retroErrorText = computed(() =>
  t('chat.line.run.retroError', { error: retroForRun.value?.error?.trim() || retroForRun.value?.code || '—' }),
)
const retroEnabled = computed(
  () => Boolean(props.run?.runId) && storage.value.persisted && !props.retroLoading,
)
const retroTitle = computed(() => {
  if (!storage.value.persisted) return t('chat.line.run.retroDisabledNoRecord', { error: storageErrorText.value })
  if (props.retroLoading) return t('chat.line.run.retroLoading')
  return t('chat.line.run.retroHint')
})

function requestRetro(): void {
  const runId = props.run?.runId
  if (runId) emit('load-retro', runId)
}

function kindLabel(kind: string | undefined): string {
  if (kind === 'finding') return t('chat.line.run.kindFinding')
  if (kind === 'decision') return t('chat.line.run.kindDecision')
  if (kind === 'risk') return t('chat.line.run.kindRisk')
  if (kind === 'question') return t('chat.line.run.kindQuestion')
  return kind ? t('chat.line.run.kindOther', { kind }) : '—'
}

function memberOf(claim: LineClaim): string {
  return claim.by?.member?.trim() || '—'
}

function evidenceOf(claim: LineClaim): string[] {
  return Array.isArray(claim.evidence) ? claim.evidence : []
}
</script>

<style scoped>
.lr {
  display: flex;
  flex-direction: column;
  gap: var(--sp-2);
  padding: 10px 12px;
  border: 1px solid var(--hairline);
  border-radius: var(--radius-sm);
  background: var(--card);
  font-size: 12px;
  color: var(--text);
}

.lr__head {
  display: flex;
  align-items: baseline;
  gap: 6px;
  flex-wrap: wrap;
}

.lr__title {
  font-size: 12.5px;
}

.lr__badge {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  padding: 1px 7px;
  border-radius: 999px;
  font-size: 10.5px;
  border: 1px solid var(--hairline);
}

.lr__badge--ok {
  color: var(--ok);
  border-color: color-mix(in srgb, var(--ok) 45%, var(--hairline));
  background: color-mix(in srgb, var(--ok) 12%, var(--card));
}

.lr__badge--bad {
  color: var(--danger);
  border-color: color-mix(in srgb, var(--danger) 45%, var(--hairline));
  background: color-mix(in srgb, var(--danger) 12%, var(--card));
}

.lr__shape {
  font-size: 11px;
  color: var(--text-dim);
  font-variant-numeric: tabular-nums;
  max-width: 46%;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.lr__ident {
  margin-left: auto;
  display: inline-flex;
  align-items: baseline;
  gap: 6px;
  min-width: 0;
}

.lr__runId,
.lr__at {
  font-size: 10.5px;
  color: var(--text-dim);
  font-family: var(--font-mono);
  font-variant-numeric: tabular-nums;
  max-width: 190px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.lr__goal {
  margin: 0;
  font-size: 11.5px;
  color: var(--text-dim);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.lr__notice {
  display: flex;
  align-items: flex-start;
  gap: 6px;
  margin: 0;
  padding: 5px 8px;
  border-left: 2px solid color-mix(in srgb, var(--warn) 60%, var(--hairline));
  background: color-mix(in srgb, var(--warn) 10%, var(--card));
  border-radius: var(--radius-xs);
  font-size: 11.5px;
  color: var(--text);
}

.lr__ladder {
  display: flex;
  flex-direction: column;
  gap: 4px;
  margin: 0;
  padding: 0;
  list-style: none;
}

.lr__stage {
  border: 1px solid var(--hairline);
  border-radius: var(--radius-xs);
  padding: 5px 8px;
  background: var(--bg-surface);
}

.lr__stage--failed {
  border-color: color-mix(in srgb, var(--danger) 40%, var(--hairline));
}

.lr__stage--skipped {
  border-style: dashed;
  background: transparent;
}

.lr__row {
  display: flex;
  align-items: baseline;
  gap: 6px;
  flex-wrap: wrap;
}

.lr__ord {
  flex: 0 0 auto;
  width: 16px;
  color: var(--text-dim);
  font-variant-numeric: tabular-nums;
}

.lr__pattern {
  flex: 0 0 auto;
  font-weight: 600;
}

.lr__width {
  flex: 0 0 auto;
  color: var(--text-dim);
  font-size: 11px;
  font-variant-numeric: tabular-nums;
}

.lr__status {
  display: inline-flex;
  align-items: center;
  gap: 3px;
  flex: 0 0 auto;
  padding: 0 6px;
  border-radius: 999px;
  border: 1px solid var(--hairline);
  font-size: 10.5px;
}

.lr__status--ok {
  color: var(--ok);
  border-color: color-mix(in srgb, var(--ok) 40%, var(--hairline));
}

.lr__status--failed {
  color: var(--danger);
  border-color: color-mix(in srgb, var(--danger) 40%, var(--hairline));
}

.lr__status--skipped {
  color: var(--text-dim);
}

.lr__backend {
  flex: 1 1 90px;
  min-width: 0;
  font-size: 10.5px;
  font-family: var(--font-mono);
  color: var(--text-dim);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.lr__elapsed {
  flex: 0 0 auto;
  text-align: right;
  font-variant-numeric: tabular-nums;
}

.lr__expand {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  flex: 0 0 auto;
  padding: 1px 7px;
  border: 1px solid var(--hairline);
  border-radius: 999px;
  background: transparent;
  color: var(--text-dim);
  font-size: 10.5px;
  cursor: pointer;
  transition: color var(--dur-fast) var(--ease-standard),
    border-color var(--dur-fast) var(--ease-standard);
}

.lr__expand:hover {
  color: var(--text);
  border-color: color-mix(in srgb, var(--accent) 50%, var(--hairline));
}

.lr__caret {
  display: inline-flex;
  transition: transform var(--dur-fast) var(--ease-standard);
}

.lr__caret--on {
  transform: rotate(90deg);
}

.lr__skipped {
  margin: 4px 0 0;
  font-size: 11px;
  color: var(--text-dim);
}

.lr__detail {
  display: flex;
  flex-direction: column;
  gap: 6px;
  margin-top: 6px;
  padding-top: 6px;
  border-top: 1px dashed var(--hairline);
}

.lr__error {
  margin: 0;
  padding: 5px 8px;
  border-radius: var(--radius-xs);
  background: color-mix(in srgb, var(--danger) 12%, var(--card));
  color: var(--danger);
  font-size: 11px;
  font-family: var(--font-mono);
  white-space: pre-wrap;
  word-break: break-word;
}

.lr__kv {
  display: flex;
  align-items: baseline;
  gap: 6px;
  margin: 0;
  font-size: 11px;
}

.lr__kvLabel {
  flex: 0 0 auto;
  color: var(--text-dim);
  font-size: 10.5px;
  text-transform: uppercase;
  letter-spacing: 0.04em;
}

.lr__kvValue {
  min-width: 0;
  color: var(--text);
}

.lr__block {
  display: flex;
  flex-direction: column;
  gap: 3px;
}

.lr__reply {
  margin: 0;
  padding: 6px 8px;
  max-height: 180px;
  overflow-y: auto;
  border: 1px solid var(--hairline);
  border-radius: var(--radius-xs);
  background: var(--bg);
  color: var(--text);
  font-family: var(--font-mono);
  font-size: 11px;
  line-height: 1.45;
  white-space: pre-wrap;
  word-break: break-word;
}

.lr__muted {
  margin: 0;
  font-size: 11px;
  color: var(--text-dim);
}

.lr__claims {
  display: flex;
  flex-direction: column;
  gap: 5px;
  margin: 0;
  padding: 0;
  list-style: none;
}

.lr__claim {
  display: flex;
  flex-direction: column;
  gap: 3px;
  padding: 5px 7px;
  border-left: 2px solid color-mix(in srgb, var(--accent) 40%, var(--hairline));
  background: var(--bg);
  border-radius: var(--radius-xs);
}

.lr__claim--dissent {
  border-left-color: var(--warn);
}

.lr__claimMeta {
  display: flex;
  align-items: baseline;
  gap: 5px;
  flex-wrap: wrap;
}

.lr__kind,
.lr__dissent,
.lr__pending {
  padding: 0 6px;
  border-radius: 999px;
  border: 1px solid var(--hairline);
  font-size: 10px;
  color: var(--text-dim);
}

.lr__dissent {
  color: var(--warn);
  border-color: color-mix(in srgb, var(--warn) 45%, var(--hairline));
}

.lr__pending {
  color: var(--text-dim);
}

.lr__by {
  margin-left: auto;
  font-size: 10.5px;
  color: var(--text-dim);
}

.lr__claimText {
  margin: 0;
  font-size: 11.5px;
  line-height: 1.45;
  white-space: pre-wrap;
  word-break: break-word;
}

.lr__evidence {
  display: flex;
  align-items: baseline;
  gap: 4px;
  flex-wrap: wrap;
  margin: 0;
  font-size: 10.5px;
}

.lr__evItem {
  font-family: var(--font-mono);
  color: var(--text-dim);
  border: 1px solid var(--hairline);
  border-radius: var(--radius-xs);
  padding: 0 4px;
}

.lr__verdict {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(112px, 1fr));
  gap: 6px;
  padding: 6px 0 0;
  border-top: 1px solid var(--hairline);
}

.lr__metric {
  display: flex;
  flex-direction: column;
  gap: 1px;
  min-width: 0;
}

.lr__metricLabel {
  font-size: 10.5px;
  color: var(--text-dim);
}

.lr__metricValue {
  font-size: 12.5px;
  font-weight: 600;
  font-variant-numeric: tabular-nums;
}

.lr__metricHint {
  font-size: 10.5px;
  color: var(--text-dim);
  line-height: 1.35;
}

.lr__failures {
  display: flex;
  flex-direction: column;
  gap: 3px;
  padding: 6px 8px;
  border-radius: var(--radius-xs);
  background: color-mix(in srgb, var(--danger) 10%, var(--card));
  font-size: 11.5px;
}

.lr__failures ul {
  margin: 0;
  padding-left: 16px;
  color: var(--danger);
}

.lr__storage {
  display: flex;
  align-items: flex-start;
  gap: 6px;
  margin: 0;
  font-size: 11px;
  color: var(--text-dim);
  word-break: break-word;
}

.lr__storage--missing {
  color: var(--danger);
}

.lr__foot {
  display: flex;
  align-items: center;
  gap: 8px;
  flex-wrap: wrap;
  padding-top: 6px;
  border-top: 1px solid var(--hairline);
}

.lr__btn {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  padding: 4px 11px;
  border: 1px solid color-mix(in srgb, var(--accent) 55%, var(--hairline));
  border-radius: 999px;
  background: color-mix(in srgb, var(--accent) 16%, var(--card));
  color: var(--text);
  font-size: 11.5px;
  cursor: pointer;
  transition: filter var(--dur-fast) var(--ease-standard);
}

.lr__btn:hover:not(:disabled) {
  filter: brightness(1.08);
}

.lr__btn:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}

.lr__footNote {
  font-size: 11px;
  color: var(--text-dim);
}

.lr__footNote--bad {
  color: var(--danger);
}

@media (prefers-reduced-motion: reduce) {
  .lr__expand,
  .lr__caret,
  .lr__btn {
    transition: none;
  }
}

@media (max-width: 520px) {
  .lr__ident {
    margin-left: 0;
    width: 100%;
  }

  .lr__verdict {
    grid-template-columns: repeat(auto-fit, minmax(96px, 1fr));
  }
}
</style>
