// @vitest-environment happy-dom
import { createApp, h, nextTick } from 'vue'
import { createI18n } from 'vue-i18n'
import { afterEach, describe, expect, it, vi } from 'vitest'

import ChatComposerPlanMode from './ChatComposerPlanMode.vue'
import chatComposerSource from './ChatComposer.vue?raw'
import runModePanelSource from './RunModePanel.vue?raw'
import type { CollaborationMode } from '@/types/plans'

const mountedApps: ReturnType<typeof createApp>[] = []
const i18n = createI18n({
  legacy: false,
  locale: 'en',
  messages: {
    en: {
      chat: {
        planMode: {
          label: 'Plan mode',
          readOnly: 'Research and propose a plan without changing files.',
          nextTurn: 'Applies to the next turn',
          updating: 'Updating…',
          turnOff: 'Turn plan mode off',
          turnOn: 'Turn plan mode on',
        },
      },
    },
  },
})

interface PlanModeProps {
  available: boolean
  mode: CollaborationMode
  busy: boolean
  appliesNextTurn: boolean
  onSetMode?: (mode: CollaborationMode) => void
}

function mountMode(props: PlanModeProps) {
  const host = document.createElement('div')
  document.body.appendChild(host)
  const app = createApp({
    render: () => h(ChatComposerPlanMode, props),
  })
  mountedApps.push(app)
  app.use(i18n)
  app.mount(host)
  return host
}

afterEach(() => {
  while (mountedApps.length) mountedApps.pop()?.unmount()
  document.body.innerHTML = ''
})

describe('ChatComposerPlanMode', () => {
  it('shows the active read-only contract and requests an explicit mode change', async () => {
    const setMode = vi.fn()
    const host = mountMode({
      available: true,
      mode: 'plan',
      busy: false,
      appliesNextTurn: true,
      onSetMode: setMode,
    })
    await nextTick()

    const group = host.querySelector<HTMLElement>('[role="group"]')
    const status = host.querySelector<HTMLElement>('[role="status"]')
    const button = host.querySelector<HTMLButtonElement>('button')
    expect(group?.getAttribute('aria-label')).toBe('Plan mode')
    expect(group?.classList.contains('composer-plan-mode')).toBe(true)
    expect(status?.textContent).toContain('Applies to the next turn')
    expect(button?.getAttribute('aria-pressed')).toBe('true')
    expect(button?.getAttribute('aria-label')).toBe('Turn plan mode off')

    button?.click()
    expect(setMode).toHaveBeenCalledWith('default')
  })

  it('stays absent when the connected gateway does not advertise plan RPCs', async () => {
    const host = mountMode({
      available: false,
      mode: 'default',
      busy: false,
      appliesNextTurn: false,
    })
    await nextTick()

    expect(host.querySelector('.composer-plan-mode')).toBeNull()
  })

  it('stays absent while plan mode is available but has not been activated', async () => {
    const host = mountMode({
      available: true,
      mode: 'default',
      busy: false,
      appliesNextTurn: false,
    })
    await nextTick()

    expect(host.querySelector('.composer-plan-mode')).toBeNull()
  })

  it('is reused inside the unified run-mode panel rendered in the composer footer', () => {
    const leftIndex = chatComposerSource.indexOf('<div class="chat-input-actions chat-input-actions--left">')
    const panelIndex = chatComposerSource.indexOf('<RunModePanel', leftIndex)
    const rightActionsIndex = chatComposerSource.indexOf(
      '<div class="chat-input-actions chat-input-actions--right">',
      leftIndex,
    )

    expect(leftIndex).toBeGreaterThanOrEqual(0)
    expect(panelIndex).toBeGreaterThan(leftIndex)
    expect(panelIndex).toBeLessThan(rightActionsIndex)
    // 协作维度在统一面板内复用 ChatComposerPlanMode
    expect(runModePanelSource).toContain('<ChatComposerPlanMode')
  })
})
