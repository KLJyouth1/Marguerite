<template>
  <section v-if="!catalogAvailable" class="lc lc--gone" data-testid="line-composer-unavailable">
    <Icon name="info" :size="12" />
    <span>{{ t('chat.line.catalogUnavailable', { error: codeText(catalogError) || '—' }) }}</span>
  </section>

  <section v-else class="lc" :aria-label="t('chat.line.compose.title')" data-testid="line-composer">
    <div class="lc__arm" :class="{ 'lc__arm--on': armed }">
      <button
        type="button"
        role="switch"
        class="lc__switch"
        :class="{ 'lc__switch--on': armed }"
        :aria-checked="armed"
        :aria-label="t('chat.line.compose.armSwitchLabel')"
        :aria-describedby="shownRefusal ? 'line-arm-refusal' : undefined"
        :title="t('chat.line.compose.armExplain')"
        data-testid="line-arm-switch"
        @click="toggleArm"
      >
        <span class="lc__track" aria-hidden="true"><span class="lc__knob" /></span>
        <strong class="lc__armHead">{{ armed ? t('chat.line.compose.armOn') : t('chat.line.compose.armOff') }}</strong>
      </button>

      <div class="lc__armMeta">
        <span class="lc__shape" data-testid="line-composer-shape">{{ shapeSummary }}</span>
        <span
          class="lc__goal"
          :class="{ 'lc__goal--empty': !goalText }"
          :title="goalTip"
          data-testid="line-composer-goal"
        >
          <span class="lc__goalKey">{{ t('chat.line.compose.goalLabel') }}</span>
          {{ goalText || t('chat.line.compose.goalMissing') }}
        </span>
        <span
          class="lc__gate"
          :class="`lc__gate--${gate}`"
          :title="gateTitle"
          data-testid="line-composer-gate"
        >
          <Icon :name="gate === 'allow' ? 'check' : 'shield'" :size="11" />
          {{ t(`chat.line.compose.gate.${gate}`) }}
        </span>
      </div>

      <p
        v-if="shownRefusal"
        id="line-arm-refusal"
        class="lc__refuse"
        role="status"
        data-testid="line-arm-refused"
      >
        <Icon name="alertTriangle" :size="11" />
        {{ t('chat.line.compose.armRefused', { error: codeText(shownRefusal) }) }}
      </p>
      <p
        v-else-if="armed && blocked.length > 0"
        class="lc__refuse"
        role="status"
        data-testid="line-arm-blocked"
      >
        <Icon name="alertTriangle" :size="11" />
        {{ t('chat.line.compose.armButBlocked', { count: blocked.length }) }}
      </p>
    </div>

    <span v-if="preflightLoading" class="lc__sweep" aria-hidden="true" data-testid="line-recalc-sweep" />

    <ShapeRail
      class="lc__rail"
      :spec="spec"
      :catalog="catalog"
      :catalog-error="catalogError"
      :errors="blocked"
      :warnings="warnList"
      @update:spec="emit('update:spec', $event)"
      @apply-recipe="onRecipe"
      @open-roster="openRoster"
      @open-criteria="criteriaOpen = true"
    />
    <p v-if="recipeNotice" class="lc__notice" role="status" data-testid="line-recipe-unavailable">
      <Icon name="info" :size="11" />
      {{ t('chat.line.recipeUnavailable') }}
    </p>

    <div class="lc__pfBar">
      <strong class="lc__pfTitle">{{ t('chat.line.preflightTitle') }}</strong>
      <span v-if="preflightLoading" class="lc__spin" data-testid="line-preflight-loading">
        {{ t('chat.line.compose.recalculating') }}
      </span>
      <span v-else-if="!preflight" class="lc__idle" data-testid="line-preflight-none">
        {{ t('chat.line.compose.noPreflight') }}
      </span>
      <button
        type="button"
        class="lc__refresh"
        :disabled="preflightLoading"
        data-testid="line-preflight-refresh"
        @click="emit('preflight-refresh')"
      >
        <Icon name="refresh" :size="12" />
        {{ t('chat.line.compose.recalculate') }}
      </button>
    </div>

    <PreflightCard
      :preflight="preflight ?? null"
      :loading="preflightLoading"
      :spec="spec"
      :catalog="catalog"
      :plan="plan ?? null"
    />

    <p v-if="running" class="lc__running" data-testid="line-run-dispatched">
      <Icon name="zap" :size="12" />
      {{ t('chat.line.compose.dispatched') }}
    </p>

    <div v-if="blocked.length > 0" class="lc__block" data-testid="line-blocking">
      <strong class="lc__blockHead">
        <Icon name="lock" :size="12" />
        {{ t('chat.line.compose.blockedTitle', { count: blocked.length }) }}
      </strong>
      <ul class="lc__list">
        <li v-for="(message, index) in blocked" :key="`${index}-${message}`">{{ message }}</li>
      </ul>
    </div>

    <div v-if="warnList.length > 0" class="lc__warns" data-testid="line-warnings">
      <strong class="lc__warnsHead">
        <Icon name="info" :size="12" />
        {{ t('chat.line.compose.warningsTitle', { count: warnList.length }) }}
      </strong>
      <ul class="lc__list lc__list--soft">
        <li v-for="(message, index) in warnList" :key="`${index}-${message}`">{{ message }}</li>
      </ul>
    </div>

    <p v-if="lastRunError" class="lc__fail" data-testid="line-run-error">
      <Icon name="alertTriangle" :size="12" />
      <span class="lc__failLabel">{{ t('chat.line.compose.runFailedLabel') }}</span>
      <span class="lc__failRaw">{{ codeText(lastRunError) }}</span>
    </p>

    <RosterSheet
      :open="rosterStage !== null"
      :stage="rosterStage"
      :catalog="catalog"
      @close="rosterStageId = null"
      @update:stage="writeStage"
    />
    <CriteriaSheet
      :open="criteriaOpen"
      :spec="spec"
      :catalog="catalog"
      @close="criteriaOpen = false"
      @update:spec="emit('update:spec', $event)"
    />
  </section>
