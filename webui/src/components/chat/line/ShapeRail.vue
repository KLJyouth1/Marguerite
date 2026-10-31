<template>
  <section class="rail" data-testid="line-shape-rail">
    <div class="rail__presets">
      <span class="rail__presetsLabel">{{ t('chat.line.presets') }}</span>
      <button
        v-for="recipe in catalog.recipes"
        :key="recipe.id"
        type="button"
        class="rail__preset"
        :disabled="recipe.stages.length === 0"
        :title="recipeTitle(recipe)"
        @click="emit('apply-recipe', recipe)"
      >
        {{ recipe.label }}
        <span v-if="recipe.stages.length === 0" class="rail__presetEmpty">{{ t('chat.line.recipeUnavailable') }}</span>
      </button>
      <span v-if="catalog.loaded && catalog.recipes.length === 0" class="rail__empty">
        {{ t('chat.line.noRecipes') }}
      </span>
      <span v-if="!catalog.loaded" class="rail__empty">{{ catalogHint }}</span>
    </div>

    <ol class="rail__track">
      <li v-for="(stage, index) in spec.stages" :key="stage.id" class="rail__slot">
        <article class="rail__card" :class="{ 'rail__card--bad': stageErrors(stage.id).length > 0 }">
          <header class="rail__cardHead">
            <span class="rail__ord">{{ index + 1 }}</span>
            <div class="rail__cardOps">
              <button
                type="button"
                class="rail__op"
                :disabled="index === 0"
                :aria-label="t('chat.line.moveLeft')"
                @click="move(index, index - 1)"
              >
                <Icon name="chevronLeft" :size="13" />
              </button>
              <button
                type="button"
                class="rail__op"
                :disabled="index === spec.stages.length - 1"
                :aria-label="t('chat.line.moveRight')"
                @click="move(index, index + 1)"
              >
                <Icon name="chevronRight" :size="13" />
              </button>
              <button
                type="button"
                class="rail__op rail__op--del"
                :disabled="spec.stages.length <= 1"
                :aria-label="t('chat.line.removeStage')"
                @click="remove(index)"
              >
                <Icon name="x" :size="13" />
              </button>
            </div>
          </header>

          <div class="rail__shapes" role="group" :aria-label="t('chat.line.shapeGroup')">
            <button
              v-for="meta in catalog.patterns"
              :key="meta.id"
              type="button"
              class="rail__shape"
              :class="{ 'rail__shape--on': stage.pattern === meta.id }"
              :aria-pressed="stage.pattern === meta.id"
              :title="`${patternLabel(meta)} · ${patternHint(meta)} · ${rosterKindText(meta.rosterKind)}`"
              @click="pickPattern(stage, meta)"
            >
              <Icon :name="patternIcon(meta.id)" :size="14" />
              <span class="rail__shapeLabel">{{ patternLabel(meta) }}</span>
            </button>
            <span v-if="!catalog.loaded" class="rail__empty">{{ catalogHint }}</span>
          </div>

          <div class="rail__row">
            <label class="rail__fieldLabel" :for="`width-${stage.id}`">{{ t('chat.line.width') }}</label>
            <div class="rail__stepper">
              <button
                type="button"
                class="rail__op"
                :disabled="isSolo(stage) || stage.width <= 1"
                :aria-label="t('chat.line.widthMinus')"
                @click="setWidth(stage, stage.width - 1)"
              >
                −
              </button>
              <input
                :id="`width-${stage.id}`"
                class="rail__width"
                type="number"
                min="1"
                :max="maxWidth"
                :disabled="isSolo(stage)"
                :value="isSolo(stage) ? 1 : stage.width"
                @change="onWidthInput(stage, $event)"
              />
              <button
                type="button"
                class="rail__op"
                :disabled="isSolo(stage) || stage.width >= maxWidth"
                :aria-label="t('chat.line.widthPlus')"
                @click="setWidth(stage, stage.width + 1)"
              >
                ＋
              </button>
            </div>
            <span
              v-if="overConcurrency(stage)"
              class="rail__flag rail__flag--warn"
              :title="t('chat.line.overConcurrency', { width: stage.width, cap: concurrencyCap })"
            >
              <Icon name="alertTriangle" :size="12" />
            </span>
          </div>

          <button
            type="button"
            class="rail__cast"
            :class="{ 'rail__cast--missing': castMissing(stage) }"
            :disabled="!openable(stage)"
            @click="emit('open-roster', stage)"
          >
            <Icon :name="openable(stage) ? 'collab' : 'info'" :size="13" />
            <span>{{ castLabel(stage) }}</span>
            <span v-if="openable(stage)" class="rail__castCount">{{ castCount(stage) }}</span>
          </button>

          <label class="rail__check">
            <input
              type="checkbox"
              :checked="stage.synthesize"
              :disabled="stage.pattern === 'solo'"
              @change="patch(stage, { synthesize: ($event.target as HTMLInputElement).checked })"
            />
            <span>{{ t('chat.line.synthesize') }}</span>
          </label>

          <input
            class="rail__note"
            type="text"
            :value="stage.note"
            :placeholder="t('chat.line.notePlaceholder')"
            :aria-label="t('chat.line.note')"
            @input="patch(stage, { note: ($event.target as HTMLInputElement).value })"
          />

          <ul v-if="stageErrors(stage.id).length" class="rail__errors">
            <li v-for="message in stageErrors(stage.id)" :key="message">{{ message }}</li>
          </ul>
        </article>

        <span
          v-if="index < spec.stages.length - 1"
          class="rail__link"
          :title="t('chat.line.carryHint')"
          aria-hidden="true"
        >
          <Icon name="chevronRight" :size="16" />
          <em>{{ t('chat.line.carry') }}</em>
        </span>
      </li>
    </ol>

    <div class="rail__foot">
      <button
        type="button"
        class="rail__add"
        :disabled="!canAdd"
        :title="canAdd ? '' : t('chat.line.stageCap', { cap: stageCap })"
        @click="add"
      >
        <Icon name="plus" :size="13" />
        {{ t('chat.line.addStage') }}
      </button>
      <button type="button" class="rail__criteria" @click="emit('open-criteria')">
        <Icon name="gauge" :size="13" />
        {{ t('chat.line.criteria') }}
        <span class="rail__castCount">{{ spec.acceptance.length }}</span>
      </button>
      <span class="rail__shapeText" data-testid="line-shape-text">{{ shapeText }}</span>
    </div>
  </section>
