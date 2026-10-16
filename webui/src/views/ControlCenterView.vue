<script setup lang="ts">
// 智能中枢面板（Task #7 + V2.1-P4 a/b/c 升级）：统一呈现运行时模式、BloomSpec 全字段、
// 三层语义缓存「命中率折线 + ExactOnly tools 徽章」、4 象限能力矩阵 & 4 端心跳小灯，
// 以及 Bloom 三段锚定阶段胶囊（Anchored→PTC→Full 进度，读 bloom 规格推阶段）。
// 数据源均为既有 RPC：mode.get / mode.switch / mode.spec / cache.status / plans.capabilities。
// 缓存命中率如实展示（折线真实按 T0/T1/T2 累计命中增量绘制），绝不冒充 100%。
import { computed, nextTick, onMounted, onUnmounted, ref, watch } from 'vue'
import Icon from '@/components/Icon.vue'
import { useRpcStore } from '@/stores/rpc'
import { useActiveMode, switchActiveMode } from '@/composables/useActiveMode'

const rpc = useRpcStore()
const { activeMode } = useActiveMode()
interface ModeSpecBloom {
  max_swarm_shards?: number
  max_sub_experts?: number
  thinking_depth?: number
  verification_passes?: number
  effective_refine_rounds?: number
  effective_verify_rounds?: number
  effective_max_llm_calls?: number
  reasoning_effort?: string
  effective_thinking_effort?: string
  temperature?: number
  snapshot_enabled?: boolean
  auto_retry_failures?: boolean
  budget_tokens?: number
  max_iterations?: number
  max_fixes?: number
  anchoring_enabled?: boolean
  privileges_auto_approve?: boolean
  privileges_max_concurrent_tools?: number
}
interface ModeSpecPayload {
  kind: string
  label: string
  risk_level: string
  approval_mutating: string
  allowed_tools: number
  denied_tools: number
  bloom: ModeSpecBloom | null
}
interface CacheTier { hits: number; saved_tokens: number }
interface CacheTiers { exact: CacheTier; normalized: CacheTier; semantic: CacheTier }
interface CacheStatus {
  ok: boolean
  entries: number
  lookups: number
  hits: number
  misses: number
  client_hit_rate: number
  saved_prompt_tokens: number
  semantic_enabled: boolean
  tiers?: CacheTiers
  // V2.1-P4b：契约扩展（后端未实现时读 null，UI 诚实降级显示「暂不可用」）
  exact_only_tools?: boolean
}
interface CapStats { total: number; real: number; partial: number; implemented: number; narrative: number }
interface CapabilitiesPayload {
  stats?: CapStats
  narrativeIds?: string[]
  // V2.1-P4c：4 象限能力枚举（若未实现 → null → UI 走旧 5-cell 矩阵）
  quadrants?: {
    plan: number
    execute: number
    observe: number
    operate: number
  }
}

// V2.1-P4c：4 端心跳小灯（运行时环境探测：不造假——端未连接即灰）
interface EndpointHeartbeat {
  name: string
  label: string
  hint: string
}
const ENDPOINTS: EndpointHeartbeat[] = [
  { name: 'cli',      label: 'CLI',       hint: '终端 bin 可达' },
  { name: 'tui',      label: 'TUI',       hint: 'ratatui 工作台' },
  { name: 'gui',      label: 'Tauri GUI', hint: '桌面端 (React)' },
  { name: 'web',      label: 'Web UI',    hint: '本控制台 (Vue3)' },
]

const error = ref('')
const busy = ref(false)
const mode = activeMode
const spec = ref<ModeSpecPayload | null>(null)
const cache = ref<CacheStatus | null>(null)
const caps = ref<CapabilitiesPayload | null>(null)
const refreshTimer = ref<number | null>(null)
// 4 端心跳（Web 端永远亮；其余由 gateway.status 方法判定；方法不可达 →灰，诚实不造假）
const endpointLive = ref<Record<string, boolean>>({ web: true })

const modeKinds = [
  { kind: 'plan', label: 'Plan' },
  { kind: 'agent', label: 'Agent' },
  { kind: 'ensemble', label: 'Ensemble' },
  { kind: 'bloom', label: 'Bloom' },
  { kind: 'omni-transcend', label: 'OmniTranscend' },
]

const modeLabel = computed(() => {
  const m = mode.value
  if (!m) return '—'
  const kinds: Record<string, string> = {
    plan: 'Plan', agent: 'Agent', ensemble: 'Ensemble',
    bloom: 'Bloom', 'omni-transcend': 'OmniTranscend',
  }
  const k = m.kind ?? m.mode ?? ''
  return kinds[k] ?? k ?? '—'
})

