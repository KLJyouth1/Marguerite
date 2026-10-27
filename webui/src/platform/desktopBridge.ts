/**
 * 桌面壳桥接：把 `window.MargueriteDesktop` 真正装上。
 *
 * # 背景（实机取证结论）
 *
 * `platform/capabilities.ts` 用 `window.MargueriteDesktop` 是否存在来判定
 * 「桌面端 / Web 端」。但全仓从来没有任何代码给它赋值 —— 类型声明在
 * `vite-env.d.ts`、测试里用 `setDesktopApi()` 注入，只有生产路径缺失。
 * 结果：**打包后的桌面应用一直跑在 web 平台分支**，所有 desktop-only 能力
 * （Runtime 设置、原生目录选择、桌面偏好等）全部降级或不可达。
 *
 * # 为什么在 WebUI 侧注入，而不是 Rust 初始化脚本
 *
 * Tauri 的 `initialization_script` 只有在「以代码创建窗口」时才方便挂载，而
 * 本项目主窗口来自 `tauri.conf.json`。改去 `on_page_load` 里 `eval` 会与
 * 前端模块求值竞争 —— `getPlatform()` 可能在脚本注入前就被调用。
 * 在 `main.ts` 第一行同步安装则没有竞态：任何组件 setup 之前桥已在位。
 *
 * # 诚实原则
 *
 * 只桥接**后端真实存在**的命令（`gui/src-tauri/src/lib.rs` 的 invoke_handler
 * 与已初始化的 `tauri-plugin-opener` / `tauri-plugin-dialog`）。
 * 没有对应命令的能力**不提供假实现**：`createDesktopPlatform()` 会按方法是否
 * 存在来收缩 capabilities，让 UI 走既有降级路径，而不是点了报 TypeError。
 */

import { restartDesktopSidecar } from '@/utils/desktopSidecar'
import { getTauriInvoke } from '@/utils/tauriBridge'
import type { GatewayStatus } from './types'

/** 侧车状态 → UI 契约里的网关状态。语义一一对应，不做乐观映射。 */
const SIDECAR_STATE_TO_GATEWAY: Record<string, GatewayStatus['status']> = {
  running: 'ready',
  starting: 'starting',
  restarting: 'starting',
  stopping: 'stopped',
  stopped: 'stopped',
  failed: 'error',
  degraded: 'error',
}

const UNAVAILABLE_STATUS: GatewayStatus = {
  url: '',
  port: 0,
  owned: true,
  status: 'stopped',
  logPath: '',
}

interface SidecarStatusPayload {
  status?: string
  port?: number | null
  base_url?: string | null
}

interface GatewayHealth {
  status?: string
  version?: string
}

/**
 * 读网关自身的健康探针（同源，桌面壳里 origin 就是网关）。
 *
 * 为什么不能只信 `sidecar_status`：那条命令报告的是 **Python 侧车** 的状态，
 * 而本产品允许「Rust 引擎直接托管网关」（health.engine=rust）。实测安装版：
 * 网关健康、界面就是从 18790 加载的，而 `sidecar_status` 回 `idle`
 * （`AppState.py_sidecar` 为 None），Runtime 面板因此显示「无活动网关」。
 *
 * 判定口径：**能给这个页面提供服务的那个网关，才是活动网关** —— 用同源
 * `/api/health` 取证，比任何进程内状态都直接。
 */
async function readGatewayHealth(): Promise<GatewayHealth | null> {
  if (typeof fetch !== 'function') return null
  const controller = typeof AbortController === 'function' ? new AbortController() : null
  const timer = controller ? setTimeout(() => controller.abort(), 1500) : null
  try {
    const res = await fetch('/api/health', {
      cache: 'no-store',
      ...(controller ? { signal: controller.signal } : {}),
    })
    if (!res.ok) return null
    const body = (await res.json()) as GatewayHealth
    return body && body.status === 'ok' ? body : null
  } catch {
    return null
  } finally {
    if (timer) clearTimeout(timer)
  }
}

/** 当前网关的版本串（页面就是它提供的，因此这也是产品的运行版本）。 */
let lastKnownVersion = ''

async function readGatewayStatus(): Promise<GatewayStatus> {
  const invoke = getTauriInvoke()
  if (!invoke) return { ...UNAVAILABLE_STATUS }
  try {
    // 三条信息互不依赖，并行取；日志目录/健康探针取不到时各自降级，
    // 而不是让整块状态读取失败。
    const [raw, logDir, health] = await Promise.all([
      invoke('sidecar_status') as Promise<SidecarStatusPayload | null>,
      (invoke('sidecar_log_dir') as Promise<unknown>).catch(() => ''),
      readGatewayHealth(),
    ])
    const port = Number(raw?.port ?? 0) || 0
    if (health) lastKnownVersion = String(health.version ?? '')
    return {
      // 网关在服务这个页面 ⇒ origin 就是它的真实地址与端口。
      url: health && typeof location !== 'undefined'
        ? location.origin
        : raw?.base_url || (port > 0 ? `http://127.0.0.1:${port}` : ''),
      port: health && typeof location !== 'undefined'
        ? Number(location.port) || 0
        : port,
      owned: true,
      status: health ? 'ready' : SIDECAR_STATE_TO_GATEWAY[String(raw?.status ?? '')] ?? 'stopped',
      // 与 `core/src/pyengine/sidecar.rs` 的 `sidecar_log_dir()` 同源：
      // 侧车的 stdout/stderr 落在该目录的 `sidecar-{port}.{out,err}.log`。
      logPath: typeof logDir === 'string' ? logDir : '',
    }
  } catch (error) {
    return {
      ...UNAVAILABLE_STATUS,
      status: 'error',
      error: error instanceof Error ? error.message : String(error),
    }
  }
}

