<template>
  <!-- 统一空状态：图标 + 标题 + 说明 + 可选主操作。
       抽这一个组件的原因：真机上同一个「没有内容」在四个页面呈现四种样子
       （mcp 拼图+CTA、plugins 一行灰字、skills 大图标+文案、sessions 卡片），
       用户读到的暗示完全不同。样式以 mcp 页那版为基准（审计里唯一被点名「做得最好」的）。 -->
  <div class="empty-panel" :data-testid="testId">
    <Icon v-if="icon" :name="icon" :size="iconSize" class="empty-panel__icon" aria-hidden="true" />
    <div v-if="title || description || $slots.default" class="empty-panel__copy">
      <p v-if="title" class="empty-panel__title">{{ title }}</p>
      <p v-if="description" class="empty-panel__hint">{{ description }}</p>
      <slot />
    </div>
    <button
      v-if="actionLabel"
      type="button"
      class="btn btn--sm btn--primary"
      :disabled="actionDisabled"
      @click="emit('action')"
    >{{ actionLabel }}</button>
  </div>
</template>

<script setup lang="ts">
import Icon from '@/components/Icon.vue'
import type { IconName } from '@/utils/icons'

withDefaults(defineProps<{
  /** Icon 名称（`Icon.vue` 的注册名） */
  icon?: IconName
  /** 图标尺寸；默认与审计基准（mcp 空状态）一致 */
  iconSize?: number
  title?: string
  description?: string
  /** 有主操作时给出按钮文案；点击只发 `action` 事件，不内嵌业务逻辑 */
  actionLabel?: string
  actionDisabled?: boolean
  testId?: string
}>(), {
  icon: undefined,
  iconSize: 28,
  title: undefined,
  description: undefined,
  actionLabel: undefined,
  actionDisabled: false,
  testId: undefined,
})

const emit = defineEmits<{ action: [] }>()
</script>

<style scoped>
.empty-panel {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: var(--sp-3);
  padding: 40px var(--sp-4);
  text-align: center;
  color: var(--text-muted);
}

.empty-panel__icon {
  color: var(--text-dim);
}

.empty-panel__copy {
  display: flex;
  flex-direction: column;
  gap: var(--sp-1);
  max-width: 640px;
}

.empty-panel__title {
  margin: 0;
  font-size: var(--fs-sm);
  font-weight: 600;
  color: var(--text);
}

.empty-panel__hint {
  margin: 0;
  font-size: var(--fs-xs);
  line-height: 1.6;
  color: var(--text-muted);
}
</style>