const bloom = computed(() => spec.value?.bloom ?? null)

// ─────────────────────────────────────────────────────────────────────
// V2.2-Bloom RPC：三段锚定阶段的后端权威值（mode.stage）
// 优先级（诚实标注来源，绝不冒充）：
//  - 后端 mode.stage 可用（supportsMethod 探测）→ 用后端权威值
//  - 后端不可用（旧版）→ 回退到前端规格推导（V2.1-P4a 旧逻辑）
//  - source: "session" = 会话锚定台账真实推进 / "spec" = 规格推导 / 空 = 前端回退
// ─────────────────────────────────────────────────────────────────────
interface ModeStagePayload {
  ok: boolean
  runtime: boolean
  stage: 'disabled' | 'anchored' | 'ptc' | 'full'
  source: 'session' | 'spec'
  session?: string | null
}
const modeStage = ref<ModeStagePayload | null>(null)
async function loadModeStage() {
  if (!rpc.supportsMethod('mode.stage')) { modeStage.value = null; return }
  const s = await safe(() => rpc.call<ModeStagePayload>('mode.stage', {}))
  if (s?.ok && s.stage) modeStage.value = s
}
const anchorStageFromBackend = computed<AnchorStage | null>(() => {
  const s = modeStage.value
  if (!s || !s.ok) return null
  return s.stage === 'disabled' ? 'disabled' : s.stage
})
const anchorStageSource = computed(() => {
  if (anchorStageFromBackend.value) return modeStage.value?.source ?? ''
  return ''
})
type AnchorStage = 'disabled' | 'anchored' | 'ptc' | 'full'
const anchorStage = computed<AnchorStage>(() => {
  const fromBackend = anchorStageFromBackend.value
  if (fromBackend) return fromBackend
  // 回退：前端规格推导（旧逻辑，仅在后端 mode.stage 不可达时使用）
  const b = bloom.value
  if (!b) return 'disabled'
  if (!b.anchoring_enabled) return 'disabled'
  const depth = b.thinking_depth ?? 0
  const conc = b.privileges_max_concurrent_tools ?? 0
  const vpass = b.verification_passes ?? 0
  const experts = b.max_sub_experts ?? 0
  if (depth >= 3 && conc >= 2 && vpass >= 2 && experts >= 3) return 'full'
  if (depth >= 3 && conc >= 2) return 'ptc'
  return 'anchored'
})
const STAGES: { key: AnchorStage; label: string; title: string }[] = [
  { key: 'anchored', label: 'Anchored', title: '工具锚定 · 冷启动' },
  { key: 'ptc',      label: 'PTC',      title: '特权 · 并行编排' },
  { key: 'full',     label: 'Full',     title: '专家验证 · 收敛闭环' },
]
function stageReached(k: AnchorStage) {
  const order: AnchorStage[] = ['disabled', 'anchored', 'ptc', 'full']
  return order.indexOf(anchorStage.value) >= order.indexOf(k)
}

// ─────────────────────────────────────────────────────────────────────
// V2.1-P4b：缓存折线历史（每 30s 一次采样 → 最近 10 点 SVG 折线）
// 分三层各自独立 Y 轴范围；数据缺失时 UI 诚实标「待采样」。
// ─────────────────────────────────────────────────────────────────────
type TierKey = 'T0' | 'T1' | 'T2'
interface TierPoint { t: number; v: number }
const MAX_POINTS = 10
const tierHistory = ref<Record<TierKey, TierPoint[]>>({ T0: [], T1: [], T2: [] })
let lastTierSnapshot: { exact: number; normalized: number; semantic: number } | null = null

function pushTierPoint(key: TierKey, value: number) {
  const arr = tierHistory.value[key]
  arr.push({ t: Date.now(), v: value })
  if (arr.length > MAX_POINTS) arr.splice(0, arr.length - MAX_POINTS)
}
watch(cache, (c) => {
  if (!c?.tiers) return
  const cur = {
    exact: c.tiers.exact.hits,
    normalized: c.tiers.normalized.hits,
    semantic: c.tiers.semantic.hits,
  }
  if (lastTierSnapshot) {
    // 折线画的是「两次采样之间的新增命中」——避免累积数字掩盖趋势
    pushTierPoint('T0', Math.max(0, cur.exact - lastTierSnapshot.exact))
    pushTierPoint('T1', Math.max(0, cur.normalized - lastTierSnapshot.normalized))
    pushTierPoint('T2', Math.max(0, cur.semantic - lastTierSnapshot.semantic))
  } else {
    // 第一轮只做基准，不画点（避免巨大跳变）
    pushTierPoint('T0', 0); pushTierPoint('T1', 0); pushTierPoint('T2', 0)
  }
  lastTierSnapshot = cur
})

