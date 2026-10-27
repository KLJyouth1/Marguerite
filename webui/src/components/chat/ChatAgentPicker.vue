<!-- P2·E 子智能体指派面板：选择 orchestrator/researcher/coder/sandbox/browser/guard
     多选角色写回 OrchestrationState.agents；父级据此把 form 置为 subagent。 -->
<script setup lang="ts">
import Icon from '@/components/Icon.vue'
import { useI18n } from 'vue-i18n'
import { ORCHESTRATION_ROLES } from '@/composables/chat/orchestration'

const props = defineProps<{
  open: boolean
  agents: string[]
}>()

const emit = defineEmits<{
  'update:open': [value: boolean]
  'update:agents': [value: string[]]
}>()

const { t } = useI18n()

function toggleRole(id: string) {
  const set = new Set(props.agents)
  if (set.has(id)) set.delete(id)
  else set.add(id)
  emit('update:agents', [...set])
}
</script>

<template>
  <div v-if="open" class="orch-popover" role="dialog" :aria-label="t('chat.orchestration.agentTitle')">
    <div class="orch-popover__head">
      <Icon name="agents" :size="14" />
      <span>{{ t('chat.orchestration.agentTitle') }}</span>
      <!-- 选了几个角色在标题上就可见，不必滚到底部看提示 -->
      <span class="orch-popover__count">{{ agents.length }}</span>
    </div>
    <div class="orch-popover__body">
      <button
        v-for="role in ORCHESTRATION_ROLES"
        :key="role.id"
        type="button"
        class="orch-option orch-agent"
        :class="{ 'is-on': agents.includes(role.id) }"
        :aria-pressed="agents.includes(role.id) ? 'true' : 'false'"
        @click="toggleRole(role.id)"
      >
        <span class="orch-option__check" aria-hidden="true">
          <Icon v-if="agents.includes(role.id)" name="check" :size="12" />
        </span>
        <span class="orch-option__copy">
          <strong>{{ role.label }}</strong>
          <small>{{ role.description }}</small>
        </span>
      </button>
      <p class="orch-popover__hint">
        {{ t('chat.orchestration.agentsCount', { n: agents.length }) }}
      </p>
    </div>
  </div>
</template>

<style scoped>
.orch-popover {
  position: absolute;
  right: 0;
  bottom: calc(100% + 6px);
  width: min(300px, calc(100vw - 32px));
  border: 1px solid var(--border-strong);
  border-radius: var(--radius-md);
  background: var(--bg-elevated, var(--bg-surface));
  box-shadow: var(--shadow-xl);
  z-index: 40;
  overflow: hidden;
}
.orch-popover__head {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 8px 12px;
  border-bottom: 1px solid var(--border);
  color: var(--text-dim);
  font-size: var(--fs-xs);
  font-weight: 700;
  letter-spacing: 0.02em;
}
.orch-popover__count {
  margin-left: auto;
  color: var(--text-dim);
  font-weight: 500;
  font-variant-numeric: tabular-nums;
}
.orch-popover__body {
  padding: 10px 12px 12px;
  /* 与 ChatTeamPicker 同口径：不钉死 320px。子智能体卡片带两行说明，
     320px 只能装 2 张半，首尾都被切成半截（截图里第一张被顶栏压住、最后一张被裁）。 */
  max-height: min(520px, calc(100dvh - 14rem));
  overflow-y: auto;
  overscroll-behavior: contain;
  scrollbar-width: thin;
  scrollbar-color: var(--border-strong) transparent;
}
.orch-option {
  display: flex;
  align-items: flex-start;
  gap: 8px;
  width: 100%;
  padding: 7px 8px;
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  background: transparent;
  color: var(--text-muted);
  font: inherit;
  text-align: left;
  cursor: pointer;
  transition:
    border-color var(--dur-fast) var(--ease-standard),
    background var(--dur-fast) var(--ease-standard),
    color var(--dur-fast) var(--ease-standard);
}
.orch-option + .orch-option {
  margin-top: 6px;
}
.orch-option:hover,
.orch-option:focus-visible {
  outline: 0;
  border-color: color-mix(in srgb, var(--accent) 45%, var(--border));
  background: var(--bg-hover);
}
.orch-option.is-on {
  border-color: var(--accent);
  background: color-mix(in srgb, var(--accent) 9%, transparent);
  color: var(--text);
}
.orch-option__check {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  flex: 0 0 18px;
  width: 18px;
  height: 18px;
  margin-top: 1px;
  border: 1px solid var(--border-strong);
  border-radius: var(--radius-sm);
  color: var(--accent);
}
.orch-option.is-on .orch-option__check {
  border-color: var(--accent);
  background: var(--accent);
  color: var(--bg-surface);
}
.orch-option__copy {
  display: flex;
  flex-direction: column;
  gap: 2px;
  min-width: 0;
}
.orch-option__copy strong {
  color: var(--text);
  font-size: var(--fs-sm);
  font-weight: 650;
}
.orch-option__copy small {
  color: var(--text-dim);
  font-size: var(--fs-xs);
  line-height: 1.4;
}
.orch-popover__hint {
  margin: 10px 0 0;
  padding-top: 8px;
  border-top: 1px solid var(--border);
  color: var(--text-dim);
  font-size: var(--fs-xs);
  line-height: 1.5;
}
</style>