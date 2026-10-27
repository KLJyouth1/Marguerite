<template>
  <section
    ref="rootRef"
    tabindex="-1"
    class="composer-run-mode"
    :class="{ 'is-inline': inline }"
    :role="inline ? undefined : 'dialog'"
    :aria-label="t('chat.composer.runMode')"
    @keydown.esc.stop="$emit('close')"
  >
    <div v-if="!inline" class="composer-run-mode__head">
      <span>{{ t('chat.composer.runMode') }}</span>
      <button type="button" class="composer-run-mode__close" :aria-label="t('chat.closeComposerSettings')" @click="$emit('close')">
        <Icon name="x" :size="14" />
      </button>
    </div>

    <div class="composer-run-mode__list" role="radiogroup" :aria-label="t('chat.composer.runMode')">
      <button
        v-for="option in runModeOptions"
        :key="option.value"
        type="button"
        class="composer-run-mode__option"
        :class="{ 'is-active': selectedRunMode === option.value }"
        :disabled="isDisabled(option.value)"
        :title="isDisabled(option.value) ? disabledReason : undefined"
        role="radio"
        :aria-checked="selectedRunMode === option.value ? 'true' : 'false'"
        @click="selectRunMode(option.value)"
      >
        <span class="composer-run-mode__option-main">
          <span class="composer-run-mode__option-label">{{ option.label }}</span>
          <Icon v-if="selectedRunMode === option.value" name="check" :size="14" />
        </span>
        <span class="composer-run-mode__option-desc">{{ option.description }}</span>
      </button>
    </div>

    <!-- 禁用必须给可见原因：原生 disabled 按钮不派发 click，没有这段文案就是
         「点了没反应」。安全模式本身不再因「setup 未就绪」被禁用。 -->
    <p
      v-if="disabledReason"
      class="composer-run-mode__hint"
      role="note"
    >{{ disabledReason }}</p>
  </section>
</template>

<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import Icon from '@/components/Icon.vue'
import type { SandboxRunMode, SandboxSetupState } from '@/types/sandbox'
import { SANDBOX_RUN_MODES, normalizeSandboxRunMode } from '@/types/sandbox'

const { t } = useI18n()

const props = withDefaults(defineProps<{
  runMode: SandboxRunMode
  allowedRunModes: SandboxRunMode[]
  safeSetupAvailable?: boolean
  /**
   * 沙箱 setup 的真实状态。未 ready 时 safe 仍可选（点击 → 安装/提示），
   * 只把标签改成「安全（待安装）」，不再禁用。
   */
  safeSetupState?: SandboxSetupState | null
  /** 会话进行中锁定：所有沙箱选项禁用，仅展示锁定提示。 */
  runModeLocked?: boolean
  runModeLockMessage?: string
  /**
   * 内联模式：被 `RunModePanel` 嵌进它自己的滚动面板时使用。
   * 绝对定位的浮层会在锚点 chip 上再弹一层、逃出父面板的 max-height（实测「模型路由」
   * 那一段内容被输入区吃掉），内联后它只是普通的一行排版。
   */
  inline?: boolean
}>(), {
  inline: false,
})

const emit = defineEmits<{
  close: []
  setRunMode: [mode: SandboxRunMode]
}>()

/** setup 未 ready（含未知）：safe 仍可选，但标签要老实标「待安装」。 */
const safeSetupPending = computed(
  () => props.safeSetupState != null && props.safeSetupState !== 'ready',
)

const runModeOptions = computed(() => [
  {
    value: 'safe',
    label: safeSetupPending.value
      ? t('settings.sandbox.safeMode.pendingLabel')
      : t('chat.composer.runModeSafe'),
    description: safeSetupPending.value
      ? t('settings.sandbox.safeMode.pendingComposerDesc')
      : t('chat.composer.runModeSafeDesc'),
  },
  {
    value: 'full',
    label: t('chat.composer.runModeFull'),
    description: t('chat.composer.runModeFullDesc'),
  },
] satisfies Array<{ value: SandboxRunMode; label: string; description: string }>)

const selectedRunMode = computed(() => normalizeSandboxRunMode(props.runMode))

const allowedRunModes = computed(() => {
  const allowed = props.allowedRunModes.filter(mode => SANDBOX_RUN_MODES.includes(mode))
  return allowed.length > 0 ? allowed : [...SANDBOX_RUN_MODES]
})

