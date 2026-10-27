<!--
  运行态面板（工作台）：一屏看清 Harness 侧的「决定 / 环境 / 约束 / 证据」。

  对应知识图 `Agent = Model + Harness`：模型推理本身不可内省，但 Harness 的
  模式规格、执行环境、上下文压力、工具面与质量结论都可以如实展示。

  设计原则（与全站一致）：
  · **只读**：这里不发起任何执行，不改变任何配置；
  · **不猜**：任一数据源不可用时显示「—」或「不可用」，不用 0 / 默认值填充；
  · **阈值只有一处**：上下文压力只认后端 `context.status.alert`，前端不另设阈值。
-->
<template>
  <div class="rt">
    <header class="rt__bar">
      <span class="rt__chip" :class="`is-${contextToneValue}`">
        {{ t('workbench.runtime.contextChip', {
          percent: formatPercent(percent),
          state: contextAlertLabel,
        }) }}
      </span>
      <span class="rt__chip is-muted">{{ modeLabel }}</span>
      <span class="rt__chip is-muted">{{ runModeLabel }}</span>
      <span class="rt__spacer" />
      <span v-if="loading" class="rt__hint">{{ t('workbench.runtime.loading') }}</span>
      <button type="button" class="rt__refresh" :disabled="loading" @click="load()">
        <Icon name="refresh" :size="13" aria-hidden="true" />
        <span>{{ t('workbench.runtime.refresh') }}</span>
      </button>
    </header>

    <p v-if="loadError" class="rt__error">{{ loadError }}</p>

    <div class="rt__scroll">
      <!-- 1. 决策（Control） -->
      <section class="rt__section">
        <h4 class="rt__title">{{ t('workbench.runtime.section.control') }}</h4>
        <dl class="rt__grid">
          <dt>{{ t('workbench.runtime.control.userMode') }}</dt>
          <dd>{{ userModeText }}</dd>
          <dt>{{ t('workbench.runtime.control.mode') }}</dt>
          <dd>{{ modeKindText }}</dd>
          <dt>{{ t('workbench.runtime.control.approval') }}</dt>
          <dd>{{ approvalText }}</dd>
          <dt>{{ t('workbench.runtime.control.orchestration') }}</dt>
          <dd>{{ orchestrationText }}</dd>
        </dl>
        <p class="rt__note">{{ t('workbench.runtime.control.note') }}</p>
      </section>

      <!-- 2. 执行环境（Runtime） -->
      <section class="rt__section">
        <h4 class="rt__title">{{ t('workbench.runtime.section.runtime') }}</h4>
        <dl class="rt__grid">
          <dt>{{ t('workbench.runtime.runtime.runMode') }}</dt>
          <dd>{{ runModeLabel }}</dd>
          <dt>{{ t('workbench.runtime.runtime.platform') }}</dt>
          <dd>{{ platformLabel }}</dd>
          <dt>{{ t('workbench.runtime.runtime.workspace') }}</dt>
          <dd class="rt__mono" :title="workspaceRoot">{{ workspaceRoot || '—' }}</dd>
          <dt>{{ t('workbench.runtime.runtime.git') }}</dt>
          <dd>{{ gitText }}</dd>
          <dt>{{ t('workbench.runtime.runtime.terminals') }}</dt>
          <dd>
            {{ terminalsValue.total
              ? t('workbench.runtime.runtime.terminalsValue', {
                running: terminalsValue.running,
                total: terminalsValue.total,
              })
              : t('workbench.runtime.runtime.noTerminals') }}
          </dd>
        </dl>
      </section>

      <!-- 3. 上下文工程 -->
      <section class="rt__section">
        <h4 class="rt__title">{{ t('workbench.runtime.section.context') }}</h4>
        <dl class="rt__grid">
          <dt>{{ t('workbench.runtime.context.usage') }}</dt>
          <dd>{{ contextUsageText }}</dd>
          <dt>{{ t('workbench.runtime.context.compressions') }}</dt>
          <dd>{{ compactionText }}</dd>
        </dl>
        <p class="rt__note">{{ t('workbench.runtime.context.note') }}</p>
      </section>

      <!-- 4. 工具面 -->
      <section class="rt__section">
        <h4 class="rt__title">{{ t('workbench.runtime.section.tools') }}</h4>
        <dl class="rt__grid">
          <dt>{{ t('workbench.runtime.tools.enabled') }}</dt>
          <dd>{{ toolText }}</dd>
        </dl>
        <p class="rt__note">{{ t('workbench.runtime.tools.note') }}</p>
      </section>

      <!-- 5. 质量门 -->
      <section class="rt__section">
        <h4 class="rt__title">{{ t('workbench.runtime.section.quality') }}</h4>
        <dl class="rt__grid">
          <dt>{{ t('workbench.runtime.quality.verdict') }}</dt>
          <dd>
            <span class="rt__verdict" :class="`is-${qualityTone}`">{{ verdictLabel }}</span>
          </dd>
          <dt>{{ t('workbench.runtime.quality.changes') }}</dt>
          <dd>{{ changeCount }}</dd>
          <dt>{{ t('workbench.runtime.quality.artifacts') }}</dt>
          <dd>{{ artifactCount }}</dd>
          <dt>{{ t('workbench.runtime.quality.tokens') }}</dt>
          <dd>{{ tokenText }}</dd>
        </dl>
        <p class="rt__note">{{ t('workbench.runtime.quality.note') }}</p>
      </section>
    </div>

    <footer class="rt__foot">{{ t('workbench.runtime.footnote') }}</footer>
  </div>
