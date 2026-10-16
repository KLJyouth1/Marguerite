/**
 * GDD（Game Design Document）机器可读契约 + 手写校验器（纯函数，可 vitest）。
 *
 * ── 为什么产物必须是「带 Schema 的结构化文件」而非自由文本 ────────────────
 * 设计依据 `_island_3d_game_design.md` §7「AI 自主设计闭环」：GCA 论文验证的
 * 路径是「GDD → 契约 → 确定性测试 → 多智能体并行生产」。自由文本（散文 GDD、
 * markdown 段落）无法被程序消费：字段名漂移、数值边界缺失、枚举外溢——多智能体
 * 各读各的解读，产物拼不上。结构化文件的价值：
 *   1. **可拦截**：validateGdd 在落盘/热加载入口把关，非法产物被自动化测试拦下，
 *      而不是等到运行时渲染崩溃才暴露（确定性测试挂在契约上而非渲染上）；
 *   2. **可回灌**：exportCurrent() 导出的仍是同构 JSON，AI 下一轮改进的输入输出
 *      同型，闭环可迭代（§7.2 自改进流程）；
 *   3. **可版本化**：semver + revision 递增，运行时按版本热加载，失败回滚旧配置。
 * 因此本文件的每个字段都是**契约**：枚举收口、数值边界、id 唯一——全部可断言。
 *
 * 校验器手写（不用 zod/ajv）：与 propsSpec.ts 同风格的纯函数依赖，零新依赖，
 * 且错误明细结构（level/code/path）本身就是给 UI 与测试消费的契约。
 */

import type { PropGeometry, PropInteraction, PropKind } from '../propsSpec'
import { PROP_KINDS } from '../propsSpec'

// ───────────────────────── 契约类型 ─────────────────────────

/** geometry.main 允许的 primitive 枚举（与 propsSpec.PropGeometry 收口一致） */
export const PROP_GEOMETRY_MAINS: readonly PropGeometry['main'][] = ['octahedron', 'cone', 'sphere', 'tower', 'flower']
/** geometry.accent 允许的附加 primitive 枚举 */
export const PROP_GEOMETRY_ACCENTS: readonly NonNullable<PropGeometry['accent']>[] = ['prism', 'scroll', 'berry', 'lamp', 'petal']

/** GDD 中的道具契约（PropSpec 的可配置子集，AI 可改几何/数值，不可改语义锚点） */
export interface GddPropSpec {
  kind: PropKind
  /** 拾取交互语义（枚举收口，防止 AI 造出运行时不认识的交互） */
  interaction: PropInteraction
  geometry: PropGeometry
  /** 同屏道具上限：1..64（防刷屏的内存闸门，64 是粒子预算之外的硬顶） */
  maxInstances: number
  /** i18n key 后缀（islandGame.world3d.prop.<kind>.*） */
  labelKey: string
  /** 拾取奖励 EXP 基数（必须为正） */
  expReward: number
}

/** 任务契约：id / 标题 / 触发条件 / 奖励区域 */
export interface GddQuest {
  id: string
  title: string
  /** 触发条件（确定性表达式字符串，由任务状态机消费；契约只保证非空） */
  trigger: string
  /** 完成任务后解锁的区域 id（必须存在于 regions，或留空字符串表示无解锁） */
  rewardRegion: string
}

/** 可解锁区域契约 */
export interface GddRegion {
  id: string
  name: string
  /** 初始是否处于迷雾中（true=需要解锁后可见） */
  locked: boolean
}

/** 数值曲线（道具刷新间隔 + 经验值曲线） */
export interface GddBalance {
  /** 道具刷新间隔 ms（>0；下限 250ms 防止刷新风暴） */
  propSpawnIntervalMs: number
  /** 经验值曲线：升到第 n 级所需 EXP = base * n^exponent */
  expCurve: {
    base: number
    exponent: number
  }
}

/** GDD 顶层契约（AI 每一轮自改进的输入/输出同型） */
export interface Gdd {
  /** semver 版本号（major.minor.patch，前向兼容的版本锚点） */
  version: string
  /** 主题（自由文本，但必须非空——是世界氛围的语义锚） */
  theme: string
  /** 玩法支柱（≥1 条非空字符串） */
  pillars: string[]
  props: GddPropSpec[]
  quests: GddQuest[]
  regions: GddRegion[]
  balance: GddBalance
}

