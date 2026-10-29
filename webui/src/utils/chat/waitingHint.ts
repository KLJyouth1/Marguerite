// 「在等模型但界面什么都不说」是桌面端最伤人的一种沉默。
//
// 真机审计 2026-10-29 P1-3：双标签页并行时，B 页在 A 跑模型期间干等 **27s+**，
// 屏幕上只有「仍在等待模型 · 27s」的转圈，既没有解释也没有"我能做什么"。
// 用户无法区分「卡死」与「在排队」，只能反复刷新或重发。
//
// 本函数只产出**能诚实说的那一半**：
// - ✅ 已等多少秒（秒数由调用方给的回合时钟传入，不在这里起计时器）；
// - ✅ 为什么慢：默认 Agent 模式下模型调用串行；
// - ✅ 怎么办：切 Bloom 可并行。
// - ❌ **不编"前方还有 N 个"**：`task.queued` 只有 Python 侧车会发
//   （`core/src/gateway/events.rs` 的侧车白名单里），纯 Rust 模式拿不到队列深度。
//   编一个数字比不说更糟——用户会据此判断"快到了"，然后被再晾 30 秒。
//
// 阈值 10s：审计实测首 token 在 11–14s 才到，所以这段提示的作用是**解释这十几秒**，
// 而不是催；低于 10s 弹出来只会变噪声。

/** 默认提示阈值（秒）。 */
export const WAITING_HINT_AFTER_SEC = 10

export interface WaitingHintInput {
  /** 本页是否处于流式（请求已发出、尚未结束）。 */
  streaming: boolean
  /** 是否已有可见输出：一旦有输出就不该再说「在等模型」。 */
  hasVisibleOutput: boolean
  /** 回合已耗时（秒），来自回合时钟（`streamTurnElapsedSeconds` 的去后缀版本）。 */
  elapsedSec: number
  /** 本地化函数（注入便于单测）。 */
  translate: (key: string, params: Record<string, unknown>) => string
  /** 覆盖阈值（测试用）。 */
  afterSec?: number
}

/** 返回提示文案；不该提示时返回空串（调用方据此决定是否渲染）。 */
export function waitingSerialHint(input: WaitingHintInput): string {
  const after = input.afterSec ?? WAITING_HINT_AFTER_SEC
  if (!input.streaming) return ''
  if (input.hasVisibleOutput) return ''
  const secs = Math.max(0, Math.floor(input.elapsedSec || 0))
  if (secs < after) return ''
  return input.translate('chat.stream.waitingSerialHint', { s: secs })
}
