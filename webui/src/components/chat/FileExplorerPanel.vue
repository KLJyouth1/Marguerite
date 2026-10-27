<!--
  项目树（IDE 式文件资源管理器）。

  设计要点（对齐 Cursor / Trae 的文件面板）：
  · 目录**原地展开**（懒加载子项），不再「点一下跳进一个目录、靠返回键退出」——
    前者是浏览器行为，后者才是 IDE 行为；
  · 单击选中，双击文件在**代码编辑器面板**打开（`requestEditorWorkbenchOpen`），
    目录双击=展开/收起；
  · 右键菜单：打开 / 新建文件 / 新建文件夹 / 重命名 / 删除 / 复制路径 /
    在文件管理器中显示 / 刷新，全部走后端真实 RPC（不是前端假动作）；
  · 路径一律用后端返回的**绝对路径**：`workspace.file.*` 与
    `sandbox.path.*` 都能解析根内绝对路径，避免相对/绝对两套口径互相打架。
-->
<template>
  <div class="fx">
    <div class="fx__toolbar">
      <button type="button" class="fx__icon" :title="t('chat.rightSidebar.treeRefresh')" @click="refresh">
        <Icon name="refresh" :size="13" />
      </button>
      <button type="button" class="fx__icon" :title="t('chat.rightSidebar.explorer.newFile')" @click="askNewFile(rootAbs)">
        <Icon name="fileText" :size="13" />
      </button>
      <button type="button" class="fx__icon" :title="t('chat.rightSidebar.explorer.newFolder')" @click="askNewFolder(rootAbs)">
        <Icon name="folder" :size="13" />
      </button>
      <button type="button" class="fx__icon" :title="t('chat.rightSidebar.explorer.collapseAll')" @click="collapseAll">
        <Icon name="collapse" :size="13" />
      </button>
      <span class="fx__root" :title="rootAbs">{{ rootLabel }}</span>
    </div>

    <p v-if="error" class="fx__error">{{ error }}</p>
    <p v-else-if="!rootAbs" class="fx__hint">{{ t('chat.rightSidebar.explorer.loading') }}</p>

    <ul v-else class="fx__list" role="tree" :aria-label="t('chat.rightSidebar.tabs.tree')" @contextmenu.prevent>
      <li
        v-for="row in visibleRows"
        :key="row.entry.path"
        class="fx__row"
        :class="{
          'is-selected': selected === row.entry.path,
          'is-dir': row.entry.kind === 'directory',
        }"
        :style="{ paddingInlineStart: `${6 + row.depth * 14}px` }"
        role="treeitem"
        :aria-level="row.depth + 1"
        :aria-expanded="row.entry.kind === 'directory' ? expanded.has(row.entry.path) : undefined"
        :title="row.entry.path"
        @click="onRowClick(row.entry)"
        @dblclick="onRowDblClick(row.entry)"
        @contextmenu.prevent.stop="openMenu($event, row.entry)"
        @keydown.enter.prevent="onRowDblClick(row.entry)"
      >
        <Icon
          v-if="row.entry.kind === 'directory'"
          :name="expanded.has(row.entry.path) ? 'chevronDown' : 'chevronRight'"
          :size="11"
          class="fx__chevron"
          aria-hidden="true"
        />
        <span v-else class="fx__chevron" aria-hidden="true" />
        <Icon
          :name="row.entry.kind === 'directory' ? 'folder' : fileIcon(row.entry.name)"
          :size="13"
          class="fx__kind"
          :class="{ 'is-dir': row.entry.kind === 'directory' }"
          aria-hidden="true"
        />
        <span class="fx__name">{{ row.entry.name }}</span>
        <span v-if="loadingDirs.has(row.entry.path)" class="fx__spinner" aria-hidden="true">…</span>
      </li>
      <li v-if="!visibleRows.length && !loading" class="fx__empty">
        {{ t('chat.rightSidebar.treeEmptyDir') }}
      </li>
    </ul>

    <p class="fx__foot">{{ t('chat.rightSidebar.explorer.openHint') }}</p>

    <!-- 右键菜单：Teleport + fixed，避免被侧栏滚动容器裁切 -->
    <Teleport to="body">
      <div
        v-if="menu"
        ref="menuEl"
        class="fx-menu"
        :style="{ top: `${menu.y}px`, left: `${menu.x}px` }"
        role="menu"
        @contextmenu.prevent
      >
        <button v-if="menu.entry.kind === 'file'" type="button" class="fx-menu__item" @click="menuOpen">
          <Icon name="fileCode" :size="13" /><span>{{ t('chat.rightSidebar.explorer.open') }}</span>
        </button>
        <button v-if="menu.entry.kind === 'directory'" type="button" class="fx-menu__item" @click="menuNewFile">
          <Icon name="fileText" :size="13" /><span>{{ t('chat.rightSidebar.explorer.newFile') }}</span>
        </button>
        <button v-if="menu.entry.kind === 'directory'" type="button" class="fx-menu__item" @click="menuNewFolder">
          <Icon name="folder" :size="13" /><span>{{ t('chat.rightSidebar.explorer.newFolder') }}</span>
        </button>
        <div class="fx-menu__sep" />
        <button type="button" class="fx-menu__item" @click="menuRename">
          <Icon name="pencil" :size="13" /><span>{{ t('common.rename') }}</span>
        </button>
        <button type="button" class="fx-menu__item is-danger" @click="menuDelete">
          <Icon name="trash" :size="13" /><span>{{ t('common.delete') }}</span>
        </button>
        <div class="fx-menu__sep" />
        <button type="button" class="fx-menu__item" @click="menuCopyPath">
          <Icon name="copy" :size="13" /><span>{{ t('chat.rightSidebar.explorer.copyPath') }}</span>
        </button>
        <button type="button" class="fx-menu__item" @click="menuReveal">
          <Icon name="externalLink" :size="13" /><span>{{ t('chat.rightSidebar.explorer.reveal') }}</span>
        </button>
      </div>
    </Teleport>

    <!-- 新建 / 重命名 对话框（一次只开一个） -->
    <Teleport to="body">
      <div v-if="dialog" class="fx-dialog__backdrop" @click.self="closeDialog">
        <form class="fx-dialog" @submit.prevent="submitDialog">
          <h3 class="fx-dialog__title">{{ dialogTitle }}</h3>
          <label class="fx-dialog__label" for="fx-name-input">
            {{ t('chat.rightSidebar.explorer.nameLabel') }}
          </label>
          <input
            id="fx-name-input"
            ref="dialogInput"
            v-model="dialogValue"
            class="fx-dialog__input"
            :placeholder="t('chat.rightSidebar.explorer.namePlaceholder')"
            spellcheck="false"
            autocomplete="off"
          >
          <p v-if="dialogError" class="fx-dialog__error">{{ dialogError }}</p>
          <div class="fx-dialog__actions">
            <button type="button" class="btn btn--sm btn--ghost" @click="closeDialog">
              {{ t('common.cancel') }}
            </button>
            <button type="submit" class="btn btn--sm btn--primary" :disabled="dialogBusy || !dialogValue.trim()">
              {{ dialogBusy ? t('chat.rightSidebar.explorer.working') : dialogPrimary }}
            </button>
          </div>
        </form>
      </div>
    </Teleport>
  </div>
