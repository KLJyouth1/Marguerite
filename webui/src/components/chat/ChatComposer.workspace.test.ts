// @vitest-environment happy-dom
import { afterEach, describe, expect, it, vi } from 'vitest'
import { createApp, nextTick } from 'vue'
import { createPinia } from 'pinia'
import i18n from '@/i18n'
import ChatComposer from './ChatComposer.vue'

afterEach(() => {
  document.body.innerHTML = ''
})

describe('ChatComposer project draft', () => {
  function composerProps(overrides: Record<string, unknown> = {}) {
    return {
      modelValue: '',
      'onUpdate:modelValue': () => {},
      attachments: [],
      busySendMode: 'queue',
      hasSendContent: false,
      isStreaming: false,
      canStop: false,
      isNewLanding: true,
      placeholder: 'Send a message',
      sendButtonTitle: 'Send',
      runMode: 'safe',
      allowedRunModes: ['safe', 'full'],
      runModeLocked: false,
      runModeLockMessage: '',
      modelRoutingMode: 'off',
      modelRoutingSettingsBusy: false,
      routerVisualEffectsEnabled: true,
      codingModeEnabled: false,
      codingModeSettingsBusy: false,
      voiceBusy: false,
      voiceRecording: false,
      voiceReady: true,
      projectWorkspace: {
        id: 'project-a',
        name: 'Project A',
        path: 'D:\\repos\\project-a',
      },
      ...overrides,
    }
  }

  it('uses a folder-styled project chooser in the context bar', async () => {
    const chooseProject = vi.fn()
    const host = document.createElement('div')
    document.body.appendChild(host)
    const app = createApp(ChatComposer, composerProps({
      projectWorkspace: null,
      onChooseProject: chooseProject,
    }))
    app.use(createPinia())
    app.use(i18n)
    app.mount(host)
    await nextTick()

    const chooser = host.querySelector<HTMLButtonElement>('.ccc-chip--button')
    expect(chooser).toBeTruthy()
    // 上下文条在输入框上方（Codex 式），不再是底栏里的小胶囊。
    expect(chooser?.closest('.chat-input-panel')).toBeTruthy()
    expect(chooser?.closest('.chat-input-footer')).toBeNull()
    chooser?.click()
    expect(chooseProject).toHaveBeenCalledOnce()

    app.unmount()
  })

  it('hides the project chooser when the principal cannot create projects', async () => {
    const host = document.createElement('div')
    document.body.appendChild(host)
    const app = createApp(ChatComposer, composerProps({
      projectWorkspace: null,
      canChooseProject: false,
    }))
    app.use(createPinia())
    app.use(i18n)
    app.mount(host)
    await nextTick()

    expect(host.querySelector('.ccc-chip--button')).toBeNull()

    app.unmount()
  })

  it('shows a new project draft in the context bar and lets it close', async () => {
    const closeProject = vi.fn()
    const host = document.createElement('div')
    document.body.appendChild(host)
    const app = createApp(ChatComposer, composerProps({
      canCloseProject: true,
      onCloseProject: closeProject,
    }))
    app.use(createPinia())
    app.use(i18n)
    app.mount(host)
    await nextTick()

    expect(host.querySelector('.ccc-chip__text')?.textContent?.trim()).toBe('Project A')
    expect(host.querySelector('.ccc-chip')?.getAttribute('title')).toBe('D:\\repos\\project-a')
    expect(host.querySelector('.ccc-chip')?.closest('.chat-input-panel')).toBeTruthy()
    host.querySelector<HTMLButtonElement>('.ccc-chip--icon')?.click()
    expect(closeProject).toHaveBeenCalledOnce()

    app.unmount()
  })

  it('keeps project context visible in the context bar inside a durable project task', async () => {
    const host = document.createElement('div')
    document.body.appendChild(host)
    const app = createApp(ChatComposer, composerProps({
      isNewLanding: false,
      canCloseProject: false,
    }))
    app.use(createPinia())
    app.use(i18n)
    app.mount(host)
    await nextTick()

    // 工作区属于「这次任务在哪做」的前提信息，持久任务里同样必须可见；
    // 旧实现只有落地页才显示，进入正式会话后胶囊直接消失，用户会以为没绑定。
    expect(host.querySelector('.ccc-chip__text')?.textContent?.trim()).toBe('Project A')

    app.unmount()
  })

  it('shows additional project chips and opens the multi-select dialog', async () => {
    const removeProjectRoot = vi.fn()
    const host = document.createElement('div')
    document.body.appendChild(host)
    const app = createApp(ChatComposer, composerProps({
      canAttachProjectRoots: true,
      primaryProjectId: 'project-a',
      projectRoots: [
        { id: 'extra-a', name: 'Extra A', path: 'D:\\repos\\extra-a', available: true },
      ],
      projectRootOptions: [
        { id: 'project-a', name: 'Project A', path: 'D:\\repos\\project-a', available: true },
        { id: 'extra-a', name: 'Extra A', path: 'D:\\repos\\extra-a', available: true },
        { id: 'extra-b', name: 'Extra B', path: 'D:\\repos\\extra-b', available: true },
      ],
      onRemoveProjectRoot: removeProjectRoot,
    }))
    app.use(createPinia())
    app.use(i18n)
    app.mount(host)
    await nextTick()

    const chip = host.querySelector<HTMLElement>('.ccc-chip--root')
    expect(chip?.textContent).toContain('Extra A')
    expect(chip?.getAttribute('title')).toBe('D:\\repos\\extra-a')
    chip?.querySelector<HTMLButtonElement>('.ccc-root-remove')?.click()
    expect(removeProjectRoot).toHaveBeenCalledWith('extra-a')

    host.querySelector<HTMLButtonElement>('.ccc-chip--add-root')?.click()
    await nextTick()
    expect(document.body.textContent).toContain('Additional projects')
    expect(
      document.querySelector<HTMLButtonElement>('.project-picker__root[disabled]')?.textContent,
    ).toContain('Primary project')

    app.unmount()
  })

  it('announces an unavailable active project and disables sending', async () => {
    const host = document.createElement('div')
    document.body.appendChild(host)
    const app = createApp(ChatComposer, composerProps({
      isNewLanding: false,
      canCloseProject: false,
      modelValue: 'hello',
      hasSendContent: true,
      projectWorkspaceStatus: 'unavailable',
      projectStatusMessage: 'This project directory is unavailable.',
      sendBlockedMessage: 'This project directory is unavailable.',
    }))
    app.use(createPinia())
    app.use(i18n)
    app.mount(host)
    await nextTick()

    expect(host.querySelector('.ccc-chip')?.getAttribute('data-status')).toBe('unavailable')
    expect(host.querySelector('.ccc-chip__status')?.textContent).toContain('unavailable')
    expect(host.querySelector<HTMLButtonElement>('.chat-send-btn')?.disabled).toBe(true)
    expect(host.querySelector('#chat-composer-send-status')?.textContent).toContain('unavailable')

    app.unmount()
  })
})
