// 对话产线规格 —— 构造、旧挡位互转、本地校验与持久化。
//
// 这里没有"界面状态"这一层：形状只存在于 `LineSpec` 对象里（设计定律一）。
// 组件不得为某个旋钮另开一个 ref —— 那等于把 §1.1 的根因重新请回来。
import type { LegacyOrchestrationForm, LineCatalog, LinePatternMeta, LineSpec, LineStage } from './types'
import { LINE_PATTERNS, type LinePattern } from './types'

export const LINE_SPEC_STORAGE_KEY = 'marguerite.line.spec.v1'

/** 与后端 `MAX_STAGES`/`MAX_WIDTH` 同名的兜底值，仅在 `line.catalog` 还没 loaded 时用。
 *  用途只有一个：让"加阶段"按钮在断线时也不给出无界 UI。真正的裁决永远以后端 `limits` 为准。 */
const FALLBACK_LIMITS = { maxStages: 3, maxWidth: 16 } as const

let seq = 0

function stageId(): string {
  seq += 1
  return `s${seq}`
}

/**
 * 新建阶段：宽度与收口都由后端目录的元数据决定（`rosterKind` / `needsSynthesis`），
 * 前端不持有任何"几种形状各自该长什么样"的第二份描述。
 * 目录未 loaded ⇒ 元数据为 null ⇒ 走 solo 的保守缺省（宽度 1、不收口），
 * 因为这是唯一一种"填错也不会静默降级"的形状。
 */
export function createStage(pattern: LinePattern, meta?: LinePatternMeta | null): LineStage {
  const kind = meta?.rosterKind ?? 'none'
  return {
    id: stageId(),
    pattern,
    width: !meta ? 1 : kind === 'none' ? 3 : 2,
    roster: [],
    facets: [],
    // 面板/接力天生需要"谁来收口"；并联/蜂群没有收口就是散会（`needs_synthesis` 同源）。
    synthesize: meta ? meta.needsSynthesis : pattern === 'panel',
    note: '',
  }
}

/** 空规格 = 一条 solo：单代理是产线的退化情形，不是"没开产线"。 */
export function createDefaultSpec(goal = ''): LineSpec {
  return {
    specId: `ls-${Date.now()}-${Math.random().toString(36).slice(2, 10)}`,
    goal,
    acceptance: [],
    stages: [createStage('solo')],
    criteria: { maxFailProbability: 0.25, budgetTokens: null, deadlineMs: null },
  }
}

/**
 * 旧五挡 → 规格（设计文档 §2 的兼容表，此处是该表可执行的那一份）。
 *
 * `subagent` 的 `synthesize:false` 是**如实**：引擎那条路径今天就直接返回 `summaries[]`，
 * 不做 lead 综合。把它在这儿写成 true 会让界面宣称一个不存在的行为。
 */
export function fromLegacyForm(
  form: LegacyOrchestrationForm,
  knobs: { agents?: string[]; teams?: string[]; swarmSize?: number } = {},
): LineSpec {
  const spec = createDefaultSpec()
  switch (form) {
    case 'single':
      return spec
    case 'subagent': {
      const stage = createStage('fanout')
      stage.roster = knobs.agents ?? []
      stage.width = Math.max(1, stage.roster.length)
      stage.synthesize = false
      spec.stages = [stage]
      return spec
    }
    case 'team':
    case 'expert': {
      const stage = createStage('panel')
      stage.roster = knobs.teams ?? knobs.agents ?? []
      stage.width = Math.max(1, stage.roster.length || 3)
      stage.synthesize = true
      spec.stages = [stage]
      return spec
    }
    case 'swarm': {
      const stage = createStage('swarm')
      stage.width = Math.max(2, knobs.swarmSize ?? 3)
      stage.synthesize = false
      spec.stages = [stage]
      return spec
    }
  }
}

/**
 * 规格 → 旧五挡（给还没迁走的 `orchestrate.run` 路径用）。
 * 多于一个阶段就返回 null：**组合形状在旧契约里不可表示**，如实拒绝比压扁成单挡诚实。
 */
export function toLegacyForm(spec: LineSpec): {
  form: LegacyOrchestrationForm
  agents: string[]
  teams: string[]
  swarmOn: boolean
  swarmSize: number
} | null {
  if (spec.stages.length !== 1) return null
  const [stage] = spec.stages
  const base = { agents: [], teams: [], swarmOn: false, swarmSize: 3 }
  switch (stage.pattern) {
    case 'solo':
      return { ...base, form: 'single' }
    case 'fanout':
      return { ...base, form: 'subagent', agents: stage.roster }
    case 'panel':
      return { ...base, form: 'team', teams: stage.roster }
    case 'swarm':
      return { ...base, form: 'swarm', swarmOn: true, swarmSize: stage.width }
    case 'relay':
      // 接力是链式依赖，旧契约没有对应挡位。
      return null
  }
}

