<template>
  <ContentSheet :open="open" :label="sheetLabel" @close="emit('close')">
    <template #header>
      <div class="why__head">
        <strong class="why__title">{{ t('chat.line.retro.title') }}</strong>
        <code class="why__runId" :title="runIdText">{{ runIdText }}</code>
      </div>
    </template>

    <template #actions>
      <button
        type="button"
        class="why__btn"
        data-testid="line-retro-retry"
        :disabled="!canRetry"
        :title="t('chat.line.retro.retryHint')"
        @click="requestRetry"
      >
        <Icon name="refresh" :size="13" />
        {{ t('chat.line.retro.retry') }}
      </button>
    </template>

    <div class="why" data-testid="line-five-why">
      <p v-if="loading" class="why__state" data-testid="line-retro-loading">
        <Icon name="clock" :size="12" />
        {{ t('chat.line.retro.loading') }}
      </p>

      <!-- 复盘本身失败时只报失败：空链、"没有缺口"都不能被读成"证据齐全"。 -->
      <p v-else-if="retroErrorText" class="why__error" data-testid="line-retro-error">
        <Icon name="alertTriangle" :size="12" />
        <span>{{ t('chat.line.retro.error', { error: retroErrorText }) }}</span>
      </p>

      <template v-else-if="retroUsable">
        <div class="why__meta">
          <span>{{ sourceText }}</span>
          <span
            v-if="insufficientEvidence"
            class="why__flag"
            data-testid="line-retro-insufficient"
          >{{ t('chat.line.retro.insufficientEvidence') }}</span>
        </div>

        <section class="why__section">
          <h3 class="why__sectionTitle">{{ t('chat.line.retro.whyTitle') }}</h3>
          <p v-if="chain.length" class="why__sectionNote">
            {{ t('chat.line.retro.whyCount', { count: chain.length }) }}
          </p>
          <!-- 凑不满五层就少给几层：链子短是证据稀疏的真相，不拿推测补齐。 -->
          <p v-else class="why__notice" data-testid="line-retro-why-empty">
            {{ t('chat.line.retro.whyEmpty') }}
          </p>
          <ol v-if="chain.length" class="why__chain" data-testid="line-retro-chain">
            <li
              v-for="link in chain"
              :key="link.key"
              class="why__link"
              :class="[`why__link--${link.tone}`, `why__link--d${link.depth}`]"
              :data-testid="`line-retro-why-${link.index}`"
            >
              <div class="why__linkTop">
                <span class="why__step">{{ t('chat.line.retro.step', { n: link.index + 1 }) }}</span>
                <span class="why__kind" :class="`why__kind--${link.tone}`">{{ link.kindText }}</span>
              </div>
              <p class="why__question">{{ link.question }}</p>
              <p class="why__answer">{{ link.answer }}</p>
              <p v-if="link.evidence" class="why__cite">
                <span class="why__citeLabel">{{ t('chat.line.retro.citeLabel') }}</span>
                <code class="why__citeText">{{ link.evidence }}</code>
              </p>
            </li>
          </ol>
        </section>

        <section v-if="measuredCells.length" class="why__section">
          <h3 class="why__sectionTitle">{{ t('chat.line.retro.measuredTitle') }}</h3>
          <div class="why__grid" data-testid="line-retro-measured">
            <div v-for="cell in measuredCells" :key="cell.key" class="why__cell">
              <span class="why__cellLabel">{{ cell.label }}</span>
              <span class="why__cellValue">{{ cell.value }}</span>
            </div>
          </div>
          <!-- comparable=false ⇔ 有阶段失败：实测只是"跑到失败处"的时间，与预测不同口径。 -->
          <p v-if="!comparable" class="why__notice why__notice--warn" data-testid="line-retro-not-comparable">
            <Icon name="alertTriangle" :size="12" />
            <span>{{ t('chat.line.retro.notComparable') }}</span>
          </p>

          <template v-if="deltas.length">
            <h4 class="why__subtitle">{{ t('chat.line.retro.deltasTitle') }}</h4>
            <table class="why__table" data-testid="line-retro-deltas">
              <thead>
                <tr>
                  <th>{{ t('chat.line.retro.deltasColTactic') }}</th>
                  <th class="why__numCol">{{ t('chat.line.retro.deltasColSuccess') }}</th>
                  <th class="why__numCol">{{ t('chat.line.retro.deltasColTime') }}</th>
                  <th class="why__numCol">{{ t('chat.line.retro.deltasColTokens') }}</th>
                </tr>
              </thead>
              <tbody>
                <tr v-for="(row, i) in deltas" :key="`${row.tactic ?? 'tactic'}-${i}`">
                  <td><code class="why__mono">{{ row.tactic ?? '—' }}</code></td>
                  <td class="why__numCol">
                    {{ formatPct(row.successRate, 1) }}
                    <span class="why__dim">{{ formatDeltaPct(row.successDelta) }}</span>
                  </td>
                  <td class="why__numCol">
                    {{ formatMs(row.p50Ms) }}
                    <span class="why__dim">{{ formatDeltaPct(row.timeDeltaPct) }}</span>
                  </td>
                  <td class="why__numCol">
                    {{ formatTokens(row.p90Tokens) }}
                    <span class="why__dim">{{ tokenDeltaText(row) }}</span>
                  </td>
                </tr>
              </tbody>
            </table>
            <p class="why__footnote">{{ t('chat.line.retro.deltasNote') }}</p>
          </template>

          <template v-if="items.length">
            <h4 class="why__subtitle">{{ t('chat.line.retro.itemsTitle') }}</h4>
            <table class="why__table" data-testid="line-retro-items">
              <thead>
                <tr>
                  <th>{{ t('chat.line.retro.itemsColStage') }}</th>
                  <th class="why__numCol">{{ t('chat.line.retro.itemsColElapsed') }}</th>
                  <th>{{ t('chat.line.retro.itemsColStatus') }}</th>
                </tr>
              </thead>
              <tbody>
                <tr v-for="(item, i) in items" :key="`${item.name ?? 'item'}-${i}`">
                  <td>{{ itemName(item, i) }}</td>
                  <td class="why__numCol">{{ formatMs(item.elapsedMs) }}</td>
                  <td class="why__statusCell" :class="{ 'why__statusCell--bad': item.ok === false }">
                    {{ itemStatus(item) }}
                  </td>
                </tr>
              </tbody>
            </table>
            <p class="why__footnote">{{ t('chat.line.retro.itemsTokensGap') }}</p>
          </template>
        </section>

        <section class="why__section">
          <h3 class="why__sectionTitle">{{ t('chat.line.retro.gapsTitle') }}</h3>
          <ul v-if="gaps.length" class="why__gaps" data-testid="line-retro-gaps">
            <li v-for="(gap, i) in gaps" :key="`${gap.field ?? 'gap'}-${i}`">
              <code class="why__mono">{{ gap.field ?? '—' }}</code>
              <span>{{ gap.reason?.trim() || t('chat.line.retro.gapNoReason') }}</span>
            </li>
          </ul>
          <p v-else class="why__state" data-testid="line-retro-gaps-none">
            {{ t('chat.line.retro.gapsNone') }}
          </p>
        </section>

        <section v-if="nextHint" class="why__section why__next" data-testid="line-retro-next">
          <h3 class="why__sectionTitle">{{ t('chat.line.retro.nextTitle') }}</h3>
          <p class="why__nextText">{{ nextHint }}</p>
          <p class="why__footnote">{{ t('chat.line.retro.nextIsAdvice') }}</p>
        </section>
      </template>

      <p v-else class="why__state" data-testid="line-retro-idle">{{ t('chat.line.retro.idle') }}</p>

      <footer class="why__foot">
        <button type="button" class="why__btn why__btn--primary" data-testid="line-retro-close" @click="emit('close')">
          {{ t('chat.line.retro.done') }}
        </button>
      </footer>
    </div>
  </ContentSheet>
