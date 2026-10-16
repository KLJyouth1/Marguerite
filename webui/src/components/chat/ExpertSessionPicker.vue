<script setup lang="ts">
// P0-2 专家可见性：会话头部「新专家会话」入口。
//
// 与 AgentPersonaBadge（展示当前人格、点击跳 /agents）互补：徽章回答「当前在和谁
// 对话」，本组件提供一个**开启新专家会话**的显式入口。数据源复用全局单例
// useAgentOptions（App.vue 已全局加载 agents.list），此处**只读**不触发新请求，
// 避免绕过 chat bootstrap admission 的 RPC 门控。
//
// 有意边界：**不提供会话中途换人格**——历史消息的人格语境会错乱；换专家 = 开新会话，
// 新会话键为 `agent:{agentId}:webchat:{suffix}`（与 AgentsView 的入口同构）。
import { computed, ref } from 'vue'
import { useRouter } from 'vue-router'
import { useI18n } from 'vue-i18n'
import Icon from '@/components/Icon.vue'
import { useAgentOptions } from '@/composables/useAgentOptions'
import { useDialogLayer } from '@/composables/useDialogA11y'
import { useDocumentEvent } from '@/composables/useDocumentEvent'
import { newWebchatSessionKey } from '@/utils/chat/sessionKeys'

const { t } = useI18n()
const router = useRouter()
const { agents } = useAgentOptions()

const open = ref(false)
const rootRef = ref<HTMLElement | null>(null)
const triggerRef = ref<HTMLButtonElement | null>(null)
useDialogLayer(computed(() => open.value))

const label = computed(() => t('chat.persona.newSession'))

function toggle() {
  open.value = !open.value
}

function close(restoreFocus = false) {
  if (restoreFocus) triggerRef.value?.focus()
  open.value = false
}

/** 开启一个新专家会话：以 `agent:{agentId}:webchat:{suffix}` 键切到该会话。 */
function startSession(agentId: string) {
  const session = newWebchatSessionKey(agentId)
  close()
  void router.push({ path: '/chat', query: { session } })
}

useDocumentEvent('click', (event) => {
  if (!open.value) return
  if (event.target instanceof Node && !rootRef.value?.contains(event.target)) {
    open.value = false
  }
})

useDocumentEvent('keydown', (event) => {
  if (event.key === 'Escape' && open.value) {
    event.preventDefault()
    close(true)
  }
})
</script>

<template>
  <div ref="rootRef" class="expert-picker">
    <button
      ref="triggerRef"
      type="button"
      class="expert-picker__trigger"
      :class="{ 'is-open': open }"
      :title="label"
      :aria-label="label"
      aria-haspopup="menu"
      :aria-expanded="open"
      aria-controls="expert-session-menu"
      data-testid="expert-session-trigger"
      @click.stop="toggle"
    >
      <Icon name="agents" :size="14" />
      <span class="expert-picker__label">{{ label }}</span>
    </button>
    <div
      v-if="open"
      id="expert-session-menu"
      class="expert-picker__menu"
      role="menu"
      :aria-label="label"
      data-testid="expert-session-menu"
    >
      <button
        v-for="agent in agents"
        :key="agent.id"
        type="button"
        class="expert-picker__item"
        role="menuitem"
        :data-testid="`expert-session-${agent.id}`"
        @click="startSession(agent.id)"
      >
        <Icon name="agents" :size="14" />
        <span class="expert-picker__item-label">{{ agent.name }}</span>
      </button>
    </div>
  </div>
</template>

<style scoped>
.expert-picker {
  flex: 0 0 auto;
  position: relative;
}

.expert-picker__trigger {
  align-items: center;
  background: var(--bg-surface);
  border: 1px solid var(--border);
  border-radius: var(--radius-full);
  color: var(--text-muted);
  cursor: pointer;
  display: inline-flex;
  font: inherit;
  font-size: 0.8125rem;
  gap: var(--sp-1);
  justify-content: center;
  min-height: 30px;
  padding: 0.25rem 0.625rem;
  white-space: nowrap;
}

.expert-picker__trigger:hover,
.expert-picker__trigger:focus-visible,
.expert-picker__trigger.is-open {
  background: var(--bg-hover);
  border-color: var(--border-strong);
  color: var(--text);
}

.expert-picker__label {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.expert-picker__menu {
  background: var(--bg-elevated);
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  box-shadow: var(--shadow-lg);
  display: flex;
  flex-direction: column;
  gap: 1px;
  max-height: 320px;
  max-width: calc(100vw - (2 * var(--sp-3)));
  min-width: min(220px, calc(100vw - (2 * var(--sp-3))));
  overflow-y: auto;
  padding: var(--sp-1);
  position: absolute;
  right: 0;
  top: calc(100% + 6px);
  z-index: 80;
}

.expert-picker__item {
  align-items: center;
  background: none;
  border: 0;
  border-radius: var(--radius-sm);
  color: var(--text-muted);
  cursor: pointer;
  display: flex;
  font: inherit;
  font-size: var(--fs-sm);
  gap: var(--sp-2);
  min-height: 40px;
  padding: 8px 10px;
  text-align: left;
  width: 100%;
}

.expert-picker__item:hover,
.expert-picker__item:focus-visible {
  background: var(--bg-hover);
  color: var(--text);
  outline: none;
}

.expert-picker__item-label {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
</style>
