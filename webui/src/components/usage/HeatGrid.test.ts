// @vitest-environment happy-dom

import { createApp, nextTick } from 'vue'
import { afterEach, describe, expect, it } from 'vitest'
import i18n from '@/i18n'
import HeatGrid from './HeatGrid.vue'

const mounted: Array<ReturnType<typeof createApp>> = []

afterEach(() => {
  mounted.splice(0).forEach(app => app.unmount())
  document.body.innerHTML = ''
})

function mount(points: Array<{ dayMs: number; totalTokens: number; calls: number }>, state = 'ready') {
  i18n.global.locale.value = 'en'
  const root = document.createElement('div')
  document.body.appendChild(root)
  const app = createApp(HeatGrid, {
    points,
    state,
    weeks: 2,
    now: new Date('2024-01-03T12:00:00Z'),
  })
  app.use(i18n)
  app.mount(root)
  mounted.push(app)
  return root
}

describe('HeatGrid', () => {
  it('renders one cell per day and marks future days without a tooltip', async () => {
    const root = mount([
      { dayMs: Date.parse('2024-01-01T00:00:00Z'), totalTokens: 100, calls: 4 },
      { dayMs: Date.parse('2024-01-02T00:00:00Z'), totalTokens: 40, calls: 2 },
    ])
    await nextTick()

    const cells = Array.from(root.querySelectorAll<HTMLElement>('.usage-heatmap__grid .usage-heatmap__cell'))
    expect(cells).toHaveLength(14)
    expect(cells.filter(cell => cell.classList.contains('usage-heatmap__cell--future'))).toHaveLength(4)
    expect(cells[7]?.classList.contains('usage-heatmap__cell--l4')).toBe(true)
    expect(cells[8]?.classList.contains('usage-heatmap__cell--l2')).toBe(true)
    expect(cells[0]?.getAttribute('title')).toContain('2023')
    expect(cells[13]?.getAttribute('title')).toBeNull()
  })

  it('shows an explicit empty state when no day has usage', async () => {
    const root = mount([])
    await nextTick()
    expect(root.querySelector('.usage-heatmap__empty')?.textContent).toContain('No daily usage yet')
    expect(root.querySelector('.usage-heatmap__grid')).toBeNull()
  })

  it('distinguishes unavailable data from an empty history', async () => {
    const root = mount([], 'error')
    await nextTick()
    expect(root.querySelector('.usage-heatmap__empty')?.textContent).toContain('unavailable')
    expect(root.querySelector('.usage-heatmap__empty')?.textContent).not.toContain('No daily usage yet')
  })

  it('keeps the skeleton grid while loading', async () => {
    const root = mount([], 'loading')
    await nextTick()
    expect(root.querySelector('.usage-heatmap__empty')).toBeNull()
    expect(root.querySelectorAll('.usage-heatmap__grid .usage-heatmap__cell')).toHaveLength(14)
  })
})
