import type { Component } from 'vue'

/**
 * UI slot contract — the append-only extension seam for the console shell.
 *
 * Design rules (mirrors the reviewed DSH-desktop slot discipline):
 *
 * - **Append-only.** A contribution can only ADD to a slot. Nothing may replace
 *   or hide a host-rendered row — otherwise a plugin could silently delete
 *   official UI, which is unreviewable by design.
 * - **Ordered.** Every contribution declares a stable `order`; equal orders fall
 *   back to `id`, so rendering is deterministic and never depends on Map order.
 * - **Identified.** `id` is unique within its slot. Registering a duplicate id
 *   throws — silent shadowing is exactly how "installed but inert" starts.
 * - **Namespaced.** Ids are `<namespace>.<name>`; the `core` namespace is
 *   reserved for the host, so third-party ids cannot impersonate it.
 *
 * This is the FRONTEND seam only. It deliberately does not reuse the backend's
 * `PluginSlot` (core/src/plugins/slot.rs), which models tool/skill/MCP capability
 * slots rather than UI surfaces; the two only agree on the id namespace idea.
 */
export type UiSlotId = 'sidebar.nav' | 'sidebar.footer' | 'topbar.actions' | 'shell.overlay'

/** Reserved id namespace — owned by the host. */
export const HOST_ID_NAMESPACE = 'core'

export interface UiSlotContribution {
  /** Unique within its slot: `<namespace>.<name>`. Must not start with `core.`. */
  id: string
  slot: UiSlotId
  /** Lower renders first. Ties break on `id` (stable, deterministic). */
  order: number
  /** Rendered component. */
  component: Component
  /** Optional i18n key for the contribution's accessible label / tooltip. */
  labelKey?: string
}
