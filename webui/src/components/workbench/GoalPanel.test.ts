// @vitest-environment happy-dom
import { createApp, nextTick } from 'vue'
import { afterEach, describe, expect, it, vi } from 'vitest'
import GoalPanel from './GoalPanel.vue'
import { createGoalWorkbenchDefinition } from './goalWorkbenchProvider'
import { createGoalWorkbenchItem } from '@/workbench/goalItems'

const call = vi.fn()
const on = vi.fn(() => () => undefined)
vi.mock('@/stores/rpc', () => ({
  useRpcStore: () => ({ isConnected: true, call, on }),
}))

function mountPanel() {
  const host = document.createElement('div')
  document.body.appendChild(host)
  const app = createApp(GoalPanel, { item: createGoalWorkbenchItem({ goalId: 'g-1' }) })
  app.mount(host)
  return { app, host }
}

afterEach(() => {
  call.mockReset()
  on.mockClear()
  document.body.innerHTML = ''
})

describe('GoalPanel', () => {
  it('registers a goal provider with a renderable component', () => {
    const definition = createGoalWorkbenchDefinition()
    expect(definition.kind).toBe('goal')
    expect(definition.component).toBe(GoalPanel)
    expect(definition.supports?.(createGoalWorkbenchItem())).toBe(true)
  })

  it('renders an empty state when goal.get reports no goal', async () => {
    call.mockRejectedValueOnce(Object.assign(new Error('missing'), { code: 'GOAL_NOT_FOUND' }))
    const { app, host } = mountPanel()
    await nextTick(); await nextTick()
    expect(host.textContent).toContain('No active goal is available.')
    app.unmount()
  })

  it('disables actions for a terminal goal', async () => {
    call.mockResolvedValueOnce({ id: 'g-1', objective: 'Ship', phase: 'completed', roundsStarted: 1, terminal: true })
    const { app, host } = mountPanel()
    await nextTick(); await nextTick()
    const buttons = [...host.querySelectorAll('button')]
    expect(buttons.filter(button => button.textContent?.includes('Complete')).every(button => button.disabled)).toBe(true)
    app.unmount()
  })

  it('calls goal.pause from the active action', async () => {
    call.mockResolvedValueOnce({ id: 'g-1', objective: 'Ship', phase: 'active', roundsStarted: 1, revision: 2 })
    call.mockResolvedValueOnce({ id: 'g-1', objective: 'Ship', phase: 'paused', roundsStarted: 1, revision: 3 })
    const { app, host } = mountPanel()
    await nextTick(); await nextTick()
    const pause = [...host.querySelectorAll('button')].find(button => button.textContent?.includes('Pause'))
    pause?.dispatchEvent(new MouseEvent('click', { bubbles: true }))
    await nextTick()
    expect(call).toHaveBeenLastCalledWith('goal.pause', { goalId: 'g-1', expectedRevision: 2 })
    app.unmount()
  })
})