/**
 * 本地校验：**只做后端口径的子集**，且上限全部取自 `line.catalog`。
 * catalog 未 loaded ⇒ 不校验（绝不"猜一个上限"去拦用户，那会造成界面拒了、后端其实放行）。
 *
 * 名册按 `rosterKind` 分口径查候选：`panel` 要的是**专家团 id**、`fanout`/`relay` 要的是
 * **角色席位名**。喂错一类，后端的 `selected_rosters` 就解析不到 ⇒ 静默降级成自动检测
 * （DEF-10 那一族）。所以这里拦的不是"名字不存在"，而是"这一类名字根本不该出现在这里"。
 */
export function validateLocal(
  spec: LineSpec,
  catalog: LineCatalog,
): { errors: string[]; warnings: string[] } {
  const errors: string[] = []
  const warnings: string[] = []
  if (!catalog.loaded || !catalog.limits) return { errors, warnings }
  const { maxStages, maxWidth } = catalog.limits

  if (!spec.goal.trim()) errors.push('goal 不可为空')
  if (spec.stages.length === 0) errors.push('至少需要一个阶段')
  if (spec.stages.length > maxStages) errors.push(`阶段数 ${spec.stages.length} 超过上限 ${maxStages}`)

  const teamIds = new Set(catalog.teams.map((t) => t.id))
  const roleIds = new Set(catalog.roles.map((r) => r.id))

  for (const stage of spec.stages) {
    const kind = catalog.patterns.find((p) => p.id === stage.pattern)?.rosterKind ?? 'none'
    if (stage.pattern === 'solo') continue
    if (stage.width < 1) errors.push(`${stage.id}：宽度必须 ≥ 1`)
    if (stage.width > maxWidth) errors.push(`${stage.id}：宽度 ${stage.width} 超过上限 ${maxWidth}`)

    if (kind !== 'none' && stage.roster.length === 0) {
      errors.push(`${stage.id}：${stage.pattern} 需要非空名册，否则会静默降级`)
    }
    if (kind === 'teams') {
      for (const id of stage.roster) {
        if (!teamIds.has(id)) {
          errors.push(`${stage.id}：「${id}」不是专家团 id（这一格要填团队，不是角色名）`)
        }
      }
      if (stage.roster.length > 0 && stage.width > stage.roster.length) {
        warnings.push(
          `${stage.id}：宽度 ${stage.width} > 名册团队数 ${stage.roster.length}，多出的成员由名册展开补齐`,
        )
      }
    }
    if (kind === 'roles') {
      for (const id of stage.roster) {
        if (!roleIds.has(id)) {
          warnings.push(`${stage.id}：席位「${id}」不在内建角色表里，将按席位名执行`)
        }
      }
    }
    if (stage.width > catalog.limits.subagentMaxConcurrent) {
      // 排队不是失败：Semaphore 会排队，但吞吐瓶颈是真实的，必须说出来而不是藏起来。
      warnings.push(
        `${stage.id}：${stage.width} 个成员 > 并发上限 ${catalog.limits.subagentMaxConcurrent}，多出的会排队`,
      )
    }
  }
  return { errors, warnings }
}

/** 把形状说成一句话给轨道摘要用；标签由调用方注入（本模块不持有文案）。 */
export function summarizeShape(
  spec: LineSpec,
  labelOf: (pattern: LinePattern) => string,
  widthOf = (s: LineStage) => (s.pattern === 'solo' ? '' : String(s.width)),
): string {
  return spec.stages
    .map((s) => {
      const w = widthOf(s)
      return w ? `${w} ${labelOf(s.pattern)}` : labelOf(s.pattern)
    })
    .join(' → ')
}

function sanitizeStage(value: unknown): LineStage | null {
  if (!value || typeof value !== 'object') return null
  const raw = value as Partial<LineStage>
  if (typeof raw.pattern !== 'string' || !LINE_PATTERNS.includes(raw.pattern as LinePattern)) {
    return null
  }
  const pattern = raw.pattern as LinePattern
  const width = Number(raw.width)
  return {
    id: typeof raw.id === 'string' && raw.id ? raw.id : stageId(),
    pattern,
    width: Number.isFinite(width) && width > 0 ? Math.trunc(width) : 1,
    roster: Array.isArray(raw.roster) ? raw.roster.filter((x): x is string => typeof x === 'string') : [],
    facets: Array.isArray(raw.facets) ? raw.facets.filter((x): x is string => typeof x === 'string') : [],
    synthesize: raw.synthesize === true,
    note: typeof raw.note === 'string' ? raw.note : '',
  }
}

