// @vitest-environment happy-dom
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { createPinia, setActivePinia } from 'pinia'
import { useAppStore } from './app'

const KEY = 'marguerite-boot-animation'

function stubMatchMedia(matches = false) {
  window.matchMedia = vi.fn().mockImplementation((query: string) => ({
    matches,
    media: query,
    onchange: null,
    addEventListener: vi.fn(),
    removeEventListener: vi.fn(),
    addListener: vi.fn(),
    removeListener: vi.fn(),
    dispatchEvent: vi.fn(),
  })) as unknown as typeof window.matchMedia
}

describe('app store — boot splash animation tier', () => {
  beforeEach(() => {
    setActivePinia(createPinia())
    localStorage.clear()
    stubMatchMedia(false)
    document.documentElement.removeAttribute('data-boot')
  })

  it('defaults to "full" and stamps it on <html> when nothing is saved', () => {
    const store = useAppStore()
    expect(store.bootAnimation).toBe('full')
    expect(document.documentElement.getAttribute('data-boot')).toBe('full')
  })

  it('hydrates a saved tier and persists + stamps an explicit choice', () => {
    localStorage.setItem(KEY, 'off')
    const store = useAppStore()
    expect(store.bootAnimation).toBe('off')
    expect(document.documentElement.getAttribute('data-boot')).toBe('off')

    store.setBootAnimation('minimal')
    expect(store.bootAnimation).toBe('minimal')
    expect(localStorage.getItem(KEY)).toBe('minimal')
    expect(document.documentElement.getAttribute('data-boot')).toBe('minimal')
  })

  it('ignores an unknown saved value and falls back to "full"', () => {
    localStorage.setItem(KEY, 'loud')
    const store = useAppStore()
    expect(store.bootAnimation).toBe('full')
    expect(document.documentElement.getAttribute('data-boot')).toBe('full')
  })

  it('degrades "full" to "minimal" when the system requests reduced motion', () => {
    stubMatchMedia(true)
    const store = useAppStore()
    // The stored preference stays 'full'; only the stamped attribute degrades.
    expect(store.bootAnimation).toBe('full')
    expect(document.documentElement.getAttribute('data-boot')).toBe('minimal')
  })
})
