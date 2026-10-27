<template>
  <div class="control-stage harness-stage">
    <header class="control-stage__header">
      <div class="control-stage__title-block">
        <h1 class="control-stage__title">{{ t('harness.title') }}</h1>
        <p class="control-stage__subtitle">{{ t('harness.subtitle') }}</p>
      </div>
      <div class="control-stage__actions">
        <button class="btn btn--ghost" :disabled="busy" @click="loadAll">
          <Icon name="refresh" :size="16" /><span>{{ t('harness.refresh') }}</span>
        </button>
      </div>
    </header>

    <!-- KPI：台账不可用时六项全部显示「—」，不放任何示例值 -->
    <section class="harness-kpis">
      <div v-for="card in kpiCards" :id="`harness-kpi-${card.key}`" :key="card.key" class="harness-kpi">
        <div class="harness-kpi__label">{{ t(`harness.kpi.${card.key}`) }}</div>
        <div class="harness-kpi__value">{{ card.value }}</div>
      </div>
    </section>

    <!-- 不可读原因就地展示（不弹 toast、不把连接状态当错误） -->
    <div v-if="statusReason" class="harness-note harness-note--warn">{{ statusReason }}</div>

    <!-- 任务复盘：只读 sessions.db 的聚合（任务类型 / 工具调度 / 思考正文） -->
    <section class="harness-card">
      <div class="harness-card__head">
        <div class="harness-card__title">{{ t('harness.insights.title') }}</div>
        <div class="harness-controls">
          <label class="harness-control">
            <span>{{ t('harness.insights.window') }}</span>
            <select v-model.number="insightsLimit" class="harness-select" @change="loadInsights">
              <option v-for="n in INSIGHTS_LIMIT_OPTIONS" :key="n" :value="n">{{ n }}</option>
            </select>
          </label>
        </div>
      </div>

      <div v-if="insightsNote" class="harness-note">{{ insightsNote }}</div>
      <template v-else-if="insights">
        <div class="harness-pills">
          <span class="harness-pill">{{ t('harness.insights.sessions', { count: formatTokens(insights.totals?.sessions ?? 0) }) }}</span>
          <span class="harness-pill">{{ t('harness.insights.turns', { count: formatTokens(insights.totals?.assistantTurns ?? 0) }) }}</span>
          <span class="harness-pill">{{ t('harness.insights.toolCalls', { count: formatTokens(insights.totals?.toolCalls ?? 0) }) }}</span>
          <span class="harness-pill">{{ t('harness.insights.toolFailures', { count: formatTokens(insightFailures) }) }}</span>
          <span class="harness-pill">{{ t('harness.insights.thinking', { percent: formatPercent(insightThinkingShare) }) }}</span>
        </div>

        <div class="harness-card__subtitle">{{ t('harness.insights.taskTypes') }}</div>
        <table class="harness-table">
          <thead>
            <tr>
              <th>{{ t('harness.insights.kindLabel') }}</th>
              <th>{{ t('harness.insights.count') }}</th>
              <th>{{ t('harness.insights.share') }}</th>
            </tr>
          </thead>
          <tbody>
            <tr v-for="row in insightTaskTypes" :key="row.key">
              <td>{{ t(`harness.insights.kind.${row.key}`) }}</td>
              <td class="harness-num">{{ row.count }}</td>
              <td class="harness-num">{{ formatPercent(row.share) }}</td>
            </tr>
          </tbody>
        </table>

        <div class="harness-card__subtitle">{{ t('harness.insights.tools') }}</div>
        <div v-if="!insightTools.length" class="harness-note">{{ t('harness.insights.noTools') }}</div>
        <table v-else class="harness-table">
          <thead>
            <tr>
              <th>{{ t('harness.insights.tool') }}</th>
              <th>{{ t('harness.insights.count') }}</th>
              <th>{{ t('harness.insights.failures') }}</th>
              <th>{{ t('harness.insights.unknown') }}</th>
              <th>{{ t('harness.insights.avg') }}</th>
              <th>p95</th>
            </tr>
          </thead>
          <tbody>
            <tr v-for="tool in insightTools" :key="tool.name">
              <td><code class="harness-mono">{{ tool.name }}</code></td>
              <td class="harness-num">{{ tool.calls ?? 0 }}</td>
              <td class="harness-num">{{ tool.failures ?? 0 }}</td>
              <td class="harness-num">{{ tool.unknown ?? 0 }}</td>
              <td class="harness-num">{{ formatDuration(tool.avgMs) }}</td>
              <td class="harness-num">{{ formatDuration(tool.p95Ms) }}</td>
            </tr>
          </tbody>
        </table>

        <div class="harness-card__subtitle">{{ t('harness.insights.sessionsTitle') }}</div>
        <div v-if="!insightSessions.length" class="harness-note">{{ t('harness.insights.noSessions') }}</div>
        <table v-else class="harness-table">
          <thead>
            <tr>
              <th>{{ t('harness.insights.session') }}</th>
              <th>{{ t('harness.insights.messages') }}</th>
              <th>{{ t('harness.insights.toolCallsShort') }}</th>
              <th>{{ t('harness.insights.thinkingChars') }}</th>
              <th>{{ t('harness.insights.lastTool') }}</th>
            </tr>
          </thead>
          <tbody>
            <tr v-for="row in insightSessions" :key="row.key">
              <td>
                <code class="harness-mono">{{ row.key }}</code>
                <div class="harness-note">{{ row.title }}</div>
              </td>
              <td class="harness-num">{{ row.messages ?? 0 }}</td>
              <td class="harness-num">{{ row.toolCalls ?? 0 }}</td>
              <td class="harness-num">{{ formatTokens(row.thinkingChars) }}</td>
              <td><code class="harness-mono">{{ row.lastTool || '—' }}</code></td>
            </tr>
          </tbody>
        </table>

        <ul v-if="insightGapList.length" class="harness-gaps">
          <li v-for="gap in insightGapList" :key="gap" class="harness-gaps__row">{{ gap }}</li>
        </ul>
      </template>
    </section>

    <section class="harness-card">
      <div class="harness-card__head">
        <div class="harness-card__title">{{ t('harness.runs.title') }}</div>
        <div class="harness-controls">
          <label class="harness-control">
            <span>{{ t('harness.runs.session') }}</span>
            <select v-model="sessionFilter" class="harness-select">
              <option value="">{{ t('harness.allSessions') }}</option>
              <option v-for="key in sessions" :key="key" :value="key">{{ key }}</option>
            </select>
          </label>
          <label class="harness-control">
            <span>{{ t('harness.limit') }}</span>
            <select v-model.number="limit" class="harness-select" @change="loadQuery">
              <option v-for="n in LIMIT_OPTIONS" :key="n" :value="n">{{ n }}</option>
            </select>
          </label>
        </div>
      </div>

      <div v-if="queryUnavailable" class="harness-note">{{ queryReason }}</div>
      <template v-else-if="query">
        <div v-if="!runs.length" class="harness-note">{{ t('harness.runs.empty') }}</div>
        <div v-else-if="!shownRuns.length" class="harness-note">{{ t('harness.runs.empty') }}</div>
        <template v-else>
          <table class="harness-table">
            <thead>
              <tr>
                <th>{{ t('harness.runs.runId') }}</th>
                <th>{{ t('harness.runs.session') }}</th>
                <th>{{ t('harness.runs.target') }}</th>
                <th>{{ t('harness.runs.started') }}</th>
                <th>{{ t('harness.runs.status') }}</th>
                <th>{{ t('harness.runs.verdict') }}</th>
                <th>{{ t('harness.runs.duration') }}</th>
                <th>{{ t('harness.runs.items') }}</th>
              </tr>
            </thead>
            <tbody>
              <tr
                v-for="run in shownRuns"
                :id="`harness-run-${run.runId}`"
                :key="run.runId"
                class="harness-row"
                :class="{ 'harness-row--active': selectedRun?.runId === run.runId }"
                @click="selectRunRow(run.runId)"
              >
                <td><code class="harness-mono">{{ run.runId }}</code></td>
                <td><code class="harness-mono">{{ run.sessionKey }}</code></td>
                <td>{{ run.target }}</td>
                <td class="harness-num">{{ formatTimestamp(run.startedAtMs) }}</td>
                <td>
                  <span class="harness-badge" :class="badgeClass(run.status)">{{ statusLabel(run.status) }}</span>
                </td>
                <td>
                  <span
                    class="harness-badge"
                    :class="verdictClass(run.verdict)"
                    :title="t('harness.verdict.hint')"
                  >{{ verdictLabel(run.verdict) }}</span>
                </td>
                <td class="harness-num">{{ formatDuration(run.elapsedMs) }}</td>
                <td class="harness-num">{{ resolveItemCount(run) }}</td>
              </tr>
            </tbody>
          </table>
          <div v-if="truncated" class="harness-note">{{ t('harness.runs.truncated', { count: limit }) }}</div>
        </template>
      </template>
    </section>

    <section class="harness-card">
      <div class="harness-card__head">
        <div class="harness-card__title">{{ t('harness.detail.title') }}</div>
        <button class="btn btn--sm btn--ghost" :disabled="busy || !selectedRun || replayBusy" @click="runReplay">
          <Icon name="refresh" :size="14" /><span>{{ t('harness.detail.replay') }}</span>
        </button>
      </div>

      <div v-if="!selectedRun" class="harness-note">{{ t('harness.detail.empty') }}</div>
      <template v-else>
        <div class="harness-meta">
          <span class="harness-meta__item">
            <b>{{ t('harness.runs.runId') }}</b><code class="harness-mono">{{ selectedRun.runId }}</code>
          </span>
          <span class="harness-meta__item">
            <b>{{ t('harness.runs.session') }}</b><code class="harness-mono">{{ selectedRun.sessionKey }}</code>
          </span>
          <span class="harness-meta__item">
            <b>{{ t('harness.runs.target') }}</b>{{ selectedRun.target }}
          </span>
          <span class="harness-meta__item">
            <b>{{ t('harness.runs.started') }}</b>{{ formatTimestamp(selectedRun.startedAtMs) }}
          </span>
          <span class="harness-meta__item">
            <b>{{ t('harness.runs.status') }}</b>
            <span class="harness-badge" :class="badgeClass(selectedRun.status)">{{ statusLabel(selectedRun.status) }}</span>
          </span>
          <span class="harness-meta__item">
            <b>{{ t('harness.runs.verdict') }}</b>
            <span class="harness-badge" :class="verdictClass(selectedRun.verdict)">{{ verdictLabel(selectedRun.verdict) }}</span>
          </span>
          <span class="harness-meta__item">
            <b>{{ t('harness.runs.duration') }}</b>{{ formatDuration(selectedRun.elapsedMs) }}
          </span>
          <span class="harness-meta__item">
            <b>{{ t('harness.kpi.tokens') }}</b>{{ runTokens(selectedRun) }}
          </span>
        </div>

        <div class="harness-replay">
          <div class="harness-card__subtitle">{{ t('harness.detail.digest') }}</div>
          <div v-if="replayError" class="harness-note">{{ replayError }}</div>
          <div v-else-if="!digest" class="harness-note">{{ t('harness.detail.noDigest') }}</div>
          <div v-else class="harness-replay__row">
            <code class="harness-digest">{{ digest }}</code>
            <button class="btn btn--sm btn--ghost" @click="copyDigest">
              <Icon :name="copied ? 'check' : 'copy'" :size="14" />
              <span>{{ copied ? t('harness.detail.copied') : t('harness.detail.copy') }}</span>
            </button>
          </div>
          <div v-if="replay && replay.ok && replay.deterministic" class="harness-note">
            {{ t('harness.note.replayDeterministic', { count: replay.itemCount }) }}
          </div>
        </div>

        <div class="harness-card__subtitle">{{ t('harness.detail.timeline') }}</div>
        <div v-if="!items.length" class="harness-note">{{ t('harness.detail.empty') }}</div>
        <table v-else class="harness-table">
          <thead>
            <tr>
              <th>#</th>
              <th>{{ t('harness.detail.itemName') }}</th>
              <th>{{ t('harness.detail.itemType') }}</th>
              <th>{{ t('harness.runs.status') }}</th>
              <th>{{ t('harness.runs.duration') }}</th>
              <th>{{ t('harness.kpi.tokens') }}</th>
            </tr>
          </thead>
          <tbody>
            <tr v-for="item in items" :key="item.seq">
              <td class="harness-num">{{ item.seq }}</td>
              <td><code class="harness-mono">{{ item.name }}</code></td>
              <td>{{ item.type }}</td>
              <td>
                <span class="harness-badge" :class="badgeClass(item.ok ? item.status : 'failed')">
                  {{ statusLabel(item.ok ? item.status : 'failed') }}
                </span>
                <span v-if="item.errorCode" class="harness-error-code">{{ item.errorCode }}</span>
              </td>
              <td class="harness-num">{{ formatDuration(item.elapsedMs) }}</td>
              <td class="harness-num">{{ formatTokens(item.tokensIn) }} / {{ formatTokens(item.tokensOut) }}</td>
            </tr>
          </tbody>
        </table>
      </template>
    </section>

    <section class="harness-card">
      <div class="harness-card__head">
        <div class="harness-card__title">{{ t('harness.skills.title') }}</div>
      </div>
      <div v-if="coverageNote" class="harness-note">{{ coverageNote }}</div>
      <template v-else-if="coverage && coverage.ok === true">
        <div class="harness-pills">
          <span class="harness-pill">{{ t('harness.skills.loaded', { count: formatTokens(coverage.loaded) }) }}</span>
          <span class="harness-pill">{{ t('harness.skills.directories', { count: formatTokens(coverage.directories) }) }}</span>
          <span class="harness-pill">{{ t('harness.skills.skillMd', { count: formatTokens(coverage.skillMd) }) }}</span>
          <span class="harness-pill">{{ t('harness.skills.yaml', { count: formatTokens(coverage.yaml) }) }}</span>
        </div>
        <div class="harness-note">{{ t('harness.skills.root', { path: coverage.root }) }}</div>
        <div v-if="skipped.length" class="harness-skipped">
          <button class="btn btn--sm btn--ghost" :aria-expanded="skippedOpen" @click="skippedOpen = !skippedOpen">
            <Icon name="logs" :size="14" />
            <span>{{ t('harness.skills.skipped', { count: skippedCount }) }}</span>
          </button>
          <ul v-if="skippedOpen" class="harness-skipped__list">
            <li v-for="entry in skipped" :key="entry.path" class="harness-skipped__row">
              <code class="harness-mono">{{ entry.path }}</code>
              <span class="harness-skipped__reason">{{ entry.reason }}</span>
            </li>
          </ul>
        </div>
      </template>
    </section>
  </div>