</template>

<script setup lang="ts">
// 复盘单（FiveWhyCard）—— 产线闭环的最后一步：把这一轮的真实证据折成因果链。
//
// 它整体复用 `tactics::replay` 的产物（whyChain / deltas / gaps），所以：
// · 链子的长度由证据决定，`whyChain` 为空就直说"没有成链"，绝不渲染空列表冒充复盘；
// · `measuredVsPredicted.comparable === false` 意味着有阶段失败，实测耗时天然更短，
//   此时偏差只是"少跑了两段"的副产品，不能折成一句"预测很准"；
// · `gaps[]` 是台账没记到的字段，显示为缺口而不是 0（产线至今没有 token 计量）。
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'

import ContentSheet from '@/components/chat/ContentSheet.vue'
import Icon from '@/components/Icon.vue'
import { formatDeltaPct, formatMs, formatPct, formatTokens } from '@/utils/tactics/model'
import type {
  MeasuredItemView,
  ReplayDeltaView,
  ReplayGapView,
  WhyLinkView,
} from '@/utils/tactics/model'
import type { LineRetro } from '@/composables/line/types'

const props = defineProps<{
  open: boolean
  retro: LineRetro | null
  loading: boolean
  runId: string
  /** 加载失败时 retro 为 null，原因由父级从工作流喂进来——回执不能开成空白。 */
  error?: string | null
}>()

