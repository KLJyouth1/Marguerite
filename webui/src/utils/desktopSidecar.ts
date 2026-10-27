/**
 * 桌面壳：宿主（Python 侧车）进程的重启入口。
 *
 * # 为什么需要这个模块
 *
 * `HostExitCard` 的「重试」过去只做了一件事：重新读一次 `host.exit_evidence`。
 * 而 `useHostExitEvidence.fetchEvidence()` 在**未连接**时直接 return —— 于是
 * 宿主已经死掉、WebSocket 也断了的时候，点「重试」既不重启、也不请求、也没有
 * 任何提示，纯粹是一个死按钮（真机反馈「按了确认没有用」）。
 *
 * 桌面壳本身已经暴露了 `sidecar_status` / `sidecar_stop` / `sidecar_start`
 * 三个 Tauri 命令（`gui/src-tauri/src/lib.rs`），只是 UI 从没调用过。
 * 这里把它们收敛成一个幂等动作：
 *
 *  · `running` / `starting` / `restarting` → 不打断正在工作的侧车，直接返回；
 *  · 其它状态 → `sidecar_stop`（幂等，未在跑时失败可忽略）→ `sidecar_start`；
 *  · 浏览器形态（没有 Tauri invoke）→ 明确回 `unavailable`，由 UI 如实提示，
 *    绝不假装"已重启"。
 */

import { getTauriInvoke } from '@/utils/tauriBridge'

export interface SidecarStatusPayload {
  status?: string
  port?: number | null
  python?: string | null
  base_url?: string | null
}

export type SidecarRestartResult =
  | { ok: true; action: 'already-running' | 'restarted'; status: string }
  | { ok: false; reason: 'unavailable' | 'failed'; error?: string }

/** 这些状态代表侧车仍被守护任务接管，重启只会打断它。 */
const LIVE_SIDECAR_STATES = new Set(['running', 'starting', 'restarting'])

export async function restartDesktopSidecar(): Promise<SidecarRestartResult> {
  const invoke = getTauriInvoke()
  if (!invoke) return { ok: false, reason: 'unavailable' }

  try {
    const status = (await invoke('sidecar_status')) as SidecarStatusPayload | null
    const current = String(status?.status ?? '')
    if (LIVE_SIDECAR_STATES.has(current)) {
      return { ok: true, action: 'already-running', status: current }
    }

    // 先停：`sidecar_start` 在 guard 已持有实例时只会回报状态、不会重新拉起，
    // 而熔断（degraded）与 failed 恰恰都还持有实例。
    try {
      await invoke('sidecar_stop')
    } catch {
      // 未在运行时 stop 会失败，属正常路径，不阻断后续 start。
    }
    const started = (await invoke('sidecar_start')) as SidecarStatusPayload | null
    return { ok: true, action: 'restarted', status: String(started?.status ?? '') }
  } catch (error) {
    return {
      ok: false,
      reason: 'failed',
      error: error instanceof Error ? error.message : String(error),
    }
  }
}
