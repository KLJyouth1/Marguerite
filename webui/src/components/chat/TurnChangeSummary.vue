<template>
  <section
    v-if="show"
    class="turn-changes"
    role="region"
    :aria-label="t('chat.turnChanges.region')"
    data-testid="turn-changes"
  >
    <div class="turn-changes__bar">
      <button
        type="button"
        class="turn-changes__toggle"
        :aria-expanded="expanded"
        :title="t('chat.turnChanges.toggle')"
        @click="expanded = !expanded"
      >
        <Icon name="fileText" :size="13" aria-hidden="true" />
        <span class="turn-changes__title">{{ title }}</span>
        <span v-if="summary.added || summary.removed" class="turn-changes__stats">
          <span v-if="summary.added" class="turn-changes__stat turn-changes__stat--add">+{{ summary.added }}</span>
          <span v-if="summary.removed" class="turn-changes__stat turn-changes__stat--del">−{{ summary.removed }}</span>
        </span>
        <Icon :name="expanded ? 'chevronDown' : 'chevronRight'" :size="11" aria-hidden="true" />
      </button>

      <div v-if="!confirming" class="turn-changes__actions">
        <button type="button" class="turn-changes__action" @click="emit('view')">
          {{ t('chat.turnChanges.view') }}
        </button>
        <button type="button" class="turn-changes__action" @click="dismiss">
          {{ t('chat.turnChanges.keep') }}
        </button>
        <button
          v-if="canRevert"
          type="button"
          class="turn-changes__action turn-changes__action--danger"
          :disabled="reverting"
          @click="confirming = true"
        >
          {{ t('chat.turnChanges.revert') }}
        </button>
      </div>
      <div v-else class="turn-changes__actions">
        <button
          type="button"
          class="turn-changes__action"
          :disabled="reverting"
          @click="confirming = false"
        >
          {{ t('common.cancel') }}
        </button>
        <button
          type="button"
          class="turn-changes__action turn-changes__action--danger turn-changes__action--solid"
          :disabled="reverting"
          data-testid="turn-changes-confirm"
          @click="confirmRevert"
        >
          {{ reverting ? t('chat.turnChanges.reverting') : confirmLabel }}
        </button>
      </div>
    </div>

    <p v-if="confirming && !reverting" class="turn-changes__hint">
      {{ t('chat.turnChanges.confirmHint') }}
    </p>
    <p v-if="reverting" class="turn-changes__hint" role="status" aria-live="polite">
      {{ t('chat.turnChanges.revertingStatus', { n: changes.length }) }}
    </p>
    <p v-if="failureText" class="turn-changes__hint turn-changes__hint--danger" role="alert">
      {{ failureText }}
    </p>

    <ul v-if="expanded" class="turn-changes__list">
      <li v-for="change in changes" :key="change.path" class="turn-changes__row">
        <span v-if="kindLabel(change.kind)" class="turn-changes__kind">{{ kindLabel(change.kind) }}</span>
        <code class="turn-changes__path" :title="change.path">{{ change.path }}</code>
        <span class="turn-changes__delta">
          <span v-if="change.added" class="turn-changes__stat turn-changes__stat--add">+{{ change.added }}</span>
          <span v-if="change.removed" class="turn-changes__stat turn-changes__stat--del">−{{ change.removed }}</span>
        </span>
      </li>
    </ul>
  </section>
</template>

<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import Icon from '@/components/Icon.vue'
import { useRpcStore } from '@/stores/rpc'
import { useToasts } from '@/composables/useToasts'
import {
  useTurnFileChanges,
  type TurnRevertFailure,
} from '@/composables/chat/useTurnFileChanges'

const props = defineProps<{
  sessionKey: string
  /** 本回合的用户消息时间戳（epoch 秒/毫秒或 ISO 串均可） */
  since: string | number | null
}>()

const emit = defineEmits<{
  view: []
  reverted: [count: number]
  stats: [payload: { files: number, canRevert: boolean }]
}>()

const { t } = useI18n()
const rpc = useRpcStore()
const { pushToast } = useToasts()
const store = useTurnFileChanges({ rpc })
const { changes, reverting, canRevert, summary } = store

const dismissed = ref(false)
const expanded = ref(false)
const confirming = ref(false)
const failures = ref<TurnRevertFailure[]>([])

const show = computed(() => !dismissed.value && store.visible.value)

const title = computed(() => (
  summary.value.files === 1
    ? t('chat.turnChanges.titleOne')
    : t('chat.turnChanges.title', { n: summary.value.files })
))

const confirmLabel = computed(() => (
  summary.value.files === 1
    ? t('chat.turnChanges.confirmOne')
    : t('chat.turnChanges.confirm', { n: summary.value.files })
))

const failureText = computed(() => {
  const first = failures.value[0]
  if (!first) return ''
  let text = ''
  if (first.code === 'no_snapshot') {
    text = t('chat.turnChanges.noSnapshot', { path: first.path })
  } else if (first.code === 'ledger_unavailable') {
    text = t('chat.turnChanges.ledgerUnavailable', { path: first.path })
  } else {
    text = first.error
      ? t('chat.turnChanges.revertFailedWithError', { path: first.path, error: first.error })
      : t('chat.turnChanges.revertFailed', { path: first.path })
  }
  const rest = failures.value.length - 1
  return rest > 0 ? `${text} ${t('chat.turnChanges.moreFailures', { n: rest })}` : text
})

function kindLabel(kind: string): string {
  // 后端没给类型时留空，不猜成「修改」。
  if (kind !== 'create' && kind !== 'modify' && kind !== 'delete') return ''
  return t(`chat.fileChange.${kind}`)
}