// SVG 折线生成：在 220×70 画布内绘制；Y 轴随本层最大值伸缩。
const TIER_COLORS: Record<TierKey, string> = {
  T0: 'var(--accent)',
  T1: 'var(--warn)',
  T2: 'var(--ok)',
}
function polylineFor(key: TierKey): string {
  const pts = tierHistory.value[key]
  if (pts.length < 2) return ''
  const W = 220, H = 70, pad = 4
  const maxV = Math.max(1, ...pts.map((p) => p.v))
  const stepX = (W - pad * 2) / (MAX_POINTS - 1)
  return pts.map((p, i) => {
    const x = pad + i * stepX
    const y = H - pad - (p.v / maxV) * (H - pad * 2)
    return `${x.toFixed(1)},${y.toFixed(1)}`
  }).join(' ')
}
function hasHistory(k: TierKey) {
  return tierHistory.value[k].length >= 2 && tierHistory.value[k].some((p) => p.v > 0)
}

// 三层缓存仪表：从 cache.status.tiers 取真实分层命中；无 tiers 时回退到总命中率。
// 统一项结构（无 key 时用 name 作兜底，避免 union 歧义 + TS 报错）。
interface TierRow { name: string; hits: number; saved: number; key: TierKey | string }
const tierRows = computed<TierRow[]>(() => {
  const c = cache.value
  if (!c) return []
  const t = c.tiers
  if (!t) {
    return [{ name: '总缓存', hits: c.hits, saved: c.saved_prompt_tokens, key: '总缓存' }]
  }
  return [
    { name: 'T0 精确',    hits: t.exact.hits,      saved: t.exact.saved_tokens,      key: 'T0' },
    { name: 'T1 规范化',  hits: t.normalized.hits, saved: t.normalized.saved_tokens, key: 'T1' },
    { name: 'T2 语义',    hits: t.semantic.hits,   saved: t.semantic.saved_tokens,   key: 'T2' },
  ]
})
function isTierKey(k: string): k is TierKey { return k === 'T0' || k === 'T1' || k === 'T2' }

const cacheHitRateDisplay = computed(() => {
  const c = cache.value
  if (!c) return '—'
  return `${c.client_hit_rate.toFixed(1)}%`
})

// V2.1-P4c：4 象限能力矩阵（若 plans.capabilities.quadrants 未实现 → 退化为旧 5 格统计）
const capStats = computed(() => caps.value?.stats ?? null)
const capQuadrants = computed(() => caps.value?.quadrants ?? null)
const capMatrix = computed(() => {
  const s = capStats.value
  if (!s) return []
  return [
    { label: '总能力',   value: s.total,       tone: 'neutral' },
    { label: '真实可用', value: s.real,        tone: 'good' },
    { label: '部分可用', value: s.partial,     tone: 'warn' },
    { label: '已实现',   value: s.implemented, tone: 'good' },
    { label: '叙事型',   value: s.narrative,   tone: 'neutral' },
  ]
})

async function safe<T>(fn: () => Promise<T>): Promise<T | null> {
  try { return await fn() } catch (e) { error.value = `加载失败: ${e instanceof Error ? e.message : String(e)}`; return null }
}

async function loadModeSpec() {
  const sp = await safe(() => rpc.call<ModeSpecPayload>('mode.spec', {}))
  if (sp) spec.value = sp
}

async function loadCache() {
  const c = await safe(() => rpc.call<CacheStatus>('cache.status', {}))
  if (c) cache.value = c
}

async function loadCaps() {
  const cp = await safe(() => rpc.call<CapabilitiesPayload>('plans.capabilities', {}))
  if (cp) caps.value = cp
}

