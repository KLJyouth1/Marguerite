<!-- 推理等级拖动条（Codex 风格 · 本桌面端实现）。
     设计取舍（为什么不是一枚开/关按钮）：
       后端 `chat.send` 的 `effort` 参数本来就有四档（off/normal/deep/max），
       旧 UI 只发 `thinking: true`，等于把「深度/极限」两档永久锁死。
       这里把四档做成一条**连续拖动 + 松手吸附**的滑轨：
         · 拖动时填充条连续跟手（无级），松手才吸附到最近档位 —— 手感像调音台，
           而不是四个离散按钮；
         · 档位越高，轨道填充推进为「镭射粒子渐变」：光谱色带 +
           斜向粒子流 + 扫描高光，极限档拇指核心轻微呼吸；
         · 键盘可达（←/→ 逐档、Home/End 到底），`role="slider"` + `aria-valuetext`。
     纪律：只用主题 token（禁止 hex/rgb）、动效只用 --dur-*/--ease-* 令牌、
     圆角只用 --radius-*；`prefers-reduced-motion` 下全部动效关闭。 -->
<template>
  <div ref="anchorEl" class="effort-picker" data-testid="composer-effort-bar">
    <button
      type="button"
      class="effort-picker__trigger"
      :class="{ 'is-open': open, 'is-hot': meta.index >= 2, 'is-max': meta.index === 3 }"
      :aria-expanded="open"
      aria-haspopup="dialog"
      :aria-label="t('chat.effort.title')"
      :title="`${t('chat.effort.title')} · ${t(meta.labelKey)}`"
      @click="toggle"
    >
      <Icon name="zap" :size="14" aria-hidden="true" class="effort-picker__bolt" />
      <span class="effort-picker__label">{{ t(meta.labelKey) }}</span>
      <span class="effort-picker__meter" aria-hidden="true">
        <i
          v-for="(level, i) in EFFORT_LEVELS"
          :key="level"
          class="effort-picker__cell"
          :class="{ 'is-lit': i <= meta.index, 'is-max': i === 3 && meta.index === 3 }"
        />
      </span>
    </button>

    <Transition name="effort-pop">
      <div
        v-if="open"
        class="effort-pop"
        role="dialog"
        :aria-label="t('chat.effort.title')"
      >
        <div class="effort-pop__head">
          <span class="effort-pop__title">{{ t('chat.effort.title') }}</span>
          <span class="effort-pop__current">{{ t(meta.labelKey) }}</span>
        </div>

        <div
          ref="trackEl"
          class="effort-track"
          :class="{ 'is-dragging': dragging, 'is-max': meta.index === 3 }"
          @pointerdown="onPointerDown"
          @pointermove="onPointerMove"
          @pointerup="onPointerUp"
          @pointercancel="onPointerUp"
        >
          <div class="effort-track__rail" aria-hidden="true" />
          <div class="effort-track__fill" :style="{ width: `${pct}%` }" aria-hidden="true" />
          <div class="effort-track__ticks" aria-hidden="true">
            <span v-for="(level, i) in EFFORT_LEVELS" :key="level" :class="{ 'is-lit': i <= displayIndex }" />
          </div>
          <span v-if="dragging" class="effort-track__bubble" :style="{ left: `${pct}%` }" aria-hidden="true">
            {{ t(dragLabelKey) }}
          </span>
          <button
            type="button"
            class="effort-track__thumb"
            :style="{ left: `${pct}%` }"
            role="slider"
            :aria-valuemin="0"
            :aria-valuemax="EFFORT_LEVELS.length - 1"
            :aria-valuenow="meta.index"
            :aria-valuetext="t(meta.labelKey)"
            :aria-label="t('chat.effort.title')"
            :disabled="disabled"
            @keydown="onThumbKeydown"
            @pointerdown.stop="onThumbPointerDown"
          >
            <span class="effort-track__thumb-core" aria-hidden="true" />
          </button>
        </div>

        <div class="effort-scale">
          <button
            v-for="(level, i) in EFFORT_LEVELS"
            :key="level"
            type="button"
            class="effort-scale__item"
            :class="{ 'is-active': i === meta.index }"
            :disabled="disabled"
            :aria-pressed="i === meta.index"
            @click="pick(level)"
          >{{ t(EFFORT_META[level].labelKey) }}</button>
        </div>

        <p class="effort-pop__hint">{{ t(meta.hintKey) }}</p>
        <p v-if="routingOn" class="effort-pop__routing">
          <Icon name="router" :size="12" aria-hidden="true" />
          <span>{{ t('chat.effort.routingOn') }}</span>
        </p>
        <p v-if="appliesNextTurn" class="effort-pop__next">{{ t('chat.effort.appliesNextTurn') }}</p>
      </div>
    </Transition>
  </div>
