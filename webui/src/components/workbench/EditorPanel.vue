<!--
  内置代码编辑器（工作台面板，IDE 式单文件标签）。

  设计取舍：
  · 编辑内核用 CodeMirror 6（语法高亮、括号匹配、折叠、搜索、撤销栈）；
    不再用「highlight.js + textarea」——查看与编辑两套渲染必然漂移。
  · 智能体的改动来自 `workspace.file_changes.list` 的真实台账，逐行标记
    新增/修改；没有台账就**不画**任何标记，不猜。
  · 保存前先回读磁盘：内容变了就要求「重新载入 / 仍然覆盖」，
    绝不静默覆盖智能体刚写下的内容。
  · `projectPath` 必须随打开事件一路传到这里：后端的 `workspace.file.*`
    以它为根，丢了它就会打开同名但不同目录的文件。
-->
<template>
  <div class="pet-editor" :class="{ 'is-busy': busy }">
    <div class="pet-editor__bar">
      <button
        v-if="!loading && !loadError"
        type="button"
        class="pet-editor__btn"
        :class="{ 'is-active': mode === 'edit' }"
        :title="t('workbench.editor.editTitle')"
        @click="toggleEdit"
      >
        <Icon :name="mode === 'edit' ? 'eye' : 'edit'" :size="14" aria-hidden="true" />
        <span>{{ mode === 'edit' ? t('workbench.editor.view') : t('workbench.editor.edit') }}</span>
      </button>
      <button
        v-if="hasAgentDiff"
        type="button"
        class="pet-editor__btn"
        :class="{ 'is-active': mode === 'diff' }"
        :title="t('workbench.editor.diffTitle')"
        @click="toggleDiff"
      >
        <Icon name="fileCode" :size="14" aria-hidden="true" />
        <span>{{ t('workbench.editor.diff') }}</span>
      </button>
      <span class="pet-editor__spacer" />
      <span v-if="dirty" class="pet-editor__dirty">{{ t('workbench.editor.dirty') }}</span>
      <button type="button" class="pet-editor__btn" :title="t('workbench.editor.reloadTitle')" @click="reload()">
        <Icon name="refresh" :size="14" aria-hidden="true" />
        <span>{{ t('workbench.editor.reload') }}</span>
      </button>
      <button
        type="button"
        class="pet-editor__btn pet-editor__btn--primary"
        :disabled="!dirty || busy || mode === 'view'"
        :title="t('workbench.editor.saveTitle')"
        @click="save()"
      >
        <Icon name="check" :size="14" aria-hidden="true" />
        <span>{{ busy ? t('workbench.editor.saving') : t('workbench.editor.save') }}</span>
      </button>
    </div>

    <p v-if="loadError" class="pet-editor__error">{{ loadError }}</p>
    <p v-else-if="loading" class="pet-editor__hint">{{ t('workbench.editor.loading') }}</p>

    <!-- 智能体改动横幅：只会出现在有真实台账时 -->
    <div v-if="agentChange" class="pet-editor__agent">
      <Icon name="sparkles" :size="14" aria-hidden="true" />
      <span>
        {{ t('workbench.editor.agentChanged', {
          tool: agentChange.tool || 'agent',
          added: agentStats.added,
          modified: agentStats.modified,
          removed: agentStats.removed,
        }) }}
      </span>
      <button type="button" class="pet-editor__link" @click="toggleDiff">
        {{ t('workbench.editor.viewChanges') }}
      </button>
    </div>

    <!-- 磁盘侧变化（可能是智能体刚写入，也可能是外部编辑）：不静默覆盖 -->
    <div v-if="diskChanged" class="pet-editor__conflict">
      <span>{{ dirty ? t('workbench.editor.conflictDirty') : t('workbench.editor.diskChanged') }}</span>
      <button type="button" class="pet-editor__link" @click="reload()">{{ t('workbench.editor.reload') }}</button>
      <button v-if="dirty" type="button" class="pet-editor__link" @click="save({ force: true })">
        {{ t('workbench.editor.overwrite') }}
      </button>
    </div>

    <p v-if="statusNote" class="pet-editor__note">{{ statusNote }}</p>

    <!-- 差异视图 -->
    <div v-if="mode === 'diff'" class="pet-editor__diff">
      <p class="pet-editor__diff-legend">
        <span class="pet-editor__tag pet-editor__tag--added">+ {{ agentStats.added }}</span>
        <span class="pet-editor__tag pet-editor__tag--modified">~ {{ agentStats.modified }}</span>
        <span class="pet-editor__tag pet-editor__tag--removed">- {{ agentStats.removed }}</span>
      </p>
      <div v-for="(row, i) in diffRows" :key="i" class="pet-editor__diff-row" :class="`is-${row.status}`">
        <span class="pet-editor__diff-sign">{{ row.status === 'added' ? '+' : row.status === 'removed' ? '-' : row.status === 'modified' ? '~' : ' ' }}</span>
        <code>{{ row.status === 'removed' ? row.before : row.after }}</code>
      </div>
      <p v-if="!diffRows || !diffRows.length" class="pet-editor__hint">{{ t('workbench.editor.diffEmpty') }}</p>
    </div>

    <!-- 编辑 / 只读：同一个 CodeMirror 实例，只切换 readOnly -->
    <div v-else-if="!loading && !loadError" class="pet-editor__body">
      <CodeMirrorEditor
        :model-value="draft"
        :path="filePath"
        :readonly="mode !== 'edit'"
        :active="props.active !== false"
        @update:model-value="onEditorInput"
        @save="mode === 'edit' ? save() : undefined"
      />
    </div>

    <footer class="pet-editor__foot">
      <span>{{ t('workbench.editor.linesBytes', { lines: lineCount, bytes: byteCount }) }}</span>
      <span>{{ languageLabel }}</span>
      <span v-if="absoluteTarget">{{ t('workbench.editor.root', { root: absoluteTarget.root }) }}</span>
      <span v-else-if="resolvedProjectPath">{{ t('workbench.editor.root', { root: resolvedProjectPath }) }}</span>
      <span v-else>{{ t('workbench.editor.rootUnbound') }}</span>
    </footer>
  </div>