</template>

<script setup lang="ts">
// 形状轨（ShapeRail）—— 产线的**唯一**编辑面。
//
// 为什么不是"下拉框里选一个挡位"：旧面板把协作形态做成了一次性选择，选完就再也看不见
// 它由什么组成（蜂群到底几路、专家团到底是哪几个团）。这里把整条线摊在一条可横向
// 读序的轨上：**阶段 = 卡片，串行 = 卡间的「接力」标记，形状词表直接摊开成 5 个按钮**。
// 5 种形状是能穷举的词汇表，藏进菜单等于让用户背。
//
// 所有候选与上限都来自 `line.catalog`；目录没到位时控件留白并说明原因，绝不顶一套假常量。
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'

import Icon from '@/components/Icon.vue'
import type { LineCatalog, LinePattern, LinePatternMeta, LineSpec, LineStage, RosterKind } from '@/composables/line/types'
import { canAddStage, createStage, moveStage, setStagePattern } from '@/composables/line/lineSpec'
import { LINE_PATTERNS } from '@/composables/line/types'

const props = defineProps<{
  spec: LineSpec
  catalog: LineCatalog
  catalogError?: string | null
  errors?: string[]
  warnings?: string[]
}>()

const emit = defineEmits<{
  'update:spec': [LineSpec]
  'apply-recipe': [LineCatalog['recipes'][number]]
  'open-roster': [LineStage]
  'open-criteria': []
}>()

const { t, te } = useI18n()

const stageCap = computed(() => props.catalog.limits?.maxStages ?? 3)
const maxWidth = computed(() => props.catalog.limits?.maxWidth ?? 16)
const concurrencyCap = computed(() => props.catalog.limits?.subagentMaxConcurrent ?? 0)
const canAdd = computed(() => canAddStage(props.spec, props.catalog))
const catalogHint = computed(() =>
  props.catalog.loaded ? '' : t('chat.line.catalogUnavailable', { error: props.catalogError ?? '—' }),
)

const shapeText = computed(() =>
  props.spec.stages
    .map((s) => {
      const meta = props.catalog.patterns.find((p) => p.id === s.pattern) ?? null
      const label = meta ? patternLabel(meta) : s.pattern
      return s.pattern === 'solo' ? label : `${label}×${s.width}`
    })
    .join(' → '),
)

/**
 * 形状名与释义优先走语言包，缺条目时**回落到后端目录的原文**。
 * 词汇表（有哪几种形状、各要什么名册）仍然只有后端一份；这里落的只是译文，
 * 后端新增第六种形状而翻译没跟上时，界面显示英文/中文原标签而不是空白键名。
 */
function patternLabel(meta: LinePatternMeta): string {
  const key = `chat.line.pattern.${meta.id}`
  return te(key) ? t(key) : meta.label
}

function patternHint(meta: LinePatternMeta): string {
  const key = `chat.line.patternHint.${meta.id}`
  return te(key) ? t(key) : meta.description
}

function stageErrors(id: string): string[] {
  return (props.errors ?? []).filter((message) => message.startsWith(`${id}：`))
}

