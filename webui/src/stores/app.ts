import { ref, computed, watch } from 'vue'
import { defineStore } from 'pinia'
import { getPlatform } from '@/platform'
import i18n, {
  resolveInitialLocale,
  loadLocaleMessages,
  isSupportedLocale,
  type LocaleCode,
} from '@/i18n'
import { useToasts } from '@/composables/useToasts'
import { useRpcStore } from '@/stores/rpc'
import { getManifest, isValueThemeId, normalizeThemeId, themePickerOptions } from '@/themes/registry'
import { ensureThemeWorld } from '@/themes/apply'
import {
  SIDEBAR_WIDTH_PRESETS,
  SIDEBAR_WIDTH_STORAGE_KEY,
  normalizeSidebarWidthPreference,
  parseSidebarWidthPreference,
  type SidebarWidthPreference,
} from '@/utils/sidebarLayout'

// 'system' or any registered value-theme id. The string branch keeps custom
// themes typeable while preserving autocomplete for the built-ins.
export type ThemeMode = 'light' | 'dark' | 'system' | (string & {})

/**
 * Boot-splash animation tier. Mirrors the inline script in index.html, which
 * stamps `data-boot` on <html> before first paint from the same storage key.
 */
export type BootAnimationMode = 'off' | 'minimal' | 'full'

const BOOT_ANIMATION_STORAGE_KEY = 'marguerite-boot-animation'

function hydrateBootAnimation(): BootAnimationMode {
  try {
    const raw = localStorage.getItem(BOOT_ANIMATION_STORAGE_KEY)
    if (raw === 'off' || raw === 'minimal' || raw === 'full') return raw
  } catch {
    // Unavailable storage: fall through to the default.
  }
  return 'full'
}

const LOCALE_SYNC_PENDING_KEY = 'marguerite-locale-sync-pending'

function readPendingLocaleSync(): LocaleCode | null {
  try {
    const saved = localStorage.getItem(LOCALE_SYNC_PENDING_KEY)
    return isSupportedLocale(saved) ? saved : null
  } catch {
    return null
  }
}

type FeatureWindow = Window & {
  Marguerite_FEATURES?: Record<string, boolean>
}

function hydrateSidebarWidthPreference(): SidebarWidthPreference {
  try {
    return parseSidebarWidthPreference(localStorage.getItem(SIDEBAR_WIDTH_STORAGE_KEY))
  } catch {
    return { ...SIDEBAR_WIDTH_PRESETS.default }
  }
}

/** One pending approval, ordered oldest-first. */
export interface PendingApproval {
  approvalId: string
  sessionKey: string
  tool: string
  command: string
}

/** One-shot request for ChatView to reveal and focus a pending approval card. */
export interface ApprovalFocusRequest {
  requestId: number
  approvalId: string
  sessionKey: string
}