</template>

<script setup lang="ts">
// 产线装配台（LineComposer）—— 形状轨 + 预演 + 闸门的**编排层**，自身不持形状。
//
// 组件是哑的：spec / catalog / preflight / 闸门裁决全部由父级（useLineWorkflow）喂进来，
// 这里唯一合法的本地状态是"哪个抽屉开着"——那是视口，不是规格。
// 数字一律来自后端：预演为 null 就说"尚无预演"，绝不本地补零。
import { computed, ref } from 'vue'
import { useI18n } from 'vue-i18n'

import Icon from '@/components/Icon.vue'
import CriteriaSheet from '@/components/chat/line/CriteriaSheet.vue'
import PreflightCard from '@/components/chat/line/PreflightCard.vue'
import RosterSheet from '@/components/chat/line/RosterSheet.vue'
import ShapeRail from '@/components/chat/line/ShapeRail.vue'
import { applyRecipe, summarizeShape } from '@/composables/line/lineSpec'
import { lineCodeText } from '@/composables/line/lineCode'
import type {
  GateDecision,
  LineCatalog,
  LinePattern,
  LinePlan,
  LinePreflight,
  LineRecipe,
  LineSpec,
  LineStage,
} from '@/composables/line/types'

const props = withDefaults(
  defineProps<{
    spec: LineSpec
    catalog: LineCatalog
    catalogError?: string | null
    catalogAvailable?: boolean
    preflight?: LinePreflight | null
    preflightLoading?: boolean
    plan?: LinePlan | null
    blockingErrors?: string[]
    warnings?: string[]
    gateDecision?: GateDecision | 'unknown'
    armed?: boolean
    running?: boolean
    lastRunError?: string | null
    /** 输入框里那句话 —— 产线没有第二个目标输入框，本轮目标就是它。 */
    draftGoal?: string
  }>(),
  {
    catalogError: null,
    catalogAvailable: true,
    preflight: null,
    preflightLoading: false,
    plan: null,
    blockingErrors: () => [],
    warnings: () => [],
    gateDecision: 'unknown',
    armed: false,
    running: false,
    lastRunError: null,
    draftGoal: '',
  },
)

