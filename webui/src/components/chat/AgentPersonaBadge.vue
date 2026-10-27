<script setup lang="ts">
// P0-2「当前在和谁对话」：会话命中智能体人格（agent:{agentId}:...）时，在会话头部
// 显示当前身份徽章；非智能体会话不渲染。点击跳 /agents（既有路由，
// 不新增选择器，避免与 /agents 页双头管理人格）。
//
// 2026-10-18：主智能体（`main`）此前也被套上「当前专家」——它不是专家，是
// 默认应答者（严工）。文案按 `agentId === 'main'` 分叉到 `chat.persona.currentMain`，
// 与 Expert Center 的「主智能体」口径一致。
//
// 数据来源：router_decision 载荷的 persona，经 useChatRouteFeedback 的共享
// activePersona 单例下发（同一事件管道内读写，不会分叉）。
import { computed } from 'vue'
import { useRouter } from 'vue-router'
import { useI18n } from 'vue-i18n'
import Icon from '@/components/Icon.vue'
import { useChatRouteFeedback } from '@/composables/chat/useChatRouteFeedback'

const { t } = useI18n()
const router = useRouter()
const { activePersona } = useChatRouteFeedback()

const label = computed(() => {
  const persona = activePersona.value
  if (!persona) return ''
  const key = persona.agentId === 'main' ? 'chat.persona.currentMain' : 'chat.persona.current'
  return t(key, { name: persona.name })
})
const tooltip = computed(() => {
  const persona = activePersona.value
  if (!persona) return ''
  return [persona.description, persona.agentId].filter(Boolean).join(' · ')
})

function openAgents() {
  void router.push('/agents')
}
</script>

<template>
  <button
    v-if="activePersona"
    type="button"
    class="agent-persona-badge"
    :title="tooltip"
    :aria-label="label"
    data-testid="agent-persona-badge"
    @click="openAgents"
  >
    <Icon name="agents" :size="13" aria-hidden="true" />
    <span class="agent-persona-badge__label">{{ label }}</span>
  </button>
</template>

<style scoped>
.agent-persona-badge {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  flex: none;
  max-width: 220px;
  padding: 2px 8px;
  border-radius: var(--radius-sm);
  border: 1px solid var(--border, color-mix(in srgb, var(--text) 18%, transparent));
  background: color-mix(in srgb, var(--accent) 12%, transparent);
  color: var(--text);
  font-size: 12px;
  line-height: 1.4;
  cursor: pointer;
}
.agent-persona-badge:hover {
  background: color-mix(in srgb, var(--accent) 20%, transparent);
}
.agent-persona-badge:focus-visible {
  outline: 2px solid var(--accent);
  outline-offset: 1px;
}
.agent-persona-badge__label {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
</style>