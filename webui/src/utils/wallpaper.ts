// 壁纸的持久化、「冷启动恢复」与「主题×壁纸对比度守卫」单一事实源。
//
// 为什么必须独立成模块：壁纸状态（body 背景 + `html[data-wallpaper]` 标记）此前
// 只在 `/wallpaper` 视图 mount 时恢复。用户重启应用后落在 `/chat`，于是**已选的
// 壁纸不生效**，直到再次进入壁纸页 —— 「重启后数据状态一致」这条验收点因此失败。
// 现在 App.vue 在挂载时调用 `restoreStoredWallpaper()`，任何入口都一致。
//
// 【2026-10-23 素材与可读性治理】
// - 预设里的「专属影像」原本是带第三方水印的位图；现改为**主题令牌渐变**（零字节、
//   随主题换色、无水印），id 保持 `marguerite` 以兼容已有本地快照。
// - 新增「面板遮罩」强度：壁纸越透越好看，但正文对比度会被背景吃掉。这里提供
//   可持久化的强度 + **对比度下限守卫**（图片壁纸取样亮度 → 自动抬高面板不透明度）。

export const WALLPAPER_STORAGE_KEY = 'marguerite.wallpaper'
export const WALLPAPER_VEIL_KEY = 'marguerite.wallpaper.veil'
export const WALLPAPER_MAX_PERSIST_BYTES = 2 * 1024 * 1024

/** 面板不透明度（%）：越大越遮住壁纸、正文越稳；越小壁纸越明显。 */
export const WALLPAPER_PANEL_ALPHA_MIN = 45
export const WALLPAPER_PANEL_ALPHA_MAX = 95
export const WALLPAPER_PANEL_ALPHA_DEFAULT = 62
/** 聊天页用侧栏底色混合，视觉上略更实一点。 */
export const WALLPAPER_CHAT_ALPHA_OFFSET = 8

/** WCAG AA 正文对比度下限 */
export const WALLPAPER_MIN_CONTRAST = 4.5

export interface WallpaperPreset {
  id: string
  label: string
  css: string
}

/**
 * 预设壁纸库（6 款：1 款专属星夜 + 5 款主题令牌渐变）。
 *
 * 走主题令牌（`--accent` / `--bg` / `--warn` / `--info` / `--card`）而不是写死色值：
 * 切主题时壁纸渐变会随主题一起变，不会出现「深色主题配浅色壁纸」的突兀。
 */
export const WALLPAPER_PRESETS: readonly WallpaperPreset[] = [
  {
    // 保持 id='marguerite'：旧版本已把它写进 localStorage，换素材不能换 id。
    id: 'marguerite',
    label: '玛格丽特 · 星夜（专属）',
    css:
      'radial-gradient(120% 90% at 78% 16%, color-mix(in srgb, var(--accent) 42%, transparent) 0%, transparent 58%), '
      + 'radial-gradient(90% 70% at 16% 84%, color-mix(in srgb, var(--info) 28%, transparent) 0%, transparent 62%), '
      + 'linear-gradient(160deg, var(--bg) 0%, color-mix(in srgb, var(--accent) 14%, var(--bg)) 55%, var(--bg) 100%)',
  },
  { id: 'aurora', label: '极光 · 蓝橙', css: 'linear-gradient(135deg, color-mix(in srgb, var(--accent) 85%, var(--bg)) 0%, color-mix(in srgb, var(--accent) 25%, var(--bg)) 45%, var(--bg) 100%)' },
  { id: 'dusk', label: '黄昏 · 墨彩', css: 'linear-gradient(160deg, var(--bg) 0%, color-mix(in srgb, var(--warn) 22%, var(--bg)) 55%, color-mix(in srgb, var(--accent) 12%, var(--bg)) 100%)' },
  { id: 'ink', label: '水墨 · 青灰', css: 'linear-gradient(120deg, var(--bg) 20%, color-mix(in srgb, var(--info) 16%, var(--bg)) 60%, var(--card) 100%)' },
  { id: 'ember', label: '余烬 · 暖橙', css: 'linear-gradient(145deg, color-mix(in srgb, var(--warn) 30%, var(--bg)) 0%, var(--bg) 70%)' },
  { id: 'night', label: '夜航 · 深蓝', css: 'radial-gradient(120% 90% at 20% 10%, color-mix(in srgb, var(--info) 30%, var(--bg)) 0%, var(--bg) 60%)' },
]

export type StoredWallpaper =
  | { type: 'preset'; id: string }
  | { type: 'image'; data: string }

export function isGradient(css: string): boolean {
  return css.startsWith('linear') || css.startsWith('radial') || css.startsWith('conic')
}

/* ────────────────────────── 面板遮罩（主题 × 壁纸可读性） ────────────────────────── */

export function clampPanelAlpha(value: number): number {
  if (!Number.isFinite(value)) return WALLPAPER_PANEL_ALPHA_DEFAULT
  return Math.min(WALLPAPER_PANEL_ALPHA_MAX, Math.max(WALLPAPER_PANEL_ALPHA_MIN, Math.round(value)))
}