</template>

<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import Icon from '@/components/Icon.vue'
import {
  EFFORT_LEVELS,
  EFFORT_META,
  effortFromIndex,
  effortIndex,
  type EffortLevel,
} from '@/types/effort'

const props = withDefaults(defineProps<{
  modelValue: EffortLevel
  /** 会话锁定/发送中：滑轨仍可读，但不可改（避免「点了不生效」的假反馈）。 */
  disabled?: boolean
  /** 「自动模型破甲」是否开启 —— 只做状态提示，不改变本轴语义。 */
  routingOn?: boolean
  /** 本轮已在执行：档位改动对下一轮生效，需显式告知。 */
  appliesNextTurn?: boolean
}>(), {
  disabled: false,
  routingOn: false,
  appliesNextTurn: false,
})

const emit = defineEmits<{
  'update:modelValue': [level: EffortLevel]
}>()

const { t } = useI18n()

const open = ref(false)
const anchorEl = ref<HTMLElement | null>(null)
const trackEl = ref<HTMLElement | null>(null)

/** 拖动中的连续档位（0..3 小数）；未拖动时为 null。 */
const dragValue = ref<number | null>(null)
const dragging = computed(() => dragValue.value !== null)

const meta = computed(() => EFFORT_META[props.modelValue] ?? EFFORT_META.off)
const displayIndex = computed(() => dragValue.value ?? meta.value.index)
const pct = computed(() => (displayIndex.value / (EFFORT_LEVELS.length - 1)) * 100)
/** 拖动气泡要显示"松手后会落到哪一档"，而不是当前已提交的档位。 */
const dragLabelKey = computed(
  () => EFFORT_META[effortFromIndex(displayIndex.value)]?.labelKey ?? meta.value.labelKey,
)

function toggle() {
  open.value = !open.value
}
function close() {
  open.value = false
}

/** 指针横坐标 → 连续档位（0..3），越界夹紧。 */
function valueFromClientX(clientX: number): number {
  const el = trackEl.value
  if (!el) return meta.value.index
  const rect = el.getBoundingClientRect()
  if (rect.width <= 0) return meta.value.index
  const ratio = (clientX - rect.left) / rect.width
  return Math.max(0, Math.min(1, ratio)) * (EFFORT_LEVELS.length - 1)
}

function onPointerDown(event: PointerEvent) {
  if (props.disabled) return
  // 点在滑轨任意位置即开始拖动（无需先精确按住拇指）。
  trackEl.value?.setPointerCapture?.(event.pointerId)
  dragValue.value = valueFromClientX(event.clientX)
  event.preventDefault()
}

function onThumbPointerDown(event: PointerEvent) {
  if (props.disabled) return
  trackEl.value?.setPointerCapture?.(event.pointerId)
  dragValue.value = displayIndex.value
  event.preventDefault()
}

function onPointerMove(event: PointerEvent) {
  if (dragValue.value === null) return
  dragValue.value = valueFromClientX(event.clientX)
}

function onPointerUp(event: PointerEvent) {
  if (dragValue.value === null) return
  const next = effortFromIndex(dragValue.value)
  dragValue.value = null
  trackEl.value?.releasePointerCapture?.(event.pointerId)
  if (next !== props.modelValue) emit('update:modelValue', next)
}

function pick(level: EffortLevel) {
  if (props.disabled || level === props.modelValue) return
  emit('update:modelValue', level)
}

function onThumbKeydown(event: KeyboardEvent) {
  if (props.disabled) return
  const i = effortIndex(props.modelValue)
  const last = EFFORT_LEVELS.length - 1
  let next: EffortLevel | null = null
  switch (event.key) {
    case 'ArrowLeft':
    case 'ArrowDown':
      next = effortFromIndex(i - 1)
      break
    case 'ArrowRight':
    case 'ArrowUp':
      next = effortFromIndex(i + 1)
      break
    case 'Home':
      next = EFFORT_LEVELS[0]
      break
    case 'End':
      next = EFFORT_LEVELS[last]
      break
    case 'Escape':
      close()
      return
    default:
      return
  }
  event.preventDefault()
  if (next && next !== props.modelValue) emit('update:modelValue', next)
}

