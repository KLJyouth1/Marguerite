<template>
  <div class="control-stage editor-stage" :class="{ 'editor-stage--focus': focusMode }">
    <header class="control-stage__header">
      <div class="control-stage__title-block">
        <h1 class="control-stage__title">{{ t('workspaceEditor.title') }}</h1>
        <p class="control-stage__subtitle">{{ t('workspaceEditor.subtitle') }}</p>
      </div>
      <div class="control-stage__actions">
        <!-- 专注模式：编辑器是大内容面，聚焦时只折叠标题块，保存/刷新必须留在原地 -->
        <button
          class="btn btn--ghost"
          type="button"
          :aria-pressed="focusMode"
          :title="focusMode ? t('chrome.focusModeOff') : t('chrome.focusMode')"
          data-testid="editor-focus-toggle"
          @click="focusMode = !focusMode"
        >
          <Icon :name="focusMode ? 'collapse' : 'expand'" :size="16" />
          <span>{{ focusMode ? t('chrome.focusModeOff') : t('chrome.focusMode') }}</span>
        </button>
        <button class="btn btn--ghost" :disabled="busy" @click="loadTree(treePath)">
          <Icon name="refresh" :size="16" /><span>{{ t('workspaceEditor.refresh') }}</span>
        </button>
        <button class="btn btn--primary" :disabled="busy || tabs.every(t => !t.dirty)" @click="saveAll">
          <Icon name="save" :size="16" /><span>{{ t('workspaceEditor.saveAll') }}</span>
        </button>
      </div>
    </header>

    <!-- 统一错误态组件（含重试） -->
    <ErrorState v-if="error" :message="error" :on-retry="() => loadTree(treePath)" />

    <div class="editor-layout">
      <!-- 文件树 -->
      <aside class="editor-tree">
        <div class="editor-tree__toolbar">
          <button class="btn btn--sm btn--ghost" :disabled="treeStack.length === 0" @click="goUp">{{ t('workspaceEditor.up') }}</button>
          <span class="editor-tree__path">{{ treePath || t('workspaceEditor.root') }}</span>
        </div>
        <div v-if="treeLoading" class="editor-empty">{{ t('workspaceEditor.loading') }}</div>
        <ul v-else class="editor-tree__list">
          <li v-for="e in treeEntries" :key="e.path" class="editor-tree__item">
            <button class="editor-tree__row" :class="{ 'editor-tree__row--file': e.kind === 'file' }" @click="onEntry(e)">
              <Icon :name="e.kind === 'directory' ? 'folder' : 'fileText'" :size="14" />
              <span>{{ e.name }}</span>
              <span v-if="isOpen(e.path)" class="editor-open-dot">●</span>
            </button>
          </li>
        </ul>
        <div v-if="!treeLoading && treeEntries.length === 0" class="editor-empty">{{ t('workspaceEditor.emptyDir') }}</div>
      </aside>

      <!-- 多标签编辑器 -->
      <section class="editor-main">
        <div v-if="tabs.length === 0" class="editor-empty editor-empty--big">
          <Icon name="fileText" :size="34" />
          <p>{{ t('workspaceEditor.openHint') }}</p>
        </div>
        <template v-else>
          <div class="editor-tabs">
            <button
              v-for="t in tabs"
              :key="t.path"
              class="editor-tab"
              :class="{ 'editor-tab--active': t.path === activePath, 'editor-tab--dirty': t.dirty }"
              @click="activePath = t.path"
            >
              <span>{{ t.name }}</span>
              <span class="editor-tab__close" @click.stop="closeTab(t.path)">×</span>
            </button>
          </div>
          <textarea
            v-model="activeContent"
            class="editor-area"
            spellcheck="false"
            @input="markDirty"
          ></textarea>
          <div class="editor-status">
            <span>{{ activePath }}</span>
            <span>{{ t('workspaceEditor.status', { lines: activeLines, chars: activeContent.length }) }}</span>
            <button class="btn btn--sm btn--primary" :disabled="busy || !activeDirty" @click="saveTab(activePath)">{{ t('common.save') }}</button>
          </div>
        </template>
      </section>
    </div>
  </div>
</template>