</template>

<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import Icon from '@/components/Icon.vue'
import type { IconName } from '@/utils/icons'
import { useConfirm } from '@/composables/useConfirm'
import { useToasts } from '@/composables/useToasts'
import { useRpcStore } from '@/stores/rpc'
import { useWorkbenchStore } from '@/workbench/store'
import { requestEditorWorkbenchOpen } from '@/workbench/editorItems'
import type { SandboxPathEntry } from '@/types/rpc'

const props = withDefaults(defineProps<{
  /** 会话绑定的项目根（绝对路径）；缺省时向 workspace.status 解析 */
  rootPath?: string
  /** 编辑器条目归属的会话作用域 */
  scopeId?: string
}>(), { rootPath: '', scopeId: '' })

const { t } = useI18n()
const { confirm } = useConfirm()
const { pushToast } = useToasts()
const rpc = useRpcStore()
const workbench = useWorkbenchStore()

const rootAbs = ref('')
const loading = ref(false)
const error = ref('')
const children = ref<Record<string, SandboxPathEntry[]>>({})
const expanded = ref<Set<string>>(new Set())
const loadingDirs = ref<Set<string>>(new Set())
const selected = ref('')

interface MenuState { x: number; y: number; entry: SandboxPathEntry }
const menu = ref<MenuState | null>(null)
const menuEl = ref<HTMLElement | null>(null)

