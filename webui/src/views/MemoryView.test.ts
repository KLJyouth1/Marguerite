// @vitest-environment happy-dom
// 记忆一等页：页头标题/副标题走 memoryPage.* 两个键，六块设置面板成列挂载。
// 面板用 stub 顶替 —— 真实面板挂载即发 RPC（memory.* / profile.* / experts.*），
// 那是面板自己的契约测试，这里只验证「页面有没有把它们都放上来」。
import { createApp, nextTick, type App } from 'vue'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import i18n from '@/i18n'

// 函数声明（而非 const）是刻意的：vi.mock 会被提升到文件顶部，工厂参数在
// 提升位置求值，只有函数声明的绑定在那一刻已经初始化。
function stub(name: string, testid: string) {
  return async () => {
    const { defineComponent, h } = await import('vue')
    return {
      default: defineComponent({
        name,
        setup: () => () => h('section', { 'data-testid': testid }),
      }),
    }
  }
}

vi.mock('@/components/settings/CrossProjectMemoryPanel.vue', stub('CrossProjectMemoryPanelStub', 'stub-cross-project-memory'))
vi.mock('@/components/settings/KnowledgeBasePanel.vue', stub('KnowledgeBasePanelStub', 'stub-knowledge-base'))
vi.mock('@/components/settings/MemoryLayersPanel.vue', stub('MemoryLayersPanelStub', 'stub-memory-layers'))
vi.mock('@/components/settings/ProfileCard.vue', stub('ProfileCardStub', 'stub-profile-card'))
vi.mock('@/components/settings/ExpertsPanel.vue', stub('ExpertsPanelStub', 'stub-experts'))
vi.mock('@/components/settings/SettingsMemoryPanel.vue', stub('SettingsMemoryPanelStub', 'stub-settings-memory'))

import MemoryView from '@/views/MemoryView.vue'
import { sharedRoutes } from '@/router/sharedRoutes'

const PANEL_STUBS = [
  'stub-cross-project-memory',
  'stub-knowledge-base',
  'stub-memory-layers',
  'stub-profile-card',
  'stub-experts',
  'stub-settings-memory',
]

let app: App<Element> | null = null
let host: HTMLElement | null = null

async function mountView(): Promise<HTMLElement> {
  host = document.createElement('div')
  document.body.appendChild(host)
  app = createApp(MemoryView)
  app.use(i18n)
  app.mount(host)
  await nextTick()
  return host
}

beforeEach(() => {
  i18n.global.locale.value = 'en'
})

afterEach(() => {
  app?.unmount()
  app = null
  host?.remove()
  host = null
})

describe('MemoryView page shell', () => {
  it('renders the memoryPage title and subtitle in the stage header', async () => {
    const el = await mountView()
    const title = el.querySelector('.control-stage__title')
    const subtitle = el.querySelector('.control-stage__subtitle')

    expect(title?.textContent?.trim()).toBe('Memory')
    // 渲染的是解析后的文案，不是原始键名。
    expect(title?.textContent?.trim()).not.toBe('memoryPage.title')
    expect(subtitle?.textContent?.trim()).toBe(
      'Profiles, knowledge, layers, experts and cross-project memory.',
    )
    expect(subtitle?.textContent?.trim()).not.toBe('memoryPage.subtitle')
  })

  it('mounts all six memory panels as one vertical stack', async () => {
    const el = await mountView()
    const stack = el.querySelector('.memory-stack')

    expect(stack).not.toBeNull()
    for (const testid of PANEL_STUBS) {
      expect(el.querySelector(`[data-testid="${testid}"]`), testid).not.toBeNull()
    }
    expect(stack?.querySelectorAll('[data-testid^="stub-"]').length).toBe(PANEL_STUBS.length)
  })
})

describe('/memory route meta', () => {
  it('registers a first-class Settings destination next to /rules', () => {
    const route = sharedRoutes.find((candidate) => candidate.path === '/memory')

    expect(route).toBeDefined()
    expect(route?.name).toBe('memory')
    expect(route?.component).toBeTypeOf('function')
    expect(route?.meta?.titleKey).toBe('memoryPage.title')
    expect(route?.meta?.navLabelKey).toBe('memoryPage.navLabel')
    expect(route?.meta?.group).toBe('Settings')
    expect(route?.meta?.icon).toBe('user')
    expect(route?.meta?.nav).toBe('primary')
    expect(route?.meta?.platforms).toEqual(['web', 'desktop'])
    expect(route?.meta?.keepAlive).toBe(true)
    expect(route?.meta?.navOrder).toBe(95)
  })

  it('keeps navOrder unique inside the Settings group', () => {
    const settingsOrders = sharedRoutes
      .filter((route) => route.meta?.group === 'Settings')
      .map((route) => route.meta?.navOrder)
      .filter((order): order is number => typeof order === 'number')

    expect(settingsOrders).toContain(95)
    expect(new Set(settingsOrders).size).toBe(settingsOrders.length)
  })
})