const emit = defineEmits<{
  close: []
  retry: [runId: string]
}>()

const { t } = useI18n()

type WhyTone = 'evidence' | 'inference' | 'gap' | 'other'

interface WhyRow {
  key: string
  index: number
  depth: number
  tone: WhyTone
  kindText: string
  question: string
  answer: string
  evidence: string
}

const sheetLabel = computed(() => t('chat.line.retro.title'))

const retroId = computed(() => props.runId.trim() || props.retro?.runId?.trim() || '')
const runIdText = computed(() => retroId.value || '—')
const canRetry = computed(() => !props.loading && Boolean(retroId.value))
const retroUsable = computed(() => props.retro !== null && props.retro.ok !== false)

const retroErrorText = computed(() => {
  const r = props.retro
  // retro 为 null（加载失败的归零态）时回落父级传入的原因，绝不开成空白单。
  if (!r) return props.error?.trim() || ''
  if (r.ok !== false) return ''
  return r.error?.trim() || r.code?.trim() || t('chat.line.retro.errorUnknown')
})

const sourceText = computed(() =>
  t('chat.line.retro.source', { source: props.retro?.scenarioSource?.trim() || '—' }),
)
const insufficientEvidence = computed(() => props.retro?.insufficientEvidence === true)

const nextHint = computed(() => props.retro?.nextPreflightHint?.trim() || '')

function toneOf(kind: string | undefined): WhyTone {
  if (kind === 'evidence') return 'evidence'
  if (kind === 'inference') return 'inference'
  if (kind === 'gap') return 'gap'
  return 'other'
}

function kindTextOf(kind: string | undefined): string {
  const tone = toneOf(kind)
  if (tone === 'evidence') return t('chat.line.retro.kindEvidence')
  if (tone === 'inference') return t('chat.line.retro.kindInference')
  if (tone === 'gap') return t('chat.line.retro.kindGap')
  return kind ? t('chat.line.retro.kindOther', { kind }) : '—'
}

const chain = computed<WhyRow[]>(() => {
  const links: WhyLinkView[] = props.retro?.whyChain ?? []
  return links.map(
    (link, i): WhyRow => ({
      key: `why-${i}`,
      index: i,
      depth: Math.min(i, 4),
      tone: toneOf(link.kind),
      kindText: kindTextOf(link.kind),
      question: link.question?.trim() || '—',
      answer: link.answer?.trim() || t('chat.line.retro.answerMissing'),
      evidence: link.evidence?.trim() || '',
    }),
  )
})

const comparable = computed(() => props.retro?.measuredVsPredicted?.comparable !== false)

const measuredCells = computed<Array<{ key: string; label: string; value: string }>>(() => {
  const m = props.retro?.measuredVsPredicted
  if (!m) return []
  const actual = numberOrNull(m.elapsedMsActual)
  const predicted = numberOrNull(m.elapsedMsPredicted)
  const ratio =
    actual !== null && predicted !== null && predicted > 0 ? (actual - predicted) / predicted : null
  return [
    { key: 'actual', label: t('chat.line.retro.measuredActual'), value: formatMs(actual ?? undefined) },
    { key: 'predicted', label: t('chat.line.retro.measuredPredicted'), value: formatMs(predicted ?? undefined) },
    { key: 'delta', label: t('chat.line.retro.measuredDelta'), value: formatDeltaPct(ratio ?? undefined) },
    {
      // 没有实测成功率：这一格永远是预演值，标签已经写死这一点。
      key: 'success',
      label: t('chat.line.retro.measuredSuccess'),
      value: formatPct(numberOrNull(m.successRatePredicted) ?? undefined, 1),
    },
  ]
})

const deltas = computed<ReplayDeltaView[]>(() => props.retro?.deltas ?? [])
const gaps = computed<ReplayGapView[]>(() => props.retro?.gaps ?? [])
const items = computed<MeasuredItemView[]>(() => props.retro?.observed?.items ?? [])

// observed.tokens === 0 ⇔ 产线没有 token 计量；后端 pct() 会把这种"没量到"折成 0.0 偏差。
// 0.0 读起来像"预测分毫不差"，所以这里必须改写成语义上的"未计量"。
const tokensMeasured = computed(() => {
  const observed = props.retro?.observed
  if ((numberOrNull(observed?.tokens) ?? 0) > 0) return true
  return items.value.some((item) => (numberOrNull(item.tokens) ?? 0) > 0)
})