type DialogMode = 'newFile' | 'newFolder' | 'rename'
const dialog = ref<{ mode: DialogMode; entry: SandboxPathEntry | null } | null>(null)
const dialogValue = ref('')
const dialogError = ref('')
const dialogBusy = ref(false)
const dialogInput = ref<HTMLInputElement | null>(null)
/** 新建文件/文件夹的目标目录（对话框提交时用） */
const pendingDir = ref('')

const rootLabel = computed(() => {
  const root = rootAbs.value
  if (!root) return ''
  return root.replace(/\\/g, '/').split('/').filter(Boolean).pop() || root
})

interface VisibleRow { entry: SandboxPathEntry; depth: number }
const visibleRows = computed<VisibleRow[]>(() => {
  const out: VisibleRow[] = []
  const walk = (dir: string, depth: number) => {
    for (const entry of children.value[dir] || []) {
      out.push({ entry, depth })
      if (entry.kind === 'directory' && expanded.value.has(entry.path)) {
        walk(entry.path, depth + 1)
      }
    }
  }
  if (rootAbs.value) walk(rootAbs.value, 0)
  return out
})

const dialogTitle = computed(() => {
  if (!dialog.value) return ''
  if (dialog.value.mode === 'newFile') return t('chat.rightSidebar.explorer.newFile')
  if (dialog.value.mode === 'newFolder') return t('chat.rightSidebar.explorer.newFolder')
  return t('common.rename')
})
const dialogPrimary = computed(() => (
  dialog.value?.mode === 'rename' ? t('common.rename') : t('common.create')
))

function fileIcon(name: string): IconName {
  const ext = name.split('.').pop()?.toLowerCase() || ''
  if (['ts', 'tsx', 'js', 'jsx', 'vue', 'rs', 'py', 'go', 'java', 'c', 'cc', 'cpp', 'h', 'css', 'html', 'json', 'yaml', 'yml', 'toml'].includes(ext)) return 'fileCode'
  if (['md', 'txt', 'log', 'csv'].includes(ext)) return 'fileText'
  if (['png', 'jpg', 'jpeg', 'gif', 'webp', 'svg'].includes(ext)) return 'image'
  return 'fileText'
}

/**
 * 冷加载竞态守卫：面板可能先于 WebSocket 就绪挂载。
 *
 * 先直接调用（正常情况一次成功）；只有失败且当前未连接时，等一次连接再重试。
 * 不无条件 `waitForConnection`：离线/未配置网关时那会把每个请求都拖 8s。
 */
async function callRpc<T>(method: string, params?: Record<string, unknown>): Promise<T> {
  try {
    return await rpc.call<T>(method, params)
  } catch (error) {
    if (rpc.isConnected) throw error
    await rpc.waitForConnection(8_000).catch(() => undefined)
    return await rpc.call<T>(method, params)
  }
}