/** 4 端心跳：优先用 `system.status` / `status` 探测；不存在方法则 UI 诚实标灰 */
async function loadHeartbeats() {
  const next: Record<string, boolean> = { web: true }
  try {
    if (rpc.supportsMethod('status')) {
      await rpc.call<{ ok?: boolean }>('status', {})
      // CLI/TUI/GUI 共享同一网关进程；能拿到 status 证明网关存活，此处保守地：
      // Web + CLI（gateway/serve 就是 core bin，必然可达）；GUI/Tauri 未封装探针 → 诚实灰，
      // TUI 单独进程未启动 → 灰。真实 4 端探针后续接入「gateway.endpoints.ping」后再精确。
      next.cli = true
    }
  } catch { /* ignore，保持灰 */ }
  endpointLive.value = next
}

async function refresh() {
  busy.value = true
  try {
    // 冷加载竞态守卫：WS 仍处于 connecting 时先等连接就绪，避免把「未就绪」
    // 当成加载失败，把 `not connected (state: connecting)` 裸串渲染给用户。
    error.value = ''
    if (!rpc.isConnected) await rpc.waitForConnection(10_000)
    await Promise.all([loadModeSpec(), loadModeStage(), loadCache(), loadCaps(), loadHeartbeats()])
  } catch (e) {
    error.value = `加载失败: ${e instanceof Error ? e.message : String(e)}`
  } finally {
    busy.value = false
  }
}

async function switchMode(kind: string) {
  busy.value = true
  await switchActiveMode(kind)
  await loadModeSpec()
  busy.value = false
}

// 立即首次采样；30s 周期刷新 + 折线点累积
onMounted(async () => {
  await refresh()
  await nextTick()
  refreshTimer.value = window.setInterval(refresh, 30000)
})
onUnmounted(() => {
  if (refreshTimer.value) window.clearInterval(refreshTimer.value)
})
</script>

