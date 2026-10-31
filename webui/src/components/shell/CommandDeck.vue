<!-- 指挥台（CommandDeck）：一条贴在工作区顶部的自适应横栏，把「当前相位 + 为什么 +
     下一步可做的事」摊开。诚实纪律：动作语义全部由父组件的 `actions` 提供，
     组件不自造按钮；`actions` 为空时整块不渲染（不摆一条只会说"暂无"的空条）。
     相位与原因来自 `useWorkPhase` 的纯推导，信号缺失时那里已经落到 idle，
     这里只负责如实显示，不做二次猜测。 -->
<template>
  <section
    v-if="visible"
    class="cmd-deck"
    :class="[`cmd-deck--${phase.kind}`, { 'cmd-deck--reduced': reducedMotion }]"
    role="region"
    :aria-label="t('shell.commandDeck.label')"
    data-testid="command-deck"
    :data-phase="phase.kind"
  >
    <Transition name="cmd-deck-phase" mode="out-in">
      <span :key="phase.kind" class="cmd-deck__phase" data-testid="command-deck-phase">
        {{ t(`shell.commandDeck.phase.${phase.kind}`) }}
      </span>
    </Transition>
    <span class="cmd-deck__reason" data-testid="command-deck-reason">{{ reasonText }}</span>
    <div class="cmd-deck__actions">
      <button
        v-for="action in actions"
        :key="action.id"
        type="button"
        class="cmd-deck__action"
        :data-action="action.id"
        :aria-keyshortcuts="ACTIVATION_KEYS"
        @click="emit('activate', action.id)"
        @keydown.enter.prevent="emit('activate', action.id)"
        @keydown.space.prevent="emit('activate', action.id)"
      >
        <span class="cmd-deck__action-label">{{ action.label }}</span>
        <span v-if="action.hint" class="cmd-deck__action-hint">{{ action.hint }}</span>
      </button>
    </div>
  </section>
</template>

<script setup lang="ts">
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'
import { useMediaQuery } from '@/composables/chat/useMediaQuery'
import type { WorkPhase } from '@/composables/shell/useWorkPhase'

const props = defineProps<{
  phase: WorkPhase
  actions: ReadonlyArray<{ id: string; label: string; hint?: string }>
}>()

const emit = defineEmits<{ activate: [actionId: string] }>()

// Enter / Space 是按钮的原生激活键，显式写进 aria-keyshortcuts 而不是假称某个
// 全局快捷键（本组件没有注册任何全局热键）。
const ACTIVATION_KEYS = 'Enter Space'

const { t } = useI18n()

// 与 <style> 里的 media query 同一判据；ref 版本让"减少动效"这条分支可被单测钉住。
const reducedMotion = useMediaQuery('(prefers-reduced-motion: reduce)')

const visible = computed(() => props.actions.length > 0)

const reasonText = computed(() =>
  t(`shell.commandDeck.reason.${props.phase.reason}`, { count: props.phase.count ?? 0 }),
)
</script>

<style scoped>
/* 入场 180ms / 相位切换 120ms 只落在本组件（不改全局 tokens）。
   动效只碰 transform / opacity，reduced-motion 时整体关闭。 */
.cmd-deck {
  --cmd-deck-enter: 180ms;
  --cmd-deck-switch: 120ms;
  display: flex;
  align-items: center;
  flex-wrap: wrap;
  gap: var(--sp-2);
  box-sizing: border-box;
  width: 100%;
  min-width: 0;
  padding: 6px var(--sp-4);
  border-bottom: 1px solid var(--hairline);
  background: color-mix(in srgb, var(--bg-elevated) 70%, transparent);
  color: var(--text-muted);
  font-size: var(--fs-xs);
  animation: cmd-deck-enter var(--cmd-deck-enter) var(--ease-out) both;
}

.cmd-deck__phase {
  flex: 0 0 auto;
  padding: 1px var(--sp-2);
  border: 1px solid var(--border);
  border-radius: var(--radius-pill);
  color: var(--text);
  font-weight: 600;
  letter-spacing: 0.02em;
  white-space: nowrap;
}

.cmd-deck--explore .cmd-deck__phase {
  color: var(--accent);
  border-color: color-mix(in srgb, var(--accent) 35%, var(--border));
}

.cmd-deck--execute .cmd-deck__phase {
  color: var(--info);
  border-color: color-mix(in srgb, var(--info) 35%, var(--border));
}

.cmd-deck--review .cmd-deck__phase {
  color: var(--warn);
  border-color: color-mix(in srgb, var(--warn) 40%, var(--border));
}

.cmd-deck__reason {
  flex: 1 1 12ch;
  min-width: 0;
  color: var(--text-dim);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.cmd-deck__actions {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
  justify-content: flex-end;
  min-width: 0;
}

.cmd-deck__action {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  min-height: 26px;
  padding: 2px var(--sp-2);
  border: 1px solid var(--border);
  border-radius: var(--radius-pill);
  background: var(--bg-elevated, var(--bg-surface));
  color: var(--text);
  font-family: var(--font-sans);
  font-size: var(--fs-xs);
  line-height: 1.5;
  white-space: nowrap;
  cursor: pointer;
  /* 动效只碰 transform / opacity（颜色不做过渡，瞬时切换）。 */
  transition: transform var(--cmd-deck-switch) var(--ease-standard);
}

.cmd-deck__action:hover {
  border-color: var(--border-strong);
  background: var(--bg-hover);
}

.cmd-deck__action:active {
  transform: translateY(1px);
}

.cmd-deck__action:focus-visible {
  outline: 2px solid var(--accent);
  outline-offset: 2px;
}

.cmd-deck__action-hint {
  color: var(--text-dim);
}

/* 相位切换：只动 opacity / transform，120ms。 */
.cmd-deck-phase-enter-active,
.cmd-deck-phase-leave-active {
  transition:
    opacity var(--cmd-deck-switch) var(--ease-standard),
    transform var(--cmd-deck-switch) var(--ease-standard);
}

.cmd-deck-phase-enter-from {
  opacity: 0;
  transform: translateY(2px);
}

.cmd-deck-phase-leave-to {
  opacity: 0;
  transform: translateY(-2px);
}

@keyframes cmd-deck-enter {
  from {
    opacity: 0;
    transform: translateY(-4px);
  }
}

/* 减少动效：入场与相位切换一起关掉（两条判据取其一：运行时 ref + 系统偏好）。 */
.cmd-deck--reduced,
.cmd-deck--reduced .cmd-deck__phase,
.cmd-deck--reduced .cmd-deck__action {
  animation: none;
  transition: none;
}

@media (prefers-reduced-motion: reduce) {
  .cmd-deck,
  .cmd-deck__phase,
  .cmd-deck__action,
  .cmd-deck-phase-enter-active,
  .cmd-deck-phase-leave-active {
    animation: none;
    transition: none;
  }
}

@media (max-width: 720px) {
  .cmd-deck__reason {
    flex-basis: 100%;
    order: 3;
  }
}
</style>
