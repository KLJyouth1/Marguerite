<!--
  会话文件改动的内联 diff 查看器。

  数据源优先级：
  1. `sessions.fileChanges`（新契约：带统一 diff 正文与 +/- 行数）；
  2. 旧网关没有该方法 → 降级到 `artifacts.list`，只列文件行、不显示 diff 正文，
     并在卡片顶部给一行说明。降级不是错误，所以不会弹 toast、不会伪造 diff。

  颜色只走语义 token / color-mix：+ 绿、- 红、@@ 中性、改动处黄色 gutter。
-->
<template>
  <section
    class="fcv"
    :class="{ 'fcv--embedded': embedded }"
    :data-state="state"
    data-testid="file-change-viewer"
  >
    <header v-if="!embedded" class="fcv__head">
      <Icon name="fileText" :size="13" aria-hidden="true" />
      <span class="fcv__title">{{ t('chat.diff.title') }}</span>
      <span v-if="visibleChanges.length" class="fcv__count">{{ visibleChanges.length }}</span>
    </header>

    <p v-if="state === 'loading'" class="fcv__note" data-testid="file-change-loading">
      {{ t('chat.diff.loading') }}
    </p>
    <p v-else-if="state === 'error'" class="fcv__note fcv__note--danger" data-testid="file-change-failed">
      {{ t('chat.diff.failed') }}
    </p>
    <p v-else-if="degraded" class="fcv__note" data-testid="file-change-degraded">
      {{ t('chat.fileChange.loadFailed') }}
    </p>
    <p v-else-if="!visibleChanges.length" class="fcv__note" data-testid="file-change-empty">
      {{ t('chat.diff.noChanges') }}
    </p>

    <ul v-if="state !== 'loading' && state !== 'error' && visibleChanges.length" class="fcv__list">
      <li
        v-for="(change, index) in visibleChanges"
        :key="`${change.path}:${index}`"
        class="fcv__item"
        data-testid="file-change-row"
        :data-path="change.path"
        :data-kind="change.kind"
      >
        <div class="fcv__row">
          <button
            v-if="change.diff"
            type="button"
            class="fcv__toggle"
            :aria-expanded="isOpen(change)"
            :aria-controls="diffId(change, index)"
            :title="change.path"
            data-testid="file-change-toggle"
            @click="toggle(change)"
          >
            <span v-if="change.kind" class="fcv__kind" :class="`fcv__kind--${change.kind}`">
              {{ kindLabel(change.kind) }}
            </span>
            <code class="fcv__path">{{ change.path }}</code>
            <span class="fcv__stats" data-testid="file-change-stats">
              <span class="fcv__added">+{{ statsOf(change).added }}</span>
              <span class="fcv__removed">-{{ statsOf(change).removed }}</span>
            </span>
            <Icon
              class="fcv__chevron"
              :name="isOpen(change) ? 'chevronDown' : 'chevronRight'"
              :size="12"
              aria-hidden="true"
            />
          </button>
          <div v-else class="fcv__toggle fcv__toggle--static">
            <span v-if="change.kind" class="fcv__kind" :class="`fcv__kind--${change.kind}`">
              {{ kindLabel(change.kind) }}
            </span>
            <code class="fcv__path">{{ change.path }}</code>
          </div>
          <button
            type="button"
            class="fcv__open"
            :title="t('chat.diff.openInEditor')"
            :aria-label="t('chat.diff.openInEditor')"
            data-testid="file-change-open"
            @click="openInEditor(change)"
          >
            <Icon name="externalLink" :size="12" aria-hidden="true" />
          </button>
        </div>

        <div
          v-if="change.diff && isOpen(change)"
          :id="diffId(change, index)"
          class="fcv__diff"
          role="region"
          data-testid="file-change-diff"
        >
          <div
            v-for="(line, lineIndex) in linesOf(change)"
            :key="lineIndex"
            class="fcv__line"
            :class="`fcv__line--${diffLineTone(line)}`"
          >
            <span class="fcv__gutter" aria-hidden="true" />
            <span class="fcv__code">{{ line || ' ' }}</span>
          </div>
          <p
            v-if="change.truncated"
            class="fcv__note fcv__note--inline"
            data-testid="file-change-truncated"
          >
            {{ t('chat.orchestration.runTruncated', { n: change.diff.length }) }}
          </p>
        </div>
      </li>
    </ul>
  </section>
</template>