// ───────────────────────── 校验结果 ─────────────────────────

export interface GddIssue {
  /** error 阻断加载；warning 只提示不阻断（未知字段=前向兼容） */
  level: 'error' | 'warning'
  /** 机器可读错误码（UI/测试断言用） */
  code: string
  /** 出错位置（JSON path 风格，如 props[0].geometry.scale） */
  path: string
  /** 人类可读明细（i18n 标签由 UI 决定，这里是语言中立的描述） */
  message: string
}

export interface GddValidation {
  ok: boolean
  errors: GddIssue[]
  /** 非阻断提示（如未知字段）；与 errors 分离，UI 可分别着色 */
  warnings: GddIssue[]
  /** 校验通过时的规范化 GDD（失败为 null） */
  value: Gdd | null
}

const SEMVER_RE = /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?$/

/** propsSpec.PropInteraction 枚举收口（与 propsSpec.ts 的类型字面量保持一致） */
const PROP_INTERACTIONS: readonly PropInteraction[] = ['analyze', 'explain', 'insight', 'quest', 'brainstorm']

/** 顶层已知字段（未知字段只 warning，保证旧校验器能读新 GDD——前向兼容） */
const KNOWN_TOP_FIELDS = new Set(['version', 'theme', 'pillars', 'props', 'quests', 'regions', 'balance'])
const KNOWN_PROP_FIELDS = new Set(['kind', 'interaction', 'geometry', 'maxInstances', 'labelKey', 'expReward'])
const KNOWN_GEOMETRY_FIELDS = new Set(['main', 'accent', 'detail', 'scale'])
const KNOWN_QUEST_FIELDS = new Set(['id', 'title', 'trigger', 'rewardRegion'])
const KNOWN_REGION_FIELDS = new Set(['id', 'name', 'locked'])
const KNOWN_BALANCE_FIELDS = new Set(['propSpawnIntervalMs', 'expCurve'])
const KNOWN_EXPCURVE_FIELDS = new Set(['base', 'exponent'])

function isRecord(v: unknown): v is Record<string, unknown> {
  return typeof v === 'object' && v !== null && !Array.isArray(v)
}

function isPositiveInt(v: unknown): v is number {
  return typeof v === 'number' && Number.isInteger(v) && v > 0
}

function reportUnknownFields(
  obj: Record<string, unknown>,
  known: Set<string>,
  path: string,
  warnings: GddIssue[],
): void {
  for (const key of Object.keys(obj)) {
    if (!known.has(key)) {
      warnings.push({
        level: 'warning',
        code: 'unknown_field',
        path: path ? `${path}.${key}` : key,
        message: `未知字段「${key}」已忽略（前向兼容，不阻断加载）`,
      })
    }
  }
}

function validateGeometry(raw: unknown, path: string, errors: GddIssue[], warnings: GddIssue[]): PropGeometry | null {
  if (!isRecord(raw)) {
    errors.push({ level: 'error', code: 'type_error', path, message: 'geometry 必须是对象' })
    return null
  }
  reportUnknownFields(raw, KNOWN_GEOMETRY_FIELDS, path, warnings)
  let ok = true

  const main = raw.main
  if (typeof main !== 'string' || !PROP_GEOMETRY_MAINS.includes(main as PropGeometry['main'])) {
    // 枚举收口：main 必须是 propsSpec 的 5 种 primitive 之一
    errors.push({
      level: 'error',
      code: 'invalid_enum',
      path: `${path}.main`,
      message: `geometry.main 必须是 ${PROP_GEOMETRY_MAINS.join('/')} 之一`,
    })
    ok = false
  }

  if (raw.accent !== undefined) {
    const accent = raw.accent
    if (typeof accent !== 'string' || !PROP_GEOMETRY_ACCENTS.includes(accent as NonNullable<PropGeometry['accent']>)) {
      errors.push({
        level: 'error',
        code: 'invalid_enum',
        path: `${path}.accent`,
        message: `geometry.accent 必须是 ${PROP_GEOMETRY_ACCENTS.join('/')} 之一`,
      })
      ok = false
    }
  }

  const detail = raw.detail
  if (detail !== 1 && detail !== 2 && detail !== 3) {
    errors.push({ level: 'error', code: 'invalid_enum', path: `${path}.detail`, message: 'geometry.detail 必须是 1/2/3' })
    ok = false
  }

  const scale = raw.scale
  // 数值边界：scale 必须 > 0（0/负数会让 three 缩放塌陷或翻面）
  if (typeof scale !== 'number' || !Number.isFinite(scale) || scale <= 0) {
    errors.push({ level: 'error', code: 'out_of_range', path: `${path}.scale`, message: 'geometry.scale 必须是 > 0 的有限数' })
    ok = false
  }

  if (!ok) return null
  return {
    main: main as PropGeometry['main'],
    ...(raw.accent !== undefined ? { accent: raw.accent as NonNullable<PropGeometry['accent']> } : {}),
    detail: detail as 1 | 2 | 3,
    scale: scale as number,
  }
}

