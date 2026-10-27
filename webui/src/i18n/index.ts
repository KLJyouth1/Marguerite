import { watch } from 'vue'
import { createI18n } from 'vue-i18n'
import en from '@/locales/en.json'

// Single i18n instance for the whole app. Composition mode (legacy:false) so
// `i18n.global.locale` is a writable ref and `t()` is reactive — calling it
// inside a computed (e.g. the nav builders) re-runs that computed on locale
// change. English is bundled eagerly as the fallback so the first paint is
// never untranslated and any missing key degrades to English; every other
// locale is a lazy dynamic-import chunk loaded on demand by loadLocaleMessages.

export const SUPPORTED_LOCALES = ['en', 'zh-Hans', 'ja', 'fr', 'de', 'es'] as const
export type LocaleCode = (typeof SUPPORTED_LOCALES)[number]
// Literal 'en' (not widened to LocaleCode) so Exclude<LocaleCode, …> below
// resolves to the non-en locales rather than never.
export const DEFAULT_LOCALE = 'en' satisfies LocaleCode

type MessageSchema = typeof en

const i18n = createI18n({
  legacy: false,
  globalInjection: true,
  locale: DEFAULT_LOCALE,
  fallbackLocale: DEFAULT_LOCALE,
  // Cast so vue-i18n infers the full locale union (en is the only EAGER bundle;
  // zh-Hans is registered later via setLocaleMessage). Without this it would
  // type the locale as just 'en' and reject setting it to 'zh-Hans'.
  messages: { en } as Record<LocaleCode, MessageSchema>,
})

export default i18n

// Non-setup modules (stores, the router guard, nav builders) import this `t`
// instead of useI18n(), which only works inside component setup.
export const t = i18n.global.t

// Static import map — one discrete chunk per non-en locale. A fully dynamic
// `import('@/locales/' + code)` would emit a broad glob and risk bundling en
// into the lazy chunk, so each locale gets an explicit specifier.
const loaders: Record<Exclude<LocaleCode, typeof DEFAULT_LOCALE>, () => Promise<{ default: MessageSchema }>> = {
  'zh-Hans': () => import('@/locales/zh-Hans.json'),
  ja: () => import('@/locales/ja.json'),
  fr: () => import('@/locales/fr.json'),
  de: () => import('@/locales/de.json'),
  es: () => import('@/locales/es.json'),
}

const loaded = new Set<LocaleCode>([DEFAULT_LOCALE])

// ---------------------------------------------------------------------------
// 首屏标题就绪判定
// ---------------------------------------------------------------------------
// initLocale 的顺序是「先 loadLocaleMessages(目标语言)，再把 locale 切过去」。
// 在这两步之间（以及更早）routeTitle() 只能回退到英文 meta.title，此时写
// document.title 会在语言包就绪后再被改成中文，造成「英文 → 中文」跳变。
// 所以记录最近一次解析出的目标语言：只有「语言包已注册 + locale 已是目标语言」
// 才算标题可写。
let titleTargetLocale: LocaleCode | null = null
let localeResolutionSettled = false
let titleReady = false
let resolveTitleReady: (() => void) | null = null
const titleReadyPromise = new Promise<void>((resolve) => { resolveTitleReady = resolve })

function maybeSettleTitleReady() {
  if (titleReady) return
  const target = titleTargetLocale
  if (!localeResolutionSettled || !target) return
  if (i18n.global.locale.value !== target || !loaded.has(target)) return
  titleReady = true
  resolveTitleReady?.()
  resolveTitleReady = null
}

// app store 的 applyLocale 直接给 locale 赋值，没有回调可挂，只能用同步 watch
// 捕获那次赋值。en 默认语言不会变，由 loadLocaleMessages 的 settle 分支覆盖。
watch(i18n.global.locale, maybeSettleTitleReady, { flush: 'sync' })

/** 首屏语言是否已确定且对应语言包已注册（此时写标题不会再有语言跳变）。 */
export function isI18nReady(): boolean {
  return titleReady
}

/** 首屏语言就绪的一次性 promise：用于补写首屏期间刻意跳过的页面标题。 */
export function whenI18nReady(): Promise<void> {
  return titleReadyPromise
}

export function isSupportedLocale(value: unknown): value is LocaleCode {
  return typeof value === 'string' && (SUPPORTED_LOCALES as readonly string[]).includes(value)
}

/** Map an arbitrary BCP-47 tag to a supported locale, or null if unsupported. */
export function normalizeLocale(raw: string | null | undefined): LocaleCode | null {
  if (!raw) return null
  const lower = raw.toLowerCase()
  if (lower === 'en' || lower.startsWith('en-') || lower.startsWith('en_')) return 'en'
  if (lower.startsWith('zh')) return 'zh-Hans'
  if (lower.startsWith('ja')) return 'ja'
  if (lower.startsWith('fr')) return 'fr'
  if (lower.startsWith('de')) return 'de'
  if (lower.startsWith('es')) return 'es'
  return null
}

/** Load and register a locale's messages (idempotent). en is always present. */
export async function loadLocaleMessages(code: LocaleCode): Promise<void> {
  // 目标语言由 initLocale / setLocale 给出：标题是否可写以此为准。
  titleTargetLocale = code
  if (loaded.has(code)) {
    localeResolutionSettled = true
    maybeSettleTitleReady()
    return
  }
  const loader = loaders[code as Exclude<LocaleCode, typeof DEFAULT_LOCALE>]
  if (!loader) return
  try {
    const mod = await loader()
    i18n.global.setLocaleMessage(code, mod.default)
    loaded.add(code)
  } finally {
    localeResolutionSettled = true
    // 加载失败时调用方会回退到当前 locale（initLocale 兜底 en）：把目标改成当前
    // locale，否则标题会永远停在「未就绪」而再也不更新。
    if (!loaded.has(code)) titleTargetLocale = i18n.global.locale.value as LocaleCode
    maybeSettleTitleReady()
  }
}

/**
 * Resolve the initial locale, first match wins:
 *   1. saved preference (localStorage)
 *   2. host OS locale (Platform.getOsLocale, desktop) — passed in by the store
 *   3. server/desktop-injected #Marguerite-data data-locale
 *   4. <html lang> (set by the gateway template / pre-paint guard)
 *   5. navigator.languages
 *   6. DEFAULT_LOCALE
 * The OS locale precedes data-locale because the desktop gateway injects a
 * default-`en` data-locale; the actual OS preference must win over it.
 */
export function resolveInitialLocale(osLocale?: string | null): LocaleCode {
  try {
    const saved = localStorage.getItem('marguerite-locale')
    if (isSupportedLocale(saved)) return saved
  } catch {
    // localStorage unavailable (private mode) — fall through
  }

  const os = normalizeLocale(osLocale)
  if (os) return os

  try {
    const data = document.getElementById('Marguerite-data')?.dataset.locale
    if (isSupportedLocale(data)) return data
  } catch {
    // ignore
  }

  const htmlLang = normalizeLocale(document.documentElement.lang)
  if (htmlLang) return htmlLang

  const navs = navigator.languages && navigator.languages.length
    ? navigator.languages
    : [navigator.language]
  for (const tag of navs) {
    const norm = normalizeLocale(tag)
    if (norm) return norm
  }

  return DEFAULT_LOCALE
}
