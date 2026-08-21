<template>
  <!-- 右侧侧边栏：tasks 轴 + 历史产物 + 文件更改 -->
  <Transition name="rs-slide">
    <aside v-if="open" class="rs-panel" :class="{ 'rs-panel--wide': wide }">
      <div class="rs-tabs">
        <button
          v-for="tab in tabs"
          :key="tab.id"
          type="button"
          class="rs-tab"
          :class="{ 'rs-tab--active': activeTab === tab.id }"
          @click="activeTab = tab.id"
        >
          <Icon :name="tab.icon" :size="14" />
          <span>{{ tab.label }}</span>
          <span v-if="tab.count > 0" class="rs-tab__badge">{{ tab.count }}</span>
        </button>
        <div class="rs-tabs__spacer" />
        <button
          type="button"
          class="rs-icon-btn"
          :title="wide ? '窄栏' : '宽栏'"
          @click="wide = !wide"
        >
          <Icon :name="wide ? 'panel-right-open' : 'panel-right-close'" :size="14" />
        </button>
        <button
          type="button"
          class="rs-icon-btn"
          title="折叠侧边栏"
          @click="emit('close')"
        >
          <Icon name="x" :size="14" />
        </button>
      </div>

      <div class="rs-body">
        <!-- Tasks 时间轴 -->
        <div v-if="activeTab === 'tasks'" class="rs-section">
          <div v-if="tasks.length === 0" class="rs-empty">
            <Icon name="listChecks" :size="28" :opacity="0.3" />
            <p>发送消息后，任务步骤将实时显示在这里</p>
          </div>
          <ol v-else class="rs-timeline">
            <li
              v-for="(task, i) in tasks"
              :key="i"
              class="rs-timeline__item"
              :class="`rs-timeline__item--${task.status}`"
            >
              <span class="rs-timeline__dot" />
              <div class="rs-timeline__content">
                <div class="rs-timeline__title">{{ task.title }}</div>
                <div v-if="task.detail" class="rs-timeline__detail">{{ task.detail }}</div>
                <div v-if="task.tools && task.tools.length > 0" class="rs-timeline__tools">
                  <span v-for="tool in task.tools" :key="tool" class="rs-tool-chip">
                    <Icon name="play" :size="10" />{{ tool }}
                  </span>
                </div>
                <div v-if="task.duration" class="rs-timeline__time">{{ task.duration }}</div>
              </div>
            </li>
          </ol>
        </div>

        <!-- 历史产物 -->
        <div v-if="activeTab === 'artifacts'" class="rs-section">
          <!-- 诊断条：本次会话已有产物但未载入侧边栏（重连后、索引尚未就绪等）。
               作为兜底入口，避免「生成了文件却看不到」的死路。 -->
          <div v-if="diagnosticList.length > 0" class="rs-diagnostic">
            <Icon name="alertTriangle" :size="14" />
            <span>检测到 {{ diagnosticList.length }} 个未载入产物</span>
            <button type="button" class="rs-diagnostic__retry" @click="emit('retryArtifacts')">
              <Icon name="refresh" :size="12" />重试
            </button>
          </div>

          <div v-if="artifacts.length === 0" class="rs-empty rs-empty--artifacts">
            <Icon name="fileStack" :size="28" :opacity="0.3" />
            <p class="rs-empty__lead">本次会话还没有生成文件</p>
            <p class="rs-empty__sub">发送任务，让 Marguerite 为你产出文档、表格或代码</p>
            <div class="rs-empty__actions">
              <button type="button" class="rs-empty__btn" @click="emit('compose')">
                <Icon name="pencil" :size="13" />发送任务
              </button>
              <button type="button" class="rs-empty__btn rs-empty__btn--ghost" @click="emit('retryArtifacts')">
                <Icon name="refresh" :size="13" />重新载入
              </button>
            </div>
          </div>

          <ul v-else class="rs-artifact-list">
            <li v-for="a in artifacts" :key="artifactKey(a)" class="rs-artifact">
              <span
                class="rs-artifact__type-dot"
                :class="`rs-artifact__type-dot--${statusFor(a)}`"
                :title="statusTitle(a)"
                aria-hidden="true"
              />
              <Icon :name="classify(a).icon" :size="16" :style="{ color: classify(a).colorVar }" />
              <div class="rs-artifact__info">
                <div class="rs-artifact__name" :title="a.name || a.path">{{ a.name || artifactBasename(a.path) }}</div>
                <div class="rs-artifact__path">{{ a.path || formatLabel(a) }}</div>
              </div>
              <span class="rs-artifact__format" :style="{ color: classify(a).colorVar }">
                {{ formatLabel(a) }}
              </span>
              <button
                type="button"
                class="rs-artifact__btn"
                :title="'在 Marguerite 内预览 / 编辑'"
                @click="emit('openArtifact', a)"
              >
                <Icon name="edit" :size="13" />
              </button>
              <button
                type="button"
                class="rs-artifact__btn"
                :title="'用系统程序打开'"
                @click="emit('openArtifactSystem', a)"
              >
                <Icon name="appWindow" :size="13" />
              </button>
              <div class="rs-artifact__menu-wrap">
                <button
                  type="button"
                  class="rs-artifact__btn"
                  :title="'更多操作'"
                  aria-haspopup="menu"
                  :aria-expanded="openMenuKey === artifactKey(a)"
                  @click.stop="toggleMenu(a)"
                >
                  <Icon name="moreHorizontal" :size="14" />
                </button>
                <ul v-if="openMenuKey === artifactKey(a)" class="rs-artifact__menu" role="menu">
                  <li role="none">
                    <button type="button" role="menuitem" @click="previewArtifact(a)">
                      <Icon name="eye" :size="14" /><span>预览</span>
                    </button>
                  </li>
                  <li role="none">
                    <button type="button" role="menuitem" @click="editArtifact(a)">
                      <Icon name="edit" :size="14" /><span>编辑</span>
                    </button>
                  </li>
                  <li role="none">
                    <button type="button" role="menuitem" @click="openArtifactSystemAction(a)">
                      <Icon name="appWindow" :size="14" /><span>系统打开</span>
                    </button>
                  </li>
                  <li role="none">
                    <button type="button" role="menuitem" @click="copyArtifactPath(a)">
                      <Icon name="copy" :size="14" /><span>复制路径</span>
                    </button>
                  </li>
                </ul>
              </div>
            </li>
          </ul>
        </div>

        <!-- 文件更改 -->
        <div v-if="activeTab === 'changes'" class="rs-section">
          <div v-if="changes.length === 0" class="rs-empty">
            <Icon name="fork" :size="28" :opacity="0.3" />
            <p>文件更改（+/- 行数）将显示在这里</p>
          </div>
          <ul v-else class="rs-change-list">
            <li v-for="c in changes" :key="c.path" class="rs-change">
              <Icon :name="changeIcon(c)" :size="14" />
              <span class="rs-change__path" :title="c.path">{{ c.name || c.path }}</span>
              <span v-if="c.additions > 0" class="rs-change__add">+{{ c.additions }}</span>
              <span v-if="c.deletions > 0" class="rs-change__del">-{{ c.deletions }}</span>
            </li>
          </ul>
        </div>

        <!-- 文件树 -->
        <div v-if="activeTab === 'tree'" class="rs-section">
          <div class="rs-tree-toolbar">
            <button type="button" class="rs-icon-btn" title="上级目录" :disabled="treeStack.length === 0" @click="goUpTree">
              <Icon name="chevronLeft" :size="14" />
            </button>
            <span class="rs-tree-path" :title="treePath">{{ treePath || '工作区根目录' }}</span>
            <button type="button" class="rs-icon-btn" title="刷新" @click="loadTree(treePath)">
              <Icon name="refresh" :size="14" />
            </button>
          </div>
          <div v-if="treeError" class="rs-diagnostic">
            <Icon name="alertTriangle" :size="14" />
            <span>{{ treeError }}</span>
          </div>
          <div v-if="treeLoading" class="rs-empty">
            <p>加载中…</p>
          </div>
          <ul v-else-if="treeEntries.length > 0" class="rs-tree-list">
            <li v-for="e in treeEntries" :key="e.path" class="rs-tree-item">
              <button
                type="button"
                class="rs-tree-row"
                :class="{ 'rs-tree-row--file': e.kind === 'file' }"
                :title="e.path"
                @click="enterTree(e)"
              >
                <Icon
                  :name="e.kind === 'directory' ? 'folder' : 'fileText'"
                  :size="14"
                  :style="{ color: e.kind === 'directory' ? 'var(--accent)' : 'var(--text-muted)' }"
                />
                <span class="rs-tree-name">{{ e.name }}</span>
              </button>
            </li>
          </ul>
          <div v-else-if="!treeLoading" class="rs-empty">
            <Icon name="folder" :size="28" :opacity="0.3" />
            <p>（空目录）</p>
          </div>
        </div>
      </div>
    </aside>
  </Transition>

  <!-- 折叠按钮（侧边栏关闭时显示在右边缘） -->
  <button
    v-if="!open"
    type="button"
    class="rs-toggle"
    title="展开侧边栏"
    @click="emit('open')"
  >
    <Icon name="panel-right-open" :size="16" />
  </button>
