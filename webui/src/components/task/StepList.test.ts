// @vitest-environment happy-dom
//
// T05 P0-4 acceptance — the task workspace renders a live task.
//
// The component test injects the very event stream the backend emits (T03/T04):
//   · `router_decision` (subtasks = 3)  → three ordered steps
//   · `step_finish`                     → the step's status changes
//   · `artifact.updated` × 2            → two deliverable cards
//   · no events                         → an explicit empty-state string
//
// Steps travel through the shared `useTaskWorkspace` store (bound to a fake RPC)
// so this exercises the real event → state → render path, not a hand-built prop.
import { createApp, defineComponent, h, nextTick } from 'vue'
import { beforeEach, describe, expect, it } from 'vitest'
import { createPinia, setActivePinia, type Pinia } from 'pinia'
import i18n from '@/i18n'
import type { RpcEventHandler } from '@/lib/rpc'
import { useTaskWorkspaceStore } from '@/composables/task/useTaskWorkspace'
import StepList from './StepList.vue'

interface FakeRpc {
  on(event: string, handler: RpcEventHandler): () => void
  emit(event: string, payload: unknown): void
}

function createFakeRpc(): FakeRpc {
  const handlers = new Map<string, Set<RpcEventHandler>>()
  return {
    on(event, handler) {
      const set = handlers.get(event) ?? new Set<RpcEventHandler>()
      set.add(handler)
      handlers.set(event, set)
      return () => set.delete(handler)
    },
    emit(event, payload) {
      for (const handler of handlers.get(event) ?? []) handler(payload)
    },
  }
}

let pinia: Pinia

function mountList() {
  const fake = createFakeRpc()
  const host = document.createElement('div')
  document.body.appendChild(host)
  const Harness = defineComponent({
    setup() {
      const store = useTaskWorkspaceStore()
      store.bindToRpc(fake)
      return () => h(StepList, { steps: store.steps, artifacts: store.artifacts })
    },
  })
  const app = createApp(Harness)
  app.use(pinia)
  app.use(i18n)
  app.mount(host)
  return { app, host, fake }
}

beforeEach(() => {
  pinia = createPinia()
  setActivePinia(pinia)
  i18n.global.locale.value = 'en'
})

describe('StepList — live task workspace', () => {
  it('renders three steps from router_decision and flips status on step_finish', async () => {
    const { app, host, fake } = mountList()
    await nextTick()

    fake.emit('router_decision', {
      strategy: 'Expert Team',
      mode_route: 'MultiCandidate',
      decision: {
        pipeline: true,
        subtasks: [
          { id: 'd1', role: 'researcher', title: 'Research', kind: 'Research', target_path: 'notes/research.md' },
          { id: 'd2', role: 'writer', title: 'Draft', kind: 'Document', target_path: 'out/draft.md' },
          { id: 'd3', role: 'designer', title: 'Slides', kind: 'Presentation', target_path: 'out/deck.pptx' },
        ],
      },
    })
    await nextTick()

    const rows = host.querySelectorAll('.step-list__row')
    expect(rows.length).toBe(3)
    expect(rows[0].getAttribute('data-step-id')).toBe('d1')
    expect(rows[2].getAttribute('data-step-id')).toBe('d3')

    // step_finish flips the first step to done and its status label updates.
    fake.emit('step_finish', {
      task_step_id: 'd1',
      index: 0,
      total: 3,
      title: 'Research',
      status: 'done',
      elapsed_ms: 1200,
    })
    await nextTick()

    const first = host.querySelector('[data-step-id="d1"]') as HTMLElement
    expect(first.classList.contains('step-list__row--done')).toBe(true)
    expect(first.querySelector('.step-list__status')?.textContent).toContain('Done')

    app.unmount()
  })

  it('renders one card per artifact.updated and an empty state when none have landed', async () => {
    const { app, host, fake } = mountList()
    await nextTick()

    // Empty-state string is present before any deliverable lands.
    const empty = host.querySelector('.step-list__empty')
    expect(empty).toBeTruthy()
    expect(empty?.textContent).toContain('No deliverables yet')

    fake.emit('router_decision', {
      decision: {
        pipeline: true,
        subtasks: [
          { id: 'd1', title: 'Research', kind: 'Research', target_path: 'a.md' },
          { id: 'd2', title: 'Draft', kind: 'Document', target_path: 'b.md' },
        ],
      },
    })
    await nextTick()
    // Steps exist but no artifact has landed yet → still the empty string.
    expect(host.querySelectorAll('.step-list__artifact').length).toBe(0)
    expect(host.querySelector('.step-list__empty')).toBeTruthy()

    fake.emit('artifact.updated', { path: 'a.md', kind: 'Research' })
    fake.emit('artifact.updated', { path: 'b.md', kind: 'Document' })
    await nextTick()

    const cards = host.querySelectorAll('.step-list__artifact')
    expect(cards.length).toBe(2)
    expect(cards[0].getAttribute('data-artifact-path')).toBe('a.md')
    expect(host.querySelector('.step-list__empty')).toBeFalsy()

    app.unmount()
  })
})
