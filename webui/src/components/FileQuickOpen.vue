<template>
  <Transition name="qopen">
    <div
      v-if="isOpen"
      class="quick-open-backdrop"
      role="presentation"
      @mousedown="onBackdrop"
    >
      <section
        ref="dialogRef"
        class="quick-open"
        role="dialog"
        aria-modal="true"
        :aria-label="t('quickOpen.title')"
      >
        <div class="quick-open__search">
          <!-- The magnifier sits inside the field so the input reads as the search
               box itself; dismissal stays outside it. -->
          <div class="quick-open__field">
            <Icon name="search" :size="16" class="quick-open__icon" />
            <input
              ref="inputRef"
              v-model="query"
              type="text"
              class="quick-open__input"
              :placeholder="t('quickOpen.placeholder')"
              role="combobox"
              aria-expanded="true"
              aria-controls="quick-open-listbox"
              aria-autocomplete="list"
              :aria-activedescendant="activeId"
              autocomplete="off"
              spellcheck="false"
              @keydown="onInputKeydown"
            />
          </div>
          <button
            type="button"
            class="quick-open__close"
            :aria-label="t('common.close')"
            :title="t('common.close')"
            @click="close"
          >
            <Icon name="x" :size="15" />
          </button>
        </div>

        <div
          id="quick-open-listbox"
          ref="listRef"
          class="quick-open__list"
          role="listbox"
          :aria-label="t('quickOpen.title')"
        >
          <p
            v-if="statusText"
            class="quick-open__status"
            :class="{ 'quick-open__status--error': status === 'error' }"
            aria-live="polite"
          >{{ statusText }}</p>

          <button
            v-for="(item, index) in items"
            :id="`quick-open-option-${index}`"
            :key="item.path"
            type="button"
            class="quick-open__item"
            :class="{ 'is-active': index === activeIndex }"
            role="option"
            :aria-selected="index === activeIndex"
            :aria-label="t('quickOpen.open', { name: item.name })"
            tabindex="-1"
            @click="choose(item)"
            @mousemove="activeIndex = index"
          >
            <Icon name="fileText" :size="15" class="quick-open__item-icon" />
            <span class="quick-open__item-body">
              <span class="quick-open__item-name">{{ item.name }}</span>
              <span class="quick-open__item-path">{{ item.relative || item.path }}</span>
            </span>
          </button>

          <p v-if="truncated && items.length > 0" class="quick-open__note">
            {{ t('quickOpen.truncated', { n: items.length }) }}
          </p>
        </div>

        <!-- The chord hint is only useful once there is a list to walk; the
             blank-query state already shows it as its status line. -->
        <footer v-if="items.length > 0" class="quick-open__footer">{{ t('quickOpen.hint') }}</footer>
      </section>
    </div>
  </Transition>
</template>

<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import Icon from './Icon.vue'
import { useDialogA11y } from '@/composables/useDialogA11y'
import { useRpcStore } from '@/stores/rpc'

// Ctrl/⌘+P quick-open: a centred search field over the bound project's files.
//
// Deliberately a modal, not the right-side workbench ContentSheet — the gesture
// is "type a name, press Enter, land in the editor", so it must not compete for
// layout with the workbench or the chat composer. The search itself is a
// gateway RPC (`sandbox.path.search`, another agent's contract); when the
// gateway does not advertise that method we show the unavailable state instead
// of firing a call that would 501.
//
// State model (one status line at a time): noProject → error (unsupported or
// failed) → searching → hint (blank query) → empty (no matches). Results stay
// mounted while a newer query is in flight so the list does not flash empty on
// every keystroke; the generation counter makes a slow, stale response unable
// to overwrite the newer one.

const props = defineProps<{
  open: boolean
  /** Absolute project root on the gateway host; empty means "nothing bound". */
  basePath?: string
}>()

const emit = defineEmits<{
  (e: 'close'): void
  (e: 'open', path: string): void
}>()

interface QuickOpenItem {
  name: string
  path: string
  relative?: string
  kind?: string
}

interface PathSearchResponse {
  items?: QuickOpenItem[]
  truncated?: boolean
}

const DEBOUNCE_MS = 180
const RESULT_LIMIT = 20

const { t } = useI18n()
const rpcStore = useRpcStore()

const dialogRef = ref<HTMLElement | null>(null)
const inputRef = ref<HTMLInputElement | null>(null)
const listRef = ref<HTMLElement | null>(null)
const query = ref('')
const items = ref<QuickOpenItem[]>([])
const truncated = ref(false)
const searching = ref(false)
const failed = ref(false)
const activeIndex = ref(0)

const isOpen = computed(() => props.open)
const basePath = computed(() => (props.basePath || '').trim())
// Reactive on the gateway handshake: if the sidecar/Rust endpoint appears after
// a reconnect, the modal starts working without a reload. The literal gate is
// what scripts/check-rpc-contract.mjs validates against SUPPORTED_METHODS.
const searchSupported = computed(() => rpcStore.supportsMethod('sandbox.path.search'))
const canSearch = computed(() => !!basePath.value && searchSupported.value)

