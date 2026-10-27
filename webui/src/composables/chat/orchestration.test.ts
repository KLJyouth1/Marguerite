// @vitest-environment happy-dom
import { afterEach, describe, expect, it } from 'vitest'
import {
  createDefaultOrchestration,
  loadStoredOrchestration,
  persistOrchestration,
  shouldSendDispatchBrief,
} from './orchestration'

const KEY = 'marguerite.chat.orchestration.v1'

afterEach(() => {
  localStorage.removeItem(KEY)
})

describe('orchestration preference persistence', () => {
  it('falls back to the default state when nothing was stored', () => {
    expect(loadStoredOrchestration()).toEqual(createDefaultOrchestration())
  })

  it('restores the swarm toggle, size, roles and capabilities across mounts', () => {
    persistOrchestration({
      ...createDefaultOrchestration(),
      form: 'swarm',
      swarmOn: true,
      swarmSize: 6,
      agents: ['researcher', 'coder'],
      capabilities: ['web_search', 'sandbox'],
    })

    const restored = loadStoredOrchestration()
    expect(restored.form).toBe('swarm')
    expect(restored.swarmOn).toBe(true)
    expect(restored.swarmSize).toBe(6)
    expect(restored.agents).toEqual(['researcher', 'coder'])
    expect(restored.capabilities).toEqual(['web_search', 'sandbox'])
  })

  it('never persists one-shot confirmation', () => {
    persistOrchestration({ ...createDefaultOrchestration(), confirmed: true })
    expect(loadStoredOrchestration().confirmed).toBe(false)
  })

  it('drops unknown forms, sizes, roles and capability ids', () => {
    localStorage.setItem(KEY, JSON.stringify({
      form: 'galaxy',
      swarmOn: false,
      swarmSize: 99,
      agents: ['researcher', 'missing-role'],
      capabilities: ['web_search', 'made-up-capability'],
      teams: ['   ', 42 as unknown as string],
    }))

    const restored = loadStoredOrchestration()
    expect(restored.form).toBe('single')
    expect(restored.swarmSize).toBe(3)
    expect(restored.agents).toEqual(['researcher'])
    expect(restored.capabilities).toEqual(['web_search'])
    expect(restored.teams).toEqual([])
  })

  it('migrates the legacy single `team` value into `teams`', () => {
    localStorage.setItem(KEY, JSON.stringify({
      form: 'expert',
      team: 'deck_craft',
      swarmOn: false,
    }))

    expect(loadStoredOrchestration().teams).toEqual(['deck_craft'])
  })

  it('dedupes and caps multi-select teams', () => {
    localStorage.setItem(KEY, JSON.stringify({
      form: 'team',
      teams: ['deck_craft', 'deck_craft', 'fullstack_craft'],
      swarmOn: false,
    }))

    expect(loadStoredOrchestration().teams).toEqual(['deck_craft', 'fullstack_craft'])
  })

  it('keeps the swarm form and the swarm switch consistent', () => {
    localStorage.setItem(KEY, JSON.stringify({ form: 'swarm', swarmOn: false }))
    expect(loadStoredOrchestration().form).toBe('single')

    localStorage.setItem(KEY, JSON.stringify({ form: 'single', swarmOn: true }))
    const restored = loadStoredOrchestration()
    expect(restored.form).toBe('swarm')
    expect(restored.swarmOn).toBe(true)
  })

  it('survives malformed JSON without throwing', () => {
    localStorage.setItem(KEY, '{not json')
    expect(loadStoredOrchestration()).toEqual(createDefaultOrchestration())
  })
})

describe('dispatch brief gating', () => {
  it('全默认（single + 无任何勾选）不上行简报，保持原 chat.send 零开销路径', () => {
    expect(shouldSendDispatchBrief(createDefaultOrchestration())).toBe(false)
  })

  it('任一分工选择都必须上行简报（否则用户选了却像没选）', () => {
    const base = createDefaultOrchestration()
    expect(shouldSendDispatchBrief({ ...base, form: 'subagent', agents: ['researcher'] })).toBe(true)
    expect(shouldSendDispatchBrief({ ...base, form: 'expert', teams: ['deck_craft'] })).toBe(true)
    expect(shouldSendDispatchBrief({ ...base, form: 'team' })).toBe(true)
    expect(shouldSendDispatchBrief({ ...base, swarmOn: true })).toBe(true)
    expect(shouldSendDispatchBrief({ ...base, capabilities: ['web_search'] })).toBe(true)
  })
})
