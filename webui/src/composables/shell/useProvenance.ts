/**
 * 记忆溯源（Provenance Drawer）的数据层 —— `assistant.brief.get` 契约的唯一消费点。
 *
 * ## 5 Why：为什么需要这个抽屉
 *
 * 1. 用户看不到智能体「记住了什么」——聊天界面只呈现回答，记忆是黑箱。
 * 2. 更糟的是看不到「为什么记得」与「为什么某条被丢掉」——`assistant.brief.get`
 *    已经在返回 `projection.dropped`（key + reason），但前端此前没有一处展示它。
 * 3. 结果是用户无法判断回答的依据是否可信：一条被 `low_score` 丢掉的事实，
 *    与一条根本没被提取过的事实，在 UI 上看起来完全一样。
 * 4. 记忆因此不可审计：用户无法纠正（「这条别记」「这条为什么没进来」），
 *    只能反复重问，把不确定性变成额外轮次。
 * 5. 所以把「记忆投影」提升为可检查的一等公民：分区展示钉住/偏好/事实/被丢弃
 *    的事实与原因，并在网关没提供投影明细时给出**诚实的空态**。
 *
 * 设计纪律：
 * - **门控**：先 `supportsMethod('assistant.brief.get')`，网关没声明该方法就完全
 *   不发请求，状态落 `unsupported`（不伪造一份「空简报」）。
 * - **不伪造**：`projection` 缺失时 `hasProjection=false`，UI 显示「网关未提供投影
 *   明细」，**绝不**用空数组假装「没有丢弃」。
 * - **类型收敛**：`groupProvenance` 把 `unknown` 收敛成模板可安全直读的模型，
 *   字符串缺省 `''`、数字缺省 `0`，绝不让 `undefined` 泄漏到模板。
 * - **错误如实**：RPC reject 的原文保留在 `error`，UI 直接显示，不改写成
 *   「加载失败」这种无法排障的通用文案。
 */
import { ref, type Ref } from 'vue'
import { useRpcStore } from '@/stores/rpc'

export const PROVENANCE_METHOD = 'assistant.brief.get'
/** 可撤销性的落点：删掉一条**本引擎写回层**的事实（R-02）。 */
export const FACTS_DELETE_METHOD = 'assistant.facts.delete'

export type ProvenanceState = 'idle' | 'loading' | 'ready' | 'unsupported' | 'error'

export interface ProvenancePinned {
  text: string
  source: string
  at: string
}

export interface ProvenancePreference {
  key: string
  value: string
  confidence: number
  evidence: number
}

export interface ProvenanceFact {
  text: string
  source: string
  score: number
  /** 可撤销标识（`fact:<id>`）；空串表示这条不归本应用写，删不了。 */
  id: string
  /** 后端显式声明「可删」；**不靠 id 是否存在推断**，两侧同一口径。 */
  deletable: boolean
}

export interface ProvenanceDropped {
  key: string
  reason: string
}

export interface ProvenanceBudget {
  maxChars: number
  used: number
  notes: string
}

export interface ProvenanceModel {
  /** 线上原值（`global` / `project`）；空串表示网关未报告。 */
  scopeLabel: string
  workspace: string
  pinned: ProvenancePinned[]
  preferences: ProvenancePreference[]
  facts: ProvenanceFact[]
  dropped: ProvenanceDropped[]
  /** 投影策略原值；空串表示未报告。 */
  policy: string
  /** `projection` 字段是否真的在响应里 —— 决定 UI 显示明细还是诚实空态。 */
  hasProjection: boolean
  budget: ProvenanceBudget
}

export const EMPTY_PROVENANCE: ProvenanceModel = {
  scopeLabel: '',
  workspace: '',
  pinned: [],
  preferences: [],
  facts: [],
  dropped: [],
  policy: '',
  hasProjection: false,
  budget: { maxChars: 0, used: 0, notes: '' },
}

function asRecord(value: unknown): Record<string, unknown> | null {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : null
}

function asString(value: unknown): string {
  return typeof value === 'string' ? value : ''
}

function asNumber(value: unknown): number {
  return typeof value === 'number' && Number.isFinite(value) ? value : 0
}

function asArray(value: unknown): unknown[] {
  return Array.isArray(value) ? value : []
}

