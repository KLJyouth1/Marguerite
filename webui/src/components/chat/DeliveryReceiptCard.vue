<!--
  交付回执卡片（FR-07）。

  一回合结束后回答四个问题：改了什么 / 产出了什么 / 验证了什么 / 花了多少。
  所有数字都来自后端台账（`task.receipt`），**没有验证记录就显示「未验证」**，
  不用模型自述冒充证据。

  设计取舍：
  · 默认折叠：正常回合只占一行，不打断阅读；
  · 验证结论用颜色 + 文字双编码（不只靠颜色区分）；
  · 列表里出现的产物会标注「磁盘上是否还在」，登记过但已消失的会单独点名；
  · 后端不可用时降级为一行说明，不弹 toast、不伪造数据。
-->
<template>
  <section v-if="visible" class="receipt" :class="`receipt--${tone}`">
    <button
      type="button"
      class="receipt__head"
      :aria-expanded="open"
      :title="scopeTitle"
      @click="open = !open"
    >
      <Icon name="listChecks" :size="13" aria-hidden="true" />
      <span class="receipt__title">{{ t('chat.receipt.title') }}</span>
      <span class="receipt__verdict">{{ verdictLabel }}</span>
      <span v-if="summaryText" class="receipt__summary">{{ summaryText }}</span>
      <span class="receipt__spacer" />
      <span v-if="loading" class="receipt__hint">{{ t('chat.receipt.loading') }}</span>
      <Icon :name="open ? 'chevronDown' : 'chevronRight'" :size="13" aria-hidden="true" />
    </button>

    <p v-if="loadError" class="receipt__note">{{ loadError }}</p>

    <div v-else-if="open && receipt" class="receipt__body">
      <!-- 验证 -->
      <div class="receipt__section">
        <h4 class="receipt__label">{{ t('chat.receipt.verification') }}</h4>
        <p v-if="!checks.length" class="receipt__note">{{ t('chat.receipt.noVerification') }}</p>
        <ul v-else class="receipt__list">
          <li v-for="(check, i) in checks" :key="`v-${i}`" class="receipt__row">
            <span class="receipt__dot" :class="check.ok === true ? 'is-ok' : check.ok === false ? 'is-fail' : 'is-unknown'" />
            <code class="receipt__cmd">{{ check.command }}</code>
            <span class="receipt__meta">
              {{ check.ok === true
                ? t('chat.receipt.checkPassed')
                : check.ok === false
                  ? t('chat.receipt.checkFailed')
                  : t('chat.receipt.checkUnknown') }}
              <template v-if="check.elapsedMs > 0"> · {{ formatDuration(check.elapsedMs) }}</template>
            </span>
          </li>
        </ul>
      </div>

      <!-- 改动：每一行都可点开内联 diff（+绿 / -红 / hunk 中性 / 黄色 gutter）。 -->
      <div v-if="changes.length" class="receipt__section">
        <h4 class="receipt__label">{{ t('chat.receipt.changes', { n: changes.length }) }}</h4>
        <ul class="receipt__list">
          <li v-for="(change, i) in changes" :key="`c-${i}`">
            <button
              type="button"
              class="receipt__row receipt__row--change"
              :class="{ 'is-open': openedChangePath === change.path }"
              :aria-expanded="openedChangePath === change.path"
              data-testid="receipt-change-row"
              @click="toggleChange(change.path)"
            >
              <span class="receipt__kind" :class="`is-${change.kind}`">{{ kindLabel(change.kind) }}</span>
              <code class="receipt__path">{{ change.path }}</code>
              <span class="receipt__meta">{{ change.tool }}</span>
              <Icon
                :name="openedChangePath === change.path ? 'chevronDown' : 'chevronRight'"
                :size="11"
                aria-hidden="true"
              />
            </button>
          </li>
        </ul>
        <FileChangeDiffViewer
          v-if="openedChangePath"
          :session-key="sessionKey"
          :only-path="openedChangePath"
          :initial-open-path="openedChangePath"
          embedded
        />
      </div>

      <!-- 产物 -->
      <div v-if="artifacts.length" class="receipt__section">
        <h4 class="receipt__label">{{ t('chat.receipt.artifacts', { n: artifacts.length }) }}</h4>
        <ul class="receipt__list">
          <li v-for="(artifact, i) in artifacts" :key="`a-${i}`" class="receipt__row">
            <span class="receipt__dot" :class="artifact.present ? 'is-ok' : 'is-fail'" />
            <code class="receipt__path">{{ artifact.name }}</code>
            <span class="receipt__meta">
              {{ formatBytes(artifact.size) }}
              <template v-if="!artifact.present"> · {{ t('chat.receipt.missingOnDisk') }}</template>
            </span>
          </li>
        </ul>
      </div>

      <!-- 用量 -->
      <div class="receipt__section receipt__section--budget">
        <h4 class="receipt__label">{{ t('chat.receipt.budget') }}</h4>
        <p class="receipt__note">
          {{ budgetText }}
        </p>
      </div>

      <!-- 未证实项 -->
      <div v-if="gaps.length" class="receipt__section">
        <h4 class="receipt__label">{{ t('chat.receipt.unattested') }}</h4>
        <ul class="receipt__list">
          <li v-for="(gap, i) in gaps" :key="`g-${i}`" class="receipt__row receipt__row--gap">
            <span class="receipt__dot is-warn" />
            <span class="receipt__gap">{{ gapText(gap) }}</span>
          </li>
        </ul>
      </div>
    </div>

    <button
      v-if="open && receipt"
      type="button"
      class="receipt__refresh"
      :disabled="loading"
      @click="load()"
    >
      <Icon name="refresh" :size="12" aria-hidden="true" />
      <span>{{ t('chat.receipt.refresh') }}</span>
    </button>
  </section>
