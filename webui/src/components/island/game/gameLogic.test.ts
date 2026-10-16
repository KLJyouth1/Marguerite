import { describe, it, expect } from 'vitest'
import {
  advanceQuest,
  completeQuest,
  dayPhaseOf,
  defaultSave,
  deriveWeather,
  expForLevel,
  isNightTime,
  levelFromExp,
  partyCap,
  questNextStatus,
  TITLES,
  type EngineSnapshot,
} from './gameLogic'

const offline: EngineSnapshot = {
  connected: false,
  modeKind: null,
  reasoningEffort: null,
  maxSwarmShards: null,
  hasTierRpc: false,
  hasBudgetRpc: false,
  hasGameRpc: false,
}

describe('quest state machine', () => {
  it('walks posted -> accepted -> in_progress -> verifying -> completed', () => {
    expect(questNextStatus('posted')).toBe('accepted')
    expect(questNextStatus('accepted')).toBe('in_progress')
    expect(questNextStatus('in_progress')).toBe('verifying')
    expect(questNextStatus('verifying')).toBe('completed')
    expect(questNextStatus('completed')).toBeNull()
  })

  it('advanceQuest mutates status forward', () => {
    let s = defaultSave()
    const id = s.quests[0].id
    s = advanceQuest(s, id)
    expect(s.quests[0].status).toBe('accepted')
    s = advanceQuest(s, id)
    expect(s.quests[0].status).toBe('in_progress')
  })
})

describe('EXP / levels', () => {
  it('levelFromExp starts at 1 and grows', () => {
    expect(levelFromExp(0)).toBe(1)
    expect(levelFromExp(expForLevel(1))).toBe(2)
    expect(levelFromExp(expForLevel(1) + expForLevel(2))).toBe(3)
  })
})

describe('completeQuest', () => {
  it('awards EXP with difficulty multiplier and grants reward title', () => {
    let s = defaultSave()
    const hard = s.quests.find((q) => q.rewardTitle === 'nightOwl')!
    const res = completeQuest(s, hard.id)
    expect(res.gainedExp).toBe(hard.expReward * 2) // difficulty 2
    expect(res.save.quests.find((q) => q.id === hard.id)!.status).toBe('completed')
    expect(res.unlockedTitles).toContain('nightOwl')
    expect(res.save.titles).toContain('nightOwl')
  })

  it('is immutable and idempotent on completed quests', () => {
    let s = defaultSave()
    const id = s.quests[0].id
    s = completeQuest(s, id).save
    const again = completeQuest(s, id)
    expect(again.gainedExp).toBe(0)
    expect(again.save).toBe(s)
  })

  it('ships at least 4 titles', () => {
    expect(TITLES.length).toBeGreaterThanOrEqual(4)
  })
})

describe('day/night follows real system clock', () => {
  it('23:00 and 06:00 are night; 12:00 is day', () => {
    expect(isNightTime(new Date(2026, 9, 16, 23, 0))).toBe(true)
    expect(isNightTime(new Date(2026, 9, 16, 6, 0))).toBe(true)
    expect(dayPhaseOf(new Date(2026, 9, 16, 12, 0))).toBe('day')
  })
})

describe('deriveWeather (honest: no telemetry fabrication)', () => {
  it('returns clear when gateway offline (does not fake storms)', () => {
    expect(deriveWeather(offline, new Date(2026, 9, 16, 12, 0))).toBe('clear')
  })
  it('night when connected and at night, else clear', () => {
    const online: EngineSnapshot = { ...offline, connected: true }
    expect(deriveWeather(online, new Date(2026, 9, 16, 23, 0))).toBe('night')
    expect(deriveWeather(online, new Date(2026, 9, 16, 12, 0))).toBe('clear')
  })
})

describe('partyCap exposes real max_swarm_shards', () => {
  it('falls back to 4 when unknown', () => {
    expect(partyCap(offline)).toBe(4)
  })
  it('clamps to 4 UI slots but honors larger real budgets', () => {
    expect(partyCap({ ...offline, maxSwarmShards: 11 })).toBe(4)
  })
})