const emit = defineEmits<{
  'update:spec': [LineSpec]
  'update:armed': [boolean]
  'preflight-refresh': []
}>()

const { t } = useI18n()

// 工作流层只回大写状态码（它不认识 i18n），这里是唯一的翻译点；认不出的原样透传。
const codeText = (value: string | null | undefined): string => lineCodeText(value, t)

const blocked = computed(() => props.blockingErrors)
const warnList = computed(() => props.warnings)
const gate = computed(() => props.gateDecision)

const labelOf = (pattern: LinePattern): string =>
  props.catalog.patterns.find((p) => p.id === pattern)?.label ?? pattern

const shapeSummary = computed(() => summarizeShape(props.spec, labelOf))

/**
 * 本轮目标：草稿优先，没有草稿才回落到"上一次真正派出去的那个"。
 *
 * 与工作流层的 `specWithDraft` 同一口径 —— 这里只是把它**显示**出来，不重算规则。
 * 界面上没有第二个目标输入框，所以必须让用户看见"点发送时线瞄准的是哪句话"，
 * 否则 `goal 不可为空` 就是一条无处可解的阻塞（2026-10-31 真机实测）。
 */
const draft = computed(() => props.draftGoal.trim())
const goalText = computed(() => (draft.value ? draft.value : props.spec.goal.trim()))
/** 空目标与草稿目标说的是同一件事：去输入框里改。只有"沿用上次"需要单独解释。 */
const goalHint = computed(() =>
  goalText.value && !draft.value
    ? t('chat.line.compose.goalHintStored')
    : t('chat.line.compose.goalHintDraft'),
)
// 芯片会被裁切，全文只能靠 title 兜住 —— 换行让提示落在第二行，不与目标文本混排。
const goalTip = computed(() => (goalText.value ? `${goalText.value}\n${goalHint.value}` : goalHint.value))

/** 闸门取**所有阶段里最严的一档**，所以文案要把是哪一段在拖它点名出来。 */
const gatedStageNames = computed(() => {
  const names: string[] = []
  props.preflight?.stages.forEach((row, index) => {
    if (row.gated !== true) return
    names.push(
      t('chat.line.compose.gateStage', {
        index: index + 1,
        pattern: props.spec.stages[index] ? labelOf(props.spec.stages[index].pattern) : row.tactic,
      }),
    )
  })
  return names
})

const gateTitle = computed(() => {
  if (gate.value === 'unknown') return t('chat.line.compose.gateUnknownReason')
  if (gatedStageNames.value.length === 0) return t('chat.line.compose.gateStrictestNoStage')
  return t('chat.line.compose.gateStrictest', { stages: gatedStageNames.value.join(' · ') })
})

const armRefusal = ref<string | null>(null)
const shownRefusal = computed(() => (blocked.value.length > 0 ? armRefusal.value : null))

// 阻断时"开闸"等于派一条必然被拒的线，所以给原因而不是静默忽略（拒绝发生在花钱之前）。
function toggleArm(): void {
  const next = !props.armed
  if (next && blocked.value.length > 0) {
    armRefusal.value = blocked.value[0]
    return
  }
  armRefusal.value = null
  emit('update:armed', next)
}

const recipeNotice = ref(false)

function onRecipe(recipe: LineRecipe): void {
  const next = applyRecipe(props.spec, recipe.stages)
  recipeNotice.value = !next.applied
  if (next.applied) emit('update:spec', next.spec)
}

// 抽屉只记 id：形状真相始终在 spec 里，阶段被删则抽屉自动收起。
const rosterStageId = ref<string | null>(null)
const criteriaOpen = ref(false)

const rosterStage = computed<LineStage | null>(
  () => props.spec.stages.find((s) => s.id === rosterStageId.value) ?? null,
)

function openRoster(stage: LineStage): void {
  rosterStageId.value = stage.id
}

