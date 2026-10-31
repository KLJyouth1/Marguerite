/**
 * Registers the Attention Shelf into the shell's `shell.overlay` slot.
 *
 * Why a registration module instead of a plain `<AttentionLayer />` tag in
 * `App.vue`: the slot registry is the console's documented append-only seam, so
 * the shelf stays a contribution (removable, testable in isolation) rather than
 * another permanent line in a 96 KB single-file component.
 *
 * Idempotency matters here. Vite HMR re-evaluates `main.ts`, and
 * `registerUiSlotContribution` throws on a duplicate id by design (silent
 * shadowing is how "installed but inert" starts). The flag lives on
 * `globalThis` — the same reason `registry.ts` keys its state there — so every
 * copy of this module in every chunk shares one answer.
 */

import { registerUiSlotContribution } from '@/shell/slots/registry'

import AttentionLayer from './AttentionLayer.vue'

const INSTALLED_KEY = '__margueriteAttentionLayerInstalled__'

export const ATTENTION_SLOT_ID = 'attention.shelf'

export function installAttentionLayer(): void {
  const g = globalThis as unknown as Record<string, unknown>
  if (g[INSTALLED_KEY]) return
  g[INSTALLED_KEY] = true
  // No try/catch: a genuine registration failure (bad id, wrong namespace) must
  // surface loudly at boot, not be swallowed into a silently missing surface.
  registerUiSlotContribution({
    id: ATTENTION_SLOT_ID,
    slot: 'shell.overlay',
    order: 100,
    component: AttentionLayer,
    labelKey: 'attention.label',
  })
}
