<template>
  <section
    v-if="line.catalogAvailable.value"
    class="ld"
    :class="{ 'ld--open': open, 'ld--armed': armed }"
    data-testid="line-dock"
    :aria-label="t('chat.line.dock.title')"
  >
    <div class="ld__bar">
      <span class="ld__lead">
        <Icon name="listTree" :size="13" />
        <strong class="ld__title">{{ t('chat.line.dock.title') }}</strong>
      </span>

      <span class="ld__ladder" role="group" :aria-label="t('chat.line.dock.ladderAria')">
        <span class="ld__shape" data-testid="line-dock-shape">{{ shapeText }}</span>
        <span
          class="ld__gate"
          :class="`ld__gate--${gate}`"
          :title="gate === 'unknown' ? t('chat.line.compose.gateUnknownReason') : undefined"
          data-testid="line-dock-gate"
        >
          <Icon :name="gate === 'allow' ? 'check' : 'shield'" :size="11" />
          {{ t(`chat.line.compose.gate.${gate}`) }}
        </span>
      </span>

      <p
        v-if="blockedVisible"
        class="ld__blocked"
        role="status"
        data-testid="line-dock-blocked"
      >
        <Icon name="alertTriangle" :size="11" />
        {{ t('chat.line.compose.armButBlocked', { count: blockedCount }) }}
      </p>

      <button
        v-if="gearVisible"
        type="button"
        class="ld__gear"
        :title="t('chat.line.dock.fromGearHint')"
        :data-gear-key="gearKey"
        data-testid="line-dock-from-gear"
        @click="useLegacyGear"
      >
        <Icon name="zap" :size="12" />
        {{ t('chat.line.dock.fromGear') }}
      </button>

      <p
        v-if="gearFailed"
        class="ld__gearFail"
        role="status"
        data-testid="line-dock-from-gear-failed"
      >
        <Icon name="alertTriangle" :size="11" />
        {{ t('chat.line.dock.fromGearFailed') }}
      </p>

      <p
        v-if="shownRefusal"
        class="ld__refuse"
        role="status"
        data-testid="line-dock-arm-refused"
      >
        <Icon name="alertTriangle" :size="11" />
        {{ t('chat.line.compose.armRefused', { error: codeText(shownRefusal) }) }}
      </p>

      <button
        type="button"
        role="switch"
        class="ld__switch"
        :class="{ 'ld__switch--on': armed }"
        :aria-checked="armed"
        :aria-label="t('chat.line.compose.armSwitchLabel')"
        :title="t('chat.line.compose.armExplain')"
        data-testid="line-arm-dock"
        @click="toggleArm"
      >
        <span class="ld__track" aria-hidden="true"><span class="ld__knob" /></span>
        <span class="ld__armState">{{ armed ? t('chat.line.compose.armOn') : t('chat.line.compose.armOff') }}</span>
      </button>

      <button
        type="button"
        class="ld__toggle"
        :aria-expanded="open"
        :aria-label="open ? t('chat.line.dock.collapse') : t('chat.line.dock.expand')"
        :title="open ? t('chat.line.dock.collapse') : t('chat.line.dock.expand')"
        data-testid="line-dock-toggle"
        @click="open = !open"
      >
        <Icon name="chevronDown" :size="13" class="ld__chev" :class="{ 'ld__chev--open': open }" />
      </button>
    </div>

    <div v-if="open" class="ld__body" data-testid="line-dock-body">
      <LineComposer
        class="ld__composer"
        :spec="line.spec.value"
        :catalog="line.catalog.value"
        :catalog-error="line.catalogError.value"
        :catalog-available="line.catalogAvailable.value"
        :preflight="line.preflight.value"
        :preflight-loading="line.preflightLoading.value"
        :plan="line.plan.value"
        :blocking-errors="line.blockingErrors.value"
        :warnings="line.warnings.value"
        :gate-decision="line.gateDecision.value"
        :armed="line.armed.value"
        :running="line.running.value"
        :last-run-error="line.lastRunError.value"
        :draft-goal="line.draftGoal.value"
        @update:spec="writeSpec"
        @update:armed="writeArmed"
        @preflight-refresh="refreshPreflight"
      />
    </div>
  </section>
</template>

