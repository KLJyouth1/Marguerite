/**
 * 推理等级（reasoning effort）单一事实源。
 *
 * 为什么单独成文件：这个轴此前有三份互不相认的表示 ——
 *   · 后端 `chat.send` 的 `effort` 参数（`core/src/gateway/chat.rs::parse_effort_param`，
 *     合法值 off|normal|deep|max，别名 low|medium|high）；
 *   · `types/rpc.ts::ChatSendParams.reasoningEffort`（**声明了却从未被发送**，死字段）；
 *   · 输入框那枚二值 `thinking` 开关（只发 `thinking: true`，表达不了 4 档）。
 * 于是 UI 只能表达「开/关」，后端能力四档白给。本文件把档位、wire 值、
 * i18n 键、排序收敛到一处，三端（composer / ChatView / chat.send）共用。
 */

export type EffortLevel = 'off' | 'normal' | 'deep' | 'max'

/** 档位由弱到强（顺序即语义强度，禁止重排：滑块位置、吸附、ARIA 值都依赖它）。 */
export const EFFORT_LEVELS: readonly EffortLevel[] = ['off', 'normal', 'deep', 'max'] as const

export interface EffortMeta {
  level: EffortLevel
  /** 0..3，滑块档位序号 */
  index: number
  /** 短标签 i18n 键 */
  labelKey: string
  /** 一句话说明 i18n 键 */
  hintKey: string
  /** 落到请求体的 `reasoning_effort` 值；`off` 不注入字段 */
  wire: string | null
}

export const EFFORT_META: Record<EffortLevel, EffortMeta> = {
  off: {
    level: 'off',
    index: 0,
    labelKey: 'chat.effort.levelOff',
    hintKey: 'chat.effort.hintOff',
    wire: null,
  },
  normal: {
    level: 'normal',
    index: 1,
    labelKey: 'chat.effort.levelNormal',
    hintKey: 'chat.effort.hintNormal',
    wire: 'low',
  },
  deep: {
    level: 'deep',
    index: 2,
    labelKey: 'chat.effort.levelDeep',
    hintKey: 'chat.effort.hintDeep',
    wire: 'medium',
  },
  max: {
    level: 'max',
    index: 3,
    labelKey: 'chat.effort.levelMax',
    hintKey: 'chat.effort.hintMax',
    wire: 'high',
  },
}

/** 后端接受的别名（与 `parse_effort_param` 保持一致，避免前端先拒掉合法值）。 */
const ALIASES: Record<string, EffortLevel> = {
  off: 'off',
  none: 'off',
  low: 'normal',
  normal: 'normal',
  medium: 'deep',
  deep: 'deep',
  high: 'max',
  max: 'max',
}

/**
 * 归一化任意输入为合法档位。
 *
 * 与后端的关键差别：后端对**未知值响亮拒绝**（成本治理不允许把 typo 静默降级）；
 * 前端这里只在读回自己写的持久化值时使用，未知值按 `off` 处理并交由调用方决定是否上报，
 * 绝不把 `higg` 当成 `deep`。
 */
export function normalizeEffort(raw: unknown): EffortLevel {
  if (typeof raw !== 'string') return 'off'
  return ALIASES[raw.trim().toLowerCase()] ?? 'off'
}

export function isEffortLevel(raw: unknown): raw is EffortLevel {
  return typeof raw === 'string' && (EFFORT_LEVELS as readonly string[]).includes(raw)
}

export function effortIndex(level: EffortLevel): number {
  return EFFORT_META[level]?.index ?? 0
}

/** 把（可能是连续拖动产生的）小数档位吸附到最近的合法档位。 */
export function effortFromIndex(index: number): EffortLevel {
  const clamped = Math.max(0, Math.min(EFFORT_LEVELS.length - 1, Math.round(index)))
  return EFFORT_LEVELS[clamped] ?? 'off'
}

/** 二值 `thinking` 开关 → 档位（旧偏好迁移用：开=深度，关=关闭）。 */
export function effortFromThinking(thinking: boolean): EffortLevel {
  return thinking ? 'deep' : 'off'
}

/** 档位 → 二值开关（保持 `thinking` 语义不倒退：非 off 即视为开）。 */
export function thinkingFromEffort(level: EffortLevel): boolean {
  return level !== 'off'
}