export function readWallpaperVeil(): number {
  try {
    const raw = localStorage.getItem(WALLPAPER_VEIL_KEY)
    if (!raw) return WALLPAPER_PANEL_ALPHA_DEFAULT
    return clampPanelAlpha(Number(raw))
  } catch {
    return WALLPAPER_PANEL_ALPHA_DEFAULT
  }
}

export function persistWallpaperVeil(value: number): void {
  try { localStorage.setItem(WALLPAPER_VEIL_KEY, String(clampPanelAlpha(value))) } catch { /* 忽略 */ }
}

/** 把遮罩强度写到 `<html>` 的 CSS 变量上（base.css 消费）。 */
export function applyWallpaperVeil(value: number): void {
  const alpha = clampPanelAlpha(value)
  const root = document.documentElement
  root.style.setProperty('--wp-panel-alpha', `${alpha}%`)
  root.style.setProperty('--wp-chat-alpha', `${clampPanelAlpha(alpha + WALLPAPER_CHAT_ALPHA_OFFSET)}%`)
}

/** sRGB 相对亮度（WCAG 定义）。`rgb` 分量取 0..1。 */
export function relativeLuminance(r: number, g: number, b: number): number {
  const lin = (c: number) => (c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4)
  return 0.2126 * lin(r) + 0.7152 * lin(g) + 0.0722 * lin(b)
}

function contrastRatio(l1: number, l2: number): number {
  const hi = Math.max(l1, l2)
  const lo = Math.min(l1, l2)
  return (hi + 0.05) / (lo + 0.05)
}

/**
 * 对比度守卫：给定主题底色/文字色的相对亮度与壁纸平均亮度，
 * 反解「面板不透明度」下限，使（面板色 over 壁纸）与文字的对比度 ≥ 4.5。
 *
 * 纯函数、可单测。返回的是**至少**需要的不透明度（%），调用方取
 * `max(用户选择, 守卫值)`；壁纸亮度未知（渐变壁纸走主题令牌）时返回 0（不干预）。
 */
export function minPanelAlphaForContrast(
  panelLum: number,
  textLum: number,
  wallpaperLum: number | null,
): number {
  if (wallpaperLum === null || !Number.isFinite(wallpaperLum)) return 0
  const a0 = 0.35
  if (contrastRatio(panelLum, textLum) < WALLPAPER_MIN_CONTRAST) return 0
  // 面板色 alpha=a 叠在壁纸上：L(a) = a*panel + (1-a)*wallpaper（亮度近似线性）
  // 求最小的 a 使 contrast(L(a), text) >= 4.5。
  for (let a = a0; a <= 1.0001; a += 0.01) {
    const mixed = a * panelLum + (1 - a) * wallpaperLum
    if (contrastRatio(mixed, textLum) >= WALLPAPER_MIN_CONTRAST) return Math.ceil(a * 100)
  }
  return 100
}

/** 读主题令牌的底色/文字色相对亮度（依赖运行时计算样式；无 document 时返回 null）。 */
export function themeLuminance(): { panel: number; text: number } | null {
  if (typeof window === 'undefined' || typeof document === 'undefined' || !document.body) return null
  const cs = getComputedStyle(document.documentElement)
  const resolve = (raw: string): [number, number, number] | null => {
    const value = raw.trim()
    if (!value) return null
    // 令牌可能是 hex / rgb() / hsl() / color-mix()：交给浏览器解析，再读 color 的计算值。
    const probe = document.createElement('span')
    probe.style.color = value
    probe.style.display = 'none'
    document.body.appendChild(probe)
    const computed = getComputedStyle(probe).color
    probe.remove()
    const nums = (computed.match(/[\d.]+/g) || []).slice(0, 3).map(Number)
    if (nums.length < 3 || nums.some((n) => !Number.isFinite(n))) return null
    return [nums[0] / 255, nums[1] / 255, nums[2] / 255]
  }
  const p = resolve(cs.getPropertyValue('--bg-surface') || cs.getPropertyValue('--bg'))
  const t = resolve(cs.getPropertyValue('--text'))
  if (!p || !t) return null
  return {
    panel: relativeLuminance(p[0], p[1], p[2]),
    text: relativeLuminance(t[0], t[1], t[2]),
  }
}

/**
 * 图片壁纸的平均亮度（0..1）。仅对 `url(...)` 生效；渐变返回 null（主题令牌已保证对比度）。
 * 8×8 缩略采样，够用且不会阻塞主线程太久。
 */
