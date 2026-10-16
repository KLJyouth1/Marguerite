/**
 * WP6 device-bus panel state layer (pure, DOM-free).
 *
 * The device bus pushes a flat list of physical devices; this composable is the
 * single translation step that turns those raw RPC entities into the normalized
 * "device building" nodes the island workspace renders.
 *
 * Design notes (mirrors islandWorld.ts / useA11y.ts conventions):
 * - **Honest visualisation**: a device only becomes a node when the bus actually
 *   reports it. Empty bus → empty node list (we never fabricate buildings).
 * - **No colour-only status**: every hazard band resolves through `hazardIcon`
 *   from `useA11y`, so each node carries an icon token AND a text label.
 * - **Framework-free core**: the `map*` functions are pure over plain data so
 *   the whole surface is unit-testable under the node environment (no mount).
 *   The `useDevicesPanel()` wrapper only adds the reactive shell around them.
 */

import { computed, ref, type ComputedRef, type Ref } from 'vue'

import { hazardIcon, type HazardLevel } from './useA11y'

/**
 * Raw device-bus entity as pushed by the devices RPC. Field names follow the
 * wire contract (snake_case); this layer normalizes them into the node shape
 * the island consumes.
 */
export interface DeviceBusEntity {
  /** Stable device id assigned by the bus (used as the island node id). */
  id: string
  /** Human-readable device name. */
  name: string
  /** Hazard band as a string: 'L0'..'L3' (case/whitespace tolerant). */
  hazard_level: string
  /** Transport medium label, e.g. 'usb' | 'ble' | 'ethernet' | 'serial'. */
  transport: string
  /** Whether the bus reports the device online right now. */
  online: boolean
}

/**
 * Normalized "device building" node for the island workspace. Language-neutral
 * raw values only; the 2D card / 3D mesh decide how to present them.
 */
export interface DeviceNode {
  /** = bus entity id (island uses it as the diff key). */
  id: string
  /** Display name (fallback to id when empty). */
  name: string
  /** Normalized hazard band L0..L3. */
  hazardLevel: HazardLevel
  /** Transport medium (trimmed; empty string when unknown). */
  transport: string
  /** Current online state. */
  online: boolean
  /** Icon token resolved via `hazardIcon` (the renderer picks the glyph). */
  icon: string
  /** Text label paired with the icon — never convey hazard by colour alone. */
  label: string
}

const HAZARD_BANDS: readonly HazardLevel[] = ['L0', 'L1', 'L2', 'L3']

/**
 * Normalize a wire hazard string into a HazardLevel. Tolerant of casing and
 * surrounding whitespace; anything unrecognized degrades to the safe L0 band
 * (matches `hazardIcon`'s own fallback policy — never throw on bad input).
 */
export function normalizeHazardLevel(raw: string | undefined | null): HazardLevel {
  const band = (raw ?? '').trim().toUpperCase()
  return (HAZARD_BANDS as readonly string[]).includes(band) ? (band as HazardLevel) : 'L0'
}

/** Map one raw bus entity to a normalized island node. */
export function mapDeviceToNode(entity: DeviceBusEntity): DeviceNode {
  const hazardLevel = normalizeHazardLevel(entity.hazard_level)
  const glyph = hazardIcon(hazardLevel)
  const name = (entity.name || '').trim() || entity.id || 'device'
  return {
    id: entity.id,
    name,
    hazardLevel,
    transport: (entity.transport || '').trim(),
    online: Boolean(entity.online),
    icon: glyph.icon,
    label: glyph.label,
  }
}

/** Map a whole bus push to the node list (null/undefined → empty list). */
export function mapDevicesToNodes(entities: readonly DeviceBusEntity[] | null | undefined): DeviceNode[] {
  return (entities ?? []).map(mapDeviceToNode)
}

/** Hazard rank used to derive the worst band currently on the island. */
const HAZARD_RANK: Record<HazardLevel, number> = { L0: 0, L1: 1, L2: 2, L3: 3 }

export interface DevicesPanelState {
  /** Current device building nodes, in bus order. */
  nodes: Ref<DeviceNode[]>
  /** How many nodes the bus currently reports online. */
  onlineCount: ComputedRef<number>
  /** How many nodes the bus currently reports offline. */
  offlineCount: ComputedRef<number>
  /** Worst hazard band present across all nodes (L0 when empty). */
  maxHazard: ComputedRef<HazardLevel>
  /** True when the bus has reported no devices (island shows empty state). */
  empty: ComputedRef<boolean>
  /** Replace the whole node list from a devices RPC push. */
  setDevices: (entities: readonly DeviceBusEntity[]) => void
}

/**
 * Reactive shell around the pure mappers. Call once where the island panel owns
 * the device-bus subscription; `setDevices` is the single entry point for every
 * RPC push (full-list reconciliation, matching how the bus currently emits).
 */
export function useDevicesPanel(): DevicesPanelState {
  const entities = ref<readonly DeviceBusEntity[]>([])

  const nodes = computed<DeviceNode[]>(() => mapDevicesToNodes(entities.value))
  const onlineCount = computed<number>(() => nodes.value.filter((n) => n.online).length)
  const offlineCount = computed<number>(() => nodes.value.length - onlineCount.value)
  const maxHazard = computed<HazardLevel>(() =>
    nodes.value.reduce<HazardLevel>(
      (worst, node) => (HAZARD_RANK[node.hazardLevel] > HAZARD_RANK[worst] ? node.hazardLevel : worst),
      'L0',
    ),
  )
  const empty = computed<boolean>(() => nodes.value.length === 0)

  function setDevices(next: readonly DeviceBusEntity[]): void {
    entities.value = [...(next ?? [])]
  }

  return { nodes, onlineCount, offlineCount, maxHazard, empty, setDevices }
}
