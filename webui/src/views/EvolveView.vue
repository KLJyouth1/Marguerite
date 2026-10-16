<script setup lang="ts">
/**
 * 自进化 —— 两级闭环的操控台。
 *
 * ① **元智能体（selfevolve.\*）**：监控（行为台账抽取失败信号）→ 根因裁决 → 白名单改进规划
 *    → 沙箱回放四闸裁决 → 版本快照 / 灰度 / 回滚。默认 `auto_apply=false`：**提案只登记，不改
 *    任何真实配置**，必须由人在此页批准。
 * ② **prompt 自进化（evolve.history）**：既有 EvolutionRunner 的历史轮次（诊断→变异→评分→采纳）。
 *
 * 诚实边界：本页不发任何「应用改动」指令——「批准/灰度」只推进台账阶段（可审计、可回滚），
 * 真实配置的落地仍由人工或受控执行器完成。无信号时不编造提案。
 */
import { ref, computed, onMounted } from 'vue'
import Icon from '@/components/Icon.vue'
import { useRpcStore } from '@/stores/rpc'

const rpc = useRpcStore()

interface EvolveRecord {
  round: number; timestamp: string; diagnosis: string;
  score_before: number; score_after: number; accepted: boolean;
  rejection_reason: string | null;
}

interface Outcome { passed: number; total: number; tokens: number; ms: number; success_rate?: number }
interface Snapshot {
  version: number; at: string; stage: string; cause: string;
  target: string; risk: string; mutation: string;
  baseline: Outcome | null; candidate: Outcome | null;
  verdict: { verdict: string; promoted?: boolean; detail?: unknown } | null;
  note: string | null
}
interface Signal { kind: string; session: string; tool: string | null; detail: string; at_ms: number }
interface MetaStatus {
  auto_apply: boolean; max_risk: string; min_success_rate: number;
  max_token_growth: number; max_latency_growth: number;
  pending_approvals: number; versions: number; records: number;
  active_version: number | null; active_stage: string | null; active_target: string | null;
  rollback_target: number | null; ledger: string
}

const records = ref<EvolveRecord[]>([])
const snapshots = ref<Snapshot[]>([])
const signals = ref<Signal[]>([])
const status = ref<MetaStatus | null>(null)
const message = ref('')
const error = ref('')
const busy = ref(false)
const available = computed(() => rpc.supportsMethod('selfevolve.status'))

const acceptedCount = computed(() => records.value.filter(r => r.accepted).length)
const latestScore = computed(() =>
  records.value.length ? records.value[records.value.length - 1].score_after.toFixed(3) : '—')
const pending = computed(() => snapshots.value.filter(v => v.stage === 'proposed'))
const active = computed(() => snapshots.value.find(v => v.version === status.value?.active_version) ?? null)

const STAGE_LABEL: Record<string, string> = {
  proposed: '待审', approved: '已批准', canary: '灰度中', live: '已全量',
  rolled_back: '已回滚', rejected: '未采纳',
}
const KIND_LABEL: Record<string, string> = {
  agent_self: '主智能体', tool_fault: '工具故障',
  tool_insufficient: '工具能力不足', capability_gap: '能力缺口',
}

function stageLabel(s: string) { return STAGE_LABEL[s] ?? s }
function kindLabel(k: string) { return KIND_LABEL[k] ?? k }
function rate(o: Outcome | null): string {
  if (!o) return '未测量'
  if (!o.total) return '0/0'
  return `${o.passed}/${o.total}（${Math.round((o.success_rate ?? o.passed / o.total) * 100)}%）`
}

async function loadEvolveHistory() {
  try {
    // 冷加载竞态守卫：WS 仍处于 connecting 时先等连接就绪，避免把「未就绪」
    // 当成加载失败，把 `not connected (state: connecting)` 裸串渲染给用户。
    if (!rpc.isConnected) await rpc.waitForConnection(10_000)
    const res = await rpc.call<{ records: EvolveRecord[]; error?: string }>('evolve.history')
    if (res.error) { error.value = res.error } else { records.value = (res.records || []).reverse() }
  } catch (e) {
    error.value = `自进化历史加载失败: ${e instanceof Error ? e.message : String(e)}`
  }
}

async function loadMeta() {
  if (!available.value) return
  if (!rpc.isConnected) await rpc.waitForConnection(10_000)
  const hist = await rpc.call<{ versions: Snapshot[] }>('selfevolve.history')
  snapshots.value = hist.versions ?? []
  status.value = await rpc.call<MetaStatus>('selfevolve.status')
  const sig = await rpc.call<{ items: Signal[] }>('selfevolve.signals', { limit: 60 })
  signals.value = sig.items ?? []
}

