<template>
  <ContentSheet :open="open" :label="sheetLabel" @close="emit('close')">
    <template #header>
      <div class="crit__head">
        <strong>{{ sheetLabel }}</strong>
        <span class="crit__hint">{{ t('chat.line.criteriaLead') }}</span>
      </div>
    </template>

    <div class="crit" data-testid="line-criteria-sheet">
      <section class="crit__block">
        <h4 class="crit__title">{{ t('chat.line.acceptanceTitle') }}</h4>
        <p class="crit__sub">{{ t('chat.line.acceptanceLead') }}</p>
        <ul class="crit__list">
          <li v-for="(clause, index) in spec.acceptance" :key="index" class="crit__clause">
            <span class="crit__clauseNo">{{ index + 1 }}</span>
            <input
              class="crit__clauseInput"
              type="text"
              :value="clause"
              :placeholder="t('chat.line.acceptancePlaceholder')"
              :aria-label="t('chat.line.acceptanceTitle')"
              @input="setClause(index, ($event.target as HTMLInputElement).value)"
            />
            <button
              type="button"
              class="crit__del"
              :aria-label="t('chat.line.acceptanceRemove')"
              @click="removeClause(index)"
            >
              <Icon name="x" :size="13" />
            </button>
          </li>
        </ul>
        <button type="button" class="crit__add" @click="addClause">
          <Icon name="plus" :size="13" />
          {{ t('chat.line.acceptanceAdd') }}
        </button>
      </section>

      <section class="crit__block">
        <h4 class="crit__title">{{ t('chat.line.judgeTitle') }}</h4>
        <p class="crit__sub">{{ t('chat.line.judgeLead') }}</p>

        <label class="crit__row" for="crit-maxfail">
          <span>{{ t('chat.line.maxFailProbability') }}</span>
          <output class="crit__value">{{ pct(spec.criteria.maxFailProbability) }}</output>
        </label>
        <div class="crit__sliderRow">
          <input
            id="crit-maxfail"
            class="crit__slider"
            type="range"
            min="0.02"
            max="0.9"
            step="0.01"
            :value="spec.criteria.maxFailProbability"
            @input="setCriteria({ maxFailProbability: num(($event.target as HTMLInputElement).value, 0.15) })"
          />
          <span v-if="defaultCriteria" class="crit__default" @click="setCriteria({ maxFailProbability: defaultCriteria.maxFailProbability })">
            {{ t('chat.line.useDefault', { value: pct(defaultCriteria.maxFailProbability) }) }}
          </span>
        </div>
        <p class="crit__explain">{{ t('chat.line.maxFailLead') }}</p>

        <div class="crit__grid">
          <label class="crit__field">
            <span>{{ t('chat.line.budgetTokens') }}</span>
            <input
              class="crit__num"
              type="number"
              min="0"
              step="1000"
              :value="spec.criteria.budgetTokens ?? ''"
              @input="setCriteria({ budgetTokens: optNum(($event.target as HTMLInputElement).value) })"
            />
          </label>
          <label class="crit__field">
            <span>{{ t('chat.line.deadlineMs') }}</span>
            <input
              class="crit__num"
              type="number"
              min="0"
              step="1000"
              :value="spec.criteria.deadlineMs ?? ''"
              @input="setCriteria({ deadlineMs: optNum(($event.target as HTMLInputElement).value) })"
            />
          </label>
        </div>
        <!-- 空着就是"不设这条 kill-check"，界面必须说出来：留白和 0 在判据里含义完全不同。 -->
        <p class="crit__explain">
          {{ t('chat.line.thresholdNote', { tokens: thresholdText('budgetTokens'), deadline: thresholdText('deadlineMs') }) }}
        </p>
      </section>
    </div>
  </ContentSheet>
</template>

<script setup lang="ts">
// 判据面板（CriteriaSheet）—— 把"什么算做完"和"什么情况下别做了"放在一起。
//
// 两块必须同一个入口：验收条款（acceptance）决定产物长什么样才被接受，
// 判据（criteria）决定这次派单在概率/预算上是否值得。旧界面两者都没有，
// 于是"跑完了"的唯一含义是"文字流结束了"。
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'

import ContentSheet from '@/components/chat/ContentSheet.vue'
import Icon from '@/components/Icon.vue'
import type { LineCatalog, LineCriteria, LineSpec } from '@/composables/line/types'