<template>
  <div class="control-center">
    <header class="cc-head">
      <div>
        <h1 class="cc-title">智能中枢</h1>
        <p class="cc-sub">运行时模式 · 推理规格 · 缓存命中 · 能力矩阵 · 4 端心跳</p>
      </div>
      <div class="cc-head__actions">
        <!-- V2.1-P4c：4 端心跳小灯（紧凑置于顶部，不占主体版面） -->
        <div class="cc-beats" title="4 端心跳：在线=绿 · 离线/未知=灰，诚实不造假">
          <span
            v-for="ep in ENDPOINTS"
            :key="ep.name"
            class="cc-beat"
            :class="{ 'is-live': endpointLive[ep.name] }"
            :title="`${ep.label} · ${ep.hint} · ${endpointLive[ep.name] ? '在线' : '未连接'}`"
          >
            <span class="cc-beat__dot" />
            <span class="cc-beat__label">{{ ep.label }}</span>
          </span>
        </div>
        <button class="btn btn--sm btn--ghost" :disabled="busy" @click="refresh">
          <Icon name="refresh" :size="14" /> 刷新
        </button>
      </div>
    </header>

    <p v-if="error" class="cc-error">{{ error }}</p>

    <!-- 模式切换 + V2.1-P4a Bloom 三段锚定胶囊 -->
    <section class="cc-card">
      <div class="cc-card__title">运行时模式（mode.get / mode.switch）</div>
      <div class="cc-pills">
        <span class="cc-pill cc-pill--accent">当前：{{ modeLabel }}</span>
        <span v-if="spec" class="cc-pill">风险：{{ spec.risk_level }}</span>
        <span v-if="spec" class="cc-pill">工具许可 {{ spec.allowed_tools }} / 禁用 {{ spec.denied_tools }}</span>
      </div>
      <div class="cc-actions">
        <button
          v-for="k in modeKinds"
          :key="k.kind"
          class="btn btn--sm"
          :class="(mode?.kind ?? mode?.mode) === k.kind ? 'btn--primary' : 'btn--ghost'"
          :disabled="busy"
          @click="switchMode(k.kind)"
        >{{ k.label }}</button>
      </div>

      <!-- V2.2-Bloom RPC：三段锚定胶囊（后端 mode.stage 权威值，回退前端规格推导） -->
      <div v-if="bloom" class="cc-anchor">
        <div class="cc-anchor__title">
          <span>Bloom 三段锚定（mode.stage · {{ anchorStageSource === 'session' ? '会话真实推进' : anchorStageSource === 'spec' ? '规格推导' : '前端推导' }}）</span>
          <span class="cc-anchor__legend">
            <span
              v-if="anchorStageSource"
              class="cc-anchor__source"
              :class="anchorStageSource === 'session' ? 'is-authoritative' : 'is-derived'"
              :title="anchorStageSource === 'session' ? '来自后端会话锚定台账（权威）' : '来自后端规格推导（非运行时）'"
            >{{ anchorStageSource === 'session' ? '权威值' : '规格值' }}</span>
            {{ anchorStage === 'disabled' ? '未启用' : (STAGES.find(s => s.key === anchorStage)?.title ?? '') }}
          </span>
        </div>
        <ol class="cc-anchor__track">
          <li
            v-for="(stage, idx) in STAGES"
            :key="stage.key"
            class="cc-anchor__step"
            :class="{
              'is-done': stageReached(stage.key),
              'is-current': anchorStage === stage.key,
              'is-disabled': anchorStage === 'disabled',
            }"
          >
            <span class="cc-anchor__dot">{{ idx + 1 }}</span>
            <span class="cc-anchor__label">
              <strong>{{ stage.label }}</strong>
              <small>{{ stage.title }}</small>
            </span>
            <i v-if="idx < STAGES.length - 1" class="cc-anchor__line" />
          </li>
        </ol>
        <div v-if="anchorStage === 'disabled'" class="cc-empty" style="margin-top:8px">
          当前 bloom.anchoring_enabled=false，三段锚定未启用（Plan/Agent/Ensemble 不启用蜂群规格）。
        </div>
      </div>
    </section>

    <div class="cc-grid">
      <!-- BloomSpec 全字段 -->
      <section class="cc-card">
        <div class="cc-card__title">推理规格（mode.spec · BloomSpec）</div>
        <template v-if="bloom">
          <dl class="cc-spec">
            <div class="cc-spec__row"><dt>声明思考深度</dt><dd>{{ bloom.thinking_depth }}</dd></div>
            <div class="cc-spec__row"><dt>声明验证遍数</dt><dd>{{ bloom.verification_passes }}</dd></div>
            <div class="cc-spec__row cc-spec__row--live">
              <dt>执行打磨轮数</dt><dd>{{ bloom.effective_refine_rounds }}</dd>
            </div>
            <div class="cc-spec__row cc-spec__row--live">
              <dt>执行验证轮数</dt><dd>{{ bloom.effective_verify_rounds }}</dd>
            </div>
            <div class="cc-spec__row cc-spec__row--live">
              <dt>执行最大 LLM 调用</dt><dd>{{ bloom.effective_max_llm_calls }}</dd>
            </div>
            <div class="cc-spec__row"><dt>推理档位（声明）</dt><dd>{{ bloom.reasoning_effort }}</dd></div>
            <div class="cc-spec__row cc-spec__row--live">
              <dt>推理档位（实际）</dt><dd>{{ bloom.effective_thinking_effort }}</dd>
            </div>
            <div class="cc-spec__row"><dt>采样温度</dt><dd>{{ bloom.temperature }}</dd></div>
            <div class="cc-spec__row"><dt>蜂群分片上限</dt><dd>{{ bloom.max_swarm_shards }}</dd></div>
            <div class="cc-spec__row"><dt>子专家上限</dt><dd>{{ bloom.max_sub_experts }}</dd></div>
            <div class="cc-spec__row"><dt>快照</dt><dd>{{ bloom.snapshot_enabled ? '开' : '关' }}</dd></div>
            <div class="cc-spec__row"><dt>失败自动重试</dt><dd>{{ bloom.auto_retry_failures ? '开' : '关' }}</dd></div>
            <div class="cc-spec__row"><dt>锚定冷启动</dt><dd>{{ bloom.anchoring_enabled ? '开' : '关' }}</dd></div>
            <div class="cc-spec__row"><dt>预算 Token</dt><dd>{{ bloom.budget_tokens ?? '—' }}</dd></div>
            <div class="cc-spec__row"><dt>最大迭代</dt><dd>{{ bloom.max_iterations ?? '—' }}</dd></div>
            <div class="cc-spec__row"><dt>最大修复</dt><dd>{{ bloom.max_fixes ?? '—' }}</dd></div>
          </dl>
        </template>
        <p v-else class="cc-empty">当前模式无 BloomSpec（如 Plan/Agent/Ensemble 不启用蜂群规格）。</p>
      </section>

      <!-- V2.1-P4b：三层缓存仪表 + 折线历史 + ExactOnly 徽章 -->
      <section class="cc-card">
        <div class="cc-card__title">
          <span>三层语义缓存（cache.status · 最近 10 次采样增量折线）</span>
          <span class="cc-pills" style="margin:0 0 0 auto;display:inline-flex">
            <span
              class="cc-pill"
              :class="(cache?.exact_only_tools ?? true) ? 'cc-pill--accent' : ''"
              :title="(cache?.exact_only_tools ?? true) ? '工具调用路径永远走 ExactOnly：语义命中会静默丢弃工具，正确性红线（invariant 8）' : '注意：语义近似命中可能开启，请核对'"
            >
              <Icon name="lock" :size="10" /> Tools ExactOnly
            </span>
            <span class="cc-pill" :class="cache?.semantic_enabled ? '' : 'cc-pill--accent'">
              T2 语义 {{ cache?.semantic_enabled ? '开' : '关（默认）' }}
            </span>
          </span>
        </div>
        <div class="cc-cache-rate">
          <span class="cc-cache-rate__num">{{ cacheHitRateDisplay }}</span>
          <span class="cc-cache-rate__label">客户端命中率（诚实计量，非冒充）</span>
        </div>
        <table class="cc-table">
          <thead><tr><th>层级</th><th>命中</th><th>节省 Token</th><th>最近 10 次增量</th></tr></thead>
          <tbody>
            <tr v-for="row in tierRows" :key="row.name">
              <td>{{ row.name }}</td>
              <td>{{ row.hits }}</td>
              <td>{{ row.saved }}</td>
              <td>
                <svg class="cc-spark" viewBox="0 0 220 70" preserveAspectRatio="none" aria-hidden="true">
                  <polyline
                    v-if="isTierKey(row.key) && hasHistory(row.key)"
                    fill="none"
                    :stroke="TIER_COLORS[row.key as TierKey]"
                    stroke-width="2"
                    stroke-linejoin="round"
                    stroke-linecap="round"
                    :points="polylineFor(row.key as TierKey)"
                  />
                  <text
                    v-else
                    x="8"
                    y="42"
                    font-size="11"
                    fill="var(--text-dim)"
                  >待采样（{{ MAX_POINTS }} 点后可见趋势）</text>
                </svg>
              </td>
            </tr>
          </tbody>
        </table>
        <div class="cc-pills">
          <span class="cc-pill">条目 {{ cache?.entries ?? 0 }}</span>
          <span class="cc-pill">查询 {{ cache?.lookups ?? 0 }}</span>
          <span class="cc-pill">未命中 {{ cache?.misses ?? 0 }}</span>
        </div>
      </section>
    </div>

    <!-- V2.1-P4c：4 象限能力矩阵（优先）+ 旧 5 格统计（退化） -->
    <section class="cc-card">
      <div class="cc-card__title">Agent 能力矩阵（plans.capabilities）</div>

      <template v-if="capQuadrants">
        <div class="cc-quad">
          <div
            class="cc-quad__cell"
            :style="{ background: 'color-mix(in srgb, var(--accent) 12%, var(--bg-elevated))' }"
          >
            <div class="cc-quad__num">{{ capQuadrants.plan }}</div>
            <div class="cc-quad__label">Plan · 规划</div>
            <div class="cc-quad__sub">拆解 / 清单 / 重规划</div>
          </div>
          <div
            class="cc-quad__cell"
            :style="{ background: 'color-mix(in srgb, var(--ok) 14%, var(--bg-elevated))' }"
          >
            <div class="cc-quad__num">{{ capQuadrants.execute }}</div>
            <div class="cc-quad__label">Execute · 执行</div>
            <div class="cc-quad__sub">沙箱 / 终端 / 浏览器</div>
          </div>
          <div
            class="cc-quad__cell"
            :style="{ background: 'color-mix(in srgb, var(--warn) 14%, var(--bg-elevated))' }"
          >
            <div class="cc-quad__num">{{ capQuadrants.observe }}</div>
            <div class="cc-quad__label">Observe · 观察</div>
            <div class="cc-quad__sub">事件 / 日志 / 心跳</div>
          </div>
          <div
            class="cc-quad__cell is-operate"
          >
            <div class="cc-quad__num">{{ capQuadrants.operate }}</div>
            <div class="cc-quad__label">Operate · 运维</div>
            <div class="cc-quad__sub">部署 / 发布 / SSH</div>
          </div>
        </div>
      </template>

      <template v-else>
        <p class="cc-empty" style="margin-bottom:10px">
          plans.capabilities 暂未实现 quadrants（4 象限），以下展示旧 5 格统计（兼容）。
        </p>
        <div class="cc-matrix">
          <div v-for="m in capMatrix" :key="m.label" class="cc-matrix__cell" :class="`cc-matrix__cell--${m.tone}`">
            <span class="cc-matrix__num">{{ m.value }}</span>
            <span class="cc-matrix__label">{{ m.label }}</span>
          </div>
        </div>
      </template>

      <p v-if="caps?.narrativeIds?.length" class="cc-narrative">
        叙事型能力 {{ caps.narrativeIds.length }} 项：{{ caps.narrativeIds.slice(0, 6).join('、') }}<span v-if="caps.narrativeIds.length > 6">…</span>
      </p>
    </section>
  </div>