<script setup lang="ts">
import { computed, onMounted, ref, useId, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import Icon from '@/components/Icon.vue'
import { useRpcStore } from '@/stores/rpc'
import type { ArtifactsListResponse } from '@/types/rpc'
import { requestEditorWorkbenchOpen } from '@/workbench/editorItems'
import {
  countDiffLines,
  diffLineTone,
  normalizeSessionFileChanges,
  type SessionFileChange,
} from '@/utils/chat/toolPayload'

const props = withDefaults(defineProps<{
  sessionKey: string
  /** 只展示这一个路径（交付回执的行内点开）。 */
  onlyPath?: string
  /** 首次渲染时展开该路径。 */
  initialOpenPath?: string
  /** 作为回执的子区域渲染：隐藏自己的标题行。 */
  embedded?: boolean
}>(), {
  onlyPath: '',
  initialOpenPath: '',
  embedded: false,
})

const { t } = useI18n()
const rpc = useRpcStore()
const uid = useId().replace(/[^a-zA-Z0-9_-]/g, '')

interface FileChangesResponse {
  ok?: boolean
  changes?: unknown
  error?: string
  code?: string
}

const loading = ref(false)
const error = ref('')
const degraded = ref(false)
const changes = ref<SessionFileChange[]>([])
const openPaths = ref(new Set<string>())

const visibleChanges = computed(() => {
  const target = props.onlyPath.trim()
  return target ? changes.value.filter(change => change.path === target) : changes.value
})

const state = computed(() => {
  if (loading.value) return 'loading'
  if (error.value) return 'error'
  if (degraded.value) return 'degraded'
  if (!visibleChanges.value.length) return 'empty'
  return 'list'
})

function isMethodMissing(code: string | undefined, message: string | undefined): boolean {
  const normalized = (code || '').toUpperCase()
  if (normalized === 'NOT_IMPLEMENTED' || normalized === 'METHOD_NOT_FOUND') return true
  return /not[_ ]?implemented|unknown method|method not found|方法不存在|未实现/i.test(message || '')
}

async function ensureConnected(): Promise<boolean> {
  if (rpc.isConnected) return true
  try {
    await rpc.waitForConnection(10_000)
    return rpc.isConnected
  } catch {
    return false
  }
}

function seedOpenPaths() {
  const initial = props.initialOpenPath.trim()
  const only = props.onlyPath.trim()
  openPaths.value = new Set([initial || only].filter(Boolean))
}

/**
 * 旧网关降级：用产物台账列出文件行。
 * 没有 diff 正文就绝不显示 diff —— 行本身来自真实的 artifacts.list。
 */
async function loadArtifactFallback(): Promise<void> {
  degraded.value = true
  if (!rpc.supportsMethod('artifacts.list')) {
    changes.value = []
    return
  }
  const res = await rpc.call<ArtifactsListResponse>('artifacts.list', {
    sessionKey: props.sessionKey,
    limit: 50,
  })
  const artifacts = Array.isArray(res?.artifacts) ? res!.artifacts : []
  changes.value = artifacts
    .map((artifact): SessionFileChange | null => {
      const path = String(artifact.path || artifact.name || '').trim()
      if (!path) return null
      return {
        path,
        kind: '',
        tool: String(artifact.kind || '').trim(),
        added: null,
        removed: null,
        diff: '',
        truncated: false,
        ts: null,
      }
    })
    .filter((row): row is SessionFileChange => row !== null)
}

async function load(): Promise<void> {
  const session = props.sessionKey.trim()
  if (!session) {
    changes.value = []
    return
  }
  loading.value = true
  error.value = ''
  degraded.value = false
  try {
    if (!await ensureConnected()) {
      error.value = t('chat.receipt.notConnected')
      changes.value = []
      return
    }
    if (!rpc.supportsMethod('sessions.fileChanges')) {
      await loadArtifactFallback()
      return
    }
    const res = await rpc.call<FileChangesResponse>('sessions.fileChanges', { session })
    if (res && res.ok === false) {
      if (isMethodMissing(res.code, res.error)) {
        await loadArtifactFallback()
        return
      }
      error.value = res.error || t('chat.diff.failed')
      changes.value = []
      return
    }
    changes.value = normalizeSessionFileChanges(res)
  } catch (err) {
    const message = err instanceof Error ? err.message : String(err)
    if (isMethodMissing(undefined, message)) {
      try {
        await loadArtifactFallback()
        return
      } catch (fallbackError) {
        error.value = fallbackError instanceof Error ? fallbackError.message : String(fallbackError)
        changes.value = []
        return
      }
    }
    error.value = message || t('chat.diff.failed')
    changes.value = []
  } finally {
    loading.value = false
  }
}

function statsOf(change: SessionFileChange): { added: number, removed: number } {
  const counted = countDiffLines(change.diff)
  return {
    added: change.added ?? counted.added,
    removed: change.removed ?? counted.removed,
  }
}

function linesOf(change: SessionFileChange): string[] {
  return String(change.diff || '').split(/\r?\n/)
}

function kindLabel(kind: SessionFileChange['kind']): string {
  if (kind === 'create') return t('chat.diff.added')
  if (kind === 'delete') return t('chat.diff.removed')
  return t('chat.diff.modified')
}

function diffId(change: SessionFileChange, index: number): string {
  return `file-change-diff-${uid}-${index}-${change.path.replace(/[^a-zA-Z0-9]/g, '-')}`
}

function isOpen(change: SessionFileChange): boolean {
  return openPaths.value.has(change.path)
}

function toggle(change: SessionFileChange) {
  const next = new Set(openPaths.value)
  if (next.has(change.path)) next.delete(change.path)
  else next.add(change.path)
  openPaths.value = next
}

function openInEditor(change: SessionFileChange) {
  const session = props.sessionKey.trim()
  if (!session || !change.path) return
  requestEditorWorkbenchOpen({ scopeId: session, path: change.path })
}

watch(() => [props.sessionKey, props.onlyPath, props.initialOpenPath], () => {
  seedOpenPaths()
  changes.value = []
  void load()
})

onMounted(() => {
  seedOpenPaths()
  void load()
})
</script>

<style scoped>
.fcv {
  margin-top: 0.5rem;
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  background: var(--bg-elevated, var(--bg-surface));
  overflow: hidden;
  font-size: var(--fs-xs);
}

.fcv--embedded {
  margin: 0.375rem 0 0;
}

.fcv__head {
  display: flex;
  align-items: center;
  gap: 0.375rem;
  padding: 0.375rem 0.5rem;
  border-bottom: 1px solid var(--hairline);
  color: var(--text-muted);
  font-weight: 600;
}

.fcv__title {
  color: var(--text);
}

.fcv__count {
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

.fcv__note {
  margin: 0;
  padding: 0.375rem 0.5rem;
  color: var(--text-muted);
  line-height: 1.5;
}

.fcv__note--inline {
  padding: 0.25rem 0.5rem;
  border-top: 1px dashed var(--border);
}

.fcv__note--danger {
  color: var(--danger);
}

.fcv__list {
  margin: 0;
  padding: 0;
  list-style: none;
}

.fcv__item + .fcv__item {
  border-top: 1px solid var(--hairline);
}

.fcv__row {
  display: flex;
  align-items: center;
  min-width: 0;
}

.fcv__toggle {
  display: flex;
  align-items: center;
  gap: 0.375rem;
  flex: 1 1 auto;
  min-width: 0;
  padding: 0.375rem 0.5rem;
  border: 0;
  background: transparent;
  color: var(--text);
  font: inherit;
  text-align: left;
  cursor: pointer;
}

.fcv__toggle--static {
  cursor: default;
}

.fcv__toggle:hover {
  background: var(--bg-hover);
}

.fcv__toggle:focus-visible,
.fcv__open:focus-visible {
  outline: none;
  box-shadow: var(--focus-ring);
}

.fcv__kind {
  flex: 0 0 auto;
  padding: 0 0.3125rem;
  border-radius: var(--radius-sm);
  background: var(--bg-hover);
  color: var(--text-muted);
  font-size: 0.625rem;
  font-weight: 600;
  letter-spacing: 0.02em;
}

.fcv__kind--create {
  background: color-mix(in srgb, var(--ok) 16%, transparent);
  color: var(--ok);
}

.fcv__kind--modify {
  background: color-mix(in srgb, var(--warn) 16%, transparent);
  color: var(--warn);
}

.fcv__kind--delete {
  background: color-mix(in srgb, var(--danger) 14%, transparent);
  color: var(--danger);
}

.fcv__path {
  flex: 1 1 auto;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-family: var(--font-mono);
  font-size: 0.75rem;
  color: var(--text);
}

.fcv__stats {
  flex: 0 0 auto;
  display: inline-flex;
  gap: 0.25rem;
  font-family: var(--font-mono);
  font-size: 0.6875rem;
  font-variant-numeric: tabular-nums;
}

.fcv__added { color: var(--ok); }
.fcv__removed { color: var(--danger); }

.fcv__chevron {
  flex: 0 0 auto;
  color: var(--text-dim);
}

.fcv__open {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  flex: 0 0 auto;
  width: 1.75rem;
  height: 1.75rem;
  margin-right: 0.25rem;
  border: 0;
  border-radius: var(--radius-sm);
  background: transparent;
  color: var(--text-dim);
  cursor: pointer;
}

.fcv__open:hover {
  background: var(--bg-hover);
  color: var(--accent);
}

.fcv__diff {
  max-height: 20rem;
  overflow: auto;
  border-top: 1px solid var(--hairline);
  background: var(--bg);
  font-family: var(--font-mono);
  font-size: 0.6875rem;
  line-height: 1.5;
}

.fcv__line {
  display: flex;
  min-width: max-content;
}

.fcv__gutter {
  flex: 0 0 auto;
  width: 0.25rem;
  background: transparent;
}

.fcv__line--add {
  background: color-mix(in srgb, var(--ok) 10%, transparent);
  color: color-mix(in srgb, var(--ok) 80%, var(--text));
}

.fcv__line--del {
  background: color-mix(in srgb, var(--danger) 10%, transparent);
  color: color-mix(in srgb, var(--danger) 80%, var(--text));
}

.fcv__line--hunk {
  background: color-mix(in srgb, var(--text-muted) 10%, transparent);
  color: var(--text-muted);
}

.fcv__line--meta {
  color: var(--text-dim);
}

.fcv__line--context {
  color: var(--text-muted);
}

/* 改动处（+ / -）用黄色 gutter 标出，hunk 头保持中性。 */
.fcv__line--add .fcv__gutter,
.fcv__line--del .fcv__gutter {
  background: color-mix(in srgb, var(--warn) 72%, transparent);
}

.fcv__line--hunk .fcv__gutter {
  background: color-mix(in srgb, var(--text-muted) 30%, transparent);
}

.fcv__code {
  padding: 0 0.5rem;
  white-space: pre;
}
</style>
