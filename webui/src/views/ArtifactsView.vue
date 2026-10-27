<template>
  <div class="control-stage artifacts-stage">
    <header class="control-stage__header">
      <div class="control-stage__title-block">
        <h1 class="control-stage__title">{{ t('artifacts.title') }}</h1>
        <p class="control-stage__subtitle">{{ t('artifacts.subtitle') }}</p>
      </div>
      <div class="control-stage__actions">
        <span class="artifacts-count">{{ t('artifacts.count', { count: total }) }}</span>
        <button class="btn btn--ghost" :disabled="busy" @click="reload">
          <Icon name="refresh" :size="16" /><span>{{ t('artifacts.refresh') }}</span>
        </button>
      </div>
    </header>

    <p v-if="note" class="artifacts-note">{{ note }}</p>

    <section class="artifacts-card">
      <p v-if="!items.length && !busy && !note" class="artifacts-empty">{{ t('artifacts.empty') }}</p>
      <ul v-else class="artifacts-list">
        <li v-for="a in items" :key="a.id" class="artifacts-row">
          <span class="artifacts-row__icon" aria-hidden="true">
            <Icon :name="iconFor(a.name)" :size="16" />
          </span>
          <span class="artifacts-row__main">
            <span class="artifacts-row__name" :title="a.path">{{ a.name }}</span>
            <span class="artifacts-row__meta">
              <span>{{ formatSize(a.size) }}</span>
              <span v-if="a.session_key" class="artifacts-row__session">{{ shortSession(a.session_key) }}</span>
              <span v-if="!a.present" class="artifacts-badge artifacts-badge--warn">{{ t('artifacts.missing') }}</span>
            </span>
            <span class="artifacts-row__path">{{ a.path }}</span>
          </span>
          <span class="artifacts-row__actions">
            <button class="btn btn--sm btn--ghost" :disabled="!a.present" @click="open(a)">
              <Icon name="externalLink" :size="14" /><span>{{ t('artifacts.open') }}</span>
            </button>
            <button class="btn btn--sm btn--ghost" @click="copyPath(a)">
              <Icon :name="copiedId === a.id ? 'check' : 'copy'" :size="14" />
              <span>{{ copiedId === a.id ? t('artifacts.copied') : t('artifacts.copy') }}</span>
            </button>
          </span>
        </li>
      </ul>

      <div v-if="hasMore" class="artifacts-more">
        <button class="btn btn--ghost" :disabled="busy" @click="loadMore">
          <Icon name="chevronDown" :size="15" /><span>{{ t('artifacts.loadMore') }}</span>
        </button>
      </div>
    </section>
  </div>
</template>

<script setup lang="ts">
/**
 * 我的文件（产物中心）—— 跨会话聚合。
 *
 * 数据源：`artifacts.list { allSessions: true }`（游标分页，按时间倒序）。
 * 每条都带 `present`（后端回查磁盘），因此「登记过但文件已不在」会显式标出来，
 * 而不是点开才报错。打开走 `artifacts.open_in_system`，复制走剪贴板降级工具。
 *
 * 诚实边界：这里只展示**台账登记过**的产物；未登记的磁盘文件不会凭空出现。
 */
