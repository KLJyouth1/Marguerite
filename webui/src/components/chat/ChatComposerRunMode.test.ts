// @vitest-environment happy-dom
import { afterEach, describe, expect, it } from 'vitest'
import { createApp } from 'vue'
import { createI18n } from 'vue-i18n'

import ChatComposerRunMode from './ChatComposerRunMode.vue'
import type { SandboxSetupState } from '@/types/sandbox'

let unmount: (() => void) | null = null

function mount(
  allowedRunModes: Array<'safe' | 'full'>,
  safeSetupAvailable = false,
  onSetRunMode?: (mode: 'safe' | 'full') => void,
  options: { safeSetupState?: SandboxSetupState | null; runModeLocked?: boolean } = {},
) {
  const el = document.createElement('div')
  document.body.appendChild(el)
  const app = createApp(ChatComposerRunMode, {
    runMode: 'full',
    allowedRunModes,
    safeSetupAvailable,
    safeSetupState: options.safeSetupState ?? null,
    runModeLocked: options.runModeLocked ?? false,
    runModeLockMessage: 'Run mode cannot be changed while this session has active work.',
    onSetRunMode,
  })
  app.use(createI18n({
    legacy: false,
    locale: 'en',
    messages: {
      en: {
        chat: {
          closeComposerSettings: 'Close',
          composer: {
            runMode: 'Run mode',
            runModeSafe: 'Safe',
            runModeSafeDesc: 'Sandboxed',
            runModeFull: 'Full access',
            runModeFullDesc: 'Host access',
            runModeLocked: 'Run mode cannot be changed while this session has active work.',
            runModePolicyBlocked: 'The current gateway policy does not allow this run mode.',
          },
        },
        settings: {
          sandbox: {
            safeMode: {
              pendingLabel: 'Safe (setup pending)',
              pendingComposerDesc: 'Your choice is kept; the isolation runtime is not installed.',
            },
          },
        },
      },
    },
  }))
  app.mount(el)
  unmount = () => app.unmount()
  return el
}

afterEach(() => {
  unmount?.()
  unmount = null
  document.body.innerHTML = ''
})

describe('ChatComposerRunMode', () => {
  it('renders exactly Safe and Full', () => {
    const el = mount(['safe', 'full'])
    const radios = [...el.querySelectorAll<HTMLButtonElement>('[role="radio"]')]
    expect(radios).toHaveLength(2)
    expect(radios.map(radio => radio.textContent?.trim())).toEqual([
      'SafeSandboxed',
      'Full accessHost access',
    ])
  })

  it('keeps Safe clickable when setup is unavailable (no silent no-op)', () => {
    const selected: Array<'safe' | 'full'> = []
    const el = mount(['safe', 'full'], false, mode => selected.push(mode), {
      safeSetupState: 'unavailable',
    })
    const radios = [...el.querySelectorAll<HTMLButtonElement>('[role="radio"]')]

    expect(radios[0].disabled).toBe(false)
    expect(radios[0].textContent).toContain('Safe (setup pending)')

    radios[0].click()
    expect(selected).toEqual(['safe'])
  })

  it('lets a repairable Safe choice request first-time setup', () => {
    const selected: Array<'safe' | 'full'> = []
    const el = mount(['safe', 'full'], true, mode => selected.push(mode), {
      safeSetupState: 'not_setup',
    })
    const radios = [...el.querySelectorAll<HTMLButtonElement>('[role="radio"]')]

    expect(radios[0].disabled).toBe(false)
    radios[0].click()

    expect(selected).toEqual(['safe'])
    expect(radios[0].getAttribute('aria-checked')).toBe('false')
  })

  it('disables both options with a visible reason only while the session is locked', () => {
    const el = mount(['safe', 'full'], true, undefined, { runModeLocked: true })
    const radios = [...el.querySelectorAll<HTMLButtonElement>('[role="radio"]')]
    expect(radios.map(radio => radio.disabled)).toEqual([true, true])
    expect(el.querySelector('.composer-run-mode__hint')?.textContent)
      .toContain('Run mode cannot be changed')
  })
})