export const useAppStore = defineStore('app', () => {
  const theme = ref<ThemeMode>('system')
  // Boot-splash tier (browser-local). Hydrated synchronously and stamped below,
  // so the store re-applies the same value the index.html inline script already
  // wrote before first paint.
  const bootAnimation = ref<BootAnimationMode>(hydrateBootAnimation())
  // Active UI locale. Browser-local storage preserves the immediate UI
  // preference; an explicit selection also syncs the Gateway-wide language
  // used for fixed channel notices. The sidebar/topbar switcher and the
  // Settings Appearance Language row both write through setLocale.
  const locale = ref<LocaleCode>('en')
  const pendingChannelNoticeLocale = ref<LocaleCode | null>(readPendingLocaleSync())
  const sidebarOpen = ref(true)
  // Browser-local layout preference, hydrated synchronously so the first
  // mounted frame uses the saved width. Viewport clamping is intentionally a
  // consumer concern: zooming or rotating must not overwrite this preference.
  const sidebarWidthPreference = ref<SidebarWidthPreference>(hydrateSidebarWidthPreference())
  // App-wide pending approvals, kept live by the gateway push events and a
  // reconnect seed fetch (App.vue). Ordered oldest-first. `approvalCount` is
  // derived from this list once it becomes the source, but `setApprovalCount`
  // still supports snapshot consumers (back-compat).
  const pendingApprovals = ref<PendingApproval[]>([])
  const approvalCountRaw = ref(0)
  const approvalFocusRequest = ref<ApprovalFocusRequest | null>(null)
  let approvalFocusRequestId = 0

  // True once App.vue has wired the live approval source (push events + seed
  // fetch). While live, `approvalCount` is derived from `pendingApprovals`;
  // before then it falls back to whatever `setApprovalCount` last wrote so
  // snapshot consumers keep working in isolation.
  const approvalsLive = ref(false)

  const approvalCount = computed(() =>
    approvalsLive.value ? pendingApprovals.value.length : approvalCountRaw.value)

  // The oldest pending approval with a routable session.
  const oldestPendingWithSession = computed<PendingApproval | null>(() =>
    pendingApprovals.value.find(item => !!item.sessionKey) ?? null)

  const systemDark = ref<boolean>(
    window.matchMedia('(prefers-color-scheme: dark)').matches
  )

  // The applied theme id written to data-theme: a value-theme id when one is
  // chosen, else the OS-resolved light/dark for 'system'.
  const resolvedTheme = computed<string>(() => {
    if (theme.value !== 'system') return theme.value
    return systemDark.value ? 'dark' : 'light'
  })

  const desktopNativeThemeSource = computed<'light' | 'dark'>(() => {
    const colorScheme = getManifest(resolvedTheme.value)?.capabilities.colorScheme
    if (colorScheme === 'light') return 'light'
    if (colorScheme === 'dark') return 'dark'
    return systemDark.value ? 'dark' : 'light'
  })

  let mq: MediaQueryList | null = null
  let mqHandler: ((e: MediaQueryListEvent) => void) | null = null
  let themeWatchStop: (() => void) | null = null
  let localeSyncPromise: Promise<void> | null = null
  let localeSyncWarningShown = false
  const rpcStore = useRpcStore()
  const { pushToast } = useToasts()

  function applyTheme() {
    const platform = getPlatform()
    document.documentElement.setAttribute('data-theme', resolvedTheme.value)
    // Lazily bring in the theme's global "world" layer (structure/type/texture)
    // if it has one; flat value themes have no world and this is a no-op.
    void ensureThemeWorld(resolvedTheme.value)
    void platform.setNativeTheme({ source: desktopNativeThemeSource.value })
  }

  function initTheme() {
    try {
      const raw = localStorage.getItem('marguerite-theme')
      // A choice persisted under an old id (e.g. 'nord'/'phosphor' before the
      // rename) is normalized to its current canonical id first, so it keeps
      // applying instead of being dropped as unknown.
      const saved = raw ? (normalizeThemeId(raw) as ThemeMode) : null
      // Valid choices come from the theme registry now (any registered value
      // theme) plus 'system' — not a hardcoded list — so a new value theme is
      // selectable without editing the store.
      if (saved && (saved === 'system' || isValueThemeId(saved))) {
        theme.value = saved
        // Persist the canonical id so the migration happens once, and the
        // anti-flash script stamps the correct theme on the next cold load.
        if (saved !== raw) {
          try { localStorage.setItem('marguerite-theme', saved) } catch {}
        }
      } else if (raw) {
        // Stale id (theme removed) or corrupt value. The index.html anti-flash
        // script stamps it verbatim pre-paint, which paints the :root dark
        // fallback — drop the key so that flash happens at most once instead of
        // on every cold load.
        localStorage.removeItem('marguerite-theme')
      }
    } catch {
      // ignore
    }

    if (!themeWatchStop) {
      themeWatchStop = watch(resolvedTheme, applyTheme, { immediate: true })
    } else {
      applyTheme()
    }

    if (mq) return // idempotent
    mq = window.matchMedia('(prefers-color-scheme: dark)')
    mqHandler = (e: MediaQueryListEvent) => {
      systemDark.value = e.matches
    }
    if (mq.addEventListener) mq.addEventListener('change', mqHandler)
    else if (mq.addListener) mq.addListener(mqHandler)
  }

  function destroyTheme() {
    if (mq && mqHandler) {
      if (mq.removeEventListener) mq.removeEventListener('change', mqHandler)
      else if (mq.removeListener) mq.removeListener(mqHandler)
    }
    mq = null
    mqHandler = null
    if (themeWatchStop) {
      themeWatchStop()
      themeWatchStop = null
    }
  }

  function setTheme(mode: ThemeMode) {
    theme.value = mode
    try { localStorage.setItem('marguerite-theme', mode) } catch {}
  }

  function cycleTheme() {
    // Cycle the basic appearance modes (Light → Dark → System), matching the
    // topbar menu — custom themes are chosen in Settings → Appearance, not
    // cycled here. From a custom theme (not in the basic set) the cycle enters
    // at the first basic mode.
    const order = themePickerOptions({ scope: 'basic' }).map((o) => o.mode)
    const idx = order.indexOf(theme.value)
    const next = order[(idx + 1) % order.length]
    setTheme(next)
  }

  // Resolve and stamp the boot-splash tier. 'full' degrades to 'minimal' when the
  // OS asks for reduced motion — boot.css carries the same media query as the
  // belt to this braces, so a mid-session preference change also stops the loop.
  // flush:'sync' keeps the DOM write synchronous (and directly testable).
  function applyBootAnimation() {
    const reduced = window.matchMedia('(prefers-reduced-motion: reduce)').matches
    const mode = reduced && bootAnimation.value === 'full' ? 'minimal' : bootAnimation.value
    document.documentElement.setAttribute('data-boot', mode)
  }

  watch(bootAnimation, applyBootAnimation, { immediate: true, flush: 'sync' })

  function setBootAnimation(mode: BootAnimationMode) {
    bootAnimation.value = mode
    try { localStorage.setItem(BOOT_ANIMATION_STORAGE_KEY, mode) } catch {}
    // Choosing Full again re-arms the post-mount constellation orchestra so
    // operators can replay the fanfare from Appearance without clearing storage.
    if (mode === 'full') {
      try { localStorage.removeItem('marguerite-boot-orchestra-seen') } catch {}
    }
  }

  function applyLocale(code: LocaleCode) {
    i18n.global.locale.value = code
    document.documentElement.setAttribute('lang', code)
    document.documentElement.setAttribute('dir', 'ltr')
  }

  function savePendingLocaleSync(code: LocaleCode) {
    pendingChannelNoticeLocale.value = code
    try { localStorage.setItem(LOCALE_SYNC_PENDING_KEY, code) } catch {}
  }

  function clearPendingLocaleSync(code: LocaleCode) {
    if (pendingChannelNoticeLocale.value !== code) return
    pendingChannelNoticeLocale.value = null
    try { localStorage.removeItem(LOCALE_SYNC_PENDING_KEY) } catch {}
  }

  function notifyLocaleSyncPending() {
    if (localeSyncWarningShown) return
    localeSyncWarningShown = true
    pushToast(i18n.global.t('settings.appearance.channelNoticeLocaleSyncPending'), { tone: 'warn' })
  }

  async function syncLocaleToGateway(
    { warnOnUnavailable = true }: { warnOnUnavailable?: boolean } = {},
  ): Promise<void> {
    if (localeSyncPromise) return localeSyncPromise
    localeSyncPromise = (async () => {
      while (pendingChannelNoticeLocale.value) {
        if (!rpcStore.isConnected || !rpcStore.supportsMethod('config.patch.safe')) {
          if (warnOnUnavailable) notifyLocaleSyncPending()
          return
        }
        const target = pendingChannelNoticeLocale.value
        try {
          await rpcStore.call('config.patch.safe', {
            patches: { 'control_ui.default_locale': target },
          })
          clearPendingLocaleSync(target)
          localeSyncWarningShown = false
        } catch {
          // Keep the latest explicit selection for the next successful connection.
          if (warnOnUnavailable) notifyLocaleSyncPending()
          return
        }
      }
    })().finally(() => {
      localeSyncPromise = null
    })
    return localeSyncPromise
  }

  watch([() => rpcStore.state, () => rpcStore.methods], ([state]) => {
    if (state === 'connected' && pendingChannelNoticeLocale.value) {
      void syncLocaleToGateway()
    }
  })

  // Resolve and apply the startup locale (saved → OS locale → data-locale →
  // <html lang> → navigator → en). Loads the locale chunk before applying so the
  // first paint is never half-translated; a failed chunk load falls back to en.
  // It does not replace the browser's saved UI preference.
  // Desktop has one native client locale, so it queues that value for the
  // Gateway-wide channel-notice setting without making a disconnected startup
  // noisy. Browser clients remain read-only until an explicit language choice.
  async function initLocale() {
    let osLocale: string | undefined
    const platform = getPlatform()
    try {
      osLocale = await platform.getOsLocale()
    } catch {
      osLocale = undefined
    }
    const resolved = resolveInitialLocale(osLocale)
    try {
      await loadLocaleMessages(resolved)
      locale.value = resolved
      applyLocale(resolved)
      if (platform.capabilities.isDesktop) {
        savePendingLocaleSync(resolved)
        void syncLocaleToGateway({ warnOnUnavailable: false })
      }
    } catch {
      locale.value = 'en'
      applyLocale('en')
    }
  }

  async function setLocale(code: LocaleCode) {
    if (!isSupportedLocale(code)) return
    let target = code
    try {
      await loadLocaleMessages(target)
    } catch {
      target = 'en'
    }
    locale.value = target
    try { localStorage.setItem('marguerite-locale', target) } catch {}
    applyLocale(target)
    savePendingLocaleSync(target)
    await syncLocaleToGateway()
  }

  function setSidebarOpen(open: boolean) {
    sidebarOpen.value = open
  }

  function toggleSidebar() {
    sidebarOpen.value = !sidebarOpen.value
  }

  /**
   * 禅模式：一键把外壳收掉，只留对话/内容区。
   *
   * 为什么用"记住并还原 sidebarOpen"而不是新增一套平行的可见性派生：
   * `App.vue` 里侧栏的渲染 / drawer / inert 判定有十余处都绑在
   * `appStore.sidebarOpen` 上，再造一个状态必然漏改（漏一处就会出现
   * "禅模式下半截侧栏还在"）。复用既有单一事实源最稳。
   */
  const zenMode = ref(false)
  let sidebarBeforeZen: boolean | null = null

  function setZenMode(on: boolean) {
    if (on === zenMode.value) return
    if (on) {
      sidebarBeforeZen = sidebarOpen.value
      sidebarOpen.value = false
    } else if (sidebarBeforeZen !== null) {
      sidebarOpen.value = sidebarBeforeZen
      sidebarBeforeZen = null
    }
    zenMode.value = on
  }

  function toggleZenMode() {
    setZenMode(!zenMode.value)
  }

  function setSidebarWidthPreference(preference: SidebarWidthPreference) {
    const normalized = normalizeSidebarWidthPreference(preference)
    sidebarWidthPreference.value = normalized
    try {
      localStorage.setItem(SIDEBAR_WIDTH_STORAGE_KEY, JSON.stringify(normalized))
    } catch {
      // Ignore unavailable browser storage; the in-memory preference still applies.
    }
  }

  function resetSidebarWidthPreference() {
    sidebarWidthPreference.value = { ...SIDEBAR_WIDTH_PRESETS.default }
    try {
      localStorage.removeItem(SIDEBAR_WIDTH_STORAGE_KEY)
    } catch {
      // Ignore unavailable browser storage.
    }
  }

  function setApprovalCount(count: number) {
    approvalCountRaw.value = count
  }

  // Replace the app-wide pending list and mark it the live source so
  // `approvalCount` derives from it. Called on the reconnect seed fetch.
  function setPendingApprovals(items: PendingApproval[]) {
    approvalsLive.value = true
    pendingApprovals.value = items
  }

  // `*.approval.requested` push: add or update by approvalId, preserving the
  // oldest-first order (new ids append to the tail).
  function upsertPendingApproval(item: PendingApproval) {
    approvalsLive.value = true
    const idx = pendingApprovals.value.findIndex(a => a.approvalId === item.approvalId)
    if (idx === -1) {
      pendingApprovals.value = [...pendingApprovals.value, item]
    } else {
      const next = pendingApprovals.value.slice()
      next[idx] = item
      pendingApprovals.value = next
    }
  }

  // `*.approval.resolved` push: drop by approvalId.
  function removePendingApproval(approvalId: string) {
    approvalsLive.value = true
    pendingApprovals.value = pendingApprovals.value.filter(a => a.approvalId !== approvalId)
    if (approvalFocusRequest.value?.approvalId === approvalId) {
      approvalFocusRequest.value = null
    }
  }

  // A deleted session cannot own an actionable approval. Apply this locally
  // as an idempotent latency guard; the Gateway remains authoritative and
  // emits the matching `*.approval.resolved` events.
  function removePendingApprovalsForSessions(sessionKeys: Iterable<string>) {
    const keys = new Set(
      [...sessionKeys]
        .map(key => String(key || '').trim())
        .filter(Boolean),
    )
    if (keys.size === 0) return
    approvalsLive.value = true
    pendingApprovals.value = pendingApprovals.value.filter(
      approval => !keys.has(approval.sessionKey),
    )
    if (
      approvalFocusRequest.value
      && keys.has(approvalFocusRequest.value.sessionKey)
    ) {
      approvalFocusRequest.value = null
    }
  }

  function requestApprovalFocus(
    approval: Pick<PendingApproval, 'approvalId' | 'sessionKey'>,
  ) {
    if (!approval.approvalId || !approval.sessionKey) return
    approvalFocusRequest.value = {
      requestId: ++approvalFocusRequestId,
      approvalId: approval.approvalId,
      sessionKey: approval.sessionKey,
    }
  }

  function clearApprovalFocusRequest(requestId: number) {
    if (approvalFocusRequest.value?.requestId === requestId) {
      approvalFocusRequest.value = null
    }
  }

  const features = ref<Record<string, boolean>>({
    tokenViz: false,
    contractDebug: false,
    // MetaSkill run-history drawer + toolbar button: on by default so the run
    // history is reachable out of the box. Operators can disable it via
    // window.Marguerite_FEATURES. The preflight + ribbon cards are always-on
    // (driven by stream events) regardless of this flag.
    metaRuns: true,
    // Application-level artifact Workbench. Operators can temporarily disable
    // it to retain the previous Drawer/lightbox flow for one release cycle.
    artifactWorkbench: true,
    ...((window as FeatureWindow).Marguerite_FEATURES || {}),
  })

  return {
    theme,
    bootAnimation,
    locale,
    pendingChannelNoticeLocale,
    resolvedTheme,
    sidebarOpen,
    sidebarWidthPreference,
    approvalCount,
    pendingApprovals,
    oldestPendingWithSession,
    approvalFocusRequest,
    features,
    initTheme,
    destroyTheme,
    setTheme,
    cycleTheme,
    setBootAnimation,
    initLocale,
    setLocale,
    syncLocaleToGateway,
    setSidebarOpen,
    toggleSidebar,
    zenMode,
    setZenMode,
    toggleZenMode,
    setSidebarWidthPreference,
    resetSidebarWidthPreference,
    setApprovalCount,
    setPendingApprovals,
    upsertPendingApproval,
    removePendingApproval,
    removePendingApprovalsForSessions,
    requestApprovalFocus,
    clearApprovalFocusRequest,
  }
})