</template>

<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch } from 'vue'
import Icon from '@/components/Icon.vue'
import type { IconName } from '@/utils/icons'
import type { ArtifactPayload, SandboxPathEntry } from '@/types/rpc'
import { classifyArtifact, type ArtifactKindDescriptor } from '@/utils/artifactKind'
import { useToasts } from '@/composables/useToasts'
import { useRpcStore } from '@/stores/rpc'

export interface RsTask {
  title: string
  detail?: string
  status: 'running' | 'done' | 'error' | 'pending'
  tools?: string[]
  duration?: string
}

export type RsArtifact = ArtifactPayload & { openable?: boolean }

export interface RsChange {
  path: string
  name?: string
  additions: number
  deletions: number
  kind?: 'created' | 'modified' | 'deleted'
}

const props = defineProps<{
  open: boolean
  tasks: RsTask[]
  artifacts: RsArtifact[]
  changes: RsChange[]
  /** 诊断：有产物但未载入侧边栏（如重连后索引未就绪）。 */
  diagnostics?: string[]
  /** 是否正在流式输出（当前 turn 实时写入产物）。 */
  streaming?: boolean
  /** 流式期间经 session.event.artifact 出现的产物 key 集合。 */
  liveKeys?: string[]
  /** 后端已有更新版本、侧边栏仍指向旧内容的产物 key 集合。 */
  staleKeys?: string[]
}>()