<script setup lang="ts">
// 产线停靠条（LineDock）—— 输入框上方那条**永远只占一行**的窄条，展开即是 LineComposer。
//
// 为什么要有它：装配台整块摊开时，聊天页被一个不常用的面板顶走了首屏；完全收掉又会让
// "这条线已经挂上、发送会走产线"这件事变成用户看不见的暗礁。挂线状态必须**常驻可见**，
// 细节则按需展开。
//
// 状态纪律：这里**一个事实源都不持有**。形状、闸门、阻塞项全部读 `line.*`（useLineWorkflow
// 的实例），写回也直接落在那个实例上 —— 组件不 emit，因为它改的就是唯一那份状态。
// 唯一的本地状态是"展开还是收起"：那是视口偏好，不是规格。
//
// 诚实纪律：目录 RPC 不存在 ⇒ 整条不渲染（不给一套假控件）；挂线但被阻塞 ⇒ 明说还差几项，
// 不让开关看着像能用；"从当前挡位生成"失败 ⇒ 说失败，绝不静默。
import { computed, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'

import Icon from '@/components/Icon.vue'
import LineComposer from '@/components/chat/line/LineComposer.vue'
import { lineCodeText } from '@/composables/line/lineCode'
import type { LineWorkflow } from '@/composables/line/useLineWorkflow'
import type { LegacyOrchestrationForm, LineSpec } from '@/composables/line/types'

const OPEN_STORAGE_KEY = 'marguerite.line.dock.open.v1'

const props = withDefaults(
  defineProps<{
    line: LineWorkflow
    gearKey?: string
    legacyForm?: LegacyOrchestrationForm
    legacyKnobs?: { agents: string[]; teams: string[]; swarmSize: number }
  }>(),
  {
    gearKey: '',
    legacyForm: 'single',
    legacyKnobs: () => ({ agents: [], teams: [], swarmSize: 3 }),
  },
)

const { t } = useI18n()

/** 隐私模式 / 配额满 ⇒ 静默：视口偏好丢了不能影响发送（同 useLineWorkflow 的口径）。 */
function readStoredOpen(): boolean | null {
  try {
    const raw = globalThis.localStorage?.getItem(OPEN_STORAGE_KEY)
    return raw === 'true' ? true : raw === 'false' ? false : null
  } catch {
    return null
  }
}

function writeStoredOpen(value: boolean): void {
  try {
    globalThis.localStorage?.setItem(OPEN_STORAGE_KEY, value ? 'true' : 'false')
  } catch {
    // 保持内存态。
  }
}

// 缺省跟随"已挂线"：挂线时收起会把最该看见的状态藏起来，未挂线时展开只是噪声。
const open = ref<boolean>(readStoredOpen() ?? props.line.armed.value)
watch(open, (value) => writeStoredOpen(value))

const armed = computed(() => props.line.armed.value)
const shapeText = computed(() => props.line.shapeText.value)
const gate = computed(() => props.line.gateDecision.value)
const blockedCount = computed(() => props.line.blockingErrors.value.length)

/** 挂着线却派不出去就说出来（数字是阻塞项条数，不是形容词）。 */
const blockedVisible = computed(
  () => armed.value && !props.line.canRun.value && blockedCount.value > 0,
)

/** `single` 就是"没开协作"，从它生成等于把线清成 solo —— 那不是升级，是清空。 */
const gearVisible = computed(() => props.legacyForm !== 'single')

const gearFailed = ref(false)

/**
 * 挂线被拒的原因（与装配台 `LineComposer.toggleArm` 同一语义）。
 *
 * 两个开关做同一件事却一个拒、一个照样点亮，用户只能靠猜哪个是真的 ——
 * 状态位必须只有一个解释。被拦着时的"上膛"是一次无效状态，所以拒了要说出来。
 */
const armRefusal = ref<string | null>(null)
const shownRefusal = computed(() => (blockedCount.value > 0 ? armRefusal.value : null))

// 工作流层只回大写状态码（它不认识 i18n），这里与装配台同一处理：认不出就原样透传。
const codeText = (value: string | null | undefined): string => lineCodeText(value, t)

function toggleArm(): void {
  const next = !armed.value
  if (next && blockedCount.value > 0) {
    armRefusal.value = props.line.blockingErrors.value[0] ?? null
    return
  }
  armRefusal.value = null
  props.line.armed.value = next
}

/* 装配台的三个回调全部落回同一个工作流实例 —— 组件不 emit，因为没有第二份状态可通知。 */
function writeSpec(next: LineSpec): void {
  props.line.setSpec(next)
}

function writeArmed(value: boolean): void {
  props.line.armed.value = value
}

function refreshPreflight(): void {
  void props.line.refreshPreflight()
}

function useLegacyGear(): void {
  const ok = props.line.armNextFromLegacy(props.legacyKnobs, props.legacyForm)
  gearFailed.value = !ok
  // 成功也要看得见：刚被写进形状的东西不该停在一个收起的条后面。
  if (ok) open.value = true
}
</script>

<style scoped>
.ld {
  display: flex;
  flex-direction: column;
  gap: var(--sp-1);
  padding: var(--sp-1) 0;
  font-size: 12px;
  color: var(--text);
}

.ld__bar {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 6px;
  padding: 4px 8px;
  border: 1px solid var(--hairline);
  border-radius: var(--radius-sm);
  background: var(--card);
}

.ld--armed .ld__bar {
  border-color: color-mix(in srgb, var(--accent) 45%, var(--hairline));
  background: color-mix(in srgb, var(--accent) 8%, var(--card));
}

.ld__lead {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  flex: 0 0 auto;
}

.ld__title {
  font-size: 11.5px;
  font-weight: 600;
  letter-spacing: 0.02em;
}

.ld__ladder {
  display: inline-flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 6px;
  min-width: 0;
}

.ld__shape {
  font-size: 11.5px;
  color: var(--text-dim);
  font-variant-numeric: tabular-nums;
  word-break: break-word;
}

.ld__gate {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  padding: 1px 7px;
  border: 1px solid var(--hairline);
  border-radius: 999px;
  font-size: 10.5px;
  font-variant-numeric: tabular-nums;
}

.ld__gate--allow {
  color: var(--ok);
  border-color: color-mix(in srgb, var(--ok) 50%, var(--hairline));
  background: color-mix(in srgb, var(--ok) 12%, var(--card));
}

.ld__gate--confirm {
  color: var(--warn);
  border-color: color-mix(in srgb, var(--warn) 55%, var(--hairline));
  background: color-mix(in srgb, var(--warn) 14%, var(--card));
}

.ld__gate--deny {
  color: var(--danger);
  border-color: color-mix(in srgb, var(--danger) 55%, var(--hairline));
  background: color-mix(in srgb, var(--danger) 12%, var(--card));
}

.ld__gate--unknown {
  color: var(--text-dim);
}

.ld__blocked,
.ld__gearFail,
.ld__refuse {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  margin: 0;
  font-size: 11px;
  color: var(--danger);
}

.ld__gear {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  border: 1px solid var(--hairline);
  border-radius: 999px;
  background: transparent;
  color: var(--text);
  cursor: pointer;
  font: inherit;
  font-size: 11px;
  padding: 2px 9px;
  transition: border-color var(--dur-fast) var(--ease-standard);
}

.ld__gear:hover {
  border-color: color-mix(in srgb, var(--accent) 50%, var(--hairline));
}

.ld__switch {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  margin-left: auto;
  border: 0;
  background: transparent;
  color: var(--text);
  cursor: pointer;
  font: inherit;
  font-size: 11.5px;
  padding: 0;
}

.ld__switch:focus-visible,
.ld__toggle:focus-visible {
  outline: 2px solid var(--accent);
  outline-offset: 3px;
  border-radius: var(--radius-xs);
}

.ld__track {
  position: relative;
  flex: 0 0 auto;
  width: 26px;
  height: 14px;
  border: 1px solid var(--hairline);
  border-radius: 999px;
  background: color-mix(in srgb, var(--text-dim) 22%, var(--card));
  transition:
    background var(--dur-fast) var(--ease-standard),
    border-color var(--dur-fast) var(--ease-standard);
}

.ld__knob {
  position: absolute;
  top: 2px;
  left: 2px;
  width: 8px;
  height: 8px;
  border-radius: 50%;
  background: var(--card);
  box-shadow: var(--shadow-sm);
  transition: transform var(--dur-fast) var(--ease-spring);
}

.ld__switch--on .ld__track {
  border-color: color-mix(in srgb, var(--accent) 60%, var(--hairline));
  background: color-mix(in srgb, var(--accent) 55%, var(--card));
}

.ld__switch--on .ld__knob {
  transform: translateX(12px);
}

.ld__armState {
  font-weight: 600;
}

.ld__toggle {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  flex: 0 0 auto;
  width: 22px;
  height: 22px;
  border: 1px solid var(--hairline);
  border-radius: var(--radius-xs);
  background: transparent;
  color: var(--text-dim);
  cursor: pointer;
  padding: 0;
  transition:
    color var(--dur-fast) var(--ease-standard),
    border-color var(--dur-fast) var(--ease-standard);
}

.ld__toggle:hover {
  color: var(--text);
  border-color: color-mix(in srgb, var(--accent) 45%, var(--hairline));
}

.ld__chev {
  transition: transform var(--dur-fast) var(--ease-standard);
}

.ld__chev--open {
  transform: rotate(180deg);
}

.ld__body {
  padding: var(--sp-1) 8px 0;
  border: 1px solid var(--hairline);
  border-radius: var(--radius-sm);
  background: var(--bg);
}

/* 展开时窄条与装配台连成一块：中间留缝会让"这是同一个东西的两层"读丢。 */
.ld--open {
  gap: 0;
}

.ld--open .ld__bar {
  border-bottom-left-radius: 0;
  border-bottom-right-radius: 0;
}

.ld--open .ld__body {
  border-top-left-radius: 0;
  border-top-right-radius: 0;
}

.ld__composer {
  min-width: 0;
}

@media (max-width: 520px) {
  .ld__switch {
    margin-left: 0;
  }
}

@media (prefers-reduced-motion: reduce) {
  .ld__track,
  .ld__knob,
  .ld__gear,
  .ld__toggle,
  .ld__chev {
    transition: none;
  }
}
</style>