/** 读回上次编辑的规格；任何脏数据一律退回默认值（绝不把非法形状送进后端）。 */
export function loadStoredLineSpec(): LineSpec {
  const fallback = createDefaultSpec()
  let raw: string | null = null
  try {
    raw = globalThis.localStorage?.getItem(LINE_SPEC_STORAGE_KEY) ?? null
  } catch {
    return fallback
  }
  if (!raw) return fallback

  let saved: Partial<LineSpec> | null = null
  try {
    saved = JSON.parse(raw) as Partial<LineSpec>
  } catch {
    return fallback
  }
  if (!saved || typeof saved !== 'object' || !Array.isArray(saved.stages)) return fallback

  const stages = saved.stages.map(sanitizeStage).filter((s): s is LineStage => s !== null)
  if (stages.length === 0 || stages.length > FALLBACK_LIMITS.maxStages) return fallback

  const criteria = (saved.criteria ?? {}) as Partial<LineSpec['criteria']>
  const maxFail = Number(criteria.maxFailProbability)
  return {
    specId: typeof saved.specId === 'string' ? saved.specId : fallback.specId,
    goal: typeof saved.goal === 'string' ? saved.goal : '',
    acceptance: Array.isArray(saved.acceptance)
      ? saved.acceptance.filter((x): x is string => typeof x === 'string' && x.trim() !== '')
      : [],
    stages,
    criteria: {
      maxFailProbability: Number.isFinite(maxFail) ? maxFail : fallback.criteria.maxFailProbability,
      budgetTokens: typeof criteria.budgetTokens === 'number' ? criteria.budgetTokens : null,
      deadlineMs: typeof criteria.deadlineMs === 'number' ? criteria.deadlineMs : null,
    },
  }
}

/** 落盘偏好（隐私模式 / 配额满 ⇒ 静默，不阻断发送）。 */
export function persistLineSpec(spec: Readonly<LineSpec>): void {
  try {
    globalThis.localStorage?.setItem(LINE_SPEC_STORAGE_KEY, JSON.stringify(spec))
  } catch {
    // 存储不可用时保持内存态。
  }
}

export function canAddStage(spec: LineSpec, catalog: Pick<LineCatalog, 'loaded' | 'limits'>): boolean {
  const cap = catalog.loaded && catalog.limits ? catalog.limits.maxStages : FALLBACK_LIMITS.maxStages
  return spec.stages.length < cap
}

/**
 * 套用配方：把引擎给定的阶段序列装进当前规格。
 *
 * 只换 `stages`，**不动** `goal/acceptance/criteria` —— 配方描述的是"这件事由几种形状
 * 怎么串"，目标与判据是用户这一轮自己的事。配方阶段为空（后端契约变更）时返回 false，
 * 调用方必须显示"配方不可用"，而不是把产线悄悄清成一条 solo。
 */
export function applyRecipe(spec: LineSpec, stages: LineStage[]): { spec: LineSpec; applied: boolean } {
  if (stages.length === 0) return { spec, applied: false }
  return {
    spec: {
      ...spec,
      // 阶段 id 重新发号：同一会话里先后套过两个配方时，id 撞车会让进度帧串门。
      stages: stages.map((s) => ({ ...s, id: stageId(), roster: [...s.roster], facets: [...s.facets] })),
    },
    applied: true,
  }
}

/** 换形状：名册按新形状的 `rosterKind` 清空。留着旧一类 id 就是"填错类"的直接来源。 */
export function setStagePattern(
  stage: LineStage,
  pattern: LinePattern,
  meta?: LinePatternMeta | null,
): LineStage {
  const fresh = createStage(pattern, meta)
  return {
    ...stage,
    pattern,
    width: fresh.width,
    synthesize: fresh.synthesize,
    roster: stage.pattern === pattern ? stage.roster : [],
    facets: pattern === 'swarm' ? stage.facets : [],
  }
}

export function toggleRoster(stage: LineStage, id: string): LineStage {
  const next = stage.roster.includes(id)
    ? stage.roster.filter((r) => r !== id)
    : [...stage.roster, id]
  // 宽度跟名册走：名字改了、宽度还留着旧值，界面显示 4 路实际跑 2 个成员
  // ——这正是 `orchestrate.run` 忽略 `swarmSize` 那一类"说了没做"的缺陷。
  const width = stage.pattern === 'solo' ? 1 : Math.max(1, next.length || stage.width)
  return { ...stage, roster: next, width }
}

export function moveStage(stages: LineStage[], from: number, to: number): LineStage[] {
  if (from === to || from < 0 || to < 0 || from >= stages.length || to >= stages.length) {
    return stages
  }
  const next = [...stages]
  const [picked] = next.splice(from, 1)
  next.splice(to, 0, picked)
  return next
}
