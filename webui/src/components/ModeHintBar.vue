<script setup lang="ts">
// ModeHintBar — upgrade-masterplan §6 阶段 D（模式能力→UI 映射）
//
// 只读模式能力提示条（v2.1 收敛）：本组件不提供任何切换按钮。
// 模式切换的两个入口（顶栏 ModeBadge 下拉、输入框上方 3×3 ModeMatrix）在 P0-2 之后
// 已收敛到同一个九档词表与同一个状态源，因此这里不再强调「唯一入口」这类说法。
// 本组件刻意继续读取**运行时 kind**（网关 from_user_mode 派生出的执行档），
// 因为它描述的是引擎此刻真实在跑的能力（如蜂群并行开关），与用户所选格位是两件事。
//
// 仍读取共享 `activeMode` 单例（由 useActiveMode 驱动），切换即时刷新。
// v2.2: 整合 useGatewayReadiness，在「已连接但未就绪」时显示配置提示，
// 而非笼统的"网关正在加载运行时配置"——用户能直接看出需要配 API Key。

import { computed } from 'vue'
import Icon from '@/components/Icon.vue'
import { useActiveMode } from '@/composables/useActiveMode'
import { useGatewayReadiness } from '@/composables/useGatewayReadiness'
import { normalizeModeKind } from '@/stores/activeMode'
import { useI18n } from 'vue-i18n'

const { t } = useI18n()
const { activeMode } = useActiveMode()
const { readiness } = useGatewayReadiness()

// D-03：模式名/提示走 i18n 别名键（zh-Hans 中文别名，en 技术名），
// 不再硬编码英文 label + 中文 hint（此前绕过 i18n，英文界面会漏中文）。
const MODE_KINDS = ['plan', 'agent', 'ensemble', 'bloom', 'omni-transcend'] as const

const currentKind = computed(() =>
  normalizeModeKind(activeMode.value?.kind ?? activeMode.value?.mode),
)
const currentLabel = computed(() => {
  if (readiness.value === 'not_ready') return t('modeHintBar.setupNeeded')
  if (currentKind.value && (MODE_KINDS as readonly string[]).includes(currentKind.value)) {
    return t(`mode.alias.${currentKind.value}`)
  }
  return currentKind.value || t('modeHintBar.initializing')
})
const currentHint = computed(() => {
  if (readiness.value === 'not_ready') return t('modeHintBar.setupNeededHint')
  if (readiness.value === 'disconnected' || readiness.value === 'connecting') {
    return t('modeHintBar.gatewayLoading')
  }
  if (currentKind.value && (MODE_KINDS as readonly string[]).includes(currentKind.value)) {
    return t(`mode.hint.${currentKind.value}`)
  }
  return t('modeHintBar.gatewayLoading')
})

// P1-1：`bloom.parallel_enabled` 默认 false（蜂群并行是 Bloom 模式的卖点，但并非默认开启）。
// 处于非 Bloom 模式时如实标注「蜂群并行当前关闭」，避免 UI 名不副实。
const showBloomParallelOff = computed(
  () => !!currentKind.value && currentKind.value !== 'bloom',
)

// 收敛（2026-10-13）：这条提示条此前**常驻**渲染成整行带底色的色块，在正常运行时
// 纯属噪声（WorkBuddy / Codex 都没有这样一条常驻横幅）。现在默认是安静的一行灰字，
// 只有网关未就绪 / 未连接这类**需要用户动作**的状态才恢复醒目底色。
const needsAttention = computed(() => readiness.value !== 'ready')
</script>

<template>
  <div
    class="mode-hint-bar"
    :class="{ 'mode-hint-bar--attention': needsAttention }"
    role="status"
    aria-live="polite"
  >
    <Icon name="mode" :size="15" class="mode-hint-bar__icon" />
    <span class="mode-hint-bar__label">{{ currentLabel }}</span>
    <span class="mode-hint-bar__sep" aria-hidden="true">·</span>
    <span class="mode-hint-bar__hint">{{ currentHint }}</span>
    <span v-if="showBloomParallelOff" class="mode-hint-bar__note">{{ t('modeHintBar.bloomParallelOff') }}</span>
  </div>
</template>

<style scoped>
/* 安静态：一行灰字，不占块、不抢视觉（对齐 WorkBuddy / Codex 的状态行克制感）。 */
.mode-hint-bar {
  display: flex;
  align-items: center;
  gap: 6px;
  flex-wrap: wrap;
  padding: 2px 2px;
  margin: 0 0 4px;
  font-size: var(--fs-xs);
  line-height: 1.4;
  color: var(--text-dim);
}
/* 需要用户动作时（未就绪 / 未连接）才恢复醒目底色。 */
.mode-hint-bar--attention {
  gap: 8px;
  padding: 6px 12px;
  border-radius: var(--radius-md);
  background: color-mix(in srgb, var(--text) 8%, transparent);
  border: 1px solid color-mix(in srgb, var(--text) 18%, transparent);
  font-size: 12px;
}
.mode-hint-bar__icon {
  color: var(--accent);
  flex: none;
  opacity: 0.8;
}
.mode-hint-bar__label {
  font-weight: 600;
  color: var(--text-muted);
}
.mode-hint-bar--attention .mode-hint-bar__label {
  color: var(--text);
}
.mode-hint-bar__sep {
  opacity: 0.5;
}
.mode-hint-bar__hint {
  flex: 1 1 auto;
  min-width: 120px;
}
.mode-hint-bar__note {
  color: var(--text-dim);
  opacity: 0.75;
  white-space: nowrap;
}
</style>
