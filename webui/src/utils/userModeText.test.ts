// @vitest-environment happy-dom
// Guards for the 3x3 user-mode copy helpers.
//
// The interesting invariant is the fallback rule: a missing i18n message must
// never reach the UI as the raw key (`mode.userMode.minimal_ask`), and a locale
// chunk that failed to load must still render the canonical label from
// USER_MODE_META.
import { describe, expect, it, beforeEach } from 'vitest'
import i18n, { loadLocaleMessages } from '@/i18n'
import { USER_MODE_META } from '@/stores/activeMode'
import { userModeHint, userModeIntensityLabel, userModeLabel, userModeWorkingLabel } from '@/utils/userModeText'

beforeEach(() => {
  i18n.global.locale.value = 'en'
})

describe('userModeLabel', () => {
  it('localizes every canonical slot in en and zh-Hans', async () => {
    for (const key of Object.keys(USER_MODE_META)) {
      expect(userModeLabel(key)).toBeTruthy()
      expect(userModeLabel(key)).not.toContain('mode.userMode.')
    }
    expect(userModeLabel('standard_doit')).toBe('Standard · Act')

    await loadLocaleMessages('zh-Hans')
    i18n.global.locale.value = 'zh-Hans'
    expect(userModeLabel('standard_doit')).toBe('标准·执行')
    expect(userModeLabel('minimal_ask')).toBe('极简·咨询')
  })

  it('falls back to the canonical metadata label for an unknown slot', () => {
    // A gateway/browser sending a key we do not ship must not leak the i18n key.
    expect(userModeLabel('ultra_doit')).toBe('ultra_doit')
    expect(USER_MODE_META.ultra_doit).toBeUndefined()
  })
})

describe('axis labels', () => {
  it('localizes the intensity and working axes', () => {
    expect(userModeIntensityLabel('extreme')).toBe('Extreme')
    expect(userModeIntensityLabel('standard')).toBe('Standard')
    expect(userModeWorkingLabel('doit')).toBe('Act')
    expect(userModeWorkingLabel('ask')).toBe('Ask')
  })

  it('never leaks an i18n key for an unknown axis value', () => {
    expect(userModeIntensityLabel('ultra')).toBe('ultra')
    expect(userModeWorkingLabel('sleep')).toBe('sleep')
  })
})

describe('userModeHint', () => {
  it('returns the localized description, empty for an unknown slot', () => {
    expect(userModeHint('extreme_doit')).toBe('Swarm collaboration, deep push')
    expect(userModeHint('nope')).toBe('')
  })
})
