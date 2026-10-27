<!-- 对话框内的模式选择器（WorkBuddy / Codex 风格）。
     旧版把 3×3 网格常驻铺在输入框上方，占掉大块垂直空间；现在收敛为输入框底栏里
     的一枚小胶囊：`⚡ 标准·执行 ▾`，点开才在**上方**弹出模式面板 + 自动模型开关 + 高级入口。
     顶层只呈现「极简 / 标准 / 极致 × 咨询 / 方案 / 执行」9 档；旧 bloom / 全谱 / 单模型
     路由能力已由网关 `from_user_mode` 收编进「执行」列。 -->
<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import Icon from '@/components/Icon.vue'
import ModeMatrix from '@/components/chat/ModeMatrix.vue'
import { useActiveMode } from '@/composables/useActiveMode'
import type { ModelRoutingMode } from '@/types/modelRouting'
import { userModeLabel } from '@/utils/userModeText'

const props = withDefaults(defineProps<{
  /** 会话锁定 / 切换中时禁用整条选择器（避免「点了不生效」的假反馈）。 */
  runModeLocked?: boolean
  /** 当前模型路由档；非 `off` 视为「自动模型破甲」已开启。 */
  modelRoutingMode?: ModelRoutingMode
  /** 路由设置写盘进行中 → 禁用开关。 */
  modelRoutingBusy?: boolean
}>(), {
  runModeLocked: false,
  modelRoutingMode: 'omni_agent',
  modelRoutingBusy: false,
})

const emit = defineEmits<{
  openAdvanced: []
  setModelRoutingMode: [mode: ModelRoutingMode]
}>()

const { t } = useI18n()
const { activeUserMode } = useActiveMode()

const open = ref(false)
const anchorEl = ref<HTMLElement | null>(null)

// 标签走本地 i18n（mode.userMode.*）而不是直接信任载荷里的 `label`：
// 网关的 USER_MODE_META 只有一份中文文案，直接渲染会让英文界面漏中文。
const currentLabel = computed(() =>
  activeUserMode.value?.key ? userModeLabel(activeUserMode.value.key) : t('chat.presets.label'),
)
// 未加载到模式时 `currentLabel` 会退化成标题本身（「模式 / Mode」）→ 标题行右侧重复，故隐去。
const showCurrentInHead = computed(() => currentLabel.value !== t('chat.presets.label'))
const autoModelOn = computed(() => props.modelRoutingMode !== 'off')

/**
 * 上次生效的「非 off」破甲档位 —— 关闭再打开时**恢复它**，而不是硬写 `omni_agent`。
 *
 * 缺陷背景：旧实现 `autoModelOn ? 'off' : 'omni_agent'` 把 5 档路由（direct / router /
 * ensemble / bloom / omni）压成了二值。用户选了轻量的 `squilla_router` 后关一次破甲
 * 再打开，会被静默升级成最重的 `omni_agent` —— 成本与延迟都变了，界面却只显示「开」。
 * 这是「状态被开关吞掉」，属于静默失效，必须记住用户真实选择。
 */
const lastActiveRoutingMode = ref<ModelRoutingMode>(
  props.modelRoutingMode === 'off' ? 'omni_agent' : props.modelRoutingMode,
)
watch(
  () => props.modelRoutingMode,
  (mode) => {
    if (mode !== 'off') lastActiveRoutingMode.value = mode
  },
)

