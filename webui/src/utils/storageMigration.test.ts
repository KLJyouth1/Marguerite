// @vitest-environment happy-dom
import { describe, it, expect, beforeEach } from 'vitest'
import { migrateLegacyStorageKeys } from './storageMigration'

describe('migrateLegacyStorageKeys', () => {
  beforeEach(() => {
    localStorage.clear()
    sessionStorage.clear()
  })

  it('将 localStorage 的 Marguerite.* 键迁移到 marguerite.* 并删除旧键', () => {
    localStorage.setItem('Marguerite.wsUrl', 'ws://localhost:18790/ws')
    localStorage.setItem('Marguerite.chat.runMode', 'full')
    migrateLegacyStorageKeys()
    expect(localStorage.getItem('marguerite.wsUrl')).toBe('ws://localhost:18790/ws')
    expect(localStorage.getItem('marguerite.chat.runMode')).toBe('full')
    expect(localStorage.getItem('Marguerite.wsUrl')).toBeNull()
    expect(localStorage.getItem('Marguerite.chat.runMode')).toBeNull()
  })

  it('迁移无点的 Marguerite-theme 主题键', () => {
    localStorage.setItem('Marguerite-theme', 'dark')
    migrateLegacyStorageKeys()
    expect(localStorage.getItem('marguerite-theme')).toBe('dark')
    expect(localStorage.getItem('Marguerite-theme')).toBeNull()
  })

  it('目标键已存在时不覆盖新值', () => {
    localStorage.setItem('Marguerite.wsUrl', 'old')
    localStorage.setItem('marguerite.wsUrl', 'new')
    migrateLegacyStorageKeys()
    expect(localStorage.getItem('marguerite.wsUrl')).toBe('new')
    expect(localStorage.getItem('Marguerite.wsUrl')).toBeNull()
  })

  it('同时迁移 sessionStorage（会话令牌/缓存认证）', () => {
    sessionStorage.setItem('Marguerite.wsToken', 'tok')
    sessionStorage.setItem('Marguerite.cachedAuth', 'auth')
    migrateLegacyStorageKeys()
    expect(sessionStorage.getItem('marguerite.wsToken')).toBe('tok')
    expect(sessionStorage.getItem('marguerite.cachedAuth')).toBe('auth')
    expect(sessionStorage.getItem('Marguerite.wsToken')).toBeNull()
  })

  it('空 store 与已迁移的 store 均为幂等 no-op', () => {
    migrateLegacyStorageKeys()
    localStorage.setItem('marguerite.shortcuts', '{}')
    migrateLegacyStorageKeys()
    expect(localStorage.getItem('marguerite.shortcuts')).toBe('{}')
  })
})
