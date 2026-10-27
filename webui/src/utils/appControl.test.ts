// @vitest-environment happy-dom
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import {
  clickTarget,
  fillTarget,
  isBenignNoise,
  listInteractiveElements,
  pressKey,
  readTarget,
  resolveTarget,
  scanRoutes,
  scanState,
  selectorFor,
  waitForTarget,
} from './appControl'

// happy-dom 没有排版引擎，getBoundingClientRect 恒为 0×0；
// 生产判据（不可见则不计入清单）依赖真实布局盒，故测试里给定固定盒。
const RECT = { x: 10, y: 20, width: 120, height: 32, top: 20, right: 130, bottom: 52, left: 10, toJSON: () => ({}) }

beforeEach(() => {
  document.body.innerHTML = ''
  vi.spyOn(Element.prototype, 'getBoundingClientRect').mockReturnValue(RECT as DOMRect)
})

afterEach(() => {
  vi.restoreAllMocks()
  document.body.innerHTML = ''
  scanState.entries = []
  scanState.running = false
})

describe('selectorFor', () => {
  it('prefers data-testid, then id, then a structural path', () => {
    document.body.innerHTML = `
      <div><button data-testid="save">保存</button></div>
      <section><input id="name" /></section>
      <nav><a href="/x">链接</a></nav>
    `
    expect(selectorFor(document.querySelector('[data-testid="save"]')!)).toBe('[data-testid="save"]')
    expect(selectorFor(document.querySelector('#name')!)).toBe('#name')
    expect(selectorFor(document.querySelector('a')!)).toContain('a')
  })
})

describe('listInteractiveElements', () => {
  it('lists visible interactive elements with role, name and box', () => {
    document.body.innerHTML = `
      <button data-testid="a">开始</button>
      <input aria-label="搜索" value="abc" />
      <div style="visibility:hidden"><button>隐藏</button></div>
      <p>静态文本</p>
    `
    const list = listInteractiveElements()
    expect(list.map(el => el.id)).toEqual(['a', '搜索'])
    expect(list[0]).toMatchObject({ role: 'button', name: '开始', disabled: false })
    expect(list[1]).toMatchObject({ role: 'textbox', name: '搜索', value: 'abc' })
    expect(list[0]!.rect.width).toBe(120)
  })
})

describe('resolveTarget', () => {
  it('accepts testid: prefix, css and data-testid shorthand', () => {
    document.body.innerHTML = '<button data-testid="go">Go</button>'
    expect(resolveTarget('testid:go')).not.toBeNull()
    expect(resolveTarget('go')).not.toBeNull()
    expect(resolveTarget('button')).not.toBeNull()
    expect(resolveTarget('missing')).toBeNull()
  })
})

describe('actions', () => {
  it('clicks an element through real mouse events', () => {
    const onClick = vi.fn()
    document.body.innerHTML = '<button data-testid="go">Go</button>'
    document.querySelector('[data-testid="go"]')!.addEventListener('click', onClick)
    expect(clickTarget('testid:go').ok).toBe(true)
    expect(onClick).toHaveBeenCalledTimes(1)
  })

  it('fills an input so the value is readable afterwards', () => {
    document.body.innerHTML = '<input data-testid="q" value="" />'
    const input = document.querySelector('[data-testid="q"]') as HTMLInputElement
    const onInput = vi.fn()
    input.addEventListener('input', onInput)
    const result = fillTarget('testid:q', 'hello')
    expect(result).toMatchObject({ ok: true, value: 'hello' })
    expect(onInput).toHaveBeenCalledTimes(1)
  })

  it('reads text content and reports missing targets honestly', () => {
    document.body.innerHTML = '<p data-testid="msg">就绪</p>'
    expect(readTarget('testid:msg')).toMatchObject({ ok: true, value: '就绪' })
    expect(readTarget('testid:none').ok).toBe(false)
  })

  it('dispatches keys to the focused element', () => {
    document.body.innerHTML = '<input data-testid="q" />'
    const input = document.querySelector('[data-testid="q"]') as HTMLInputElement
    input.focus()
    const onKey = vi.fn()
    input.addEventListener('keydown', onKey)
    expect(pressKey('Enter').ok).toBe(true)
    expect(onKey).toHaveBeenCalledTimes(1)
  })

  it('waits for a late element and times out otherwise', async () => {
    setTimeout(() => {
      document.body.insertAdjacentHTML('beforeend', '<button data-testid="late">late</button>')
    }, 30)
    await expect(waitForTarget('testid:late', 500)).resolves.toMatchObject({ ok: true })
    await expect(waitForTarget('testid:never', 60)).resolves.toMatchObject({ ok: false })
  })
})

describe('scanRoutes', () => {
  it('marks a route with an alert as failed and a clean route as passed', async () => {
    const navigate = vi.fn(async (path: string) => {
      if (path === '/bad') {
        document.body.innerHTML = '<div role="alert">加载失败</div><button>重试</button>'
      } else {
        document.body.innerHTML = '<button data-testid="ok">好</button>'
      }
    })

    const entries = await scanRoutes(['/good', '/bad'], navigate, { settleMs: 5 })

    expect(entries).toHaveLength(2)
    expect(entries[0]).toMatchObject({ path: '/good', ok: true, elements: 1 })
    expect(entries[1]).toMatchObject({ path: '/bad', ok: false })
    expect(entries[1]!.alerts).toEqual(['加载失败'])
    expect(scanState.running).toBe(false)
    expect(scanState.entries).toHaveLength(2)
  })

  it('captures console.error during the scan and restores console afterwards', async () => {
    const original = console.error
    const entries = await scanRoutes(['/x'], () => {
      document.body.innerHTML = '<button>ok</button>'
      console.error('boom')
    }, { settleMs: 5 })

    expect(entries[0]!.errors).toContain('boom')
    expect(console.error).toBe(original)
  })

  it('ignores the benign ResizeObserver loop noise but keeps real errors', async () => {
    // 实测来源：/device-twin 的 3D 舞台触发该提示，曾被误判为路由失败。
    expect(isBenignNoise('ResizeObserver loop completed with undelivered notifications.')).toBe(true)
    expect(isBenignNoise('ResizeObserver loop limit exceeded')).toBe(true)
    expect(isBenignNoise('Cannot read properties of null')).toBe(false)

    const entries = await scanRoutes(['/y'], () => {
      document.body.innerHTML = '<button>ok</button>'
      window.dispatchEvent(new ErrorEvent('error', {
        message: 'ResizeObserver loop completed with undelivered notifications.',
      }))
    }, { settleMs: 5 })

    expect(entries[0]!.ok).toBe(true)
    expect(entries[0]!.errors).toEqual([])
  })
})