function writeStage(next: LineStage): void {
  emit('update:spec', {
    ...props.spec,
    stages: props.spec.stages.map((s) => (s.id === next.id ? next : s)),
  })
}
</script>

<style scoped>
.lc {
  display: flex;
  flex-direction: column;
  gap: var(--sp-2);
  padding: var(--sp-2) 0;
  font-size: 12px;
  color: var(--text);
}

.lc--gone {
  font-size: 11.5px;
  color: var(--text-dim);
}

.lc__arm {
  display: flex;
  flex-direction: column;
  gap: 6px;
  padding: 8px 10px;
  border: 1px solid var(--hairline);
  border-radius: var(--radius-sm);
  background: var(--card);
}

.lc__arm--on {
  border-color: color-mix(in srgb, var(--accent) 45%, var(--hairline));
  background: color-mix(in srgb, var(--accent) 8%, var(--card));
}

.lc__switch {
  display: flex;
  align-items: center;
  gap: 8px;
  border: 0;
  background: transparent;
  color: var(--text);
  cursor: pointer;
  font: inherit;
  font-size: 12.5px;
  padding: 0;
  text-align: left;
}

.lc__switch:focus-visible {
  outline: 2px solid var(--accent);
  outline-offset: 3px;
  border-radius: var(--radius-xs);
}

.lc__track {
  position: relative;
  flex: 0 0 auto;
  width: 30px;
  height: 16px;
  border: 1px solid var(--hairline);
  border-radius: 999px;
  background: color-mix(in srgb, var(--text-dim) 22%, var(--card));
  transition:
    background var(--dur-fast) var(--ease-standard),
    border-color var(--dur-fast) var(--ease-standard);
}

.lc__knob {
  position: absolute;
  top: 2px;
  left: 2px;
  width: 10px;
  height: 10px;
  border-radius: 50%;
  background: var(--card);
  box-shadow: var(--shadow-sm);
  transition: transform var(--dur-fast) var(--ease-spring);
}

.lc__switch--on .lc__track {
  border-color: color-mix(in srgb, var(--accent) 60%, var(--hairline));
  background: color-mix(in srgb, var(--accent) 55%, var(--card));
}

.lc__switch--on .lc__knob {
  transform: translateX(14px);
}

.lc__armHead {
  min-width: 0;
  font-weight: 600;
}

.lc__armMeta {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 6px;
}

.lc__shape {
  font-size: 11.5px;
  color: var(--text-dim);
  font-variant-numeric: tabular-nums;
  word-break: break-word;
}

.lc__goal {
  display: inline-flex;
  align-items: baseline;
  gap: 5px;
  min-width: 0;
  max-width: 100%;
  padding: 1px 8px;
  border: 1px dashed var(--hairline);
  border-radius: 999px;
  font-size: 11px;
  color: var(--text-dim);
  cursor: help;
}

.lc__goal--empty {
  color: var(--warn);
  border-color: color-mix(in srgb, var(--warn) 45%, var(--hairline));
}

.lc__goalKey {
  flex: 0 0 auto;
  font-weight: 600;
  color: var(--text);
}