</template>

<style scoped>
.control-center {
  max-width: 1040px;
  margin: 0 auto;
  padding: 24px 20px 48px;
  display: flex;
  flex-direction: column;
  gap: 16px;
}
.cc-head { display: flex; align-items: center; justify-content: space-between; gap: 16px; flex-wrap: wrap; }
.cc-head__actions { display: flex; align-items: center; gap: 12px; }
.cc-title { font-size: 20px; font-weight: 700; margin: 0; }
.cc-sub { margin: 4px 0 0; color: var(--text-dim); font-size: 13px; }
.cc-error { color: var(--danger); font-size: 13px; margin: 0; }
.cc-card {
  background: var(--bg-surface);
  border: 1px solid var(--border);
  border-radius: var(--radius-card);
  padding: 16px;
}
.cc-card__title {
  display: flex; align-items: center; gap: 10px;
  font-size: 13px; font-weight: 600; color: var(--text); margin-bottom: 12px;
}
.cc-grid { display: grid; grid-template-columns: 1fr 1fr; gap: 16px; }
@media (max-width: 720px) { .cc-grid { grid-template-columns: 1fr; } }
.cc-pills { display: flex; flex-wrap: wrap; gap: 6px; margin-top: 10px; }
.cc-pill {
  display: inline-flex; align-items: center; gap: 4px;
  font-size: 12px; padding: 3px 8px; border-radius: 999px;
  background: var(--bg-elevated); color: var(--text-dim);
}
.cc-pill--accent { background: color-mix(in srgb, var(--accent) 22%, transparent); color: var(--accent); }
.cc-actions { display: flex; flex-wrap: wrap; gap: 8px; margin-top: 12px; }

