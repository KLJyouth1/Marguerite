// @vitest-environment happy-dom
/**
 * Gating regression test for the custom title-bar controls.
 *
 * The shell-only invariant worth locking down here is the browser degradation:
 * the CLI gateway serves this same bundle in a plain browser with no window, so
 * the controls must render nothing at all rather than dead buttons.
 *
 * NOTE: happy-dom has no layout engine, so geometry (the `position: absolute`
 * pin and the drag-region width) is NOT asserted here — those need a real Tauri
 * window. This file deliberately does not fake a pass for them.
 */
import { describe, it, expect, afterEach } from 'vitest'
import { createApp, defineComponent, h, type App } from 'vue'
import { createI18n } from 'vue-i18n'
import TitleBarControls from './TitleBarControls.vue'

const mounted: App<Element>[] = []

function setTauriGlobal(value: unknown) {
  const scope = window as unknown as Record<string, unknown>
  if (value === undefined) {
    delete scope.__TAURI__
    delete scope.__TAURI_INTERNALS__
    return
  }
  scope.__TAURI__ = value
}

async function mountControls() {
  const root = document.createElement('div')
  document.body.appendChild(root)
  const i18n = createI18n({
    legacy: false,
    locale: 'en',
    messages: { en: { shell: { window: {
      minimize: 'Minimize window',
      maximize: 'Maximize window',
      restore: 'Restore window',
      close: 'Close window',
    } } } },
  })
  const app = createApp(defineComponent({
    setup: () => () => h(TitleBarControls),
  }))
  app.use(i18n)
  app.mount(root)
  mounted.push(app)
  await Promise.resolve()
  return root
}

describe('TitleBarControls shell gating', () => {
  afterEach(() => {
    while (mounted.length) mounted.pop()?.unmount()
    document.body.innerHTML = ''
    setTauriGlobal(undefined)
  })

  it('renders the three window buttons inside the Tauri shell', async () => {
    setTauriGlobal({
      window: {
        getCurrentWindow: () => ({
          minimize: async () => { /* no-op */ },
          toggleMaximize: async () => { /* no-op */ },
          isMaximized: async () => false,
          close: async () => { /* no-op */ },
        }),
      },
    })
    const root = await mountControls()
    expect(root.querySelector('[data-testid="titlebar-minimize"]')).not.toBeNull()
    expect(root.querySelector('[data-testid="titlebar-maximize"]')).not.toBeNull()
    expect(root.querySelector('[data-testid="titlebar-close"]')).not.toBeNull()
  })

  it('renders nothing in the CLI gateway browser mode', async () => {
    const root = await mountControls()
    expect(root.querySelector('.title-bar-controls')).toBeNull()
    expect(root.querySelector('[data-testid="titlebar-close"]')).toBeNull()
  })

  it('does not throw when the marker exists but exposes no window module', async () => {
    setTauriGlobal({}) // marker present, `window` module absent
    const root = await mountControls()
    // Buttons render (the marker is the shell signal) but must not have crashed
    // while probing for the window handle.
    expect(root.querySelector('.title-bar-controls')).not.toBeNull()
  })
})