<script setup lang="ts">
import { ref, computed, onMounted, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { useRoute } from 'vue-router'
import ErrorState from '@/components/ErrorState.vue'
import Icon from '@/components/Icon.vue'
import { useRpcStore } from '@/stores/rpc'

const { t } = useI18n()

const rpc = useRpcStore()
const route = useRoute()

interface TreeEntry { path: string; name: string; kind: 'file' | 'directory'; hidden?: boolean }
interface ReadResult { ok: boolean; path?: string; content?: string; bytes?: number; error?: string }
interface WriteResult { ok: boolean; path?: string; bytes?: number; error?: string }
interface EditorTab { path: string; name: string; content: string; dirty: boolean }

const error = ref('')
const busy = ref(false)
// 专注模式（瞬态，不持久化）：与 TerminalView 同款，只折叠标题块、保留操作钮
const focusMode = ref(false)
const treePath = ref('')
const treeRoot = ref('')
const treeEntries = ref<TreeEntry[]>([])
const treeStack = ref<string[]>([])
const treeLoading = ref(false)
const tabs = ref<EditorTab[]>([])
const activePath = ref('')

const activeTab = computed(() => tabs.value.find(t => t.path === activePath.value))
const activeContent = computed({
  get: () => activeTab.value?.content ?? '',
  set: (v: string) => { if (activeTab.value) activeTab.value.content = v },
})
const activeDirty = computed(() => activeTab.value?.dirty ?? false)
const activeLines = computed(() => activeContent.value.split('\n').length)

async function loadTree(path: string) {
  treeLoading.value = true
  error.value = ''
  try {
    // 冷加载竞态守卫：WS 仍处于 connecting 时先等连接就绪，避免把「未就绪」
    // 当成加载失败，把 `not connected (state: connecting)` 裸串渲染给用户。
    if (!rpc.isConnected) await rpc.waitForConnection(10_000)
    const res = await rpc.call<{ entries: TreeEntry[] }>('sandbox.path.list', { path })
    treeEntries.value = (res.entries || []).filter(e => !e.hidden)
    treePath.value = path
  } catch (e) {
    error.value = e instanceof Error ? e.message : String(e)
  } finally {
    treeLoading.value = false
  }
}

function goUp() {
  const parent = treeStack.value.pop()
  void loadTree(parent ?? '')
}

async function onEntry(e: TreeEntry) {
  if (e.kind === 'directory') {
    treeStack.value.push(treePath.value)
    void loadTree(e.path)
    return
  }
  await openFile(e.path)
}

async function openFile(path: string) {
  const existing = tabs.value.find(t => t.path === path)
  if (existing) { activePath.value = path; return }
  busy.value = true
  error.value = ''
  try {
    // 快速打开来自一个已绑定项目：读写都必须带上同一个 projectPath，
    // 否则后端会按应用数据目录解析，把项目内文件判成「路径超出工作区」。
    const params = treeRoot.value ? { path, projectPath: treeRoot.value } : { path }
    const r = await rpc.call<ReadResult>('workspace.file.read', params)
    if (!r.ok) { error.value = r.error ?? t('workspaceEditor.readFailed'); return }
    tabs.value.push({ path, name: path.split(/[\\/]/).pop() || path, content: r.content ?? '', dirty: false })
    activePath.value = path
  } catch (e) {
    error.value = e instanceof Error ? e.message : String(e)
  } finally {
    busy.value = false
  }
}

function markDirty() {
  const tab = activeTab.value
  if (tab) tab.dirty = true
}

function isOpen(path: string) {
  return tabs.value.some(t => t.path === path)
}

function closeTab(path: string) {
  const i = tabs.value.findIndex(t => t.path === path)
  if (i < 0) return
  const wasActive = path === activePath.value
  tabs.value.splice(i, 1)
  if (wasActive && tabs.value.length) activePath.value = tabs.value[Math.max(0, i - 1)].path
  else if (wasActive) activePath.value = ''
}

async function saveTab(path: string) {
  // 变量名刻意不用单字母 t：它会**遮蔽** useI18n 的 t，使下面那句 i18n 调用
  // 变成「调用标签页对象」而在运行时炸掉（i18n 改造时极易踩到）。
  const tab = tabs.value.find(x => x.path === path)
  if (!tab) return
  busy.value = true
  try {
    const params = treeRoot.value ? { path, content: tab.content, projectPath: treeRoot.value } : { path, content: tab.content }
    const r = await rpc.call<WriteResult>('workspace.file.write', params)
    if (r.ok) tab.dirty = false
    else error.value = r.error ?? t('workspaceEditor.saveFailed')
  } catch (e) {
    error.value = e instanceof Error ? e.message : String(e)
  } finally {
    busy.value = false
  }
}

async function saveAll() {
  for (const t of [...tabs.value]) {
    if (t.dirty) await saveTab(t.path)
  }
}

/**
 * Absolute-looking path on either platform: `/unix/...`, `C:\...`, `C:/...` or
 * a UNC `\\host\share`. Relative names are ignored so a hand-edited query can
 * never make the editor read a path relative to the gateway's CWD.
 */
function looksAbsolutePath(path: string): boolean {
  return /^(?:[A-Za-z]:[\\/]|\/|\\\\)/.test(path)
}

/** Containing directory, preserving the root itself (`/`, `C:\`). */
function directoryOf(path: string): string {
  const trimmed = path.replace(/[\\/]+$/, '')
  const cut = Math.max(trimmed.lastIndexOf('/'), trimmed.lastIndexOf('\\'))
  if (cut < 0) return ''
  if (cut === 0) return trimmed.slice(0, 1)
  const head = trimmed.slice(0, cut)
  // A drive designator must keep its separator (`C:` → `C:\`), otherwise the
  // gateway resolves it as "current directory on drive C".
  return /^[A-Za-z]:$/.test(head) ? trimmed.slice(0, cut + 1) : head
}

/** 快速打开的深链：定位目录 + 打开文件；路径不合法时原样交给调用方回落。 */
async function openPathFromQuery(raw: unknown, rawRoot?: unknown): Promise<boolean> {
  const queryPath = typeof raw === 'string' ? raw.trim() : ''
  if (!queryPath || !looksAbsolutePath(queryPath)) return false
  const queryRoot = typeof rawRoot === 'string' ? rawRoot.trim() : ''
  if (queryRoot && looksAbsolutePath(queryRoot)) treeRoot.value = queryRoot
  const directory = directoryOf(queryPath)
  treePath.value = directory
  await loadTree(directory)
  await openFile(queryPath)
  return true
}

// 已经在 /editor 上再次快速打开另一个文件时，路由不会重挂载本视图，所以查询参数
// 的变化必须自己接住；只响应 `?path=`，其余查询变化一概忽略。
watch(
  () => route.query.path,
  (next) => {
    if (next) void openPathFromQuery(next, route.query.root)
  },
)

onMounted(async () => {
  const queryRoot = typeof route.query.root === 'string' ? route.query.root.trim() : ''
  const hasQueryRoot = !!queryRoot && looksAbsolutePath(queryRoot)
  if (hasQueryRoot) treeRoot.value = queryRoot
  else {
    // 文件树以**工作区根**为起点，而不是平台根（HOME）：`workspace.file.read/write`
    // 只接受工作区内的路径（越界读写会泄露 `~/.ssh` 与网关 token）。若此处仍从 HOME
    // 起浏览，用户会看到一棵「能点但不能打开」的树。
    try {
      const s = await rpc.call<{ root?: string }>('workspace.status', {})
      treeRoot.value = String(s.root || '')
    } catch { /* 取不到根时退回平台根，只影响初始浏览位置 */ }
  }

  // 快速打开（Ctrl/⌘+P）的深链：`?path=<绝对文件路径>` 先把文件树定位到其所在
  // 目录，再把该文件打开成标签页。没有该查询参数时保持原来的「工作区根」落地行为。
  if (await openPathFromQuery(route.query.path, route.query.root)) return

  void loadTree(treeRoot.value)
})
</script>

<style scoped>
.editor-stage { padding: 24px 28px; max-width: 1400px; margin: 0 auto; }
.editor-layout { display: grid; grid-template-columns: 280px 1fr; gap: 14px; margin-top: 16px; align-items: start; }
.editor-tree { background: var(--card); border: 1px solid var(--hairline); border-radius: var(--radius-md); padding: 10px; max-height: 74vh; overflow: auto; }
.editor-tree__toolbar { display: flex; gap: 8px; align-items: center; padding-bottom: 8px; border-bottom: 1px solid var(--hairline); }
.editor-tree__path { font-size: 11px; color: var(--text-dim); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.editor-tree__list { list-style: none; margin: 8px 0 0; padding: 0; }
.editor-tree__row { display: flex; gap: 6px; align-items: center; width: 100%; padding: 5px 6px; background: transparent; border: 0; border-radius: var(--radius-sm); color: var(--text); font-size: 12px; text-align: left; cursor: pointer; }
.editor-tree__row:hover { background: color-mix(in srgb, var(--text) 8%, transparent); }
.editor-tree__row--file { font-family: var(--font-mono); }
.editor-open-dot { margin-left: auto; color: var(--accent); font-size: 10px; }
.editor-main { background: var(--card); border: 1px solid var(--hairline); border-radius: var(--radius-md); min-height: 70vh; display: flex; flex-direction: column; }
.editor-tabs { display: flex; flex-wrap: wrap; gap: 4px; padding: 8px 10px 0; border-bottom: 1px solid var(--hairline); }
.editor-tab { display: inline-flex; gap: 6px; align-items: center; padding: 6px 10px; background: color-mix(in srgb, var(--text) 6%, var(--card)); border: 1px solid var(--hairline); border-bottom: 0; border-radius: var(--radius-sm) var(--radius-sm) 0 0; color: var(--text); font-size: 12px; cursor: pointer; }
.editor-tab--active { background: var(--bg); border-color: var(--accent); }
.editor-tab--dirty::after { content: '•'; color: var(--accent); }
.editor-tab__close { color: var(--text-dim); padding: 0 2px; }
.editor-area { flex: 1; min-height: 60vh; padding: 14px; background: var(--bg); color: var(--text); border: 0; font-family: var(--font-mono); font-size: 13px; line-height: 1.55; resize: none; outline: none; }

/* ── 专注模式：折叠标题块，内容面吃满视口（操作钮保留）── */
.editor-stage--focus .control-stage__title-block { display: none; }
.editor-stage--focus .editor-tree { max-height: calc(100vh - 200px); }
.editor-stage--focus .editor-main { min-height: calc(100vh - 200px); }
.editor-stage--focus .editor-area { min-height: calc(100vh - 250px); }
.editor-status { display: flex; gap: 12px; align-items: center; padding: 8px 12px; border-top: 1px solid var(--hairline); font-size: 11px; color: var(--text-dim); }
.editor-status span:first-child { flex: 1; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.editor-empty { padding: 24px; color: var(--text-dim); font-size: 13px; text-align: center; }
.editor-empty--big { display: flex; flex-direction: column; gap: 10px; align-items: center; justify-content: center; min-height: 50vh; }
</style>