async function refresh() {
  error.value = ''
  try { await loadMeta(); await loadEvolveHistory() } finally { /* 错误已在各自函数内呈现 */ }
}

/** 扫描：监控→诊断→规划→登记提案（不应用任何改动） */
async function sweep() {
  error.value = ''; message.value = ''; busy.value = true
  try {
    const res = await rpc.call<{ message: string; stage: string; version: number }>('selfevolve.sweep', { limit: 200 })
    message.value = res.version > 0 ? `已登记 v${res.version}：${res.message}` : res.message
    await loadMeta()
  } catch (e) {
    error.value = `扫描失败: ${e instanceof Error ? e.message : String(e)}`
  } finally { busy.value = false }
}

/** 仅分析（只读预演）：不写台账，可安全反复点 */
async function analyze() {
  error.value = ''; message.value = ''; busy.value = true
  try {
    const res = await rpc.call<{ message: string; signals: number }>('selfevolve.analyze', { limit: 200 })
    message.value = res.message
    await loadMeta()
  } catch (e) {
    error.value = `分析失败: ${e instanceof Error ? e.message : String(e)}`
  } finally { busy.value = false }
}

async function act(method: string, params: Record<string, unknown>, okText: string) {
  error.value = ''; message.value = ''; busy.value = true
  try {
    await rpc.call(method, params)
    message.value = okText
    await loadMeta()
  } catch (e) {
    error.value = `${okText}失败: ${e instanceof Error ? e.message : String(e)}`
  } finally { busy.value = false }
}

const approve = (v: number) => act('selfevolve.approve', { version: v }, `已批准 v${v}`)
const reject = (v: number) => act('selfevolve.reject', { version: v, reason: '人工否决' }, `已否决 v${v}`)
const toCanary = (v: number) => act('selfevolve.promote', { version: v, stage: 'canary' }, `v${v} 已进入灰度`)
const toLive = (v: number) => act('selfevolve.promote', { version: v, stage: 'live' }, `v${v} 已转全量`)
const rollback = () => act('selfevolve.rollback', { reason: '指标回落，人工回滚' }, '已回滚')

onMounted(refresh)
</script>