function onDocPointerDown(event: PointerEvent) {
  if (!open.value) return
  const root = anchorEl.value
  if (root && !(event.target instanceof Node && root.contains(event.target))) close()
}
function onKeydown(event: KeyboardEvent) {
  if (event.key === 'Escape') close()
}

onMounted(() => {
  document.addEventListener('pointerdown', onDocPointerDown, true)
  document.addEventListener('keydown', onKeydown)
})
onBeforeUnmount(() => {
  document.removeEventListener('pointerdown', onDocPointerDown, true)
  document.removeEventListener('keydown', onKeydown)
})
</script>

<style scoped>
.effort-picker {
  position: relative;
  display: inline-flex;
  min-width: 0;
}

/* ── 触发器：输入框底栏里的胶囊 ─────────────────────────────── */
.effort-picker__trigger {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  min-height: 30px;
  padding: 3px 8px;
  border: 1px solid transparent;
  border-radius: var(--radius-pill);
  background: transparent;
  color: var(--text-muted);
  font: inherit;
  font-size: var(--fs-xs);
  font-weight: 600;
  cursor: pointer;
  transition:
    color var(--dur-fast) var(--ease-standard),
    background var(--dur-fast) var(--ease-standard),
    border-color var(--dur-fast) var(--ease-standard);
}
.effort-picker__trigger:hover,
.effort-picker__trigger:focus-visible,
.effort-picker__trigger.is-open {
  outline: 0;
  border-color: var(--border);
  background: var(--bg-hover);
  color: var(--text);
}
.effort-picker__trigger:focus-visible {
  box-shadow: var(--focus-ring);
}
.effort-picker__bolt {
  flex: 0 0 auto;
  color: var(--text-dim);
  transition: color var(--dur-fast) var(--ease-standard);
}
.effort-picker__trigger.is-hot .effort-picker__bolt {
  color: var(--accent);
}
.effort-picker__trigger.is-max .effort-picker__bolt {
  color: var(--warn);
}
.effort-picker__label {
  white-space: nowrap;
}

/* 四格电量条：一眼看出档位，不用展开面板。 */
.effort-picker__meter {
  display: inline-flex;
  align-items: flex-end;
  gap: 2px;
  height: 12px;
}
.effort-picker__cell {
  width: 3px;
  height: 5px;
  border-radius: var(--radius-xs);
  background: var(--border-strong);
  transition: background var(--dur-fast) var(--ease-standard);
}
.effort-picker__cell:nth-child(2) { height: 7px; }
.effort-picker__cell:nth-child(3) { height: 9px; }
.effort-picker__cell:nth-child(4) { height: 11px; }
.effort-picker__cell.is-lit {
  background: var(--accent);
}
.effort-picker__cell.is-max {
  background: var(--warn);
}

/* ── 弹出层 ─────────────────────────────────────────────── */
.effort-pop {
  position: absolute;
  z-index: 60;
  bottom: calc(100% + 0.5rem);
  /* 触发器在底栏右侧（发送键左边），浮层必须向左展开：
     之前 `left: 0` 会让 300px 的面板顶到窗口右缘、并被右侧栏压住一半。 */
  right: 0;
  width: min(300px, calc(100vw - 32px));
  display: flex;
  flex-direction: column;
  gap: 8px;
  padding: 10px 12px 9px;
  border: 1px solid var(--border);
  border-radius: var(--radius-lg);
  background: var(--bg-surface);
  box-shadow: var(--shadow-lg);
}
.effort-pop__head {
  display: flex;
  align-items: baseline;
  justify-content: space-between;
  gap: 8px;
}
.effort-pop__title {
  font-size: var(--fs-xs);
  font-weight: 600;
  color: var(--text);
}
.effort-pop__current {
  font-size: var(--fs-xs);
  font-weight: 600;
  color: var(--accent);
}

