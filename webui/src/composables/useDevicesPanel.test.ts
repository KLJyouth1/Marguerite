import { describe, expect, it } from 'vitest'

import { hazardIcon } from '../composables/useA11y'
import {
  mapDeviceToNode,
  mapDevicesToNodes,
  normalizeHazardLevel,
  useDevicesPanel,
  type DeviceBusEntity,
} from './useDevicesPanel'

function device(overrides: Partial<DeviceBusEntity> = {}): DeviceBusEntity {
  return {
    id: 'dev-1',
    name: 'Laser rangefinder',
    hazard_level: 'L1',
    transport: 'usb',
    online: true,
    ...overrides,
  }
}

describe('normalizeHazardLevel', () => {
  it('accepts the canonical L0..L3 bands', () => {
    expect(normalizeHazardLevel('L0')).toBe('L0')
    expect(normalizeHazardLevel('L1')).toBe('L1')
    expect(normalizeHazardLevel('L2')).toBe('L2')
    expect(normalizeHazardLevel('L3')).toBe('L3')
  })

  it('is tolerant of casing and surrounding whitespace', () => {
    expect(normalizeHazardLevel(' l2 ')).toBe('L2')
    expect(normalizeHazardLevel('l3')).toBe('L3')
  })

  it('degrades unknown / missing bands to the safe L0', () => {
    expect(normalizeHazardLevel('L9')).toBe('L0')
    expect(normalizeHazardLevel('danger')).toBe('L0')
    expect(normalizeHazardLevel('')).toBe('L0')
    expect(normalizeHazardLevel(undefined)).toBe('L0')
    expect(normalizeHazardLevel(null)).toBe('L0')
  })
})

describe('mapDeviceToNode (entity → building node)', () => {
  it('passes through id, transport and online state', () => {
    const node = mapDeviceToNode(device({ id: 'dev-7', transport: 'ble', online: false }))
    expect(node.id).toBe('dev-7')
    expect(node.transport).toBe('ble')
    expect(node.online).toBe(false)
  })

  it('trims the name and falls back to id (then "device") when blank', () => {
    expect(mapDeviceToNode(device({ name: '  Wi-Fi bridge  ' })).name).toBe('Wi-Fi bridge')
    expect(mapDeviceToNode(device({ name: '', id: 'cam-3' })).name).toBe('cam-3')
    expect(mapDeviceToNode(device({ name: '   ', id: '' })).name).toBe('device')
  })

  it('coerces online to a strict boolean', () => {
    expect(mapDeviceToNode(device({ online: 1 as unknown as boolean })).online).toBe(true)
    expect(mapDeviceToNode(device({ online: 0 as unknown as boolean })).online).toBe(false)
  })

  it('trims transport and leaves an empty string when unknown', () => {
    expect(mapDeviceToNode(device({ transport: '  serial  ' })).transport).toBe('serial')
    expect(mapDeviceToNode(device({ transport: '' })).transport).toBe('')
  })
})

describe('hazard grading L0..L3', () => {
  it('resolves every band to the useA11y icon token + label pair', () => {
    for (const level of ['L0', 'L1', 'L2', 'L3'] as const) {
      const node = mapDeviceToNode(device({ hazard_level: level }))
      expect(node.hazardLevel).toBe(level)
      expect(node.icon).toBe(hazardIcon(level).icon)
      expect(node.label).toBe(hazardIcon(level).label)
    }
  })

  it('uses distinct icon tokens across the four bands', () => {
    const icons = ['L0', 'L1', 'L2', 'L3'].map(
      (level) => mapDeviceToNode(device({ hazard_level: level })).icon,
    )
    expect(new Set(icons).size).toBe(4)
  })

  it('falls back to the L0 glyph for an unrecognized band', () => {
    const node = mapDeviceToNode(device({ hazard_level: 'L9' }))
    expect(node.hazardLevel).toBe('L0')
    expect(node.icon).toBe(hazardIcon('L0').icon)
    expect(node.label).toBe(hazardIcon('L0').label)
  })
})

describe('mapDevicesToNodes (whole-bus push)', () => {
  it('maps every entity in bus order', () => {
    const nodes = mapDevicesToNodes([
      device({ id: 'a', hazard_level: 'L0' }),
      device({ id: 'b', hazard_level: 'L3' }),
    ])
    expect(nodes.map((n) => n.id)).toEqual(['a', 'b'])
    expect(nodes[1].hazardLevel).toBe('L3')
  })

  it('returns an empty list for empty / null / undefined pushes', () => {
    expect(mapDevicesToNodes([])).toEqual([])
    expect(mapDevicesToNodes(null)).toEqual([])
    expect(mapDevicesToNodes(undefined)).toEqual([])
  })
})

describe('useDevicesPanel (reactive shell)', () => {
  it('starts empty and reports empty=true', () => {
    const panel = useDevicesPanel()
    expect(panel.nodes.value).toEqual([])
    expect(panel.empty.value).toBe(true)
    expect(panel.onlineCount.value).toBe(0)
    expect(panel.offlineCount.value).toBe(0)
    expect(panel.maxHazard.value).toBe('L0')
  })

  it('reconciles the list on every setDevices push', () => {
    const panel = useDevicesPanel()
    panel.setDevices([
      device({ id: 'a', online: true, hazard_level: 'L1' }),
      device({ id: 'b', online: false, hazard_level: 'L3' }),
    ])
    expect(panel.nodes.value.map((n) => n.id)).toEqual(['a', 'b'])
    expect(panel.onlineCount.value).toBe(1)
    expect(panel.offlineCount.value).toBe(1)
    expect(panel.empty.value).toBe(false)
  })

  it('derives the worst hazard band across all nodes', () => {
    const panel = useDevicesPanel()
    panel.setDevices([
      device({ id: 'a', hazard_level: 'L1' }),
      device({ id: 'b', hazard_level: 'L0' }),
      device({ id: 'c', hazard_level: 'L2' }),
    ])
    expect(panel.maxHazard.value).toBe('L2')

    panel.setDevices([device({ id: 'a', hazard_level: 'L3' })])
    expect(panel.maxHazard.value).toBe('L3')
  })

  it('resets to empty when the bus pushes an empty list', () => {
    const panel = useDevicesPanel()
    panel.setDevices([device({ id: 'a' })])
    expect(panel.empty.value).toBe(false)
    panel.setDevices([])
    expect(panel.nodes.value).toEqual([])
    expect(panel.empty.value).toBe(true)
    expect(panel.maxHazard.value).toBe('L0')
  })
})
