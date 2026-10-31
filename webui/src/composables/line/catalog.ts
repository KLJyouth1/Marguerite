// 对话产线目录 store —— 展示层常量的唯一来源。
//
// 这个文件存在的意义就是**消掉常量**：`orchestration.ts` 里手抄的 `ORCHESTRATION_ROLES`
// / `EXPERT_TEAMS` / 蜂群规模档位，都是"前端一份、后端一份"的两份描述，本模块一律不复制，
// 全部从 `line.catalog` 读。
//
// 诚实约束（沿用 `MembershipProbe 恒 NotProbed` 的既有判据）：**没有数据就不假装有能力**。
// 目录未加载 ⇒ 空数组 + `loaded:false`，组件据此把控件置灰并说明原因，
// 绝不用前端硬编码顶上去——那正是本设计要消灭的那类缺陷。
//
// 解析纪律：每个 `xxxOf()` 都按 Rust 侧 `line_catalog()` 的**真实**字段名取值。
// 上一版把 `facets`（`{id, instruction}` 对象数组）当字符串数组读、把 `validation.errors`
// 当顶层 `errors` 读，结果类型看着齐、值永远是空 —— 空数组在这套契约里等于"功能静默失效"。
import { defineStore } from 'pinia'
import { computed, ref } from 'vue'

import { useRpcStore } from '@/stores/rpc'
import { isLinePattern, LINE_PATTERNS, type LineCatalog, type LineFacet, type LineGate, type LineLimits, type LinePattern, type LinePatternMeta, type LinePreflight, type LineRecipe, type LineRoleCandidate, type LineSpec, type LineStage, type LineTeamCandidate, type LineValidation, type LegacyOrchestrationForm, type LineLegacyFormMeta, type LinePlan, type RosterKind, type StagePlanEntry } from './types'

const ROSTER_KINDS: readonly RosterKind[] = ['teams', 'roles', 'none']
const LEGACY_FORMS: readonly LegacyOrchestrationForm[] = [
  'single',
  'subagent',
  'team',
  'expert',
  'swarm',
]

const EMPTY: LineCatalog = {
  loaded: false,
  patterns: [],
  recipes: [],
  legacyForms: [],
  roles: [],
  teams: [],
  facets: [],
  limits: null,
  defaults: null,
}

function asRecord(value: unknown): Record<string, unknown> | null {
  return value && typeof value === 'object' ? (value as Record<string, unknown>) : null
}

function strings(value: unknown): string[] {
  return Array.isArray(value) ? value.filter((x): x is string => typeof x === 'string') : []
}

function nullableNumber(value: unknown): number | null {
  return typeof value === 'number' && Number.isFinite(value) ? value : null
}

function patternMetaOf(value: unknown): LinePatternMeta | null {
  const item = asRecord(value)
  if (!item) return null
  const id = typeof item.id === 'string' ? item.id : ''
  // 后端将来加第六种形状时，前端**丢掉这一条**而不是猜一个：未知形状进不了 UI，
  // 也就进不了规格，更不会被当成"已知但没文案的形状"渲染成空白按钮。
  if (!isLinePattern(id)) return null
  const rosterKind = ROSTER_KINDS.includes(item.rosterKind as RosterKind)
    ? (item.rosterKind as RosterKind)
    : 'none'
  return {
    id,
    label: typeof item.label === 'string' ? item.label : id,
    description: typeof item.description === 'string' ? item.description : '',
    rosterKind,
    needsSynthesis: item.needsSynthesis === true,
    topology: typeof item.topology === 'string' ? item.topology : '',
  }
}

/** 阶段条目的前端侧消毒：未知 pattern / 非数值宽度一律整条丢弃。 */
function stageOf(value: unknown): LineStage | null {
  const item = asRecord(value)
  if (!item || !isLinePattern(item.pattern)) return null
  const width = Number(item.width)
  return {
    id: typeof item.id === 'string' && item.id ? item.id : `s${width || 1}`,
    pattern: item.pattern,
    width: Number.isFinite(width) && width > 0 ? Math.trunc(width) : 1,
    roster: strings(item.roster),
    facets: strings(item.facets),
    synthesize: item.synthesize === true,
    note: typeof item.note === 'string' ? item.note : '',
  }
}

function recipeOf(value: unknown): LineRecipe | null {
  const item = asRecord(value)
  if (!item) return null
  const id = typeof item.id === 'string' ? item.id : ''
  if (!id) return null
  // 配方的阶段序列由引擎给出，前端原样收下；一条都解析不出来时留空数组，
  // 由 `applyRecipe` 显式报错，而不是假装配方能用。
  const stages = Array.isArray(item.stages)
    ? item.stages.map(stageOf).filter((s): s is LineStage => s !== null)
    : []
  return { id, label: typeof item.label === 'string' ? item.label : id, stages }
}