function validateProp(raw: unknown, index: number, errors: GddIssue[], warnings: GddIssue[], seen: Set<string>): GddPropSpec | null {
  const path = `props[${index}]`
  if (!isRecord(raw)) {
    errors.push({ level: 'error', code: 'type_error', path, message: 'prop 必须是对象' })
    return null
  }
  reportUnknownFields(raw, KNOWN_PROP_FIELDS, path, warnings)
  let ok = true

  const kind = raw.kind
  if (typeof kind !== 'string' || !PROP_KINDS.includes(kind as PropKind)) {
    errors.push({ level: 'error', code: 'invalid_enum', path: `${path}.kind`, message: `kind 必须是 ${PROP_KINDS.join('/')} 之一` })
    ok = false
  } else if (seen.has(kind)) {
    // id 唯一：同一 kind 的道具契约只能有一份（否则渲染/拾取语义二义）
    errors.push({ level: 'error', code: 'duplicate_id', path: `${path}.kind`, message: `kind「${kind}」重复` })
    ok = false
  } else {
    seen.add(kind)
  }

  const interaction = raw.interaction
  if (typeof interaction !== 'string' || !PROP_INTERACTIONS.includes(interaction as PropInteraction)) {
    errors.push({ level: 'error', code: 'invalid_enum', path: `${path}.interaction`, message: `interaction 必须是 ${PROP_INTERACTIONS.join('/')} 之一` })
    ok = false
  }

  const geometry = validateGeometry(raw.geometry, `${path}.geometry`, errors, warnings)
  if (!geometry) ok = false

  const maxInstances = raw.maxInstances
  // 数值边界：1..64（0=永不出现无意义；>64 突破粒子/内存预算闸门）
  if (!isPositiveInt(maxInstances) || maxInstances > 64) {
    errors.push({ level: 'error', code: 'out_of_range', path: `${path}.maxInstances`, message: 'maxInstances 必须是 1..64 的整数' })
    ok = false
  }

  const labelKey = raw.labelKey
  if (typeof labelKey !== 'string' || labelKey.trim() === '') {
    errors.push({ level: 'error', code: 'type_error', path: `${path}.labelKey`, message: 'labelKey 必须是非空字符串' })
    ok = false
  }

  const expReward = raw.expReward
  if (typeof expReward !== 'number' || !Number.isFinite(expReward) || expReward <= 0) {
    errors.push({ level: 'error', code: 'out_of_range', path: `${path}.expReward`, message: 'expReward 必须是 > 0 的有限数' })
    ok = false
  }

  if (!ok || !geometry) return null
  return {
    kind: kind as PropKind,
    interaction: interaction as PropInteraction,
    geometry,
    maxInstances: maxInstances as number,
    labelKey: labelKey as string,
    expReward: expReward as number,
  }
}

function validateQuest(raw: unknown, index: number, errors: GddIssue[], warnings: GddIssue[], seen: Set<string>): GddQuest | null {
  const path = `quests[${index}]`
  if (!isRecord(raw)) {
    errors.push({ level: 'error', code: 'type_error', path, message: 'quest 必须是对象' })
    return null
  }
  reportUnknownFields(raw, KNOWN_QUEST_FIELDS, path, warnings)
  let ok = true

  const id = raw.id
  if (typeof id !== 'string' || id.trim() === '') {
    errors.push({ level: 'error', code: 'type_error', path: `${path}.id`, message: 'id 必须是非空字符串' })
    ok = false
  } else if (seen.has(id)) {
    errors.push({ level: 'error', code: 'duplicate_id', path: `${path}.id`, message: `id「${id}」重复` })
    ok = false
  } else {
    seen.add(id)
  }

  for (const field of ['title', 'trigger'] as const) {
    if (typeof raw[field] !== 'string' || (raw[field] as string).trim() === '') {
      errors.push({ level: 'error', code: 'type_error', path: `${path}.${field}`, message: `${field} 必须是非空字符串` })
      ok = false
    }
  }

  const rewardRegion = raw.rewardRegion
  if (typeof rewardRegion !== 'string') {
    errors.push({ level: 'error', code: 'type_error', path: `${path}.rewardRegion`, message: 'rewardRegion 必须是字符串（可为空=无解锁）' })
    ok = false
  }

  if (!ok) return null
  return {
    id: id as string,
    title: raw.title as string,
    trigger: raw.trigger as string,
    rewardRegion: rewardRegion as string,
  }
}

