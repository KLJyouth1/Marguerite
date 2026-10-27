<!-- P2·E 编排活动状态条：输入框上方的实时滚动条，展示编排层子代理/团队事件的
     最近滚动（订阅 `agent.started / agent.stopped`，由父级 useOrchestrationActivity 提供）。
     仅在存在编排活动时渲染；无事件时零占用。 -->
<script setup lang="ts">
import { computed } from 'vue'
import Icon from '@/components/Icon.vue'
import { useI18n } from 'vue-i18n'
import type { OrchestrationActivityItem } from '@/composables/chat/useOrchestrationActivity'

const props = defineProps<{
  items: OrchestrationActivityItem[]
}>()

const { t } = useI18n()

function formLabel(form: string | undefined): string {
  if (!form) return ''
  const key = `chat.orchestration.form${form[0].toUpperCase()}${form.slice(1)}`
  return t(key)
}

function itemLabel(item: OrchestrationActivityItem): string {
  const form = typeof item.payload?.form === 'string' ? item.payload.form : undefined
  if (item.kind === 'started') {
    const base = t('chat.orchestration.eventStarted')
    return form ? `${base} · ${formLabel(form)}` : base
  }
  if (item.kind === 'stopped') {
    const ok = item.payload?.ok === true
    const base = ok ? t('chat.orchestration.eventStopped') : t('chat.orchestration.eventFailed')
    return form ? `${base} · ${formLabel(form)}` : base
  }
  return item.event
}

const shown = computed(() => props.items.slice(0, 4))
</script>

<template>
  <div
    v-if="shown.length > 0"
    class="orch-status"
    data-testid="chat-orchestration-status"
    role="status"
    aria-live="polite"
  >
    <div
      v-for="item in shown"
      :key="item.id"
      class="orch-status__item"
      :class="[`orch-status__item--${item.kind}`]"
    >
      <Icon
        v-if="item.kind === 'started'"
        name="play"
        :size="11"
        class="orch-status__ic"
        aria-hidden="true"
      />
      <Icon
        v-else-if="item.kind === 'stopped' && item.payload?.ok === true"
        name="check"
        :size="11"
        class="orch-status__ic"
        aria-hidden="true"
      />
      <Icon
        v-else-if="item.kind === 'stopped'"
        name="alertTriangle"
        :size="11"
        class="orch-status__ic"
        aria-hidden="true"
      />
      <span v-else class="orch-status__dot" aria-hidden="true" />
      <span class="orch-status__event">{{ itemLabel(item) }}</span>
      <span class="orch-status__ts">{{ item.ts }}</span>
    </div>
  </div>
</template>

<style scoped>
.orch-status {
  display: flex;
  flex-direction: column;
  gap: 3px;
  width: min(100%, var(--composer-col, 820px));
  margin: 0 auto 6px;
  padding: 6px 10px;
  border: 1px solid color-mix(in srgb, var(--accent) 30%, var(--border));
  border-radius: var(--radius-md);
  background: color-mix(in srgb, var(--accent) 7%, var(--bg-surface));
  box-shadow: var(--shadow-sm);
}
.orch-status__item {
  display: flex;
  align-items: center;
  gap: 6px;
  font-size: var(--fs-xs);
  color: var(--text-muted);
  line-height: 1.4;
}
.orch-status__item--started {
  color: var(--text);
}
.orch-status__item--stopped {
  color: var(--text-dim);
}
.orch-status__ic {
  color: var(--accent);
  flex-shrink: 0;
}
.orch-status__item--stopped .orch-status__ic {
  color: var(--ok, var(--success, var(--accent)));
}
.orch-status__dot {
  width: 5px;
  height: 5px;
  border-radius: 50%;
  background: var(--text-dim);
  flex-shrink: 0;
}
.orch-status__event {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.orch-status__ts {
  margin-left: auto;
  color: var(--text-dim);
  flex-shrink: 0;
}
</style>