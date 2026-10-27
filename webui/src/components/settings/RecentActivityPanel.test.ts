// @vitest-environment happy-dom
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { createApp, nextTick, type App } from 'vue'
import i18n from '@/i18n'
import zhHans from '@/locales/zh-Hans.json'

// 真机缺陷（2026-10-24，安装版 0.2.16）：/usage「最近活动」里两条「切换模式 core」
// 显示成「—」→「—」。写入侧 core::events::product::audit 会把业务字段包在
// `fields` 里（emit 才是平铺），前端却只读顶层 from/to，于是永远取不到值。
const h = vi.hoisted(() => ({
  call: null as unknown as ReturnType<typeof vi.fn>,
}))

vi.mock('@/stores/rpc', async () => {
  const { reactive } = await import('vue')
  const call = vi.fn()
  h.call = call
  return {
    useRpcStore: () => reactive({
      call,
      waitForConnection: vi.fn(() => Promise.resolve()),
    }),
  }
})

import RecentActivityPanel from './RecentActivityPanel.vue'

let app: App<Element> | null = null

const flush = () => new Promise(resolve => setTimeout(resolve, 0))

async function mountPanel(entries: unknown[]): Promise<HTMLElement> {
  h.call.mockResolvedValue({ entries, total: entries.length })
  const el = document.createElement('div')
  document.body.appendChild(el)
  app = createApp(RecentActivityPanel)
  app.use(i18n)
  app.mount(el)
  await flush()
  await nextTick()
  return el
}

beforeEach(() => {
  document.body.innerHTML = ''
  i18n.global.setLocaleMessage('zh-Hans', zhHans)
  i18n.global.locale.value = 'zh-Hans'
})

describe('RecentActivityPanel — 切换模式前后值', () => {
  it('读取 audit 嵌套在 fields 里的 from/to', async () => {
    const el = await mountPanel([
      {
        id: 2,
        ts: '2026-10-24T10:00:00Z',
        session: 'core',
        method: 'event.mode.changed',
        event: 'mode.changed',
        fields: { from: 'agent', to: 'bloom' },
      },
    ])

    const detail = el.querySelector('.activity-detail')?.textContent?.trim()
    expect(detail).toBe('「agent」→「bloom」')
    app?.unmount()
  })

  it('兼容事件字段平铺的老记录', async () => {
    const el = await mountPanel([
      {
        id: 1,
        ts: '2026-10-24T09:00:00Z',
        session: 'core',
        method: 'event.mode.changed',
        event: 'mode.changed',
        from: 'plan',
        to: 'agent',
      },
    ])

    expect(el.querySelector('.activity-detail')?.textContent?.trim())
      .toBe('「plan」→「agent」')
    app?.unmount()
  })

  it('确实缺前后值时只显示事件摘要，不再渲染「—」→「—」', async () => {
    const el = await mountPanel([
      {
        id: 3,
        ts: '2026-10-24T11:00:00Z',
        session: 'core',
        method: 'event.mode.changed',
        event: 'mode.changed',
      },
    ])

    const row = el.querySelector('.activity-row')
    expect(row?.textContent).toContain('切换模式')
    expect(row?.textContent).toContain('core')
    expect(el.querySelector('.activity-detail')).toBeNull()
    expect(row?.textContent).not.toContain('—')
    app?.unmount()
  })
})
