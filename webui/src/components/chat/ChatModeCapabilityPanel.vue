<!-- P2·E 能力组合面板：只做「能力勾选」。
     3×3 档位矩阵已经由输入框旁「模式」弹层（ChatComposerPresets → ModeMatrix）承担，
     此处再画一遍会让同一份 USER_MODE_META 出现两个可写副本 —— 实测两个面板内容完全重复。
     这里保留一行只读的「当前模式」，告诉用户档位在哪里改。 -->
<script setup lang="ts">
import { computed } from 'vue'
import Icon from '@/components/Icon.vue'
import { useI18n } from 'vue-i18n'
import { USER_MODE_META } from '@/stores/activeMode'
import { CAPABILITY_GROUPS } from '@/composables/chat/orchestration'

const props = defineProps<{
  open: boolean
  modeKey: string
  capabilities: string[]
}>()

const emit = defineEmits<{
  'update:open': [value: boolean]
  'update:capabilities': [value: string[]]
}>()

const { t } = useI18n()

const currentMeta = computed(() => USER_MODE_META[props.modeKey])

function toggleCapability(id: string) {
  const set = new Set(props.capabilities)
  if (set.has(id)) set.delete(id)
  else set.add(id)
  emit('update:capabilities', [...set])
}
</script>

<template>
  <div v-if="open" class="orch-popover orch-mode" role="dialog" :aria-label="t('chat.orchestration.capabilitiesTitle')">
    <div class="orch-popover__head">
      <Icon name="capabilities" :size="14" />
      <span>{{ t('chat.orchestration.capabilitiesTitle') }}</span>
    </div>
    <div class="orch-popover__body">
      <p class="orch-mode__current">
        <span class="orch-mode__current-label">{{ t('chat.presets.label') }}</span>
        {{ currentMeta?.label || t('chat.orchestration.modeNone') }}
      </p>

      <div
        v-for="group in CAPABILITY_GROUPS"
        :key="group.id"
        class="orch-mode__group"
      >
        <p class="orch-mode__group-title">
          <Icon :name="group.icon" :size="12" />
          {{ group.label }}
        </p>
        <div class="orch-mode__group-items">
          <button
            v-for="item in group.items"
            :key="item.id"
            type="button"
            class="orch-mode__cap"
            :class="{ 'is-on': capabilities.includes(item.id) }"
            :aria-pressed="capabilities.includes(item.id) ? 'true' : 'false'"
            @click="toggleCapability(item.id)"
          >
            <span class="orch-mode__cap-check" aria-hidden="true">
              <Icon v-if="capabilities.includes(item.id)" name="check" :size="11" />
            </span>
            {{ item.label }}
          </button>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.orch-popover {
  position: absolute;
  right: 0;
  bottom: calc(100% + 6px);
  width: min(320px, calc(100vw - 32px));
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
.orch-popover__body {
  padding: 10px 12px 12px;
  /* 能力分组较多，高度跟随可用空间并允许滚动，避免看起来像坏掉的半截面板。 */
  max-height: min(520px, calc(100dvh - 14rem));
  overflow-y: auto;
  overscroll-behavior: contain;
  scrollbar-width: thin;
  scrollbar-color: var(--border-strong) transparent;
}
.orch-mode__current {
  display: flex;
  align-items: baseline;
  gap: 6px;
  margin: 0 0 10px;
  padding-bottom: 8px;
  border-bottom: 1px solid var(--border);
  color: var(--text);
  font-size: var(--fs-xs);
}
.orch-mode__current-label {
  color: var(--text-dim);
  font-weight: 600;
}
.orch-mode__group + .orch-mode__group {
  margin-top: 8px;
}
.orch-mode__group-title {
  display: flex;
  align-items: center;
  gap: 4px;
  margin: 0 0 4px;
  color: var(--text-muted);
  font-size: var(--fs-xs);
  font-weight: 600;
}
.orch-mode__group-items {
  display: flex;
  flex-wrap: wrap;
  gap: 4px;
}
.orch-mode__cap {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  padding: 3px 7px;
  border: 1px solid var(--border);
  border-radius: var(--radius-pill);
  background: transparent;
  color: var(--text-muted);
  font: inherit;
  font-size: var(--fs-xs);
  cursor: pointer;
  transition:
    border-color var(--dur-fast) var(--ease-standard),
    background var(--dur-fast) var(--ease-standard),
    color var(--dur-fast) var(--ease-standard);
}
.orch-mode__cap:hover,
.orch-mode__cap:focus-visible {
  outline: 0;
  border-color: color-mix(in srgb, var(--accent) 45%, var(--border));
  background: var(--bg-hover);
}
.orch-mode__cap.is-on {
  border-color: var(--accent);
  background: color-mix(in srgb, var(--accent) 10%, transparent);
  color: var(--text);
}
.orch-mode__cap-check {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 13px;
  height: 13px;
  border: 1px solid var(--border-strong);
  border-radius: var(--radius-xs);
  color: var(--accent);
}
.orch-mode__cap.is-on .orch-mode__cap-check {
  border-color: var(--accent);
  background: var(--accent);
  color: var(--bg-surface);
}
</style>