// @vitest-environment happy-dom
// 右侧栏：① 文件树必须以"当前工作区"为根；② 任务列要能展开看到入参/出参。
//
// 这两条都是真机缺陷的回归守卫：
//   · 树根此前只发 `{ path: '' }`，后端回落到用户主目录（AppData/Cookies/Desktop…）；
//   · 任务列此前只有一行标题，命令与输出、文件内容、思考正文都看不到。
import { createApp, nextTick, type App } from 'vue'
import { createPinia, setActivePinia } from 'pinia'
import { afterEach, describe, expect, it, vi } from 'vitest'

import i18n from '@/i18n'
import RightSidebar, { type RsSubagent, type RsTask } from './RightSidebar.vue'
import { useRpcStore } from '@/stores/rpc'
import { sideChatStorageKey } from '@/composables/chat/useSideChatSession'

const apps: App<Element>[] = []

afterEach(() => {
  apps.splice(0).forEach(instance => instance.unmount())
  document.body.innerHTML = ''
  localStorage.removeItem('marguerite-right-sidebar-tabs')
  localStorage.removeItem(sideChatStorageKey('agent:main:webchat:main'))
  vi.unstubAllGlobals()
  vi.restoreAllMocks()
})

/** 让 computed / watcher / promise 链跑完（happy-dom 没有真实的宏任务队列）。 */
const flush = () => new Promise<void>(resolve => setTimeout(resolve, 0))

interface MountOpts {
  tasks?: RsTask[]
  subagents?: RsSubagent[]
  focusSubagentKey?: string
  focusTab?: 'tasks' | 'subagents' | 'artifacts' | 'changes' | 'tree' | 'sideChat'
  focusSeq?: number
  keepStoredTabs?: boolean
  rootPath?: string
  sessionKey?: string
  streaming?: boolean
  /** sandbox.path.list 返回的条目 */
  entries?: Array<{ name: string; path: string; kind: 'directory' | 'file'; selectable?: boolean; hidden?: boolean }>
  /** workspace.status 返回的默认根（仅在未传 rootPath 时用到） */
  defaultRoot?: string
}

function mount(opts: MountOpts = {}) {
  i18n.global.locale.value = 'en'
  if (!opts.keepStoredTabs) localStorage.removeItem('marguerite-right-sidebar-tabs')
  const el = document.createElement('div')
  document.body.appendChild(el)

  const pinia = createPinia()
  setActivePinia(pinia)

  const entries = opts.entries ?? [
    { name: 'src', path: '/proj/src', kind: 'directory' as const },
    { name: 'README.md', path: '/proj/README.md', kind: 'file' as const },
  ]
  const call = vi.fn(async (method: string, _params?: Record<string, unknown>) => {
    if (method === 'workspace.status') return { root: opts.defaultRoot ?? '/data/workspace' }
    if (method === 'sandbox.path.list') return { entries }
    return {}
  })
  // 先拿到 store 并替换 RPC，再把组件挂上去：组件 setup 里拿到的是同一个实例。
  const rpc = useRpcStore()
  ;(rpc as unknown as { call: typeof call }).call = call
  ;(rpc as unknown as { supportsMethod: (m: string) => boolean }).supportsMethod = () => true
  // 侧聊天标签会在挂载时订阅事件并拉一次审批快照；两者都必须被隔离。
  ;(rpc as unknown as {
    on: (event: string, handler: (payload: unknown, meta?: unknown) => void) => () => void
  }).on = () => () => {}
  vi.stubGlobal('fetch', vi.fn(async (_input: unknown, init?: RequestInit) => ({
    ok: true,
    status: 200,
    json: async () => (init?.method === 'POST'
      ? { resolved: true, approved: true, resolution: 'approved' }
      : { pending: [] }),
  })))

  const instance = createApp(RightSidebar, {
    open: true,
    tasks: opts.tasks ?? [],
    subagents: opts.subagents ?? [],
    focusSubagentKey: opts.focusSubagentKey,
    focusSeq: opts.focusSeq ?? (opts.focusSubagentKey ? 1 : 0),
    focusTab: opts.focusTab,
    artifacts: [],
    changes: [],
    rootPath: opts.rootPath,
    sessionKey: opts.sessionKey,
    streaming: opts.streaming ?? false,
  })
  apps.push(instance)
  instance.use(pinia)
  instance.use(i18n)
  instance.mount(el)

  return { el, call }
}

