// Active runtime mode — Pinia store（**不是** composable 里的模块级单例）。
//
// 为什么要用 Pinia（本文件存在的唯一理由）：
// `webui/` 是路由懒加载 + 多 chunk 打包，**同一个模块会被复制进多个 chunk**。
// 实测（`webui/dist` 构建产物）：
//   * `session.event.mode.changed`（即 initActiveMode 的 hydrate + 订阅）**只出现在 `index-*.js`**；
//   * `MODE_KINDS` 的 `omni_transcend` 同时出现在 `index-*.js` 与 `ChatView-*.js`。
// 也就是说 `useActiveMode.ts` 被打了两份，**模块级 `ref(null)` 各 chunk 一份**。
// `App.vue`（主 chunk）里的 `initActiveMode()` 只 hydrate 主 chunk 那份；
// 而 `ModeHintBar`（懒加载的 `ChatView` chunk）读到的是自己那份、**永远是 null**，
// 于是 `currentKind` 为空 -> `currentMeta` 为 null -> 界面稳定显示：
//
//   "Initializing · The gateway is loading its runtime configuration..."
//
// 而顶栏（主 chunk，读的是已被 hydrate 的那份）显示的是正确状态 —— 这就是同一个页面上
// 两个组件对同一件事给出矛盾结论的真实原因。它**不是** readiness 问题：
// `rpc.state` 正常到达 `connected`，`onboarding.status` 也正常返回。
//
// Pinia 的 store 挂在 app 级实例上，跨 chunk 天然共享。
import { computed, ref } from 'vue'
import { defineStore } from 'pinia'

export interface ModePayload {
  kind?: string
  mode?: string
  label?: string
  userMode?: UserModeInfo
  userModes?: UserModeInfo[]
}

/** 单个 3×3 用户模式格位（强度 × 工作态）。 */
export interface UserModeInfo {
  key: string
  label: string
  intensity: string
  intensityLabel: string
  working: string
  workingLabel: string
  agentMode?: string
}

/** 强度档（第一轴）——与 core/src/domain ModeIntensity 对齐。 */
export const USER_MODE_INTENSITY = ['minimal', 'standard', 'extreme'] as const
/** 工作态（第二轴）——与 core/src/domain WorkingMode 对齐。 */
export const USER_MODE_WORKING = ['ask', 'plan', 'doit'] as const

/**
 * 前端 3×3 元数据（label/axes）。选中格位后经 `switchUserMode(key)` 下发；后端
 * 的 `mode.get.userMode` / `mode.changed` 广播也回填这些字段，保证跨 chunk 一致。
 * 这是**展示层**单一事实源；执行映射在 Rust（`from_user_mode`）。
 */
export const USER_MODE_META: Record<string, Omit<UserModeInfo, 'agentMode'>> = {
  minimal_ask: { key: 'minimal_ask', label: '极简·咨询', intensity: 'minimal', intensityLabel: '极简', working: 'ask', workingLabel: '咨询' },
  minimal_plan: { key: 'minimal_plan', label: '极简·方案', intensity: 'minimal', intensityLabel: '极简', working: 'plan', workingLabel: '方案' },
  minimal_doit: { key: 'minimal_doit', label: '极简·执行', intensity: 'minimal', intensityLabel: '极简', working: 'doit', workingLabel: '执行' },
  standard_ask: { key: 'standard_ask', label: '标准·咨询', intensity: 'standard', intensityLabel: '标准', working: 'ask', workingLabel: '咨询' },
  standard_plan: { key: 'standard_plan', label: '标准·方案', intensity: 'standard', intensityLabel: '标准', working: 'plan', workingLabel: '方案' },
  standard_doit: { key: 'standard_doit', label: '标准·执行', intensity: 'standard', intensityLabel: '标准', working: 'doit', workingLabel: '执行' },
  extreme_ask: { key: 'extreme_ask', label: '极致·咨询', intensity: 'extreme', intensityLabel: '极致', working: 'ask', workingLabel: '咨询' },
  extreme_plan: { key: 'extreme_plan', label: '极致·方案', intensity: 'extreme', intensityLabel: '极致', working: 'plan', workingLabel: '方案' },
  extreme_doit: { key: 'extreme_doit', label: '极致·执行', intensity: 'extreme', intensityLabel: '极致', working: 'doit', workingLabel: '执行' },
}

/**
 * 运行时执行档 → 规范 3×3 格位键（镜像 Rust `default_user_mode_for`）。
 * 用于把 `session.event.mode.changed` 广播的 ModeKind 回落成网格高亮。
 */
