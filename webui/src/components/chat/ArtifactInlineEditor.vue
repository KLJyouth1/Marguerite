<template>
  <section class="aie" :data-editable="kind.inlineEditable">
    <!-- 头部：类型色条 + 图标 + 名称 + 格式 + 双轨动作 -->
    <header class="aie__bar">
      <span class="aie__type-bar" :style="{ background: kind.colorVar }" aria-hidden="true" />
      <Icon :name="kind.icon" :size="16" :style="{ color: kind.colorVar }" />
      <span class="aie__title" :title="artifact.name || artifact.path">{{ artifact.name || basename }}</span>
      <span class="aie__format" :style="{ color: kind.colorVar }">{{ kind.format }}</span>
      <span class="aie__spacer" />
      <button
        type="button"
        class="aie__btn"
        :title="'用系统程序打开'"
        @click="emit('open-system', artifact)"
      >
        <Icon name="appWindow" :size="14" />
      </button>
      <button
        type="button"
        class="aie__btn"
        :title="'关闭编辑器'"
        @click="emit('close')"
      >
        <Icon name="x" :size="14" />
      </button>
    </header>

    <!-- 二进制 / 不支持内联编辑：给出下一步动作，而非死路 -->
    <div v-if="!kind.inlineEditable" class="aie__blocked">
      <Icon name="fileQuestion" :size="26" :style="{ color: kind.colorVar }" :opacity="0.5" />
      <p class="aie__blocked-reason">{{ blockedReason }}</p>
      <button type="button" class="aie__blocked-btn" @click="emit('open-system', artifact)">
        <Icon name="appWindow" :size="13" />用系统程序打开
      </button>
    </div>

    <!-- 可编辑 -->
    <div v-else class="aie__body">
      <div
        class="aie__pane aie__pane--outline"
        :style="{ width: outlineWidth + 'px' }"
        :data-collapsed="outlineCollapsed"
      >
        <div class="aie__outline-head">
          <Icon name="listTree" :size="13" />
          <span>大纲</span>
          <button
            type="button"
            class="aie__outline-toggle"
            :title="outlineCollapsed ? '展开大纲' : '收起大纲'"
            @click="outlineCollapsed = !outlineCollapsed"
          >
            <Icon :name="outlineCollapsed ? 'chevronRight' : 'chevronLeft'" :size="12" />
          </button>
        </div>
        <nav v-if="!outlineCollapsed" class="aie__outline">
          <button
            v-for="(node, i) in outline"
            :key="i"
            type="button"
            class="aie__outline-item"
            :class="`aie__outline-item--lv${node.level}`"
            @click="scrollToLine(node.line)"
          >{{ node.label }}</button>
          <p v-if="outline.length === 0" class="aie__outline-empty">暂无可生成的大纲</p>
        </nav>
      </div>

      <div
        class="aie__resizer"
        role="separator"
        aria-orientation="vertical"
        tabindex="0"
        @pointerdown="startResize"
        @keydown.left.prevent="nudgeOutline(-16)"
        @keydown.right.prevent="nudgeOutline(16)"
      />

      <div class="aie__pane aie__pane--editor">
        <div v-if="loadState === 'loading'" class="aie__status">
          <Icon name="refresh" :size="14" class="aie__spin" />读取中…
        </div>
        <div v-else-if="loadState === 'error'" class="aie__status aie__status--err">
          <Icon name="alertTriangle" :size="14" />{{ loadMessage || '读取失败' }}
          <button type="button" class="aie__status-retry" @click="loadContent">
            <Icon name="refresh" :size="12" />重试
          </button>
        </div>
        <div v-else-if="loadState === 'unsupported'" class="aie__status aie__status--err">
          <Icon name="alertTriangle" :size="14" />{{ loadMessage }}
        </div>
        <template v-else>
          <textarea
            ref="editorEl"
            v-model="draft"
            class="aie__textarea"
            :class="{ 'aie__textarea--dirty': isDirty }"
            spellcheck="false"
            :aria-label="`编辑 ${artifact.name || basename}`"
            @input="onInput"
          />
          <footer class="aie__footer">
            <span class="aie__meta">
              {{ draft.split('\n').length }} 行 · {{ byteLength }} 字符
              <span v-if="isDirty" class="aie__dirty">· 未保存</span>
            </span>
            <span class="aie__footer-actions">
              <button type="button" class="aie__btn-text" :disabled="!isDirty" @click="resetDraft">撤销更改</button>
              <button
                type="button"
                class="aie__btn-primary"
                :disabled="!isDirty || saving"
                @click="save"
              >
                <Icon name="save" :size="13" />{{ saving ? '保存中…' : '保存' }}
              </button>
            </span>
          </footer>
          <div v-if="saveMessage" class="aie__save-msg" :data-status="saveStatus">
            <Icon :name="saveStatus === 'ok' ? 'check' : 'alertTriangle'" :size="13" />{{ saveMessage }}
          </div>
        </template>
      </div>
    </div>
  </section>