function validateRegion(raw: unknown, index: number, errors: GddIssue[], warnings: GddIssue[], seen: Set<string>): GddRegion | null {
  const path = `regions[${index}]`
  if (!isRecord(raw)) {
    errors.push({ level: 'error', code: 'type_error', path, message: 'region 必须是对象' })
    return null
  }
  reportUnknownFields(raw, KNOWN_REGION_FIELDS, path, warnings)
  let ok = true

  const id = raw.id
  if (typeof id !== 'string' || id.trim() === '') {
    errors.push({ level: 'error', code: 'type_error', path: `${path}.id`, message: 'id 必须是非空字符串' })
    ok = false
  } else if (seen.has(id)) {
    errors.push({ level: 'error', code: 'duplicate_id', path: `${path}.id`, message: `id「${id}」重复` })
    ok = false
  } else {
    seen.add(id)
  }

  if (typeof raw.name !== 'string' || (raw.name as string).trim() === '') {
    errors.push({ level: 'error', code: 'type_error', path: `${path}.name`, message: 'name 必须是非空字符串' })
    ok = false
  }
  if (typeof raw.locked !== 'boolean') {
    errors.push({ level: 'error', code: 'type_error', path: `${path}.locked`, message: 'locked 必须是布尔值' })
    ok = false
  }

  if (!ok) return null
  return { id: id as string, name: raw.name as string, locked: raw.locked as boolean }
}

function validateBalance(raw: unknown, errors: GddIssue[], warnings: GddIssue[]): GddBalance | null {
  const path = 'balance'
  if (!isRecord(raw)) {
    errors.push({ level: 'error', code: 'type_error', path, message: 'balance 必须是对象' })
    return null
  }
  reportUnknownFields(raw, KNOWN_BALANCE_FIELDS, path, warnings)
  let ok = true

  const interval = raw.propSpawnIntervalMs
  // 刷新间隔：>0 且 ≥250ms（防刷新风暴）且 ≤1 小时（明显笔误兜底）
  if (typeof interval !== 'number' || !Number.isFinite(interval) || interval < 250 || interval > 3_600_000) {
    errors.push({ level: 'error', code: 'out_of_range', path: `${path}.propSpawnIntervalMs`, message: 'propSpawnIntervalMs 必须是 250..3600000 的有限数' })
    ok = false
  }

  let expCurve: GddBalance['expCurve'] | null = null
  const curveRaw = raw.expCurve
  if (!isRecord(curveRaw)) {
    errors.push({ level: 'error', code: 'type_error', path: `${path}.expCurve`, message: 'expCurve 必须是对象' })
    ok = false
  } else {
    reportUnknownFields(curveRaw, KNOWN_EXPCURVE_FIELDS, `${path}.expCurve`, warnings)
    const base = curveRaw.base
    const exponent = curveRaw.exponent
    if (typeof base !== 'number' || !Number.isFinite(base) || base <= 0) {
      errors.push({ level: 'error', code: 'out_of_range', path: `${path}.expCurve.base`, message: 'expCurve.base 必须是 > 0 的有限数' })
      ok = false
    }
    if (typeof exponent !== 'number' || !Number.isFinite(exponent) || exponent < 1 || exponent > 4) {
      errors.push({ level: 'error', code: 'out_of_range', path: `${path}.expCurve.exponent`, message: 'expCurve.exponent 必须是 1..4 的有限数' })
      ok = false
    }
    if (typeof base === 'number' && typeof exponent === 'number') {
      expCurve = { base: base as number, exponent: exponent as number }
    }
  }

  if (!ok || !expCurve) return null
  return { propSpawnIntervalMs: interval as number, expCurve }
}

