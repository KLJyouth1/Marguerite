/**
 * 游戏规则纯函数（委托状态机 / EXP 升级 / 称号 / 编队 / 天气昼夜推导 / 存档）。
 *
 * 诚实性红线：
 * - 本文件**不直接读 RPC**。它只消费上层 `useEngineBridge` 归一后的 EngineSnapshot，
 *   且对「引擎未连接 / 字段缺失」做降级——不编造引擎数值。
 * - 存档（localStorage）是合法的客户端真实存档（委托/EXP/称号/编队/采集）；
 *   引擎状态本身永不落档为「当前值」。
 */

import type { WeatherKind } from './worldSpec'

export type QuestStatus =
  | 'posted'
  | 'accepted'
  | 'in_progress'
  | 'verifying'
  | 'completed'
  | 'failed'

export interface Quest {
  id: string
  label: string
  difficulty: 1 | 2 | 3
  expReward: number
  status: QuestStatus
  rewardTitle?: string
}

export interface TitleDef {
  id: string
  /** i18n key 后缀（islandGame.title.<id>） */
  labelKey: string
}

/** 至少 4 个称号（蓝图 D5-6）。 */
export const TITLES: readonly TitleDef[] = [
  { id: 'zeroWarn', labelKey: 'zeroWarn' },
  { id: 'gateKeeper', labelKey: 'gateKeeper' },
  { id: 'nightOwl', labelKey: 'nightOwl' },
  { id: 'thrifty', labelKey: 'thrifty' },
]

export type PartyClass = 'scout' | 'smith' | 'aegis' | 'gull'
export const PARTY_CLASSES: readonly PartyClass[] = ['scout', 'smith', 'aegis', 'gull']

export interface PartyElf {
  slot: number
  cls: PartyClass
}

export interface EngineSnapshot {
  connected: boolean
  /** plan / agent / ensemble / bloom / omni-transcend（归一化连字符） */
  modeKind: string | null
  reasoningEffort: string | null
  /** 来自 mode.spec 的编队上限（Bloom 11 / Omni 13）；未知为 null */
  maxSwarmShards: number | null
  /** core 分片正在新增的 RPC；方法不存在时保持 false */
  hasTierRpc: boolean
  hasBudgetRpc: boolean
  hasGameRpc: boolean
}

export interface GameSave {
  exp: number
  level: number
  titles: string[]
  quests: Quest[]
  party: (PartyElf | null)[]
  collected: { code: number; reason: number; craft: number }
  revision: number
}

/** 状态机：可接 → 进行中 → 可交付 → 完成。返回 null 表示终态/非法迁移。 */
export function questNextStatus(s: QuestStatus): QuestStatus | null {
  switch (s) {
    case 'posted':
      return 'accepted'
    case 'accepted':
      return 'in_progress'
    case 'in_progress':
      return 'verifying'
    case 'verifying':
      return 'completed'
    default:
      return null
  }
}

/** 升到下一级所需累计 EXP（曲线平缓，桌面常驻不肝）。 */
export function expForLevel(level: number): number {
  return 100 + (level - 1) * 60
}

export function levelFromExp(exp: number): number {
  let level = 1
  let rest = exp
  while (rest >= expForLevel(level)) {
    rest -= expForLevel(level)
    level += 1
  }
  return level
}

export function defaultSave(): GameSave {
  return {
    exp: 0,
    level: 1,
    titles: [],
    quests: [
      { id: 'welcome', label: 'First Light', difficulty: 1, expReward: 50, status: 'posted' },
      { id: 'patrol', label: 'Garden Patrol', difficulty: 1, expReward: 80, status: 'posted' },
      { id: 'deep_spire', label: 'Spire Ascent', difficulty: 2, expReward: 180, status: 'posted', rewardTitle: 'nightOwl' },
    ],
    party: [null, null, null, null],
    collected: { code: 0, reason: 0, craft: 0 },
    revision: 1,
  }
}

export interface QuestCompletion {
  save: GameSave
  gainedExp: number
  unlockedTitles: string[]
  leveledUp: boolean
}

