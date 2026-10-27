import { describe, expect, it } from 'vitest'
import { createBrowserWorkbenchItem } from '@/workbench/browserItems'
import { createBrowserWorkbenchDefinition } from './browserWorkbenchProvider'

/**
 * 侧边浏览器面板的契约（2026-10-24 改版：CDP 镜像，不再是原生 WebContents 表面）。
 *
 * 回归锚点：这一族用例原本守护的是「原生表面失败时不要留白」；改版后需要守护的是
 * **面板不再依赖壳的原生表面能力**（壳里根本没实现那组命令），以及 URL 归一与
 * 作用域透传仍然是 fail-closed 的。
 */
describe('browser Workbench provider (CDP mirror)', () => {
  const definition = createBrowserWorkbenchDefinition({ t: key => key })

  it('面板项走 DOM 宿主，不再要求原生表面能力', () => {
    const item = createBrowserWorkbenchItem({ scopeId: 'session-a', url: 'https://example.test/start' })
    expect(item).toBeTruthy()
    expect(item!.hostKind).toBe('dom')
    expect(item!.kind).toBe('browser')
  })

  it('拒绝非 http(s) 与带控制字符的地址（fail-closed）', () => {
    expect(createBrowserWorkbenchItem({ scopeId: 's', url: 'javascript:alert(1)' })).toBeNull()
    expect(createBrowserWorkbenchItem({ scopeId: 's', url: 'file:///C:/Windows/win.ini' })).toBeNull()
    expect(createBrowserWorkbenchItem({ scopeId: 's', url: 'https://a.test/\u0000' })).toBeNull()
    expect(createBrowserWorkbenchItem({ scopeId: '  ', url: 'https://a.test' })).toBeNull()
  })

  it('只接 browser 项且必须有可用 URL', () => {
    const item = createBrowserWorkbenchItem({ scopeId: 'session-a', url: 'https://example.test/x' })!
    expect(definition.supports?.(item)).toBe(true)
    expect(definition.supports?.({ ...item, kind: 'file' })).toBe(false)
    expect(definition.supports?.({ ...item, payload: { initialUrl: '', scopeId: 'session-a' } })).toBe(false)
  })

  it('把会话作用域与初始 URL 透传给面板，并只在激活时让面板刷新', () => {
    const item = createBrowserWorkbenchItem({ scopeId: 'session-a', url: 'https://example.test/x' })!
    expect(definition.getProps?.(item, {
      active: true,
      hostAvailable: true,
      nativeSurface: false,
      runtimeState: {},
    })).toEqual({
      scopeId: 'session-a',
      initialUrl: 'https://example.test/x',
      active: true,
    })
    expect(definition.getProps?.(item, {
      active: false,
      hostAvailable: true,
      nativeSurface: false,
      runtimeState: {},
    })).toMatchObject({ active: false })
  })

  it('标题回退到主机名（没有页标题时不能显示一整条 URL）', () => {
    const item = createBrowserWorkbenchItem({ scopeId: 'session-a', url: 'https://example.test/a/b?c=d' })!
    const header = definition.getHeader!(item, {
      active: true,
      hostAvailable: true,
      nativeSurface: false,
      runtimeState: {},
    })
    expect(header.title).toBe('example.test')
    expect(header.subtitle).toBe('https://example.test/a/b?c=d')
    expect(header.icon).toBe('languages')
  })
})