async function openTreeTab(el: HTMLElement) {
  const tabs = Array.from(el.querySelectorAll<HTMLButtonElement>('.rs-tab'))
  const tree = tabs.find(t => t.textContent?.trim().endsWith('Tree'))
  expect(tree).toBeTruthy()
  tree!.click()
  await nextTick()
  await flush()
  await nextTick()
}

describe('RightSidebar · 工作区树', () => {
  it('uses the bound project path as the explorer root', async () => {
    const { el, call } = mount({ rootPath: 'D:/work/proj' })
    await openTreeTab(el)

    // 新实现：项目树直接用绝对根作为列目录的 path（不再依赖 basePath+空 path）
    const list = call.mock.calls.find(c => c[0] === 'sandbox.path.list')
    expect(list?.[1]).toEqual({ path: 'D:/work/proj' })
    expect(el.querySelectorAll('.fx__name')).toHaveLength(2)
    // 根目录显示工作区名，而不是空占位
    expect(el.querySelector('.fx__root')?.textContent?.trim()).toBe('proj')
  })

  it('falls back to workspace.status root when no project is bound', async () => {
    const { el, call } = mount({ defaultRoot: 'C:/Users/me/AppData/Local/Marguerite' })
    await openTreeTab(el)

    // 先问解析器，再拿它的 root 作为树根
    expect(call.mock.calls.some(c => c[0] === 'workspace.status')).toBe(true)
    const list = call.mock.calls.find(c => c[0] === 'sandbox.path.list')
    expect(list?.[1]).toEqual({ path: 'C:/Users/me/AppData/Local/Marguerite' })
  })
})

describe('RightSidebar · 面板宽度', () => {
  const panelWidth = (el: HTMLElement): number =>
    Number.parseFloat(el.querySelector<HTMLElement>('.rs-panel')!.style.getPropertyValue('--rs-panel-w'))

  it('会话列与面板之间有可访问的垂直分割线', async () => {
    const { el } = mount()
    const sep = el.querySelector<HTMLElement>('.rs-panel [role="separator"]')
    expect(sep, '分割线必须挂在面板上（贴会话列一侧）').toBeTruthy()
    expect(sep!.getAttribute('aria-orientation')).toBe('vertical')
    expect(sep!.getAttribute('aria-label')).toBeTruthy()
    expect(sep!.getAttribute('tabindex')).toBe('0')
    expect(sep!.getAttribute('aria-valuenow')).toBeTruthy()
    expect(sep!.getAttribute('aria-valuemin')).toBe('280')
  })

  it('宽度受控：面板宽度来自 usePanelResize（内联变量），且落在 280px..55vw', async () => {
    const { el } = mount()
    const width = panelWidth(el)
    expect(Number.isFinite(width)).toBe(true)
    expect(width).toBeGreaterThanOrEqual(280)
    expect(width).toBeLessThanOrEqual(Math.round(window.innerWidth * 0.55))
    const sep = el.querySelector('[role="separator"]')!
    expect(Number(sep.getAttribute('aria-valuenow'))).toBe(width)
  })

  it('拖拽分割线改宽，松手写入 localStorage', async () => {
    const { el } = mount()
    const before = panelWidth(el)
    const sep = el.querySelector<HTMLElement>('[role="separator"]')!

    sep.dispatchEvent(new MouseEvent('pointerdown', { bubbles: true, cancelable: true, clientX: 900, button: 0 }))
    window.dispatchEvent(new MouseEvent('pointermove', { clientX: 860 }))
    await nextTick()
    const after = panelWidth(el)
    expect(after).toBeGreaterThan(before)
    expect(el.querySelector('.rs-panel')!.classList.contains('is-resizing')).toBe(true)

    window.dispatchEvent(new MouseEvent('pointerup', { clientX: 860 }))
    await nextTick()
    expect(el.querySelector('.rs-panel')!.classList.contains('is-resizing')).toBe(false)
    expect(localStorage.getItem('marguerite.layout.rightPanelWidth')).toBe(String(after))
  })

  it('键盘 → 让面板变窄 24px 并落盘', async () => {
    const { el } = mount()
    const before = panelWidth(el)
    const sep = el.querySelector<HTMLElement>('[role="separator"]')!
    sep.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowRight', bubbles: true, cancelable: true }))
    await nextTick()
    const after = panelWidth(el)
    expect(after).toBe(Math.max(280, before - 24))
    expect(localStorage.getItem('marguerite.layout.rightPanelWidth')).toBe(String(after))
  })
})