const props = defineProps<{
  open: boolean
  spec: LineSpec
  catalog: LineCatalog
}>()

const emit = defineEmits<{
  close: []
  'update:spec': [LineSpec]
}>()

const { t } = useI18n()

const sheetLabel = computed(() => t('chat.line.criteriaTitle'))
const defaultCriteria = computed(() => props.catalog.defaults?.criteria ?? null)

function num(value: string, fallback: number): number {
  const parsed = Number(value)
  return Number.isFinite(parsed) ? parsed : fallback
}

function optNum(value: string): number | null {
  if (value.trim() === '') return null
  const parsed = Number(value)
  return Number.isFinite(parsed) && parsed > 0 ? parsed : null
}

function pct(value: number): string {
  return `${Math.round(value * 100)}%`
}

function thresholdText(field: 'budgetTokens' | 'deadlineMs'): string {
  const value = props.spec.criteria[field]
  return value === null || value === undefined ? t('chat.line.thresholdOff') : String(value)
}

function setCriteria(next: Partial<LineCriteria>): void {
  emit('update:spec', { ...props.spec, criteria: { ...props.spec.criteria, ...next } })
}

function addClause(): void {
  emit('update:spec', { ...props.spec, acceptance: [...props.spec.acceptance, ''] })
}

function setClause(index: number, value: string): void {
  emit('update:spec', {
    ...props.spec,
    acceptance: props.spec.acceptance.map((c, i) => (i === index ? value : c)),
  })
}

function removeClause(index: number): void {
  emit('update:spec', { ...props.spec, acceptance: props.spec.acceptance.filter((_, i) => i !== index) })
}
</script>

<style scoped>
.crit {
  display: flex;
  flex-direction: column;
  gap: var(--sp-4);
  overflow-y: auto;
  width: 100%;
  font-size: 12.5px;
  color: var(--text);
}

.crit__head {
  display: flex;
  flex-direction: column;
  gap: 2px;
}

.crit__hint {
  font-size: 11px;
  color: var(--text-dim);
}

.crit__block {
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.crit__title {
  margin: 0;
  font-size: 12.5px;
  font-weight: 600;
}

.crit__sub,
.crit__explain {
  margin: 0;
  font-size: 11.5px;
  color: var(--text-dim);
}

.crit__list {
  display: flex;
  flex-direction: column;
  gap: 5px;
  margin: 0;
  padding: 0;
  list-style: none;
}

.crit__clause {
  display: flex;
  align-items: center;
  gap: 6px;
}

.crit__clauseNo {
  font-size: 11px;
  color: var(--text-dim);
  font-variant-numeric: tabular-nums;
  width: 16px;
}

.crit__clauseInput,
.crit__num {
  flex: 1;
  border: 1px solid var(--hairline);
  border-radius: var(--radius-xs);
  background: transparent;
  color: var(--text);
  font-size: 12px;
  padding: 5px 7px;
}

.crit__num {
  flex: none;
  width: 120px;
}

.crit__del {
  border: 1px solid var(--hairline);
  border-radius: var(--radius-xs);
  background: transparent;
  color: var(--text-dim);
  cursor: pointer;
  width: 22px;
  height: 22px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
}

.crit__add {
  align-self: flex-start;
  display: inline-flex;
  align-items: center;
  gap: 5px;
  border: 1px dashed var(--hairline);
  border-radius: 999px;
  background: transparent;
  color: var(--text);
  cursor: pointer;
  font-size: 11.5px;
  padding: 4px 10px;
}

.crit__row {
  display: flex;
  align-items: baseline;
  justify-content: space-between;
  gap: 8px;
}

.crit__value {
  font-variant-numeric: tabular-nums;
  font-weight: 600;
}

.crit__sliderRow {
  display: flex;
  align-items: center;
  gap: 10px;
}

.crit__slider {
  flex: 1;
}

.crit__default {
  font-size: 11px;
  color: var(--text-dim);
  cursor: pointer;
  text-decoration: underline dotted;
}

.crit__grid {
  display: flex;
  flex-wrap: wrap;
  gap: 12px;
}

.crit__field {
  display: flex;
  align-items: center;
  gap: 8px;
  font-size: 11.5px;
  color: var(--text-dim);
}
</style>