async function ensureRoot(): Promise<void> {
  if (props.rootPath.trim()) {
    rootAbs.value = props.rootPath.trim()
    return
  }
  rootAbs.value = ''
  if (!rpc.supportsMethod('workspace.status')) return
  try {
    const res = await callRpc<{ root?: string }>('workspace.status')
    rootAbs.value = String(res?.root || '')
  } catch {
    /* 保持空：下方会显示加载失败/空态，不猜路径 */
  }
}

async function loadDir(dir: string, force = false): Promise<void> {
  if (!dir) return
  if (children.value[dir] && !force) return
  loadingDirs.value = new Set(loadingDirs.value).add(dir)
  try {
    const res = await callRpc<{ entries?: SandboxPathEntry[] }>('sandbox.path.list', { path: dir })
    children.value = {
      ...children.value,
      [dir]: (res.entries || []).filter(entry => !entry.hidden),
    }
    error.value = ''
  } catch (e) {
    error.value = e instanceof Error ? e.message : String(e)
  } finally {
    const next = new Set(loadingDirs.value)
    next.delete(dir)
    loadingDirs.value = next
  }
}

async function bootstrap(): Promise<void> {
  loading.value = true
  error.value = ''
  children.value = {}
  expanded.value = new Set()
  selected.value = ''
  await ensureRoot()
  if (rootAbs.value) await loadDir(rootAbs.value)
  loading.value = false
}

async function refresh(): Promise<void> {
  if (!rootAbs.value) {
    await bootstrap()
    return
  }
  // 刷新已展开目录，保留展开状态（IDE 的刷新不该把树收回去）
  const dirs = [rootAbs.value, ...expanded.value]
  await Promise.all(dirs.map(dir => loadDir(dir, true)))
}

function collapseAll(): void {
  expanded.value = new Set()
}

async function toggleDir(dir: string): Promise<void> {
  const next = new Set(expanded.value)
  if (next.has(dir)) {
    next.delete(dir)
  } else {
    next.add(dir)
    await loadDir(dir)
  }
  expanded.value = next
}

function onRowClick(entry: SandboxPathEntry): void {
  selected.value = entry.path
  if (entry.kind === 'directory') void toggleDir(entry.path)
}

function onRowDblClick(entry: SandboxPathEntry): void {
  if (entry.kind === 'directory') {
    void toggleDir(entry.path)
    return
  }
  openInEditor(entry.path)
}

function openInEditor(path: string): void {
  const scopeId = (props.scopeId || workbench.activeSessionId || 'app').trim()
  const opened = requestEditorWorkbenchOpen({ scopeId, path, projectPath: rootAbs.value })
  if (!opened) pushToast(t('chat.rightSidebar.explorer.nameInvalid'), { tone: 'warn' })
}

function openMenu(event: MouseEvent, entry: SandboxPathEntry): void {
  selected.value = entry.path
  const width = 196
  const height = 250
  const x = Math.min(event.clientX, window.innerWidth - width - 8)
  const y = Math.min(event.clientY, window.innerHeight - height - 8)
  menu.value = { x: Math.max(8, x), y: Math.max(8, y), entry }
}

function closeMenu(): void {
  menu.value = null
}

function menuOpen(): void {
  const entry = menu.value?.entry
  closeMenu()
  if (entry && entry.kind === 'file') openInEditor(entry.path)
}

function menuNewFile(): void {
  const entry = menu.value?.entry
  closeMenu()
  if (entry) askNewFile(entry.kind === 'directory' ? entry.path : rootAbs.value)
}

function menuNewFolder(): void {
  const entry = menu.value?.entry
  closeMenu()
  if (entry) askNewFolder(entry.kind === 'directory' ? entry.path : rootAbs.value)
}

function menuRename(): void {
  const entry = menu.value?.entry
  closeMenu()
  if (!entry) return
  dialog.value = { mode: 'rename', entry }
  dialogValue.value = entry.name
  dialogError.value = ''
  focusDialog()
}