describe('RightSidebar · 运行流', () => {
  const tasks: RsTask[] = [
    {
      id: 'thinking:live',
      kind: 'thinking',
      title: '',
      status: 'running',
      output: '先看看 package.json',
    },
    {
      id: 'tool:t1',
      kind: 'tool',
      title: 'run_command',
      status: 'error',
      tools: ['run_command'],
      input: 'npm test',
      output: '2 failing',
    },
    {
      id: 'summary:1',
      kind: 'summary',
      title: '',
      status: 'done',
      output: '已修复 2 个用例',
    },
  ]

  it('localizes thinking/summary titles and shows kind-specific icons', async () => {
    const { el } = mount({ tasks, streaming: true })
    const titles = Array.from(el.querySelectorAll('.rs-run-row__text')).map(n => n.textContent?.trim())
    expect(titles).toEqual(['Thinking', 'run_command', 'Summary'])
    // 运行态在头部有实时指示
    expect(el.querySelector('.rs-run-live.is-on')).toBeTruthy()
  })

  it('auto-expands running/error rows and renders input/output bodies', async () => {
    const { el } = mount({ tasks })
    const rows = Array.from(el.querySelectorAll('.rs-run-row'))
    expect(rows).toHaveLength(3)
    // 思考（running）与命令（error）默认展开；总结（done）默认收起
    expect(rows.map(r => r.getAttribute('aria-expanded'))).toEqual(['true', 'true', 'false'])

    const bodies = Array.from(el.querySelectorAll('.rs-run-body'))
    expect(bodies).toHaveLength(2)
    expect(bodies[0].textContent).toContain('先看看 package.json')
    expect(bodies[1].textContent).toContain('npm test')
    expect(bodies[1].textContent).toContain('2 failing')
  })

  it('toggles a finished row open and closed without losing its content', async () => {
    const { el } = mount({ tasks })
    const summaryRow = Array.from(el.querySelectorAll<HTMLButtonElement>('.rs-run-row'))[2]
    summaryRow.click()
    await nextTick()
    expect(summaryRow.getAttribute('aria-expanded')).toBe('true')
    expect(el.querySelectorAll('.rs-run-body')).toHaveLength(3)

    summaryRow.click()
    await nextTick()
    expect(summaryRow.getAttribute('aria-expanded')).toBe('false')
    expect(el.querySelectorAll('.rs-run-body')).toHaveLength(2)
  })

  it('keeps a collapsed running row collapsed after the user closes it', async () => {
    const { el } = mount({ tasks })
    const thinkingRow = Array.from(el.querySelectorAll<HTMLButtonElement>('.rs-run-row'))[0]
    expect(thinkingRow.getAttribute('aria-expanded')).toBe('true')
    thinkingRow.click()
    await nextTick()
    expect(thinkingRow.getAttribute('aria-expanded')).toBe('false')
  })

  it('does not make rows without payload expandable', async () => {
    const { el } = mount({
      tasks: [{ kind: 'step', title: '读取文件', status: 'done' }],
    })
    const row = el.querySelector<HTMLButtonElement>('.rs-run-row')!
    expect(row.disabled).toBe(true)
    expect(row.getAttribute('aria-expanded')).toBeNull()
    expect(el.querySelector('.rs-run-body')).toBeNull()
  })
})