</template>

<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import Icon from '@/components/Icon.vue'
import { useRpcStore } from '@/stores/rpc'
import {
  bloomLimits,
  contextPercent,
  contextTone,
  formatPercent,
  formatTokens,
  terminalSummary,
  toolCounts,
  verdictTone,
  verificationVerdict,
} from './runtimeViewModel'

const props = withDefaults(defineProps<{
  scopeId: string
  /** 面板是否处于激活标签（非激活时不轮询） */
  active?: boolean
}>(), { active: true })

const { t } = useI18n()
const rpc = useRpcStore()

interface UserMode {
  label?: string
  intensityLabel?: string
  workingLabel?: string
  agentMode?: string
}
interface ModeGet {
  kind?: string
  runtimeKind?: string
  userMode?: UserMode
}
interface ModeSpec {
  label?: string
  approval_mutating?: string
  allowed_tools?: number
  denied_tools?: number
  bloom?: {
    max_swarm_shards?: number
    max_sub_experts?: number
    verification_passes?: number
    thinking_depth?: number
    effective_max_llm_calls?: number
  } | null
}
interface SandboxStatus {
  run_mode?: string
  platform?: string
  data_dir?: string
}
interface WorkspaceStatus {
  root?: string
  bound?: boolean
  git?: { branch?: string | null; detached?: boolean } | null
}
interface ContextStatus {
  used_tokens?: number
  budget?: number
  alert?: string
  compaction_count?: number
  last_mode?: string | null
}
interface ToolCatalog {
  items?: Array<{ name?: string; enabled?: boolean }>
  total?: number
}
interface TerminalRow {
  running?: boolean
  shellLabel?: string
}
interface Receipt {
  ok?: boolean
  counts?: { changes?: number; artifacts?: number }
  verification?: { status?: string }
  budget?: { tokensIn?: number | null; tokensOut?: number | null }
}

const loading = ref(false)
const loadError = ref('')
const mode = ref<ModeGet | null>(null)
const spec = ref<ModeSpec | null>(null)
const sandbox = ref<SandboxStatus | null>(null)
const workspace = ref<WorkspaceStatus | null>(null)
const contextStatus = ref<ContextStatus | null>(null)
const catalog = ref<ToolCatalog | null>(null)
const terminals = ref<TerminalRow[]>([])
const receipt = ref<Receipt | null>(null)

const percent = computed(() => contextPercent(contextStatus.value?.used_tokens, contextStatus.value?.budget))
const contextToneValue = computed(() => contextTone(contextStatus.value?.alert))
const contextAlertLabel = computed(() => {
  const alert = contextStatus.value?.alert
  if (!alert) return t('workbench.runtime.context.alertUnknown')
  return t(`workbench.runtime.context.alert.${alert}`)
})

const userModeText = computed(() => {
  const um = mode.value?.userMode
  if (!um?.label) return '—'
  const parts = [um.intensityLabel, um.workingLabel].filter(Boolean)
  return parts.length ? `${um.label}（${parts.join(' × ')}）` : um.label
})
const modeKindText = computed(() => mode.value?.kind || '—')
/** 顶栏胶囊用的短标签（长文案留给「决策」区的完整一行）。 */
const modeLabel = computed(() => mode.value?.userMode?.label || mode.value?.kind || '—')
const approvalText = computed(() => spec.value?.approval_mutating || '—')
const limits = computed(() => bloomLimits(spec.value))
const orchestrationText = computed(() => {
  const l = limits.value
  const parts: string[] = []
  if (l.shards !== null) parts.push(t('workbench.runtime.control.shards', { n: l.shards }))
  if (l.subExperts !== null) parts.push(t('workbench.runtime.control.subExperts', { n: l.subExperts }))
  if (l.verificationPasses !== null) parts.push(t('workbench.runtime.control.verify', { n: l.verificationPasses }))
  if (l.thinkingDepth !== null) parts.push(t('workbench.runtime.control.thinking', { n: l.thinkingDepth }))
  return parts.length ? parts.join(' · ') : t('workbench.runtime.control.noSpec')
})

