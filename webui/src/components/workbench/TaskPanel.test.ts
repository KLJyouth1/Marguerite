// @vitest-environment happy-dom
import { createApp, nextTick } from 'vue'
import { afterEach, describe, expect, it } from 'vitest'
import { createPinia } from 'pinia'
import i18n from '@/i18n'
import TaskPanel from './TaskPanel.vue'
import { createTaskWorkbenchDefinition } from './taskWorkbenchProvider'
import { createTaskWorkbenchItem } from '@/workbench/taskItems'

function mountPanel() {
  const host = document.createElement('div')
  document.body.appendChild(host)
  const app = createApp(TaskPanel, { item: createTaskWorkbenchItem() })
  // T05: the panel now reads the shared useTaskWorkspace store, so it needs Pinia.
  app.use(createPinia())
  app.use(i18n)
  app.mount(host)
  return { app, host }
}

afterEach(() => {
  document.body.innerHTML = ''
})

describe('TaskPanel', () => {
  it('registers a task provider with a renderable component', () => {
    const definition = createTaskWorkbenchDefinition()

    expect(definition.kind).toBe('task')
    expect(definition.component).toBe(TaskPanel)
    expect(definition.supports?.(createTaskWorkbenchItem())).toBe(true)
    expect(definition.supports?.({ ...createTaskWorkbenchItem(), kind: 'goal' })).toBe(false)
  })

  it('renders the task header and empty state from the shared store', async () => {
    const { app, host } = mountPanel()
    await nextTick()

    expect(host.textContent).toContain('Task progress')
    expect(host.textContent).toContain('No active task steps yet.')
    app.unmount()
  })
})