export async function estimateWallpaperLuminance(css: string): Promise<number | null> {
  if (!css || isGradient(css)) return null
  const m = css.match(/url\(["']?(.+?)["']?\)/)
  const url = m?.[1]
  if (!url || typeof document === 'undefined') return null
  return new Promise((resolve) => {
    const img = new Image()
    img.crossOrigin = 'anonymous'
    img.onload = () => {
      try {
        const canvas = document.createElement('canvas')
        canvas.width = 8
        canvas.height = 8
        const ctx = canvas.getContext('2d')
        if (!ctx) return resolve(null)
        ctx.drawImage(img, 0, 0, 8, 8)
        const { data } = ctx.getImageData(0, 0, 8, 8)
        let sum = 0
        for (let i = 0; i < data.length; i += 4) {
          sum += relativeLuminance(data[i] / 255, data[i + 1] / 255, data[i + 2] / 255)
        }
        resolve(sum / (data.length / 4))
      } catch {
        resolve(null)
      }
    }
    img.onerror = () => resolve(null)
    img.src = url
  })
}

/* ────────────────────────── 应用 / 恢复 ────────────────────────── */

/** 把背景应用到 body，并打上 `data-wallpaper` 让内容面板转为半透明（见 base.css）。 */
export function applyWallpaperToBody(css: string, veil?: number): void {
  if (!css) return
  document.body.style.backgroundImage = isGradient(css) ? css : `url("${css}")`
  document.body.style.backgroundSize = 'cover'
  document.body.style.backgroundAttachment = 'fixed'
  document.documentElement.dataset.wallpaper = '1'
  applyWallpaperVeil(veil ?? readWallpaperVeil())
}

export function clearWallpaperFromBody(): void {
  document.body.style.backgroundImage = ''
  document.body.style.backgroundSize = ''
  document.body.style.backgroundAttachment = ''
  delete document.documentElement.dataset.wallpaper
}

/** 读取持久化选择；兼容旧格式（裸 dataURL）。损坏数据一律当作「没有壁纸」。 */
export function readStoredWallpaper(): StoredWallpaper | null {
  let raw: string | null = null
  try { raw = localStorage.getItem(WALLPAPER_STORAGE_KEY) } catch { return null }
  if (!raw) return null
  if (raw.startsWith('data:')) return { type: 'image', data: raw }
  try {
    const parsed = JSON.parse(raw) as Partial<StoredWallpaper> | null
    if (!parsed) return null
    if (parsed.type === 'preset' && typeof parsed.id === 'string') {
      return { type: 'preset', id: parsed.id }
    }
    if (parsed.type === 'image' && typeof parsed.data === 'string') {
      return { type: 'image', data: parsed.data }
    }
  } catch {
    // 损坏的数据不该让页面起不来。
  }
  return null
}

export function persistPreset(id: string): void {
  try {
    localStorage.setItem(WALLPAPER_STORAGE_KEY, JSON.stringify({ type: 'preset', id }))
  } catch { /* 配额满 / 隐私模式：保持当前会话生效即可 */ }
}

/** 图片超过 2MB 不落盘（localStorage 配额），返回是否持久化成功。 */
export function persistImage(dataUrl: string): boolean {
  const bytes = Math.ceil((dataUrl.length - dataUrl.indexOf(',') - 1) * 3 / 4)
  if (bytes > WALLPAPER_MAX_PERSIST_BYTES) return false
  try {
    localStorage.setItem(WALLPAPER_STORAGE_KEY, JSON.stringify({ type: 'image', data: dataUrl }))
    return true
  } catch { return false }
}

export function clearStoredWallpaper(): void {
  try { localStorage.removeItem(WALLPAPER_STORAGE_KEY) } catch { /* 忽略 */ }
}

/** 解析已持久化的壁纸为可直接应用的 CSS（预设 id 已下线时返回 null）。 */
export function resolveStoredCss(stored: StoredWallpaper): string | null {
  if (stored.type === 'image') return stored.data
  const preset = WALLPAPER_PRESETS.find((p) => p.id === stored.id)
  return preset ? preset.css : null
}

/**
 * 冷启动恢复：App.vue 挂载时调用一次。
 * 先把遮罩强度写进 CSS 变量（即使没有壁纸，切主题后的默认面板也一致）。
 */
export function restoreStoredWallpaper(): void {
  applyWallpaperVeil(readWallpaperVeil())
  const stored = readStoredWallpaper()
  if (!stored) return
  const css = resolveStoredCss(stored)
  if (!css) {
    // 预设已下线（改版）：清掉脏数据，避免每次启动空转。
    clearStoredWallpaper()
    return
  }
  applyWallpaperToBody(css)
}

/** 供壁纸页调用：应用 + 按「图片亮度」抬升面板不透明度（对比度守卫）。 */
export async function applyWallpaperGuarded(css: string, veil: number): Promise<{ applied: number; guard: number }> {
  const requested = clampPanelAlpha(veil)
  const lum = await estimateWallpaperLuminance(css)
  const theme = themeLuminance()
  const guard = theme ? minPanelAlphaForContrast(theme.panel, theme.text, lum) : 0
  const applied = clampPanelAlpha(Math.max(requested, guard))
  applyWallpaperToBody(css, applied)
  return { applied, guard }
}
