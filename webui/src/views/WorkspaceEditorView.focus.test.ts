// @vitest-environment happy-dom
// WorkspaceEditorView 专注模式（D 线推广）：折叠标题块、保留保存/刷新操作钮。
// scoped CSS 在 happy-dom 不生效，样式断言走「磁盘读 CSS 契约」先例（App.sidebar-icon-rail）。
import { createApp, nextTick } from 'vue'
import { describe, expect, it, vi } from 'vitest'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import i18n from '@/i18n'

vi.mock('@/stores/rpc', () => ({
  useRpcStore: () => ({
    call: vi.fn().mockResolvedValue({ entries: [] }),
    on: () => () => {},
    supportsMethod: () => false,
  }),
}))
vi.mock('vue-router', () => ({ useRoute: () => ({ query: {} }) }))

import WorkspaceEditorView from '@/views/WorkspaceEditorView.vue'

describe('WorkspaceEditorView focus mode (D-line rollout)', () => {
  it('toggles the focus class while keeping the action buttons mounted', async () => {
    const host = document.createElement('div')
    document.body.appendChild(host)
    const app = createApp(WorkspaceEditorView)
    app.use(i18n)
    app.mount(host)
    await nextTick()

    const stage = host.querySelector('.editor-stage') as HTMLElement
    expect(stage?.classList.contains('editor-stage--focus')).toBe(false)

    const toggle = host.querySelector<HTMLButtonElement>('[data-testid="editor-focus-toggle"]')!
    expect(toggle).not.toBeNull()
    toggle.click()
    await nextTick()
    expect(stage.classList.contains('editor-stage--focus')).toBe(true)
    expect(toggle.getAttribute('aria-pressed')).toBe('true')

    // 折叠的是标题块，不是操作钮：focus 类落根节点，操作钮原样保留
    expect(host.querySelector('.control-stage__title-block')).not.toBeNull()
    expect(host.querySelector('.control-stage__actions')).not.toBeNull()

    // CSS 契约（磁盘读，同 sidebar-icon-rail 先例）：focus 态隐藏标题块、放大内容面
    const css = readFileSync(resolve(process.cwd(), 'src/views/WorkspaceEditorView.vue'), 'utf8')
    expect(css).toContain('.editor-stage--focus .control-stage__title-block { display: none; }')
    expect(css).toContain('.editor-stage--focus .editor-main { min-height: calc(100vh - 200px); }')

    app.unmount()
    document.body.innerHTML = ''
  })
})