/** 完成一张委托：发 EXP、结算称号、采集结晶。返回新存档（不可变更新）。 */
export function completeQuest(save: GameSave, questId: string): QuestCompletion {
  const quest = save.quests.find((q) => q.id === questId)
  if (!quest || quest.status === 'completed') {
    return { save, gainedExp: 0, unlockedTitles: [], leveledUp: false }
  }
  const multiplier = quest.difficulty >= 3 ? 3 : quest.difficulty === 2 ? 2 : 1
  const gainedExp = quest.expReward * multiplier
  const beforeLevel = levelFromExp(save.exp)
  const newExp = save.exp + gainedExp
  const afterLevel = levelFromExp(newExp)

  const unlockedTitles: string[] = []
  const titles = new Set(save.titles)
  if (quest.rewardTitle && TITLES.some((t) => t.id === quest.rewardTitle) && !titles.has(quest.rewardTitle)) {
    titles.add(quest.rewardTitle)
    unlockedTitles.push(quest.rewardTitle)
  }

  const collected = { ...save.collected }
  if (quest.difficulty === 1) collected.code += 1
  else if (quest.difficulty === 2) collected.reason += 1
  else collected.craft += 1

  const next: GameSave = {
    ...save,
    exp: newExp,
    level: afterLevel,
    titles: [...titles],
    collected,
    quests: save.quests.map((q) => (q.id === questId ? { ...q, status: 'completed' as QuestStatus } : q)),
    revision: save.revision + 1,
  }
  return { save: next, gainedExp, unlockedTitles, leveledUp: afterLevel > beforeLevel }
}

/** 推进一张委托一个状态；返回新存档（无操作时原样返回）。 */
export function advanceQuest(save: GameSave, questId: string): GameSave {
  const quest = save.quests.find((q) => q.id === questId)
  if (!quest) return save
  const next = questNextStatus(quest.status)
  if (!next) return save
  return {
    ...save,
    revision: save.revision + 1,
    quests: save.quests.map((q) => (q.id === questId ? { ...q, status: next } : q)),
  }
}

/** 昼夜跟随系统真实时间：21:00–07:00 视为夜间。 */
export function isNightTime(d: Date = new Date()): boolean {
  const h = d.getHours()
  return h >= 21 || h < 7
}

export type DayPhase = 'day' | 'night'

export function dayPhaseOf(d: Date = new Date()): DayPhase {
  return isNightTime(d) ? 'night' : 'day'
}

/**
 * 由真实引擎快照推导天气（蓝图 §2.1：天气=引擎状态的拟物化）。
 * - 引擎未连接 → 不假装恶劣天气，回退 clear（昼夜仍跟系统时间）。
 * - CPU/并发过载 或 连续错误 → rain（雷雨/酸雨的 MVP 近似）。
 * - 夜间且无其它信号 → night。
 * - 长任务上下文压缩 → fog（这里以 hasGameRpc 不可知时不臆断）。
 */
export function deriveWeather(engine: EngineSnapshot, now: Date = new Date()): WeatherKind {
  if (!engine.connected) return 'clear'
  // 未来 core 透传 metrics 时，这里读取真实阈值；当前 MVP 不臆造数值。
  // 预留扩展点：engine 若带 telemetry 字段，在此映射。
  return isNightTime(now) ? 'night' : 'clear'
}

/** 编队槽位上限：真实 RPC 透出 max_swarm_shards；未知时给安全默认 4（MVP 槽位数）。 */
export function partyCap(engine: EngineSnapshot): number {
  if (typeof engine.maxSwarmShards === 'number' && engine.maxSwarmShards > 0) {
    // 编队 UI 只露 4 个精灵位（蓝图 D7）；真实上限用于「符文点灯」展示，不撑爆画面。
    return Math.min(4, engine.maxSwarmShards)
  }
  return 4
}

const STORAGE_KEY = 'marguerite.islandGame.save.v1'

/** 从 localStorage 读取存档（损坏/缺失 → null，调用方回退 defaultSave）。 */
export function loadSave(): GameSave | null {
  if (typeof localStorage === 'undefined') return null
  try {
    const raw = localStorage.getItem(STORAGE_KEY)
    if (!raw) return null
    const parsed = JSON.parse(raw) as GameSave
    if (!parsed || typeof parsed.exp !== 'number' || !Array.isArray(parsed.quests)) return null
    return parsed
  } catch {
    return null
  }
}

export function persistSave(save: GameSave): void {
  if (typeof localStorage === 'undefined') return
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(save))
  } catch {
    /* 存储配额/隐私模式下静默失败，不阻塞游戏 */
  }
}

export function clearSave(): void {
  if (typeof localStorage === 'undefined') return
  try {
    localStorage.removeItem(STORAGE_KEY)
  } catch {
    /* noop */
  }
}