/* 目标是一句话，不是标签：装不下就省略号，但 title 里给全文（不能靠裁切骗过对账）。 */
.lc__goal > :last-child {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.lc__gate {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  margin-left: auto;
  padding: 2px 8px;
  border: 1px solid var(--hairline);
  border-radius: 999px;
  font-size: 11px;
  font-variant-numeric: tabular-nums;
}

.lc__gate--allow {
  color: var(--ok);
  border-color: color-mix(in srgb, var(--ok) 50%, var(--hairline));
  background: color-mix(in srgb, var(--ok) 12%, var(--card));
}

.lc__gate--confirm {
  color: var(--warn);
  border-color: color-mix(in srgb, var(--warn) 55%, var(--hairline));
  background: color-mix(in srgb, var(--warn) 14%, var(--card));
}

.lc__gate--deny {
  color: var(--danger);
  border-color: color-mix(in srgb, var(--danger) 55%, var(--hairline));
  background: color-mix(in srgb, var(--danger) 12%, var(--card));
}

.lc__gate--unknown {
  color: var(--text-dim);
}

.lc__refuse {
  display: flex;
  align-items: flex-start;
  gap: 5px;
  margin: 0;
  font-size: 11.5px;
  color: var(--danger);
}

.lc__sweep {
  height: 2px;
  border-radius: 999px;
  background: linear-gradient(
    to right,
    transparent,
    color-mix(in srgb, var(--accent) 70%, transparent),
    transparent
  );
  background-size: 200% 100%;
  animation: lcSweep var(--dur-slow) linear infinite;
}

@keyframes lcSweep {
  from {
    background-position: 0% 0;
  }
  to {
    background-position: -200% 0;
  }
}

.lc__rail {
  min-width: 0;
}

.lc__notice {
  display: flex;
  align-items: center;
  gap: 6px;
  margin: 0;
  padding: 6px 9px;
  border: 1px dashed color-mix(in srgb, var(--warn) 45%, var(--hairline));
  border-radius: var(--radius-sm);
  font-size: 11.5px;
  color: var(--warn);
}

.lc__pfBar {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 8px;
}

.lc__pfTitle {
  font-size: 12px;
  font-weight: 600;
}

.lc__spin {
  font-size: 11px;
  color: var(--accent);
}

.lc__idle {
  font-size: 11px;
  color: var(--text-dim);
}

.lc__refresh {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  margin-left: auto;
  border: 1px solid var(--hairline);
  border-radius: 999px;
  background: transparent;
  color: var(--text);
  cursor: pointer;
  font-size: 11.5px;
  padding: 3px 10px;
  transition: border-color var(--dur-fast) var(--ease-standard);
}

.lc__refresh:hover:not(:disabled) {
  border-color: color-mix(in srgb, var(--accent) 50%, var(--hairline));
}

.lc__refresh:disabled {
  cursor: default;
  opacity: 0.5;
}

.lc__running {
  display: flex;
  align-items: center;
  gap: 6px;
  margin: 0;
  font-size: 11.5px;
  color: var(--text-dim);
}

.lc__block,
.lc__warns {
  display: flex;
  flex-direction: column;
  gap: 4px;
  padding: 8px 10px;
  border-radius: var(--radius-sm);
}

.lc__block {
  border: 1px solid color-mix(in srgb, var(--danger) 45%, var(--hairline));
  background: color-mix(in srgb, var(--danger) 8%, var(--card));
}

.lc__blockHead {
  display: flex;
  align-items: center;
  gap: 6px;
  font-size: 11.5px;
  color: var(--danger);
}

.lc__warns {
  border: 1px dashed color-mix(in srgb, var(--warn) 40%, var(--hairline));
}

.lc__warnsHead {
  display: flex;
  align-items: center;
  gap: 6px;
  font-size: 11.5px;
  font-weight: 600;
  color: var(--warn);
}

.lc__list {
  margin: 0;
  padding-left: 16px;
  display: flex;
  flex-direction: column;
  gap: 3px;
  font-size: 11.5px;
  color: var(--text);
}

.lc__list--soft {
  color: var(--text-dim);
}

.lc__fail {
  display: flex;
  align-items: flex-start;
  gap: 6px;
  margin: 0;
  padding: 8px 10px;
  border-left: 3px solid var(--danger);
  border-radius: var(--radius-xs);
  background: color-mix(in srgb, var(--danger) 10%, var(--card));
  font-size: 11.5px;
}

.lc__failLabel {
  flex: 0 0 auto;
  color: var(--danger);
  font-weight: 600;
}

.lc__failRaw {
  color: var(--text);
  word-break: break-word;
}

@media (max-width: 520px) {
  .lc__gate,
  .lc__refresh {
    margin-left: 0;
  }

  .lc__armMeta,
  .lc__pfBar {
    align-items: flex-start;
  }
}

@media (prefers-reduced-motion: reduce) {
  .lc__sweep {
    animation: none;
  }

  .lc__track,
  .lc__knob,
  .lc__refresh {
    transition: none;
  }
}
</style>