function tokenDeltaText(row: ReplayDeltaView): string {
  return tokensMeasured.value ? formatDeltaPct(row.tokenDeltaPct) : t('chat.line.retro.tokensUnmeasured')
}

function itemName(item: MeasuredItemView, index: number): string {
  const name = item.name?.trim()
  if (name) return name
  return typeof item.seq === 'number' ? `${item.seq}` : `#${index + 1}`
}

function itemStatus(item: MeasuredItemView): string {
  if (item.ok === false) {
    return item.errorCode?.trim()
      ? t('chat.line.retro.itemFailedCode', { code: item.errorCode.trim() })
      : t('chat.line.retro.itemFailed')
  }
  if (item.ok === true) return t('chat.line.retro.itemOk')
  return t('chat.line.retro.itemUnknown')
}

function numberOrNull(value: unknown): number | null {
  return typeof value === 'number' && Number.isFinite(value) ? value : null
}

function requestRetry(): void {
  if (retroId.value) emit('retry', retroId.value)
}
</script>

<style scoped>
.why {
  display: flex;
  flex-direction: column;
  gap: var(--sp-3);
  overflow-y: auto;
  width: 100%;
  font-size: 12.5px;
  color: var(--text);
}

.why__head {
  display: flex;
  flex-direction: column;
  gap: 2px;
  min-width: 0;
}

.why__title {
  font-size: 13px;
}

.why__runId {
  font-size: 10.5px;
  color: var(--text-dim);
  font-family: var(--font-mono);
  max-width: 420px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.why__state {
  margin: 0;
  font-size: 11.5px;
  color: var(--text-dim);
}

.why__error {
  display: flex;
  align-items: flex-start;
  gap: 6px;
  margin: 0;
  padding: 7px 10px;
  border: 1px solid color-mix(in srgb, var(--danger) 45%, var(--hairline));
  border-radius: var(--radius-sm);
  background: color-mix(in srgb, var(--danger) 10%, var(--card));
  color: var(--danger);
  font-size: 11.5px;
}

.why__meta {
  display: flex;
  align-items: center;
  gap: 8px;
  flex-wrap: wrap;
  font-size: 11px;
  color: var(--text-dim);
}

.why__flag {
  padding: 1px 7px;
  border-radius: 999px;
  border: 1px solid color-mix(in srgb, var(--warn) 45%, var(--hairline));
  color: var(--warn);
  font-size: 10.5px;
}

.why__section {
  display: flex;
  flex-direction: column;
  gap: 6px;
  padding-bottom: var(--sp-3);
  border-bottom: 1px solid var(--hairline);
}

.why__section:last-of-type {
  border-bottom: none;
}

.why__sectionTitle {
  margin: 0;
  font-size: 12px;
  font-weight: 600;
}

.why__sectionNote,
.why__footnote {
  margin: 0;
  font-size: 11px;
  color: var(--text-dim);
  line-height: 1.4;
}

.why__subtitle {
  margin: 4px 0 0;
  font-size: 11.5px;
  font-weight: 600;
  color: var(--text-dim);
}

.why__chain {
  display: flex;
  flex-direction: column;
  gap: 5px;
  margin: 0;
  padding: 0;
  list-style: none;
}

.why__link {
  display: flex;
  flex-direction: column;
  gap: 3px;
  padding: 6px 9px;
  border: 1px solid var(--hairline);
  border-left-width: 3px;
  border-radius: var(--radius-xs);
  background: var(--card);
}

.why__link--d1 {
  margin-left: 14px;
}

.why__link--d2 {
  margin-left: 28px;
}

.why__link--d3 {
  margin-left: 42px;
}

.why__link--d4 {
  margin-left: 56px;
}

.why__link--evidence {
  border-left-color: var(--ok);
}

.why__link--inference {
  border-left-color: var(--accent);
}

.why__link--gap {
  border-left-color: var(--warn);
  border-style: dashed;
}

.why__link--other {
  border-left-color: var(--hairline);
}

.why__linkTop {
  display: flex;
  align-items: baseline;
  gap: 6px;
}

.why__step {
  font-size: 10.5px;
  color: var(--text-dim);
  font-variant-numeric: tabular-nums;
}

.why__kind {
  padding: 0 6px;
  border-radius: 999px;
  border: 1px solid var(--hairline);
  font-size: 10px;
  color: var(--text-dim);
}

.why__kind--evidence {
  color: var(--ok);
  border-color: color-mix(in srgb, var(--ok) 45%, var(--hairline));
  background: color-mix(in srgb, var(--ok) 12%, var(--card));
}

.why__kind--inference {
  color: var(--accent);
  border-color: color-mix(in srgb, var(--accent) 45%, var(--hairline));
  background: color-mix(in srgb, var(--accent) 12%, var(--card));
}

.why__kind--gap {
  color: var(--warn);
  border-color: color-mix(in srgb, var(--warn) 45%, var(--hairline));
  background: color-mix(in srgb, var(--warn) 12%, var(--card));
}

.why__question {
  margin: 0;
  font-size: 12px;
  color: var(--text-dim);
}

.why__answer {
  margin: 0;
  font-size: 12.5px;
  line-height: 1.5;
  white-space: pre-wrap;
  word-break: break-word;
}

.why__cite {
  display: flex;
  align-items: baseline;
  gap: 5px;
  margin: 0;
  font-size: 10.5px;
}

.why__citeLabel {
  color: var(--text-dim);
  text-transform: uppercase;
  letter-spacing: 0.04em;
}

.why__citeText {
  font-family: var(--font-mono);
  color: var(--text);
  border: 1px solid var(--hairline);
  border-radius: var(--radius-xs);
  padding: 0 4px;
  word-break: break-all;
}

.why__notice {
  display: flex;
  align-items: flex-start;
  gap: 6px;
  margin: 0;
  padding: 6px 9px;
  border-radius: var(--radius-xs);
  font-size: 11.5px;
  color: var(--text-dim);
  background: var(--bg-surface);
}

.why__notice--warn {
  color: var(--warn);
  background: color-mix(in srgb, var(--warn) 10%, var(--card));
}

.why__grid {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(120px, 1fr));
  gap: 6px;
}

