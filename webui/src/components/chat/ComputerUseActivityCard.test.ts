// @vitest-environment happy-dom
import { afterEach, describe, expect, it } from 'vitest'
import { createApp, nextTick, type App } from 'vue'
import i18n from '@/i18n'
import ComputerUseActivityCard from './ComputerUseActivityCard.vue'
import type { ComputerUseEvent } from '@/utils/chat/toolPayload'

const apps: App<Element>[] = []

afterEach(() => {
  apps.splice(0).forEach(app => app.unmount())
  document.body.innerHTML = ''
})

async function mountCard(events: ComputerUseEvent[]): Promise<HTMLElement> {
  const host = document.createElement('div')
  document.body.appendChild(host)
  const app = createApp(ComputerUseActivityCard, { events })
  app.use(i18n)
  app.mount(host)
  apps.push(app)
  await nextTick()
  return host
}

function event(overrides: Partial<ComputerUseEvent> = {}): ComputerUseEvent {
  return {
    id: 'cu-1',
    action: '',
    target: '',
    ok: null,
    approval: null,
    note: '',
    ...overrides,
  }
}

describe('ComputerUseActivityCard', () => {
  it('无事件时整卡不渲染', async () => {
    const host = await mountCard([])
    expect(host.querySelector('[data-testid="computer-use-card"]')).toBeNull()
  })

  it('渲染动作 / 目标 / 成败 / 审批', async () => {
    const host = await mountCard([
      event({ id: 'cu-1', action: 'click', target: '保存按钮', ok: true, approval: 'approved' }),
      event({ id: 'cu-2', action: 'type', target: '搜索框', ok: false, approval: 'pending' }),
      event({ id: 'cu-3', action: 'key', target: 'Enter', ok: null, approval: 'denied' }),
    ])
    const rows = host.querySelectorAll('[data-testid="computer-use-row"]')
    expect(rows).toHaveLength(3)
    expect(rows[0].textContent).toContain('click')
    expect(rows[0].textContent).toContain('保存按钮')
    expect(rows[0].querySelector('.cua__result.is-ok')).toBeTruthy()
    expect(rows[0].getAttribute('data-approval')).toBe('approved')
    expect(rows[1].querySelector('.cua__result.is-fail')).toBeTruthy()
    expect(rows[1].querySelector('.cua__approval--pending')).toBeTruthy()
    expect(rows[2].querySelector('.cua__result')).toBeNull()
    expect(rows[2].querySelector('.cua__approval--denied')).toBeTruthy()
    expect(rows[2].getAttribute('data-ok')).toBe('unknown')
  })

  it('缺字段只省略对应徽章，不补默认值', async () => {
    const host = await mountCard([event({ id: 'cu-1', action: 'move' })])
    const row = host.querySelector('[data-testid="computer-use-row"]')
    expect(row?.textContent).toContain('move')
    expect(row?.querySelector('.cua__target')).toBeNull()
    expect(row?.querySelector('.cua__result')).toBeNull()
    expect(row?.querySelector('[data-testid="computer-use-approval"]')).toBeNull()
    expect(row?.getAttribute('data-ok')).toBe('unknown')
  })

  it('approval=denied 时渲染拒绝徽章（引擎真实裁决）', async () => {
    const host = await mountCard([event({ id: 'cu-denied', action: 'click', approval: 'denied' })])
    const row = host.querySelector('[data-testid="computer-use-row"]')
    const badge = row?.querySelector('[data-testid="computer-use-approval"]')
    expect(row?.getAttribute('data-approval')).toBe('denied')
    expect(badge?.classList.contains('cua__approval--denied')).toBe(true)
    expect(badge?.textContent?.trim().length).toBeGreaterThan(0)
  })
})