const status = computed<'none' | 'noProject' | 'error' | 'searching' | 'hint' | 'empty'>(() => {
  if (!basePath.value) return 'noProject'
  if (!searchSupported.value) return 'error'
  if (failed.value) return 'error'
  if (searching.value) return 'searching'
  if (!query.value.trim()) return 'hint'
  return items.value.length === 0 ? 'empty' : 'none'
})

const statusText = computed(() => {
  switch (status.value) {
    case 'noProject': return t('quickOpen.noProject')
    case 'error': return t('quickOpen.error')
    case 'searching': return t('quickOpen.searching')
    case 'hint': return t('quickOpen.hint')
    case 'empty': return t('quickOpen.empty')
    default: return ''
  }
})

const activeId = computed(() => (
  items.value[activeIndex.value] ? `quick-open-option-${activeIndex.value}` : undefined
))

// Trap focus / Escape / restore focus to the invoking element. The input is
// passed explicitly so focus lands in the field regardless of DOM order.
useDialogA11y(dialogRef, isOpen, close, { initialFocus: inputRef })

let debounceTimer: ReturnType<typeof setTimeout> | null = null
// Monotonic generation so a slow response for an older query is dropped.
let requestGeneration = 0

function resetResults() {
  items.value = []
  truncated.value = false
  searching.value = false
  failed.value = false
  activeIndex.value = 0
}

function cancelPending() {
  if (debounceTimer) {
    clearTimeout(debounceTimer)
    debounceTimer = null
  }
  requestGeneration += 1
}

async function runSearch(q: string) {
  const generation = ++requestGeneration
  searching.value = true
  failed.value = false
  try {
    const res = await rpcStore.call<PathSearchResponse>('sandbox.path.search', {
      basePath: basePath.value,
      query: q,
      limit: RESULT_LIMIT,
    })
    if (generation !== requestGeneration) return
    items.value = Array.isArray(res?.items)
      ? res.items.filter(item => !!item && typeof item.path === 'string' && item.path !== '')
      : []
    truncated.value = res?.truncated === true
    activeIndex.value = 0
  } catch {
    if (generation !== requestGeneration) return
    items.value = []
    truncated.value = false
    failed.value = true
  } finally {
    if (generation === requestGeneration) searching.value = false
  }
}

watch(query, (raw) => {
  if (debounceTimer) {
    clearTimeout(debounceTimer)
    debounceTimer = null
  }
  const q = raw.trim()
  // Blank query / no project / unsupported method: never hit the gateway.
  if (!canSearch.value || !q) {
    requestGeneration += 1
    resetResults()
    return
  }
  debounceTimer = setTimeout(() => {
    debounceTimer = null
    void runSearch(q)
  }, DEBOUNCE_MS)
})

// Fresh state on every open so yesterday's query / results never flash; cancel
// the timer on close so a dismissed modal cannot fire a late request.
watch(isOpen, (open) => {
  cancelPending()
  if (open) {
    query.value = ''
    resetResults()
  }
})

onBeforeUnmount(cancelPending)

function scrollActiveIntoView() {
  void nextTick(() => {
    listRef.value
      ?.querySelector<HTMLElement>('.quick-open__item.is-active')
      ?.scrollIntoView?.({ block: 'nearest' })
  })
}

function move(delta: number) {
  const count = items.value.length
  if (count === 0) return
  activeIndex.value = (activeIndex.value + delta + count) % count
  scrollActiveIntoView()
}

function choose(item: QuickOpenItem | undefined) {
  if (!item) return
  emit('open', item.path)
}

function onInputKeydown(e: KeyboardEvent) {
  if (e.key === 'ArrowDown') {
    e.preventDefault()
    move(1)
  } else if (e.key === 'ArrowUp') {
    e.preventDefault()
    move(-1)
  } else if (e.key === 'Enter') {
    e.preventDefault()
    choose(items.value[activeIndex.value])
  } else if (e.key === 'Escape') {
    // useDialogA11y owns Escape at the document level; preventing the default
    // here makes its handler step aside (it bails on defaultPrevented), so the
    // modal closes exactly once.
    e.preventDefault()
    close()
  }
}

function close() {
  emit('close')
}

function onBackdrop(e: MouseEvent) {
  if (e.target === e.currentTarget) close()
}
</script>

<style scoped>
.quick-open-backdrop {
  position: fixed;
  inset: 0;
  /* Modal tier, matching the command palette: above the fixed sidebar and the
     settings overlay so the scrim dims every shell surface. */
  z-index: 320;
  display: flex;
  align-items: flex-start;
  justify-content: center;
  padding: 12vh 16px 16px;
  background: var(--scrim);
}

.quick-open {
  width: min(620px, calc(100vw - 32px));
  max-height: min(560px, calc(100vh - 18vh));
  display: flex;
  flex-direction: column;
  overflow: hidden;
  background: var(--bg-surface);
  border: 1px solid var(--border);
  border-radius: var(--radius-modal);
  box-shadow: var(--shadow-lg);
}

