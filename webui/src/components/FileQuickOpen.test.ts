// @vitest-environment happy-dom
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { createApp, defineComponent, h, nextTick, type App } from 'vue'
import { createI18n } from 'vue-i18n'
import FileQuickOpen from './FileQuickOpen.vue'

const mocks = vi.hoisted(() => ({
  rpcCall: vi.fn(),
  supportsMethod: vi.fn(() => true),
}))

vi.mock('@/stores/rpc', () => ({
  useRpcStore: () => ({ call: mocks.rpcCall, supportsMethod: mocks.supportsMethod }),
}))

const BASE = '/workspace/proj'

function i18n() {
  return createI18n({
    legacy: false,
    locale: 'en',
    messages: {
      en: {
        common: { close: 'Close' },
        quickOpen: {
          title: 'Quick open',
          placeholder: 'Search files by name',
          hint: '↑↓ to move · Enter to open · Esc to close',
          empty: 'No matching files',
          searching: 'Searching…',
          noProject: 'Bind a project to search its files.',
          error: 'File search is unavailable.',
          truncated: 'Showing the first {n} matches.',
          open: 'Open {name}',
        },
      },
    },
  })
}

interface Item { name: string; path: string; relative: string; kind: 'file' }

const ITEMS: Item[] = [
  { name: 'app.ts', path: `${BASE}/src/app.ts`, relative: 'src/app.ts', kind: 'file' },
  { name: 'apply.ts', path: `${BASE}/src/apply.ts`, relative: 'src/apply.ts', kind: 'file' },
  { name: 'app.spec.ts', path: `${BASE}/tests/app.spec.ts`, relative: 'tests/app.spec.ts', kind: 'file' },
]

