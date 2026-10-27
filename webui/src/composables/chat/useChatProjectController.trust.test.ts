// @vitest-environment happy-dom
import { afterEach, describe, expect, it } from 'vitest'
import {
  isProjectPathTrusted,
  rememberTrustedProjectPath,
} from './useChatProjectController'

const KEY = 'marguerite.chat.trustedProjectPaths'

afterEach(() => {
  localStorage.removeItem(KEY)
})

describe('project trust memory', () => {
  it('treats an unknown path as untrusted', () => {
    expect(isProjectPathTrusted('D:\\work\\alpha')).toBe(false)
  })

  it('remembers a path after a successful open', () => {
    rememberTrustedProjectPath('D:\\work\\alpha')
    expect(isProjectPathTrusted('D:\\work\\alpha')).toBe(true)
    // 尾部分隔符不应产生第二条记录
    expect(isProjectPathTrusted('D:\\work\\alpha\\')).toBe(true)
  })

  it('promotes the most recent path and keeps the list bounded', () => {
    for (let i = 0; i < 60; i += 1) rememberTrustedProjectPath(`D:\\work\\p${i}`)
    const stored = JSON.parse(localStorage.getItem(KEY) || '[]') as string[]
    expect(stored).toHaveLength(50)
    expect(stored[0]).toContain('p59')
    // 最早的条目被 LRU 淘汰
    expect(isProjectPathTrusted('D:\\work\\p0')).toBe(false)
  })

  it('survives malformed storage by falling back to untrusted', () => {
    localStorage.setItem(KEY, '{not json')
    expect(isProjectPathTrusted('D:\\work\\alpha')).toBe(false)
    rememberTrustedProjectPath('D:\\work\\alpha')
    expect(isProjectPathTrusted('D:\\work\\alpha')).toBe(true)
  })
})