<template>
  <div class="control-stage evolve-stage">
    <header class="control-stage__header">
      <div class="control-stage__title-block">
        <h1 class="control-stage__title">自进化</h1>
        <p class="control-stage__subtitle">
          元智能体闭环（监控 → 诊断 → 规划 → 沙箱裁决 → 版本）· 提案须人工批准
        </p>
      </div>
      <div class="control-stage__actions">
        <button class="btn btn--ghost" :disabled="busy" @click="refresh">
          <Icon name="refresh" :size="16" /><span>刷新</span>
        </button>
      </div>
    </header>

    <section v-if="error" class="evolve-error">{{ error }}</section>
    <section v-if="message" class="evolve-note">{{ message }}</section>

    <!-- ── 元智能体：策略与生效状态 ── -->
    <section v-if="available" class="meta-card">
      <div class="meta-card__head">
        <span class="meta-card__title">元智能体</span>
        <span class="meta-pill" :class="status?.auto_apply ? 'meta-pill--warn' : 'meta-pill--safe'">
          {{ status?.auto_apply ? '自动应用已开启' : '自动应用关闭（提案待人工批准）' }}
        </span>
      </div>
      <dl class="meta-grid">
        <div><dt>风险上限</dt><dd>{{ status?.max_risk ?? '—' }}</dd></div>
        <div><dt>采纳门槛</dt><dd>{{ status ? Math.round(status.min_success_rate * 100) + '%' : '—' }}</dd></div>
        <div><dt>成本涨幅上限</dt><dd>{{ status ? '+' + Math.round((status.max_token_growth - 1) * 100) + '%' : '—' }}</dd></div>
        <div><dt>时延涨幅上限</dt><dd>{{ status ? '+' + Math.round((status.max_latency_growth - 1) * 100) + '%' : '—' }}</dd></div>
        <div><dt>生效版本</dt>
          <dd>
            <template v-if="active">v{{ active.version }} · {{ stageLabel(active.stage) }}</template>
            <template v-else>无</template>
          </dd>
        </div>
        <div><dt>待审提案</dt><dd>{{ status?.pending_approvals ?? 0 }}</dd></div>
        <div><dt>版本 / 记录</dt><dd>{{ status?.versions ?? 0 }} / {{ status?.records ?? 0 }}</dd></div>
        <div><dt>回滚落点</dt><dd>{{ status?.rollback_target ? 'v' + status.rollback_target : '内置基线' }}</dd></div>
      </dl>
      <div class="meta-actions">
        <button class="btn btn--primary" :disabled="busy || !rpc.isConnected" @click="sweep">扫描并提案</button>
        <button class="btn btn--ghost" :disabled="busy || !rpc.isConnected" @click="analyze">仅分析（只读）</button>
        <button v-if="active && (active.stage === 'canary' || active.stage === 'live')"
                class="btn btn--ghost btn--danger" :disabled="busy" @click="rollback">
          回滚 v{{ active.version }}
        </button>
      </div>
      <p class="meta-hint">
        台账：<code>{{ status?.ledger }}</code> —
        扫描只登记提案，<b>不会修改任何提示词、工具参数或脚本</b>；落地由人工或受控执行器完成。
      </p>
    </section>

    <!-- ── 待审提案 / 推进 ── -->
    <section v-if="available && pending.length" class="meta-section">
      <h2 class="meta-section__title">待审提案（{{ pending.length }}）</h2>
      <div v-for="p in pending" :key="'p' + p.version" class="meta-row">
        <div class="meta-row__head">
          <span class="meta-row__ver">v{{ p.version }}</span>
          <span class="meta-row__badge">改 {{ p.target }}</span>
          <span class="meta-row__badge meta-row__badge--risk">风险 {{ p.risk }}</span>
          <span class="meta-row__time">{{ p.at }}</span>
        </div>
        <p class="meta-row__text">{{ p.mutation }}</p>
        <div class="meta-row__foot">
          <span class="meta-row__dim">基线 {{ rate(p.baseline) }} · 候选 {{ rate(p.candidate) }}</span>
          <span class="meta-row__btns">
            <button class="btn btn--primary" :disabled="busy" @click="approve(p.version)">批准</button>
            <button class="btn btn--ghost" :disabled="busy" @click="reject(p.version)">否决</button>
          </span>
        </div>
      </div>
    </section>

    <section v-if="available && active && active.stage === 'approved'" class="meta-section">
      <h2 class="meta-section__title">已批准，待灰度</h2>
      <div class="meta-row">
        <div class="meta-row__head">
          <span class="meta-row__ver">v{{ active.version }}</span>
          <span class="meta-row__badge">改 {{ active.target }}</span>
          <span class="meta-row__time">{{ active.at }}</span>
        </div>
        <p class="meta-row__text">{{ active.mutation }}</p>
        <div class="meta-row__foot">
          <span class="meta-row__dim">{{ active.note ?? '' }}</span>
          <span class="meta-row__btns">
            <button class="btn btn--primary" :disabled="busy" @click="toCanary(active.version)">进入灰度</button>
          </span>
        </div>
      </div>
    </section>

    <section v-if="available && active && active.stage === 'canary'" class="meta-section">
      <h2 class="meta-section__title">灰度中</h2>
      <div class="meta-row">
        <div class="meta-row__head">
          <span class="meta-row__ver">v{{ active.version }}</span>
          <span class="meta-row__badge">改 {{ active.target }}</span>
          <span class="meta-row__time">{{ active.at }}</span>
        </div>
        <p class="meta-row__text">观察通过后可转全量；出现回落随时回滚。</p>
        <div class="meta-row__foot">
          <span class="meta-row__dim">基线 {{ rate(active.baseline) }} · 候选 {{ rate(active.candidate) }}</span>
          <span class="meta-row__btns">
            <button class="btn btn--primary" :disabled="busy" @click="toLive(active.version)">转全量</button>
          </span>
        </div>
      </div>
    </section>

    <!-- ── 失败信号 ── -->
    <section v-if="available" class="meta-section">
      <h2 class="meta-section__title">失败信号（近窗口 {{ signals.length }} 条）</h2>
      <ul v-if="signals.length" class="sig-list">
        <li v-for="(s, i) in signals.slice(-12).reverse()" :key="i" class="sig-item">
          <span class="sig-kind">{{ kindLabel(s.kind) }}</span>
          <span v-if="s.tool" class="sig-tool">{{ s.tool }}</span>
          <span class="sig-detail">{{ s.detail }}</span>
        </li>
      </ul>
      <div v-else class="evolve-empty">窗口内没有失败信号 —— 没有信号就不生成提案。</div>
    </section>

    <!-- ── 版本史（折叠后）── -->
    <section v-if="available && snapshots.length" class="meta-section">
      <h2 class="meta-section__title">版本史（{{ snapshots.length }}）</h2>
      <div v-for="v in [...snapshots].reverse()" :key="'v' + v.version" class="meta-row meta-row--tight">
        <div class="meta-row__head">
          <span class="meta-row__ver">v{{ v.version }}</span>
          <span class="meta-row__badge">{{ stageLabel(v.stage) }}</span>
          <span class="meta-row__badge">改 {{ v.target }}</span>
          <span class="meta-row__time">{{ v.at }}</span>
        </div>
        <p class="meta-row__text">{{ v.mutation }}</p>
        <p class="meta-row__dim">
          根因 {{ v.cause }} · 基线 {{ rate(v.baseline) }} · 候选 {{ rate(v.candidate) }}
          <template v-if="v.verdict"> · 裁决 {{ v.verdict.verdict }}</template>
          <template v-if="v.note"> · {{ v.note }}</template>
        </p>
      </div>
    </section>

    <section v-if="!available" class="evolve-empty">
      当前引擎未声明 selfevolve.* 方法（旧版引擎）。
    </section>

    <!-- ── prompt 自进化历史（既有 EvolutionRunner）── -->
    <section class="meta-section">
      <h2 class="meta-section__title">prompt 自进化历史</h2>
      <div class="evolve-stats">
        <div class="evolve-stat">
          <div class="evolve-stat__label">总轮次</div>
          <div class="evolve-stat__value">{{ records.length }}</div>
        </div>
        <div class="evolve-stat">
          <div class="evolve-stat__label">已采纳</div>
          <div class="evolve-stat__value evolve-stat__value--ok">{{ acceptedCount }}</div>
        </div>
        <div class="evolve-stat">
          <div class="evolve-stat__label">最新评分</div>
          <div class="evolve-stat__value">{{ latestScore }}</div>
        </div>
      </div>
      <div class="evolve-timeline">
        <div v-for="r in records" :key="r.round" class="evolve-record">
          <div class="evolve-record__dot" :class="{ 'is-accepted': r.accepted }"></div>
          <div class="evolve-record__body">
            <div class="evolve-record__header">
              <span class="evolve-record__round">第 {{ r.round }} 轮</span>
              <span class="evolve-record__badge" :class="r.accepted ? 'badge--ok' : 'badge--no'">
                {{ r.accepted ? '采纳' : '拒绝' }}
              </span>
              <span class="evolve-record__time">{{ r.timestamp }}</span>
            </div>
            <p class="evolve-record__diagnosis">{{ r.diagnosis }}</p>
            <div class="evolve-record__scores">
              <span>评分: {{ r.score_before.toFixed(3) }} → <b>{{ r.score_after.toFixed(3) }}</b></span>
              <span v-if="r.rejection_reason" class="evolve-record__rejection">拒绝原因: {{ r.rejection_reason }}</span>
            </div>
          </div>
        </div>
        <div v-if="!records.length" class="evolve-empty">暂无进化记录</div>
      </div>
    </section>
  </div>
