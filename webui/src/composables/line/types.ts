// 对话产线（Conversation Line）—— 前端契约镜像。
//
// 唯一事实源 = `core/src/line/` + `core/src/gateway/line.rs`
// + `docs/plans/2026-10-30-conversation-line-workflow.md` §3/§4。
// 本文件只做**类型镜像**，不放任何展示常量（角色集 / 团队 / 切面 / 上限一律来自 `line.catalog`）：
// 在前端抄一份常量，就是设计文档 §1.1 那条 5-Why 根因本身——两份描述必然漂移。
//
// 镜像纪律：这里的每一个字段都必须能在 Rust 侧 `json!` / `Serialize` 里指到具体行。
// 反例（本文件第一版踩过）：把 `verdict` 写成 `p90MsActual`、把 `errors` 当成顶层字段，
// 于是界面"读得到类型、读不到值"，缺陷静默潜伏。改 Rust 返回结构时必须同步改这里。
//
// 裁决/事前验尸/未知先验这些形状**复用** `@/utils/tactics/model`：产线的预演就是
// `tactics::plan` 的产物，重新声明一遍只会给漂移开第二道口子。
import type {
  PremortemItemView,
  RationaleView,
  ReplayReportView,
  TacticResultView,
  UnknownView,
} from '@/utils/tactics/model'

/** 产品名形状；与 `tactics::Topology` 一比一（solo↔Solo、fanout↔Star、panel↔Panel、swarm↔Swarm、relay↔Relay）。 */
export type LinePattern = 'solo' | 'fanout' | 'panel' | 'swarm' | 'relay'

export const LINE_PATTERNS: readonly LinePattern[] = ['solo', 'fanout', 'panel', 'swarm', 'relay']

/**
 * 名册该填什么 —— 由后端 `LinePattern::roster_kind()` 裁决，前端只照做。
 *
 * 这条字段的存在理由：`panel` 的名册进的是 `teams`（专家团 id），`fanout`/`relay`
 * 进的是 `agents`（角色名）。两者混用就是"选了却解析不到"，而引擎的旧行为是
 * 静默降级成自动检测 —— 同一个界面控件喂两种 id 必然复发。
 */
export type RosterKind = 'teams' | 'roles' | 'none'

/** 旧五挡（`orchestrate.run` 的 `form`），在本设计里降级为"一个阶段的简写"。 */
export type LegacyOrchestrationForm = 'single' | 'subagent' | 'team' | 'expert' | 'swarm'

/** 一个阶段 = 一个形状 + 它的宽度与名册。串行阶段之间只增量传递上一阶段的产物。 */
export interface LineStage {
  id: string
  pattern: LinePattern
  /** solo 恒 1；fanout/panel/swarm/relay = 并行规模或链长（名册语义看 `rosterKind`）。 */
  width: number
  /** `rosterKind`：teams ⇒ 专家团 id；roles ⇒ 角色席位名；none ⇒ 留空。 */
  roster: string[]
  /** 仅 swarm；缺省时后端取 `SWARM_FACETS` 前 `width` 个（切面表在引擎里，前端不复制）。 */
  facets: string[]
  /** 本阶段末尾是否做一次 lead 综合。旧 `subagent` 挡位为 false（保持既有"只并列不综合"语义）。 */
  synthesize: boolean
  note: string
}

/** 判据（与 `tactics::Criteria` 同形，camelCase 由 serde `rename_all` 保证）。 */
export interface LineCriteria {
  maxFailProbability: number
  budgetTokens: number | null
  deadlineMs: number | null
}

/** 整条产线的规格：可编辑、可编译、可回溯，三者同一个对象。 */
export interface LineSpec {
  specId: string
  goal: string
  /** 逐条可判真的验收条款——"对话的产物是断言链，不是文字流"的落点。 */
  acceptance: string[]
  stages: LineStage[]
  /** 阈值只有判据这一处：`budgetTokens`/`deadlineMs` 直接喂 `tactics` 的 kill-check。 */
  criteria: LineCriteria
}

/** 后端 `check()` 的合并结论：结构校验 + 名册可解析性校验。 */
export interface LineValidation {
  valid: boolean
  errors: string[]
  warnings: string[]
  /** 引擎当时的并发闸门；宽度超过它是 warning，不是 error。 */
  maxConcurrent: number
}

/** 一条断言（预研/风险/结论），带出处与异议标记。 */
export interface LineClaim {
  claim: string
  kind: 'finding' | 'decision' | 'risk' | 'question'
  evidence: string[]
  by: { stage: string; member: string }
  /** 与同阶段多数结论措辞相近却相反时保留下来的少数派（反极化）。 */
  dissent: boolean
  /**
   * 恒为 `null`：验收结论属于 `harness.ledger.verdict`，产线不自造。
   * 前端因此**不得**把这一列渲染成"已验收"，只能显示"待判"。
   */
  accepted: boolean | null
}

/** `line::Preflight`（camelCase）里的 `total`。 */
export interface LineTotalEstimate {
  p50Ms: number
  p90Ms: number
  p50Tokens: number
  p90Tokens: number
  p90CostUsd: number
  /** 阶段串行 ⇒ 成功率相乘，不是取平均。 */
  successRate: number
  failProbability: number
}

export interface StageEstimate extends TacticResultView {
  index: number
  pattern: LinePattern
  width: number
}

export type GateDecision = 'allow' | 'confirm' | 'deny'