const runModeLabel = computed(() => sandbox.value?.run_mode || '—')
const platformLabel = computed(() => sandbox.value?.platform || '—')
const workspaceRoot = computed(() => workspace.value?.root || '')
const gitText = computed(() => {
  const git = workspace.value?.git
  if (!git) return t('workbench.runtime.runtime.notRepo')
  if (git.detached) return t('workbench.runtime.runtime.detached')
  return git.branch || t('workbench.runtime.runtime.detached')
})
const terminalsValue = computed(() => terminalSummary(terminals.value))

const contextUsageText = computed(() => {
  const used = formatTokens(contextStatus.value?.used_tokens ?? null)
  const budget = formatTokens(contextStatus.value?.budget ?? null)
  if (used === '—' && budget === '—') return '—'
  return t('workbench.runtime.context.usageValue', {
    percent: formatPercent(percent.value),
    used,
    budget,
  })
})
const compactionText = computed(() => {
  const count = contextStatus.value?.compaction_count
  if (typeof count !== 'number') return '—'
  return t('workbench.runtime.context.compactionsValue', { n: count })
})

const tools = computed(() => toolCounts(catalog.value?.items))
const toolText = computed(() => {
  if (!catalog.value) return '—'
  return t('workbench.runtime.tools.value', {
    enabled: tools.value.enabled,
    total: tools.value.total,
    disabled: tools.value.disabled,
  })
})

const verdict = computed(() => verificationVerdict(receipt.value))
const qualityTone = computed(() => verdictTone(verdict.value))
const verdictLabel = computed(() => t(`workbench.runtime.quality.verdictValue.${verdict.value}`))
const changeCount = computed(() => {
  const n = receipt.value?.counts?.changes
  return typeof n === 'number' ? String(n) : '—'
})
const artifactCount = computed(() => {
  const n = receipt.value?.counts?.artifacts
  return typeof n === 'number' ? String(n) : '—'
})
const tokenText = computed(() => {
  const b = receipt.value?.budget
  if (!b) return '—'
  const inTok = formatTokens(b.tokensIn ?? null)
  const outTok = formatTokens(b.tokensOut ?? null)
  if (inTok === '—' && outTok === '—') return '—'
  return t('workbench.runtime.quality.tokensValue', { input: inTok, output: outTok })
})

async function ensureConnected(): Promise<boolean> {
  if (rpc.isConnected) return true
  try {
    await rpc.waitForConnection(10_000)
    return rpc.isConnected
  } catch {
    return false
  }
}

/** 单个数据源失败不拖垮整面；失败项保持 null，UI 显示「—」。 */
async function callSafe<T>(method: string, params?: Record<string, unknown>): Promise<T | null> {
  if (!rpc.supportsMethod(method)) return null
  try {
    return await rpc.call<T>(method, params)
  } catch {
    return null
  }
}

async function load(): Promise<void> {
  if (!(await ensureConnected())) {
    loadError.value = t('workbench.runtime.notConnected')
    return
  }
  loading.value = true
  loadError.value = ''
  try {
    const scope = props.scopeId.trim()
    const [m, s, sb, ws, ctx, cat, term, rc] = await Promise.all([
      callSafe<ModeGet>('mode.get'),
      callSafe<ModeSpec>('mode.spec'),
      callSafe<SandboxStatus>('sandbox.status'),
      callSafe<WorkspaceStatus>('workspace.status'),
      scope ? callSafe<ContextStatus>('context.status', { session: scope }) : Promise.resolve(null),
      callSafe<ToolCatalog>('tools.catalog'),
      callSafe<{ terminals?: TerminalRow[] }>('sandbox.terminal.list'),
      scope ? callSafe<Receipt>('task.receipt', { sessionKey: scope, limit: 20 }) : Promise.resolve(null),
    ])
    mode.value = m
    spec.value = s
    sandbox.value = sb
    workspace.value = ws
    contextStatus.value = ctx
    catalog.value = cat
    terminals.value = Array.isArray(term?.terminals) ? term!.terminals! : []
    receipt.value = rc && rc.ok !== false ? rc : null
  } catch (err) {
    loadError.value = err instanceof Error ? err.message : String(err)
  } finally {
    loading.value = false
  }
}

