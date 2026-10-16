/**
 * Tauri 桌面壳桥接（非响应式工具）。
 *
 * 为什么必须"动态取"而不能静态 import：
 * web 包**没有**安装 `@tauri-apps/api` —— `components/shell/TitleBarControls.vue` 的注释
 * 已明确记载，`import('@tauri-apps/api/window')` 会让 `vite build` 直接失败。
 * 因此沿用仓内既有做法（`views/PetOverlayView.vue`）：从 `window.__TAURI__` /
 * `window.__TAURI_INTERNALS__` 上拿全局对象，再挑出 `invoke`。
 *
 * 契约：`getTauriInvoke()` 返回 `undefined` 表示"当前不在桌面壳里"（浏览器形态）。
 * 调用方应据此**隐藏入口**，而不是发起一个必然失败的调用、再把报错甩给用户。
 */

export type TauriInvoke = (
  cmd: string,
  args?: Record<string, unknown>,
) => Promise<unknown>

function tauriGlobal(): Record<string, unknown> | undefined {
  if (typeof window === 'undefined') return undefined
  const scope = window as unknown as Record<string, unknown>
  const tauri = (scope.__TAURI__ ?? scope.__TAURI_INTERNALS__) as
    | Record<string, unknown>
    | undefined
  return tauri && typeof tauri === 'object' ? tauri : undefined
}

/** 当前是否运行在 Tauri 桌面壳里（与 App.vue 的 `isDesktopShell` 同一判据）。 */
export function isDesktopShell(): boolean {
  return tauriGlobal() !== undefined
}

/** 取 `invoke`；不在桌面壳里返回 `undefined`。 */
export function getTauriInvoke(): TauriInvoke | undefined {
  const tauri = tauriGlobal()
  if (!tauri) return undefined
  try {
    // `tauri.core.invoke` 是 Tauri 2 全局注入的形状；`tauri.invoke` 是旧形状，两者都兼容。
    const coreApi: unknown = tauri.core ?? tauri.invoke
    if (typeof coreApi === 'object' && coreApi && 'invoke' in (coreApi as object)) {
      return (coreApi as { invoke?: TauriInvoke }).invoke?.bind(coreApi)
    }
    return typeof tauri.invoke === 'function'
      ? (tauri.invoke as TauriInvoke).bind(tauri)
      : undefined
  } catch {
    // 全局对象形状异常时**如实返回 undefined**，让调用方走"不可用"分支，而不是崩在渲染里。
    return undefined
  }
}
