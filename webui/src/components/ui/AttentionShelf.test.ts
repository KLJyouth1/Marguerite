// @vitest-environment happy-dom
import { createApp, nextTick, type App } from 'vue'
import { afterEach, describe, expect, it, vi } from 'vitest'

import i18n from '@/i18n'

import AttentionShelf from './AttentionShelf.vue'
import { attentionCounts, rankAttention, type AttentionItem } from '@/composables/ui/attentionQueue'

const NOW = 1_800_000_000_000

function item(id: string, ageMs: number, over: Partial<AttentionItem> = {}): AttentionItem {
  return {
    id,
    kind: 'approval',
    at: NOW - ageMs,
    title: `tool-${id}`,
    detail: 'npm run build',
    target: { kind: 'approval', approvalId: id, sessionKey: 'sess-1' },
    ...over,
  }
}

let app: App | null = null

function mount(props: Record<string, unknown>) {
  const host = document.createElement('div')
  document.body.appendChild(host)
  app = createApp(AttentionShelf, { now: NOW, reducedMotion: false, motionKey: 0, cue: null, ...props })
  app.use(i18n)
  app.mount(host)
  return host
}

afterEach(() => {
  app?.unmount()
  app = null
  document.body.innerHTML = ''
})

describe('AttentionShelf — 收起态', () => {
  it('只显示一个胶囊，徽标是全部待办数（不是可见切片数）', async () => {
    const items = [item('a1', 0), item('a2', 5_000), item('a3', 9_000)]
    const host = mount({
      items: rankAttention(items, NOW, 2),
      counts: attentionCounts(items),
      total: 3,
      expanded: false,
    })
    await nextTick()

    const pill = host.querySelector('.attn-pill')
    expect(pill).not.toBeNull()
    expect(pill?.querySelector('.attn-pill__count')?.textContent).toBe('3')
    // 只有两行可见，但徽标必须说 3 —— 货架的计数口径是「债务」，不是「行数」。
    expect(host.querySelectorAll('.attn-dot').length).toBe(1)
  })

  it('胶囊带 aria-expanded=false 与 aria-controls，展开面板不渲染', async () => {
    const host = mount({
      items: rankAttention([item('a1', 0)], NOW, 5),
      counts: attentionCounts([item('a1', 0)]),
      total: 1,
      expanded: false,
    })
    await nextTick()

    const pill = host.querySelector('.attn-pill') as HTMLButtonElement
    expect(pill.getAttribute('aria-expanded')).toBe('false')
    expect(pill.getAttribute('aria-controls')).toBe('attn-panel')
    expect(host.querySelector('#attn-panel')).toBeNull()
  })
})

describe('AttentionShelf — 展开态', () => {
  it('逐行渲染标题/详情/类别与相对时间', async () => {
    const items = [item('a1', 12 * 60_000), item('a2', 90 * 60_000)]
    const host = mount({
      items: rankAttention(items, NOW, 5),
      counts: attentionCounts(items),
      total: 2,
      expanded: true,
    })
    await nextTick()

    const rows = host.querySelectorAll('.attn-item')
    expect(rows).toHaveLength(2)
    expect(rows[0].querySelector('.attn-item__title')?.textContent).toBe('tool-a1')
    expect(rows[0].querySelector('.attn-item__meta')?.textContent).toContain('12')
    expect(rows[1].querySelector('.attn-item__meta')?.textContent).toContain('1')
  })

  it('点击行发出 open（货架只做路由，绝不就地裁决）', async () => {
    const onAction = vi.fn()
    const items = [item('a1', 0)]
    const host = mount({
      items: rankAttention(items, NOW, 5),
      counts: attentionCounts(items),
      total: 1,
      expanded: true,
      onAction,
    })
    await nextTick()

    host.querySelector<HTMLButtonElement>('.attn-item__hit')?.click()
    await nextTick()

    expect(onAction).toHaveBeenCalledTimes(1)
    const [action, payload] = onAction.mock.calls[0]
    expect(action).toBe('open')
    expect((payload as AttentionItem).id).toBe('a1')
  })

  it('没有可路由 target 的条目照样渲染（不得静默丢弃债务）', async () => {
    const items = [item('a1', 0, { target: undefined })]
    const host = mount({
      items: rankAttention(items, NOW, 5),
      counts: attentionCounts(items),
      total: 1,
      expanded: true,
    })
    await nextTick()

    expect(host.querySelectorAll('.attn-item')).toHaveLength(1)
  })

  it('空列表给出明确空态，而不是一个空白面板', async () => {
    const host = mount({
      items: [],
      counts: attentionCounts([]),
      total: 0,
      expanded: true,
    })
    await nextTick()

    expect(host.querySelector('.attn-empty')).not.toBeNull()
  })
})

describe('AttentionShelf — 动效属性', () => {
  it('cue 为 null 时不产出 data-motion（静止就是静止）', async () => {
    const items = [item('a1', 0)]
    const host = mount({
      items: rankAttention(items, NOW, 5),
      counts: attentionCounts(items),
      total: 1,
      expanded: false,
      cue: null,
    })
    await nextTick()

    expect(host.querySelector('.attn-pill')?.hasAttribute('data-motion')).toBe(false)
  })

  it('arrive 落在胶囊上；reduced-motion 下标记 static（仍有反馈，只是不位移）', async () => {
    const items = [item('a1', 0)]
    const host = mount({
      items: rankAttention(items, NOW, 5),
      counts: attentionCounts(items),
      total: 1,
      expanded: false,
      cue: 'arrive',
      reducedMotion: true,
    })
    await nextTick()

    const pill = host.querySelector('.attn-pill')
    expect(pill?.getAttribute('data-motion')).toBe('arrive')
    expect(pill?.getAttribute('data-motion-static')).toBe('true')
  })
})
