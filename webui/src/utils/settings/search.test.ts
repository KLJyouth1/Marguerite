import { describe, expect, it } from 'vitest'
import {
  filterSettingsSections,
  matchesSettingsSection,
  normalizeSettingsQuery,
  settingsSectionKeywords,
} from './search'

const sections = [
  { id: 'provider', label: 'Model Service' },
  { id: 'sandbox', label: 'Sandbox' },
  { id: 'pet', label: 'Pet' },
  { id: 'collabSsh', label: 'Collab & SSH' },
]

describe('settings search', () => {
  it('returns every section for an empty query', () => {
    expect(filterSettingsSections(sections, '   ')).toEqual(sections)
  })

  it('matches the localized label case-insensitively', () => {
    expect(filterSettingsSections(sections, 'model').map(s => s.id)).toEqual(['provider'])
    expect(filterSettingsSections(sections, 'SANDBOX').map(s => s.id)).toEqual(['sandbox'])
  })

  it('matches ids and technical keywords that are not in the label', () => {
    expect(filterSettingsSections(sections, 'api key').map(s => s.id)).toEqual(['provider'])
    expect(filterSettingsSections(sections, 'ssh').map(s => s.id)).toEqual(['collabSsh'])
    expect(filterSettingsSections(sections, '桌宠').map(s => s.id)).toEqual(['pet'])
  })

  it('supports multi-character partial matches and returns no false positives', () => {
    expect(filterSettingsSections(sections, 'coll').map(s => s.id)).toEqual(['collabSsh'])
    expect(filterSettingsSections(sections, 'zzz')).toEqual([])
  })

  it('normalizes whitespace and exposes the keyword map safely', () => {
    expect(normalizeSettingsQuery('  Model  ')).toBe('model')
    expect(settingsSectionKeywords('does-not-exist')).toEqual([])
    expect(matchesSettingsSection({ id: 'pet', label: 'Pet' }, '')).toBe(true)
  })
})
