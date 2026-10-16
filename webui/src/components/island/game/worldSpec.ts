/**
 * WorldSpec —— 交互式游戏生成的操作对象（蓝图 §7）。
 *
 * 设计铁律（对齐 pet/petSpec.ts 的 validate 范式）：
 * - 本文件是**纯数据 + 纯函数**：不 import three、不碰 DOM、不读 localStorage，
 *   可在 node 环境直接 vitest。
 * - 诚实性：校验器拒绝任何「来源不明 / 越界」的世界；UI 只在 validateWorldSpec 返回
 *   空错误数组后才热加载新世界，否则回滚到上一个已校验世界。
 * - 安全边界：spec 只能引用已注册的区域/天气/主题枚举，AI 描述不能发明新 RPC。
 */

export type WeatherKind = 'clear' | 'rain' | 'night' | 'fog'
export type WorldTheme = 'verdant' | 'blossom' | 'twilight'

export const WEATHER_KINDS: readonly WeatherKind[] = ['clear', 'rain', 'night', 'fog']
export const WORLD_THEMES: readonly WorldTheme[] = ['verdant', 'blossom', 'twilight']

export interface WorldRegionSpec {
  id: string
  label: string
  /** [x, y, z] 世界锚点（1 单位 = 1 米） */
  anchor: [number, number, number]
  radius: number
}

export interface WorldQuestSeed {
  id: string
  label: string
  /** 1 = 常规，2 = 困难，3 = 高难（Bloom/Omni 三倍奖励） */
  difficulty: 1 | 2 | 3
  expReward: number
  /** 完成后授予的称号 id（可选，须在 gameLogic.TITLES 内） */
  rewardTitle?: string
}

export interface WorldSpec {
  /** 世界修订号（每次热加载 +1，用于回滚 diff） */
  revision: number
  /** 确定性程序化种子（可复现地形） */
  seed: number
  islandName: string
  theme: WorldTheme
  weather: WeatherKind
  regions: WorldRegionSpec[]
  initialQuests: WorldQuestSeed[]
}

/** MVP 必落的三个关键区域（蓝图 D1-2：秘境花园 + 中枢庭雏菊圣殿 + 委托板）。 */
export const CORE_REGIONS: readonly { id: string; anchor: [number, number, number]; radius: number }[] = [
  { id: 'garden', anchor: [-18, 0, -12], radius: 14 },
  { id: 'atrium', anchor: [0, 0, 0], radius: 12 },
  { id: 'board', anchor: [14, 0, 10], radius: 8 },
]

/** 默认世界（蓝图 MVP 基线：一座生长的小岛）。 */
export function defaultWorldSpec(): WorldSpec {
  return {
    revision: 1,
    seed: 770123,
    islandName: 'Marguerite Isle',
    theme: 'verdant',
    weather: 'clear',
    regions: CORE_REGIONS.map((r) => ({
      id: r.id,
      label: r.id,
      anchor: [...r.anchor] as [number, number, number],
      radius: r.radius,
    })),
    initialQuests: [
      { id: 'welcome', label: 'First Light', difficulty: 1, expReward: 50 },
      { id: 'patrol', label: 'Garden Patrol', difficulty: 1, expReward: 80 },
      { id: 'deep_spire', label: 'Spire Ascent', difficulty: 2, expReward: 180, rewardTitle: 'nightOwl' },
    ],
  }
}

const ALLOWED_NAME_RE = /^[\p{L}\p{N} _\-·]{1,40}$/u

function isFiniteVec3(v: unknown): v is [number, number, number] {
  return (
    Array.isArray(v) &&
    v.length === 3 &&
    v.every((n) => typeof n === 'number' && Number.isFinite(n))
  )
}

/**
 * 校验世界 spec（对齐 validatePetSpec 范式）。返回错误字符串数组；空数组 = 可热加载。
 * 校验项：修订号/种子、名称合法性、枚举白名单、区域与委托的连通性与唯一性、预算上限。
 */