describe('RightSidebar · 子智能体轴', () => {
  const subagents: RsSubagent[] = [
    { key: 'agent:main:subagent:aaaa1111', tone: 'ok', status: 'completed', summary: 'finished the parser', ts: '2026-10-26T10:00:00Z' },
    { key: 'agent:main:subagent:bbbb2222', tone: 'run', status: 'running', summary: '', ts: '2026-10-26T10:05:00Z' },
  ]

  it('lists subagent runs and keeps the running count in the header', async () => {
    const { el } = mount({ subagents })
    const tab = Array.from(el.querySelectorAll<HTMLButtonElement>('.rs-tab'))
      .find(t => t.textContent?.trim().startsWith('Subagents'))
    expect(tab).toBeTruthy()
    expect(tab!.textContent).toContain('2')
    tab!.click()
    await nextTick()

    expect(el.querySelectorAll('.rs-subagent')).toHaveLength(2)
    expect(el.querySelector('.rs-subagent__summary')?.textContent).toBe('finished the parser')
    expect(el.querySelector('.rs-run-live')?.textContent).toContain('1 running')
  })

  it('opens the axis filtered to the clicked child session and can clear it', async () => {
    const { el } = mount({ subagents, focusSubagentKey: 'agent:main:subagent:aaaa1111' })
    await nextTick()

    expect(el.querySelectorAll('.rs-subagent')).toHaveLength(1)
    expect(el.querySelector('.rs-subagent__key')?.textContent).toContain('aaaa1111')
    const chip = el.querySelector<HTMLButtonElement>('.rs-subagent-filter')
    expect(chip?.textContent).toContain('aaaa1111')

    chip!.click()
    await nextTick()
    expect(el.querySelectorAll('.rs-subagent')).toHaveLength(2)
    expect(el.querySelector('.rs-subagent-filter')).toBeNull()
  })

  it('explains an empty axis without pretending there is data', async () => {
    const { el } = mount({ subagents: [] })
    const tab = Array.from(el.querySelectorAll<HTMLButtonElement>('.rs-tab'))
      .find(t => t.textContent?.trim().startsWith('Subagents'))
    tab!.click()
    await nextTick()
    expect(el.querySelector('.rs-empty')?.textContent).toContain('Subagent runs will appear here')
  })
})

describe('RightSidebar · 多标签容器', () => {
  it('closes the active axis and reopens it from the add menu', async () => {
    const { el } = mount()
    const active = el.querySelector<HTMLElement>('.rs-tab--active')!
    expect(active.textContent).toContain('Tasks')

    active.querySelector<HTMLButtonElement>('.rs-tab__close')!.click()
    await nextTick()
    expect(Array.from(el.querySelectorAll('.rs-tab')).map(tab => tab.textContent?.trim()))
      .not.toContain('Tasks')
    expect(el.querySelector('.rs-tab--active')?.textContent).toContain('Subagents')

    el.querySelector<HTMLButtonElement>('.rs-tabs__add .rs-icon-btn')!.click()
    await nextTick()
    const menuItems = Array.from(el.querySelectorAll<HTMLElement>('.rs-tabs__add-menu [role="menuitem"]'))
    expect(menuItems.map(item => item.textContent?.trim())).toContain('Tasks')
    menuItems.find(item => item.textContent?.includes('Tasks'))!.click()
    await nextTick()

    expect(el.querySelectorAll('.rs-tab')).toHaveLength(5)
    expect(el.querySelector('.rs-tab--active')?.textContent).toContain('Tasks')
  })

  it('never closes the last remaining tab', async () => {
    const { el } = mount()
    for (let i = 0; i < 4; i += 1) {
      el.querySelector<HTMLElement>('.rs-tab--active .rs-tab__close')!.click()
      await nextTick()
    }
    expect(el.querySelectorAll('.rs-tab')).toHaveLength(1)
    expect(el.querySelector('.rs-tab__close')).toBeNull()

    el.querySelector<HTMLButtonElement>('.rs-tabs__add .rs-icon-btn')!.click()
    await nextTick()
    // 5 = 关掉的四个轴 + 默认关闭的「Side chat」
    expect(el.querySelectorAll('.rs-tabs__add-menu [role="menuitem"]')).toHaveLength(5)
  })

  it('persists the open set and active tab across remounts', async () => {
    const first = mount()
    first.el.querySelector<HTMLElement>('.rs-tab--active .rs-tab__close')!.click()
    await nextTick()

    const second = mount({ keepStoredTabs: true })
    expect(second.el.querySelectorAll('.rs-tab')).toHaveLength(4)
    expect(second.el.querySelector('.rs-tab--active')?.textContent).toContain('Subagents')
    expect(Array.from(second.el.querySelectorAll('.rs-tab')).map(tab => tab.textContent?.trim()))
      .not.toContain('Tasks')
  })
})

