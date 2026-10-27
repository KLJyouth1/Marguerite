// @vitest-environment happy-dom
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { createApp, defineComponent, h, nextTick, ref, type App } from 'vue'
import { createI18n } from 'vue-i18n'
import type { SandboxPathListResponse } from '@/types/rpc'
import ProjectWorkspacePickerDialog from './ProjectWorkspacePickerDialog.vue'

const PICKER_KEY = 'agent:main:webchat:picker'

const mocks = vi.hoisted(() => ({
  platform: {
    files: {} as {
      chooseProjectDirectory?: (request?: { initialPath?: string }) => Promise<{ path: string } | null>
    },
  },
  rpcCall: vi.fn(),
}))

vi.mock('@/platform', () => ({
  getPlatform: () => mocks.platform,
}))

vi.mock('@/stores/rpc', () => ({
  useRpcStore: () => ({ call: mocks.rpcCall }),
}))

const mountedApps: App<Element>[] = []

/**
 * Per-method RPC responders. The picker now calls `workspaces.list` before the
 * directory browser, so tests must answer by method instead of relying on call
 * order (`mockResolvedValueOnce` would hand a path listing to the workspace list).
 */
const rpcResponders = new Map<string, () => unknown>()

function respond(method: string, ...responses: unknown[]) {
  const queue = [...responses]
  rpcResponders.set(method, () => (queue.length > 1 ? queue.shift() : queue[0]))
}

function callsFor(method: string) {
  return mocks.rpcCall.mock.calls.filter(call => call[0] === method)
}

function i18n() {
  return createI18n({
    legacy: false,
    locale: 'en',
    messages: {
      en: {
        common: { close: 'Close', cancel: 'Cancel' },
        chat: {
          composer: {
            projectRoots: {
              add: 'Add project',
              primary: 'Primary project',
              dialogTitle: 'Additional projects',
              hint: 'Additional projects only add context.',
              remove: 'Remove additional project',
              extraCount: '{count} additional project | {count} additional projects',
              empty: 'No other projects yet.',
              apply: 'Apply',
            },
          },
        },
        workspaces: {
          chooseProject: 'Choose project',
          webPickerScope: 'Paths are on the gateway host.',
          recentTitle: 'Recently opened',
          recentLoading: 'Loading recent projects…',
          unavailableRecentSelection: '“{name}” is unavailable. Restore the folder or choose another project.',
          retryRecentList: 'Retry recent list',
          browseFolders: 'Browse folders…',
          pathPlaceholder: 'Project path',
          projectPath: 'Project path',
          browse: 'Browse',
          choose: 'Choose',
          goToPath: 'Go to path',
          parentDirectory: 'Parent directory',
          retryDirectoryPicker: 'Retry',
          directoryPickerFailed: 'Directory picker failed: {error}',
          chooseSelectedDirectory: 'Choose selected directory',
          newDirectory: 'New folder',
          newDirectoryName: 'Folder name',
          createDirectory: 'Create',
          createDirectoryFailed: 'Could not create the folder: {error}',
          pin: 'Pin project',
          unavailable: 'Directory unavailable',
        },
      },
    },
  })
}

function pathResult(
  currentPath: string,
  children: Array<string | {
    path: string
    kind?: 'directory' | 'file'
    selectable?: boolean
  }>,
  systemPickerAvailable = true,
): SandboxPathListResponse {
  const parent = currentPath === '/'
    ? null
    : currentPath.replace(/[\\/][^\\/]+$/, '') || '/'
  return {
    currentPath,
    path: currentPath,
    parentPath: parent,
    systemPickerAvailable,
    entries: children.map(child => {
      const item = typeof child === 'string' ? { path: child } : child
      const segments = item.path.split(/[\\/]/)
      return {
        name: segments[segments.length - 1] || item.path,
        path: item.path,
        kind: item.kind ?? 'directory',
        selectable: item.selectable ?? true,
      }
    }),
  }
}

function deferred<T>() {
  let resolve!: (value: T) => void
  let reject!: (reason?: unknown) => void
  const promise = new Promise<T>((ok, fail) => {
    resolve = ok
    reject = fail
  })
  return { promise, resolve, reject }
}

