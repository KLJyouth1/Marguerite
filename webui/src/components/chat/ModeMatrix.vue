<script setup lang="ts">
// 3×3 模式网格（**面板形态**，对齐 WorkBuddy / Codex 的克制布局）。
//
// 与旧版的区别：旧版把这张网格**常驻**渲染在输入框上方，占了近 150px 垂直空间，
// 且与顶栏 ModeBadge、ModeHintBar 形成三套并列的模式控件。现在网格默认收起，
// 只在用户点开 `ChatComposerPresets` 的触发器时出现在弹出层里 —— 输入框恢复成
// 一行文字 + 一行小控件。
//
// 选择语义不变：点选 → `switchUserMode(key)` 下发 `{ userMode }`，由网关
// `from_user_mode` 派生真正的执行档（旧 bloom / 全谱能力收编进「执行」列）。
//
// 与顶栏 ModeBadge 的关系（P0-2 收敛后）：两者是**同一个九档词表的两个入口**，
// 共用 `activeUserMode` 单一状态源与 `switchUserMode` 同一写入路径；徽标菜单只是
// 这张网格的下拉形态。文案全部走 i18n（mode.userMode.* / mode.intensity.* /
// mode.working.*），不再硬编码中文 —— 英文界面曾整块漏中文。
import { computed } from 'vue'
import { useActiveMode } from '@/composables/useActiveMode'
import { useI18n } from 'vue-i18n'
import {
  USER_MODE_META,
  USER_MODE_INTENSITY,
  USER_MODE_WORKING,
  type UserModeInfo,
} from '@/stores/activeMode'
import {
  userModeHint,
  userModeIntensityLabel,
  userModeWorkingLabel,
} from '@/utils/userModeText'

const props = defineProps<{
  /** 会话锁定 / 外部忙碌时禁用整格（与内部 switching 取或）。 */
  disabled?: boolean
}>()

const emit = defineEmits<{
  /** 用户完成一次选择（含「点了当前项」）→ 父级据此收起弹出层。 */
  picked: []
}>()

const { t } = useI18n()
const { activeUserMode, switching, switchError, switchUserMode } = useActiveMode()

const busy = computed(() => props.disabled === true || switching.value)

const currentKey = computed(() => activeUserMode.value?.key ?? '')

function cell(intensity: string, working: string): UserModeInfo | null {
  const info = USER_MODE_META[`${intensity}_${working}`] as UserModeInfo | undefined
  return info ?? null
}

async function pick(key: string) {
  if (key && key !== currentKey.value) await switchUserMode(key)
  emit('picked')
}
</script>

<template>
  <div class="mode-matrix" role="grid" :aria-label="t('chrome.modeMatrix')">
    <div class="mode-matrix__corner" aria-hidden="true" />
    <div v-for="w in USER_MODE_WORKING" :key="w" class="mode-matrix__colhead">{{ userModeWorkingLabel(w) }}</div>

    <template v-for="i in USER_MODE_INTENSITY" :key="i">
      <div class="mode-matrix__rowhead">{{ userModeIntensityLabel(i) }}</div>
      <button
        v-for="w in USER_MODE_WORKING"
        :key="`${i}_${w}`"
        type="button"
        role="gridcell"
        class="mode-matrix__cell"
        :class="[
          `mode-matrix__cell--${w}`,
          { 'is-active': currentKey === `${i}_${w}` },
        ]"
        :aria-pressed="currentKey === `${i}_${w}`"
        :disabled="busy"
        :title="userModeHint(`${i}_${w}`)"
        @click="pick(cell(i, w)?.key ?? '')"
      >
        <span class="mode-matrix__cell-label">{{ userModeIntensityLabel(i) }} · {{ userModeWorkingLabel(w) }}</span>
        <span class="mode-matrix__cell-desc">{{ userModeHint(`${i}_${w}`) }}</span>
      </button>
    </template>

    <p v-if="switchError" class="mode-matrix__error">{{ t('chrome.modeSwitchFailed') }} {{ switchError }}</p>
  </div>
</template>

<style scoped>
.mode-matrix {
  display: grid;
  grid-template-columns: 34px repeat(3, 1fr);
  gap: 4px;
  width: 100%;
}
.mode-matrix__colhead,
.mode-matrix__rowhead {
  display: flex;
  align-items: center;
  justify-content: center;
  font-size: var(--fs-xs);
  font-weight: 500;
  color: var(--text-muted);
}
.mode-matrix__rowhead {
  justify-content: flex-start;
}
.mode-matrix__cell {
  display: flex;
  flex-direction: column;
  gap: 1px;
  padding: 5px 6px;
  min-height: 38px;
  border-radius: var(--radius-sm);
  border: 1px solid var(--border);
  background: var(--bg-surface);
  color: var(--text);
  cursor: pointer;
  text-align: left;
  transition: border-color var(--dur-fast) var(--ease-standard), background var(--dur-fast) var(--ease-standard);
}
.mode-matrix__cell:hover:not(:disabled) {
  border-color: var(--border-strong);
}
.mode-matrix__cell:disabled {
  opacity: var(--state-disabled-opacity);
  cursor: progress;
}
.mode-matrix__cell--doit {
  border-color: color-mix(in srgb, var(--warn) 40%, transparent);
}
.mode-matrix__cell.is-active {
  border-color: var(--accent);
  background: color-mix(in srgb, var(--accent) 16%, transparent);
  box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--accent) 50%, transparent);
}
.mode-matrix__cell--doit.is-active {
  border-color: var(--warn);
  background: color-mix(in srgb, var(--warn) 18%, transparent);
  box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--warn) 55%, transparent);
}
.mode-matrix__cell-label {
  font-size: var(--fs-xs);
  font-weight: 500;
}
.mode-matrix__cell-desc {
  font-size: 10px;
  color: var(--text-muted);
}
.mode-matrix__error {
  grid-column: 1 / -1;
  font-size: 10px;
  line-height: 1.5;
  color: var(--danger);
  margin: 2px 0 0;
}
</style>