/** 运行版本：优先用健康探针（同一进程的真实版本），退化为最近一次读到的值。 */
async function readAppVersion(): Promise<string | undefined> {
  const health = await readGatewayHealth()
  if (health?.version) lastKnownVersion = String(health.version)
  return lastKnownVersion || undefined
}

/**
 * 原生目录选择：走已初始化的 `tauri-plugin-dialog`。
 * 该插件命令在 v2 里的调用形状是 `plugin:dialog|open` + `{ options }`。
 */
async function chooseProjectDirectory(
  request?: { initialPath?: string },
): Promise<{ path: string } | null> {
  const invoke = getTauriInvoke()
  if (!invoke) return null
  const picked = await invoke('plugin:dialog|open', {
    options: {
      directory: true,
      multiple: false,
      ...(request?.initialPath ? { defaultPath: request.initialPath } : {}),
    },
  })
  const path = typeof picked === 'string'
    ? picked
    : Array.isArray(picked) && typeof picked[0] === 'string' ? picked[0] : null
  return path ? { path } : null
}

function buildBridge() {
  return {
    // WebView 的 locale 就是宿主 OS 的 locale；壳里没有单独的命令。
    getOsLocale: async (): Promise<string | undefined> => navigator.language || undefined,
    getGatewayStatus: readGatewayStatus,
    // 版本来自同源健康探针（与网关同进程），供 Runtime 面板「当前版本」显示；
    // 壳本身没有 updater 命令，这里不伪造更新能力。
    getAppVersion: readAppVersion,
    retryStartup: async () => {
      const result = await restartDesktopSidecar()
      return result.ok ? { ok: true } : { ok: false, error: result.error ?? result.reason }
    },
    chooseProjectDirectory,
    // 本壳**没有**原生更新器（tauri-plugin-updater 未启用）。这两个值必须如实为
    // false：否则 UpdateBanner 会以为"桌面壳托管更新"而永久隐藏更新提示，
    // 用户就再也看不到任何版本通知了。
    isAutoUpdateEnabled: async (): Promise<boolean> => false,
    isDesktopUpdateManaged: async (): Promise<boolean> => false,
    // 桌面桌宠悬浮窗：壳里真实存在 pet_overlay_show/hide/toggle（lib.rs:587/633/643）。
    // 以前桥没暴露这三条命令，UI 也就无法打开悬浮窗 —— 用户只能从托盘菜单找，
    // 「桌宠不显示」的根因之一。这里只做透传，不伪造成功。
    petOverlayShow: async () => {
      const invoke = getTauriInvoke()
      if (!invoke) throw new Error('desktop bridge unavailable')
      return await invoke('pet_overlay_show') as { created: boolean; label: string }
    },
    petOverlayHide: async () => {
      const invoke = getTauriInvoke()
      if (!invoke) throw new Error('desktop bridge unavailable')
      await invoke('pet_overlay_hide')
    },
    petOverlayToggle: async () => {
      const invoke = getTauriInvoke()
      if (!invoke) throw new Error('desktop bridge unavailable')
      return await invoke('pet_overlay_toggle') as { created: boolean; label: string }
    },
    // 视频下载器：壳里真实存在 launch_downloader（lib.rs）。只在壳里转发；
    // 找不到时把 searched 数组原样带回，UI 展示真实搜索路径而不是「失败」两个字。
    launchDownloader: async (options?: { url?: string }) => {
      const invoke = getTauriInvoke()
      if (!invoke) throw new Error('desktop bridge unavailable')
      return await invoke('launch_downloader', {
        url: options?.url ?? null,
      }) as {
        ok: boolean
        launched: boolean
        code?: string
        message?: string
        path?: string
        searched?: string[]
      }
    },
  }
}

/**
 * 幂等安装。返回是否「此刻处于桌面桥可用状态」。
 * 已有桥（真壳或测试注入）时不覆盖。
 */
export function installDesktopBridge(): boolean {
  if (typeof window === 'undefined') return false
  const target = window as unknown as {
    MargueriteDesktop?: unknown
    __TAURI__?: unknown
    __TAURI_INTERNALS__?: unknown
  }
  if (target.MargueriteDesktop) return true
  if (!getTauriInvoke()) return false
  target.MargueriteDesktop = buildBridge()
  return true
}