</template>

<style scoped>
.evolve-stage { padding: 24px 28px; max-width: 900px; margin: 0 auto; }
.evolve-error { margin: 16px 0; padding: 10px 14px; background: color-mix(in srgb, var(--danger) 10%, var(--card)); border: 1px solid color-mix(in srgb, var(--danger) 30%, transparent); border-radius: var(--radius-sm); color: var(--danger); font-size: 12px; }
.evolve-note { margin: 16px 0; padding: 10px 14px; background: color-mix(in srgb, var(--ok) 8%, var(--card)); border: 1px solid color-mix(in srgb, var(--ok) 25%, transparent); border-radius: var(--radius-sm); color: var(--text); font-size: 12px; }
.evolve-stats { display: grid; grid-template-columns: repeat(3, 1fr); gap: 12px; margin: 20px 0; }
.evolve-stat { background: var(--card); border: 1px solid var(--hairline); border-radius: var(--radius-md); padding: 16px; text-align: center; }
.evolve-stat__label { font-size: 11px; color: var(--text-dim); }
.evolve-stat__value { font-size: 28px; font-weight: 700; margin-top: 4px; }
.evolve-stat__value--ok { color: var(--ok); }
.evolve-timeline { position: relative; padding-left: 24px; }
.evolve-timeline::before { content: ''; position: absolute; left: 7px; top: 0; bottom: 0; width: 2px; background: var(--hairline); }
.evolve-record { position: relative; margin-bottom: 16px; }
.evolve-record__dot { position: absolute; left: -22px; top: 4px; width: 12px; height: 12px; border-radius: 50%; background: var(--hairline); border: 2px solid var(--bg); }
.evolve-record__dot.is-accepted { background: var(--ok); }
.evolve-record__body { background: var(--card); border: 1px solid var(--hairline); border-radius: var(--radius-md); padding: 12px 14px; }
.evolve-record__header { display: flex; align-items: center; gap: 10px; }
.evolve-record__round { font-weight: 600; font-size: 13px; }
.evolve-record__badge { font-size: 10px; font-weight: 700; padding: 2px 8px; border-radius: var(--radius-pill); }
.badge--ok { background: color-mix(in srgb, var(--ok) 15%, transparent); color: var(--ok); }
.badge--no { background: color-mix(in srgb, var(--danger) 15%, transparent); color: var(--danger); }
.evolve-record__time { margin-left: auto; font-size: 10px; color: var(--text-dim); }
.evolve-record__diagnosis { font-size: 12px; color: var(--text-muted); margin: 6px 0; line-height: 1.5; }
.evolve-record__scores { display: flex; gap: 16px; font-size: 11px; color: var(--text-dim); }
.evolve-record__rejection { color: var(--danger); }
.evolve-empty { text-align: center; padding: 40px; color: var(--text-dim); font-size: 13px; }

