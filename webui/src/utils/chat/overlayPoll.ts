/**
 * 任务悬浮球的兜底轮询节奏（R-17 轮询止血，照 Trae v3.4.1「后台已关闭即停止轮询」）。
 *
 * 悬浮球是一个常驻的独立 Tauri 窗口，多数时候「没有任务在跑」，而它过去无论有无
 * 任务都以 15 s 拉一次 `sessions.list { limit: 200 }` —— 一天 5760 次全量列表查询，
 * 换回来的只是一句「还是 0」。
 *
 * 这里刻意**放慢而不是停死**：新任务必然带 `task.queued` / `sessions.changed` 事件
 * 回来重新驱动（视图已订阅这些事件），但事件会丢、WS 会断，所以空闲时降到 60 s
 * 而不是 clearInterval。这个区别必须写下来，否则下一个读代码的人会以为这里可以直接停掉。
 *
 * 抽成纯函数的原因与本仓其它 util 一样：节奏是**决策**，决策要有单测守住，
 * 不然它就只是视图里一个随手可改的数字。
 */

/** 有任务在跑（或连接还没就绪）时的兜底间隔。 */
export const OVERLAY_ACTIVE_POLL_MS = 15_000

/** 空闲时的兜底间隔：网眼变大，网不撤。 */
export const OVERLAY_IDLE_POLL_MS = 60_000

/**
 * 下一次兜底轮询该等多久。
 *
 * `connected = false` 时按**原频率**：那正是需要靠轮询发现「网关回来了」的时刻，
 * 放慢会让悬浮球在重连后继续显示一份空数据。
 */
export function nextOverlayPollMs(running: boolean, connected: boolean): number {
  if (!connected) return OVERLAY_ACTIVE_POLL_MS
  return running ? OVERLAY_ACTIVE_POLL_MS : OVERLAY_IDLE_POLL_MS
}