export interface LineGate {
  autonomy: string
  decision: GateDecision
  reason: string
}

/** `line.preflight` 的返回 = `line::Preflight` + 网关补的 `shape/validation/gate`。 */
export interface LinePreflight {
  ok: boolean
  shape: string
  stages: TacticResultView[]
  total: LineTotalEstimate
  rationale: RationaleView
  premortem: PremortemItemView[]
  /** 非空 = 这些数字没有实测支撑，前端必须显式说出来，不能只拿来排序。 */
  unknowns: UnknownView[]
  /** 自选形状串（如 `swarm+panel+solo`）；`ok:false` 时为 null。 */
  chosen: string | null
  chosenVsRecommended: string | null
  validation: LineValidation
  gate: LineGate
  error?: string | null
  code?: string
}

/** `line.plan` 的逐阶段条目：`backend/args` 由 `route_meta` 生成，声明的与执行的是同一份。 */
export interface StagePlanEntry {
  index: number
  pattern: LinePattern
  width: number
  form?: string
  backend: string
  args: Record<string, unknown>
  roster: string[]
  note: string
  autonomy: string
}

export interface LinePlan {
  ok: boolean
  shape: string
  spec: LineSpec
  stages: StagePlanEntry[]
  validation: LineValidation
  gate: LineGate
  code?: string
}

export type StageRunStatus = 'ok' | 'failed'

/** `line.run` 的逐阶段回报（同时也是事件帧之外的对账真相）。 */
export interface LineStageRun {
  index: number
  id: string
  pattern: LinePattern
  status: StageRunStatus
  backend: string
  reply: string
  claims: LineClaim[]
  elapsedMs: number | null
  error: string | null
}

export interface LineRunVerdict {
  ok: boolean
  failures: string[]
  /** 各阶段实测耗时**相加**。 */
  elapsedMsActual: number
  /** 预演 p90，用于 `line.retro` 的偏差对照。 */
  elapsedMsPredicted: number
  tokensPredicted: number
  successRatePredicted: number
}

export interface LineStorage {
  persisted: boolean
  path?: string
  bytes?: number
  pruned?: number
  error?: string
}

export interface LineRunResult {
  runId: string
  sessionKey: string
  at: string
  shape: string
  spec: LineSpec
  stages: LineStageRun[]
  verdict: LineRunVerdict
  preflight: LinePreflight
  gate: LineGate
  storage: LineStorage
}

/** `line.retro` = `tactics::replay` 的整份报告 + 产线自己的对照面。 */
export interface LineRetro extends ReplayReportView {
  ok: boolean
  runId: string
  measuredVsPredicted: {
    elapsedMsActual: number | null
    elapsedMsPredicted: number | null
    successRatePredicted: number | null
    /** false = 有阶段失败，实测耗时天然更短，不能折成一句"预测很准"。 */
    comparable: boolean
  }
  claims: LineClaim[]
  /** 复盘 → 下次派单的先验回写建议（给建议，不自动改判据）。 */
  nextPreflightHint: string
  error?: string
  code?: string
}

export interface LineLimits {
  maxStages: number
  maxWidth: number
  subagentMaxConcurrent: number
  runKeep: number
  dirBudgetBytes: number
}

export interface LinePatternMeta {
  id: LinePattern
  label: string
  description: string
  rosterKind: RosterKind
  needsSynthesis: boolean
  /** 对应的 `tactics::Topology` 键（solo/star/panel/swarm/relay）。 */
  topology: string
}

export interface LineRoleCandidate {
  /** 席位名，即进 `roster` 的那个字符串。 */
  id: string
  label: string
  /** 后端 `role_key_for` 解析到的人设 key。 */
  agentKey: string
  /** true ⇒ 没匹配到内建人设，按席位名执行。界面必须如实标出来，不能装作命中。 */
  personaFallback: boolean
}

export interface LineTeamCandidate {
  id: string
  label: string
  roles: string[]
}

export interface LineFacet {
  id: string
  instruction: string
}

export interface LineRecipe {
  id: string
  label: string
  /** 引擎直接给出的阶段序列（配方在 Rust 侧，前端不猜、不复制）。 */
  stages: LineStage[]
}

export interface LineLegacyFormMeta {
  id: LegacyOrchestrationForm
  label: string
  /** 兼容表：这一挡编译成的阶段序列。 */
  stages: LinePattern[]
}

export interface LineCatalogPayload {
  ok: boolean
  patterns: LinePatternMeta[]
  recipes: LineRecipe[]
  legacyForms: LineLegacyFormMeta[]
  teams: LineTeamCandidate[]
  roles: LineRoleCandidate[]
  facets: LineFacet[]
  limits: LineLimits
  defaults: { criteria: LineCriteria; trials: number; maxTrials: number }
}

export interface LineCatalog {
  loaded: boolean
  patterns: LinePatternMeta[]
  recipes: LineRecipe[]
  legacyForms: LineLegacyFormMeta[]
  roles: LineRoleCandidate[]
  teams: LineTeamCandidate[]
  facets: LineFacet[]
  limits: LineLimits | null
  defaults: LineCatalogPayload['defaults'] | null
}

/** 后端返回未知 pattern 时整条丢弃的守卫集合。 */
export function isLinePattern(value: unknown): value is LinePattern {
  return typeof value === 'string' && (LINE_PATTERNS as readonly string[]).includes(value)
}