describe('FileQuickOpen', () => {
  let app: App<Element> | null = null
  const opened = vi.fn()
  const closed = vi.fn()

  beforeEach(() => {
    vi.useFakeTimers()
    mocks.rpcCall.mockReset()
    mocks.supportsMethod.mockReset()
    mocks.supportsMethod.mockReturnValue(true)
    opened.mockReset()
    closed.mockReset()
  })

  afterEach(() => {
    app?.unmount()
    app = null
    document.body.innerHTML = ''
    vi.useRealTimers()
  })

  async function mount(basePath?: string): Promise<HTMLElement> {
    const el = document.createElement('div')
    document.body.appendChild(el)
    app = createApp(defineComponent({
      render: () => h(FileQuickOpen, {
        open: true,
        basePath,
        onOpen: (path: string) => opened(path),
        onClose: () => closed(),
      }),
    }))
    app.use(i18n())
    app.mount(el)
    await nextTick()
    await nextTick()
    return el
  }

  function input(el: HTMLElement): HTMLInputElement {
    return el.querySelector<HTMLInputElement>('.quick-open__input')!
  }

  function type(el: HTMLElement, value: string) {
    const field = input(el)
    field.value = value
    field.dispatchEvent(new Event('input', { bubbles: true }))
  }

  function press(el: HTMLElement, key: string) {
    input(el).dispatchEvent(new KeyboardEvent('keydown', { key, bubbles: true, cancelable: true }))
  }

  async function settle(ms = 200) {
    await vi.advanceTimersByTimeAsync(ms)
    await nextTick()
    await nextTick()
  }

  function statusText(el: HTMLElement): string {
    return el.querySelector('.quick-open__status')?.textContent?.trim() ?? ''
  }

  function names(el: HTMLElement): string[] {
    return Array.from(el.querySelectorAll('.quick-open__item-name')).map(node => node.textContent?.trim() ?? '')
  }

  function activeName(el: HTMLElement): string {
    return el.querySelector('.quick-open__item.is-active .quick-open__item-name')?.textContent?.trim() ?? ''
  }

  it('focuses the input, shows the hint for a blank query, and never searches on open', async () => {
    const el = await mount(BASE)

    expect(document.activeElement).toBe(input(el))
    expect(statusText(el)).toBe('↑↓ to move · Enter to open · Esc to close')
    expect(el.querySelectorAll('.quick-open__item')).toHaveLength(0)
    expect(mocks.rpcCall).not.toHaveBeenCalled()
  })

  it('shows the no-project state and refuses to search without a base path', async () => {
    const el = await mount()

    expect(statusText(el)).toBe('Bind a project to search its files.')
    type(el, 'app')
    await settle()
    expect(mocks.rpcCall).not.toHaveBeenCalled()
    expect(statusText(el)).toBe('Bind a project to search its files.')
  })

  it('debounces then calls sandbox.path.search with basePath, trimmed query and limit', async () => {
    mocks.rpcCall.mockResolvedValue({ items: [], truncated: false })
    const el = await mount(BASE)

    type(el, '  app  ')
    await vi.advanceTimersByTimeAsync(100)
    expect(mocks.rpcCall, 'must not fire before the debounce elapses').not.toHaveBeenCalled()

    await settle(120)
    expect(mocks.rpcCall).toHaveBeenCalledTimes(1)
    expect(mocks.rpcCall).toHaveBeenCalledWith('sandbox.path.search', {
      basePath: BASE,
      query: 'app',
      limit: 20,
    })
  })

  it('renders matches with their relative path and the truncated note', async () => {
    mocks.rpcCall.mockResolvedValue({ items: ITEMS, truncated: true })
    const el = await mount(BASE)

    type(el, 'app')
    await settle()

    expect(names(el)).toEqual(['app.ts', 'apply.ts', 'app.spec.ts'])
    expect(
      Array.from(el.querySelectorAll('.quick-open__item-path')).map(node => node.textContent?.trim()),
    ).toEqual(['src/app.ts', 'src/apply.ts', 'tests/app.spec.ts'])
    expect(el.querySelector('.quick-open__note')?.textContent?.trim())
      .toBe('Showing the first 3 matches.')
    expect(statusText(el)).toBe('')
  })

  it('moves the selection with ArrowDown and opens the active path on Enter', async () => {
    mocks.rpcCall.mockResolvedValue({ items: ITEMS, truncated: false })
    const el = await mount(BASE)

    type(el, 'app')
    await settle()
    expect(activeName(el)).toBe('app.ts')

    press(el, 'ArrowDown')
    await nextTick()
    expect(activeName(el)).toBe('apply.ts')

    press(el, 'Enter')
    expect(opened).toHaveBeenCalledTimes(1)
    expect(opened).toHaveBeenCalledWith(`${BASE}/src/apply.ts`)
  })

  it('wraps selection with ArrowUp and opens a row on click', async () => {
    mocks.rpcCall.mockResolvedValue({ items: ITEMS, truncated: false })
    const el = await mount(BASE)

    type(el, 'app')
    await settle()

    press(el, 'ArrowUp')
    await nextTick()
    expect(activeName(el), 'ArrowUp from the first row wraps to the last').toBe('app.spec.ts')

    el.querySelectorAll<HTMLElement>('.quick-open__item')[0].click()
    expect(opened).toHaveBeenCalledWith(`${BASE}/src/app.ts`)
  })

  it('closes exactly once on Escape', async () => {
    const el = await mount(BASE)

    press(el, 'Escape')
    await nextTick()
    expect(closed).toHaveBeenCalledTimes(1)
  })

  it('shows the error state when the gateway does not support the method', async () => {
    mocks.supportsMethod.mockReturnValue(false)
    const el = await mount(BASE)

    expect(statusText(el)).toBe('File search is unavailable.')
    type(el, 'app')
    await settle()
    expect(mocks.rpcCall).not.toHaveBeenCalled()
    expect(statusText(el)).toBe('File search is unavailable.')
  })

  it('shows the error state when the search call fails', async () => {
    mocks.rpcCall.mockRejectedValue(new Error('gateway down'))
    const el = await mount(BASE)

    type(el, 'app')
    await settle()

    expect(statusText(el)).toBe('File search is unavailable.')
    expect(el.querySelectorAll('.quick-open__item')).toHaveLength(0)
  })

  it('shows the empty state for a query with no matches', async () => {
    mocks.rpcCall.mockResolvedValue({ items: [], truncated: false })
    const el = await mount(BASE)

    type(el, 'zzz')
    await settle()

    expect(statusText(el)).toBe('No matching files')
  })

  it('drops a stale response that resolves after a newer query', async () => {
    let resolveFirst: ((value: unknown) => void) | null = null
    mocks.rpcCall
      .mockImplementationOnce(() => new Promise((resolve) => { resolveFirst = resolve }))
      .mockResolvedValueOnce({ items: [ITEMS[1]], truncated: false })
    const el = await mount(BASE)

    type(el, 'a')
    await vi.advanceTimersByTimeAsync(200)
    type(el, 'app')
    await vi.advanceTimersByTimeAsync(200)
    await nextTick()
    expect(names(el)).toEqual(['apply.ts'])

    // The first (older) request now answers with a different list; it must lose.
    resolveFirst!({ items: [ITEMS[0]], truncated: false })
    await vi.advanceTimersByTimeAsync(0)
    await nextTick()
    expect(names(el)).toEqual(['apply.ts'])
  })
})
