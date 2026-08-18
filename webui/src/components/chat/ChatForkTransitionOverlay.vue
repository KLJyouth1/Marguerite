<script setup lang="ts">
// Fork-transition overlay：会话分叉加载/返回/失败的全屏状态条。
// 从 ChatView 模板抽取，独立组件便于测试与复用；只依赖 fork 状态本身，
// 不感知 ChatView 的编排细节。
import { useI18n } from 'vue-i18n'
import Icon from '@/components/Icon.vue'
import type { ForkTransitionState } from '@/utils/chat/forkTransition'

const { t } = useI18n()

defineProps<{
  fork: ForkTransitionState
}>()

const emit = defineEmits<{
  retry: []
  returnToParent: []
}>()

function phaseCopy(phase: ForkTransitionState['phase']): string {
  if (phase === 'error') return t('chat.forkOpenFailed')
  if (phase === 'creating') return t('chat.forkCreating')
  if (phase === 'returning') return t('chat.forkReturning')
  return t('chat.forkOpening')
}
</script>

<template>
  <div class="chat-fork-transition-overlay" data-testid="chat-fork-transition-overlay">
    <div
      class="chat-fork-transition-status"
      :class="{ 'chat-fork-transition-status--error': fork.phase === 'error' }"
      :role="fork.phase === 'error' ? 'alert' : 'status'"
      :aria-live="fork.phase === 'error' ? 'assertive' : 'polite'"
      aria-atomic="true"
      data-testid="chat-fork-transition-status"
    >
      <span
        v-if="fork.phase !== 'error'"
        class="chat-fork-transition-status__spinner"
        aria-hidden="true"
      />
      <Icon v-else name="info" :size="15" aria-hidden="true" />
      <span class="chat-fork-transition-status__copy">{{ phaseCopy(fork.phase) }}</span>
      <template v-if="fork.phase === 'error'">
        <button
          type="button"
          class="btn btn--ghost chat-fork-transition-status__action"
          data-testid="chat-fork-retry"
          @click="emit('retry')"
        >{{ t('chat.reloadSession') }}</button>
        <button
          type="button"
          class="btn btn--ghost chat-fork-transition-status__action"
          data-testid="chat-fork-return"
          @click="emit('returnToParent')"
        >{{ t('chat.forkReturnOriginal') }}</button>
      </template>
    </div>
  </div>
</template>
