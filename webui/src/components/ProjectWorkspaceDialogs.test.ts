// @vitest-environment happy-dom
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { createApp, h, nextTick, ref, type App } from 'vue'
import { createI18n } from 'vue-i18n'
import en from '@/locales/en.json'
import ProjectWorkspaceDialogs from './ProjectWorkspaceDialogs.vue'

const mocks = vi.hoisted(() => ({
  rpc: {
    canChooseProject: true,
    canManageProjectWorkspaces: true,
    call: vi.fn(),
  },
  openWorkspace: vi.fn(),
  renameWorkspace: vi.fn(),
  confirm: vi.fn(),
  pushToast: vi.fn(),
}))

vi.mock('@/stores/rpc', () => ({ useRpcStore: () => mocks.rpc }))
vi.mock('@/composables/useProjectWorkspaces', () => ({
  useProjectWorkspaces: () => ({
    openWorkspace: mocks.openWorkspace,
    renameWorkspace: mocks.renameWorkspace,
  }),
}))
vi.mock('@/composables/useConfirm', () => ({ useConfirm: () => ({ confirm: mocks.confirm }) }))
vi.mock('@/composables/useToasts', () => ({ useToasts: () => ({ pushToast: mocks.pushToast }) }))
vi.mock('@/platform', () => ({ getPlatform: () => ({ files: {} }) }))

const mountedApps: App<Element>[] = []

function pathListResult(path: string, children: string[]) {
  return {
    currentPath: path,
    path,
    parentPath: path === '/' ? null : '/',
    systemPickerAvailable: false,
    entries: children.map(child => ({
      name: child.split('/').pop(),
      path: child,
      kind: 'directory',
      selectable: true,
    })),
  }
}

async function mountDialogs() {
  const host = document.createElement('div')
  document.body.appendChild(host)
  const dialogRef = ref<InstanceType<typeof ProjectWorkspaceDialogs> | null>(null)
  const app = createApp({
    render: () => h(ProjectWorkspaceDialogs, {
      ref: dialogRef,
      sessionKey: 'agent:main:webchat:picker-test',
    }),
  })
  app.use(createI18n({ legacy: false, locale: 'en', messages: { en } }))
  app.mount(host)
  mountedApps.push(app)
  await nextTick()
  return {
    async openCreator() {
      dialogRef.value?.openProjectCreator()
      await nextTick()
    },
  }
}

async function flushPromises() {
  await new Promise(resolve => setTimeout(resolve, 0))
  await nextTick()
}

beforeEach(() => {
  mocks.rpc.call.mockReset()
  mocks.rpc.call.mockImplementation((method: string) => {
    if (method === 'sandbox.path.list') return pathListResult('/repos', ['/repos/fresh'])
    return {}
  })
  mocks.openWorkspace.mockReset()
  mocks.renameWorkspace.mockReset()
  mocks.confirm.mockReset()
  mocks.pushToast.mockReset()
})

afterEach(() => {
  mountedApps.splice(0).forEach(app => app.unmount())
  document.body.innerHTML = ''
})

describe('ProjectWorkspaceDialogs', () => {
  it('opens the create-source picker in the directory browser, not the recent list', async () => {
    const dialogs = await mountDialogs()
    await dialogs.openCreator()

    const trigger = document.querySelector<HTMLButtonElement>('.project-create__source-picker')
    expect(trigger).toBeTruthy()
    trigger!.click()
    await flushPromises()

    const methods = mocks.rpc.call.mock.calls.map(call => call[0])
    expect(methods).not.toContain('workspaces.list')
    expect(methods).toContain('sandbox.path.list')
    expect(document.body.textContent).not.toContain('Recently opened')
    expect(document.body.textContent).toContain('fresh')
  })
})