async function menuDelete(): Promise<void> {
  const entry = menu.value?.entry
  closeMenu()
  if (!entry) return
  const isDir = entry.kind === 'directory'
  const ok = await confirm({
    title: t('chat.rightSidebar.explorer.deleteTitle'),
    body: isDir
      ? t('chat.rightSidebar.explorer.deleteFolderBody', { name: entry.name })
      : t('chat.rightSidebar.explorer.deleteFileBody', { name: entry.name }),
    primaryLabel: t('common.delete'),
    primaryClass: 'btn--danger',
  })
  if (!ok) return
  try {
    const res = await rpc.call<{ ok?: boolean; error?: string }>('workspace.file.delete', {
      path: entry.path,
      projectPath: rootAbs.value,
      recursive: isDir,
    })
    if (!res || res.ok === false) {
      pushToast(res?.error || t('chat.rightSidebar.explorer.failed'), { tone: 'warn' })
      return
    }
    expanded.value = new Set([...expanded.value].filter(dir => !isWithin(entry.path, dir)))
    await refreshParent(entry.path)
    pushToast(t('chat.rightSidebar.explorer.deleted', { name: entry.name }), { tone: 'ok' })
  } catch (e) {
    pushToast(e instanceof Error ? e.message : String(e), { tone: 'warn' })
  }
}

async function menuCopyPath(): Promise<void> {
  const entry = menu.value?.entry
  closeMenu()
  if (!entry) return
  try {
    await navigator.clipboard.writeText(entry.path)
    pushToast(t('chat.rightSidebar.explorer.copied'), { tone: 'ok' })
  } catch {
    pushToast(entry.path, { tone: 'info' })
  }
}

async function menuReveal(): Promise<void> {
  const entry = menu.value?.entry
  closeMenu()
  if (!entry) return
  try {
    const res = await rpc.call<{ ok?: boolean; error?: string }>('workspace.file.reveal', {
      path: entry.path,
      projectPath: rootAbs.value,
    })
    if (!res || res.ok === false) {
      pushToast(res?.error || t('chat.rightSidebar.explorer.failed'), { tone: 'warn' })
    }
  } catch (e) {
    pushToast(e instanceof Error ? e.message : String(e), { tone: 'warn' })
  }
}

function isWithin(parent: string, child: string): boolean {
  const p = parent.replace(/\\/g, '/').replace(/\/+$/, '')
  const c = child.replace(/\\/g, '/').replace(/\/+$/, '')
  return c === p || c.startsWith(`${p}/`)
}

