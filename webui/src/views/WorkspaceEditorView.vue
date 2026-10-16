<template>
  <div class="control-stage editor-stage">
    <header class="control-stage__header">
      <div class="control-stage__title-block">
        <h1 class="control-stage__title">内置工作区编辑器</h1>
        <p class="control-stage__subtitle">文件树 + 多标签文本编辑（workspace.file.read / write + sandbox.path.list）</p>
      </div>
      <div class="control-stage__actions">
        <button class="btn btn--ghost" :disabled="busy" @click="loadTree(treePath)">
          <Icon name="refresh" :size="16" /><span>刷新</span>
        </button>
        <button class="btn btn--primary" :disabled="busy || tabs.every(t => !t.dirty)" @click="saveAll">
          <Icon name="save" :size="16" /><span>保存全部</span>
        </button>
      </div>
    </header>

    <section v-if="error" class="editor-error">{{ error }}</section>

    <div class="editor-layout">
      <!-- 文件树 -->
      <aside class="editor-tree">
        <div class="editor-tree__toolbar">
          <button class="btn btn--sm btn--ghost" :disabled="treeStack.length === 0" @click="goUp">上级</button>
          <span class="editor-tree__path">{{ treePath || '工作区根' }}</span>
        </div>
        <div v-if="treeLoading" class="editor-empty">加载中…</div>
        <ul v-else class="editor-tree__list">
          <li v-for="e in treeEntries" :key="e.path" class="editor-tree__item">
            <button class="editor-tree__row" :class="{ 'editor-tree__row--file': e.kind === 'file' }" @click="onEntry(e)">
              <Icon :name="e.kind === 'directory' ? 'folder' : 'fileText'" :size="14" />
              <span>{{ e.name }}</span>
              <span v-if="isOpen(e.path)" class="editor-open-dot">●</span>
            </button>
          </li>
        </ul>
        <div v-if="!treeLoading && treeEntries.length === 0" class="editor-empty">（空目录）</div>
      </aside>

      <!-- 多标签编辑器 -->
      <section class="editor-main">
        <div v-if="tabs.length === 0" class="editor-empty editor-empty--big">
          <Icon name="fileText" :size="34" />
          <p>从左侧文件树打开文件开始编辑（多标签）</p>
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
            <span>{{ activeLines }} 行 · {{ activeContent.length }} 字符</span>
            <button class="btn btn--sm btn--primary" :disabled="busy || !activeDirty" @click="saveTab(activePath)">保存</button>
          </div>
        </template>
      </section>
    </div>
  </div>
</template>

<script setup lang="ts">
import { ref, computed, onMounted } from 'vue'
import Icon from '@/components/Icon.vue'
import { useRpcStore } from '@/stores/rpc'

const rpc = useRpcStore()

interface TreeEntry { path: string; name: string; kind: 'file' | 'directory'; hidden?: boolean }
interface ReadResult { ok: boolean; path?: string; content?: string; bytes?: number; error?: string }
interface WriteResult { ok: boolean; path?: string; bytes?: number; error?: string }
interface EditorTab { path: string; name: string; content: string; dirty: boolean }

const error = ref('')
const busy = ref(false)
const treePath = ref('')
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
    const r = await rpc.call<ReadResult>('workspace.file.read', { path })
    if (!r.ok) { error.value = r.error ?? '读取失败'; return }
    tabs.value.push({ path, name: path.split(/[\/]/).pop() || path, content: r.content ?? '', dirty: false })
    activePath.value = path
  } catch (e) {
    error.value = e instanceof Error ? e.message : String(e)
  } finally {
    busy.value = false
  }
}

function markDirty() {
  const t = activeTab.value
  if (t) t.dirty = true
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
  const t = tabs.value.find(x => x.path === path)
  if (!t) return
  busy.value = true
  try {
    const r = await rpc.call<WriteResult>('workspace.file.write', { path, content: t.content })
    if (r.ok) t.dirty = false
    else error.value = r.error ?? '保存失败'
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

onMounted(() => loadTree(''))
</script>

<style scoped>
.editor-stage { padding: 24px 28px; max-width: 1400px; margin: 0 auto; }
.editor-error { margin: 12px 0; padding: 10px 14px; background: color-mix(in srgb, var(--danger) 10%, var(--card)); border: 1px solid color-mix(in srgb, var(--danger) 30%, transparent); border-radius: var(--radius-sm); color: var(--danger); font-size: 12px; }
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
.editor-status { display: flex; gap: 12px; align-items: center; padding: 8px 12px; border-top: 1px solid var(--hairline); font-size: 11px; color: var(--text-dim); }
.editor-status span:first-child { flex: 1; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.editor-empty { padding: 24px; color: var(--text-dim); font-size: 13px; text-align: center; }
.editor-empty--big { display: flex; flex-direction: column; gap: 10px; align-items: center; justify-content: center; min-height: 50vh; }
</style>