</template>

<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import Icon from '@/components/Icon.vue'
import { useRpcStore } from '@/stores/rpc'
import { diffLines, diffSummary, type DiffRow } from '@/utils/workbench/lineDiff'
import { normalizeEditorPath } from '@/workbench/editorItems'
import CodeMirrorEditor from './CodeMirrorEditor.vue'

const props = defineProps<{
  path: string
  scopeId: string
  /** 会话绑定的项目根（可选）；缺省时面板自行从 sessions.list 解析 */
  projectPath?: string
  /** 面板是否处于激活标签页（非激活时不轮询磁盘） */
  active?: boolean
}>()

const { t } = useI18n()
const rpc = useRpcStore()

interface ReadResult { ok?: boolean; path?: string; content?: string; bytes?: number; error?: string }
interface FileChangeItem {
  path?: string
  kind?: string
  before?: string
  after?: string
  tool?: string
  ts?: number
}

const loading = ref(false)
const busy = ref(false)
const loadError = ref('')
const statusNote = ref('')
const base = ref('')
const draft = ref('')
const baseBytes = ref(0)
const mode = ref<'view' | 'edit' | 'diff'>('view')
const diskChanged = ref(false)
const agentChange = ref<FileChangeItem | null>(null)
const agentDiff = ref<DiffRow[] | null>(null)
const resolvedProjectPath = ref('')
let pollTimer: ReturnType<typeof setInterval> | null = null

const filePath = computed(() => normalizeEditorPath(props.path))
const dirty = computed(() => draft.value !== base.value)
const lineCount = computed(() => Math.max(1, draft.value.split('\n').length))
const byteCount = computed(() => {
  if (typeof TextEncoder === 'undefined') return baseBytes.value
  return new TextEncoder().encode(draft.value).length
})
const languageLabel = computed(() => {
  const ext = filePath.value.split('.').pop()?.toLowerCase() || 'text'
  const names: Record<string, string> = {
    ts: 'TypeScript', tsx: 'TSX', js: 'JavaScript', jsx: 'JSX', vue: 'Vue',
    rs: 'Rust', py: 'Python', go: 'Go', java: 'Java', css: 'CSS', scss: 'SCSS',
    html: 'HTML', json: 'JSON', yaml: 'YAML', yml: 'YAML', md: 'Markdown',
    toml: 'TOML', sh: 'Shell', sql: 'SQL',
  }
  return names[ext] || ext.toUpperCase()
})
const agentStats = computed(() => diffSummary(agentDiff.value || []))
const hasAgentDiff = computed(() => Boolean(agentDiff.value && agentDiff.value.some(r => r.status !== 'same')))
const diffRows = computed(() => (agentDiff.value || []).filter(r => r.status !== 'same'))