export function userModeKeyForKind(kind: string | null | undefined): string {
  const k = normalizeModeKind(kind)
  switch (k) {
    case 'plan':
      return 'standard_ask'
    case 'agent':
      return 'standard_plan'
    case 'ensemble':
      return 'minimal_doit'
    case 'bloom':
      return 'standard_doit'
    case 'omni-transcend':
      return 'extreme_doit'
    default:
      return 'standard_ask'
  }
}

/**
 * 把 3×3 键解析成完整元数据（含默认 agentMode），供 `mode.changed` 广播下发时补全。
 */
export function userModeInfoFor(key: string): UserModeInfo | null {
  const meta = USER_MODE_META[key]
  if (!meta) return null
  let agentMode = 'solo'
  if (meta.working === 'doit') agentMode = meta.intensity === 'extreme' ? 'cluster' : meta.intensity === 'standard' ? 'expert' : 'dual'
  else if (meta.key === 'extreme_plan') agentMode = 'expert'
  return { ...meta, agentMode }
}

/**
 * 把网关返回的 `userMode` 归一成对象。
 *
 * 为什么必须有这一步（真机缺陷，2026-10-24）：
 * `mode.get` 回的是对象 `{key,label,intensity,...}`，而 `mode.switch`
 * （core/src/gateway/mode.rs）回的是**字符串** `"standard_doit"`。
 * 两个方法共用同一份 `ModePayload`，`switchUserMode` 又直接 `store.mode = r`，
 * 于是切换成功后 `store.mode.userMode` 变成字符串 →
 *   `activeUserMode.value?.key` 为 undefined →
 *   ① 输入框胶囊退化成「模式」、3×3 网格无高亮；
 *   ② 发送门禁 `gearKey` 落到编排面板的旧副本（如 standard_plan），
 *     蜂群/团队被判「档位不允许」——用户看到的就是「切了模式却发不出去」。
 * 只接受字符串或对象两种真实形状，其它一律返回 null（不猜）。
 */
export function userModeFromWire(value: unknown): UserModeInfo | null {
  if (typeof value === 'string') return userModeInfoFor(value.trim())
  if (value && typeof value === 'object') {
    const key = (value as { key?: unknown }).key
    if (typeof key === 'string') return userModeInfoFor(key.trim())
  }
  return null
}

/**
 * 归一化任意一次 `mode.get` / `mode.switch` 载荷。
 *
 * - `userMode` 缺失（老网关只回 kind）时，用 `userModeKeyForKind` 反推规范格位，
 *   保证 UI 至少有一个真实、可核对的选中态，而不是空高亮。
 * - `kind` / `mode` 统一成字符串，避免调用方各自兼容。
 */
export function normalizeModePayload(raw: unknown): ModePayload | null {
  if (!raw || typeof raw !== 'object') return null
  const record = raw as Record<string, unknown>
  const kind = typeof record.kind === 'string'
    ? record.kind
    : typeof record.mode === 'string'
      ? record.mode
      : undefined
  const userMode = userModeFromWire(record.userMode)
    ?? (kind ? userModeInfoFor(userModeKeyForKind(kind)) : null)
  const payload: ModePayload = { ...record, kind, mode: kind }
  if (userMode) payload.userMode = userMode
  return payload
}

/**
 * Canonical runtime-mode kinds (mirrors `core/src/mode/mod.rs` `ModeKind::as_str`).
 *
 * The last entry is hyphenated on purpose: core broadcasts `omni-transcend`,
 * not the legacy `omni_transcend`. Before T05 the front-end menu advertised the
 * underscore form, so picking it sent a name `mode.switch` had to translate —
 * and a stale badge/`MODE_KINDS` mismatch made the switch look like it did nothing.
 */
export const MODE_KINDS = ['plan', 'agent', 'ensemble', 'bloom', 'omni-transcend'] as const

/**
 * Normalize a mode kind coming off the wire.
 *
 * core's older builds emitted the underscore form (`omni_transcend`) while the
 * canonical `as_str()` is hyphenated (`omni-transcend`) — see
 * `ModeKind::from_name`. Normalizing here means both forms resolve to the same
 * label/dot instead of silently degrading to the raw kind string.
 */
export function normalizeModeKind(kind: string | null | undefined): string {
  return String(kind ?? '').trim().replace(/_/g, '-')
}

export const useActiveModeStore = defineStore('activeMode', () => {
  const mode = ref<ModePayload | null>(null)
  const lastChangedFrom = ref<string | null>(null)
  /**
   * Last `mode.switch` failure message, or `null` after a successful switch.
   * Lives in the store (not a module ref) so the top-bar badge — which is in a
   * different chunk than the call sites — reads the same value.
   */
  const switchError = ref<string | null>(null)
  /** True while a `mode.switch` round-trip is in flight. */
  const switching = ref(false)

  return {
    mode,
    lastChangedFrom,
    switchError,
    switching,
    kinds: computed(() => MODE_KINDS),
  }
})
