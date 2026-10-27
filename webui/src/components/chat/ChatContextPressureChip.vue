<!-- 上下文压力告警 chip。
     为什么需要它：网关的 `usage.status` 早已返回 `contextStatus`（contextTokens /
     pressure / warningRatio），`useChatUsageWidget` 也早已算出 `contextWarning`，
     但**没有任何组件消费它** —— 数据链完整、界面缺失，用户只能在压缩发生后
     才知道上下文满了。这里把它接到对话页顶栏：越过网关阈值（默认 0.85）才出现，
     点击直接触发手动压缩（与顶栏「压缩上下文」同一动作）。

     判据纪律：只在 `contextWarning` 非空时渲染；不猜测、不在数据缺失时占位。 -->
<template>
  <button
    v-if="warning"
    type="button"
    class="ctx-pressure"
    :class="{ 'ctx-pressure--critical': warning.pct >= 95 }"
    data-testid="context-pressure-chip"
    :title="detail"
    :aria-label="detail"
    @click="emit('compact')"
  >
    <Icon name="gauge" :size="13" />
    <span class="ctx-pressure__label">{{ t('chat.contextPressure', { pct: warning.pct }) }}</span>
  </button>
</template>

<script setup lang="ts">
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'
import Icon from '@/components/Icon.vue'
import type { ContextWarning } from '@/composables/chat/useChatUsageWidget'

const props = defineProps<{ warning: ContextWarning | null }>()
const emit = defineEmits<{ compact: [] }>()

const { t } = useI18n()

const detail = computed(() => {
  const w = props.warning
  if (!w) return ''
  // 复用既有文案键（`chat.contextPressureTitle` 早已存在但此前无人消费）。
  return t('chat.contextPressureTitle', {
    used: w.usedK,
    window: w.windowK,
  })
})
</script>

<style scoped>
.ctx-pressure {
  display: inline-flex;
  align-items: center;
  gap: 0.25rem;
  min-height: 26px;
  padding: 0 0.5rem;
  border: 1px solid color-mix(in srgb, var(--warn) 45%, var(--border));
  border-radius: var(--radius-pill);
  background: color-mix(in srgb, var(--warn) 12%, transparent);
  color: var(--text);
  cursor: pointer;
  font: inherit;
  font-size: var(--fs-xs);
  font-variant-numeric: tabular-nums;
}

.ctx-pressure:hover {
  background: color-mix(in srgb, var(--warn) 20%, transparent);
}

.ctx-pressure--critical {
  border-color: color-mix(in srgb, var(--danger) 55%, var(--border));
  background: color-mix(in srgb, var(--danger) 14%, transparent);
}

.ctx-pressure--critical:hover {
  background: color-mix(in srgb, var(--danger) 22%, transparent);
}

.ctx-pressure__label {
  line-height: 1;
  white-space: nowrap;
}
</style>
