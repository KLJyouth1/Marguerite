/**
 * Registers the Provenance Drawer into the shell's `shell.overlay` slot.
 *
 * Same reasoning as `registerAttentionLayer.ts`: the slot registry is the
 * console's documented append-only seam, so the drawer stays a contribution
 * (removable, testable in isolation) rather than another permanent line in the
 * 96 KB `App.vue`. `App.vue` is off-limits for this change, which makes the seam
 * the only honest way to install a new shell surface.
 *
 * Idempotency matters here. Vite HMR re-evaluates `main.ts`, and
 * `registerUiSlotContribution` throws on a duplicate id by design (silent
 * shadowing is how "installed but inert" starts). The flag lives on
 * `globalThis` — the same reason `registry.ts` keys its state there — so every
 * copy of this module in every chunk shares one answer.
 */

import { registerUiSlotContribution } from '@/shell/slots/registry'

import ProvenanceDrawer from './ProvenanceDrawer.vue'

const INSTALLED_KEY = '__margueriteProvenanceDrawerInstalled__'

export const PROVENANCE_SLOT_ID = 'provenance.drawer'

export function installProvenanceDrawer(): void {
  const g = globalThis as unknown as Record<string, unknown>
  if (g[INSTALLED_KEY]) return
  g[INSTALLED_KEY] = true
  // No try/catch: a genuine registration failure (bad id, wrong namespace) must
  // surface loudly at boot, not be swallowed into a silently missing surface.
  registerUiSlotContribution({
    id: PROVENANCE_SLOT_ID,
    slot: 'shell.overlay',
    order: 110,
    component: ProvenanceDrawer,
    labelKey: 'shell.provenance.label',
  })
}
