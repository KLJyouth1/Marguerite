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