async function flushPromises() {
  await new Promise(resolve => setTimeout(resolve, 0))
  await nextTick()
}

async function mountPicker(options: {
  initialPath?: string
  enabled?: boolean
  startInBrowser?: boolean
} = {}) {
  const events = { close: vi.fn(), choose: vi.fn() }
  const open = ref(true)
  const enabled = ref(options.enabled ?? true)
  const host = document.createElement('div')
  document.body.appendChild(host)
  const Root = defineComponent(() => () => h(ProjectWorkspacePickerDialog, {
    open: open.value,
    sessionKey: PICKER_KEY,
    initialPath: options.initialPath,
    enabled: enabled.value,
    startInBrowser: options.startInBrowser === true,
    onClose: events.close,
    onChoose: events.choose,
  }))
  const app = createApp(Root)
  app.use(i18n())
  app.mount(host)
  mountedApps.push(app)
  await nextTick()
  return {
    events,
    async setOpen(value: boolean) {
      open.value = value
      await nextTick()
    },
    async setEnabled(value: boolean) {
      enabled.value = value
      await nextTick()
    },
    async unmount() {
      const mounted = mountedApps.pop()
      mounted?.unmount()
      await nextTick()
    },
  }
}

async function mountMultiPicker(options: {
  primaryId: string
  selectedIds: string[]
  workspaces: Array<{ id: string; name: string; path: string; available?: boolean }>
}) {
  const events = { close: vi.fn(), choose: vi.fn(), apply: vi.fn() }
  const host = document.createElement('div')
  document.body.appendChild(host)
  const Root = defineComponent(() => () => h(ProjectWorkspacePickerDialog, {
    open: true,
    multi: true,
    primaryId: options.primaryId,
    selectedIds: options.selectedIds,
    workspaces: options.workspaces.map(item => ({
      id: item.id,
      name: item.name,
      path: item.path,
      taskCount: 0,
      pinned: false,
      available: item.available !== false,
    })),
    onClose: events.close,
    onChoose: events.choose,
    onApply: events.apply,
  }))
  const app = createApp(Root)
  app.use(i18n())
  app.mount(host)
  mountedApps.push(app)
  await nextTick()
  return { events }
}

function rootRow(name: string): HTMLButtonElement {
  const match = [...document.querySelectorAll<HTMLButtonElement>('.project-picker__root')]
    .find(candidate => candidate.textContent?.includes(name))
  if (!match) throw new Error(`Missing project row: ${name}`)
  return match
}

function recentRows(): HTMLButtonElement[] {
  return [...document.querySelectorAll<HTMLButtonElement>('.project-picker__entries .project-picker__root')]
}

function recentList(): HTMLElement {
  const list = document.querySelector<HTMLElement>('.project-picker__entries')
  if (!list) throw new Error('Missing recent workspace list')
  return list
}

function locationInput(): HTMLInputElement {
  const input = document.querySelector<HTMLInputElement>('[aria-label="Project path"]')
  if (!input) throw new Error('Missing project path input')
  return input
}

function setLocation(value: string) {
  const input = locationInput()
  input.value = value
  input.dispatchEvent(new Event('input', { bubbles: true }))
}

function submitLocation() {
  locationInput().dispatchEvent(new KeyboardEvent('keydown', {
    key: 'Enter',
    bubbles: true,
  }))
}

function button(label: string): HTMLButtonElement {
  const match = [...document.querySelectorAll<HTMLButtonElement>('button')]
    .find(candidate => candidate.textContent?.trim() === label)
  if (!match) throw new Error(`Missing button: ${label}`)
  return match
}

function option(label: string): HTMLButtonElement {
  const match = [...document.querySelectorAll<HTMLButtonElement>('[role="option"]')]
    .find(candidate => candidate.textContent?.trim() === label)
  if (!match) throw new Error(`Missing option: ${label}`)
  return match
}

beforeEach(() => {
  mocks.platform.files = {}
  rpcResponders.clear()
  mocks.rpcCall.mockReset()
  mocks.rpcCall.mockImplementation((method: string) => {
    const responder = rpcResponders.get(method)
    if (responder) return responder()
    // Default: no registered workspaces → the picker falls back to the browser.
    if (method === 'workspaces.list') return { workspaces: [] }
    if (method === 'sandbox.path.list') return pathResult('/repos', [])
    return {}
  })
})