function legacyFormOf(value: unknown): LineLegacyFormMeta | null {
  const item = asRecord(value)
  if (!item) return null
  const id = typeof item.id === 'string' ? item.id : ''
  if (!LEGACY_FORMS.includes(id as LegacyOrchestrationForm)) return null
  const stages = strings(item.stages).filter(isLinePattern)
  return {
    id: id as LegacyOrchestrationForm,
    label: typeof item.label === 'string' ? item.label : id,
    stages,
  }
}

function roleOf(value: unknown): LineRoleCandidate | null {
  const item = asRecord(value)
  if (!item) return null
  const id = typeof item.id === 'string' ? item.id : ''
  if (!id) return null
  return {
    id,
    label: typeof item.label === 'string' ? item.label : id,
    agentKey: typeof item.agentKey === 'string' ? item.agentKey : id,
    personaFallback: item.personaFallback === true,
  }
}

function teamOf(value: unknown): LineTeamCandidate | null {
  const item = asRecord(value)
  if (!item) return null
  const id = typeof item.id === 'string' ? item.id : ''
  if (!id) return null
  return { id, label: typeof item.label === 'string' ? item.label : id, roles: strings(item.roles) }
}

function facetOf(value: unknown): LineFacet | null {
  const item = asRecord(value)
  if (!item) return null
  const id = typeof item.id === 'string' ? item.id : ''
  if (!id) return null
  return { id, instruction: typeof item.instruction === 'string' ? item.instruction : '' }
}

function limitsOf(value: unknown): LineLimits | null {
  const raw = asRecord(value)
  if (!raw) return null
  const { maxStages, maxWidth, subagentMaxConcurrent, runKeep, dirBudgetBytes } = raw
  if (
    typeof maxStages !== 'number'
    || typeof maxWidth !== 'number'
    || typeof subagentMaxConcurrent !== 'number'
  ) {
    return null
  }
  return {
    maxStages,
    maxWidth,
    subagentMaxConcurrent,
    runKeep: typeof runKeep === 'number' ? runKeep : 0,
    dirBudgetBytes: typeof dirBudgetBytes === 'number' ? dirBudgetBytes : 0,
  }
}

function defaultsOf(value: unknown): LineCatalog['defaults'] {
  const raw = asRecord(value)
  if (!raw) return null
  const criteria = asRecord(raw.criteria)
  if (!criteria) return null
  const maxFail = Number(criteria.maxFailProbability)
  return {
    criteria: {
      maxFailProbability: Number.isFinite(maxFail) ? maxFail : 0.25,
      budgetTokens: nullableNumber(criteria.budgetTokens),
      deadlineMs: nullableNumber(criteria.deadlineMs),
    },
    trials: typeof raw.trials === 'number' ? raw.trials : 200,
    maxTrials: typeof raw.maxTrials === 'number' ? raw.maxTrials : 2000,
  }
}