/* Open/close motion mirrors the palette: scrim fades, dialog slides up + fades
   (exit one tier faster). */
.qopen-enter-active { transition: opacity var(--dur-base) var(--ease-out); }
.qopen-leave-active { transition: opacity var(--dur-fast) var(--ease-in); }
.qopen-enter-from,
.qopen-leave-to { opacity: 0; }
.qopen-enter-active .quick-open {
  transition: transform var(--dur-base) var(--ease-out), opacity var(--dur-base) var(--ease-out);
}
.qopen-leave-active .quick-open {
  transition: transform var(--dur-fast) var(--ease-in), opacity var(--dur-fast) var(--ease-in);
}
.qopen-enter-from .quick-open { opacity: 0; transform: translateY(8px); }
.qopen-leave-to .quick-open { opacity: 0; transform: translateY(6px); }

.quick-open__search {
  display: flex;
  align-items: center;
  gap: var(--sp-2);
  padding: 12px 14px;
}

/* Hairline border does the work; the tint only hints at depth. */
.quick-open__field {
  display: flex;
  align-items: center;
  gap: var(--sp-2);
  flex: 1;
  min-width: 0;
  min-height: 44px;
  padding: 0 12px;
  background: var(--bg-surface);
  border: 1px solid var(--border);
  border-radius: var(--radius-control);
  transition: border-color var(--dur-fast), background var(--dur-fast);
}
.quick-open__field:focus-within {
  border-color: color-mix(in srgb, var(--accent) 70%, var(--border));
}
.quick-open__field:focus-within .quick-open__icon {
  color: var(--accent);
}

.quick-open__icon {
  flex-shrink: 0;
  color: var(--text-muted);
}

.quick-open__field > input.quick-open__input,
.quick-open__field > input.quick-open__input:focus {
  appearance: none;
  background: transparent;
  border: 0;
  border-radius: 0;
  box-shadow: none;
  color: var(--text);
  flex: 1;
  font-family: var(--font-sans);
  font-size: var(--fs-sm);
  font-weight: 400;
  line-height: 20px;
  min-height: 42px;
  min-width: 0;
  outline: none;
  padding: 0;
}
.quick-open__input::placeholder {
  color: var(--text-muted);
}

.quick-open__close {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 30px;
  height: 30px;
  flex-shrink: 0;
  padding: 0;
  background: transparent;
  border: 0;
  border-radius: var(--radius-sm);
  color: var(--text-muted);
  cursor: pointer;
  transition: background var(--dur-fast), color var(--dur-fast);
}
.quick-open__close:hover {
  background: var(--bg-hover);
  color: var(--text);
}
.quick-open__close:focus-visible {
  box-shadow: 0 0 0 2px color-mix(in srgb, var(--accent) 18%, transparent);
  outline: 1px solid var(--accent);
  outline-offset: -1px;
}

.quick-open__list {
  min-height: 0;
  overflow-y: auto;
  overscroll-behavior: contain;
  padding: 8px;
}

.quick-open__status {
  margin: 0;
  padding: 18px 10px;
  text-align: center;
  color: var(--text-muted);
  font-size: var(--fs-sm);
}
.quick-open__status--error {
  color: var(--danger);
}

.quick-open__item {
  display: flex;
  align-items: center;
  gap: var(--sp-2);
  width: 100%;
  min-height: 46px;
  padding: var(--sp-2) var(--sp-3);
  border: 1px solid transparent;
  border-radius: var(--radius-sm);
  background: transparent;
  color: var(--text);
  font-family: var(--font-sans);
  font-size: var(--fs-sm);
  font-weight: 400;
  text-align: left;
  cursor: pointer;
}

.quick-open__item-icon {
  flex-shrink: 0;
  color: var(--text-muted);
}

.quick-open__item-body {
  display: flex;
  flex: 1;
  flex-direction: column;
  gap: 1px;
  min-width: 0;
}

.quick-open__item-name {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.quick-open__item-path {
  color: var(--text-muted);
  font-family: var(--font-mono);
  font-size: var(--fs-xs);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

/* Pointed-at, not flagged: a plain neutral fill with no leading bar or tint. */
.quick-open__item.is-active {
  background: var(--bg-hover);
  border-color: transparent;
}

.quick-open__note {
  margin: 0;
  padding: 6px 10px 10px;
  color: var(--text-muted);
  font-size: var(--fs-xs);
}

.quick-open__footer {
  padding: 8px 14px;
  border-top: 1px solid var(--border);
  color: var(--text-muted);
  font-size: var(--fs-xs);
}

@media (prefers-reduced-motion: reduce) {
  .qopen-enter-active,
  .qopen-leave-active,
  .qopen-enter-active .quick-open,
  .qopen-leave-active .quick-open {
    transition: none;
  }
}

@media (max-width: 768px) {
  .quick-open-backdrop {
    padding: 8vh 12px 12px;
  }
}
</style>