function parentOf(path: string): string {
  const norm = path.replace(/\\/g, '/')
  const idx = norm.lastIndexOf('/')
  return idx <= 0 ? rootAbs.value : norm.slice(0, idx).replace(/\//g, '\\')
}

async function refreshParent(path: string): Promise<void> {
  const parent = parentOf(path)
  await loadDir(parent, true)
  // 根目录被删/改名时 parent 可能是根本身
  if (rootAbs.value && parent === rootAbs.value) await loadDir(rootAbs.value, true)
}

function askNewFile(dir: string): void {
  if (!dir) return
  dialog.value = { mode: 'newFile', entry: null }
  dialogValue.value = 'untitled.md'
  dialogError.value = ''
  pendingDir.value = dir
  focusDialog()
}

function askNewFolder(dir: string): void {
  if (!dir) return
  dialog.value = { mode: 'newFolder', entry: null }
  dialogValue.value = 'new-folder'
  dialogError.value = ''
  pendingDir.value = dir
  focusDialog()
}

function focusDialog(): void {
  void nextTick(() => {
    dialogInput.value?.focus()
    dialogInput.value?.select()
  })
}

function closeDialog(): void {
  dialog.value = null
  dialogError.value = ''
  dialogBusy.value = false
}

function validName(name: string): boolean {
  return Boolean(name) && !name.includes('/') && !name.includes('\\') && !name.includes('..')
}

async function submitDialog(): Promise<void> {
  if (!dialog.value || dialogBusy.value) return
  const name = dialogValue.value.trim()
  if (!validName(name)) {
    dialogError.value = t('chat.rightSidebar.explorer.nameInvalid')
    return
  }
  dialogBusy.value = true
  dialogError.value = ''
  try {
    if (dialog.value.mode === 'rename') {
      const entry = dialog.value.entry
      if (!entry) return
      const res = await rpc.call<{ ok?: boolean; error?: string }>('workspace.file.rename', {
        path: entry.path,
        newName: name,
        projectPath: rootAbs.value,
      })
      if (!res || res.ok === false) {
        dialogError.value = res?.error || t('chat.rightSidebar.explorer.failed')
        return
      }
      const renamed = new Set<string>()
      const oldPrefix = entry.path.replace(/\//g, '\\').replace(/\\+$/, '')
      const parent = parentOf(entry.path).replace(/\\+$/, '')
      const newPrefix = `${parent}\\${name}`
      for (const dir of expanded.value) {
        const norm = dir.replace(/\//g, '\\').replace(/\\+$/, '')
        renamed.add(
          norm === oldPrefix || norm.startsWith(`${oldPrefix}\\`)
            ? `${newPrefix}${norm.slice(oldPrefix.length)}`
            : dir,
        )
      }
      expanded.value = renamed
      // 子项缓存是按旧路径建的：清空后按新路径重载已展开目录。
      children.value = {}
      await Promise.all(
        [...new Set([rootAbs.value, ...renamed])].filter(Boolean).map(dir => loadDir(dir, true)),
      )
      pushToast(t('chat.rightSidebar.explorer.renamed', { name }), { tone: 'ok' })
    } else {
      const dir = pendingDir.value || rootAbs.value
      if (dialog.value.mode === 'newFolder') {
        const res = await rpc.call<{ path?: string } | null>('sandbox.path.create-directory', {
          name,
          parentPath: dir,
          basePath: rootAbs.value,
        })
        if (!res?.path) {
          dialogError.value = t('chat.rightSidebar.explorer.failed')
          return
        }
      } else {
        const target = `${dir.replace(/[\\/]+$/, '')}\\${name}`
        const res = await rpc.call<{ ok?: boolean; error?: string }>('workspace.file.write', {
          path: target,
          content: '',
          projectPath: rootAbs.value,
        })
        if (!res || res.ok === false) {
          dialogError.value = res?.error || t('chat.rightSidebar.explorer.failed')
          return
        }
      }
      await loadDir(dir, true)
      const next = new Set(expanded.value)
      next.add(dir)
      expanded.value = next
      pushToast(t('chat.rightSidebar.explorer.created', { name }), { tone: 'ok' })
    }
    closeDialog()
  } catch (e) {
    dialogError.value = e instanceof Error ? e.message : String(e)
  } finally {
    dialogBusy.value = false
  }
}

function onDocPointerDown(event: PointerEvent): void {
  if (!menu.value) return
  const node = event.target as Node | null
  if (node && menuEl.value?.contains(node)) return
  closeMenu()
}

function onDocKeydown(event: KeyboardEvent): void {
  if (event.key === 'Escape') {
    if (menu.value) closeMenu()
    else if (dialog.value) closeDialog()
  }
}

watch(() => props.rootPath, () => { void bootstrap() })

onMounted(() => {
  document.addEventListener('pointerdown', onDocPointerDown, true)
  document.addEventListener('keydown', onDocKeydown)
  void bootstrap()
})

onBeforeUnmount(() => {
  document.removeEventListener('pointerdown', onDocPointerDown, true)
  document.removeEventListener('keydown', onDocKeydown)
})
</script>

<style scoped>
.fx { display: flex; min-height: 0; flex: 1; flex-direction: column; }

.fx__toolbar {
  display: flex;
  align-items: center;
  gap: 2px;
  padding: 4px 6px;
  border-bottom: 1px solid var(--border);
}
.fx__icon {
  display: inline-flex;
  width: 24px;
  height: 24px;
  align-items: center;
  justify-content: center;
  padding: 0;
  border: 0;
  border-radius: var(--radius-sm);
  background: transparent;
  color: var(--text-muted);
  cursor: pointer;
}
.fx__icon:hover { background: var(--bg-hover); color: var(--text); }
.fx__root {
  min-width: 0;
  margin-left: 4px;
  overflow: hidden;
  color: var(--text-dim);
  font-size: 10.5px;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.fx__list {
  min-height: 0;
  flex: 1;
  margin: 0;
  padding: 4px 0 12px;
  overflow: auto;
  list-style: none;
}
.fx__row {
  display: flex;
  align-items: center;
  gap: 4px;
  padding: 3px 6px;
  color: var(--text-muted);
  cursor: default;
  font-size: 12px;
  white-space: nowrap;
  outline: none;
}
.fx__row:hover { background: var(--bg-hover); }
.fx__row.is-selected { background: color-mix(in srgb, var(--accent) 14%, transparent); color: var(--text); }
.fx__row.is-dir { cursor: pointer; }
.fx__chevron { width: 11px; flex: 0 0 auto; color: var(--text-dim); }
.fx__kind { flex: 0 0 auto; color: var(--text-muted); }
.fx__kind.is-dir { color: var(--accent); }
.fx__name { overflow: hidden; text-overflow: ellipsis; }
.fx__spinner { margin-left: auto; color: var(--text-dim); }
.fx__empty { padding: 12px; color: var(--text-dim); font-size: 11.5px; }
.fx__error { margin: 8px; color: var(--danger); font-size: 11.5px; line-height: 1.5; }
.fx__hint { margin: 8px; color: var(--text-dim); font-size: 11.5px; }
.fx__foot {
  margin: 0;
  padding: 4px 8px;
  border-top: 1px solid var(--border);
  color: var(--text-dim);
  font-size: 10px;
}

.fx-menu {
  position: fixed;
  z-index: 1300;
  width: 196px;
  padding: 5px;
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  background: var(--bg-elevated, var(--bg-surface));
  box-shadow: var(--shadow-lg);
}
.fx-menu__item {
  display: flex;
  width: 100%;
  align-items: center;
  gap: 8px;
  padding: 5px 8px;
  border: 0;
  border-radius: var(--radius-sm);
  background: transparent;
  color: var(--text);
  cursor: pointer;
  font: inherit;
  font-size: 12px;
  text-align: left;
}
.fx-menu__item:hover { background: var(--bg-hover); }
.fx-menu__item.is-danger { color: var(--danger); }
.fx-menu__sep { height: 1px; margin: 4px 6px; background: var(--border); }

.fx-dialog__backdrop {
  position: fixed;
  z-index: 1400;
  inset: 0;
  display: grid;
  place-items: center;
  background: color-mix(in srgb, var(--scrim) 42%, transparent);
}
.fx-dialog {
  display: grid;
  width: min(360px, calc(100vw - 32px));
  gap: 8px;
  padding: 14px;
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  background: var(--bg-elevated, var(--bg-surface));
  box-shadow: var(--shadow-lg);
}
.fx-dialog__title { margin: 0; color: var(--text); font-size: 13px; font-weight: 600; }
.fx-dialog__label { color: var(--text-dim); font-size: 11px; }
.fx-dialog__input {
  width: 100%;
  padding: 6px 8px;
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  background: var(--bg);
  color: var(--text);
  font: inherit;
  font-size: 12.5px;
}
.fx-dialog__error { margin: 0; color: var(--danger); font-size: 11.5px; }
.fx-dialog__actions { display: flex; justify-content: flex-end; gap: 6px; }
</style>