/**
 * 只允许两种禁用来源：会话运行锁（父级真实锁定）与网关策略不允许该模式。
 * 「setup 未就绪」不再是禁用条件 —— 点击会落盘选择并给出安装/提示。
 */
function isDisabled(mode: SandboxRunMode): boolean {
  if (props.runModeLocked) return true
  return !allowedRunModes.value.includes(mode)
}

const disabledReason = computed(() => {
  if (props.runModeLocked) return props.runModeLockMessage || t('chat.composer.runModeLocked')
  const missing = SANDBOX_RUN_MODES.some(mode => !allowedRunModes.value.includes(mode))
  return missing ? t('chat.composer.runModePolicyBlocked') : ''
})

function selectRunMode(mode: SandboxRunMode) {
  if (isDisabled(mode)) return
  emit('setRunMode', mode)
  emit('close')
}

const rootRef = ref<HTMLElement | null>(null)
onMounted(() => {
  // 内联形态不要抢焦点：它嵌在父面板的滚动盒里，focus() 会把滚动位置拉到
  // 这一段（「模型路由」在最后，等于一打开面板就被滚到底）。
  if (!props.inline) rootRef.value?.focus()
})
</script>

<style scoped>
.composer-run-mode {
  position: absolute;
  left: 0;
  bottom: calc(100% + 8px);
  width: min(340px, calc(100vw - 48px));
  padding: 0.75rem;
  border: 1px solid var(--border-strong);
  border-radius: var(--radius-md);
  background: var(--bg-surface);
  box-shadow: var(--shadow-xl);
  z-index: 30;
}

/* 内联：不再是一层浮窗，只是父面板里的一组选项 */
.composer-run-mode.is-inline {
  position: static;
  width: auto;
  padding: 0;
  border: 0;
  border-radius: 0;
  background: transparent;
  box-shadow: none;
  z-index: auto;
}

.composer-run-mode__head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 0.5rem;
  margin-bottom: 0.625rem;
  font-size: 0.8125rem;
  font-weight: 700;
  color: var(--text);
}

.composer-run-mode__close {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 26px;
  height: 26px;
  border: 1px solid transparent;
  border-radius: var(--radius-full);
  background: transparent;
  color: var(--text-muted);
  cursor: pointer;
}

.composer-run-mode__close:hover {
  background: var(--bg-hover);
  color: var(--text);
}

.composer-run-mode__list {
  display: grid;
  gap: 0.375rem;
}

.composer-run-mode__option {
  display: grid;
  gap: 0.25rem;
  width: 100%;
  min-height: 58px;
  padding: 0.625rem;
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  background: transparent;
  color: var(--text);
  text-align: left;
  cursor: pointer;
}

.composer-run-mode__option:hover {
  background: var(--bg-hover);
}

.composer-run-mode__option:focus-visible {
  outline: 2px solid color-mix(in srgb, var(--accent) 45%, transparent);
  outline-offset: 2px;
}

.composer-run-mode__option.is-active {
  border-color: color-mix(in srgb, var(--ok) 55%, var(--border));
  background: color-mix(in srgb, var(--ok) 10%, var(--bg-surface));
}

.composer-run-mode__option:disabled {
  cursor: not-allowed;
  opacity: 0.55;
}

.composer-run-mode__option:disabled:hover {
  background: transparent;
}

.composer-run-mode__option-main {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 0.5rem;
}

.composer-run-mode__option-label {
  font-size: 0.8125rem;
  font-weight: 700;
}

.composer-run-mode__option-desc {
  color: var(--text-muted);
  font-size: 0.75rem;
  line-height: 1.35;
  /* 说明文字允许折行：原先 nowrap + 省略号会把「需要沙箱」之外的限制条件吃掉，
     用户看到的是半句配置说明。 */
  white-space: normal;
}

.composer-run-mode__hint {
  margin: 0.5rem 0 0;
  padding: 6px 8px;
  border-radius: var(--radius-sm);
  background: color-mix(in srgb, var(--warn) 10%, transparent);
  color: var(--text-muted);
  font-size: 0.75rem;
  line-height: 1.4;
}

@media (max-width: 520px) {
  .composer-run-mode {
    left: -2.75rem;
    width: calc(100vw - 32px);
  }
}
</style>