function isSolo(stage: LineStage): boolean {
  return stage.pattern === 'solo'
}

function rosterKind(pattern: LinePattern): RosterKind {
  return props.catalog.patterns.find((p) => p.id === pattern)?.rosterKind ?? 'none'
}

function castLabel(stage: LineStage): string {
  const kind = rosterKind(stage.pattern)
  if (kind === 'teams') return t('chat.line.castTeams')
  if (kind === 'roles') return t('chat.line.castRoles')
  // 蜂群不点名册（由宽度决定几路），但**切面是真下发到本阶段目标里的**，所以给它一个入口。
  if (stage.pattern === 'swarm') return t('chat.line.castFacets')
  return t('chat.line.castNone')
}

/** 这一格的抽屉有没有内容可编辑：名册类形状 + 蜂群切面。 */
function openable(stage: LineStage): boolean {
  return rosterKind(stage.pattern) !== 'none' || stage.pattern === 'swarm'
}

function castCount(stage: LineStage): number {
  return rosterKind(stage.pattern) === 'none' ? stage.facets.length : stage.roster.length
}

/** 名册缺失是**错误**（会静默降级）；切面缺失不是（后端按索引轮转内建切面表）。 */
function castMissing(stage: LineStage): boolean {
  return rosterKind(stage.pattern) !== 'none' && stage.roster.length === 0
}

function overConcurrency(stage: LineStage): boolean {
  return concurrencyCap.value > 0 && stage.pattern !== 'solo' && stage.width > concurrencyCap.value
}

function recipeTitle(recipe: LineCatalog['recipes'][number]): string {
  if (recipe.stages.length === 0) return t('chat.line.recipeUnavailable')
  return recipe.stages.map((s) => s.pattern).join(' → ')
}

function patternIcon(pattern: LinePattern): 'user' | 'fork' | 'collab' | 'swarm' | 'gitMerge' {
  const map: Record<LinePattern, 'user' | 'fork' | 'collab' | 'swarm' | 'gitMerge'> = {
    solo: 'user',
    fanout: 'fork',
    panel: 'collab',
    swarm: 'swarm',
    relay: 'gitMerge',
  }
  return map[pattern] ?? 'user'
}

function rosterKindText(kind: RosterKind): string {
  if (kind === 'teams') return t('chat.line.rosterKindTeams')
  if (kind === 'roles') return t('chat.line.rosterKindRoles')
  return t('chat.line.rosterKindNone')
}

function patch(stage: LineStage, next: Partial<LineStage>): void {
  emit('update:spec', {
    ...props.spec,
    stages: props.spec.stages.map((s) => (s.id === stage.id ? { ...s, ...next } : s)),
  })
}

function pickPattern(stage: LineStage, meta: LinePatternMeta): void {
  if (stage.pattern === meta.id) return
  patch(stage, setStagePattern(stage, meta.id, meta))
}

function setWidth(stage: LineStage, width: number): void {
  patch(stage, { width: Math.min(maxWidth.value, Math.max(1, width)) })
}

function onWidthInput(stage: LineStage, event: Event): void {
  const raw = Number((event.target as HTMLInputElement).value)
  setWidth(stage, Number.isFinite(raw) ? raw : 1)
}

function move(from: number, to: number): void {
  emit('update:spec', { ...props.spec, stages: moveStage(props.spec.stages, from, to) })
}

function remove(index: number): void {
  if (props.spec.stages.length <= 1) return
  emit('update:spec', {
    ...props.spec,
    stages: props.spec.stages.filter((_, i) => i !== index),
  })
}

function add(): void {
  if (!canAdd.value) return
  // 新阶段默认给"并排多写"（fanout）：它是唯一一种填错也不会静默降级的可扩展形状。
  const fallback: LinePattern = LINE_PATTERNS.includes('fanout') ? 'fanout' : 'solo'
  const meta = props.catalog.patterns.find((p) => p.id === fallback) ?? null
  emit('update:spec', { ...props.spec, stages: [...props.spec.stages, createStage(fallback, meta)] })
}
</script>

<style scoped>
.rail {
  display: flex;
  flex-direction: column;
  gap: var(--sp-2);
  padding: var(--sp-2) 0;
}

.rail__presets {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 6px;
  font-size: 11px;
  color: var(--text-dim);
}

.rail__presetsLabel {
  letter-spacing: 0.04em;
  opacity: 0.75;
}

.rail__preset {
  border: 1px solid var(--hairline);
  border-radius: 999px;
  background: color-mix(in srgb, var(--card) 80%, transparent);
  color: var(--text);
  font-size: 11px;
  padding: 3px 9px;
  cursor: pointer;
}

.rail__preset:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}

.rail__presetEmpty {
  margin-left: 4px;
  color: var(--text-dim);
}