/** 纯函数：完整形状 / 缺 `projection` / 缺字段 / 非对象输入都归到同一个可读模型。 */
export function groupProvenance(raw: unknown): ProvenanceModel {
  const root = asRecord(raw)
  if (!root) return { ...EMPTY_PROVENANCE }

  const pinned = asArray(root.pinned)
    .map(asRecord)
    .filter((item): item is Record<string, unknown> => item !== null)
    .map(item => ({
      text: asString(item.text),
      source: asString(item.source),
      at: asString(item.at),
    }))

  const preferences = asArray(root.preferences)
    .map(asRecord)
    .filter((item): item is Record<string, unknown> => item !== null)
    .map(item => ({
      key: asString(item.key),
      value: asString(item.value),
      confidence: asNumber(item.confidence),
      evidence: asNumber(item.evidence),
    }))

  const facts = asArray(root.facts)
    .map(asRecord)
    .filter((item): item is Record<string, unknown> => item !== null)
    .map(item => ({
      text: asString(item.text),
      source: asString(item.source),
      score: asNumber(item.score),
      id: asString(item.id),
      // 只有后端明确写 `deletable: true` 才算可删。旧网关不带这个字段时
      // 一律当作不可删 —— 宁可不给按钮，也不给一个点了会失败的按钮。
      deletable: item.deletable === true,
    }))

  const projection = asRecord(root.projection)
  const dropped = projection
    ? asArray(projection.dropped)
        .map(asRecord)
        .filter((item): item is Record<string, unknown> => item !== null)
        .map(item => ({ key: asString(item.key), reason: asString(item.reason) }))
    : []

  const budget = asRecord(root.budget)

  return {
    scopeLabel: asString(root.scope),
    workspace: asString(root.workspace),
    pinned,
    preferences,
    facts,
    dropped,
    policy: projection ? asString(projection.policy) : '',
    hasProjection: projection !== null,
    budget: {
      maxChars: budget ? asNumber(budget.max_chars) : 0,
      used: budget ? asNumber(budget.used) : 0,
      notes: budget ? asString(budget.notes) : '',
    },
  }
}

export interface ProvenanceStateBundle {
  state: Ref<ProvenanceState>
  model: Ref<ProvenanceModel>
  error: Ref<string>
  /** 正在删除的事实 id（空串 = 没有进行中的删除），供按钮禁用。 */
  deleting: Ref<string>
  loadProvenance: () => Promise<void>
  deleteFact: (id: string) => Promise<boolean>
}

export function useProvenance(): ProvenanceStateBundle {
  const state = ref<ProvenanceState>('idle')
  const model = ref<ProvenanceModel>({ ...EMPTY_PROVENANCE })
  const error = ref('')
  const deleting = ref('')

  async function loadProvenance(): Promise<void> {
    if (state.value === 'loading') return
    const rpc = useRpcStore()

    if (!rpc.supportsMethod(PROVENANCE_METHOD)) {
      model.value = { ...EMPTY_PROVENANCE }
      error.value = ''
      state.value = 'unsupported'
      return
    }

    state.value = 'loading'
    error.value = ''
    try {
      const raw = await rpc.call<unknown>(PROVENANCE_METHOD, {})
      model.value = groupProvenance(raw)
      state.value = 'ready'
    } catch (err) {
      // 原文保留：网关的报错（未连接 / 方法不可用 / 参数错误）必须能被排障，
      // 改写成「加载失败」等于把可执行信息扔掉。
      model.value = { ...EMPTY_PROVENANCE }
      error.value = err instanceof Error ? err.message : String(err)
      state.value = 'error'
    }
  }

  /**
   * 忘掉一条事实。
   *
   * 三条纪律：
   * 1. **门控**：网关没声明 `assistant.facts.delete` 就一次请求都不发（旧网关不下发按钮，
   *    这里只是兜住时序竞态）。
   * 2. **不把业务失败当成功**：`prefs.*` 家族用 `{success:false, code}` 正常 resolve，
   *    不判它会把「没删掉」显示成删掉了。
   * 3. **成功后重取简报**：本地只删数组会让 `budget.used` 与 `projection` 变旧，
   *    用户看到的数字就不再是服务端的真相。
   */
  async function deleteFact(id: string): Promise<boolean> {
    if (!id || deleting.value) return false
    const rpc = useRpcStore()
    if (!rpc.supportsMethod(FACTS_DELETE_METHOD)) {
      error.value = `gateway 不支持 ${FACTS_DELETE_METHOD}`
      return false
    }
    deleting.value = id
    try {
      const res = await rpc.call<unknown>(FACTS_DELETE_METHOD, { id })
      const rec = asRecord(res)
      if (rec && rec.success === false) {
        error.value = asString(rec.message) || `${FACTS_DELETE_METHOD} failed`
        return false
      }
      await loadProvenance()
      return true
    } catch (err) {
      error.value = err instanceof Error ? err.message : String(err)
      return false
    } finally {
      deleting.value = ''
    }
  }

  return { state, model, error, deleting, loadProvenance, deleteFact }
}