/* ── 滑轨：镭射粒子渐变 ───────────────────────────────────
   视觉语言：一条「光谱能量轨」—— 填充段是
   accent-deep → accent → accent-secondary → info → ok 的连续光谱，
   表面掠过一道扫描高光，内部是斜向粒子流；拇指为 conic 镭射核心 + 柔光。
   纪律：颜色只用主题 token + color-mix（换主题自动换色相，无 hex/rgb），
   动画只碰 transform/opacity 且为 linear 循环；reduced-motion 下全部静止。
   性能：粒子用 repeating-linear-gradient 单层伪元素，不新增 DOM、不跑 JS。 */
.effort-track {
  position: relative;
  height: 32px;
  display: flex;
  align-items: center;
  cursor: pointer;
  touch-action: none;
}
.effort-track__rail,
.effort-track__fill {
  position: absolute;
  left: 0;
  height: 5px;
  border-radius: var(--radius-pill);
}
.effort-track__rail {
  right: 0;
  overflow: hidden;
  background:
    linear-gradient(
      180deg,
      color-mix(in srgb, var(--bg) 55%, transparent) 0%,
      transparent 62%
    ),
    var(--bg-hover);
  box-shadow:
    inset 0 0 0 1px var(--border),
    inset 0 1px 2px color-mix(in srgb, var(--bg) 65%, transparent);
}
.effort-track__fill {
  overflow: hidden;
  background: linear-gradient(
    90deg,
    var(--accent-deep, var(--accent)) 0%,
    var(--accent) 30%,
    var(--accent-secondary) 58%,
    var(--info) 80%,
    var(--ok) 100%
  );
  background-size: 140% 100%;
  box-shadow: inset 0 1px 0 color-mix(in srgb, var(--bg-surface) 45%, transparent);
  transition: width var(--dur-fast) var(--ease-out);
}
/* 斜向粒子流：细密条纹在浅色高光下呈「镭射颗粒」质感 */
.effort-track__fill::after {
  content: '';
  position: absolute;
  inset: -60% -30%;
  background: repeating-linear-gradient(
    112deg,
    transparent 0 4px,
    color-mix(in srgb, var(--bg-surface) 78%, transparent) 4px 5px,
    transparent 5px 9px
  );
  opacity: 0.55;
  mix-blend-mode: screen;
  animation: effort-laser 2.6s linear infinite;
}
/* 扫描高光：每轮从左掠过一次，制造「能量在流动」 */
.effort-track__fill::before {
  content: '';
  position: absolute;
  inset: 0;
  background: linear-gradient(
    90deg,
    transparent 0%,
    color-mix(in srgb, var(--bg-surface) 55%, transparent) 45%,
    color-mix(in srgb, var(--bg-surface) 88%, transparent) 50%,
    color-mix(in srgb, var(--bg-surface) 55%, transparent) 55%,
    transparent 100%
  );
  transform: translateX(-100%);
  animation: effort-sheen 3.8s linear infinite;
}
.effort-track.is-dragging .effort-track__fill {
  transition: none;
}
.effort-track__ticks {
  position: absolute;
  left: 3px;
  right: 3px;
  display: flex;
  justify-content: space-between;
  pointer-events: none;
}
.effort-track__ticks span {
  width: 3px;
  height: 3px;
  border-radius: var(--radius-pill);
  background: var(--border-strong);
}
.effort-track__ticks span.is-lit {
  background: color-mix(in srgb, var(--bg-surface) 82%, var(--accent));
}
.effort-track__thumb {
  position: absolute;
  top: 50%;
  width: 16px;
  height: 16px;
  margin-left: -8px;
  transform: translateY(-50%);
  padding: 0;
  border: 1px solid color-mix(in srgb, var(--accent) 45%, var(--border-strong));
  border-radius: var(--radius-pill);
  background:
    radial-gradient(
      circle at 32% 28%,
      color-mix(in srgb, var(--bg-surface) 88%, var(--accent)) 0%,
      var(--bg-surface) 58%,
      var(--bg-surface-2) 100%
    );
  cursor: grab;
  box-shadow:
    0 0 0 1px color-mix(in srgb, var(--accent) 20%, transparent),
    0 0 10px color-mix(in srgb, var(--accent) 28%, transparent);
  transition:
    box-shadow var(--dur-fast) var(--ease-standard),
    border-color var(--dur-fast) var(--ease-standard),
    transform var(--dur-fast) var(--ease-spring);
}
.effort-track.is-dragging .effort-track__thumb {
  cursor: grabbing;
  transform: translateY(-50%) scale(1.08);
  box-shadow:
    0 0 0 2px color-mix(in srgb, var(--accent) 28%, transparent),
    0 0 18px color-mix(in srgb, var(--accent) 55%, transparent);
}
.effort-track__thumb:hover,
.effort-track__thumb:focus-visible {
  outline: 0;
  border-color: var(--accent);
  box-shadow: var(--focus-ring);
}
.effort-track__thumb:disabled {
  cursor: default;
  opacity: var(--state-disabled-opacity);
}
.effort-track__thumb-core {
  display: block;
  width: 8px;
  height: 8px;
  margin: 0 auto;
  border-radius: var(--radius-pill);
  background: conic-gradient(
    from 0deg,
    var(--accent),
    var(--accent-secondary),
    var(--info),
    var(--ok),
    var(--accent)
  );
  box-shadow: 0 0 6px color-mix(in srgb, var(--accent) 60%, transparent);
}
/* 极限档：核心轻微呼吸，提示「这条轨已经拉满」 */
.effort-track.is-max .effort-track__thumb-core {
  animation: effort-core-pulse 2.2s linear infinite;
}