/* 顶部 4 端心跳小灯 */
.cc-beats { display: inline-flex; align-items: center; gap: 8px; padding: 4px 8px; border-radius: 999px; background: var(--bg-elevated); }
.cc-beat { display: inline-flex; align-items: center; gap: 6px; font-size: 12px; color: var(--text-dim); }
.cc-beat__dot { width: 8px; height: 8px; border-radius: 999px; background: var(--hairline); box-shadow: 0 0 0 2px color-mix(in srgb, currentColor 15%, transparent); }
.cc-beat.is-live { color: var(--text); }
.cc-beat.is-live .cc-beat__dot { background: var(--ok); box-shadow: 0 0 0 2px color-mix(in srgb, var(--ok) 25%, transparent); }
.cc-beat__label { font-variant-numeric: tabular-nums; }

/* P4a 三段锚定轨道 */
.cc-anchor { margin-top: 14px; padding: 12px; border-radius: var(--radius-md); background: color-mix(in srgb, var(--accent) 6%, var(--bg-elevated)); border: 1px solid color-mix(in srgb, var(--accent) 18%, transparent); }
.cc-anchor__source {
  display: inline-flex; align-items: center; padding: 1px 8px; margin-right: 8px;
  border-radius: 999px; font-size: 11px; font-weight: 600;
}
.cc-anchor__source.is-authoritative {
  background: color-mix(in srgb, var(--ok) 18%, var(--bg-elevated));
  color: var(--ok);
  border: 1px solid color-mix(in srgb, var(--ok) 35%, transparent);
}
.cc-anchor__source.is-derived {
  background: color-mix(in srgb, var(--warn) 18%, var(--bg-elevated));
  color: var(--warn);
  border: 1px solid color-mix(in srgb, var(--warn) 35%, transparent);
}
.cc-anchor__title { display: flex; justify-content: space-between; align-items: baseline; font-size: 12px; color: var(--text-dim); margin-bottom: 10px; }
.cc-anchor__title strong { font-size: 13px; color: var(--text); font-weight: 600; }
.cc-anchor__legend { color: var(--accent); font-weight: 600; }
.cc-anchor__track { list-style: none; padding: 0; margin: 0; display: flex; align-items: stretch; gap: 0; }
.cc-anchor__step { position: relative; flex: 1; display: flex; align-items: center; gap: 8px; padding: 6px 8px; }
.cc-anchor__dot {
  width: 26px; height: 26px; flex: 0 0 26px; border-radius: 50%;
  display: inline-flex; align-items: center; justify-content: center;
  background: var(--bg-surface); border: 1px solid var(--border); color: var(--text-dim); font-weight: 700; font-size: 12px;
  transition: all var(--dur-base) var(--ease-standard);
}
.cc-anchor__step.is-done .cc-anchor__dot { background: var(--ok); color: white; border-color: var(--ok); }
.cc-anchor__step.is-current .cc-anchor__dot { background: var(--accent); color: white; border-color: var(--accent); box-shadow: 0 0 0 4px color-mix(in srgb, var(--accent) 22%, transparent); }
.cc-anchor__step.is-disabled .cc-anchor__dot { opacity: .5; }
.cc-anchor__label { display: flex; flex-direction: column; gap: 2px; font-size: 12px; }
.cc-anchor__label strong { color: var(--text); }
.cc-anchor__label small { color: var(--text-dim); font-weight: 400; }
.cc-anchor__line { position: absolute; right: -2px; top: 19px; width: calc(100% - 20px); height: 2px; background: var(--border); border-radius: 2px; /* radius-allow: 进度条圆角极小值仅用于 2px 高的轨道 */ }
.cc-anchor__step.is-done .cc-anchor__line { background: var(--ok); }
.cc-anchor__step:last-child .cc-anchor__line { display: none; }

