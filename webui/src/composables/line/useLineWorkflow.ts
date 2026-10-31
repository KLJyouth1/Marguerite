// 对话产线的页面状态机：形状（`spec`）是唯一事实源。
//
// 本文件是 chat 页里唯一持有产线状态的地方：`LineComposer.vue` 拿到 `spec` 写回
// `setSpec`，`ChatView.vue` 只读派生量并调 `dispatch`。任何"再开一个 ref 描述同一个
// 旋钮"的做法就是把 §1.1 那条根因请回来（旧编排路径的五份副本正是黑盒之源）。
//
// 文案纪律：这里**不产出任何面向用户的字符串**。后端/校验原文如实透传，纯本地条件
// 一律返回大写 CODE（`LINE_NOT_ARMED` 等），由 UI 层翻译。
import { computed, getCurrentInstance, onBeforeUnmount, ref, watch } from 'vue'
import type { ComputedRef, Ref } from 'vue'

import type { RpcClientError } from '@/lib/rpc'
import { useLineCatalog } from './catalog'
import {
  applyRecipe,
  fromLegacyForm,
  loadStoredLineSpec,
  persistLineSpec,
  summarizeShape,
  validateLocal,
} from './lineSpec'
import type {
  GateDecision,
  LegacyOrchestrationForm,
  LineCatalog,
  LinePattern,
  LinePlan,
  LinePreflight,
  LineRetro,
  LineRunResult,
  LineSpec,
  LineStageRun,
} from './types'

const ARMED_STORAGE_KEY = 'marguerite.line.armed.v1'
const PREFLIGHT_DEBOUNCE_MS = 400

export type LineDispatchOutcome =
  | { status: 'ok'; run: LineRunResult }
  | { status: 'needs-confirm'; reason: string }
  | { status: 'denied'; reason: string }
  | { status: 'invalid'; reason: string }
  | { status: 'error'; message: string }

export interface LineWorkflow {
  spec: Ref<LineSpec>
  armed: Ref<boolean>
  running: Ref<boolean>
  catalog: ComputedRef<LineCatalog>
  catalogLoading: ComputedRef<boolean>
  catalogError: ComputedRef<string | null>
  catalogAvailable: ComputedRef<boolean>
  preflight: Ref<LinePreflight | null>
  preflightLoading: Ref<boolean>
  preflightError: Ref<string | null>
  plan: Ref<LinePlan | null>
  planLoading: Ref<boolean>
  planError: Ref<string | null>
  errors: ComputedRef<string[]>
  warnings: ComputedRef<string[]>
  blockingErrors: ComputedRef<string[]>
  gateDecision: ComputedRef<GateDecision | 'unknown'>
  canRun: ComputedRef<boolean>
  shapeText: ComputedRef<string>
  /** 输入框里的本轮目标（只读镜像，不落盘）；空串表示"没有草稿"。 */
  draftGoal: Ref<string>
  setDraftGoal(value: string): void
  lastRun: Ref<LineRunResult | null>
  lastRunError: Ref<string | null>
  retro: Ref<LineRetro | null>
  retroLoading: Ref<boolean>
  setSpec(next: LineSpec): void
  applyRecipeById(recipeId: string): boolean
  armNextFromLegacy(
    knobs?: { agents?: string[]; teams?: string[]; swarmSize?: number },
    form?: LegacyOrchestrationForm,
  ): boolean
  refreshPreflight(): Promise<void>
  dispatch(goal: string, opts: { confirmed: boolean }): Promise<LineDispatchOutcome>
  loadRetro(runId: string): Promise<void>
}

function readStoredArmed(): boolean {
  try {
    return globalThis.localStorage?.getItem(ARMED_STORAGE_KEY) === 'true'
  } catch {
    return false
  }
}

/** 隐私模式 / 配额满 ⇒ 静默：偏好丢了不能影响发送。 */
function writeStoredArmed(value: boolean): void {
  try {
    globalThis.localStorage?.setItem(ARMED_STORAGE_KEY, value ? 'true' : 'false')
  } catch {
    // 保持内存态。
  }
}

function asRecord(value: unknown): Record<string, unknown> | null {
  return value && typeof value === 'object' ? (value as Record<string, unknown>) : null
}

function str(value: unknown): string {
  return typeof value === 'string' ? value : ''
}

function numOr(value: unknown, fallback: number): number {
  return typeof value === 'number' && Number.isFinite(value) ? value : fallback
}

function strArray(value: unknown): string[] {
  return Array.isArray(value) ? value.filter((x): x is string => typeof x === 'string') : []
}