function toggle() {
  open.value = !open.value
}
function close() {
  open.value = false
}
function openAdvanced() {
  close()
  emit('openAdvanced')
}
function toggleAutoModel() {
  if (props.modelRoutingBusy) return
  emit('setModelRoutingMode', autoModelOn.value ? 'off' : lastActiveRoutingMode.value)
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

<template>
  <div
    ref="anchorEl"
    class="composer-mode-picker"
    :class="{ 'is-locked': runModeLocked }"
    data-testid="composer-mode-bar"
  >
    <button
      type="button"
      class="composer-mode-picker__trigger"
      :class="{ 'is-open': open }"
      :aria-expanded="open"
      :aria-disabled="runModeLocked ? 'true' : 'false'"
      aria-haspopup="dialog"
      :title="runModeLocked ? t('chat.presets.lockedHint') : t('chat.presets.triggerHint')"
      @click="toggle"
    >
      <Icon name="mode" :size="14" aria-hidden="true" />
      <span class="composer-mode-picker__label">{{ currentLabel }}</span>
      <Icon name="chevronDown" :size="11" aria-hidden="true" class="composer-mode-picker__caret" />
    </button>

    <Transition name="composer-mode-pop">
      <div
        v-if="open"
        class="composer-mode-pop"
        role="dialog"
        :aria-label="t('chat.presets.label')"
      >
        <div class="composer-mode-pop__head">
          <span class="composer-mode-pop__title">{{ t('chat.presets.label') }}</span>
          <span v-if="showCurrentInHead" class="composer-mode-pop__current">{{ currentLabel }}</span>
        </div>

        <ModeMatrix :disabled="runModeLocked" @picked="close" />

        <button
          type="button"
          class="composer-mode-pop__auto"
          :class="{ 'is-on': autoModelOn }"
          :aria-pressed="autoModelOn"
          :disabled="modelRoutingBusy"
          :title="t('chat.presets.autoModelHint')"
          @click="toggleAutoModel"
        >
          <Icon name="sparkles" :size="13" aria-hidden="true" />
          <span class="composer-mode-pop__auto-label">{{ t('chat.presets.autoModel') }}</span>
          <span class="composer-mode-pop__auto-state">
            {{ autoModelOn ? t('chat.presets.autoModelOn') : t('chat.presets.autoModelOff') }}
          </span>
        </button>

        <button
          type="button"
          class="composer-mode-bar__advanced"
          :title="t('chat.presets.advancedHint')"
          @click="openAdvanced"
        >
          <Icon name="settings" :size="13" aria-hidden="true" />
          <span>{{ t('chat.presets.advanced') }}</span>
          <Icon name="chevronDown" :size="11" aria-hidden="true" />
        </button>
      </div>
    </Transition>
  </div>
</template>

<style scoped>
.composer-mode-picker {
  position: relative;
  display: inline-flex;
  min-width: 0;
}
/* 锁定态只把触发器**文字**调暗，绝不在容器上加 `opacity`：
   ① `opacity < 1` 会新建层叠上下文，把弹出层的 `z-index` 困在内部，
      于是 `RunModePanel` 的锁提示（z-index: 50）照样盖住面板底部的「高级」按钮；
   ② 容器透明度会连同弹出层一起降到 50%，网格文字发灰发糊。
   锁定时仍可点开查看当前档位，禁用只体现在网格格子上（`aria-disabled` 表达）。 */
.composer-mode-picker.is-locked .composer-mode-picker__label {
  color: var(--text-dim);
}
.composer-mode-picker.is-locked .composer-mode-picker__caret {
  opacity: 0.35;
}
.composer-mode-picker__trigger {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  min-height: 30px;
  max-width: min(190px, 40vw);
  padding: 3px 8px;
  border: 1px solid transparent;
  border-radius: var(--radius-full);
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
.composer-mode-picker__trigger:hover,
.composer-mode-picker__trigger:focus-visible,
.composer-mode-picker__trigger.is-open {
  outline: 0;
  border-color: var(--border);
  background: var(--bg-hover);
  color: var(--text);
}
.composer-mode-picker__trigger:focus-visible {
  box-shadow: var(--focus-ring);
}
.composer-mode-picker__label {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.composer-mode-picker__caret {
  flex: 0 0 auto;
  opacity: 0.6;
}

.composer-mode-pop {
  position: absolute;
  /* 必须高于 `RunModePanel` 的锁提示（z-index: 50）——那条提示会在会话锁定时常驻
     悬在控件带上，否则它会盖住本面板底部的「高级」按钮，令其点不到。 */
  z-index: 60;
  bottom: calc(100% + 0.5rem);
  left: 0;
  width: min(320px, calc(100vw - 32px));
  display: flex;
  flex-direction: column;
  gap: 6px;
  padding: 8px;
  border: 1px solid var(--border);
  border-radius: var(--radius-lg);
  /* 不透明：此前用 `color-mix(… 96%, transparent)` + backdrop-blur，真机实测
     输入框的 placeholder 会透上来、网格文字发灰发糊（对比度不达标）。 */
  background: var(--bg-surface);
  box-shadow: var(--shadow-lg);
}
.composer-mode-pop__head {
  display: flex;
  align-items: baseline;
  justify-content: space-between;
  gap: 8px;
  padding: 0 2px 2px;
}
.composer-mode-pop__title {
  font-size: var(--fs-xs);
  font-weight: 600;
  color: var(--text);
}
.composer-mode-pop__current {
  font-size: var(--fs-xs);
  color: var(--text-dim);
}

.composer-mode-pop__auto,
.composer-mode-bar__advanced {
  display: flex;
  align-items: center;
  gap: 6px;
  width: 100%;
  min-height: 32px;
  padding: 5px 8px;
  border: 1px solid transparent;
  border-radius: var(--radius-control);
  background: transparent;
  color: var(--text-muted);
  font: inherit;
  font-size: var(--fs-xs);
  text-align: left;
  cursor: pointer;
  transition:
    background var(--dur-fast) var(--ease-standard),
    color var(--dur-fast) var(--ease-standard);
}
.composer-mode-pop__auto:hover,
.composer-mode-pop__auto:focus-visible,
.composer-mode-bar__advanced:hover,
.composer-mode-bar__advanced:focus-visible {
  outline: 0;
  background: var(--bg-hover);
  color: var(--text);
}
.composer-mode-pop__auto:focus-visible,
.composer-mode-bar__advanced:focus-visible {
  box-shadow: var(--focus-ring);
}
.composer-mode-pop__auto:disabled {
  cursor: default;
  opacity: var(--state-disabled-opacity);
}
.composer-mode-pop__auto-label {
  flex: 1 1 auto;
  min-width: 0;
}
.composer-mode-pop__auto-state {
  flex: 0 0 auto;
  color: var(--text-dim);
  font-weight: 600;
}
.composer-mode-pop__auto.is-on {
  color: var(--accent);
  background: color-mix(in srgb, var(--accent) 10%, transparent);
}
.composer-mode-pop__auto.is-on .composer-mode-pop__auto-state {
  color: var(--accent);
}
.composer-mode-bar__advanced {
  border-top: 1px solid var(--border);
  border-radius: 0 0 var(--radius-control) var(--radius-control);
  color: var(--text-dim);
}

.composer-mode-pop-enter-active,
.composer-mode-pop-leave-active {
  transition:
    opacity var(--dur-fast) var(--ease-out),
    transform var(--dur-fast) var(--ease-out);
  transform-origin: bottom left;
}
.composer-mode-pop-enter-from,
.composer-mode-pop-leave-to {
  opacity: 0;
  transform: translateY(4px) scale(0.98);
}
@media (prefers-reduced-motion: reduce) {
  .composer-mode-pop-enter-active,
  .composer-mode-pop-leave-active {
    transition: none;
  }
}
</style>