export const useLineCatalog = defineStore('lineCatalog', () => {
  const rpc = useRpcStore()
  const catalog = ref<LineCatalog>({ ...EMPTY })
  const loading = ref(false)
  const error = ref<string | null>(null)

  /** 后端还没上这一条 RPC 时，`available` 为 false ⇒ 轨道整体不出现，而不是出现一套假常量。 */
  const available = computed(() => rpc.supportsMethod('line.catalog'))
  /**
   * 计划（`line.plan`）单独问一次：它没被声明时只该让"执行映射"这一段消失，
   * 主目录还在、预演还在 —— 配角缺席不该把主表带下水。
   */
  const planAvailable = computed(() => rpc.supportsMethod('line.plan'))
  const limits = computed(() => catalog.value.limits)
  const patternIds = computed(() => catalog.value.patterns.map((p) => p.id))

  function patternMeta(pattern: LinePattern): LinePatternMeta | undefined {
    return catalog.value.patterns.find((p) => p.id === pattern)
  }

  function patternLabel(pattern: LinePattern): string {
    return patternMeta(pattern)?.label ?? pattern
  }

  /** 名册该填团队 id 还是角色席位名 —— 问后端，不问前端的心情。 */
  function rosterKindOf(pattern: LinePattern): RosterKind {
    return patternMeta(pattern)?.rosterKind ?? 'none'
  }

  function labelOf(id: string): string {
    // facets 只有 `instruction` 没有 `label`（Rust 侧如此），混进候选数组会让这个函数
    // 在类型上"能找到"、在值上永远拿到 undefined —— 只列真正带 label 的候选。
    const candidates: Array<{ id: string; label: string }> = [
      ...catalog.value.roles,
      ...catalog.value.teams,
      ...catalog.value.recipes,
      ...catalog.value.patterns,
    ]
    const found = candidates.find((item) => item.id === id)
    // 认不出来就原样回显：宁可显示一个陌生的 id，也不替它编一个中文名。
    return found?.label ?? id
  }

  async function load(force = false): Promise<void> {
    if (catalog.value.loaded && !force) return
    if (!available.value) {
      error.value = 'line.catalog 未被网关声明'
      return
    }
    loading.value = true
    error.value = null
    try {
      const res = asRecord(await rpc.call<Record<string, unknown>>('line.catalog', {}))
      const patterns = (Array.isArray(res?.patterns) ? res.patterns : [])
        .map(patternMetaOf)
        .filter((p): p is LinePatternMeta => p !== null)
      catalog.value = {
        // patterns 为空 ⇒ 后端契约已变，整套控件必须置灰，不能"部分可用"。
        loaded: patterns.length > 0,
        patterns,
        recipes: (Array.isArray(res?.recipes) ? res.recipes : [])
          .map(recipeOf)
          .filter((r): r is LineRecipe => r !== null),
        legacyForms: (Array.isArray(res?.legacyForms) ? res.legacyForms : [])
          .map(legacyFormOf)
          .filter((l): l is LineLegacyFormMeta => l !== null),
        roles: (Array.isArray(res?.roles) ? res.roles : [])
          .map(roleOf)
          .filter((r): r is LineRoleCandidate => r !== null),
        teams: (Array.isArray(res?.teams) ? res.teams : [])
          .map(teamOf)
          .filter((t): t is LineTeamCandidate => t !== null),
        facets: (Array.isArray(res?.facets) ? res.facets : [])
          .map(facetOf)
          .filter((f): f is LineFacet => f !== null),
        limits: limitsOf(res?.limits),
        defaults: defaultsOf(res?.defaults),
      }
      if (!catalog.value.loaded) error.value = 'line.catalog 返回的 patterns 不可用'
    } catch (e) {
      error.value = e instanceof Error ? e.message : String(e)
      catalog.value = { ...EMPTY }
    } finally {
      loading.value = false
    }
  }

  /** 后端 `validation` 段的统一读法（valid/errors/warnings 都在它里面，不在顶层）。 */
  function validationOf(res: Record<string, unknown> | null): LineValidation {
    const raw = asRecord(res?.validation)
    return {
      valid: raw ? raw.valid !== false : true,
      errors: strings(raw?.errors),
      warnings: strings(raw?.warnings),
      maxConcurrent: typeof raw?.maxConcurrent === 'number' ? raw.maxConcurrent : 0,
    }
  }

  async function compile(input: {
    goal: string
    recipeId?: string
    stages?: LineStage[]
    spec?: LineSpec
  }): Promise<{ spec: LineSpec | null; shape: string; validation: LineValidation }> {
    const res = asRecord(
      await rpc.call<Record<string, unknown>>('line.compile', {
        goal: input.goal,
        ...(input.recipeId ? { recipeId: input.recipeId } : {}),
        ...(input.stages ? { stages: input.stages } : {}),
        ...(input.spec ? { spec: input.spec } : {}),
      }),
    )
    return {
      spec: (asRecord(res?.spec) as LineSpec | null) ?? null,
      shape: typeof res?.shape === 'string' ? res.shape : '',
      validation: validationOf(res),
    }
  }

  /** 权限门结论。后端给出没见过的值时按 `deny` 处理 —— 权限判断必须失败关闭。 */
  function gateOf(value: unknown): LineGate {
    const raw = asRecord(value)
    const decision = raw?.decision
    return {
      autonomy: typeof raw?.autonomy === 'string' ? raw.autonomy : 'unknown',
      decision: decision === 'allow' || decision === 'confirm' || decision === 'deny' ? decision : 'deny',
      reason: typeof raw?.reason === 'string' ? raw.reason : '',
    }
  }

  /**
   * `line.preflight` 的**归一化**读取。
   *
   * 被拒时后端只回 `ok:false + validation + gate`，`stages/total/premortem/unknowns` 全部缺席。
   * 直接把响应体断言成 `LinePreflight` 会得到"类型齐全、值全是 undefined"的对象——
   * 那正是本功能要消灭的那类静默缺陷。这里逐段补成合法空值，UI 就能只靠 `ok` 分支。
   */
  function preflightOf(res: Record<string, unknown>): LinePreflight {
    const total = asRecord(res.total)
    const number = (value: unknown): number => (typeof value === 'number' && Number.isFinite(value) ? value : 0)
    return {
      ok: res.ok !== false,
      shape: typeof res.shape === 'string' ? res.shape : '',
      stages: Array.isArray(res.stages) ? (res.stages as LinePreflight['stages']) : [],
      total: {
        p50Ms: number(total?.p50Ms),
        p90Ms: number(total?.p90Ms),
        p50Tokens: number(total?.p50Tokens),
        p90Tokens: number(total?.p90Tokens),
        p90CostUsd: number(total?.p90CostUsd),
        successRate: number(total?.successRate),
        failProbability: number(total?.failProbability),
      },
      rationale: (asRecord(res.rationale) ?? {}) as LinePreflight['rationale'],
      premortem: Array.isArray(res.premortem) ? (res.premortem as LinePreflight['premortem']) : [],
      unknowns: Array.isArray(res.unknowns) ? (res.unknowns as LinePreflight['unknowns']) : [],
      chosen: typeof res.chosen === 'string' ? res.chosen : null,
      chosenVsRecommended:
        typeof res.chosenVsRecommended === 'string' ? res.chosenVsRecommended : null,
      validation: validationOf(res),
      gate: gateOf(res.gate),
      error: typeof res.error === 'string' ? res.error : null,
      code: typeof res.code === 'string' ? res.code : undefined,
    }
  }

  async function preflight(
    spec: LineSpec,
    modeKey?: string,
    confirmed?: boolean,
  ): Promise<LinePreflight | null> {
    const res = asRecord(
      await rpc.call<Record<string, unknown>>('line.preflight', {
        spec,
        ...(modeKey ? { mode: modeKey } : {}),
        ...(confirmed === undefined ? {} : { confirmed }),
      }),
    )
    if (!res) return null
    return preflightOf(res)
  }

  async function plan(spec: LineSpec, modeKey?: string, confirmed?: boolean): Promise<LinePlan | null> {
    const res = asRecord(
      await rpc.call<Record<string, unknown>>('line.plan', {
        spec,
        ...(modeKey ? { mode: modeKey } : {}),
        ...(confirmed === undefined ? {} : { confirmed }),
      }),
    )
    if (!res) return null
    return {
      ok: res.ok !== false,
      shape: typeof res.shape === 'string' ? res.shape : '',
      spec: res.spec as LineSpec,
      stages: (Array.isArray(res.stages) ? res.stages : []) as StagePlanEntry[],
      validation: validationOf(res),
      gate: gateOf(res.gate),
    }
  }

  /**
   * `line.run` 是唯一异步的一条；结果直接回给调用方（卡片自己管状态）。
   *
   * 超时**由预演算出来**，不拍脑袋：产线是「逐阶段同步」，三段各 p90 加起来就是
   * 这条请求的真实寿命。给一个固定 120 s，等于保证长产线一定"前端超时、后端还在跑"
   * ——那是用户反馈过的那类"确认后什么都没发生"。
   * `queue:false`：这是有副作用的写型调用，连接失效时绝不自动重放。
   */
  async function run(input: {
    spec: LineSpec
    modeKey: string
    confirmed: boolean
    sessionKey?: string
    predictedP90Ms?: number
  }): Promise<Record<string, unknown>> {
    const predicted = input.predictedP90Ms ?? 0
    const timeoutMs = Math.min(30 * 60_000, Math.max(180_000, Math.ceil(predicted * 2) + 60_000))
    return (
      (asRecord(
        await rpc.call<Record<string, unknown>>(
          'line.run',
          {
            spec: input.spec,
            mode: input.modeKey,
            confirmed: input.confirmed,
            ...(input.sessionKey ? { sessionKey: input.sessionKey } : {}),
          },
          { timeoutMs, queue: false, timeoutAction: 'reject' },
        ),
      )) ?? {}
    )
  }

  async function retro(runId: string, trials?: number): Promise<Record<string, unknown> | null> {
    return asRecord(
      await rpc.call<Record<string, unknown>>('line.retro', {
        runId,
        ...(trials ? { trials } : {}),
      }),
    )
  }

  return {
    catalog,
    loading,
    error,
    available,
    planAvailable,
    limits,
    patternIds,
    patternMeta,
    patternLabel,
    rosterKindOf,
    labelOf,
    load,
    compile,
    preflight,
    plan,
    run,
    retro,
  }
})

/** 供组件复用的只读守卫：数组里只保留合法 pattern 的键。 */
export function filterPatterns(value: unknown): LinePattern[] {
  return strings(value).filter(isLinePattern)
}

export { LINE_PATTERNS }