@keyframes effort-laser {
  from { transform: translateX(0); }
  to { transform: translateX(9px); }
}
@keyframes effort-sheen {
  0%,
  55% { transform: translateX(-100%); }
  100% { transform: translateX(100%); }
}
@keyframes effort-core-pulse {
  0%,
  100% { opacity: 1; }
  50% { opacity: 0.68; }
}

/* 拖动时的档位气泡：跟手显示"松手落到哪一档"，比顶部那行小字好读 */
.effort-track__bubble {
  position: absolute;
  bottom: calc(100% - 2px);
  transform: translateX(-50%);
  padding: 2px 7px;
  border: 1px solid var(--border-strong);
  border-radius: var(--radius-pill);
  background: var(--bg-elevated, var(--bg-surface));
  color: var(--text);
  font-size: var(--fs-xs);
  font-variant-numeric: tabular-nums;
  white-space: nowrap;
  pointer-events: none;
  box-shadow: var(--shadow);
  z-index: 1;
}

/* ── 刻度与说明 ─────────────────────────────────────────── */
.effort-scale {
  display: grid;
  grid-template-columns: repeat(4, 1fr);
  gap: 4px;
}
.effort-scale__item {
  min-width: 0;
  padding: 4px 2px;
  border: 1px solid transparent;
  border-radius: var(--radius-sm);
  background: transparent;
  font: inherit;
  font-size: var(--fs-xs);
  color: var(--text-dim);
  text-align: center;
  cursor: pointer;
  transition:
    color var(--dur-fast) var(--ease-standard),
    background var(--dur-fast) var(--ease-standard),
    border-color var(--dur-fast) var(--ease-standard);
}
.effort-scale__item:hover,
.effort-scale__item:focus-visible {
  outline: 0;
  color: var(--text-muted);
  background: var(--bg-hover);
}
.effort-scale__item:focus-visible {
  box-shadow: var(--focus-ring);
}
.effort-scale__item:disabled {
  cursor: default;
}
.effort-scale__item.is-active {
  border-color: var(--border);
  background: var(--bg-hover);
  color: var(--text);
  font-weight: 600;
}
.effort-pop__hint {
  font-size: var(--fs-xs);
  line-height: 1.5;
  color: var(--text-muted);
}
.effort-pop__routing {
  display: flex;
  align-items: center;
  gap: 5px;
  font-size: var(--fs-xs);
  color: var(--accent);
}
.effort-pop__next {
  font-size: var(--fs-xs);
  color: var(--text-dim);
}

.effort-pop-enter-active,
.effort-pop-leave-active {
  transition:
    opacity var(--dur-fast) var(--ease-out),
    transform var(--dur-fast) var(--ease-out);
  transform-origin: bottom right;
}
.effort-pop-enter-from,
.effort-pop-leave-to {
  opacity: 0;
  transform: translateY(4px) scale(0.98);
}

@media (prefers-reduced-motion: reduce) {
  .effort-pop-enter-active,
  .effort-pop-leave-active,
  .effort-track__fill,
  .effort-track__thumb { transition: none; }
  /* 镭射粒子/扫描高光/极限呼吸：全部静止（保留渐变外观，不保留运动） */
  .effort-track__fill::before,
  .effort-track__fill::after,
  .effort-track.is-max .effort-track__thumb-core { animation: none; }
}
</style>