/* ── 元智能体面板 ── */
.meta-card { background: var(--card); border: 1px solid var(--hairline); border-radius: var(--radius-md); padding: 16px 18px; margin-top: 18px; }
.meta-card__head { display: flex; align-items: center; gap: 10px; }
.meta-card__title { font-size: 14px; font-weight: 700; }
.meta-pill { font-size: 10px; font-weight: 700; padding: 3px 10px; border-radius: var(--radius-pill); }
.meta-pill--safe { background: color-mix(in srgb, var(--ok) 15%, transparent); color: var(--ok); }
.meta-pill--warn { background: color-mix(in srgb, var(--warn) 18%, transparent); color: var(--warn); }
.meta-grid { display: grid; grid-template-columns: repeat(4, 1fr); gap: 10px 14px; margin: 14px 0 0; }
.meta-grid dt { font-size: 10px; color: var(--text-dim); }
.meta-grid dd { margin: 2px 0 0; font-size: 13px; font-weight: 600; }
.meta-actions { display: flex; gap: 8px; margin-top: 14px; flex-wrap: wrap; }
.meta-hint { font-size: 11px; color: var(--text-dim); margin: 12px 0 0; line-height: 1.6; }
.meta-hint code { font-size: 10px; }
.btn--danger { color: var(--danger); }
.meta-section { margin-top: 22px; }
.meta-section__title { font-size: 13px; font-weight: 700; margin: 0 0 10px; }
.meta-row { background: var(--card); border: 1px solid var(--hairline); border-radius: var(--radius-md); padding: 12px 14px; margin-bottom: 10px; }
.meta-row--tight { padding: 10px 12px; }
.meta-row__head { display: flex; align-items: center; gap: 8px; }
.meta-row__ver { font-weight: 700; font-size: 13px; }
.meta-row__badge { font-size: 10px; padding: 2px 8px; border-radius: var(--radius-pill); background: color-mix(in srgb, var(--text-dim) 12%, transparent); color: var(--text-muted); }
.meta-row__badge--risk { background: color-mix(in srgb, var(--warn) 14%, transparent); color: var(--warn); }
.meta-row__time { margin-left: auto; font-size: 10px; color: var(--text-dim); }
.meta-row__text { font-size: 12px; color: var(--text); margin: 8px 0; line-height: 1.55; }
.meta-row__foot { display: flex; align-items: center; gap: 12px; }
.meta-row__dim { font-size: 11px; color: var(--text-dim); }
.meta-row__btns { margin-left: auto; display: flex; gap: 8px; }
.sig-list { list-style: none; margin: 0; padding: 0; }
.sig-item { display: flex; align-items: baseline; gap: 8px; padding: 7px 10px; border-bottom: 1px solid var(--hairline); font-size: 11px; }
.sig-item:last-child { border-bottom: none; }
.sig-kind { flex: 0 0 auto; font-weight: 700; color: var(--text-muted); }
.sig-tool { flex: 0 0 auto; color: var(--text-dim); font-family: var(--font-mono, monospace); font-size: 10px; }
.sig-detail { flex: 1 1 auto; color: var(--text-muted); word-break: break-word; }
</style>