const emit = defineEmits<{
  close: []
  open: []
  openArtifact: [a: RsArtifact]
  openArtifactSystem: [a: RsArtifact]
  retryArtifacts: []
  compose: []
}>()

const activeTab = ref<'tasks' | 'artifacts' | 'changes' | 'tree'>('tasks')
// Persist width across mounts so navigating away and back keeps the chosen
// layout instead of resetting to the narrow column.
const RIGHT_SIDEBAR_WIDE_KEY = 'marguerite.chat.rightSidebarWide'
const wide = ref(localStorage.getItem(RIGHT_SIDEBAR_WIDE_KEY) === '1')
watch(wide, (value) => {
  try { localStorage.setItem(RIGHT_SIDEBAR_WIDE_KEY, value ? '1' : '0') } catch { /* ignore */ }
})

const tabs = computed(() => [
  { id: 'tasks' as const, label: '任务', icon: 'listChecks' as IconName, count: props.tasks.filter(t => t.status === 'running').length },
  { id: 'artifacts' as const, label: '产物', icon: 'fileText' as IconName, count: props.artifacts.length },
  { id: 'changes' as const, label: '更改', icon: 'fork' as IconName, count: props.changes.length },
  { id: 'tree' as const, label: '树', icon: 'listTree' as IconName, count: 0 },
])

// ── 文件树（工作区目录浏览，数据源 sandbox.path.list）──
const rpc = useRpcStore()
const treePath = ref('')
const treeEntries = ref<SandboxPathEntry[]>([])
const treeLoading = ref(false)
const treeError = ref('')
const treeStack = ref<string[]>([])