.why__cell {
  display: flex;
  flex-direction: column;
  gap: 1px;
  padding: 5px 8px;
  border: 1px solid var(--hairline);
  border-radius: var(--radius-xs);
  background: var(--card);
  min-width: 0;
}

.why__cellLabel {
  font-size: 10.5px;
  color: var(--text-dim);
}

.why__cellValue {
  font-size: 12.5px;
  font-weight: 600;
  font-variant-numeric: tabular-nums;
}

.why__table {
  width: 100%;
  border-collapse: collapse;
  font-size: 11.5px;
}

.why__table th,
.why__table td {
  text-align: left;
  padding: 4px 6px;
  border-bottom: 1px solid var(--hairline);
}

.why__table th {
  color: var(--text-dim);
  font-weight: 500;
  font-size: 10.5px;
}

.why__numCol {
  text-align: right !important;
  font-variant-numeric: tabular-nums;
}

.why__statusCell {
  color: var(--ok);
}

.why__statusCell--bad {
  color: var(--danger);
}

.why__dim {
  color: var(--text-dim);
  margin-left: 4px;
}

.why__mono {
  font-family: var(--font-mono);
  font-size: 11px;
}

.why__gaps {
  display: flex;
  flex-direction: column;
  gap: 4px;
  margin: 0;
  padding: 0;
  list-style: none;
}

.why__gaps li {
  display: flex;
  gap: 6px;
  align-items: baseline;
  color: var(--text-dim);
  font-size: 11.5px;
}

.why__next {
  border-left: 2px solid color-mix(in srgb, var(--accent) 45%, var(--hairline));
  padding-left: 10px;
  border-bottom: none;
}

.why__nextText {
  margin: 0;
  font-size: 12px;
  line-height: 1.5;
  white-space: pre-wrap;
  word-break: break-word;
}

.why__foot {
  display: flex;
  justify-content: flex-end;
  gap: 8px;
  margin-top: auto;
}

.why__btn {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  padding: 4px 12px;
  border: 1px solid var(--hairline);
  border-radius: 999px;
  background: transparent;
  color: var(--text);
  font-size: 11.5px;
  cursor: pointer;
  transition: color var(--dur-fast) var(--ease-standard),
    border-color var(--dur-fast) var(--ease-standard);
}

.why__btn:hover:not(:disabled) {
  border-color: color-mix(in srgb, var(--accent) 55%, var(--hairline));
  color: var(--accent);
}

.why__btn:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}

.why__btn--primary {
  border-color: color-mix(in srgb, var(--accent) 60%, var(--hairline));
  background: color-mix(in srgb, var(--accent) 16%, var(--card));
}

@media (prefers-reduced-motion: reduce) {
  .why__btn {
    transition: none;
  }
}

@media (max-width: 768px) {
  .why__link--d1,
  .why__link--d2,
  .why__link--d3,
  .why__link--d4 {
    margin-left: 8px;
  }
}
</style>