import { computed, onMounted, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import Icon from '@/components/Icon.vue'
import type { IconName } from '@/utils/icons'
import { useRpcStore } from '@/stores/rpc'
import { copyTextWithFallback } from '@/utils/browser'

interface ArtifactRow {
  id: string
  name: string
  path: string
  kind: string
  size: number
  ts: number
  mime?: string
  session_key?: string
  present?: boolean
}
interface ListPayload {
  artifacts?: ArtifactRow[]
  has_more?: boolean
  next_cursor?: number | null
  error?: string
}

const PAGE_SIZE = 50

const { t } = useI18n()
const rpc = useRpcStore()

const items = ref<ArtifactRow[]>([])
const cursor = ref<number | null>(null)
const hasMore = ref(false)
const busy = ref(false)
const note = ref('')
const copiedId = ref('')

const total = computed(() => items.value.length)

function formatSize(size: number): string {
  if (!Number.isFinite(size) || size < 0) return '—'
  if (size < 1024) return `${size} B`
  if (size < 1024 * 1024) return `${(size / 1024).toFixed(1)} KB`
  return `${(size / 1024 / 1024).toFixed(1)} MB`
}

function shortSession(key: string): string {
  return key.length > 18 ? `${key.slice(0, 8)}…${key.slice(-6)}` : key
}

/** 图标按扩展名归类：只做展示提示，不影响可打开性。 */
function iconFor(name: string): IconName {
  const ext = name.slice(name.lastIndexOf('.') + 1).toLowerCase()
  if (['png', 'jpg', 'jpeg', 'gif', 'webp', 'svg'].includes(ext)) return 'image'
  if (['md', 'txt', 'pdf', 'docx'].includes(ext)) return 'fileText'
  if (['csv', 'xlsx', 'xls'].includes(ext)) return 'table'
  if (['js', 'ts', 'vue', 'rs', 'py', 'json', 'html', 'css'].includes(ext)) return 'fileCode'
  return 'fileText'
}

async function fetchPage(reset: boolean) {
  busy.value = true
  if (reset) note.value = ''
  try {
    if (!rpc.isConnected) await rpc.waitForConnection(8_000)
    const res = await rpc.call<ListPayload>('artifacts.list', {
      allSessions: true,
      limit: PAGE_SIZE,
      ...(reset || cursor.value === null ? {} : { cursor: cursor.value }),
    })
    if (res.error) {
      note.value = res.error
      return
    }
    const page = res.artifacts ?? []
    items.value = reset ? page : [...items.value, ...page]
    hasMore.value = res.has_more === true
    cursor.value = res.next_cursor ?? null
  } catch (e) {
    note.value = e instanceof Error ? e.message : String(e)
  } finally {
    busy.value = false
  }
}

function reload() {
  cursor.value = null
  void fetchPage(true)
}

function loadMore() {
  if (!hasMore.value) return
  void fetchPage(false)
}

async function open(a: ArtifactRow) {
  try {
    const res = await rpc.call<{ ok?: boolean; error?: string }>('artifacts.open_in_system', {
      id: a.id,
    })
    if (res && res.error) note.value = res.error
  } catch (e) {
    note.value = e instanceof Error ? e.message : String(e)
  }
}

async function copyPath(a: ArtifactRow) {
  try {
    await copyTextWithFallback(a.path)
    copiedId.value = a.id
    window.setTimeout(() => {
      if (copiedId.value === a.id) copiedId.value = ''
    }, 1600)
  } catch (e) {
    note.value = e instanceof Error ? e.message : String(e)
  }
}

onMounted(() => void fetchPage(true))
</script>

<style scoped>
.artifacts-count {
  color: var(--text-dim);
  font-size: var(--fs-sm);
}
.artifacts-note {
  margin: 0 0 12px;
  padding: 8px 10px;
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  color: var(--text-muted);
  font-size: var(--fs-sm);
}
.artifacts-card {
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  background: var(--bg-surface);
  padding: 8px;
}
.artifacts-empty {
  margin: 0;
  padding: 24px 8px;
  color: var(--text-dim);
  font-size: var(--fs-sm);
  text-align: center;
}
.artifacts-list {
  margin: 0;
  padding: 0;
  list-style: none;
}
.artifacts-row {
  display: flex;
  gap: 10px;
  align-items: flex-start;
  padding: 10px 8px;
  border-bottom: 1px solid var(--border);
}
.artifacts-row:last-child {
  border-bottom: 0;
}
.artifacts-row__icon {
  display: inline-flex;
  padding-top: 2px;
  color: var(--text-dim);
}
.artifacts-row__main {
  display: flex;
  flex: 1;
  flex-direction: column;
  gap: 3px;
  min-width: 0;
}
.artifacts-row__name {
  overflow: hidden;
  color: var(--text);
  font-size: var(--fs-sm);
  font-weight: 600;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.artifacts-row__meta {
  display: flex;
  flex-wrap: wrap;
  gap: 8px;
  align-items: center;
  color: var(--text-dim);
  font-size: var(--fs-xs);
}
.artifacts-row__session {
  font-family: var(--font-mono);
}
.artifacts-row__path {
  overflow: hidden;
  color: var(--text-dim);
  font-family: var(--font-mono);
  font-size: var(--fs-xs);
  text-overflow: ellipsis;
  white-space: nowrap;
}
.artifacts-row__actions {
  display: flex;
  gap: 6px;
  flex-shrink: 0;
}
.artifacts-badge {
  padding: 1px 6px;
  border-radius: var(--radius-sm);
  background: var(--bg-hover);
  font-size: var(--fs-xs);
}
.artifacts-badge--warn {
  background: color-mix(in srgb, var(--warn) 16%, var(--bg-surface));
  color: var(--warn);
}
.artifacts-more {
  display: flex;
  justify-content: center;
  padding: 10px 0 4px;
}
@media (max-width: 720px) {
  .artifacts-row {
    flex-wrap: wrap;
  }
  .artifacts-row__actions {
    width: 100%;
    justify-content: flex-end;
  }
}
</style>
