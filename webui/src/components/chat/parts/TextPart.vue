<template>
  <div>
    <div ref="rootEl" class="msg-ai-text" v-html="part.html" />
    <p v-if="missingCitationLabel" class="msg-ai-citation-warning">
      Some citations do not map to available sources: {{ missingCitationLabel }}
    </p>
  </div>
</template>

<script setup lang="ts">
import { computed, onMounted, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import type { ChatPart, SourcePart } from '@/types/parts'
import { decorateCitations } from '@/utils/chat/citations'
import { copyTextWithFallback } from '@/utils/browser'
import { usePlatform } from '@/platform'
import { requestBrowserWorkbenchOpen } from '@/workbench/browserItems'
import { getIconSvg } from '@/utils/icons'

const props = withDefaults(
  defineProps<{
    part: Extract<ChatPart, { type: 'text' }>
    sources?: SourcePart[]
    /** Streaming turn in progress: code blocks render expanded; on settle they
     * auto-collapse to the framed header only. */
    live?: boolean
  }>(),
  { sources: () => [], live: false },
)

const emit = defineEmits<{ citation: [sourceId: number] }>()

const { t } = useI18n()
const platform = usePlatform()
const rootEl = ref<HTMLDivElement | null>(null)
const missingCitationIds = ref<number[]>([])
// Persist per-block collapse choice across the repeated re-decorations that
// streaming triggers (v-html replaces the whole subtree on every flush).
const codeCollapseState = new Map<string, boolean>()

const missingCitationLabel = computed(() =>
  missingCitationIds.value.map(id => `[${id}]`).join(', '),
)

function labelFor(sourceId: number): string {
  const source = props.sources[sourceId - 1]
  return source ? source.title || source.domain : ''
}

function codeText(pre: HTMLPreElement): string {
  const code = pre.querySelector('code')
  return code?.textContent || ''
}

function createCodeCopyIcon(state: 'idle' | 'copied' | 'error'): SVGSVGElement {
  const svg = document.createElementNS('http://www.w3.org/2000/svg', 'svg')
  svg.setAttribute('viewBox', '0 0 24 24')
  svg.setAttribute('width', '15')
  svg.setAttribute('height', '15')
  svg.setAttribute('aria-hidden', 'true')
  svg.setAttribute('focusable', 'false')
  svg.setAttribute('fill', 'none')
  svg.setAttribute('stroke', 'currentColor')
  svg.setAttribute('stroke-width', '2')
  svg.setAttribute('stroke-linecap', 'round')
  svg.setAttribute('stroke-linejoin', 'round')

  if (state === 'copied') {
    svg.appendChild(svgNode('polyline', { points: '20 6 9 17 4 12' }))
    return svg
  }
  if (state === 'error') {
    svg.appendChild(svgNode('path', { d: 'M18 6 6 18' }))
    svg.appendChild(svgNode('path', { d: 'm6 6 12 12' }))
    return svg
  }

  svg.appendChild(svgNode('rect', { width: '14', height: '14', x: '8', y: '8', rx: '2', ry: '2' }))
  svg.appendChild(svgNode('path', { d: 'M4 16c-1.1 0-2-.9-2-2V4c0-1.1.9-2 2-2h10c1.1 0 2 .9 2 2' }))
  return svg
}

function svgNode(tag: string, attrs: Record<string, string>): SVGElement {
  const node = document.createElementNS('http://www.w3.org/2000/svg', tag)
  for (const [key, value] of Object.entries(attrs)) node.setAttribute(key, value)
  return node
}

function blockKey(pre: HTMLPreElement): string {
  const lang = pre.dataset.lang || ''
  const path = pre.dataset.path || ''
  return `${lang}|${path}|${codeText(pre).slice(0, 80)}`
}

function hasCodeFrame(pre: HTMLPreElement): boolean {
  return pre.parentElement?.classList.contains('code-block-frame') === true
}

function createChevronIcon(): SVGSVGElement {
  const svg = document.createElementNS('http://www.w3.org/2000/svg', 'svg')
  svg.setAttribute('viewBox', '0 0 24 24')
  svg.setAttribute('width', '14')
  svg.setAttribute('height', '14')
  svg.setAttribute('aria-hidden', 'true')
  svg.setAttribute('focusable', 'false')
  svg.setAttribute('fill', 'none')
  svg.setAttribute('stroke', 'currentColor')
  svg.setAttribute('stroke-width', '2')
  svg.setAttribute('stroke-linecap', 'round')
  svg.setAttribute('stroke-linejoin', 'round')
  svg.appendChild(svgNode('path', { d: 'm9 18 6-6-6-6' }))
  return svg
}

function setCodeBlockCopyState(
  button: HTMLButtonElement,
  state: 'idle' | 'copied' | 'error',
) {
  const label = state === 'copied'
    ? t('chat.copied')
    : state === 'error'
      ? t('chat.toast.copyFailed')
      : t('chat.copy')
  button.replaceChildren(createCodeCopyIcon(state))
  const text = document.createElement('span')
  text.className = 'code-block-copy__text'
  text.textContent = label
  button.append(text)
  button.title = label
  button.setAttribute('aria-label', label)
}

function decorateCodeBlocks() {
  const root = rootEl.value
  if (!root) return
  for (const pre of root.querySelectorAll<HTMLPreElement>('pre.code-block')) {
    if (hasCodeFrame(pre)) continue
    const lang = pre.dataset.lang || ''
    const path = pre.dataset.path || ''
    const label = path || lang || t('chat.code')
    // The framed header carries the language/path; drop the inline badge.
    pre.querySelector('.code-lang')?.remove()

    const key = blockKey(pre)
    const stored = codeCollapseState.get(key)
    const collapsed = stored === undefined ? !props.live : stored

    const frame = document.createElement('div')
    frame.className = `code-block-frame${collapsed ? ' is-collapsed' : ''}`

    const header = document.createElement('div')
    header.className = 'code-block-header'

    const toggle = document.createElement('button')
    toggle.type = 'button'
    toggle.className = 'code-block-header__toggle'
    const chevron = createChevronIcon()
    chevron.classList.add('code-block-chevron')
    const labelEl = document.createElement('span')
    labelEl.className = 'code-block-label'
    labelEl.textContent = label
    toggle.append(chevron, labelEl)
    toggle.setAttribute('aria-expanded', String(!collapsed))
    toggle.setAttribute('aria-label', collapsed ? t('chat.expand') : t('chat.collapse'))

    const copyBtn = document.createElement('button')
    copyBtn.type = 'button'
    copyBtn.className = 'code-block-copy'
    setCodeBlockCopyState(copyBtn, 'idle')
    copyBtn.addEventListener('click', async event => {
      event.preventDefault()
      event.stopPropagation()
      try {
        await copyTextWithFallback(codeText(pre))
        setCodeBlockCopyState(copyBtn, 'copied')
        copyBtn.classList.add('is-copied')
        window.setTimeout(() => {
          if (!copyBtn.isConnected) return
          setCodeBlockCopyState(copyBtn, 'idle')
          copyBtn.classList.remove('is-copied')
        }, 1600)
      } catch {
        setCodeBlockCopyState(copyBtn, 'error')
        copyBtn.classList.add('is-error')
        window.setTimeout(() => {
          if (!copyBtn.isConnected) return
          setCodeBlockCopyState(copyBtn, 'idle')
          copyBtn.classList.remove('is-error')
        }, 1600)
      }
    })

    header.append(toggle, copyBtn)
    pre.parentNode?.insertBefore(frame, pre)
    frame.append(header, pre)

    toggle.addEventListener('click', () => {
      const nowCollapsed = !frame.classList.contains('is-collapsed')
      frame.classList.toggle('is-collapsed', nowCollapsed)
      toggle.setAttribute('aria-expanded', String(!nowCollapsed))
      toggle.setAttribute('aria-label', nowCollapsed ? t('chat.expand') : t('chat.collapse'))
      codeCollapseState.set(blockKey(pre), nowCollapsed)
    })
  }
}

// When the streaming turn settles, collapse every code block to its header so
// the finished answer reads as a compact summary rather than long listings.
function collapseAllCodeFrames() {
  const root = rootEl.value
  if (!root) return
  for (const pre of root.querySelectorAll<HTMLPreElement>('pre.code-block')) {
    const frame = pre.parentElement
    if (!frame?.classList.contains('code-block-frame')) continue
    frame.classList.add('is-collapsed')
    const toggle = frame.querySelector<HTMLButtonElement>('.code-block-header__toggle')
    toggle?.setAttribute('aria-expanded', 'false')
    toggle?.setAttribute('aria-label', t('chat.expand'))
    codeCollapseState.set(blockKey(pre), true)
  }
}

function decorateBrowserLinks() {
  const root = rootEl.value
  if (
    !root
    || platform.id !== 'desktop'
    || !platform.capabilities.hasNativeWorkbenchSurfaces
  ) return
  for (const anchor of root.querySelectorAll<HTMLAnchorElement>('a[href]')) {
    if (!/^https?:/i.test(anchor.href)) continue
    const next = anchor.nextElementSibling
    if (next?.classList.contains('link-side-browser-btn')) continue
    const button = document.createElement('button')
    button.type = 'button'
    button.className = 'link-side-browser-btn'
    // Registry SVG rather than an "↗" glyph: the glyph's weight and baseline
    // shift with whatever font resolves it, and it does not scale crisply.
    button.innerHTML = getIconSvg('externalLink', 13)
    const label = t('workbench.browser.openSide')
    button.title = label
    button.setAttribute('aria-label', label)
    button.addEventListener('click', event => {
      event.preventDefault()
      event.stopPropagation()
      requestBrowserWorkbenchOpen(anchor.href)
    })
    anchor.insertAdjacentElement('afterend', button)
  }
}

// After `v-html` has applied the sanitized body, upgrade any `[n]` that maps to
// a real source into a focusable citation pill. The pass works on already-clean
// text nodes only (createElement/textContent — never innerHTML), so it adds no
// HTML sink and re-runs idempotently when the body re-renders during streaming.
function decorate() {
  const root = rootEl.value
  if (!root) return
  missingCitationIds.value = []
  decorateCitations(root, props.sources, {
    onActivate: n => emit('citation', n),
    labelFor,
    onMissingCitations: ids => {
      missingCitationIds.value = props.sources.length > 0 ? ids : []
    },
  })
  decorateCodeBlocks()
  decorateBrowserLinks()
}

onMounted(decorate)
watch(() => props.part.html, decorate, { flush: 'post' })
watch(() => props.sources, decorate, { flush: 'post' })
// A turn ending (live flips false) collapses code blocks to their headers.
watch(() => props.live, (live) => {
  if (live) return
  collapseAllCodeFrames()
}, { flush: 'post' })
</script>

<style scoped>
.msg-ai-text {
  font-size: 0.875rem;
  line-height: 1.6;
  color: var(--text);
  word-break: break-word;
  margin-bottom: 0.5rem;
}

.msg-ai-text :deep(p) { margin: 0.375rem 0; }
.msg-ai-text :deep(p:first-child) { margin-top: 0; }
.msg-ai-text :deep(ul), .msg-ai-text :deep(ol) { margin: 0.375rem 0; padding-left: 1.25rem; }
.msg-ai-text :deep(li) { margin: 0.125rem 0; }
.msg-ai-text :deep(code) {
  background: var(--bg-hover);
  padding: 0.0625rem 0.25rem;
  border-radius: var(--radius-sm);
  font-family: var(--font-mono);
  font-size: 0.8125rem;
  color: var(--text-muted);
}
.msg-ai-text :deep(pre) {
  background: var(--bg-elevated);
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  padding: 0.625rem;
  overflow-x: auto;
  margin: 0.375rem 0;
}
.msg-ai-text :deep(pre.code-block) {
  margin: 0;
  border: 0;
  border-radius: 0;
  background: transparent;
  padding: 0.625rem;
}

.msg-ai-text :deep(.code-block-frame) {
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  margin: 0.375rem 0;
  background: var(--bg-elevated);
  overflow: hidden;
}

.msg-ai-text :deep(.code-block-header) {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 0.5rem;
  padding: 0.25rem 0.375rem 0.25rem 0.25rem;
  border-bottom: 1px solid var(--border);
}

.msg-ai-text :deep(.code-block-header__toggle) {
  display: inline-flex;
  align-items: center;
  gap: 0.375rem;
  min-width: 0;
  padding: 0.125rem 0.375rem;
  border: 0;
  border-radius: var(--radius-sm);
  background: transparent;
  color: var(--text-muted);
  font: inherit;
  font-size: 0.75rem;
  cursor: pointer;
  transition: color var(--transition), background var(--transition);
}

.msg-ai-text :deep(.code-block-header__toggle:hover) {
  color: var(--text);
  background: var(--bg-hover);
}

.msg-ai-text :deep(.code-block-header__toggle:focus-visible) {
  outline: none;
  box-shadow: var(--focus-ring);
}

.msg-ai-text :deep(.code-block-label) {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  text-transform: uppercase;
  letter-spacing: 0.04em;
  font-family: var(--font-mono);
  font-size: 0.6875rem;
}

.msg-ai-text :deep(.code-block-chevron) {
  flex-shrink: 0;
  transition: transform var(--dur-fast) var(--ease-standard);
}

.msg-ai-text :deep(.code-block-frame:not(.is-collapsed) .code-block-chevron) {
  transform: rotate(90deg);
}

.msg-ai-text :deep(.code-block-copy) {
  display: inline-flex;
  align-items: center;
  gap: 0.25rem;
  padding: 0.125rem 0.375rem;
  border: 0;
  border-radius: var(--radius-sm);
  background: transparent;
  color: var(--text-muted);
  font: inherit;
  font-size: 0.6875rem;
  cursor: pointer;
  transition: color var(--transition), background var(--transition);
}

.msg-ai-text :deep(.code-block-copy:hover) {
  color: var(--text);
  background: var(--bg-hover);
}

.msg-ai-text :deep(.code-block-copy:focus-visible) {
  outline: none;
  box-shadow: var(--focus-ring);
}

.msg-ai-text :deep(.code-block-copy__text) { line-height: 1; }

.msg-ai-text :deep(.code-block-copy.is-copied) { color: var(--ok); }
.msg-ai-text :deep(.code-block-copy.is-error) { color: var(--danger); }

.msg-ai-text :deep(.code-block-frame.is-collapsed pre.code-block) { display: none; }

.msg-ai-text :deep(.link-side-browser-btn) {
  display: inline-flex;
  width: 1.35rem;
  height: 1.35rem;
  align-items: center;
  justify-content: center;
  margin-left: 0.2rem;
  padding: 0;
  border: 0;
  border-radius: var(--radius-sm);
  background: transparent;
  color: var(--text-dim);
  cursor: pointer;
  font: inherit;
  line-height: 1;
  vertical-align: text-bottom;
}

.msg-ai-text :deep(.link-side-browser-btn:hover),
.msg-ai-text :deep(.link-side-browser-btn:focus-visible) {
  background: var(--bg-hover);
  color: var(--accent);
}
.msg-ai-text :deep(pre code) {
  background: transparent;
  padding: 0;
}

.msg-ai-citation-warning {
  margin: 0.25rem 0 0.5rem;
  font-size: 0.75rem;
  line-height: 1.4;
  color: var(--text-muted);
}

/* Citation pills are injected outside Vue's template (built by decorateCitations
   with createElement), so the scoped data-v hash never lands on them — target
   them through :deep, the same mechanism the markdown elements above use. */
.msg-ai-text :deep(.citation-pill) {
  display: inline-flex;
  align-items: center;
  padding: 0 0.25rem;
  margin: 0 0.0625rem;
  font: inherit;
  font-family: var(--font-mono);
  font-size: 0.6875rem;
  line-height: 1.2;
  vertical-align: baseline;
  color: var(--text-muted);
  background: var(--bg-hover);
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  cursor: pointer;
  transition: color var(--transition), background var(--transition), border-color var(--transition);
}

.msg-ai-text :deep(.citation-pill:hover) {
  color: var(--accent);
  border-color: color-mix(in srgb, var(--accent) 45%, var(--border-strong));
  background: color-mix(in srgb, var(--accent) 10%, var(--bg-hover));
}

.msg-ai-text :deep(.citation-pill:focus-visible) {
  outline: none;
  box-shadow: var(--focus-ring);
}

@media (prefers-reduced-motion: reduce) {
  .msg-ai-text :deep(.citation-pill) {
    transition: none;
  }
}
</style>