</template>

<script setup lang="ts">
/**
 * Harness 运行台账 —— 只读视图。
 *
 * 数据全部来自真实 RPC（harness.ledger.status/query/replay、skills.coverage），
 * 页面内不写任何示例数据：
 *   · 无记录 → 「暂无运行记录」；
 *   · 台账不可用（ok:false / 调用失败）→ 就地显示可读原因，不弹错误 toast，
 *     也不把 WS 连接状态渲染成错误（用户偏好连接状态不可见）；
 *   · KPI 在不可用时显示「—」而不是 0。
 *
 * 所有格式化、排序、过滤、KPI 派生与状态语义映射都由 ./harness/harnessViewModel.ts
 * 的纯函数完成，本文件只做调用编排与渲染。
 */
import { computed, onMounted, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import Icon from '@/components/Icon.vue'
import { useRpcStore } from '@/stores/rpc'
import { copyTextWithFallback } from '@/utils/browser'
import {
  LIMIT_OPTIONS,
  KPI_KEYS,
  coverageSkipped,
  coverageSkippedTotal,
  deriveKpis,
  formatDuration,
  formatPercent,
  formatTimestamp,
  formatTokens,
  payloadReason,
  replayDigest,
  resolveItemCount,
  runTokens,
  selectRun,
  sessionOptions,
  sortItems,
  statusRole,
  visibleRuns,
  type LedgerQueryPayload,
  type LedgerReplayPayload,
  type LedgerRun,
  type LedgerStatusPayload,
  type SkillsCoveragePayload,
  type StatusRole,
} from './harness/harnessViewModel'
import {
  insightGaps,
  taskTypeRows,
  thinkingShare,
  toolFailureTotal,
  topSessions,
  topTools,
  type InsightsPayload,
} from './harness/insightsViewModel'

/** 复盘时间窗（最近 N 个会话）；与 harness runs 的 limit 解耦。 */
const INSIGHTS_LIMIT_OPTIONS = [20, 50, 100, 200] as const

const { t } = useI18n()
const rpc = useRpcStore()

const status = ref<LedgerStatusPayload | null>(null)
const query = ref<LedgerQueryPayload | null>(null)
const coverage = ref<SkillsCoveragePayload | null>(null)
const statusError = ref('')
const queryError = ref('')
const coverageError = ref('')
const busy = ref(false)
const replayBusy = ref(false)
const replay = ref<LedgerReplayPayload | null>(null)
const replayError = ref('')
const selectedRunId = ref('')
const sessionFilter = ref('')
const limit = ref<number>(25)
const copied = ref(false)
const skippedOpen = ref(false)
const insights = ref<InsightsPayload | null>(null)
const insightsError = ref('')
const insightsLimit = ref<number>(50)

const kpis = computed(() => deriveKpis(status.value))
const kpiCards = computed(() => KPI_KEYS.map((key) => ({ key, value: kpis.value[key] })))

const statusReason = computed(() => {
  if (statusError.value) return statusError.value
  if (status.value && status.value.ok !== true) {
    return payloadReason(status.value) || t('harness.error.unavailable')
  }
  return ''
})

const runs = computed<LedgerRun[]>(() =>
  query.value && query.value.ok === true && Array.isArray(query.value.runs) ? query.value.runs : [],
)
const shownRuns = computed(() => visibleRuns(runs.value, sessionFilter.value))
const sessions = computed(() => sessionOptions(runs.value))
const queryUnavailable = computed(
  () => Boolean(queryError.value) || (query.value !== null && query.value.ok !== true),
)
const queryReason = computed(
  () => queryError.value || payloadReason(query.value) || t('harness.runs.unavailable'),
)
const truncated = computed(() => query.value?.ok === true && query.value.truncated === true)
const selectedRun = computed(() => selectRun(shownRuns.value, selectedRunId.value))
const items = computed(() => sortItems(selectedRun.value?.items))
const digest = computed(() => replayDigest(replay.value))

const coverageNote = computed(() => {
  if (coverageError.value) return coverageError.value
  if (coverage.value && coverage.value.ok !== true) {
    return payloadReason(coverage.value) || t('harness.skills.unavailable')
  }
  return ''
})
const skipped = computed(() => coverageSkipped(coverage.value))
const skippedCount = computed(() => coverageSkippedTotal(coverage.value))

// ── 任务复盘（sessions.insights）──────────────────────────────────
const insightsNote = computed(() => {
  if (insightsError.value) return insightsError.value
  if (insights.value && insights.value.ok !== true) {
    return payloadReason(insights.value) || t('harness.insights.unavailable')
  }
  return ''
})
const insightTaskTypes = computed(() => taskTypeRows(insights.value))
const insightTools = computed(() => topTools(insights.value))
const insightSessions = computed(() => topSessions(insights.value))
const insightFailures = computed(() => toolFailureTotal(insights.value))
const insightThinkingShare = computed(() => thinkingShare(insights.value))
const insightGapList = computed(() => insightGaps(insights.value))

const STATUS_KEYS: Record<Exclude<StatusRole, 'other'>, string> = {
  ok: 'harness.status.ok',
  failed: 'harness.status.failed',
  running: 'harness.status.running',
  cancelled: 'harness.status.cancelled',
}

function statusLabel(raw: string): string {
  const role = statusRole(raw)
  return role === 'other' ? raw || '—' : t(STATUS_KEYS[role])
}

function badgeClass(raw: string): string {
  return `harness-badge--${statusRole(raw)}`
}

/**
 * 验收结论 → 徽章角色。
 *
 * 只有 `passed` 才允许显示成绿色；`unverified` / `unknown` 一律灰，缺省值也按
 * `unverified` 处理——绝不能因为「跑完了」就把徽章画成通过（真机最容易踩的坑）。
 */
const VERDICT_ROLE: Record<string, StatusRole> = {
  passed: 'ok',
  failed: 'failed',
  unverified: 'other',
  unknown: 'other',
}

function verdictClass(raw: string | undefined): string {
  const role = VERDICT_ROLE[raw ?? 'unverified'] ?? 'other'
  return `harness-badge--${role}`
}

function verdictLabel(raw: string | undefined): string {
  const key = raw && raw in VERDICT_ROLE ? raw : 'unverified'
  return t(`harness.verdict.${key}`)
}

/** 选中项变化时清掉旧 replay 摘要，避免摘要与当前 run 对不上。 */
watch(
  () => selectedRun.value?.runId,
  () => resetReplay(),
)

function resetReplay(): void {
  replay.value = null
  replayError.value = ''
  copied.value = false
}

function selectRunRow(runId: string): void {
  selectedRunId.value = runId
}

async function loadStatus(): Promise<void> {
  statusError.value = ''
  try {
    const payload = await rpc.call<LedgerStatusPayload>('harness.ledger.status', {})
    status.value = payload ?? null
  } catch {
    // 连接未就绪 / 方法不可达：如实降级为不可用，不泄露连接状态措辞。
    status.value = null
    statusError.value = t('harness.error.unavailable')
  }
}

async function loadQuery(): Promise<void> {
  queryError.value = ''
  resetReplay()
  try {
    const payload = await rpc.call<LedgerQueryPayload>('harness.ledger.query', { limit: limit.value })
    query.value = payload ?? null
    const next = selectRun(payload && payload.ok === true ? payload.runs ?? [] : [], selectedRunId.value)
    selectedRunId.value = next?.runId ?? ''
  } catch {
    query.value = null
    queryError.value = t('harness.runs.unavailable')
  }
}

async function loadCoverage(): Promise<void> {
  coverageError.value = ''
  try {
    coverage.value = (await rpc.call<SkillsCoveragePayload>('skills.coverage', {})) ?? null
  } catch {
    coverage.value = null
    coverageError.value = t('harness.skills.unavailable')
  }
}

async function loadAll(): Promise<void> {
  if (busy.value) return
  busy.value = true
  try {
    await Promise.all([loadStatus(), loadQuery(), loadCoverage(), loadInsights()])
  } finally {
    busy.value = false
  }
}

async function loadInsights(): Promise<void> {
  insightsError.value = ''
  try {
    const payload = await rpc.call<InsightsPayload>('sessions.insights', {
      limit: insightsLimit.value,
    })
    insights.value = payload ?? null
  } catch {
    insights.value = null
    insightsError.value = t('harness.insights.unavailable')
  }
}

async function runReplay(): Promise<void> {
  const run = selectedRun.value
  if (!run || replayBusy.value) return
  replayBusy.value = true
  replayError.value = ''
  copied.value = false
  try {
    const payload = await rpc.call<LedgerReplayPayload>('harness.ledger.replay', { runId: run.runId })
    if (payload && payload.ok === true) {
      replay.value = payload
    } else {
      replay.value = null
      replayError.value = payloadReason(payload) || t('harness.detail.replayFailed')
    }
  } catch {
    replay.value = null
    replayError.value = t('harness.detail.replayFailed')
  } finally {
    replayBusy.value = false
  }
}

async function copyDigest(): Promise<void> {
  if (!digest.value) return
  try {
    await copyTextWithFallback(digest.value)
    copied.value = true
    window.setTimeout(() => { copied.value = false }, 2_000)
  } catch {
    copied.value = false
  }
}

onMounted(loadAll)
</script>

<style scoped>
.harness-stage { padding: 24px 28px; max-width: 1100px; margin: 0 auto; }
.harness-kpis { display: grid; grid-template-columns: repeat(auto-fit, minmax(150px, 1fr)); gap: 10px; margin-top: 16px; }
.harness-kpi { padding: 12px; background: var(--card); border: 1px solid var(--hairline); border-radius: var(--radius-md); }
.harness-kpi__label { font-size: 11px; color: var(--text-dim); }
.harness-kpi__value { margin-top: 4px; font-size: 18px; font-weight: 600; font-variant-numeric: tabular-nums; }
.harness-card { margin-top: 16px; padding: 14px; background: var(--card); border: 1px solid var(--hairline); border-radius: var(--radius-md); }
.harness-card__head { display: flex; flex-wrap: wrap; gap: 10px; align-items: center; justify-content: space-between; }
.harness-card__title { font-size: 13px; font-weight: 600; }
.harness-card__subtitle { margin-top: 12px; font-size: 12px; font-weight: 600; color: var(--text-dim); }
.harness-controls { display: flex; flex-wrap: wrap; gap: 10px; align-items: center; }
.harness-control { display: inline-flex; gap: 6px; align-items: center; font-size: 11px; color: var(--text-dim); }
.harness-select { padding: 5px 8px; border: 1px solid var(--hairline); border-radius: var(--radius-sm); background: var(--bg); color: var(--text); font-size: 12px; }
.harness-note { margin-top: 8px; font-size: 12px; color: var(--text-dim); }
.harness-note--warn { padding: 8px 10px; border: 1px solid var(--hairline); border-radius: var(--radius-sm); }
.harness-table { width: 100%; border-collapse: collapse; font-size: 12px; margin-top: 10px; }
.harness-table th, .harness-table td { text-align: left; padding: 7px 8px; border-bottom: 1px solid var(--hairline); vertical-align: top; }
.harness-row { cursor: pointer; }
.harness-row:hover { background: var(--bg-hover); }
.harness-row--active { background: color-mix(in srgb, var(--accent) 8%, var(--card)); }
.harness-num { font-variant-numeric: tabular-nums; white-space: nowrap; }
.harness-mono { font-family: var(--font-mono); font-size: 11px; word-break: break-all; }
.harness-badge { display: inline-block; padding: 2px 8px; border-radius: 999px; font-size: 11px; font-weight: 600; }
.harness-badge--ok { background: color-mix(in srgb, var(--ok) 14%, var(--card)); color: var(--ok); }
.harness-badge--failed { background: color-mix(in srgb, var(--danger) 14%, var(--card)); color: var(--danger); }
.harness-badge--running { background: color-mix(in srgb, var(--accent) 14%, var(--card)); color: var(--accent); }
.harness-badge--cancelled { background: color-mix(in srgb, var(--text) 8%, var(--card)); color: var(--text-dim); }
.harness-badge--other { background: color-mix(in srgb, var(--text) 8%, var(--card)); color: var(--text-dim); }
.harness-error-code { margin-left: 6px; font-family: var(--font-mono); font-size: 11px; color: var(--danger); }
.harness-meta { display: flex; flex-wrap: wrap; gap: 8px 16px; margin-top: 10px; font-size: 12px; }
.harness-meta__item { display: inline-flex; gap: 6px; align-items: center; color: var(--text); }
.harness-meta__item b { font-weight: 600; color: var(--text-dim); }
.harness-replay { margin-top: 4px; }
.harness-replay__row { display: flex; flex-wrap: wrap; gap: 8px; align-items: center; margin-top: 8px; }
.harness-digest { padding: 6px 9px; border: 1px solid var(--hairline); border-radius: var(--radius-sm); background: var(--bg); font-family: var(--font-mono); font-size: 11px; word-break: break-all; }
.harness-pills { display: flex; flex-wrap: wrap; gap: 8px; margin-top: 10px; }
.harness-pill { padding: 4px 10px; border-radius: 999px; background: color-mix(in srgb, var(--text) 8%, var(--card)); font-size: 12px; }
.harness-skipped { margin-top: 10px; }
.harness-skipped__list { margin: 8px 0 0; padding: 0; list-style: none; }
.harness-skipped__row { display: flex; flex-wrap: wrap; gap: 8px; align-items: baseline; padding: 5px 0; border-bottom: 1px solid var(--hairline); font-size: 12px; }
.harness-skipped__reason { color: var(--text-dim); }
.harness-gaps { margin: 12px 0 0; padding: 0; list-style: none; }
.harness-gaps__row { padding: 5px 0; border-bottom: 1px solid var(--hairline); font-size: 12px; color: var(--text-dim); }
</style>
