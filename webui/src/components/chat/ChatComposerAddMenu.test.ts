// @vitest-environment happy-dom
import { createApp, h, nextTick } from 'vue'
import { createI18n } from 'vue-i18n'
import { afterEach, describe, expect, it, vi } from 'vitest'

import ChatComposerAddMenu from './ChatComposerAddMenu.vue'

const mountedApps: ReturnType<typeof createApp>[] = []
const i18n = createI18n({
  legacy: false,
  locale: 'en',
  messages: {
    en: {
      chat: {
        add: 'Add',
        attachFiles: 'Attach files',
        addMenu: {
          attachHint: 'Images and text are read directly',
          commands: 'Commands',
          commandsHint: 'Open the slash command list',
          artifacts: 'My artifacts',
          artifactsHint: 'View session artifacts',
          fileTree: 'Workspace files',
          fileTreeHint: 'Browse the workspace',
          skills: 'Skills',
          skillsHint: 'View and toggle skills',
          plugins: 'Plugins',
          pluginsHint: 'Manage plugins',
        },
        planMode: {
          label: 'Plan mode',
          readOnly: 'Read-only planning',
          turnOn: 'Turn plan mode on',
        },
      },
    },
  },
})

function mountMenu(overrides: Record<string, unknown> = {}) {
  const attachFiles = vi.fn()
  const activatePlanMode = vi.fn()
  const openCommands = vi.fn()
  const openArtifacts = vi.fn()
  const openFileTree = vi.fn()
  const openSkills = vi.fn()
  const openPlugins = vi.fn()
  const close = vi.fn()
  const host = document.createElement('div')
  document.body.appendChild(host)
  const app = createApp({
    render: () => h(ChatComposerAddMenu, {
      attachmentsDisabled: false,
      planModeActive: false,
      planModeAvailable: true,
      planModeBusy: false,
      onActivatePlanMode: activatePlanMode,
      onAttachFiles: attachFiles,
      onOpenCommands: openCommands,
      onOpenArtifacts: openArtifacts,
      onOpenFileTree: openFileTree,
      onOpenSkills: openSkills,
      onOpenPlugins: openPlugins,
      onClose: close,
      ...overrides,
    }),
  })
  mountedApps.push(app)
  app.use(i18n)
  app.mount(host)
  return {
    activatePlanMode,
    attachFiles,
    openCommands,
    openArtifacts,
    openFileTree,
    openSkills,
    openPlugins,
    close,
    host,
  }
}

afterEach(() => {
  while (mountedApps.length) mountedApps.pop()?.unmount()
  document.body.innerHTML = ''
})

describe('ChatComposerAddMenu', () => {
  it('offers attachment, commands, artifacts, workspace files, skills and plugins', async () => {
    const { host, openCommands, openArtifacts, openPlugins, close } = mountMenu()
    await nextTick()

    const items = [...host.querySelectorAll<HTMLButtonElement>('[role="menuitem"]')]
    const texts = items.map(item => item.textContent?.trim() ?? '')
    expect(texts).toHaveLength(7)
    expect(texts[0]).toContain('Attach files')
    expect(texts.some(text => text.includes('Commands'))).toBe(true)
    expect(texts.some(text => text.includes('My artifacts'))).toBe(true)
    expect(texts.some(text => text.includes('Workspace files'))).toBe(true)
    expect(texts.some(text => text.includes('Skills'))).toBe(true)
    expect(texts.some(text => text.includes('Plugins'))).toBe(true)

    const byText = (needle: string) =>
      items.find(item => item.textContent?.includes(needle)) as HTMLButtonElement
    byText('Commands').click()
    expect(openCommands).toHaveBeenCalledOnce()
    byText('My artifacts').click()
    expect(openArtifacts).toHaveBeenCalledOnce()
    byText('Plugins').click()
    expect(openPlugins).toHaveBeenCalledOnce()
    expect(close).toHaveBeenCalledTimes(3)
  })

  it('offers an on-demand Plan mode entry that is disabled while active', async () => {
    const { activatePlanMode, host } = mountMenu()
    await nextTick()
    const planItem = [...host.querySelectorAll<HTMLButtonElement>('[role="menuitem"]')]
      .find(item => item.textContent?.includes('Plan mode'))
    expect(planItem).toBeTruthy()
    planItem!.click()
    expect(activatePlanMode).toHaveBeenCalledOnce()

    const active = mountMenu({ planModeActive: true })
    await nextTick()
    const activePlan = [...active.host.querySelectorAll<HTMLButtonElement>('[role="menuitem"]')]
      .find(item => item.textContent?.includes('Plan mode'))
    expect(activePlan?.disabled).toBe(true)
    expect(activePlan?.getAttribute('aria-pressed')).toBe('true')
  })

  it('hides Plan mode for gateways without the Plan RPC contract', async () => {
    const { host } = mountMenu({ planModeAvailable: false })
    await nextTick()

    expect(host.querySelectorAll('[role="menuitem"]')).toHaveLength(6)
    expect(host.textContent).not.toContain('Plan mode')
  })
})