async function loadTree(path: string) {
  treeLoading.value = true
  treeError.value = ''
  try {
    const res = await rpc.call<{ entries: SandboxPathEntry[] }>('sandbox.path.list', { path })
    treeEntries.value = (res.entries || []).filter((e) => !e.hidden)
    treePath.value = path
  } catch (e) {
    treeError.value = e instanceof Error ? e.message : String(e)
  } finally {
    treeLoading.value = false
  }
}

function enterTree(entry: SandboxPathEntry) {
  if (entry.kind !== 'directory') return
  treeStack.value.push(treePath.value)
  void loadTree(entry.path)
}

function goUpTree() {
  const parent = treeStack.value.pop()
  void loadTree(parent ?? '')
}

watch(
  () => activeTab.value,
  (tab) => {
    if (tab === 'tree' && treeEntries.value.length === 0 && !treeLoading.value) {
      void loadTree('')
    }
  },
)

/** Stable key: prefer id, then download_url, then path+name. */
function artifactKey(a: RsArtifact): string {
  return String(a.id || a.download_url || `${a.path}|${a.name}`)
}

function artifactBasename(path?: string): string {
  const tail = String(path || '').split(/[\\/]/).pop() || 'file'
  return tail.split(/[?#]/)[0]
}

/**
 * Diagnostics arrive optional from the parent; normalise once so the template
 * never has to guard, and an absent prop reads as "nothing wrong" rather than
 * throwing on `.length`.
 */
const diagnosticList = computed(() => props.diagnostics ?? [])

/** Cache the classifier per artifact so the template's triple call stays cheap. */
const classifyCache = new WeakMap<object, ArtifactKindDescriptor>()
function classify(a: RsArtifact): ArtifactKindDescriptor {
  const cached = classifyCache.get(a)
  if (cached) return cached
  const name = a.name || artifactBasename(a.path || '')
  const result = classifyArtifact(name, a.mime || a.content_type)
  classifyCache.set(a, result)
  return result
}

function formatLabel(a: RsArtifact): string {
  const d = classify(a)
  if (d.format) return d.format
  return d.label
}

function changeIcon(c: RsChange): IconName {
  if (c.kind === 'created') return 'plus'
  if (c.kind === 'deleted') return 'trash'
  return 'edit'
}

const { pushToast } = useToasts()

type ArtifactStatus = 'ready' | 'updating' | 'stale'

// 用 Set 包裹父级传入的 key 集合，把每行状态查询降到 O(1)。
const liveKeysSet = computed(() => new Set(props.liveKeys ?? []))
const staleKeysSet = computed(() => new Set(props.staleKeys ?? []))

/** 派生单行的状态点语义：更新中 > 已过期 > 就绪。 */
function statusFor(a: RsArtifact): ArtifactStatus {
  if (props.streaming && liveKeysSet.value.has(artifactKey(a))) return 'updating'
  if (staleKeysSet.value.has(artifactKey(a))) return 'stale'
  return 'ready'
}

const STATUS_TITLE: Record<ArtifactStatus, string> = {
  ready: '就绪',
  updating: '更新中',
  stale: '已过期',
}
function statusTitle(a: RsArtifact): string {
  return STATUS_TITLE[statusFor(a)]
}

// 「更多」菜单：同一时刻仅允许展开一行，点击外部或按 Escape 收起。
const openMenuKey = ref<string | null>(null)
function toggleMenu(a: RsArtifact) {
  const key = artifactKey(a)
  openMenuKey.value = openMenuKey.value === key ? null : key
}
function closeMenu() {
  openMenuKey.value = null
}
function onDocumentClick(e: MouseEvent) {
  // 点击落在某行菜单容器内时不关闭，交由各菜单项自行处理。
  const target = e.target as HTMLElement | null
  if (target && target.closest('.rs-artifact__menu-wrap')) return
  closeMenu()
}
function onDocumentKeydown(e: KeyboardEvent) {
  if (e.key === 'Escape') closeMenu()
}
onMounted(() => {
  document.addEventListener('click', onDocumentClick)
  document.addEventListener('keydown', onDocumentKeydown)
})
onUnmounted(() => {
  document.removeEventListener('click', onDocumentClick)
  document.removeEventListener('keydown', onDocumentKeydown)
})

function previewArtifact(account: RsArtifact) {
  emit('openArtifact', account)
  closeMenu()
}
function editArtifact(account: RsArtifact) {
  emit('openArtifact', account)
  closeMenu()
}
function openArtifactSystemAction(account: RsArtifact) {
  emit('openArtifactSystem', account)
  closeMenu()
}
function copyArtifactPath(account: RsArtifact) {
  const text = account.path || account.name || ''
  if (!text) {
    closeMenu()
    return
  }
  navigator.clipboard.writeText(text).then(
    () => pushToast('已复制产物路径', { tone: 'ok', duration: 1600 }),
    () => pushToast('复制失败，请手动复制路径', { tone: 'danger', duration: 2000 }),
  )
  closeMenu()
}
</script>

<style scoped>
.rs-toggle {
  position: fixed;
  right: 0;
  top: 50%;
  transform: translateY(-50%);
  z-index: 90;
  display: flex;
  align-items: center;
  justify-content: center;
  width: 28px;
  height: 56px;
  border: 1px solid var(--border);
  border-right: none;
  border-radius: var(--radius-md) 0 0 var(--radius-md);
  background: var(--bg-elevated);
  color: var(--text-muted);
  cursor: pointer;
  transition: background var(--dur-fast, 120ms) var(--ease-standard), color var(--dur-fast, 120ms) var(--ease-standard);
}
.rs-toggle:hover {
  background: var(--bg-hover);
  color: var(--text);
}

.rs-panel {
  position: fixed;
  right: 0;
  top: 0;
  bottom: 0;
  width: 300px;
  z-index: 95;
  display: flex;
  flex-direction: column;
  background: var(--bg-elevated);
  border-left: 1px solid var(--border);
  box-shadow: -4px 0 24px color-mix(in srgb, var(--bg) 50%, transparent);
}
.rs-panel--wide { width: 420px; }

.rs-tabs {
  display: flex;
  align-items: center;
  gap: 2px;
  padding: 8px 8px 0;
  border-bottom: 1px solid var(--border);
}
.rs-tab {
  display: flex;
  align-items: center;
  gap: 5px;
  padding: 6px 10px;
  border: none;
  border-radius: var(--radius-sm) var(--radius-sm) 0 0;
  background: transparent;
  color: var(--text-muted);
  font-size: 12px;
  cursor: pointer;
  transition: background var(--dur-fast, 120ms), color var(--dur-fast, 120ms);
}
.rs-tab:hover { background: var(--bg-hover); color: var(--text); }
.rs-tab--active {
  color: var(--accent);
  border-bottom: 2px solid var(--accent);
}
.rs-tab__badge {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  min-width: 16px;
  height: 16px;
  padding: 0 4px;
  border-radius: var(--radius-md);
  background: var(--accent);
  color: var(--text);
  font-size: 10px;
  font-weight: 600;
}
.rs-tabs__spacer { flex: 1; }
.rs-icon-btn {
  display: flex; align-items: center; justify-content: center;
  width: 26px; height: 26px;
  border: none; border-radius: var(--radius-sm);
  background: transparent; color: var(--text-muted);
  cursor: pointer; transition: background var(--dur-fast, 120ms);
}
.rs-icon-btn:hover { background: var(--bg-hover); color: var(--text); }

.rs-body { flex: 1; overflow-y: auto; padding: 8px; }
.rs-section { display: flex; flex-direction: column; gap: 6px; }

.rs-empty {
  display: flex; flex-direction: column; align-items: center; justify-content: center;
  gap: 8px; padding: 40px 16px; color: var(--text-dim); font-size: 12px; text-align: center;
}
.rs-empty--artifacts { padding: 28px 16px; }
.rs-empty__lead { color: var(--text-muted); font-size: 12px; font-weight: 500; }
.rs-empty__sub { font-size: 11px; line-height: 1.5; max-width: 220px; }
.rs-empty__actions { display: flex; gap: 6px; margin-top: 4px; }
.rs-empty__btn {
  display: inline-flex; align-items: center; gap: 4px;
  padding: 6px 12px; border: 1px solid var(--accent);
  border-radius: var(--radius-md);
  background: var(--accent); color: var(--text);
  font-size: 12px; cursor: pointer;
  transition: background var(--dur-fast, 120ms) var(--ease-standard), transform var(--dur-fast, 120ms) var(--ease-standard);
}
.rs-empty__btn:hover { background: var(--accent-hover); }
.rs-empty__btn:active { transform: translateY(1px); }
.rs-empty__btn--ghost {
  background: transparent; color: var(--text-muted); border-color: var(--border);
}
.rs-empty__btn--ghost:hover { background: var(--bg-hover); color: var(--text); }

.rs-diagnostic {
  display: flex; align-items: center; gap: 6px;
  padding: 6px 10px; border-radius: var(--radius-md);
  background: color-mix(in srgb, var(--warn) 12%, transparent);
  border: 1px solid color-mix(in srgb, var(--warn) 40%, transparent);
  color: var(--text-muted); font-size: 11px;
}
.rs-diagnostic__retry {
  margin-left: auto; display: inline-flex; align-items: center; gap: 3px;
  padding: 2px 8px; border: 1px solid color-mix(in srgb, var(--warn) 50%, transparent);
  border-radius: var(--radius-sm); background: transparent; color: var(--text);
  font-size: 11px; cursor: pointer;
  transition: background var(--dur-fast, 120ms) var(--ease-standard);
}
.rs-diagnostic__retry:hover { background: color-mix(in srgb, var(--warn) 22%, transparent); }

/* Timeline */
.rs-timeline { list-style: none; margin: 0; padding: 0 0 0 16px; position: relative; }
.rs-timeline::before {
  content: ''; position: absolute; left: 5px; top: 4px; bottom: 4px;
  width: 1px; background: var(--border);
}
.rs-timeline__item { position: relative; padding: 6px 0 10px 14px; }
.rs-timeline__dot {
  position: absolute; left: -16px; top: 10px; width: 10px; height: 10px;
  border-radius: 50%; border: 2px solid var(--border);
  background: var(--bg-elevated);
}
.rs-timeline__item--running .rs-timeline__dot {
  border-color: var(--accent);
  background: var(--accent);
  animation: rs-pulse 1.2s ease-in-out infinite;
}
.rs-timeline__item--done .rs-timeline__dot { border-color: var(--ok); background: var(--ok); }
.rs-timeline__item--error .rs-timeline__dot { border-color: var(--danger); background: var(--danger); }
@keyframes rs-pulse { 0%,100% { opacity: 1; } 50% { opacity: 0.4; } }

.rs-timeline__title { font-size: 12px; font-weight: 500; color: var(--text); }
.rs-timeline__detail { font-size: 11px; color: var(--text-muted); margin-top: 2px; word-break: break-word; }
.rs-timeline__tools { display: flex; flex-wrap: wrap; gap: 4px; margin-top: 4px; }
.rs-tool-chip {
  display: inline-flex; align-items: center; gap: 3px;
  padding: 1px 6px; border-radius: var(--radius-xs);
  background: color-mix(in srgb, var(--accent) 12%, transparent); color: var(--accent);
  font-size: 10px; font-family: var(--font-mono, monospace);
}
.rs-timeline__time { font-size: 10px; color: var(--text-dim); margin-top: 3px; }

/* Artifacts */
.rs-artifact-list { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 4px; }
.rs-artifact {
  display: flex; align-items: center; gap: 8px;
  padding: 8px 10px; border-radius: var(--radius-md);
  background: var(--card);
  border: 1px solid var(--border);
  transition: background var(--dur-fast, 120ms) var(--ease-standard), border-color var(--dur-fast, 120ms) var(--ease-standard);
}
.rs-artifact:hover { background: var(--bg-hover); border-color: var(--border-strong); }
.rs-artifact__type-dot {
  width: 6px; height: 6px; border-radius: 50%; flex-shrink: 0;
  background: var(--ok);
  transition: background var(--dur-fast, 120ms) var(--ease-standard);
}
.rs-artifact__type-dot--ready { background: var(--ok); }
.rs-artifact__type-dot--updating {
  background: var(--accent);
  animation: rs-pulse 1.2s ease-in-out infinite;
}
.rs-artifact__type-dot--stale { background: var(--warn); }
.rs-artifact__info { flex: 1; min-width: 0; }
.rs-artifact__name { font-size: 12px; color: var(--text); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
.rs-artifact__path { font-size: 10px; color: var(--text-dim); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
.rs-artifact__format {
  font-size: 10px; font-weight: 600; font-family: var(--font-mono, monospace);
  letter-spacing: 0.04em; flex-shrink: 0;
}
.rs-artifact__btn {
  display: flex; align-items: center; justify-content: center;
  width: 26px; height: 26px; border: none; border-radius: var(--radius-sm);
  background: transparent; color: var(--text-muted); cursor: pointer; flex-shrink: 0;
  transition: background var(--dur-fast, 120ms) var(--ease-standard), color var(--dur-fast, 120ms) var(--ease-standard);
}
.rs-artifact__btn:hover { background: var(--accent); color: var(--text); }
.rs-artifact__btn:focus-visible { outline: none; box-shadow: var(--focus-ring); }

.rs-artifact__menu-wrap { position: relative; flex-shrink: 0; }
.rs-artifact__menu {
  position: absolute; right: 0; top: calc(100% + 4px); z-index: 10;
  min-width: 150px; margin: 0; padding: 4px; list-style: none;
  background: var(--bg-elevated);
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  box-shadow: var(--shadow-lg);
}
.rs-artifact__menu button {
  display: flex; align-items: center; gap: 8px; width: 100%;
  padding: 7px 8px; border: none; border-radius: var(--radius-sm);
  background: transparent; color: var(--text); font-size: 12px; cursor: pointer;
  transition: background var(--dur-fast, 120ms) var(--ease-standard);
}
.rs-artifact__menu button:hover { background: var(--bg-hover); }
.rs-artifact__menu button:focus-visible { outline: none; box-shadow: var(--focus-ring); }

/* Changes */
.rs-change-list { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 2px; }
.rs-change {
  display: flex; align-items: center; gap: 6px;
  padding: 5px 8px; border-radius: var(--radius-sm); font-size: 12px;
}
.rs-change:hover { background: var(--bg-hover); }
.rs-change__path { flex: 1; color: var(--text); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; font-family: var(--font-mono, monospace); font-size: 11px; }
.rs-change__add { color: var(--ok); font-size: 11px; font-weight: 600; font-family: var(--font-mono, monospace); }
.rs-change__del { color: var(--danger); font-size: 11px; font-weight: 600; font-family: var(--font-mono, monospace); }

/* File tree */
.rs-tree-toolbar { display: flex; align-items: center; gap: 6px; padding: 4px 6px; }
.rs-tree-path { flex: 1; font-size: 11px; color: var(--text-muted); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; font-family: var(--font-mono, monospace); }
.rs-tree-list { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 1px; }
.rs-tree-row {
  display: flex; align-items: center; gap: 8px; width: 100%;
  padding: 5px 8px; border: none; border-radius: var(--radius-sm);
  background: transparent; color: var(--text); font-size: 12px; cursor: pointer; text-align: left;
  transition: background var(--dur-fast, 120ms) var(--ease-standard);
}
.rs-tree-row:hover { background: var(--bg-hover); }
.rs-tree-row--file { cursor: default; }
.rs-tree-name { flex: 1; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
.rs-icon-btn:disabled { opacity: 0.4; cursor: default; }

.rs-slide-enter-active, .rs-slide-leave-active { transition: transform var(--dur-normal, 200ms) var(--ease-standard); }
.rs-slide-enter-from, .rs-slide-leave-to { transform: translateX(100%); }

@media (prefers-reduced-motion: reduce) {
  .rs-timeline__item--running .rs-timeline__dot { animation: none; }
  .rs-artifact__type-dot--updating { animation: none; }
  .rs-slide-enter-active, .rs-slide-leave-active { transition: none; }
}
</style>
