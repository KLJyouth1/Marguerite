<!--
  CodeMirror 6 包装组件（嵌入式代码编辑器）。

  为什么不用 Monaco：Monaco 5–10MB 且需要 worker 配置，作为「对话框旁的小面板」
  过重；CodeMirror 6 核心约 300KB、按语言懒加载，更符合工作台场景。

  设计要点：
  · 语言支持**按需动态 import**：只打开 `.rs` 不会把 Python/HTML 语法包一起加载；
  · 主题只用 CSS 变量（`--text` / `--accent` / …），跟随皮肤切换；
  · 只读/可编辑由 `EditorState.readOnly` + `EditorView.editable` 同时控制；
  · `Ctrl/Cmd+S` 交回父组件保存，不在编辑器里直接写盘。
-->
<template>
  <div ref="hostEl" class="cm-host" :class="{ 'is-readonly': readonly }" />
</template>

<script setup lang="ts">
import { onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { EditorState, Compartment } from '@codemirror/state'
import {
  EditorView,
  keymap,
  lineNumbers,
  highlightActiveLine,
  highlightActiveLineGutter,
  drawSelection,
  dropCursor,
  rectangularSelection,
  crosshairCursor,
} from '@codemirror/view'
import {
  defaultKeymap,
  history,
  historyKeymap,
  indentWithTab,
} from '@codemirror/commands'
import { searchKeymap, highlightSelectionMatches } from '@codemirror/search'
import { closeBrackets, closeBracketsKeymap } from '@codemirror/autocomplete'
import {
  bracketMatching,
  foldGutter,
  indentOnInput,
  syntaxHighlighting,
  HighlightStyle,
} from '@codemirror/language'
import { tags } from '@lezer/highlight'
import type { Extension } from '@codemirror/state'

const props = withDefaults(defineProps<{
  modelValue: string
  /** 文件路径：只用于推断语言 */
  path?: string
  readonly?: boolean
  active?: boolean
}>(), { path: '', readonly: false, active: true })

const emit = defineEmits<{
  (e: 'update:modelValue', value: string): void
  (e: 'save'): void
}>()

const hostEl = ref<HTMLElement | null>(null)
let view: EditorView | null = null
let applyingExternal = false

const languageCompartment = new Compartment()
const readonlyCompartment = new Compartment()

/** 主题：全部走主题变量，深浅皮肤自动适配 */
const themeExtension = EditorView.theme({
  '&': {
    height: '100%',
    color: 'var(--text)',
    backgroundColor: 'var(--bg-surface)',
    fontSize: '12.5px',
  },
  '.cm-scroller': {
    fontFamily: 'var(--font-mono, ui-monospace, SFMono-Regular, Menlo, Consolas, monospace)',
    lineHeight: '1.55',
    overflow: 'auto',
  },
  '.cm-content': { caretColor: 'var(--accent)', padding: '6px 0' },
  '.cm-cursor, .cm-dropCursor': { borderLeftColor: 'var(--accent)' },
  '&.cm-focused .cm-selectionBackground, .cm-selectionBackground, .cm-content ::selection': {
    backgroundColor: 'color-mix(in srgb, var(--accent) 24%, transparent)',
  },
  '.cm-gutters': {
    backgroundColor: 'var(--bg-surface)',
    color: 'var(--text-dim)',
    border: 'none',
    borderRight: '1px solid var(--border)',
  },
  '.cm-activeLineGutter': {
    backgroundColor: 'color-mix(in srgb, var(--accent) 10%, transparent)',
    color: 'var(--accent)',
  },
  '.cm-activeLine': {
    backgroundColor: 'color-mix(in srgb, var(--accent) 5%, transparent)',
  },
  '.cm-selectionMatch': {
    backgroundColor: 'color-mix(in srgb, var(--warn) 22%, transparent)',
  },
  '.cm-matchingBracket, .cm-nonmatchingBracket': {
    backgroundColor: 'color-mix(in srgb, var(--accent) 18%, transparent)',
    outline: '1px solid color-mix(in srgb, var(--accent) 45%, transparent)',
  },
  '.cm-foldPlaceholder': {
    backgroundColor: 'var(--bg-hover)',
    border: '1px solid var(--border)',
    color: 'var(--text-muted)',
  },
  '.cm-panels': {
    backgroundColor: 'var(--bg-elevated)',
    color: 'var(--text)',
    borderColor: 'var(--border)',
  },
  '.cm-searchMatch': {
    backgroundColor: 'color-mix(in srgb, var(--warn) 20%, transparent)',
  },
  '.cm-searchMatch.cm-searchMatch-selected': {
    backgroundColor: 'color-mix(in srgb, var(--accent) 30%, transparent)',
  },
  '.cm-tooltip': {
    backgroundColor: 'var(--bg-elevated)',
    border: '1px solid var(--border)',
    color: 'var(--text)',
  },
})

/** 令牌色：与站点既有语义色对齐（关键字=强调、字符串=成功、数字=警告） */
const highlightStyle = syntaxHighlighting(HighlightStyle.define([
  { tag: [tags.keyword, tags.controlKeyword, tags.moduleKeyword], color: 'var(--accent)' },
  { tag: [tags.string, tags.special(tags.string)], color: 'var(--ok)' },
  { tag: [tags.number, tags.bool, tags.null, tags.atom], color: 'var(--warn)' },
  { tag: [tags.comment], color: 'var(--text-dim)', fontStyle: 'italic' },
  { tag: [tags.function(tags.variableName), tags.function(tags.propertyName)], color: 'color-mix(in srgb, var(--accent) 72%, var(--text))' },
  { tag: [tags.typeName, tags.className, tags.namespace], color: 'var(--accent-secondary, var(--accent))' },
  { tag: [tags.propertyName, tags.attributeName], color: 'var(--text)' },
  { tag: [tags.operator, tags.punctuation], color: 'var(--text-muted)' },
  { tag: [tags.heading, tags.strong], color: 'var(--text)', fontWeight: '600' },
  { tag: [tags.link, tags.url], color: 'var(--info, var(--accent))', textDecoration: 'underline' },
]))

const saveKeymap = keymap.of([
  {
    key: 'Mod-s',
    preventDefault: true,
    run: () => {
      emit('save')
      return true
    },
  },
  ...closeBracketsKeymap,
  ...defaultKeymap,
  ...historyKeymap,
  ...searchKeymap,
  indentWithTab,
])

function baseExtensions(): Extension[] {
  return [
    lineNumbers(),
    highlightActiveLineGutter(),
    highlightActiveLine(),
    foldGutter(),
    drawSelection(),
    dropCursor(),
    rectangularSelection(),
    crosshairCursor(),
    history(),
    indentOnInput(),
    bracketMatching(),
    closeBrackets(),
    highlightSelectionMatches(),
    saveKeymap,
    highlightStyle,
    EditorState.tabSize.of(2),
    themeExtension,
  ]
}

/** 按扩展名懒加载语言包；未知类型保持纯文本，不报错。 */
async function languageFor(path: string): Promise<Extension[]> {
  const ext = path.split('.').pop()?.toLowerCase() || ''
  try {
    if (['js', 'jsx', 'mjs', 'cjs', 'ts', 'tsx'].includes(ext)) {
      const { javascript } = await import('@codemirror/lang-javascript')
      return [javascript({ typescript: ext === 'ts' || ext === 'tsx', jsx: ext === 'jsx' || ext === 'tsx' })]
    }
    if (ext === 'py') {
      const { python } = await import('@codemirror/lang-python')
      return [python()]
    }
    if (ext === 'rs') {
      const { rust } = await import('@codemirror/lang-rust')
      return [rust()]
    }
    if (['html', 'htm', 'vue'].includes(ext)) {
      const { html } = await import('@codemirror/lang-html')
      return [html()]
    }
    if (['css', 'scss', 'less'].includes(ext)) {
      const { css } = await import('@codemirror/lang-css')
      return [css()]
    }
    if (['json', 'jsonc'].includes(ext)) {
      const { json } = await import('@codemirror/lang-json')
      return [json()]
    }
    if (['md', 'markdown'].includes(ext)) {
      const { markdown } = await import('@codemirror/lang-markdown')
      return [markdown()]
    }
    if (['yaml', 'yml'].includes(ext)) {
      const { yaml } = await import('@codemirror/lang-yaml')
      return [yaml()]
    }
  } catch {
    // 语言包加载失败（离线/构建裁剪）时退回纯文本，不阻塞打开文件
  }
  return []
}

async function applyLanguage(): Promise<void> {
  if (!view) return
  const lang = await languageFor(props.path)
  // 动态 import 期间组件可能已卸载（切标签 / 关面板）：此时 view 已销毁，不得再 dispatch。
  if (!view) return
  view.dispatch({ effects: languageCompartment.reconfigure(lang) })
}

function readonlyExtension(): Extension {
  return [EditorState.readOnly.of(props.readonly), EditorView.editable.of(!props.readonly)]
}

onMounted(async () => {
  if (!hostEl.value) return
  view = new EditorView({
    parent: hostEl.value,
    state: EditorState.create({
      doc: props.modelValue,
      extensions: [
        ...baseExtensions(),
        languageCompartment.of([]),
        readonlyCompartment.of(readonlyExtension()),
        EditorView.updateListener.of((update) => {
          if (!update.docChanged || applyingExternal) return
          emit('update:modelValue', update.state.doc.toString())
        }),
      ],
    }),
  })
  await applyLanguage()
  if (props.active) view.focus()
})

watch(() => props.modelValue, (value) => {
  if (!view) return
  const current = view.state.doc.toString()
  if (current === value) return
  applyingExternal = true
  view.dispatch({ changes: { from: 0, to: current.length, insert: value } })
  applyingExternal = false
})

watch(() => props.readonly, () => {
  view?.dispatch({ effects: readonlyCompartment.reconfigure(readonlyExtension()) })
})

watch(() => props.path, () => { void applyLanguage() })

watch(() => props.active, (active) => {
  if (active && view) view.focus()
})

onBeforeUnmount(() => {
  view?.destroy()
  view = null
})

defineExpose({
  focus: () => view?.focus(),
  /** 当前光标所在行（1-based），供状态栏显示 */
  cursorLine: (): number => {
    if (!view) return 1
    return view.state.doc.lineAt(view.state.selection.main.head).number
  },
})
</script>

<style scoped>
.cm-host {
  height: 100%;
  min-height: 0;
  overflow: hidden;
}
.cm-host :deep(.cm-editor) { height: 100%; }
.cm-host :deep(.cm-editor.cm-focused) { outline: none; }
</style>
