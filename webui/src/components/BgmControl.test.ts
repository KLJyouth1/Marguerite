// @vitest-environment happy-dom
import { createApp, nextTick, type App } from 'vue'
import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest'

import i18n, { loadLocaleMessages } from '@/i18n'
import BgmControl from '@/components/BgmControl.vue'

// Mock the singleton player store: every transport action spies into h, and the
// state refs are real Vue refs so the component re-renders on mutation.
const h = vi.hoisted(() => ({
  toggle: vi.fn(async () => {}),
  next: vi.fn(async () => {}),
  prev: vi.fn(async () => {}),
  seek: vi.fn(),
  setRepeat: vi.fn(),
  setShuffle: vi.fn(),
  playing: { value: false } as import('vue').Ref<boolean>,
  repeat: { value: 'off' } as import('vue').Ref<'off' | 'all' | 'one'>,
  shuffle: { value: false } as import('vue').Ref<boolean>,
  currentTime: { value: 12 } as import('vue').Ref<number>,
  duration: { value: 75 } as import('vue').Ref<number>,
}))

vi.mock('@/composables/useBgm', async () => {
  const { ref } = await import('vue')
  h.playing = ref(false)
  h.repeat = ref('off')
  h.shuffle = ref(false)
  h.currentTime = ref(12)
  h.duration = ref(75)
  return {
    useBgm: () => ({
      tracks: [
        { id: 'dusk-pad', title: 'Dusk Pad', src: 'dusk-pad.mp3' },
        { id: 'neon-arp', title: 'Neon Arpeggio', src: 'neon-arp.mp3' },
      ],
      playing: h.playing,
      currentTrackId: ref('dusk-pad'),
      currentTitle: ref('Dusk Pad'),
      currentTime: h.currentTime,
      duration: h.duration,
      repeat: h.repeat,
      shuffle: h.shuffle,
      volume: ref(0.6),
      localTrackTitle: ref(''),
      playlistError: ref(false),
      enabled: ref(true),
      amplitude: ref(0),
      audio: {
        spectrum: ref(new Uint8Array(0)),
        waveform: ref(new Uint8Array(0)),
        bass: ref(0),
        mid: ref(0),
        treble: ref(0),
        energy: ref(0),
        beat: ref(false),
      },
      initBgm: vi.fn(async () => {}),
      toggle: h.toggle,
      next: h.next,
      prev: h.prev,
      seek: h.seek,
      setRepeat: h.setRepeat,
      setShuffle: h.setShuffle,
      selectTrack: vi.fn(async () => {}),
      setVolume: vi.fn(),
      playLocalFile: vi.fn(async () => {}),
    }),
    BGM_LOCAL_TRACK_ID: '__local__',
    BGM_SYNTH_TRACK_ID: '__synth__',
  }
})

let app: App<Element> | null = null

afterEach(() => {
  app?.unmount()
  app = null
  document.body.innerHTML = ''
  vi.clearAllMocks()
})

beforeAll(async () => {
  await loadLocaleMessages('en')
  i18n.global.locale.value = 'en'
})

async function mountControl() {
  const host = document.createElement('div')
  document.body.appendChild(host)
  app = createApp(BgmControl)
  app.use(i18n)
  app.mount(host)
  await nextTick()
  return host
}

