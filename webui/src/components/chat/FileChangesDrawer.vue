<template>
  <div v-if="open" class="fcd-overlay" @click.self="emit('close')">
    <aside
      ref="drawerRef"
      class="fcd-drawer"
      role="dialog"
      aria-modal="true"
      :aria-label="t('chat.fileChange.title')"
    >
      <header class="fcd-head">
        <h3 class="fcd-head__title">{{ t('chat.fileChange.title') }}</h3>
        <span v-if="!loading" class="fcd-head__count" aria-hidden="true">{{ changes.length }}</span>
        <button
          type="button"
          class="btn btn--icon btn--ghost"
          :aria-label="t('common.close')"
          :title="t('common.close')"
          @click="emit('close')"
        >
          <Icon name="x" :size="16" />
        </button>
      </header>

      <div class="fcd-body">
        <p v-if="loading" class="fcd-status">{{ t('shared.loading') }}</p>
        <p v-else-if="loadFailed" class="fcd-status fcd-status--error">{{ t('chat.fileChange.loadFailed') }}</p>
        <p v-else-if="changes.length === 0" class="fcd-status">{{ t('chat.fileChange.empty') }}</p>
        <ul v-else class="fcd-list">
          <li v-for="change in changes" :key="changeKey(change)" class="fcd-item">
            <FileChangeCard
              :change="change"
              :rejecting="revertingKeys.has(changeKey(change))"
              @accept="onAccept"
              @reject="onReject"
            />
          </li>
        </ul>
      </div>
    </aside>
  </div>
</template>

<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import Icon from '@/components/Icon.vue'
import FileChangeCard from '@/components/chat/FileChangeCard.vue'
import type { FileChangePayload } from '@/types/rpc'
import { useRpcStore } from '@/stores/rpc'
import { useFileChanges } from '@/composables/chat/useFileChanges'
import { useToasts } from '@/composables/useToasts'
import { useDialogA11y } from '@/composables/useDialogA11y'

const props = defineProps<{
  open: boolean
}>()

const emit = defineEmits<{
  close: []
}>()

const { t } = useI18n()
const rpc = useRpcStore()
const { pushToast } = useToasts()
const drawerRef = ref<HTMLElement | null>(null)
useDialogA11y(drawerRef, computed(() => props.open), () => emit('close'))

const { changes, loading, load } = useFileChanges({ rpc })
const loadFailed = ref(false)
const revertingKeys = ref<Set<string>>(new Set())

function changeKey(change: FileChangePayload): string {
  return `${change.path}:${change.ts}`
}

watch(
  () => props.open,
  (open) => {
    if (!open) return
    loadFailed.value = false
    void load().then((ok) => {
      loadFailed.value = !ok
    })
  },
)

function onAccept(_change: FileChangePayload) {
  // Accept keeps the change on disk; the card already flips to "已接受" locally.
}

async function onReject(change: FileChangePayload) {
  const key = changeKey(change)
  if (revertingKeys.value.has(key)) return
  revertingKeys.value = new Set([...revertingKeys.value, key])
  try {
    const result = await rpc.call<{ ok?: boolean; reverted?: boolean; error?: string }>(
      'workspace.file_changes.revert',
      { path: change.path, kind: change.kind, before: change.before },
    )
    if (result?.ok === true || result?.reverted === true) {
      changes.value = changes.value.filter(item => changeKey(item) !== key)
    } else {
      pushToast(result?.error || t('chat.fileChange.loadFailed'), { tone: 'danger' })
    }
  } catch (error) {
    pushToast(
      error instanceof Error ? error.message : t('chat.fileChange.loadFailed'),
      { tone: 'danger' },
    )
  } finally {
    const next = new Set(revertingKeys.value)
    next.delete(key)
    revertingKeys.value = next
  }
}
</script>

<style scoped>
.fcd-overlay {
  position: fixed;
  inset: 0;
  z-index: 100;
  background: color-mix(in srgb, var(--bg) 40%, transparent);
}

.fcd-drawer {
  position: absolute;
  top: 0;
  right: 0;
  bottom: 0;
  width: min(460px, calc(100vw - var(--sp-3)));
  display: flex;
  flex-direction: column;
  background: var(--bg-elevated);
  border-left: 1px solid var(--border);
  box-shadow: var(--shadow-lg);
}

.fcd-head {
  display: flex;
  align-items: center;
  gap: var(--sp-2);
  padding: var(--sp-3);
  border-bottom: 1px solid var(--border);
}

.fcd-head__title {
  flex: 1 1 auto;
  margin: 0;
  font-size: var(--fs-md);
  font-weight: 600;
}

.fcd-head__count {
  color: var(--text-muted);
  font-family: var(--font-mono);
  font-size: var(--fs-xs);
}

.fcd-body {
  flex: 1 1 auto;
  overflow: auto;
  padding: var(--sp-3);
}

.fcd-status {
  margin: 0;
  color: var(--text-muted);
  font-size: var(--fs-sm);
}

.fcd-status--error {
  color: var(--danger);
}

.fcd-list {
  display: flex;
  flex-direction: column;
  gap: var(--sp-3);
  margin: 0;
  padding: 0;
  list-style: none;
}

.fcd-item {
  min-width: 0;
}
</style>