</template>

<script setup lang="ts">
import { computed, onBeforeUnmount, ref, watch } from 'vue'
import Icon from '@/components/Icon.vue'
import type { RpcCallOptions } from '@/lib/rpc'
import type { ArtifactPayload } from '@/types/rpc'
import { classifyArtifact, artifactEditBlockedReason, type ArtifactKindDescriptor } from '@/utils/artifactKind'
import { useArtifactActions } from '@/composables/chat/useArtifactActions'

/**
 * Structural slice of the RPC store this editor needs. Kept aligned with
 * `useArtifactActions`' own requirement (same member signatures, including
 * `RpcCallOptions`) so the store can be handed straight through without a cast —
 * an `unknown` options parameter here silently widened the contract and forced
 * an `as never` at the call site, which would have hidden a real mismatch.
 */
type ArtifactRpcLike = {
  supportsMethod: (method: string) => boolean
  markMethodUnavailable: (method: string) => void
  call: <T = unknown>(
    method: string,
    params?: Record<string, unknown>,
    options?: RpcCallOptions,
  ) => Promise<T>
}

const props = defineProps<{
  artifact: ArtifactPayload
  rpc: ArtifactRpcLike
  sessionKey: string
  authToken?: () => string
  baseOrigin?: string
}>()

const emit = defineEmits<{
  close: []
  saved: [artifact: ArtifactPayload]
  'open-system': [artifact: ArtifactPayload]
}>()