/** 绝对路径（`D:/x/y.ts`）：把它拆成「根 = 所在目录 + 相对名」——
 *  `workspace.file.*` 只接受根内相对路径，拆开后既能在未绑定项目时打开任意文件，
 *  也不会让根外的读取被放行（根就是该文件所在目录）。 */
function splitAbsolute(raw: string): { root: string; rel: string } | null {
  if (!/^[A-Za-z]:\//.test(raw)) return null
  const idx = raw.lastIndexOf('/')
  if (idx <= 2) return { root: `${raw.slice(0, 3)}`, rel: raw.slice(3) }
  return { root: raw.slice(0, idx), rel: raw.slice(idx + 1) }
}

const absoluteTarget = computed(() => splitAbsolute(filePath.value))

function params() {
  const p: Record<string, unknown> = { path: filePath.value }
  const root = absoluteTarget.value?.root || resolvedProjectPath.value
  if (root) p.projectPath = root
  return p
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

/**
 * 会话绑定的项目根：`workspace.file.*` 的根必须与「智能体实际读写的位置」一致，
 * 否则编辑器打开的是另一个同名文件。面板不自带上下文，因此问会话台账要。
 */
async function resolveProjectPath(): Promise<void> {
  const explicit = (props.projectPath || '').trim()
  if (explicit) { resolvedProjectPath.value = explicit; return }
  resolvedProjectPath.value = ''
  const scope = props.scopeId.trim()
  if (!scope) return
  try {
    const res = await rpc.call<{ sessions?: Array<{ key?: string; session?: string; projectPath?: string | null }> }>(
      'sessions.list',
      { limit: 200 },
    )
    const hit = (res.sessions || []).find(s => (s.key || s.session) === scope)
    const project = hit?.projectPath
    if (typeof project === 'string' && project.trim()) resolvedProjectPath.value = project.trim()
  } catch {
    /* 台账不可用时保持未绑定：底部会如实显示「未绑定」，不猜路径 */
  }
}

async function readFile(): Promise<void> {
  if (!(await ensureConnected())) {
    loadError.value = t('workbench.editor.notConnected')
    return
  }
  loading.value = true
  loadError.value = ''
  try {
    const res = await rpc.call<ReadResult>('workspace.file.read', params())
    if (!res || res.ok === false) {
      loadError.value = res?.error || t('workbench.editor.readFailed')
      return
    }
    base.value = String(res.content ?? '')
    draft.value = base.value
    baseBytes.value = Number(res.bytes ?? base.value.length)
    diskChanged.value = false
  } catch (err) {
    loadError.value = err instanceof Error ? err.message : String(err)
  } finally {
    loading.value = false
    void loadAgentChanges()
  }
}

/** 路径匹配：台账里可能是绝对路径、也可能是相对路径。 */
function matchesFile(changePath: string | undefined, rel: string): boolean {
  const raw = (changePath || '').replace(/\\/g, '/')
  if (!raw || !rel) return false
  if (raw === rel) return true
  if (raw.endsWith(`/${rel}`)) return true
  const abs = (resolvedProjectPath.value || '').replace(/\\/g, '/')
  if (abs && raw === `${abs}/${rel}`) return true
  return false
}

async function loadAgentChanges(): Promise<void> {
  try {
    const res = await rpc.call<{ items?: FileChangeItem[] }>('workspace.file_changes.list', { limit: 120 })
    const hit = (res.items || []).find(item => matchesFile(item.path, filePath.value))
    agentChange.value = hit || null
    if (!hit) { agentDiff.value = null; return }
    // `before` 缺失（如 create）视为空文件
    const rows = diffLines(String(hit.before ?? ''), String(hit.after ?? ''))
    agentDiff.value = rows
  } catch {
    agentChange.value = null
    agentDiff.value = null
  }
}

/** 只在激活且非编辑中才轮询：编辑中切走内容会让光标位置错乱。 */
async function checkDisk(): Promise<void> {
  if (!rpc.isConnected || loading.value || mode.value === 'diff') return
  try {
    const res = await rpc.call<ReadResult>('workspace.file.read', params())
    if (!res || res.ok === false) return
    const disk = String(res.content ?? '')
    if (disk !== base.value) {
      diskChanged.value = true
      if (!dirty.value) {
        // 没改动就自动跟上（智能体刚写完，用户还没开始编辑）
        base.value = disk
        draft.value = disk
        baseBytes.value = Number(res.bytes ?? disk.length)
        diskChanged.value = false
        statusNote.value = t('workbench.editor.autoReloaded')
        void loadAgentChanges()
      }
    } else if (diskChanged.value) {
      diskChanged.value = false
    }
  } catch {
    /* 轮询失败静默：下一次再试，不打扰编辑 */
  }
}

async function reload(force = false): Promise<void> {
  if (dirty.value && !force) {
    const ok = typeof window === 'undefined' ? true : window.confirm(t('workbench.editor.reloadConfirm'))
    if (!ok) return
  }
  mode.value = 'view'
  statusNote.value = ''
  await readFile()
}

/**
 * 保存。`force` 表示用户已经看到冲突横幅并选择「仍然覆盖」。
 * 非 force 时先回读磁盘，内容与基线不同即中止（不静默覆盖）。
 */
async function save(options: { force?: boolean } = {}): Promise<void> {
  if (busy.value || !dirty.value) return
  if (!(await ensureConnected())) { loadError.value = t('workbench.editor.notConnected'); return }
  busy.value = true
  statusNote.value = ''
  try {
    if (!options.force) {
      const current = await rpc.call<ReadResult>('workspace.file.read', params())
      if (current && current.ok !== false && String(current.content ?? '') !== base.value) {
        diskChanged.value = true
        statusNote.value = t('workbench.editor.saveBlocked')
        return
      }
    }
    const res = await rpc.call<{ ok?: boolean; error?: string; bytes?: number }>('workspace.file.write', {
      ...params(),
      content: draft.value,
    })
    if (!res || res.ok === false) {
      statusNote.value = res?.error || t('workbench.editor.saveFailed')
      return
    }
    base.value = draft.value
    baseBytes.value = Number(res.bytes ?? draft.value.length)
    diskChanged.value = false
    statusNote.value = t('workbench.editor.saved')
  } catch (err) {
    statusNote.value = err instanceof Error ? err.message : String(err)
  } finally {
    busy.value = false
  }
}

function onEditorInput(value: string): void {
  draft.value = value
}

async function toggleEdit(): Promise<void> {
  mode.value = mode.value === 'edit' ? 'view' : 'edit'
}

function toggleDiff(): void {
  mode.value = mode.value === 'diff' ? 'view' : 'diff'
}

watch(() => [props.path, props.projectPath, props.scopeId], () => {
  mode.value = 'view'
  base.value = ''
  draft.value = ''
  agentChange.value = null
  agentDiff.value = null
  resolvedProjectPath.value = ''
  void bootstrap()
})

watch(() => props.active, (on) => {
  if (on) void checkDisk()
})

async function bootstrap(): Promise<void> {
  await resolveProjectPath()
  await readFile()
}

onMounted(() => {
  void bootstrap()
  pollTimer = setInterval(() => {
    if (props.active !== false) void checkDisk()
  }, 4000)
})

onBeforeUnmount(() => {
  if (pollTimer) clearInterval(pollTimer)
  pollTimer = null
})
</script>

<style scoped>
.pet-editor {
  display: flex;
  height: 100%;
  min-height: 0;
  flex-direction: column;
  background: var(--bg-surface);
  color: var(--text);
  font-size: var(--fs-sm);
}

.pet-editor__bar {
  display: flex;
  flex: 0 0 auto;
  align-items: center;
  gap: 6px;
  padding: 6px 10px;
  border-bottom: 1px solid var(--border);
  background: var(--bg-elevated, var(--bg-surface));
}

.pet-editor__spacer { flex: 1 1 auto; }

.pet-editor__btn {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  padding: 3px 8px;
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  background: transparent;
  color: var(--text-muted);
  cursor: pointer;
  font: inherit;
  font-size: var(--fs-xs);
}

.pet-editor__btn:hover:not(:disabled) { background: var(--bg-hover); color: var(--text); }
.pet-editor__btn:disabled { cursor: not-allowed; opacity: 0.5; }
.pet-editor__btn.is-active { border-color: var(--accent); color: var(--accent); }
.pet-editor__btn--primary { border-color: var(--accent); color: var(--accent); }

.pet-editor__dirty {
  padding: 2px 6px;
  border-radius: var(--radius-pill);
  background: color-mix(in srgb, var(--warn) 16%, transparent);
  color: var(--warn);
  font-size: 10.5px;
}

.pet-editor__hint,
.pet-editor__note,
.pet-editor__error {
  margin: 8px 12px;
  font-size: var(--fs-xs);
  line-height: 1.6;
}

.pet-editor__hint { color: var(--text-dim); }
.pet-editor__note { color: var(--text-muted); }
.pet-editor__error { color: var(--danger); white-space: pre-wrap; }

.pet-editor__agent {
  display: flex;
  flex: 0 0 auto;
  align-items: center;
  gap: 6px;
  padding: 6px 12px;
  border-bottom: 1px solid color-mix(in srgb, var(--accent) 26%, transparent);
  background: color-mix(in srgb, var(--accent) 9%, transparent);
  color: var(--text);
  font-size: var(--fs-xs);
}

.pet-editor__conflict {
  display: flex;
  flex: 0 0 auto;
  align-items: center;
  gap: 8px;
  padding: 6px 12px;
  border-bottom: 1px solid color-mix(in srgb, var(--warn) 30%, transparent);
  background: color-mix(in srgb, var(--warn) 12%, transparent);
  color: var(--text);
  font-size: var(--fs-xs);
}

.pet-editor__link {
  padding: 0 2px;
  border: 0;
  background: transparent;
  color: var(--accent);
  cursor: pointer;
  font: inherit;
  text-decoration: underline;
}

.pet-editor__body {
  display: flex;
  flex: 1 1 auto;
  min-height: 0;
  overflow: hidden;
}

.pet-editor__diff {
  flex: 1 1 auto;
  min-height: 0;
  overflow: auto;
  padding: 8px 0 40px;
  font-family: var(--font-mono, ui-monospace, SFMono-Regular, Menlo, Consolas, monospace);
  font-size: 12.5px;
  line-height: 1.55;
}

.pet-editor__diff-legend {
  display: flex;
  gap: 8px;
  margin: 0 0 6px;
  padding: 0 12px;
  font-size: var(--fs-xs);
}

.pet-editor__tag { padding: 1px 6px; border-radius: var(--radius-pill); }
.pet-editor__tag--added { background: color-mix(in srgb, var(--ok) 18%, transparent); color: var(--ok); }
.pet-editor__tag--modified { background: color-mix(in srgb, var(--warn) 18%, transparent); color: var(--warn); }
.pet-editor__tag--removed { background: color-mix(in srgb, var(--danger) 16%, transparent); color: var(--danger); }

.pet-editor__diff-row {
  display: flex;
  gap: 8px;
  padding: 0 12px;
  white-space: pre;
}

.pet-editor__diff-row code { font: inherit; }
.pet-editor__diff-sign { width: 1ch; flex: 0 0 auto; opacity: 0.7; }
.pet-editor__diff-row.is-added { background: color-mix(in srgb, var(--ok) 10%, transparent); }
.pet-editor__diff-row.is-modified { background: color-mix(in srgb, var(--warn) 10%, transparent); }
.pet-editor__diff-row.is-removed { background: color-mix(in srgb, var(--danger) 9%, transparent); }

.pet-editor__foot {
  display: flex;
  flex: 0 0 auto;
  gap: 12px;
  padding: 4px 12px;
  border-top: 1px solid var(--border);
  color: var(--text-dim);
  font-size: 10.5px;
  white-space: nowrap;
  overflow: hidden;
}

.pet-editor__foot span { overflow: hidden; text-overflow: ellipsis; }
</style>