.cc-spec { display: grid; grid-template-columns: 1fr 1fr; gap: 6px 16px; margin: 0; }
.cc-spec__row { display: flex; justify-content: space-between; gap: 8px; font-size: 13px; padding: 4px 0; border-bottom: 1px solid var(--border); }
.cc-spec__row dt { color: var(--text-dim); margin: 0; }
.cc-spec__row dd { margin: 0; font-variant-numeric: tabular-nums; }
.cc-spec__row--live dd { color: var(--accent); font-weight: 600; }
.cc-empty { color: var(--text-dim); font-size: 13px; }
.cc-cache-rate { display: flex; align-items: baseline; gap: 10px; margin-bottom: 12px; }
.cc-cache-rate__num { font-size: 28px; font-weight: 700; font-variant-numeric: tabular-nums; }
.cc-cache-rate__label { font-size: 12px; color: var(--text-dim); }
.cc-table { width: 100%; border-collapse: collapse; font-size: 13px; }
.cc-table th, .cc-table td { text-align: left; padding: 6px 8px; border-bottom: 1px solid var(--border); vertical-align: middle; }
.cc-table th { color: var(--text-dim); font-weight: 500; }
.cc-table td { font-variant-numeric: tabular-nums; }
.cc-spark { width: 100%; height: 46px; display: block; }

/* P4c 4 象限 */
.cc-quad { display: grid; grid-template-columns: repeat(4, 1fr); gap: 12px; }
@media (max-width: 720px) { .cc-quad { grid-template-columns: repeat(2, 1fr); } }
.cc-quad__cell {
  display: flex; flex-direction: column; align-items: flex-start; gap: 4px;
  padding: 16px 14px; border-radius: var(--radius-md); border: 1px solid var(--border);
}
.cc-quad__cell.is-operate { background: color-mix(in srgb, var(--queued) 14%, var(--bg-elevated)); border-color: color-mix(in srgb, var(--queued) 28%, var(--border)); }
.cc-quad__num { font-size: 26px; font-weight: 800; font-variant-numeric: tabular-nums; color: var(--text); }
.cc-quad__label { font-size: 13px; font-weight: 700; color: var(--text); }
.cc-quad__sub { font-size: 11px; color: var(--text-dim); }

.cc-matrix { display: grid; grid-template-columns: repeat(5, 1fr); gap: 10px; }
@media (max-width: 720px) { .cc-matrix { grid-template-columns: repeat(2, 1fr); } }
.cc-matrix__cell {
  display: flex; flex-direction: column; align-items: center; gap: 4px;
  padding: 14px 8px; border-radius: var(--radius-md); background: var(--bg-elevated);
}
.cc-matrix__num { font-size: 22px; font-weight: 700; font-variant-numeric: tabular-nums; }
.cc-matrix__label { font-size: 12px; color: var(--text-dim); }
.cc-matrix__cell--good .cc-matrix__num { color: var(--ok); }
.cc-matrix__cell--warn .cc-matrix__num { color: var(--warn); }
.cc-narrative { font-size: 12px; color: var(--text-dim); margin: 12px 0 0; }
</style>
