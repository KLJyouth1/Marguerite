// @vitest-environment happy-dom
/**
 * W5.6：CacheStrip 把「服务端前缀覆盖」与「合格请求命中率」**分开**显示，
 * tooltip 写明分子/分母；旧网关数据（只有 server_cache_hit_rate）才显示
 * 旧标签并打 legacy 标记。
 */
import { createApp, nextTick, type App } from 'vue'
import { createI18n } from 'vue-i18n'
import { afterEach, describe, expect, it } from 'vitest'
import CacheStrip from './CacheStrip.vue'

const i18n = createI18n({
  legacy: false,
  locale: 'en',
  missingWarn: false,
  fallbackWarn: false,
  messages: { en: {} },
})

let app: App<Element> | null = null

afterEach(() => {
  app?.unmount()
  app = null
  document.body.innerHTML = ''
})

async function mount(props: Record<string, unknown>): Promise<HTMLElement> {
  const el = document.createElement('div')
  document.body.appendChild(el)
  app = createApp(CacheStrip, props)
  app.use(i18n)
  app.mount(el)
  await nextTick()
  return el
}

function byTestId(el: HTMLElement, id: string): HTMLElement | null {
  return el.querySelector<HTMLElement>(`[data-testid="${id}"]`)
}

describe('CacheStrip (W5.6 honest cache metrics)', () => {
  it('renders server prefix coverage and eligible request hit rate separately', async () => {
    const el = await mount({
      coveragePct: 42,
      requestHitRatePct: 80,
      clientHitRatePct: 10,
    })

    const coverage = byTestId(el, 'cache-strip-coverage')
    expect(coverage).not.toBeNull()
    expect(coverage!.textContent).toContain('Server prefix coverage')
    expect(coverage!.textContent).toContain('42%')
    // tooltip 必须写明分子/分母
    expect(coverage!.getAttribute('title')).toContain('cache_read')
    expect(coverage!.getAttribute('title')).toContain('total input')

    const request = byTestId(el, 'cache-strip-request-hit')
    expect(request).not.toBeNull()
    expect(request!.textContent).toContain('Eligible request hit rate')
    expect(request!.textContent).toContain('80%')
    expect(request!.getAttribute('title')).toContain('cache_read > 0')
    expect(request!.getAttribute('title')).toContain('minimum cacheable')

    const client = byTestId(el, 'cache-strip-client')
    expect(client).not.toBeNull()
    expect(client!.textContent).toContain('Local prefix hits')
    expect(client!.textContent).toContain('10%')

    // 新字段数据不是 legacy
    expect(byTestId(el, 'cache-strip-legacy')).toBeNull()
  })

  it('keeps the old label and adds an explicit legacy marker for legacy gateways', async () => {
    const el = await mount({
      coveragePct: 33,
      requestHitRatePct: null,
      legacy: true,
    })

    const coverage = byTestId(el, 'cache-strip-coverage')
    expect(coverage!.textContent).toContain('Cache hit rate (legacy)')
    expect(coverage!.textContent).not.toContain('Server prefix coverage')
    expect(coverage!.textContent).toContain('33%')

    const legacy = byTestId(el, 'cache-strip-legacy')
    expect(legacy).not.toBeNull()
    expect(legacy!.textContent?.trim().toLowerCase()).toBe('legacy')
    expect(legacy!.getAttribute('title')).toContain('server_cache_hit_rate')

    // 请求级命中率不可得 → 显示占位，不拿旧口径冒充
    const request = byTestId(el, 'cache-strip-request-hit')
    expect(request!.textContent).toContain('—')
  })

  it('renders placeholders (and no legacy marker) when everything is missing', async () => {
    const el = await mount({ coveragePct: null, requestHitRatePct: null })

    expect(byTestId(el, 'cache-strip-coverage')!.textContent).toContain('—')
    expect(byTestId(el, 'cache-strip-request-hit')!.textContent).toContain('—')
    expect(byTestId(el, 'cache-strip-client')).toBeNull()
    expect(byTestId(el, 'cache-strip-legacy')).toBeNull()
  })

  it('never renders the client metric as if it were a server metric', async () => {
    const el = await mount({
      coveragePct: null,
      requestHitRatePct: null,
      clientHitRatePct: 99,
    })

    const coverage = byTestId(el, 'cache-strip-coverage')
    expect(coverage!.textContent).toContain('—')
    expect(coverage!.textContent).not.toContain('99%')
    expect(byTestId(el, 'cache-strip-client')!.textContent).toContain('99%')
  })
})
