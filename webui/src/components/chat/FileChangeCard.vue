<template>
  <article class="file-change-card" :data-kind="change.kind" :data-expanded="expanded ? 'true' : 'false'">
    <button
      type="button"
      class="file-change-card__header"
      :aria-expanded="expanded ? 'true' : 'false'"
      :aria-label="t('chat.fileChange.toggle', { path: change.path })"
      @click="expanded = !expanded"
    >
      <span class="file-change-card__chevron" aria-hidden="true">{{ expanded ? '▾' : '▸' }}</span>
      <span class="file-change-card__badge" :class="`file-change-card__badge--${change.kind}`">{{ kindLabel }}</span>
      <span class="file-change-card__path" :title="change.path">{{ change.path }}</span>
      <span class="file-change-card__meta">{{ summaryLabel }}</span>
    </button>

    <div v-if="expanded" class="file-change-card__diff" role="region">
      <div
        v-for="(line, index) in diffLines"
        :key="index"
        class="file-change-card__line"
        :data-type="line.type"
      >
        <span class="file-change-card__line-mark" aria-hidden="true">{{ lineMark(line.type) }}</span>
        <span class="file-change-card__line-text">{{ line.text || ' ' }}</span>
      </div>
      <div v-if="!diffLines.length" class="file-change-card__empty" data-testid="file-change-empty">
        {{ t('chat.fileChange.noContent') }}
      </div>
    </div>

    <div class="file-change-card__actions">
      <button
        type="button"
        class="file-change-card__action file-change-card__action--accept"
        :disabled="accepted"
        @click="accept"
      >
        <Icon v-if="accepted" name="check" :size="13" />
        <span>{{ accepted ? t('chat.fileChange.accepted') : t('chat.fileChange.accept') }}</span>
      </button>
      <button
        type="button"
        class="file-change-card__action file-change-card__action--reject"
        :disabled="rejecting"
        @click="$emit('reject', change)"
      >
        <span>{{ t('chat.fileChange.reject') }}</span>
      </button>
    </div>
  </article>
</template>

<script setup lang="ts">
import { computed, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import Icon from '@/components/Icon.vue'
import type { FileChangePayload } from '@/types/rpc'
import { diffLines as computeDiffLines, diffStats } from '@/utils/chat/fileDiff'

const props = defineProps<{
  change: FileChangePayload
  /** 回滚进行中：禁用「拒绝」按钮，避免重复提交。 */
  rejecting?: boolean
}>()

const emit = defineEmits<{
  accept: [change: FileChangePayload]
  reject: [change: FileChangePayload]
}>()

const { t } = useI18n()

const expanded = ref(false)
const accepted = ref(false)

const diffLines = computed(() => computeDiffLines(props.change.before, props.change.after))

const kindLabel = computed(() => {
  switch (props.change.kind) {
    case 'create': return t('chat.fileChange.create')
    case 'delete': return t('chat.fileChange.delete')
    default: return t('chat.fileChange.modify')
  }
})

const summaryLabel = computed(() => {
  const { added, removed } = diffStats(diffLines.value)
  const parts: string[] = []
  if (added) parts.push(`+${added}`)
  if (removed) parts.push(`-${removed}`)
  return parts.length ? parts.join(' ') : t('chat.fileChange.unchanged')
})

function lineMark(type: 'ctx' | 'del' | 'add'): string {
  if (type === 'add') return '+'
  if (type === 'del') return '-'
  return ' '
}

function accept() {
  if (accepted.value) return
  accepted.value = true
  emit('accept', props.change)
}
</script>

<style scoped>
.file-change-card {
  display: flex;
  flex-direction: column;
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  background: var(--bg-surface);
  overflow: hidden;
}

.file-change-card__header {
  display: flex;
  align-items: center;
  gap: 8px;
  width: 100%;
  padding: 8px 10px;
  border: 0;
  background: transparent;
  color: var(--text);
  text-align: left;
  cursor: pointer;
}

.file-change-card__header:hover {
  background: var(--bg-hover);
}

.file-change-card__chevron {
  flex: 0 0 auto;
  width: 12px;
  color: var(--text-muted);
  font-size: 11px;
}

.file-change-card__badge {
  flex: 0 0 auto;
  padding: 1px 6px;
  border-radius: var(--radius-sm);
  font-family: var(--font-mono);
  font-size: 10px;
  font-weight: 700;
  text-transform: uppercase;
  letter-spacing: 0.04em;
}

.file-change-card__badge--create {
  color: var(--ok);
  background: color-mix(in srgb, var(--ok) 12%, transparent);
}

.file-change-card__badge--modify {
  color: var(--accent);
  background: color-mix(in srgb, var(--accent) 12%, transparent);
}

.file-change-card__badge--delete {
  color: var(--danger);
  background: color-mix(in srgb, var(--danger) 12%, transparent);
}

.file-change-card__path {
  flex: 1 1 auto;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-family: var(--font-mono);
  font-size: 12px;
}

.file-change-card__meta {
  flex: 0 0 auto;
  color: var(--text-muted);
  font-family: var(--font-mono);
  font-size: 11px;
  font-variant-numeric: tabular-nums;
}

.file-change-card__diff {
  display: flex;
  flex-direction: column;
  max-height: 320px;
  overflow: auto;
  border-top: 1px solid var(--border);
  background: var(--bg);
  font-family: var(--font-mono);
  font-size: 11px;
  line-height: 1.5;
}

.file-change-card__line {
  display: flex;
  min-width: max-content;
  padding: 0 10px;
}

.file-change-card__line[data-type='add'] {
  background: color-mix(in srgb, var(--ok) 10%, transparent);
  color: color-mix(in srgb, var(--ok) 80%, var(--text));
}

.file-change-card__line[data-type='del'] {
  background: color-mix(in srgb, var(--danger) 10%, transparent);
  color: color-mix(in srgb, var(--danger) 80%, var(--text));
}

.file-change-card__line[data-type='ctx'] {
  color: var(--text-muted);
}

.file-change-card__line-mark {
  flex: 0 0 auto;
  width: 16px;
  user-select: none;
  text-align: center;
  opacity: 0.6;
}

.file-change-card__line-text {
  white-space: pre;
}

.file-change-card__empty {
  padding: 8px 10px;
  color: var(--text-muted);
  font-family: var(--font-sans);
  font-size: 12px;
}

.file-change-card__actions {
  display: flex;
  gap: 6px;
  padding: 6px 10px;
  border-top: 1px solid var(--border);
}

.file-change-card__action {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  height: 26px;
  padding: 0 10px;
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  background: var(--bg-surface);
  color: var(--text);
  font-size: 12px;
  cursor: pointer;
}

.file-change-card__action--accept {
  color: var(--ok);
}

.file-change-card__action--reject {
  color: var(--danger);
}

.file-change-card__action:hover {
  border-color: color-mix(in srgb, currentColor 35%, var(--border));
}

.file-change-card__action:disabled {
  cursor: default;
  opacity: 0.6;
}
</style>
