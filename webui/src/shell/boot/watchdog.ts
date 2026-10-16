/**
 * Boot watchdog — P2 切片 1.
 *
 * The splash in `index.html` lives inside `<div id="app">`, and Vue's
 * `mount()` clears that container — so a *successful* boot removes the splash
 * with no JS at all. What this guards is the *failure* case: if the app never
 * mounts (a failed locale chunk, a throwing import, a half-served bundle), the
 * splash would otherwise sit there forever and lock the user out of the UI.
 *
 * Design (adapted from the reviewed DSH-desktop pattern to Vue):
 * - it fires only after a hard deadline, and only once the condition holds on
 *   two consecutive samples — a single slow frame never trips it;
 * - a hidden document (background tab / minimized window) never counts;
 * - it renders a static, dependency-free fallback card and then STOPS — it
 *   never reloads in a loop. Retry is user-initiated.
 *
 * Text is deliberately bilingual via `navigator.language` rather than the app's
 * i18n layer: the whole point is that the i18n bundle may be exactly what
 * failed to load, so this surface must not depend on it.
 */

/** Hard deadline before the watchdog may declare a failed boot. */
export const BOOT_DEADLINE_MS = 15_000

/** Sampling interval; two consecutive post-deadline failures are required. */
const SAMPLE_INTERVAL_MS = 500

/** The splash node id declared in index.html. */
const SPLASH_ID = 'mq-boot'

let started = false

/** Arm the watchdog. Idempotent; safe to call once, early, before mount. */
export function startBootWatchdog(): void {
  if (started) return
  started = true
  if (typeof window === 'undefined' || typeof document === 'undefined') return

  const app = document.getElementById('app')
  if (!app) return

  const t0 = performance.now()
  let consecutiveFailures = 0

  const timer = window.setInterval(() => {
    const splash = document.getElementById(SPLASH_ID)

    // Success: Vue mounted → the splash is gone (or no longer a child of #app).
    if (!splash || !app.contains(splash)) {
      window.clearInterval(timer)
      return
    }

    // A hidden document is not a failure — don't count it, don't trip.
    if (document.visibilityState !== 'visible') {
      consecutiveFailures = 0
      return
    }

    if (performance.now() - t0 < BOOT_DEADLINE_MS) return

    consecutiveFailures += 1
    if (consecutiveFailures < 2) return

    window.clearInterval(timer)
    renderFallback(app)
  }, SAMPLE_INTERVAL_MS)
}

/** Replace the stuck splash with a static, dependency-free recovery card. */
function renderFallback(app: HTMLElement): void {
  const zh = (navigator.language || '').toLowerCase().startsWith('zh')

  app.innerHTML = ''
  const wrap = document.createElement('div')
  wrap.id = 'mq-boot-fallback'
  wrap.setAttribute('role', 'alert')

  const title = document.createElement('h1')
  title.textContent = 'Marguerite'

  const msg = document.createElement('p')
  msg.textContent = zh
    ? '界面未能加载完成。'
    : 'The interface failed to finish loading.'

  const hint = document.createElement('p')
  hint.textContent = zh
    ? '可以重试，或在设置中导出诊断信息以便排查。'
    : 'You can retry, or export diagnostics from Settings to help investigate.'

  const retry = document.createElement('button')
  retry.type = 'button'
  retry.textContent = zh ? '重试' : 'Retry'
  retry.addEventListener('click', () => window.location.reload())

  wrap.append(title, msg, hint, retry)
  app.append(wrap)
}