function dismiss() {
  dismissed.value = true
}

async function confirmRevert() {
  const result = await store.revertAll()
  if (!result) return
  failures.value = result.failures
  confirming.value = false
  if (result.failures.length === 0) {
    dismissed.value = true
    pushToast(t('chat.turnChanges.reverted', { n: result.reverted }), { tone: 'ok' })
    emit('reverted', result.reverted)
    return
  }
  pushToast(
    t('chat.turnChanges.partial', {
      reverted: result.reverted,
      failed: result.failures.length,
    }),
    { tone: 'warn' },
  )
}

watch(
  () => [props.sessionKey, props.since] as const,
  ([sessionKey, since]) => {
    dismissed.value = false
    expanded.value = false
    confirming.value = false
    failures.value = []
    if (!sessionKey || since === null || since === undefined || since === '') {
      store.reset()
      return
    }
    void store.load(sessionKey, since)
  },
  { immediate: true },
)

// 「下一步建议」需要知道这条汇总是否真的有内容可操作；用事件上报而不是
// 让父组件再拉一次同一份改动台账。
watch(
  () => [show.value, summary.value.files, canRevert.value] as const,
  ([visible, files, revert]) => {
    emit('stats', {
      files: visible ? files : 0,
      canRevert: visible && revert,
    })
  },
  { immediate: true },
)

/** 供「下一步建议」复用同一份状态：打开两段式确认，而不是另起一套回滚逻辑。 */
function requestRevert() {
  if (!canRevert.value || reverting.value) return
  expanded.value = true
  confirming.value = true
}

defineExpose({ requestRevert })
</script>

<style scoped>
.turn-changes {
  width: min(calc(100% - 48px), var(--composer-col, 820px));
  margin: 0 auto var(--sp-2);
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  background: var(--bg-elevated, var(--bg-surface));
  color: var(--text);
  font-size: var(--fs-xs);
  overflow: hidden;
}

.turn-changes__bar {
  display: flex;
  align-items: center;
  gap: var(--sp-2);
  padding: 6px 10px;
}

.turn-changes__toggle {
  display: flex;
  flex: 1 1 auto;
  align-items: center;
  gap: 6px;
  min-width: 0;
  padding: 0;
  border: 0;
  background: transparent;
  color: var(--text);
  font: inherit;
  text-align: start;
  cursor: pointer;
}

.turn-changes__toggle:focus-visible,
.turn-changes__action:focus-visible {
  outline: 2px solid var(--accent);
  outline-offset: 2px;
}

.turn-changes__title {
  font-weight: 600;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.turn-changes__stats,
.turn-changes__delta {
  display: inline-flex;
  gap: 6px;
  flex-shrink: 0;
  font-family: var(--font-mono);
}

.turn-changes__stat--add {
  color: var(--ok);
}

.turn-changes__stat--del {
  color: var(--danger);
}

.turn-changes__actions {
  display: flex;
  align-items: center;
  gap: 4px;
  flex-shrink: 0;
}

.turn-changes__action {
  padding: 2px 8px;
  border: 1px solid transparent;
  border-radius: var(--radius-sm);
  background: transparent;
  color: var(--text-muted);
  font-family: var(--font-sans);
  font-size: var(--fs-xs);
  font-weight: 500;
  line-height: 1.5;
  white-space: nowrap;
  cursor: pointer;
  transition: background var(--dur-fast) var(--ease-standard), color var(--dur-fast) var(--ease-standard);
}

.turn-changes__action:not(:disabled):hover {
  background: var(--bg-hover);
  color: var(--text);
}

.turn-changes__action:disabled {
  opacity: var(--state-disabled-opacity);
  cursor: not-allowed;
}

.turn-changes__action--danger {
  color: var(--danger);
  border-color: color-mix(in srgb, var(--danger) 30%, transparent);
}

.turn-changes__action--solid {
  background: var(--danger);
  border-color: var(--danger);
  color: var(--bg);
}

.turn-changes__action--solid:not(:disabled):hover {
  background: var(--danger-fill);
  border-color: var(--danger-fill);
  color: var(--bg);
  opacity: 1;
}

.turn-changes__hint {
  margin: 0;
  padding: 0 10px 6px;
  color: var(--text-muted);
  font-size: var(--fs-xs);
}

.turn-changes__hint--danger {
  color: var(--danger);
}

.turn-changes__list {
  display: flex;
  flex-direction: column;
  gap: 2px;
  margin: 0;
  padding: 6px 10px 8px;
  border-top: 1px solid var(--border);
  list-style: none;
}

.turn-changes__row {
  display: flex;
  align-items: center;
  gap: 6px;
  min-width: 0;
}

.turn-changes__kind {
  flex-shrink: 0;
  padding: 0 4px;
  border: 1px solid var(--border);
  border-radius: var(--radius-xs);
  color: var(--text-muted);
  font-size: var(--fs-xs);
}

.turn-changes__path {
  flex: 1 1 auto;
  min-width: 0;
  overflow: hidden;
  color: var(--text);
  font-family: var(--font-mono);
  text-overflow: ellipsis;
  white-space: nowrap;
}

@media (max-width: 640px) {
  .turn-changes {
    width: calc(100% - 24px);
  }

  .turn-changes__bar {
    flex-wrap: wrap;
  }

  .turn-changes__toggle {
    flex-basis: 100%;
  }

  .turn-changes__actions {
    margin-left: auto;
  }
}
</style>