/** 后端将来加第四种裁决时，前端按"未知"处理而不是当成放行。 */
function isGateDecision(value: unknown): value is GateDecision {
  return value === 'allow' || value === 'confirm' || value === 'deny'
}

/** 没有预演就是没有预演：占位值全部为"零/未知"，绝不拿猜出来的数字去排 UI。 */
function inertPreflight(): LinePreflight {
  return {
    ok: false,
    shape: '',
    stages: [],
    total: {
      p50Ms: 0,
      p90Ms: 0,
      p50Tokens: 0,
      p90Tokens: 0,
      p90CostUsd: 0,
      successRate: 0,
      failProbability: 1,
    },
    rationale: {},
    premortem: [],
    unknowns: [],
    chosen: null,
    chosenVsRecommended: null,
    validation: { valid: false, errors: [], warnings: [], maxConcurrent: 0 },
    gate: { autonomy: 'unknown', decision: 'deny', reason: '' },
    error: 'LINE_PREFLIGHT_MISSING',
  }
}

export function useLineWorkflow(
  sessionKey: { readonly value: string },
  modeKey: { readonly value: string },
): LineWorkflow {
  const store = useLineCatalog()

  const spec = ref<LineSpec>(loadStoredLineSpec()) as Ref<LineSpec>
  /**
   * 草稿 = 输入框里那一句话，也就是**本轮的目标**。
   *
   * 这不是"第二个描述同一旋钮的 ref"：`spec.goal` 记的是上一次真正派出去的目标
   * （随形状一起落盘），`draftGoal` 记的是还没发出去的那句话。产线卖的是"这一轮
   * 由几种形状去跑"，而"这一轮要做什么"本来就写在输入框里 —— 界面上没有第二个
   * 目标输入框，所以校验/预演读的是**草稿优先**的合成本，否则 `goal 不可为空`
   * 永远解不开，挂线开关就成了一个按不动的按钮（2026-10-31 真机实测踩到）。
   */
  const draftGoal = ref('')
  const specWithDraft = computed<LineSpec>(() => {
    const trimmed = draftGoal.value.trim()
    return trimmed && trimmed !== spec.value.goal ? { ...spec.value, goal: trimmed } : spec.value
  })
  const armed = ref(readStoredArmed())
  const running = ref(false)
  const preflight = ref<LinePreflight | null>(null)
  const preflightLoading = ref(false)
  const preflightError = ref<string | null>(null)
  const plan = ref<LinePlan | null>(null)
  const planLoading = ref(false)
  const planError = ref<string | null>(null)
  const lastRun = ref<LineRunResult | null>(null)
  const lastRunError = ref<string | null>(null)
  const retro = ref<LineRetro | null>(null)
  const retroLoading = ref(false)

  const catalog = computed(() => store.catalog)
  const catalogLoading = computed(() => store.loading)
  const catalogAvailable = computed(() => store.available)
  const catalogError = computed(() =>
    catalogAvailable.value ? store.error : 'LINE_CATALOG_UNAVAILABLE',
  )

  const local = computed(() => validateLocal(specWithDraft.value, catalog.value))
  const errors = computed(() => local.value.errors)

  const warnings = computed(() =>
    Array.from(
      new Set([...local.value.warnings, ...(preflight.value?.validation?.warnings ?? [])]),
    ),
  )

  /** 拦下派单的只有两类：本地校验（含名册填错类）与后端 `validation.errors`。 */
  function blockingFor(target: LineSpec): string[] {
    const remote = preflight.value && preflight.value.ok === false
      ? preflight.value.validation?.errors ?? []
      : []
    return Array.from(new Set([...validateLocal(target, catalog.value).errors, ...remote]))
  }
  const blockingErrors = computed(() => blockingFor(specWithDraft.value))

  const gateDecision = computed<GateDecision | 'unknown'>(() => {
    const raw = preflight.value?.gate?.decision
    return isGateDecision(raw) ? raw : 'unknown'
  })

  function patternLabel(pattern: LinePattern): string {
    return catalog.value.patterns.find((item) => item.id === pattern)?.label ?? pattern
  }
  const shapeText = computed(() => summarizeShape(spec.value, patternLabel))

  const canRun = computed(
    () => armed.value && catalogAvailable.value && !running.value && blockingErrors.value.length === 0,
  )

  function setSpec(next: LineSpec): void {
    spec.value = next
    persistLineSpec(next)
  }

  /**
   * 输入框 → 本轮目标。ChatView 的草稿 watcher 是唯一调用方。
   *
   * 未挂线时**不**为草稿打预演：那时这条线永远不会跑，数字是装饰，而"每敲一段
   * 停一下就来两趟 RPC"是拿用户的键盘去敲网关。挂线本身会补一次刷新（见装配段）。
   */
  function setDraftGoal(value: string): void {
    if (draftGoal.value === value) return
    draftGoal.value = value
    if (armed.value) schedulePreflight()
  }

  /* ── 预演 + 执行映射：派生 + 防抖 + 乱序丢弃 ─────────────────────────── */

  let preflightSeq = 0
  let debounceTimer: ReturnType<typeof setTimeout> | null = null

  function cancelDebounce(): void {
    if (debounceTimer !== null) {
      clearTimeout(debounceTimer)
      debounceTimer = null
    }
  }

  /** 让在途的预演响应作废：形状已改（或本地已拦下），晚到的旧结果不得落地。 */
  function invalidatePreflight(next: LinePreflight | null): void {
    preflightSeq += 1
    preflight.value = next
    preflightLoading.value = false
    invalidatePlan()
  }

  /** 计划与预演讲的都是**同一个形状**：形状失效时旧映射必须一起清掉，不能留在卡上。 */
  function invalidatePlan(): void {
    plan.value = null
    planError.value = null
    planLoading.value = false
  }

  /**
   * `line.plan`：逐阶段"声明的形状 → 真正会被调用的东西"。
   *
   * 它**不 await、不共享成败**：带数字的那份预测是预演，计划只是配套细节，
   * 慢、失败、甚至网关根本没声明这一条，都不许把预演表清空或拖住挂线。
   * 乱序保护沿用同一个 `preflightSeq` 代号 —— 两者描述同一形状，一个代号够用，
   * 而且形状一变，两条在途响应会一起变成"过期"，不会留下一新一旧拼起来读。
   */
  function runPlan(target: LineSpec, seq: number): void {
    if (!store.planAvailable) {
      invalidatePlan()
      // 只记账、不渲染：没有这一条 RPC 时"执行映射"整段消失，界面不出现空壳。
      planError.value = 'LINE_PLAN_UNAVAILABLE'
      return
    }
    planLoading.value = true
    void store
      .plan(target, modeKey.value)
      .then((res) => {
        if (seq !== preflightSeq) return
        plan.value = res
        planError.value = res === null ? 'LINE_PLAN_EMPTY' : null
      })
      .catch((err: unknown) => {
        if (seq !== preflightSeq) return
        plan.value = null
        planError.value = err instanceof Error ? err.message : String(err)
      })
      .finally(() => {
        if (seq === preflightSeq) planLoading.value = false
      })
  }

  async function runPreflight(target: LineSpec): Promise<void> {
    const seq = ++preflightSeq
    preflightLoading.value = true
    runPlan(target, seq)
    try {
      const res = await store.preflight(target, modeKey.value)
      if (seq !== preflightSeq) return
      preflight.value = res
      preflightError.value = res === null ? 'LINE_PREFLIGHT_EMPTY' : (res.error ?? null)
    } catch (err) {
      if (seq !== preflightSeq) return
      preflight.value = null
      preflightError.value = err instanceof Error ? err.message : String(err)
    } finally {
      if (seq === preflightSeq) preflightLoading.value = false
    }
  }

  async function refreshPreflight(): Promise<void> {
    cancelDebounce()
    if (!catalogAvailable.value) {
      invalidatePreflight(null)
      return
    }
    // 本地已经拦下的形状后端也一定拒 —— 不打这一趟，但本地错误照样显示（errors）。
    if (validateLocal(specWithDraft.value, catalog.value).errors.length > 0) {
      invalidatePreflight(null)
      preflightError.value = null
      return
    }
    await runPreflight(specWithDraft.value)
  }

  function schedulePreflight(): void {
    cancelDebounce()
    debounceTimer = setTimeout(() => {
      debounceTimer = null
      void refreshPreflight()
    }, PREFLIGHT_DEBOUNCE_MS)
  }

  /* ── 派单 ───────────────────────────────────────────────────────────── */

  function sendSpecFor(goal: string): LineSpec {
    const trimmed = goal.trim()
    if (!trimmed || trimmed === spec.value.goal) return spec.value
    return { ...spec.value, goal: trimmed }
  }

  function normalizeRun(raw: Record<string, unknown>, sent: LineSpec): LineRunResult {
    const storage = asRecord(raw.storage)
    const verdict = asRecord(raw.verdict)
    const echoed = asRecord(raw.spec)
    const echoedPreflight = asRecord(raw.preflight)
    const gate = asRecord(raw.gate)
    const decision = gate?.decision
    return {
      runId: str(raw.runId),
      sessionKey: str(raw.sessionKey) || sessionKey.value,
      at: str(raw.at) || new Date().toISOString(),
      shape: str(raw.shape) || shapeText.value,
      spec: echoed ? (raw.spec as LineSpec) : sent,
      stages: (Array.isArray(raw.stages) ? raw.stages : []) as LineStageRun[],
      verdict: {
        ok: verdict?.ok === true,
        failures: strArray(verdict?.failures),
        elapsedMsActual: numOr(verdict?.elapsedMsActual, 0),
        elapsedMsPredicted: numOr(verdict?.elapsedMsPredicted, 0),
        tokensPredicted: numOr(verdict?.tokensPredicted, 0),
        successRatePredicted: numOr(verdict?.successRatePredicted, 0),
      },
      // 与 `catalog.ts` 同一口径：网关回包即 `types.ts` 的镜像，缺字段时如实说"没有预演"。
      preflight: echoedPreflight
        ? (echoedPreflight as unknown as LinePreflight)
        : (preflight.value ?? inertPreflight()),
      gate: {
        autonomy: str(gate?.autonomy) || 'unknown',
        decision: isGateDecision(decision) ? decision : 'deny',
        reason: str(gate?.reason),
      },
      storage: {
        persisted: storage?.persisted === true,
        ...(str(storage?.path) ? { path: str(storage?.path) } : {}),
        ...(typeof storage?.bytes === 'number' ? { bytes: storage.bytes } : {}),
        ...(typeof storage?.pruned === 'number' ? { pruned: storage.pruned } : {}),
        ...(str(storage?.error) ? { error: str(storage?.error) } : {}),
      },
    }
  }

  async function dispatch(goal: string, opts: { confirmed: boolean }): Promise<LineDispatchOutcome> {
    if (!armed.value) return { status: 'error', message: 'LINE_NOT_ARMED' }
    if (!catalogAvailable.value) return { status: 'error', message: 'LINE_CATALOG_UNAVAILABLE' }

    const sent = sendSpecFor(goal)
    const blocking = blockingFor(sent)
    // 先拒后花：拦下的是 token 消耗，不是等后端跑完再告诉用户白烧了。
    if (blocking.length > 0) return { status: 'invalid', reason: blocking.join('；') }

    running.value = true
    lastRunError.value = null
    try {
      const raw = await store.run({
        spec: sent,
        modeKey: modeKey.value,
        confirmed: opts.confirmed,
        sessionKey: sessionKey.value,
        // 超时由预演算出（store 据此推导 timeout）：固定超时等于保证长产线
        // "前端超时、后端还在跑"，那是用户看到的那类"确认后什么都没发生"。
        predictedP90Ms: preflight.value?.total?.p90Ms,
      })
      const run = normalizeRun(raw, sent)
      if (!run.runId) return { status: 'error', message: 'LINE_RUN_UNEXPECTED_RESPONSE' }
      lastRun.value = run
      retro.value = null
      // 派出去的目标写回形状：发送后输入框会清空，`specWithDraft` 随即回落到
      // `spec.goal` —— 不写回的话，回执刚摆出来"还不能派单：goal 不可为空"就自己
      // 打了自己的脸。这一轮的目标确实已经成立，落盘它不是副作用，是对账。
      if (sent !== spec.value) setSpec(sent)
      return { status: 'ok', run }
    } catch (err) {
      const code = (err as RpcClientError | undefined)?.code
      const message = err instanceof Error ? err.message : String(err)
      // 失败不改 armed / spec：组合原样留着，用户改一处就能重发。
      if (code === 'ORCHESTRATE_CONFIRM_REQUIRED') return { status: 'needs-confirm', reason: message }
      if (code === 'ORCHESTRATE_DENIED') {
        lastRunError.value = message
        return { status: 'denied', reason: message }
      }
      if (code === 'INVALID_SPEC') return { status: 'invalid', reason: message }
      lastRunError.value = message
      return { status: 'error', message }
    } finally {
      running.value = false
    }
  }

  /* ── 配方 / 旧挡位 ──────────────────────────────────────────────────── */

  function applyRecipeById(recipeId: string): boolean {
    const recipe = catalog.value.recipes.find((item) => item.id === recipeId)
    if (!recipe) return false
    // 配方一条阶段都解析不出来 ⇒ 后端契约变了，宁可不套也不把用户的产线清成 solo。
    const result = applyRecipe(spec.value, recipe.stages)
    if (!result.applied) return false
    setSpec(result.spec)
    return true
  }

  function legacyFormKnown(form: LegacyOrchestrationForm): boolean {
    const declared = catalog.value.legacyForms
    // 目录未给出兼容表时交给 `fromLegacyForm` 的分支裁决（它只认旧五挡）。
    if (declared.length === 0) return true
    return declared.some((item) => item.id === form)
  }

  function armNextFromLegacy(
    knobs: { agents?: string[]; teams?: string[]; swarmSize?: number } = {},
    form: LegacyOrchestrationForm = 'single',
  ): boolean {
    if (!legacyFormKnown(form)) return false
    // 兼容表声称有这一挡、但 `fromLegacyForm` 表达不了（后端契约变更）时如实拒绝。
    const legacy: LineSpec | null = fromLegacyForm(form, knobs)
    if (!legacy || legacy.stages.length === 0) return false
    setSpec({
      ...legacy,
      specId: spec.value.specId,
      goal: spec.value.goal,
      acceptance: [...spec.value.acceptance],
      criteria: { ...spec.value.criteria },
    })
    armed.value = true
    return true
  }

  /* ── 复盘 ───────────────────────────────────────────────────────────── */

  async function loadRetro(runId: string): Promise<void> {
    const last = lastRun.value
    if (last && last.runId === runId && last.storage.persisted === false) {
      // 记录没落盘 ⇒ 后端必然找不到，不发这一趟，直接把如实原因摆出来。
      retro.value = null
      lastRunError.value = last.storage.error ?? 'LINE_RUN_NOT_PERSISTED'
      return
    }
    retroLoading.value = true
    try {
      const raw = await store.retro(runId)
      if (!raw || raw.ok === false) {
        retro.value = null
        lastRunError.value = str(raw?.error) || 'LINE_RETRO_UNAVAILABLE'
        return
      }
      retro.value = raw as unknown as LineRetro
    } catch (err) {
      retro.value = null
      lastRunError.value = err instanceof Error ? err.message : String(err)
    } finally {
      retroLoading.value = false
    }
  }

  /* ── 装配：目录一次加载、形状变化 → 落盘 + 防抖预演 ─────────────────── */

  watch(armed, (value) => {
    writeStoredArmed(value)
    // 挂线那一刻，草稿才从"普通对话的输入"升格为"本轮目标"：补一次刷新，
    // 否则用户要先改一个旋钮才看得到数字。
    if (value) schedulePreflight()
  })

  watch(
    [spec, () => catalog.value.loaded],
    () => {
      persistLineSpec(spec.value)
      schedulePreflight()
    },
    { deep: true },
  )

  void store.load().finally(() => schedulePreflight())

  /**
   * 上面那一次加载有握手竞态：`useLineWorkflow` 跑在 ChatView 的 setup 里，而形状目录是否
   * 存在取决于 WS `hello` 帧里的方法表 —— 帧还没落地时 `supportsMethod('line.catalog')` 是
   * false，`store.load()` 直接放弃并把错误锁成「line.catalog 未被网关声明」。方法表随后
   * 落地、可用性翻正，却没有人再叫一次 load。真机表现（2026-10-31）：同一条产线刷新两次，
   * 一次形状与预设齐全、一次整套目录空掉，还挂着一句"目录不可用"。
   * 补的就是这一步：可用性一翻正且目录仍然是空的，重新加载一次。
   */
  watch(catalogAvailable, (value) => {
    if (value && !catalog.value.loaded && !catalogLoading.value) {
      void store.load().finally(() => schedulePreflight())
    }
  })

  const dispose = (): void => {
    cancelDebounce()
    // 卸载后在途的预演不得再写状态（seq 前进即丢弃）。
    invalidatePreflight(preflight.value)
  }
  if (getCurrentInstance()) onBeforeUnmount(dispose)

  return {
    spec,
    armed,
    running,
    catalog,
    catalogLoading,
    catalogError,
    catalogAvailable,
    preflight,
    preflightLoading,
    preflightError,
    plan,
    planLoading,
    planError,
    errors,
    warnings,
    blockingErrors,
    gateDecision,
    canRun,
    shapeText,
    draftGoal,
    setDraftGoal,
    lastRun,
    lastRunError,
    retro,
    retroLoading,
    setSpec,
    applyRecipeById,
    armNextFromLegacy,
    refreshPreflight,
    dispatch,
    loadRetro,
  }
}
