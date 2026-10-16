<script setup lang="ts">
/**
 * Cross-platform custom title-bar controls for the Tauri shell.
 *
 * Renders nothing outside the Tauri shell: in the CLI gateway (pure browser) mode
 * the native window already owns the title bar, so the buttons and the drag
 * region must stay absent rather than render dead controls.
 *
 * The web bundle does not install `@tauri-apps/api` (the desktop GUI has its own
 * package.json), so the window API is reached through the runtime-injected
 * `window.__TAURI__` global, matching the existing probe in `PetOverlayView.vue`.
 * `import('@tauri-apps/api/window')` would fail `vite build` here.
 */
import { computed, ref } from 'vue'
import { useI18n } from 'vue-i18n'

interface TauriWindowHandle {
  minimize?: () => Promise<void> | void
  toggleMaximize?: () => Promise<void> | void
  isMaximized?: () => Promise<boolean> | void
  close?: () => Promise<void> | void
}

interface TauriWindowModule {
  getCurrentWindow?: () => TauriWindowHandle
}

/**
 * True only inside the Tauri shell. `__TAURI__` is the v2 global (requires
 * `withGlobalTauri`, enabled by the desktop shell); `__TAURI_INTERNALS__` is the
 * lower-level marker also used by the existing pet-overlay probe. Neither is
 * present in the CLI gateway's browser mode.
 */
function detectTauri(): boolean {
  if (typeof window === 'undefined') return false
  const scope = window as unknown as Record<string, unknown>
  return Boolean(scope.__TAURI__ || scope.__TAURI_INTERNALS__)
}

const isTauri = detectTauri()
const { t } = useI18n()
const maximized = ref(false)

const labels = computed(() => ({
  minimize: t('shell.window.minimize'),
  maximize: t('shell.window.maximize'),
  restore: t('shell.window.restore'),
  close: t('shell.window.close'),
}))

let handlePromise: Promise<TauriWindowHandle | null> | null = null

/** Resolves the current-window handle once, caching the lookup. */
function getWindowHandle(): Promise<TauriWindowHandle | null> {
  if (!isTauri) return Promise.resolve(null)
  if (!handlePromise) {
    handlePromise = (async () => {
      try {
        const scope = window as unknown as Record<string, unknown>
        const tauri = scope.__TAURI__ as { window?: TauriWindowModule } | undefined
        const getCurrentWindow = tauri?.window?.getCurrentWindow
        if (typeof getCurrentWindow !== 'function') return null
        const handle = getCurrentWindow()
        return handle ?? null
      } catch {
        // The desktop shell may expose the marker without the window module.
        return null
      }
    })()
  }
  return handlePromise
}

async function syncMaximized() {
  const handle = await getWindowHandle()
  if (!handle?.isMaximized) return
  try {
    maximized.value = Boolean(await handle.isMaximized())
  } catch {
    // Ignore: maximize state is cosmetic (it only picks the button glyph).
  }
}

async function minimize() {
  const handle = await getWindowHandle()
  try {
    await handle?.minimize?.()
  } catch (err) {
    // Surface it: a swallowed ACL rejection here looks identical to "the
    // button is not wired up", which is exactly how this feature shipped
    // broken. Keep the chrome alive, but leave a diagnostic trail.
    console.error('[title-bar] minimize() failed', err)
  }
}

async function toggleMaximize() {
  const handle = await getWindowHandle()
  try {
    await handle?.toggleMaximize?.()
    await syncMaximized()
  } catch (err) {
    console.error('[title-bar] toggleMaximize() failed', err)
  }
}

async function close() {
  const handle = await getWindowHandle()
  try {
    await handle?.close?.()
  } catch (err) {
    console.error('[title-bar] close() failed', err)
  }
}

// Keep the restore/maximize glyph accurate when the window state changes from
// outside this control (double-click drag, OS shortcuts, another monitor).
if (typeof window !== 'undefined' && isTauri) {
  void syncMaximized()
}
</script>

<template>
  <div v-if="isTauri" class="title-bar-controls">
    <button
      type="button"
      class="title-bar-controls__button"
      :aria-label="labels.minimize"
      :title="labels.minimize"
      data-testid="titlebar-minimize"
      @click="minimize"
    >
      <svg viewBox="0 0 12 12" aria-hidden="true" focusable="false">
        <path d="M2 6h8" />
      </svg>
    </button>
    <button
      type="button"
      class="title-bar-controls__button"
      :aria-label="maximized ? labels.restore : labels.maximize"
      :title="maximized ? labels.restore : labels.maximize"
      data-testid="titlebar-maximize"
      @click="toggleMaximize"
    >
      <svg viewBox="0 0 12 12" aria-hidden="true" focusable="false">
        <rect x="2.5" y="2.5" width="7" height="7" />
      </svg>
    </button>
    <button
      type="button"
      class="title-bar-controls__button title-bar-controls__button--danger"
      :aria-label="labels.close"
      :title="labels.close"
      data-testid="titlebar-close"
      @click="close"
    >
      <svg viewBox="0 0 12 12" aria-hidden="true" focusable="false">
        <path d="M3 3l6 6M9 3l-6 6" />
      </svg>
    </button>
  </div>
</template>

<style scoped>
/* Absolutely pinned to the top-right of `.topbar` (the nearest positioned
   ancestor in both the non-chat and chat modes).

   Being out of flow is the whole point: as an in-flow child it became a grid item
   of `.topbar--chat`, so it was pushed onto an implicit second row below the bar
   on `/chat` and could end up under the chat banner. Pinned, it keeps the same
   position on every route and never wraps.

   `z-index: 21` clears `.topbar`'s own `z-index: 20`, so the buttons always win
   the hit test against the teleported route actions. */
.title-bar-controls {
  position: absolute;
  top: 0;
  right: 0;
  z-index: 21;
  display: inline-flex;
  height: 100%;
  align-items: center;
  gap: 2px;
  /* `.topbar` sets `pointer-events: none` so controls can sit over the chat
     stream; the buttons are real controls and must stay clickable. */
  pointer-events: auto;
}

.title-bar-controls__button {
  display: inline-flex;
  width: 34px;
  height: 26px;
  align-items: center;
  justify-content: center;
  padding: 0;
  border: 0;
  border-radius: var(--radius-md);
  background: transparent;
  color: var(--text-dim);
  cursor: pointer;
}

.title-bar-controls__button svg {
  width: 12px;
  height: 12px;
  fill: none;
  stroke: currentcolor;
  stroke-linecap: round;
  stroke-width: 1.2;
}

.title-bar-controls__button:hover {
  background: var(--bg-hover);
  color: var(--text);
}

.title-bar-controls__button:focus-visible {
  outline: 2px solid var(--accent);
  outline-offset: -2px;
}

.title-bar-controls__button--danger:hover {
  background: color-mix(in srgb, var(--danger) 18%, transparent);
  color: var(--danger);
}
</style>