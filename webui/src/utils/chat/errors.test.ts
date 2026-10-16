// @vitest-environment happy-dom
// P0-1c / P0-4 守卫：稳定错误码 → 本地化文案；绝不把上游原始正文当作主文案回显。
import { beforeAll, describe, expect, it } from 'vitest'

import i18n, { loadLocaleMessages } from '@/i18n'
import {
  API_KEY_MISSING,
  AUTH_FAILED,
  NETWORK,
  OTHER,
  PLAN_COMPLETION_BLOCKED,
  PLAN_STEP_ARTIFACT_MISSING,
  PLAN_STEP_FAILED,
  RATE_LIMITED,
  SERVER,
  TIMEOUT,
  isAuthFailureCode,
  localizedChatErrorMessage,
} from '@/utils/chat/errors'

beforeAll(async () => {
  await loadLocaleMessages('en')
  await loadLocaleMessages('zh-Hans')
})

describe('localizedChatErrorMessage', () => {
  it('maps AUTH_FAILED to a localized message and never echoes the upstream body', () => {
    i18n.global.locale.value = 'en'
    const upstream = 'Your api key: ****gEni is invalid'
    const msg = localizedChatErrorMessage(AUTH_FAILED, upstream)
    expect(msg).not.toBe(upstream)
    expect(msg).not.toContain('****gEni')
    expect(msg).toBe(i18n.global.t('chat.errors.authFailed'))
  })

  it('maps the other provider classes to localized copy (not the server fallback)', () => {
    i18n.global.locale.value = 'en'
    for (const code of [RATE_LIMITED, NETWORK, SERVER]) {
      expect(localizedChatErrorMessage(code, 'server fallback')).not.toBe('server fallback')
    }
    expect(localizedChatErrorMessage(API_KEY_MISSING, 'x')).toBe(
      i18n.global.t('chat.errors.apiKeyMissing'),
    )
  })

  it('falls back to server-authored text for unknown codes', () => {
    expect(localizedChatErrorMessage('SOME_UNKNOWN', 'server text')).toBe('server text')
  })

  it('localizes TIMEOUT and OTHER (P2-13), distinct from NETWORK/SERVER', () => {
    i18n.global.locale.value = 'en'
    expect(localizedChatErrorMessage(TIMEOUT, 'raw timeout')).toBe(i18n.global.t('chat.errors.timeout'))
    expect(localizedChatErrorMessage(OTHER, 'raw other')).toBe(i18n.global.t('chat.errors.other'))
    // 不得回落到上游原始正文，也不得与其它码共用同一文案。
    expect(localizedChatErrorMessage(TIMEOUT, 'raw timeout')).not.toBe('raw timeout')
    expect(i18n.global.t('chat.errors.timeout')).not.toBe(i18n.global.t('chat.errors.network'))

    i18n.global.locale.value = 'zh-Hans'
    expect(localizedChatErrorMessage(TIMEOUT, 'x')).toContain('超时')
  })

  it('renders Chinese copy in the zh-Hans locale (no English leak for authFailed)', () => {
    i18n.global.locale.value = 'zh-Hans'
    const msg = localizedChatErrorMessage(AUTH_FAILED, 'fallback')
    expect(msg).toBe(i18n.global.t('chat.errors.authFailed'))
    expect(msg).toContain('API Key')
    expect(msg).not.toContain('rejected')
  })
})

describe('localizedChatErrorMessage — plan failures (R65/B1)', () => {
  it('humanizes PLAN_STEP_FAILED with the structured step label, never the raw internal string', () => {
    i18n.global.locale.value = 'zh-Hans'
    const raw = '计划子步骤 step-2 执行失败：工具执行失败: write_file denied'
    const msg = localizedChatErrorMessage(PLAN_STEP_FAILED, raw, {
      step_id: 'step-2',
      step_title: '生成 HTML 网页',
    })
    // 人话化：不再回显内部术语；插入了用户可读的步骤标题。
    expect(msg).not.toBe(raw)
    expect(msg).not.toContain('计划子步骤')
    expect(msg).not.toContain('write_file denied')
    expect(msg).toContain('生成 HTML 网页')
    expect(msg).toBe(i18n.global.t('chat.errors.planStepFailed', { step: '生成 HTML 网页' }))
  })

  it('falls back to step_id, then to a generic sentence when no step label is present', () => {
    i18n.global.locale.value = 'en'
    expect(localizedChatErrorMessage(PLAN_STEP_FAILED, 'raw', { step_id: 'step-3' })).toBe(
      i18n.global.t('chat.errors.planStepFailed', { step: 'step-3' }),
    )
    expect(localizedChatErrorMessage(PLAN_STEP_FAILED, 'raw')).toBe(
      i18n.global.t('chat.errors.planStepFailedGeneric'),
    )
    expect(localizedChatErrorMessage(PLAN_STEP_FAILED, 'raw', {})).not.toBe('raw')
  })

  it('maps artifact-missing and completion-blocked codes to distinct localized copy', () => {
    i18n.global.locale.value = 'zh-Hans'
    const missing = localizedChatErrorMessage(PLAN_STEP_ARTIFACT_MISSING, 'raw', {
      step_title: '生成演示文稿',
    })
    const blocked = localizedChatErrorMessage(PLAN_COMPLETION_BLOCKED, 'raw')
    expect(missing).toContain('生成演示文稿')
    expect(missing).not.toContain('计划子步骤')
    expect(blocked).not.toBe('raw')
    expect(blocked).not.toBe(missing)
    expect(localizedChatErrorMessage(PLAN_STEP_ARTIFACT_MISSING, 'raw')).toBe(
      i18n.global.t('chat.errors.planStepArtifactMissingGeneric'),
    )
  })
})

describe('isAuthFailureCode', () => {
  it('is true only for AUTH_FAILED', () => {
    expect(isAuthFailureCode(AUTH_FAILED)).toBe(true)
    expect(isAuthFailureCode(API_KEY_MISSING)).toBe(false)
    expect(isAuthFailureCode(undefined)).toBe(false)
  })
})