/**
 * 校验一份 GDD 原始数据（对象或已 parse 的 JSON）。
 *
 * 策略（前向兼容 + 阻断策略）：
 * - **error**：结构性/数值/枚举/唯一性问题 → ok=false，热加载拒绝并保持旧配置；
 * - **warning**：未知字段 → 记录但不阻断（旧运行时可安全读新 GDD，AI 可加字段试验）。
 */
export function validateGdd(raw: unknown): GddValidation {
  const errors: GddIssue[] = []
  const warnings: GddIssue[] = []

  if (!isRecord(raw)) {
    errors.push({ level: 'error', code: 'type_error', path: '', message: 'GDD 必须是 JSON 对象' })
    return { ok: false, errors, warnings, value: null }
  }
  reportUnknownFields(raw, KNOWN_TOP_FIELDS, '', warnings)

  // version：semver 必填（版本锚点缺失则热加载无法做版本化回灌）
  const version = raw.version
  if (typeof version !== 'string' || version.trim() === '') {
    errors.push({ level: 'error', code: 'type_error', path: 'version', message: 'version 必填（semver 字符串）' })
  } else if (!SEMVER_RE.test(version)) {
    errors.push({ level: 'error', code: 'invalid_semver', path: 'version', message: `version「${version}」不是合法 semver（major.minor.patch）` })
  }

  const theme = raw.theme
  if (typeof theme !== 'string' || theme.trim() === '') {
    errors.push({ level: 'error', code: 'type_error', path: 'theme', message: 'theme 必须是非空字符串' })
  }

  const pillars: string[] = []
  if (!Array.isArray(raw.pillars) || raw.pillars.length === 0) {
    errors.push({ level: 'error', code: 'type_error', path: 'pillars', message: 'pillars 必须是非空字符串数组（≥1 条玩法支柱）' })
  } else {
    raw.pillars.forEach((p, i) => {
      if (typeof p !== 'string' || p.trim() === '') {
        errors.push({ level: 'error', code: 'type_error', path: `pillars[${i}]`, message: '玩法支柱必须是非空字符串' })
      } else {
        pillars.push(p)
      }
    })
  }

  const props: GddPropSpec[] = []
  const seenKinds = new Set<string>()
  if (!Array.isArray(raw.props) || raw.props.length === 0) {
    errors.push({ level: 'error', code: 'type_error', path: 'props', message: 'props 必须是非空数组（≥1 种道具）' })
  } else {
    raw.props.forEach((p, i) => {
      const spec = validateProp(p, i, errors, warnings, seenKinds)
      if (spec) props.push(spec)
    })
  }

  const quests: GddQuest[] = []
  const seenQuestIds = new Set<string>()
  if (!Array.isArray(raw.quests)) {
    errors.push({ level: 'error', code: 'type_error', path: 'quests', message: 'quests 必须是数组（可为空）' })
  } else {
    raw.quests.forEach((q, i) => {
      const quest = validateQuest(q, i, errors, warnings, seenQuestIds)
      if (quest) quests.push(quest)
    })
  }

  const regions: GddRegion[] = []
  const seenRegionIds = new Set<string>()
  if (!Array.isArray(raw.regions)) {
    errors.push({ level: 'error', code: 'type_error', path: 'regions', message: 'regions 必须是数组（可为空）' })
  } else {
    raw.regions.forEach((r, i) => {
      const region = validateRegion(r, i, errors, warnings, seenRegionIds)
      if (region) regions.push(region)
    })
  }

  // 交叉引用：任务奖励区域必须指向已声明的区域（防止悬空引用在运行时炸任务状态机）
  const regionIds = new Set(regions.map((r) => r.id))
  quests.forEach((q, i) => {
    if (q.rewardRegion !== '' && !regionIds.has(q.rewardRegion)) {
      errors.push({
        level: 'error',
        code: 'dangling_ref',
        path: `quests[${i}].rewardRegion`,
        message: `奖励区域「${q.rewardRegion}」未在 regions 中声明`,
      })
    }
  })

  const balance = validateBalance(raw.balance, errors, warnings)

  if (errors.length > 0) {
    return { ok: false, errors, warnings, value: null }
  }
  return {
    ok: true,
    errors,
    warnings,
    value: {
      version: version as string,
      theme: theme as string,
      pillars,
      props,
      quests,
      regions,
      balance: balance as GddBalance,
    },
  }
}