let timer: number | null = null
function startPolling(): void {
  stopPolling()
  // 只刷便宜的状态：模式/环境变更低频，上下文压力会随对话变化。
  timer = window.setInterval(() => {
    if (props.active !== false) void load()
  }, 6000)
}
function stopPolling(): void {
  if (timer !== null) {
    window.clearInterval(timer)
    timer = null
  }
}

watch(() => props.scopeId, () => { void load() })
watch(() => props.active, on => { if (on) void load() })

onMounted(() => {
  void load()
  startPolling()
})

onBeforeUnmount(() => { stopPolling() })
</script>

<style scoped>
.rt {
  display: flex;
  height: 100%;
  min-height: 0;
  flex-direction: column;
  background: var(--bg-surface);
  color: var(--text);
  font-size: var(--fs-sm);
}

.rt__bar {
  display: flex;
  flex: 0 0 auto;
  align-items: center;
  gap: 6px;
  padding: 6px 10px;
  border-bottom: 1px solid var(--border);
  background: var(--bg-elevated, var(--bg-surface));
  overflow: hidden;
}

.rt__chip {
  flex: 0 0 auto;
  padding: 1px 8px;
  border-radius: var(--radius-pill);
  background: var(--bg-hover);
  color: var(--text-muted);
  font-size: 10.5px;
  white-space: nowrap;
}

.rt__chip.is-ok { background: color-mix(in srgb, var(--ok) 15%, transparent); color: var(--ok); }
.rt__chip.is-warn { background: color-mix(in srgb, var(--warn) 16%, transparent); color: var(--warn); }
.rt__chip.is-danger { background: color-mix(in srgb, var(--danger) 15%, transparent); color: var(--danger); }
.rt__chip.is-muted { color: var(--text-muted); }

.rt__spacer { flex: 1 1 auto; }
.rt__hint { color: var(--text-dim); font-size: 10.5px; }

.rt__refresh {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  padding: 3px 8px;
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  background: transparent;
  color: var(--text-muted);
  cursor: pointer;
  font: inherit;
  font-size: 10.5px;
}

.rt__refresh:hover:not(:disabled) { background: var(--bg-hover); color: var(--text); }
.rt__refresh:disabled { opacity: 0.5; cursor: not-allowed; }

.rt__error {
  margin: 8px 12px 0;
  color: var(--danger);
  font-size: var(--fs-xs);
}

.rt__scroll {
  flex: 1 1 auto;
  min-height: 0;
  overflow: auto;
  padding: 10px 12px 16px;
}

.rt__section { margin-bottom: 14px; }

.rt__title {
  margin: 0 0 6px;
  color: var(--text-dim);
  font-size: 10.5px;
  font-weight: 600;
  letter-spacing: 0.03em;
  text-transform: uppercase;
}

.rt__grid {
  display: grid;
  grid-template-columns: minmax(88px, max-content) 1fr;
  gap: 4px 10px;
  margin: 0;
  font-size: var(--fs-xs);
}

.rt__grid dt { color: var(--text-dim); }
.rt__grid dd {
  margin: 0;
  min-width: 0;
  color: var(--text);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.rt__mono { font-family: var(--font-mono, ui-monospace, SFMono-Regular, Menlo, Consolas, monospace); }

.rt__note {
  margin: 6px 0 0;
  color: var(--text-muted);
  font-size: 10.5px;
  line-height: 1.55;
}

.rt__verdict {
  padding: 1px 7px;
  border-radius: var(--radius-pill);
  background: var(--bg-hover);
  color: var(--text-muted);
  font-size: 10.5px;
}

.rt__verdict.is-ok { background: color-mix(in srgb, var(--ok) 15%, transparent); color: var(--ok); }
.rt__verdict.is-warn { background: color-mix(in srgb, var(--warn) 16%, transparent); color: var(--warn); }
.rt__verdict.is-danger { background: color-mix(in srgb, var(--danger) 15%, transparent); color: var(--danger); }

.rt__foot {
  flex: 0 0 auto;
  padding: 6px 12px;
  border-top: 1px solid var(--border);
  color: var(--text-dim);
  font-size: 10.5px;
  line-height: 1.5;
}
</style>