describe('BgmControl — collapsed trigger', () => {
  it('renders the note toggle and opens the player panel on click', async () => {
    const el = await mountControl()
    const toggle = el.querySelector('[data-testid="bgm-toggle"]') as HTMLButtonElement
    expect(toggle).toBeTruthy()
    expect(toggle.getAttribute('aria-label')).toBe('Play background music')
    expect(el.querySelector('[data-testid="bgm-prev"]')).toBeNull()

    el.querySelector('[data-testid="bgm-menu-trigger"]')!.dispatchEvent(new MouseEvent('click', { bubbles: true }))
    await nextTick()
    expect(el.querySelector('[data-testid="bgm-prev"]')).toBeTruthy()
    expect(el.querySelector('[data-testid="bgm-playpause"]')).toBeTruthy()
    expect(el.querySelector('[data-testid="bgm-next"]')).toBeTruthy()
    expect(el.querySelector('[data-testid="bgm-repeat"]')).toBeTruthy()
    expect(el.querySelector('[data-testid="bgm-shuffle"]')).toBeTruthy()
    expect(el.querySelector('[data-testid="bgm-seek"]')).toBeTruthy()
  })

  it('swaps the note glyph for a bouncing equaliser while playing', async () => {
    const el = await mountControl()
    expect(el.querySelector('[data-testid="bgm-toggle"] .bgm-eq')).toBeNull()
    h.playing.value = true
    await nextTick()
    const eq = el.querySelector('[data-testid="bgm-toggle"] .bgm-eq')
    expect(eq).toBeTruthy()
    expect(eq!.querySelectorAll('span')).toHaveLength(3)
  })
})

describe('BgmControl — transport wiring', () => {
  it('dispatches toggle / prev / next into the player store', async () => {
    const el = await mountControl()
    el.querySelector('[data-testid="bgm-menu-trigger"]')!.dispatchEvent(new MouseEvent('click', { bubbles: true }))
    await nextTick()

    ;(el.querySelector('[data-testid="bgm-playpause"]') as HTMLButtonElement).click()
    expect(h.toggle).toHaveBeenCalledTimes(1)
    ;(el.querySelector('[data-testid="bgm-prev"]') as HTMLButtonElement).click()
    expect(h.prev).toHaveBeenCalledTimes(1)
    ;(el.querySelector('[data-testid="bgm-next"]') as HTMLButtonElement).click()
    expect(h.next).toHaveBeenCalledTimes(1)
  })

  it('cycles repeat off → all → one and reflects state in aria', async () => {
    const el = await mountControl()
    el.querySelector('[data-testid="bgm-menu-trigger"]')!.dispatchEvent(new MouseEvent('click', { bubbles: true }))
    await nextTick()
    const btn = el.querySelector('[data-testid="bgm-repeat"]') as HTMLButtonElement

    btn.click()
    expect(h.setRepeat).toHaveBeenCalledWith('all')
    h.repeat.value = 'all'
    await nextTick()
    expect(btn.getAttribute('aria-pressed')).toBe('true')
    expect(btn.getAttribute('aria-label')).toBe('Repeat all playlist')

    btn.click()
    expect(h.setRepeat).toHaveBeenCalledWith('one')
  })

  it('toggles shuffle via aria-pressed', async () => {
    const el = await mountControl()
    el.querySelector('[data-testid="bgm-menu-trigger"]')!.dispatchEvent(new MouseEvent('click', { bubbles: true }))
    await nextTick()
    const btn = el.querySelector('[data-testid="bgm-shuffle"]') as HTMLButtonElement
    btn.click()
    expect(h.setShuffle).toHaveBeenCalledWith(true)
    h.shuffle.value = true
    await nextTick()
    expect(btn.getAttribute('aria-pressed')).toBe('true')
  })
})

describe('BgmControl — seek row', () => {
  it('addresses the bar to mm:ss and calls seek on input', async () => {
    const el = await mountControl()
    el.querySelector('[data-testid="bgm-menu-trigger"]')!.dispatchEvent(new MouseEvent('click', { bubbles: true }))
    await nextTick()

    expect(el.querySelector('[data-testid="bgm-time-current"]')!.textContent).toBe('0:12')
    expect(el.querySelector('[data-testid="bgm-time-total"]')!.textContent).toBe('1:15')

    const range = el.querySelector('[data-testid="bgm-seek"]') as HTMLInputElement
    range.value = '500'
    range.dispatchEvent(new Event('input', { bubbles: true }))
    // 500/1000 * 75s duration = 37.5s
    expect(h.seek).toHaveBeenCalledWith(37.5)
  })
})
