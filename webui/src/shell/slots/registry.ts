import { ref, type Ref } from 'vue'
import { HOST_ID_NAMESPACE, type UiSlotContribution, type UiSlotId } from './types'

interface UiSlotRegistryState {
  bySlot: Map<UiSlotId, UiSlotContribution[]>
  version: Ref<number>
}

/**
 * A module-level object would be duplicated if Vite ever splits this module into
 * more than one chunk, which silently breaks registration across chunks (this
 * repo has been bitten by exactly that — see the gateway-readiness note: use a
 * STRING key, never a Symbol, and never a bare module singleton). A string key on
 * `globalThis` makes every copy of this module share ONE registry.
 */
const GLOBAL_KEY = '__margueriteUiSlotRegistry__'

function state(): UiSlotRegistryState {
  const g = globalThis as unknown as Record<string, UiSlotRegistryState | undefined>
  let s = g[GLOBAL_KEY]
  if (!s) {
    s = { bySlot: new Map(), version: ref(0) }
    g[GLOBAL_KEY] = s
  }
  return s
}

function notify(s: UiSlotRegistryState): void {
  s.version.value += 1
}

function assertRegistrable(contribution: UiSlotContribution): void {
  if (!contribution.id.trim()) {
    throw new Error('ui slot contribution requires a non-empty id')
  }
  if (contribution.id.startsWith(`${HOST_ID_NAMESPACE}.`)) {
    throw new Error(
      `ui slot contribution id "${contribution.id}" uses the reserved "${HOST_ID_NAMESPACE}." namespace`,
    )
  }
}

/**
 * Register a UI contribution. Returns a disposer.
 *
 * @throws if `id` is empty, uses the reserved `core.` namespace, or is already
 * registered for that slot (duplicates are refused, never shadowed).
 */
export function registerUiSlotContribution(contribution: UiSlotContribution): () => void {
  assertRegistrable(contribution)
  const s = state()
  const current = s.bySlot.get(contribution.slot) ?? []
  if (current.some(c => c.id === contribution.id)) {
    throw new Error(
      `ui slot "${contribution.slot}" already has a contribution with id "${contribution.id}"`,
    )
  }
  // Append-only: the host's own rows are never mutated or removed here.
  s.bySlot.set(contribution.slot, [...current, contribution])
  notify(s)

  return () => {
    const now = s.bySlot.get(contribution.slot) ?? []
    const remaining = now.filter(c => c !== contribution)
    if (remaining.length === 0) s.bySlot.delete(contribution.slot)
    else s.bySlot.set(contribution.slot, remaining)
    notify(s)
  }
}

/** Contributions for one slot, in stable render order (`order`, then `id`). */
export function slotContributions(slot: UiSlotId): readonly UiSlotContribution[] {
  const list = state().bySlot.get(slot) ?? []
  return [...list].sort((a, b) =>
    a.order !== b.order ? a.order - b.order : a.id < b.id ? -1 : a.id > b.id ? 1 : 0,
  )
}

/** Reactive version counter — read it to track registry changes in a render. */
export function uiSlotVersion(): Ref<number> {
  return state().version
}