const basename = computed(() => {
  const p = String(props.artifact.path || props.artifact.name || 'file')
  return p.split(/[\\/]/).pop()?.split(/[?#]/)[0] || 'file'
})
const kind: ArtifactKindDescriptor = classifyArtifact(props.artifact.name || basename.value, props.artifact.mime || props.artifact.content_type)
const blockedReason = artifactEditBlockedReason(kind)

const actions = useArtifactActions({
  rpc: props.rpc,
  authToken: () => props.authToken?.() || '',
  baseOrigin: () => props.baseOrigin || window.location.origin,
  sessionKey: () => props.sessionKey,
})

const editorEl = ref<HTMLTextAreaElement | null>(null)
const draft = ref('')
const original = ref('')
const isDirty = computed(() => draft.value !== original.value)
const byteLength = computed(() => draft.value.length)
const loadState = ref<'idle' | 'loading' | 'error' | 'unsupported' | 'ready'>('idle')
const loadMessage = ref('')
const saving = ref(false)
const saveStatus = ref<'ok' | 'error'>('ok')
const saveMessage = ref('')

// ── 大纲：标题树 / 代码符号 / CSV sheet ──
interface OutlineNode { level: number; label: string; line: number }
const outline = ref<OutlineNode[]>([])

function buildOutline(text: string) {
  const nodes: OutlineNode[] = []
  const lines = text.split('\n')
  const codeExt = ['ts', 'tsx', 'js', 'mjs', 'jsx', 'vue', 'py', 'rs', 'go', 'java', 'c', 'cpp', 'h', 'cs', 'rb', 'php', 'sql', 'sh', 'json', 'yaml', 'yml']
  const ext = basename.value.split('.').pop()?.toLowerCase() || ''
  if (kind.key === 'doc') {
    lines.forEach((line, i) => {
      const m = /^(#{1,4})\s+(.*)$/.exec(line.trim())
      if (m) nodes.push({ level: m[1].length, label: m[2].slice(0, 48), line: i })
    })
  } else if (kind.key === 'code' && codeExt.includes(ext)) {
    lines.forEach((line, i) => {
      const m = /^(?:export\s+)?(?:async\s+)?function\s+([A-Za-z0-9_]+)|^class\s+([A-Za-z0-9_]+)|^(?:def|fn|func|pub\s+fn)\s+([A-Za-z0-9_]+)/.exec(line)
      if (m) nodes.push({ level: 1, label: (m[1] || m[2] || m[3]), line: i })
    })
  } else if (kind.key === 'sheet') {
    const head = lines[0]?.split(/[,;\t]/).map(c => c.trim()).filter(Boolean) || []
    head.slice(0, 8).forEach((h, i) => nodes.push({ level: 1, label: h || `列${i + 1}`, line: 0 }))
  }
  outline.value = nodes
}

async function loadContent() {
  loadState.value = 'loading'
  loadMessage.value = ''
  const res = await actions.loadTextContent(props.artifact)
  if (res.status !== 'ok') {
    loadState.value = res.status === 'unsupported' ? 'unsupported' : 'error'
    loadMessage.value = res.message || '读取失败'
    draft.value = ''
    original.value = ''
    return
  }
  draft.value = res.content
  original.value = res.content
  buildOutline(res.content)
  loadState.value = 'ready'
}

function onInput() {
  saveMessage.value = ''
}
function resetDraft() {
  draft.value = original.value
}

async function save() {
  if (!isDirty.value || saving.value) return
  saving.value = true
  saveMessage.value = ''
  const res = await actions.saveTextContent(props.artifact, draft.value)
  saving.value = false
  if (res.status === 'ok') {
    original.value = draft.value
    saveStatus.value = 'ok'
    saveMessage.value = '已保存'
    emit('saved', props.artifact)
  } else {
    saveStatus.value = 'error'
    saveMessage.value = res.message || '保存失败'
  }
}

// ── 大纲跳转：把行号映射到 textarea 滚动位置 ──
function scrollToLine(line: number) {
  const el = editorEl.value
  if (!el) return
  const target = line / Math.max(el.value.split('\n').length, 1)
  el.scrollTop = target * (el.scrollHeight - el.clientHeight)
  el.focus({ preventScroll: true })
}

// ── 分屏拖拽（沿用 min/max 区间，动画走 token duration）──
const MIN_OUTLINE = 180
const MAX_OUTLINE = 420
const outlineWidth = ref(260)
const outlineCollapsed = ref(false)
let resizing = false
function startResize(e: PointerEvent) {
  resizing = true
  ;(e.target as HTMLElement).setPointerCapture?.(e.pointerId)
  window.addEventListener('pointermove', onResize)
  window.addEventListener('pointerup', stopResize)
}
function onResize(e: PointerEvent) {
  if (!resizing) return
  const clamped = Math.min(MAX_OUTLINE, Math.max(MIN_OUTLINE, e.clientX - outlineLeft()))
  outlineWidth.value = clamped
}
function outlineLeft() {
  const pane = editorEl.value?.closest('.aie')?.querySelector('.aie__pane--outline') as HTMLElement | null
  return pane?.getBoundingClientRect().left ?? 0
}
function stopResize() {
  resizing = false
  window.removeEventListener('pointermove', onResize)
  window.removeEventListener('pointerup', stopResize)
}
function nudgeOutline(delta: number) {
  outlineWidth.value = Math.min(MAX_OUTLINE, Math.max(MIN_OUTLINE, outlineWidth.value + delta))
}

watch(() => props.artifact, () => { void loadContent() }, { immediate: true })
onBeforeUnmount(stopResize)
</script>

<style scoped>
.aie { display: flex; flex-direction: column; height: 100%; min-height: 0; background: var(--bg-elevated); }
.aie__bar {
  display: flex; align-items: center; gap: 8px;
  padding: 8px 10px; border-bottom: 1px solid var(--border);
}
.aie__type-bar { width: 3px; height: 18px; border-radius: var(--radius-xs); flex-shrink: 0; }
.aie__title { font-size: 13px; color: var(--text); font-weight: 500; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
.aie__format { font-size: 10px; font-weight: 600; font-family: var(--font-mono, monospace); letter-spacing: 0.04em; flex-shrink: 0; }
.aie__spacer { flex: 1; }
.aie__btn {
  display: inline-flex; align-items: center; justify-content: center;
  width: 28px; height: 28px; border: none; border-radius: var(--radius-sm);
  background: transparent; color: var(--text-muted); cursor: pointer;
  transition: background var(--dur-fast, 120ms) var(--ease-standard), color var(--dur-fast, 120ms) var(--ease-standard);
}
.aie__btn:hover { background: var(--bg-hover); color: var(--text); }
.aie__btn:focus-visible { outline: none; box-shadow: var(--focus-ring); }

.aie__blocked {
  flex: 1; display: flex; flex-direction: column; align-items: center; justify-content: center;
  gap: 10px; padding: 32px 20px; text-align: center;
}
.aie__blocked-reason { font-size: 12px; color: var(--text-muted); max-width: 260px; line-height: 1.5; }
.aie__blocked-btn {
  display: inline-flex; align-items: center; gap: 5px;
  padding: 7px 14px; border: 1px solid var(--accent);
  border-radius: var(--radius-md); background: var(--accent); color: var(--text);
  font-size: 12px; cursor: pointer;
  transition: background var(--dur-fast, 120ms) var(--ease-standard);
}
.aie__blocked-btn:hover { background: var(--accent-hover); }

.aie__body { flex: 1; display: flex; min-height: 0; }
.aie__pane--outline {
  display: flex; flex-direction: column; min-width: 0;
  border-right: 1px solid var(--border); background: var(--bg-surface);
}
.aie__pane--outline[data-collapsed='true'] { width: 36px !important; }
.aie__outline-head {
  display: flex; align-items: center; gap: 5px; padding: 8px 10px;
  font-size: 11px; color: var(--text-muted); border-bottom: 1px solid var(--border);
}
.aie__outline-toggle {
  margin-left: auto; width: 20px; height: 20px; border: none; background: transparent;
  color: var(--text-muted); cursor: pointer; border-radius: var(--radius-xs);
}
.aie__outline-toggle:hover { background: var(--bg-hover); color: var(--text); }
.aie__outline { flex: 1; overflow-y: auto; padding: 6px 4px; display: flex; flex-direction: column; gap: 1px; }
.aie__outline-item {
  text-align: left; border: none; background: transparent; color: var(--text-muted);
  font-size: 11px; padding: 4px 8px; border-radius: var(--radius-xs); cursor: pointer;
  white-space: nowrap; overflow: hidden; text-overflow: ellipsis;
  transition: background var(--dur-fast, 120ms) var(--ease-standard), color var(--dur-fast, 120ms) var(--ease-standard);
}
.aie__outline-item:hover { background: var(--bg-hover); color: var(--text); }
.aie__outline-item--lv2 { padding-left: 18px; }
.aie__outline-item--lv3 { padding-left: 30px; }
.aie__outline-item--lv4 { padding-left: 42px; color: var(--text-dim); }
.aie__outline-empty { font-size: 11px; color: var(--text-dim); padding: 8px 10px; }

.aie__resizer {
  width: 4px; flex-shrink: 0; cursor: col-resize; background: transparent;
  transition: background var(--dur-fast, 120ms) var(--ease-standard);
}
.aie__resizer:hover, .aie__resizer:focus-visible { background: var(--accent); outline: none; }

.aie__pane--editor { flex: 1; display: flex; flex-direction: column; min-width: 0; position: relative; }
.aie__textarea {
  flex: 1; width: 100%; resize: none; border: none; outline: none;
  background: var(--bg-elevated); color: var(--text);
  font-family: var(--font-mono, monospace); font-size: 12.5px; line-height: 1.6;
  padding: 12px 14px; tab-size: 2;
}
.aie__textarea--dirty { box-shadow: inset 3px 0 0 var(--accent); }
.aie__footer {
  display: flex; align-items: center; gap: 8px; padding: 6px 12px;
  border-top: 1px solid var(--border); font-size: 11px; color: var(--text-muted);
}
.aie__meta { flex: 1; }
.aie__dirty { color: var(--warn); }
.aie__footer-actions { display: flex; align-items: center; gap: 6px; }
.aie__btn-text {
  border: none; background: transparent; color: var(--text-muted); font-size: 12px;
  cursor: pointer; padding: 5px 8px; border-radius: var(--radius-sm);
}
.aie__btn-text:hover:not(:disabled) { background: var(--bg-hover); color: var(--text); }
.aie__btn-text:disabled { opacity: 0.4; cursor: not-allowed; }
.aie__btn-primary {
  display: inline-flex; align-items: center; gap: 4px;
  border: 1px solid var(--accent); background: var(--accent); color: var(--text);
  font-size: 12px; padding: 5px 12px; border-radius: var(--radius-md); cursor: pointer;
  transition: background var(--dur-fast, 120ms) var(--ease-standard), transform var(--dur-fast, 120ms) var(--ease-standard);
}
.aie__btn-primary:hover:not(:disabled) { background: var(--accent-hover); }
.aie__btn-primary:active:not(:disabled) { transform: translateY(1px); }
.aie__btn-primary:disabled { opacity: 0.5; cursor: not-allowed; }
.aie__btn-primary:focus-visible { outline: none; box-shadow: var(--focus-ring); }

.aie__status { display: flex; align-items: center; gap: 8px; padding: 16px; font-size: 12px; color: var(--text-muted); }
.aie__status--err { color: var(--warn); }
.aie__status-retry {
  display: inline-flex; align-items: center; gap: 3px; margin-left: 6px;
  border: 1px solid color-mix(in srgb, var(--warn) 45%, transparent);
  background: transparent; color: var(--text); font-size: 11px;
  padding: 2px 8px; border-radius: var(--radius-sm); cursor: pointer;
}
.aie__save-msg {
  display: flex; align-items: center; gap: 5px; padding: 6px 12px;
  font-size: 11px; border-top: 1px solid var(--border);
}
.aie__save-msg[data-status='ok'] { color: var(--ok); }
.aie__save-msg[data-status='error'] { color: var(--danger); }

.aie__spin { animation: aie-spin 1s linear infinite; }
@keyframes aie-spin { to { transform: rotate(360deg); } }

@media (prefers-reduced-motion: reduce) {
  .aie__spin { animation: none; }
  .aie__resizer { transition: none; }
}
</style>
