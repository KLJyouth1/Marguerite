<!--
  电脑操控（computer_use）监管卡。

  一回合里每次 computer_use 调用都渲染成一行：动作 / 目标 / 成败 / 审批状态。
  字段来自后端 payload；缺失的字段直接不渲染对应徽章，绝不补默认值。
-->
<template>
  <section v-if="events.length" class="cua" data-testid="computer-use-card">
    <header class="cua__head">
      <Icon name="monitor" :size="13" aria-hidden="true" />
      <span class="cua__title">{{ t('chat.computer.title') }}</span>
      <span class="cua__count">{{ events.length }}</span>
    </header>

    <ul class="cua__list">
      <li
        v-for="(event, index) in events"
        :key="event.id || index"
        class="cua__row"
        data-testid="computer-use-row"
        :data-ok="event.ok === null ? 'unknown' : String(event.ok)"
        :data-approval="event.approval || undefined"
      >
        <span
          class="cua__dot"
          :class="event.ok === true ? 'is-ok' : event.ok === false ? 'is-fail' : 'is-unknown'"
          aria-hidden="true"
        />
        <span v-if="event.action" class="cua__action">{{ event.action }}</span>
        <code v-if="event.target" class="cua__target" :title="event.target">{{ event.target }}</code>
        <span
          v-if="event.ok !== null"
          class="cua__result"
          :class="event.ok ? 'is-ok' : 'is-fail'"
          role="img"
          :aria-label="event.ok ? t('chat.receipt.checkPassed') : t('chat.receipt.checkFailed')"
          :title="event.ok ? t('chat.receipt.checkPassed') : t('chat.receipt.checkFailed')"
        >
          <Icon :name="event.ok ? 'check' : 'x'" :size="12" aria-hidden="true" />
        </span>
        <span
          v-if="event.approval"
          class="cua__approval"
          :class="`cua__approval--${event.approval}`"
          data-testid="computer-use-approval"
        >
          {{ approvalLabel(event.approval) }}
        </span>
        <span v-if="event.note" class="cua__note">{{ event.note }}</span>
      </li>
    </ul>
  </section>
</template>

<script setup lang="ts">
import { useI18n } from 'vue-i18n'
import Icon from '@/components/Icon.vue'
import type { ComputerUseApproval, ComputerUseEvent } from '@/utils/chat/toolPayload'

defineProps<{
  /** 一回合内聚合后的 computer_use 事件；为空时整卡不渲染。 */
  events: ComputerUseEvent[]
}>()

const { t } = useI18n()

function approvalLabel(approval: ComputerUseApproval): string {
  return `${t('chat.computer.approval')} · ${t(`chat.computer.${approval}`)}`
}
</script>

<style scoped>
.cua {
  margin-top: 0.5rem;
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  background: var(--bg-elevated, var(--bg-surface));
  overflow: hidden;
  font-size: var(--fs-xs);
}

.cua__head {
  display: flex;
  align-items: center;
  gap: 0.375rem;
  padding: 0.375rem 0.5rem;
  border-bottom: 1px solid var(--hairline);
  color: var(--text-muted);
  font-weight: 600;
}

.cua__title {
  color: var(--text);
}

.cua__count {
  min-width: 1.25rem;
  padding: 0 0.25rem;
  border-radius: var(--radius-full);
  background: var(--bg-hover);
  color: var(--text-muted);
  font-family: var(--font-mono);
  font-size: 0.6875rem;
  font-variant-numeric: tabular-nums;
  text-align: center;
}

.cua__list {
  margin: 0;
  padding: 0;
  list-style: none;
}

.cua__row {
  display: flex;
  align-items: center;
  gap: 0.375rem;
  min-width: 0;
  padding: 0.3125rem 0.5rem;
}

.cua__row + .cua__row {
  border-top: 1px solid var(--hairline);
}

.cua__dot {
  flex: 0 0 auto;
  width: 0.375rem;
  height: 0.375rem;
  border-radius: var(--radius-full);
  background: var(--text-dim);
}

.cua__dot.is-ok { background: var(--ok); }
.cua__dot.is-fail { background: var(--danger); }
.cua__dot.is-unknown { background: var(--warn); }

.cua__action {
  flex: 0 0 auto;
  color: var(--text);
  font-weight: 500;
}

.cua__target {
  flex: 1 1 auto;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-family: var(--font-mono);
  font-size: 0.6875rem;
  color: var(--text-muted);
}

.cua__result {
  display: inline-flex;
  align-items: center;
  flex: 0 0 auto;
}

.cua__result.is-ok { color: var(--ok); }
.cua__result.is-fail { color: var(--danger); }

.cua__approval {
  flex: 0 0 auto;
  padding: 0 0.3125rem;
  border-radius: var(--radius-sm);
  background: var(--bg-hover);
  color: var(--text-muted);
  font-size: 0.625rem;
  font-weight: 600;
}

.cua__approval--approved {
  background: color-mix(in srgb, var(--ok) 16%, transparent);
  color: var(--ok);
}

.cua__approval--pending {
  background: color-mix(in srgb, var(--warn) 16%, transparent);
  color: var(--warn);
}

.cua__approval--denied {
  background: color-mix(in srgb, var(--danger) 14%, transparent);
  color: var(--danger);
}

.cua__note {
  flex: 1 1 100%;
  min-width: 0;
  color: var(--text-muted);
  line-height: 1.5;
  overflow-wrap: anywhere;
}
</style>
