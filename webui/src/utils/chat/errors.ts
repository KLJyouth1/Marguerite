import i18n from '@/i18n'

export const ENSEMBLE_MULTIMODAL_UNSUPPORTED = 'ensemble_multimodal_unsupported'
export const API_KEY_MISSING = 'API_KEY_MISSING'
/** 上游 provider 归类错误码（core `model::error_taxonomy` 定义，P0-1/P0-4）。 */
export const AUTH_FAILED = 'AUTH_FAILED'
export const RATE_LIMITED = 'RATE_LIMITED'
export const NETWORK = 'NETWORK'
export const SERVER = 'SERVER'
/** 请求超时（core `ProviderErrorClass::Timeout`，P2-13）。 */
export const TIMEOUT = 'TIMEOUT'
/** 其它未归类 provider 失败（core `ProviderErrorClass::Other`）。 */
export const OTHER = 'OTHER'

/** 计划子步骤执行失败（core `error_taxonomy::PLAN_STEP_FAILED`，R65/B1）。 */
export const PLAN_STEP_FAILED = 'PLAN_STEP_FAILED'
/** 计划子步骤未产出标题声明的交付物（core `PLAN_STEP_ARTIFACT_MISSING`）。 */
export const PLAN_STEP_ARTIFACT_MISSING = 'PLAN_STEP_ARTIFACT_MISSING'
/** 计划完成门禁未放行（core `PLAN_COMPLETION_BLOCKED`）。 */
export const PLAN_COMPLETION_BLOCKED = 'PLAN_COMPLETION_BLOCKED'
/**
 * 宿主（Python 侧车）进程退出（core `pyengine::sidecar::HostExitEvidence`，S5）。
 *
 * 与上面的 provider 类错误码不同，它描述的是**宿主进程本身没了**，因此携带的
 * 上下文字段是 `{ exit_code, uptime_ms }`——分别来自 `ExitStatus::code()` 与
 * spawn→退出的时长。`stderr_tail` 刻意**不在**上下文里：它可能含路径 / URL，
 * 只允许进折叠详情（见 `error_taxonomy` 的既有纪律）。
 */
export const HOST_EXITED = 'HOST_EXITED'

/**
 * 结构化错误上下文：后端错误帧（`session.event.error` payload / `chat.send`
 * 终态响应）携带的、用于本地化插值的字段，例如计划步骤的 `step_id` / `step_title`。
 *
 * 类型故意宽松（`unknown`）：不同来源对象（`SessionEventPayload` / `ChatSendResponse`）
 * 结构不同，读取时统一走 [`contextField`] 安全取值，避免给每个来源加索引签名。
 */
export type ChatErrorContext = unknown

/** 安全读取结构化上下文中的字符串字段（非对象 / 非字符串一律回退）。 */
function contextField(context: ChatErrorContext, key: string): string {
  if (!context || typeof context !== 'object') return ''
  const value = (context as Record<string, unknown>)[key]
  return typeof value === 'string' && value.trim() ? value.trim() : ''
}

/** 计划步骤的用户可读标签：优先标题，其次步骤 id。 */
function stepLabel(context: ChatErrorContext): string {
  return contextField(context, 'step_title') || contextField(context, 'step_id')
}

/** Preserve server-authored text for unknown failures, but give stable
 * error codes an actionable message in the active UI locale. */
export function localizedChatErrorMessage(
  code: unknown,
  fallback: string,
  context: ChatErrorContext = null,
): string {
  if (code === ENSEMBLE_MULTIMODAL_UNSUPPORTED)
    return i18n.global.t('chat.composer.ensembleImageUnsupported')
  if (code === API_KEY_MISSING)
    return i18n.global.t('chat.errors.apiKeyMissing')
  // P0-4：上游错误已归类为稳定错误码——用户看到本地化文案，而非 provider 原始正文。
  if (code === AUTH_FAILED)
    return i18n.global.t('chat.errors.authFailed')
  if (code === RATE_LIMITED)
    return i18n.global.t('chat.errors.rateLimited')
  if (code === NETWORK)
    return i18n.global.t('chat.errors.network')
  if (code === SERVER)
    return i18n.global.t('chat.errors.server')
  // P2-13：超时归 TIMEOUT，其它兜底归 OTHER（core 已有这两个稳定码，前端此前未接）。
  if (code === TIMEOUT)
    return i18n.global.t('chat.errors.timeout')
  if (code === OTHER)
    return i18n.global.t('chat.errors.other')
  // R65/B1：计划执行失败的稳定码——按码本地化，用结构化上下文插值步骤信息，
  // 不再回显后端内部术语（`计划子步骤 step-2 执行失败：…`）。
  if (code === PLAN_STEP_FAILED) {
    const step = stepLabel(context)
    return step
      ? i18n.global.t('chat.errors.planStepFailed', { step })
      : i18n.global.t('chat.errors.planStepFailedGeneric')
  }
  if (code === PLAN_STEP_ARTIFACT_MISSING) {
    const step = stepLabel(context)
    return step
      ? i18n.global.t('chat.errors.planStepArtifactMissing', { step })
      : i18n.global.t('chat.errors.planStepArtifactMissingGeneric')
  }
  if (code === PLAN_COMPLETION_BLOCKED)
    return i18n.global.t('chat.errors.planCompletionBlocked')
  // S5：宿主进程退出。退出码缺失表示被信号杀死（Unix），走另一条文案——
  // 打印 "code null" 对用户毫无意义，而"被信号终止"才是真正的诊断信息。
  // 三条分支对应三种上下文完备度，与 planStepFailed{,,Generic} 的既有模式同构：
  // 有码有时长 → 全量；无码有时长 → 信号文案；全无 → 泛化文案（绝不渲染裸占位符）。
  if (code === HOST_EXITED) {
    const exitCode = contextField(context, 'exit_code')
    const uptime = contextField(context, 'uptime_ms')
    if (exitCode && uptime) {
      return i18n.global.t('chat.errors.hostExited', { exit_code: exitCode, uptime_ms: uptime })
    }
    if (uptime) return i18n.global.t('chat.errors.hostExitedNoCode', { uptime_ms: uptime })
    return i18n.global.t('chat.errors.hostExitedGeneric')
  }
  return fallback
}

/** AUTH_FAILED 是否应展示「前往设置」+「重试」操作（P0-1c）。 */
export function isAuthFailureCode(code: unknown): boolean {
  return code === AUTH_FAILED
}
