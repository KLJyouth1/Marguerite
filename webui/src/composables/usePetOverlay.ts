// 桌面桌宠悬浮窗（Tauri 独立窗口 pet-overlay）开关。
//
// 背景（真机缺陷，2026-10-24）：壳里明明有 `pet_overlay_show/hide/toggle` 三条命令
// （gui/src-tauri/src/lib.rs），但 WebUI 侧既没有桥接、也没有任何按钮调用 ——
// 用户在「设置 → 宠物」打开「桌面桌宠」只点亮了**页内**桌宠，桌面右下角什么都不会出现，
// 于是反馈「桌宠不显示」。
//
// 诚实原则：命令缺失（Web 端 / 旧壳）时返回失败原因，UI 如实置灰并说明，
// 不写 localStorage 假装成功，也不乐观显示「已开启」。
const OVERLAY_KEY = 'pet.overlayEnabled'

function desktopApi(): MargueriteDesktopApi | undefined {
  return typeof window === 'undefined' ? undefined : window.MargueriteDesktop
}

/** 当前环境是否支持桌面悬浮窗（真壳且桥已暴露命令）。 */
export function petOverlayAvailable(): boolean {
  const api = desktopApi()
  return typeof api?.petOverlayShow === 'function' && typeof api?.petOverlayHide === 'function'
}

/** 用户上次选择是否为「开启」（仅表示意图，实际是否可见以命令结果为准）。 */
export function petOverlayPreferred(): boolean {
  try {
    return localStorage.getItem(OVERLAY_KEY) === 'true'
  } catch {
    return false
  }
}

export async function setPetOverlayEnabled(on: boolean): Promise<{ ok: boolean; error?: string }> {
  const api = desktopApi()
  try {
    if (on) {
      if (typeof api?.petOverlayShow !== 'function') return { ok: false, error: 'desktop-shell-required' }
      await api.petOverlayShow()
    } else {
      if (typeof api?.petOverlayHide !== 'function') return { ok: false, error: 'desktop-shell-required' }
      await api.petOverlayHide()
    }
    localStorage.setItem(OVERLAY_KEY, String(on))
    return { ok: true }
  } catch (err) {
    return { ok: false, error: err instanceof Error ? err.message : String(err) }
  }
}

export interface PetOverlayToggleResult {
  ok: boolean
  /** 命令返回后悬浮窗是否可见（以壳的返回值为准，不做乐观假设）。 */
  visible: boolean
  error?: string
}

/**
 * 切换桌面悬浮窗显隐。
 *
 * 真壳的 `pet_overlay_toggle`（gui/src-tauri/src/lib.rs）语义：窗口可见时执行 hide 并返回
 * `created: false`；否则执行 show（新建或置前）并返回 `created: true`。
 * 旧壳只有 show/hide：按上次意图取反后走既有 `setPetOverlayEnabled`，失败如实返回原因。
 */
export async function togglePetOverlay(): Promise<PetOverlayToggleResult> {
  const api = desktopApi()
  const previous = petOverlayPreferred()
  try {
    if (typeof api?.petOverlayToggle === 'function') {
      const res = await api.petOverlayToggle()
      const visible = Boolean(res?.created)
      try { localStorage.setItem(OVERLAY_KEY, String(visible)) } catch { /* private mode */ }
      return { ok: true, visible }
    }
    const next = !previous
    const res = await setPetOverlayEnabled(next)
    return res.ok
      ? { ok: true, visible: next }
      : { ok: false, visible: previous, error: res.error }
  } catch (err) {
    return {
      ok: false,
      visible: previous,
      error: err instanceof Error ? err.message : String(err),
    }
  }
}

/**
 * 应用启动时恢复上次选择（幂等：窗口已存在则 show 置前）。
 * 悬浮窗自身也会加载本应用，必须由调用方排除 /pet-overlay 路由，避免自我唤醒。
 */
export async function restorePetOverlayPreference(): Promise<void> {
  if (!petOverlayPreferred() || !petOverlayAvailable()) return
  try {
    await window.MargueriteDesktop?.petOverlayShow?.()
  } catch {
    /* 启动期失败不打扰用户：设置页仍显示开关状态，可手动重试 */
  }
}