</template>

<script setup lang="ts">
import { computed, onMounted, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import Icon from '@/components/Icon.vue'
import FileChangeDiffViewer from '@/components/chat/FileChangeDiffViewer.vue'
import { useRpcStore } from '@/stores/rpc'
import {
  type ReceiptGap,
  type TaskReceipt,
  formatBytes,
  hasReceiptContent,
  receiptTone,
  receiptCounts,
  sortGaps,
} from './receiptViewModel'

const props = withDefaults(defineProps<{
  sessionKey: string
  /** 本回合起始（通常为用户消息时间戳） */
  since?: string | number | null
  /** 本回合结束（通常为这条助手消息时间戳） */
  until?: string | number | null
}>(), { since: null, until: null })

const { t } = useI18n()
const rpc = useRpcStore()

const loading = ref(false)
const loadError = ref('')
const receipt = ref<TaskReceipt | null>(null)
const open = ref(false)
/** 交付回执里点开的那一行文件；空串表示没有展开。 */
const openedChangePath = ref('')
/** 后端没有 `task.receipt`（旧二进制）：整张卡静默隐藏，不把「方法不存在」当成错误刷屏。 */
const unsupported = ref(false)

const tone = computed(() => receiptTone(receipt.value?.verification?.status))
const checks = computed(() => receipt.value?.verification?.checks || [])
const changes = computed(() => receipt.value?.changes || [])
const artifacts = computed(() => receipt.value?.artifacts || [])
const gaps = computed(() => sortGaps(receipt.value?.unattested))
const visible = computed(() =>
  !unsupported.value
  && (loading.value || Boolean(loadError.value) || hasReceiptContent(receipt.value)),
)

const verdictLabel = computed(() => {
  if (!receipt.value?.ok) return t('chat.receipt.verdict.unavailable')
  switch (receipt.value.verification?.status) {
    case 'passed':
      return t('chat.receipt.verdict.passed')
    case 'failed':
      return t('chat.receipt.verdict.failed')
    case 'unknown':
      return t('chat.receipt.verdict.unknown')
    default:
      return t('chat.receipt.verdict.unverified')
  }
})

const summaryText = computed(() => {
  if (!receipt.value?.ok) return ''
  const c = receiptCounts(receipt.value)
  const parts: string[] = []
  if (c.changes) parts.push(t('chat.receipt.metric.changes', { n: c.changes }))
  if (c.artifacts) parts.push(t('chat.receipt.metric.artifacts', { n: c.artifacts }))
  if (c.tools) parts.push(t('chat.receipt.metric.tools', { n: c.tools }))
  return parts.join(' · ')
})

const budgetText = computed(() => {
  const b = receipt.value?.budget
  if (!b) return t('chat.receipt.budgetUnavailable')
  const parts: string[] = []
  if (typeof b.tokensIn === 'number' || typeof b.tokensOut === 'number') {
    parts.push(t('chat.receipt.tokens', { in: b.tokensIn ?? 0, out: b.tokensOut ?? 0 }))
  }
  if (b.model) parts.push(b.model)
  if (b.policy) parts.push(b.policy)
  if (b.runsElapsedMs && b.runsElapsedMs > 0) parts.push(formatDuration(b.runsElapsedMs))
  if (!parts.length) return t('chat.receipt.budgetUnavailable')
  return parts.join(' · ')
})

const scopeTitle = computed(() => receipt.value?.scope?.note || '')

function formatDuration(ms: number): string {
  if (ms < 1000) return `${ms} ms`
  const secs = ms / 1000
  if (secs < 60) return `${Math.round(secs * 10) / 10} s`
  const mins = Math.floor(secs / 60)
  return `${mins} min ${Math.round(secs - mins * 60)} s`
}

function toggleChange(path: string) {
  openedChangePath.value = openedChangePath.value === path ? '' : path
}

function kindLabel(kind: string): string {
  const key = kind === 'create' || kind === 'modify' || kind === 'delete' ? kind : 'modify'
  return t(`chat.receipt.kind.${key}`)
}

/** 缺口文案：优先用 i18n 的 code 映射，未知 code 退回后端 detail。 */
function gapText(gap: ReceiptGap): string {
  const key = `chat.receipt.gap.${gap.code}`
  const translated = t(key, { n: gap.count ?? 0 })
  if (translated && translated !== key) return translated
  return gap.detail || gap.code
}

/**
 * 判定「这个方法在当前后端不存在」：老安装版没有 `task.receipt`，
 * 属于**版本差异**而非故障，静默隐藏卡片比每条回复下面挂一行红字更诚实。
 */
function isMethodMissing(code: string | undefined, message: string | undefined): boolean {
  const c = (code || '').toUpperCase()
  if (c === 'NOT_IMPLEMENTED' || c === 'METHOD_NOT_FOUND') return true
  return /not[_ ]?implemented|unknown method|method not found|方法不存在|未实现/i.test(message || '')
}

async function ensureConnected(): Promise<boolean> {
  if (rpc.isConnected) return true
  try {
    await rpc.waitForConnection(10_000)
    return rpc.isConnected
  } catch {
    return false
  }
}

async function load(): Promise<void> {
  if (!props.sessionKey) return
  if (!(await ensureConnected())) {
    loadError.value = t('chat.receipt.notConnected')
    return
  }
  loading.value = true
  loadError.value = ''
  try {
    const params: Record<string, unknown> = { sessionKey: props.sessionKey, limit: 50 }
    if (props.since !== null && props.since !== undefined && props.since !== '') params.since = props.since
    if (props.until !== null && props.until !== undefined && props.until !== '') params.until = props.until
    const res = await rpc.call<TaskReceipt>('task.receipt', params)
    if (!res || res.ok === false) {
      if (isMethodMissing(res?.code, res?.error)) {
        unsupported.value = true
        return
      }
      loadError.value = res?.error || t('chat.receipt.loadFailed')
      receipt.value = null
      return
    }
    receipt.value = res
    // 验证失败时默认展开：这是用户必须看到的信息，不能藏在折叠里。
    if (res.verification?.status === 'failed') open.value = true
  } catch (err) {
    const msg = err instanceof Error ? err.message : String(err)
    if (isMethodMissing(undefined, msg)) {
      unsupported.value = true
      return
    }
    loadError.value = msg
  } finally {
    loading.value = false
  }
}

watch(() => [props.sessionKey, props.since, props.until], () => {
  receipt.value = null
  open.value = false
  openedChangePath.value = ''
  void load()
})

onMounted(() => { void load() })
</script>

<style scoped>
.receipt {
  margin-top: 10px;
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  background: var(--bg-elevated, var(--bg-surface));
  font-size: var(--fs-xs);
  overflow: hidden;
}

.receipt--ok { border-color: color-mix(in srgb, var(--ok) 34%, var(--border)); }
.receipt--danger { border-color: color-mix(in srgb, var(--danger) 40%, var(--border)); }
.receipt--warn { border-color: color-mix(in srgb, var(--warn) 36%, var(--border)); }

.receipt__head {
  display: flex;
  width: 100%;
  align-items: center;
  gap: 8px;
  padding: 7px 10px;
  border: 0;
  background: transparent;
  color: var(--text);
  cursor: pointer;
  font: inherit;
  text-align: left;
}

.receipt__head:hover { background: var(--bg-hover); }

.receipt__title { font-weight: 600; color: var(--text); }

.receipt__verdict {
  padding: 1px 7px;
  border-radius: var(--radius-pill);
  font-size: 10.5px;
  background: var(--bg-hover);
  color: var(--text-muted);
}

.receipt--ok .receipt__verdict { background: color-mix(in srgb, var(--ok) 16%, transparent); color: var(--ok); }
.receipt--danger .receipt__verdict { background: color-mix(in srgb, var(--danger) 16%, transparent); color: var(--danger); }
.receipt--warn .receipt__verdict { background: color-mix(in srgb, var(--warn) 18%, transparent); color: var(--warn); }

.receipt__summary,
.receipt__hint { color: var(--text-dim); }

.receipt__spacer { flex: 1 1 auto; }

.receipt__note {
  margin: 0;
  padding: 0 10px 8px;
  color: var(--text-muted);
  line-height: 1.6;
}

.receipt__body {
  padding: 2px 10px 8px;
  border-top: 1px solid var(--border);
}

.receipt__section { margin-top: 8px; }

.receipt__section--budget .receipt__note { padding: 0; }

.receipt__label {
  margin: 0 0 4px;
  color: var(--text-dim);
  font-size: 10.5px;
  font-weight: 600;
  letter-spacing: 0.02em;
  text-transform: uppercase;
}

.receipt__list {
  margin: 0;
  padding: 0;
  list-style: none;
  display: flex;
  flex-direction: column;
  gap: 3px;
}

.receipt__row {
  display: flex;
  align-items: center;
  gap: 6px;
  min-width: 0;
}

.receipt__dot {
  width: 6px;
  height: 6px;
  flex: 0 0 auto;
  border-radius: 50%;
  background: var(--text-dim);
}

.receipt__dot.is-ok { background: var(--ok); }
.receipt__dot.is-fail { background: var(--danger); }
.receipt__dot.is-unknown { background: var(--warn); }
.receipt__dot.is-warn { background: var(--warn); }

.receipt__cmd,
.receipt__path {
  font-family: var(--font-mono, ui-monospace, SFMono-Regular, Menlo, Consolas, monospace);
  color: var(--text);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.receipt__meta { flex: 0 0 auto; color: var(--text-dim); }

.receipt__kind {
  flex: 0 0 auto;
  padding: 0 5px;
  border-radius: var(--radius-sm);
  font-size: 10px;
  background: var(--bg-hover);
  color: var(--text-muted);
}

.receipt__kind.is-create { background: color-mix(in srgb, var(--ok) 16%, transparent); color: var(--ok); }
.receipt__kind.is-delete { background: color-mix(in srgb, var(--danger) 14%, transparent); color: var(--danger); }
.receipt__kind.is-modify { background: color-mix(in srgb, var(--warn) 16%, transparent); color: var(--warn); }

.receipt__row--change {
  width: 100%;
  margin-inline: -4px;
  padding: 2px 4px;
  border: 0;
  border-radius: var(--radius-sm);
  background: transparent;
  font: inherit;
  text-align: left;
  cursor: pointer;
}

.receipt__row--change:hover,
.receipt__row--change.is-open {
  background: var(--bg-hover);
}

.receipt__row--change:focus-visible {
  outline: none;
  box-shadow: var(--focus-ring);
}

.receipt__row--change > svg {
  flex: 0 0 auto;
  color: var(--text-dim);
}

.receipt__row--gap { align-items: flex-start; }
.receipt__gap { color: var(--text-muted); line-height: 1.55; }

.receipt__refresh {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  margin: 0 10px 8px;
  padding: 2px 8px;
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  background: transparent;
  color: var(--text-muted);
  cursor: pointer;
  font: inherit;
  font-size: 10.5px;
}

.receipt__refresh:hover:not(:disabled) { background: var(--bg-hover); color: var(--text); }
.receipt__refresh:disabled { opacity: 0.5; cursor: not-allowed; }
</style>
