// @vitest-environment happy-dom
// 项目树 ↔ 编辑器联动守卫：
//   · 双击文件必须发出「在工作台打开」事件（带上绝对路径与会话作用域）；
//   · 右键必须出现真实操作菜单（打开 / 重命名 / 删除 / 复制路径 …）。
import { createApp, nextTick, type App } from 'vue'
import { createPinia, setActivePinia } from 'pinia'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import i18n from '@/i18n'
import FileExplorerPanel from './FileExplorerPanel.vue'
import { useRpcStore } from '@/stores/rpc'
import { EDITOR_WORKBENCH_OPEN_EVENT, type EditorWorkbenchOpenEventDetail } from '@/workbench/editorItems'

let app: App<Element> | null = null
const flush = () => new Promise<void>(resolve => setTimeout(resolve, 0))

function mount(rootPath = 'D:/work/proj', scopeId = 'sess-1') {
  i18n.global.locale.value = 'en'
  const el = document.createElement('div')
  document.body.appendChild(el)

  const pinia = createPinia()
  setActivePinia(pinia)

  const entries = [
    { name: 'src', path: 'D:/work/proj/src', kind: 'directory' as const },
    { name: 'main.ts', path: 'D:/work/proj/src/main.ts', kind: 'file' as const },
    { name: 'README.md', path: 'D:/work/proj/README.md', kind: 'file' as const },
  ]

  const call = vi.fn(async (method: string) => {
    if (method === 'sandbox.path.list') return { entries }
    if (method === 'workspace.status') return { root: 'D:/work/proj' }
    return { ok: true }
  })
  const rpc = useRpcStore()
  ;(rpc as unknown as { call: typeof call }).call = call
  ;(rpc as unknown as { supportsMethod: (m: string) => boolean }).supportsMethod = () => true

  app = createApp(FileExplorerPanel, { rootPath, scopeId })
  app.use(pinia)
  app.use(i18n)
  app.mount(el)
  return { el, call }
}

beforeEach(() => {
  document.body.innerHTML = ''
})

afterEach(() => {
  app?.unmount()
  app = null
  document.body.innerHTML = ''
  vi.restoreAllMocks()
})

describe('FileExplorerPanel', () => {
  it('lists the workspace root and opens a file in the editor on double-click', async () => {
    const { el } = mount()
    await flush()
    await nextTick()

    const rows = Array.from(el.querySelectorAll<HTMLElement>('.fx__row'))
    expect(rows).toHaveLength(3)

    const events: EditorWorkbenchOpenEventDetail[] = []
    const listener = (event: Event) => {
      events.push((event as CustomEvent<EditorWorkbenchOpenEventDetail>).detail)
    }
    window.addEventListener(EDITOR_WORKBENCH_OPEN_EVENT, listener)

    const file = rows.find(row => row.textContent?.includes('README.md'))
    expect(file).toBeTruthy()
    file!.dispatchEvent(new MouseEvent('dblclick', { bubbles: true }))
    await nextTick()

    expect(events).toHaveLength(1)
    expect(events[0].path).toBe('D:/work/proj/README.md')
    expect(events[0].scopeId).toBe('sess-1')
    window.removeEventListener(EDITOR_WORKBENCH_OPEN_EVENT, listener)
  })

  it('shows a real context menu on right-click', async () => {
    const { el } = mount()
    await flush()
    await nextTick()

    const file = Array.from(el.querySelectorAll<HTMLElement>('.fx__row'))
      .find(row => row.textContent?.includes('main.ts'))!
    file.dispatchEvent(new MouseEvent('contextmenu', { bubbles: true, clientX: 40, clientY: 60 }))
    await nextTick()

    const menu = document.body.querySelector('.fx-menu')
    expect(menu).toBeTruthy()
    const labels = Array.from(menu!.querySelectorAll('.fx-menu__item')).map(item => item.textContent?.trim())
    expect(labels).toContain('Open in code editor')
    expect(labels).toContain('Rename')
    expect(labels).toContain('Delete')
    expect(labels).toContain('Copy path')
  })
})