describe('RightSidebar · 侧聊天轴', () => {
  const tabLabels = (el: HTMLElement) =>
    Array.from(el.querySelectorAll('.rs-tab')).map(tab => tab.textContent?.trim())

  it('keeps the side chat closed by default but exposes an opener', async () => {
    const { el } = mount({ sessionKey: 'agent:main:webchat:main' })

    expect(tabLabels(el)).not.toContain('Side chat')
    expect(el.querySelector('[data-testid="side-chat"]')).toBeNull()
    const toggle = el.querySelector<HTMLButtonElement>('[data-testid="rs-sidechat-toggle"]')
    expect(toggle).toBeTruthy()
    expect(toggle!.getAttribute('aria-pressed')).toBe('false')
    expect(toggle!.getAttribute('aria-label')).toBe('Side chat')
  })

  it('opens and focuses the side chat from the header tool button', async () => {
    const { el } = mount({ sessionKey: 'agent:main:webchat:main' })

    el.querySelector<HTMLButtonElement>('[data-testid="rs-sidechat-toggle"]')!.click()
    await nextTick()

    expect(tabLabels(el)).toContain('Side chat')
    expect(el.querySelector('.rs-tab--active')?.textContent).toContain('Side chat')
    expect(el.querySelector<HTMLButtonElement>('[data-testid="rs-sidechat-toggle"]')!
      .getAttribute('aria-pressed')).toBe('true')
    expect(el.querySelector('[data-testid="side-chat"]')).toBeTruthy()
    expect(el.querySelector('[data-testid="side-chat-empty"]')).toBeTruthy()
  })

  it('unmounts the side chat when its tab is closed', async () => {
    const { el } = mount({ sessionKey: 'agent:main:webchat:main' })
    el.querySelector<HTMLButtonElement>('[data-testid="rs-sidechat-toggle"]')!.click()
    await nextTick()
    expect(el.querySelector('[data-testid="side-chat"]')).toBeTruthy()

    el.querySelector<HTMLElement>('.rs-tab--active .rs-tab__close')!.click()
    await nextTick()

    expect(el.querySelector('[data-testid="side-chat"]')).toBeNull()
    expect(tabLabels(el)).not.toContain('Side chat')
  })

  it('restores a persisted side chat tab across remounts', async () => {
    localStorage.setItem('marguerite-right-sidebar-tabs', JSON.stringify({
      open: ['tasks', 'sideChat'],
      active: 'sideChat',
    }))
    const { el } = mount({ keepStoredTabs: true, sessionKey: 'agent:main:webchat:main' })
    await nextTick()

    expect(el.querySelector('.rs-tab--active')?.textContent).toContain('Side chat')
    expect(el.querySelector('[data-testid="side-chat"]')).toBeTruthy()
  })
})
