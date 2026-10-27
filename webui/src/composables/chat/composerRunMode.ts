import type {
  SandboxRunMode,
  SandboxSetupState,
  SandboxSetupStatusPayload,
} from '@/types/sandbox'

/**
 * 安全模式相对「真正可隔离」的降级原因。
 *
 * 老实显示的语义：用户选了安全模式就**保留**安全模式（不再静默压回 full），
 * 但 setup 未 ready 时明确告诉 UI「隔离运行时没装/不可用」，由 UI 显示
 * 「安全（待安装）」与页脚降级原因。
 */
export type ComposerRunModeDegradation =
  | 'setup_unresolved'
  | 'setup_not_installed'
  | 'setup_in_progress'
  | 'setup_failed'
  | 'setup_unavailable'

export interface ComposerRunModeResolution {
  mode: SandboxRunMode
  /** `null` 表示安全模式确实可隔离，或当前是 full 模式（无降级）。 */
  degraded: ComposerRunModeDegradation | null
}

function degradationForState(state: SandboxSetupState): ComposerRunModeDegradation | null {
  switch (state) {
    case 'ready':
      return null
    case 'not_setup':
      return 'setup_not_installed'
    case 'setting_up':
      return 'setup_in_progress'
    case 'failed':
      return 'setup_failed'
    case 'unavailable':
      return 'setup_unavailable'
    default:
      return 'setup_unavailable'
  }
}

/**
 * 解析「用户偏好 + 沙箱能力」后的展示模式。
 *
 * 关键约束（修复「设置页能选 safe、对话页却看不到」）：
 * - 只有会话运行锁（runModeLocked）才覆盖偏好；
 * - setup 未 ready **不再**把 safe 改写成 full，而是返回 safe + degraded 原因；
 * - 状态未解析时同样保留 safe（按偏好），并标记 `setup_unresolved`。
 */
export function resolveComposerRunMode(
  preference: SandboxRunMode,
  setupStatus: SandboxSetupStatusPayload | null,
  activeLock: SandboxRunMode | null,
  setupResolved = true,
): ComposerRunModeResolution {
  if (activeLock) return { mode: activeLock, degraded: null }
  if (preference === 'full') return { mode: 'full', degraded: null }
  if (!setupResolved || setupStatus === null) {
    return { mode: 'safe', degraded: 'setup_unresolved' }
  }
  return { mode: 'safe', degraded: degradationForState(setupStatus.state) }
}

/** 仅要展示模式时的便捷包装（保留旧签名，语义已改为「不静默降级」）。 */
export function effectiveComposerRunMode(
  preference: SandboxRunMode,
  setupStatus: SandboxSetupStatusPayload | null,
  activeLock: SandboxRunMode | null,
  setupResolved = true,
): SandboxRunMode {
  return resolveComposerRunMode(preference, setupStatus, activeLock, setupResolved).mode
}

/**
 * 选择 safe 后的动作：
 * - `persist`：直接落盘（full / 已 ready / 状态未知）；
 * - `setup`：可修复（not_setup / failed 且平台支持）→ 落盘 safe 并打开安装对话框；
 * - `notice`：不可修复（unavailable）或环境不支持 → 落盘 safe 并弹出可操作提示。
 *
 * 任何分支都不再返回 `ignore`：静默吞掉点击正是「点了没反应」的根因。
 */
export type ComposerRunModeSelectionAction = 'persist' | 'setup' | 'notice'

export function composerRunModeSelectionAction(
  mode: SandboxRunMode,
  setupStatus: SandboxSetupStatusPayload | null,
  canSetup: boolean,
  setupResolved = true,
): ComposerRunModeSelectionAction {
  if (mode === 'full') return 'persist'
  if (!setupResolved || setupStatus === null) return 'persist'
  if (setupStatus.state === 'ready') return 'persist'
  // 安装进行中：保留选择并等待轮询结果，不能谎报「未安装/不可用」。
  if (setupStatus.state === 'setting_up') return 'persist'
  if (canSetup && setupStatus.state !== 'unavailable') return 'setup'
  return 'notice'
}

export async function completeComposerSafeSetup(
  ensureSetup: () => Promise<boolean>,
  persistMode: (mode: SandboxRunMode) => Promise<unknown>,
): Promise<boolean> {
  if (!await ensureSetup()) return false
  await persistMode('safe')
  return true
}