.rail__track {
  display: flex;
  align-items: stretch;
  gap: 4px;
  margin: 0;
  padding: 2px 2px 6px;
  list-style: none;
  overflow-x: auto;
  scrollbar-width: thin;
}

.rail__slot {
  display: flex;
  align-items: center;
  gap: 4px;
}

.rail__card {
  display: flex;
  flex-direction: column;
  gap: 6px;
  width: 232px;
  flex: 0 0 auto;
  padding: 8px;
  border: 1px solid var(--hairline);
  border-radius: var(--radius-sm);
  background: var(--card);
}

.rail__card--bad {
  border-color: color-mix(in srgb, var(--danger) 55%, var(--hairline));
}

.rail__cardHead {
  display: flex;
  align-items: center;
  justify-content: space-between;
}

.rail__ord {
  font-size: 11px;
  font-weight: 600;
  color: var(--text-dim);
  font-variant-numeric: tabular-nums;
}

.rail__cardOps {
  display: flex;
  gap: 2px;
}

.rail__op {
  border: 1px solid var(--hairline);
  border-radius: var(--radius-xs);
  background: transparent;
  color: var(--text-dim);
  cursor: pointer;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 20px;
  height: 20px;
  line-height: 1;
  font-size: 12px;
}

.rail__op:hover:not(:disabled) {
  color: var(--text);
  border-color: color-mix(in srgb, var(--accent) 45%, var(--hairline));
}

.rail__op:disabled {
  opacity: 0.35;
  cursor: not-allowed;
}

.rail__shapes {
  display: flex;
  flex-wrap: wrap;
  gap: 3px;
}

.rail__shape {
  display: inline-flex;
  align-items: center;
  gap: 3px;
  border: 1px solid var(--hairline);
  border-radius: 999px;
  background: transparent;
  color: var(--text-dim);
  cursor: pointer;
  font-size: 10.5px;
  padding: 2px 6px;
}

.rail__shape--on {
  border-color: color-mix(in srgb, var(--accent) 60%, var(--hairline));
  background: color-mix(in srgb, var(--accent) 14%, var(--card));
  color: var(--text);
  font-weight: 600;
}

.rail__width {
  width: 34px;
  text-align: center;
  border: 1px solid var(--hairline);
  border-radius: var(--radius-xs);
  background: transparent;
  color: var(--text);
  font-size: 12px;
  font-variant-numeric: tabular-nums;
}

.rail__row {
  display: flex;
  align-items: center;
  gap: 6px;
}

.rail__fieldLabel {
  font-size: 11px;
  color: var(--text-dim);
}

.rail__stepper {
  display: inline-flex;
  align-items: center;
  gap: 3px;
}

.rail__flag--warn {
  color: var(--warn);
  display: inline-flex;
}

.rail__cast {
  display: flex;
  align-items: center;
  gap: 6px;
  border: 1px dashed var(--hairline);
  border-radius: var(--radius-sm);
  background: transparent;
  color: var(--text);
  cursor: pointer;
  font-size: 11.5px;
  padding: 5px 7px;
  text-align: left;
}

.rail__cast:disabled {
  cursor: default;
  opacity: 0.7;
  border-style: solid;
}

.rail__cast--missing {
  color: var(--warn);
  border-color: color-mix(in srgb, var(--warn) 55%, var(--hairline));
}

.rail__castCount {
  margin-left: auto;
  font-variant-numeric: tabular-nums;
  font-size: 11px;
  color: var(--text-dim);
}

.rail__check {
  display: flex;
  align-items: center;
  gap: 6px;
  font-size: 11.5px;
  color: var(--text-dim);
}

.rail__note {
  border: 1px solid var(--hairline);
  border-radius: var(--radius-xs);
  background: transparent;
  color: var(--text);
  font-size: 11px;
  padding: 4px 6px;
}

.rail__errors {
  margin: 0;
  padding-left: 14px;
  font-size: 10.5px;
  color: var(--danger);
}

.rail__link {
  display: flex;
  flex-direction: column;
  align-items: center;
  color: var(--text-dim);
  flex: 0 0 auto;
}

.rail__link em {
  font-style: normal;
  font-size: 9.5px;
  letter-spacing: 0.02em;
}

.rail__foot {
  display: flex;
  align-items: center;
  gap: 8px;
  flex-wrap: wrap;
}

.rail__add,
.rail__criteria {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  border: 1px solid var(--hairline);
  border-radius: 999px;
  background: transparent;
  color: var(--text);
  cursor: pointer;
  font-size: 11.5px;
  padding: 4px 10px;
}

.rail__add:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}

.rail__shapeText {
  margin-left: auto;
  font-size: 11px;
  color: var(--text-dim);
  font-variant-numeric: tabular-nums;
}

.rail__empty {
  font-size: 11px;
  color: var(--text-dim);
}
</style>