afterEach(() => {
  mountedApps.splice(0).forEach(app => app.unmount())
  document.body.innerHTML = ''
})

describe('ProjectWorkspacePickerDialog', () => {
  it('does not invoke native or RPC pickers while project selection is disabled', async () => {
    const nativePicker = vi.fn()
    mocks.platform.files.chooseProjectDirectory = nativePicker

    await mountPicker({ enabled: false })
    await flushPromises()

    expect(nativePicker).not.toHaveBeenCalled()
    expect(mocks.rpcCall).not.toHaveBeenCalled()
    expect(document.querySelector('.project-picker')).toBeNull()
  })

  it('lists recent workspaces, preselects the most recent and never opens the native picker', async () => {
    const nativePicker = vi.fn()
    mocks.platform.files.chooseProjectDirectory = nativePicker
    respond('workspaces.list', {
      workspaces: [
        {
          id: 'old',
          name: 'Older project',
          path: '/repos/older',
          taskCount: 1,
          pinned: false,
          available: true,
          lastOpenedAt: '2026-09-01T10:00:00+00:00',
        },
        {
          id: 'new',
          name: 'Newest project',
          path: '/repos/newest',
          taskCount: 2,
          pinned: false,
          available: true,
          // 兼容旧网关的 snake_case 别名。
          last_opened_at: '2026-10-26T09:00:00+00:00',
        },
        {
          id: 'legacy',
          name: 'Never opened',
          path: '/repos/legacy',
          taskCount: 0,
          pinned: false,
          available: true,
        },
      ],
    })

    const { events } = await mountPicker()
    await flushPromises()

    expect(nativePicker).not.toHaveBeenCalled()
    expect(callsFor('sandbox.path.list')).toHaveLength(0)
    expect(callsFor('workspaces.list')).toHaveLength(1)

    const rows = recentRows()
    expect(rows).toHaveLength(3)
    expect(rows[0].textContent).toContain('Newest project')
    expect(rows[1].textContent).toContain('Older project')
    // 空 lastOpenedAt（旧数据）排最后。
    expect(rows[2].textContent).toContain('Never opened')
    expect(rows[0].getAttribute('aria-selected')).toBe('true')
    expect(rows[1].getAttribute('aria-selected')).toBe('false')

    // 回车即确认默认选中的最近工作区。
    recentList().dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true }))
    expect(events.choose).toHaveBeenCalledWith('/repos/newest')
  })

  it('selects another recent workspace and confirms it from the primary button', async () => {
    mocks.platform.files.chooseProjectDirectory = vi.fn()
    respond('workspaces.list', {
      workspaces: [
        {
          id: 'a',
          name: 'Alpha',
          path: '/repos/alpha',
          taskCount: 0,
          pinned: false,
          available: true,
          lastOpenedAt: '2026-10-26T09:00:00+00:00',
        },
        {
          id: 'b',
          name: 'Beta',
          path: '/repos/beta',
          taskCount: 0,
          pinned: false,
          available: true,
          lastOpenedAt: '2026-10-25T09:00:00+00:00',
        },
      ],
    })
    const { events } = await mountPicker()
    await flushPromises()

    recentRows()[1].click()
    await nextTick()
    expect(recentRows()[1].getAttribute('aria-selected')).toBe('true')
    button('Choose selected directory').click()

    expect(events.choose).toHaveBeenCalledWith('/repos/beta')
    expect(mocks.platform.files.chooseProjectDirectory).not.toHaveBeenCalled()
  })

  it('preselects the first available recent workspace and refuses to confirm an unavailable one', async () => {
    mocks.platform.files.chooseProjectDirectory = vi.fn()
    respond('workspaces.list', {
      workspaces: [
        {
          id: 'gone',
          name: 'Deleted project',
          path: '/repos/gone',
          taskCount: 1,
          pinned: false,
          available: false,
          lastOpenedAt: '2026-10-26T09:00:00+00:00',
        },
        {
          id: 'ok',
          name: 'Available project',
          path: '/repos/ok',
          taskCount: 0,
          pinned: false,
          available: true,
          lastOpenedAt: '2026-10-25T09:00:00+00:00',
        },
      ],
    })
    const { events } = await mountPicker()
    await flushPromises()

    // 排序仍是「最近优先」，但默认选中跳过不可用目录（它是排序首条）。
    expect(recentRows()[0].textContent).toContain('Deleted project')
    expect(recentRows()[0].getAttribute('aria-selected')).toBe('false')
    expect(recentRows()[1].getAttribute('aria-selected')).toBe('true')

    recentRows()[0].click()
    await nextTick()
    recentList().dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true }))
    await nextTick()

    // 回车确认不可用目录：不关窗、不发 choose，就地给出原因。
    expect(events.choose).not.toHaveBeenCalled()
    expect(events.close).not.toHaveBeenCalled()
    expect(document.body.textContent).toContain('“Deleted project” is unavailable')

    recentRows()[1].click()
    await nextTick()
    recentList().dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true }))
    await nextTick()
    expect(events.choose).toHaveBeenCalledWith('/repos/ok')
  })

  it('starts directly in the directory browser when the create flow asks for it', async () => {
    mocks.platform.files.chooseProjectDirectory = vi.fn()
    respond('workspaces.list', {
      workspaces: [{
        id: 'old',
        name: 'Old project',
        path: '/repos/old',
        taskCount: 1,
        pinned: false,
        available: true,
        lastOpenedAt: '2026-10-26T09:00:00+00:00',
      }],
    })
    respond('sandbox.path.list', pathResult('/repos', ['/repos/fresh']))

    await mountPicker({ startInBrowser: true })
    await flushPromises()

    // 创建项目不展示「最近打开」，旧目录不能当成新项目源。
    expect(callsFor('workspaces.list')).toHaveLength(0)
    expect(callsFor('sandbox.path.list')).toHaveLength(1)
    expect(document.body.textContent).not.toContain('Recently opened')
    expect(document.body.textContent).not.toContain('Old project')
    expect(option('fresh')).toBeTruthy()
  })

  it('offers a directory-browser exit while the recent list is still loading', async () => {
    const pending = deferred<unknown>()
    respond('workspaces.list', pending.promise)
    const nativePicker = vi.fn().mockResolvedValue(null)
    mocks.platform.files.chooseProjectDirectory = nativePicker
    const { events } = await mountPicker()
    await flushPromises()

    expect(document.body.textContent).toContain('Loading recent projects…')
    button('Browse folders…').click()
    await flushPromises()

    expect(nativePicker).toHaveBeenCalledOnce()
    // 挂起时取消原生选择器：直接关窗，而不是回落到一个空列表。
    expect(events.close).toHaveBeenCalledOnce()
    expect(events.choose).not.toHaveBeenCalled()
  })

  it('retries the recent list after falling back to the browser', async () => {
    const listFailure = deferred<unknown>()
    respond('workspaces.list', listFailure.promise, {
      workspaces: [{
        id: 'a',
        name: 'Alpha',
        path: '/repos/alpha',
        taskCount: 0,
        pinned: false,
        available: true,
        lastOpenedAt: '2026-10-26T09:00:00+00:00',
      }],
    })
    respond('sandbox.path.list', pathResult('/repos', ['/repos/other']))

    await mountPicker()
    listFailure.reject(new Error('workspaces unavailable'))
    await flushPromises()

    expect(document.body.textContent).toContain('workspaces unavailable')
    button('Retry recent list').click()
    await flushPromises()

    expect(callsFor('workspaces.list')).toHaveLength(2)
    expect(document.body.textContent).toContain('Recently opened')
    expect(recentRows()[0].textContent).toContain('Alpha')
  })

  it('ignores a workspaces.list response that lands after unmount', async () => {
    const pending = deferred<{ workspaces: unknown[] }>()
    respond('workspaces.list', pending.promise)
    const nativePicker = vi.fn()
    mocks.platform.files.chooseProjectDirectory = nativePicker

    const picker = await mountPicker()
    await flushPromises()
    await picker.unmount()

    pending.resolve({ workspaces: [] })
    await flushPromises()

    // 卸载后迟到的空列表不得触发「首次使用」的原生选择器。
    expect(nativePicker).not.toHaveBeenCalled()
    expect(callsFor('sandbox.path.list')).toHaveLength(0)
  })

  it('opens the native picker from the recent list and stays open on cancel', async () => {
    const nativePicker = vi.fn()
      .mockResolvedValueOnce(null)
      .mockResolvedValueOnce({ path: '/repos/picked' })
    mocks.platform.files.chooseProjectDirectory = nativePicker
    respond('workspaces.list', {
      workspaces: [{
        id: 'a',
        name: 'Alpha',
        path: '/repos/alpha',
        taskCount: 0,
        pinned: false,
        available: true,
        lastOpenedAt: '2026-10-26T09:00:00+00:00',
      }],
    })
    const { events } = await mountPicker()
    await flushPromises()

    button('Browse folders…').click()
    await flushPromises()

    expect(nativePicker).toHaveBeenCalledOnce()
    expect(nativePicker).toHaveBeenCalledWith({ initialPath: '/repos/alpha' })
    // 取消原生选择器：回到最近列表，而不是关闭整个对话框。
    expect(events.close).not.toHaveBeenCalled()
    expect(events.choose).not.toHaveBeenCalled()
    expect(document.querySelector('.project-picker')).toBeTruthy()

    button('Browse folders…').click()
    await flushPromises()
    expect(events.choose).toHaveBeenCalledWith('/repos/picked')
  })

  it('falls back to the directory browser when the recent list cannot be read', async () => {
    const nativePicker = vi.fn()
    mocks.platform.files.chooseProjectDirectory = nativePicker
    const listFailure = deferred<unknown>()
    respond('workspaces.list', listFailure.promise)
    respond('sandbox.path.list', pathResult('/repos', ['/repos/a']))

    const { events } = await mountPicker()
    listFailure.reject(new Error('workspaces unavailable'))
    await flushPromises()

    // 读取失败不是「首次使用」：不得自动弹原生选择器。
    expect(nativePicker).not.toHaveBeenCalled()
    expect(document.body.textContent).toContain('workspaces unavailable')
    expect(option('a')).toBeTruthy()
    button('Choose selected directory').click()
    expect(events.choose).toHaveBeenCalledWith('/repos')
  })

  it('omits the initial web path and exposes only selectable directories', async () => {
    respond('sandbox.path.list', pathResult('/repos', [
      '/repos/project-a',
      { path: '/repos/not-selectable', selectable: false },
      { path: '/repos/readme.txt', kind: 'file' },
    ]))

    await mountPicker()
    await flushPromises()

    expect(callsFor('workspaces.list')).toHaveLength(1)
    expect(mocks.rpcCall).toHaveBeenCalledWith('sandbox.path.list', {
      sessionKey: PICKER_KEY,
      kind: 'workspace',
    })
    expect(document.body.textContent).toContain('project-a')
    expect(document.body.textContent).not.toContain('not-selectable')
    expect(document.body.textContent).not.toContain('readme.txt')
    expect(locationInput().value).toBe('/repos')
    expect(button('Choose selected directory').disabled).toBe(false)
  })

  it('renders directory actions as recognizable icon buttons', async () => {
    respond('sandbox.path.list', pathResult('/repos', []))

    await mountPicker()
    await flushPromises()

    const parentAction = button('Parent directory')
    const browseAction = button('Browse')
    const createAction = button('New folder')
    const chooseAction = button('Choose selected directory')

    expect(parentAction.classList.contains('project-picker__action')).toBe(true)
    expect(browseAction.classList.contains('project-picker__action')).toBe(true)
    expect(createAction.classList.contains('project-picker__action')).toBe(true)
    expect(chooseAction.classList.contains('project-picker__choose')).toBe(true)
    expect(parentAction.querySelector('svg')).toBeTruthy()
    expect(browseAction.querySelector('svg')).toBeTruthy()
    expect(createAction.querySelector('svg')).toBeTruthy()
    expect(chooseAction.querySelector('svg')).toBeTruthy()
    expect(createAction.closest('.project-picker__browser-toolbar')).toBeTruthy()
  })

  it('selects on click and browses only on double click', async () => {
    respond(
      'sandbox.path.list',
      pathResult('/repos', ['/repos/a']),
      pathResult('/repos/a', []),
    )
    const { events } = await mountPicker()
    await flushPromises()

    const entry = option('a')
    entry.click()
    await nextTick()
    expect(callsFor('sandbox.path.list')).toHaveLength(1)
    expect(entry.getAttribute('aria-selected')).toBe('true')

    entry.dispatchEvent(new MouseEvent('dblclick', { bubbles: true }))
    await flushPromises()
    expect(mocks.rpcCall).toHaveBeenLastCalledWith('sandbox.path.list', {
      sessionKey: PICKER_KEY,
      path: '/repos/a',
      kind: 'workspace',
    })
    expect(locationInput().value).toBe('/repos/a')
    expect(button('Choose selected directory').disabled).toBe(false)
    button('Choose selected directory').click()
    expect(events.choose).toHaveBeenCalledWith('/repos/a')
  })

  it('ignores an older browse response that resolves last', async () => {
    const first = deferred<SandboxPathListResponse>()
    const second = deferred<SandboxPathListResponse>()
    respond(
      'sandbox.path.list',
      pathResult('/repos', ['/repos/a', '/repos/b']),
      first.promise,
      second.promise,
    )
    await mountPicker()
    await flushPromises()

    setLocation('/repos/a')
    submitLocation()
    await nextTick()
    setLocation('/repos/b')
    submitLocation()
    second.resolve(pathResult('/repos/b', []))
    first.resolve(pathResult('/repos/a', []))
    await flushPromises()

    expect(locationInput().value).toBe('/repos/b')
  })

  it('uses the current directory as the base for relative locations', async () => {
    respond(
      'sandbox.path.list',
      pathResult('/repos', []),
      pathResult('/repos/child', []),
    )
    await mountPicker()
    await flushPromises()

    setLocation('child')
    submitLocation()
    await flushPromises()

    expect(mocks.rpcCall).toHaveBeenLastCalledWith('sandbox.path.list', {
      sessionKey: PICKER_KEY,
      path: 'child',
      basePath: '/repos',
      kind: 'workspace',
    })
  })

  it('creates a folder in the current directory and opens it', async () => {
    respond(
      'sandbox.path.list',
      pathResult('/repos', []),
      pathResult('/repos/new-project', []),
    )
    respond('sandbox.path.create-directory', { path: '/repos/new-project' })
    await mountPicker()
    await flushPromises()

    button('New folder').click()
    await nextTick()
    const nameInput = document.querySelector<HTMLInputElement>('[aria-label="Folder name"]')
    expect(nameInput).toBeTruthy()
    nameInput!.value = 'new-project'
    nameInput!.dispatchEvent(new Event('input', { bubbles: true }))
    await nextTick()
    button('Create').click()
    await flushPromises()

    expect(mocks.rpcCall).toHaveBeenCalledWith('sandbox.path.create-directory', {
      sessionKey: PICKER_KEY,
      parentPath: '/repos',
      name: 'new-project',
      kind: 'workspace',
    })
    expect(mocks.rpcCall).toHaveBeenLastCalledWith('sandbox.path.list', {
      sessionKey: PICKER_KEY,
      path: '/repos/new-project',
      kind: 'workspace',
    })
    expect(locationInput().value).toBe('/repos/new-project')
  })

  it('ignores a create-folder response after close and reopen', async () => {
    const staleCreate = deferred<{ path: string }>()
    respond(
      'sandbox.path.list',
      pathResult('/repos', []),
      pathResult('/fresh', []),
    )
    respond('sandbox.path.create-directory', staleCreate.promise)
    const picker = await mountPicker()
    await flushPromises()

    button('New folder').click()
    await nextTick()
    const nameInput = document.querySelector<HTMLInputElement>('[aria-label="Folder name"]')!
    nameInput.value = 'stale'
    nameInput.dispatchEvent(new Event('input', { bubbles: true }))
    await nextTick()
    button('Create').click()
    await nextTick()

    await picker.setOpen(false)
    await picker.setOpen(true)
    staleCreate.resolve({ path: '/repos/stale' })
    await flushPromises()

    expect(locationInput().value).toBe('/fresh')
    expect(callsFor('sandbox.path.list')).toHaveLength(2)
  })

  it('browses the real parent returned by the gateway', async () => {
    respond(
      'sandbox.path.list',
      pathResult('/repos/a', []),
      pathResult('/repos', ['/repos/a']),
    )
    await mountPicker({ initialPath: '/repos/a' })
    await flushPromises()

    button('Parent directory').click()
    await flushPromises()

    expect(mocks.rpcCall).toHaveBeenLastCalledWith('sandbox.path.list', {
      sessionKey: PICKER_KEY,
      path: '/repos',
      kind: 'workspace',
    })
  })

  it('keeps the prior entries and selection when the latest browse fails', async () => {
    const failingBrowse = deferred<SandboxPathListResponse>()
    respond(
      'sandbox.path.list',
      pathResult('/repos', ['/repos/a']),
      failingBrowse.promise,
    )
    const { events } = await mountPicker()
    await flushPromises()
    option('a').click()
    await nextTick()

    setLocation('/missing')
    submitLocation()
    failingBrowse.reject(new Error('not readable'))
    await flushPromises()

    expect(document.body.textContent).toContain('not readable')
    expect(option('a').getAttribute('aria-selected')).toBe('true')
    expect(button('Choose selected directory').disabled).toBe(false)
    button('Choose selected directory').click()
    expect(events.choose).toHaveBeenCalledWith('/repos/a')
  })

  it('invalidates a pending response across close and reopen', async () => {
    const stale = deferred<SandboxPathListResponse>()
    const fresh = deferred<SandboxPathListResponse>()
    respond('sandbox.path.list', stale.promise, fresh.promise)
    const picker = await mountPicker()
    await flushPromises()

    await picker.setOpen(false)
    await picker.setOpen(true)
    fresh.resolve(pathResult('/fresh', []))
    stale.resolve(pathResult('/stale', []))
    await flushPromises()

    expect(locationInput().value).toBe('/fresh')
  })

  it('selects additional projects in multi mode without browsing or touching the primary', async () => {
    const { events } = await mountMultiPicker({
      primaryId: 'primary',
      selectedIds: ['extra-a'],
      workspaces: [
        { id: 'primary', name: 'Primary', path: '/repos/primary' },
        { id: 'extra-a', name: 'Extra A', path: '/repos/extra-a' },
        { id: 'extra-b', name: 'Extra B', path: '/repos/extra-b' },
      ],
    })
    await flushPromises()

    expect(mocks.rpcCall).not.toHaveBeenCalled()
    expect(rootRow('Primary').disabled).toBe(true)
    expect(rootRow('Primary').getAttribute('aria-pressed')).toBe('true')
    expect(document.body.textContent).toContain('Primary project')

    rootRow('Primary').click()
    await nextTick()

    rootRow('Extra A').click()
    rootRow('Extra B').click()
    await nextTick()
    button('Apply').click()

    expect(events.apply).toHaveBeenCalledWith(['extra-b'])
  })

  it('browses a selected directory from the keyboard', async () => {
    respond(
      'sandbox.path.list',
      pathResult('/repos', ['/repos/a']),
      pathResult('/repos/a', []),
    )
    await mountPicker()
    await flushPromises()

    const entry = option('a')
    entry.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true }))
    await flushPromises()

    expect(callsFor('sandbox.path.list')).toHaveLength(2)
    expect(locationInput().value).toBe('/repos/a')
  })

  it('disables choosing while a browse request is pending', async () => {
    const pending = deferred<SandboxPathListResponse>()
    respond('sandbox.path.list', pending.promise)
    await mountPicker()
    await flushPromises()

    expect(button('Choose selected directory').disabled).toBe(true)

    pending.resolve(pathResult('/repos', []))
    await flushPromises()
    expect(button('Choose selected directory').disabled).toBe(false)
  })

  it('hides the gateway system picker when the host reports it unavailable', async () => {
    respond('sandbox.path.list', pathResult('/repos', [], false))

    await mountPicker()
    await flushPromises()

    const browseAction = [...document.querySelectorAll<HTMLButtonElement>('button')]
      .find(candidate => candidate.textContent?.trim() === 'Browse')
    expect(browseAction).toBeUndefined()
  })

  it('opens the gateway system picker and immediately chooses its directory', async () => {
    respond('sandbox.path.list', pathResult('/repos', []))
    respond('sandbox.path.pick', { path: '/Volumes/workspace/project' })
    const { events } = await mountPicker()
    await flushPromises()

    button('Browse').click()
    await flushPromises()

    expect(mocks.rpcCall).toHaveBeenCalledWith('sandbox.path.pick', {
      sessionKey: PICKER_KEY,
      kind: 'workspace',
      initialPath: '/repos',
    })
    expect(events.choose).toHaveBeenCalledOnce()
    expect(events.choose).toHaveBeenCalledWith('/Volumes/workspace/project')
  })

  it('keeps the web directory browser open when the system picker is cancelled', async () => {
    respond('sandbox.path.list', pathResult('/repos', ['/repos/a']))
    respond('sandbox.path.pick', { path: null })
    const { events } = await mountPicker()
    await flushPromises()

    button('Browse').click()
    await flushPromises()

    expect(events.choose).not.toHaveBeenCalled()
    expect(events.close).not.toHaveBeenCalled()
    expect(button('Browse')).toBeTruthy()
    expect(document.body.textContent).toContain('a')
  })

  it('auto-opens the native picker exactly once when no workspaces are registered', async () => {
    mocks.platform.files.chooseProjectDirectory = vi.fn(async () => null)
    const { events } = await mountPicker({ initialPath: '/repos/current' })
    await flushPromises()

    // 首次使用：列表为空 → 自动弹一次原生选择器。
    expect(callsFor('workspaces.list')).toHaveLength(1)
    expect(mocks.platform.files.chooseProjectDirectory).toHaveBeenCalledOnce()
    expect(mocks.platform.files.chooseProjectDirectory).toHaveBeenCalledWith({
      initialPath: '/repos/current',
    })
    expect(events.choose).not.toHaveBeenCalled()
    expect(events.close).toHaveBeenCalledOnce()
    expect(callsFor('sandbox.path.list')).toHaveLength(0)
  })

  it('ignores a stale native result after close and reopen', async () => {
    const stale = deferred<{ path: string } | null>()
    const fresh = deferred<{ path: string } | null>()
    mocks.platform.files.chooseProjectDirectory = vi.fn()
      .mockReturnValueOnce(stale.promise)
      .mockReturnValueOnce(fresh.promise)
    const picker = await mountPicker()
    await flushPromises()

    await picker.setOpen(false)
    await picker.setOpen(true)
    fresh.resolve({ path: '/repos/fresh' })
    await flushPromises()
    stale.resolve({ path: '/repos/stale' })
    await flushPromises()

    expect(mocks.platform.files.chooseProjectDirectory).toHaveBeenCalledTimes(2)
    expect(picker.events.choose).toHaveBeenCalledOnce()
    expect(picker.events.choose).toHaveBeenCalledWith('/repos/fresh')
  })

  it('shows native rejection and retries natively without web fallback', async () => {
    mocks.platform.files.chooseProjectDirectory = vi.fn()
      .mockRejectedValueOnce(new Error('native unavailable'))
      .mockResolvedValueOnce({ path: '/repos/native' })
    const { events } = await mountPicker()
    await flushPromises()

    expect(document.body.textContent).toContain('native unavailable')
    expect(callsFor('sandbox.path.list')).toHaveLength(0)
    button('Retry').click()
    await flushPromises()

    expect(mocks.platform.files.chooseProjectDirectory).toHaveBeenCalledTimes(2)
    expect(events.choose).toHaveBeenCalledWith('/repos/native')
    expect(callsFor('sandbox.path.list')).toHaveLength(0)
  })

  it('allows cancelling after a native picker rejection', async () => {
    mocks.platform.files.chooseProjectDirectory = vi.fn()
      .mockRejectedValue(new Error('native unavailable'))
    const { events } = await mountPicker()
    await flushPromises()

    button('Cancel').click()

    expect(events.close).toHaveBeenCalledOnce()
    expect(callsFor('sandbox.path.list')).toHaveLength(0)
  })
})