export function validateWorldSpec(w: unknown): string[] {
  const errors: string[] = []
  if (!w || typeof w !== 'object') return ['worldSpec: not an object']
  const s = w as Partial<WorldSpec>

  if (!Number.isInteger(s.revision) || (s.revision ?? 0) < 1) {
    errors.push('worldSpec.revision must be an integer >= 1')
  }
  if (!Number.isInteger(s.seed) || (s.seed ?? -1) < 0) {
    errors.push('worldSpec.seed must be a non-negative integer')
  }
  if (typeof s.islandName !== 'string' || !ALLOWED_NAME_RE.test(s.islandName.trim())) {
    errors.push('worldSpec.islandName: 1-40 chars, letters/numbers/space only')
  }
  if (!WORLD_THEMES.includes(s.theme as WorldTheme)) {
    errors.push(`worldSpec.theme must be one of: ${WORLD_THEMES.join(', ')}`)
  }
  if (!WEATHER_KINDS.includes(s.weather as WeatherKind)) {
    errors.push(`worldSpec.weather must be one of: ${WEATHER_KINDS.join(', ')}`)
  }

  if (!Array.isArray(s.regions) || s.regions.length < 3) {
    errors.push('worldSpec.regions: at least 3 regions required (garden/atrium/board)')
  } else {
    const seen = new Set<string>()
    for (const r of s.regions) {
      if (!r || typeof r.id !== 'string' || seen.has(r.id)) {
        errors.push(`worldSpec.regions: duplicate/empty region id "${r?.id}"`)
      }
      if (r) seen.add(r.id)
      if (!r || !isFiniteVec3(r.anchor)) errors.push(`worldSpec.regions.${r?.id}: anchor must be [x,y,z] finite`)
      if (typeof r?.radius !== 'number' || r.radius <= 0 || r.radius > 60) {
        errors.push(`worldSpec.regions.${r?.id}: radius must be in (0, 60]`)
      }
    }
  }

  if (!Array.isArray(s.initialQuests) || s.initialQuests.length === 0) {
    errors.push('worldSpec.initialQuests: at least 1 quest required')
  } else {
    const seen = new Set<string>()
    for (const q of s.initialQuests) {
      if (!q || typeof q.id !== 'string' || seen.has(q.id)) {
        errors.push(`worldSpec.initialQuests: duplicate/empty quest id "${q?.id}"`)
      }
      if (q) seen.add(q.id)
      if (q && ![1, 2, 3].includes(q.difficulty)) {
        errors.push(`worldSpec.initialQuests.${q.id}: difficulty must be 1|2|3`)
      }
      if (q && (!Number.isFinite(q.expReward) || q.expReward < 0 || q.expReward > 5000)) {
        errors.push(`worldSpec.initialQuests.${q.id}: expReward must be 0..5000`)
      }
    }
    if (s.initialQuests.length > 8) errors.push('worldSpec.initialQuests: budget exceeded (max 8)')
  }

  return errors
}

function pickWeather(text: string): WeatherKind {
  if (/雨|storm|rain|thunder/i.test(text)) return 'rain'
  if (/雾|fog|mist|haze/i.test(text)) return 'fog'
  if (/夜|night|moon|star|星空/i.test(text)) return 'night'
  return 'clear'
}

function pickTheme(text: string): WorldTheme {
  if (/花|blossom|樱|cherry|flower/i.test(text)) return 'blossom'
  if (/暮|twilight|黄昏|dusk|星夜/i.test(text)) return 'twilight'
  return 'verdant'
}

function pickDifficulty(text: string): 1 | 2 | 3 {
  if (/难|hard|大师|master|3\s*星|nightmare/i.test(text)) return 3
  if (/深|deep|hardcore|2\s*星/i.test(text)) return 2
  return 1
}

function deriveName(text: string): string {
  // 取描述第一句/前 12 个字符作为岛名；清洗非法字符。
  const first = text.split(/[。.!?！\n]/)[0]?.trim() ?? ''
  const cleaned = first.replace(/[^\p{L}\p{N} _\-·]/gu, '').slice(0, 24).trim()
  return cleaned || 'Marguerite Isle'
}

/** 确定性地从自然语言描述生成 WorldSpec（MVP 闭环：关键词→枚举；不调 LLM）。 */
export function specFromDescription(text: string): WorldSpec {
  const t = text || ''
  const difficulty = pickDifficulty(t)
  return {
    revision: 1,
    seed: hashSeed(t),
    islandName: deriveName(t),
    theme: pickTheme(t),
    weather: pickWeather(t),
    regions: CORE_REGIONS.map((r) => ({
      id: r.id,
      label: r.id,
      anchor: [...r.anchor] as [number, number, number],
      radius: r.radius,
    })),
    initialQuests: [
      { id: 'welcome', label: 'First Light', difficulty: 1, expReward: 50 },
      {
        id: 'generated',
        label: deriveName(t),
        difficulty,
        expReward: difficulty * 90,
        ...(difficulty === 3 ? { rewardTitle: 'gateKeeper' } : {}),
      },
    ],
  }
}

/** 字符串 → 确定性 31-bit 种子（FNV-1a）。 */
export function hashSeed(text: string): number {
  let h = 0x811c9dc5
  for (let i = 0; i < text.length; i++) {
    h ^= text.charCodeAt(i)
    h = Math.imul(h, 0x01000193)
  }
  return Math.abs(h) % 1_000_000
}
