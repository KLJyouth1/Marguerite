<script setup lang="ts">
import { onMounted, ref, shallowRef } from 'vue'
import { useI18n } from 'vue-i18n'
import Icon from '@/components/Icon.vue'
import LoadingSpinner from '@/components/LoadingSpinner.vue'
import { useRpcStore } from '@/stores/rpc'

// 后端契约（由另一路接线，名字与形状固定，前端只读真实返回）：
//   assistant.scope.get → { scope, workspace, since }
//   assistant.scope.set ← { scope, workspace? }
//   assistant.prefs.list → { pinned: [{ text, source, at }], preferences: [{ key, value, confidence, evidence }] }
//   assistant.prefs.pin / assistant.prefs.unpin ← { text }
//
// 重要边界：自动偏好来自 user_profile.md，后端**没有**删除偏好的 RPC。
// 所以本面板对偏好只做「展示 + 钉住」，绝不伪造删除按钮（否则用户会以为已经删掉，
// 而文件里那一条其实还在，下次简报照样注入）。
const SCOPE_GET = 'assistant.scope.get'
const SCOPE_SET = 'assistant.scope.set'
const PREFS_LIST = 'assistant.prefs.list'
const PREFS_PIN = 'assistant.prefs.pin'
const PREFS_UNPIN = 'assistant.prefs.unpin'

type Scope = 'global' | 'project'

interface PinnedItem {
  text: string
  source: string
  at: string
}

interface PreferenceItem {
  key: string
  value: string
  /// 已换算成百分数；null = 网关没给置信度（显示「未知」而不是伪造 0%）。
  confidence: number | null
  evidence: number
}

const { t } = useI18n()
const rpc = useRpcStore()

const state = ref<'loading' | 'ready' | 'unsupported' | 'error'>('loading')
const scope = ref<Scope>('global')
const workspace = ref<string | null>(null)
/// 项目作用域必须带 workspace 路径（后端硬约束 ASSISTANT_SCOPE_NO_WORKSPACE）。
/// 用户可能在 global 状态下进入本面板，此时网关不回报任何候选路径 → 由用户填。
const workspaceDraft = ref('')
const pinned = shallowRef<PinnedItem[]>([])
const preferences = shallowRef<PreferenceItem[]>([])
const errorDetail = ref('')
const actionError = ref('')
/// 正在进行的动作（`pin:<text>` / `unpin:<text>`），用于禁用重复点击。
const busy = ref('')
const scopeBusy = ref(false)

function objectValue(value: unknown): Record<string, unknown> {
  return value && typeof value === 'object' ? value as Record<string, unknown> : {}
}

function textValue(value: unknown): string {
  if (typeof value === 'string') return value
  if (typeof value === 'number' && Number.isFinite(value)) return String(value)
  return ''
}

function optionalNumber(value: unknown): number | null {
  return typeof value === 'number' && Number.isFinite(value) ? value : null
}

function rpcErrorCode(error: unknown): string {
  return textValue((error as { code?: string } | undefined)?.code)
}

interface RpcFailure {
  code: string
  message: string
}

/**
 * 后端的写型方法（scope.set / pin / unpin）用 `{success:false, code, message}`
 * 表达可预期的拒绝，而不是抛 RPC 错误。不判这一层就会把「被拒」当成「成功」，
 * 让界面显示成已切换/已钉住 —— 正是本面板要避免的假完成。
 */
function rpcFailure(value: unknown): RpcFailure | null {
  const data = objectValue(value)
  if (data.success !== false) return null
  return { code: textValue(data.code), message: textValue(data.message) }
}

/// 保留机器可判定的 code（或后端原文），让失败可复述、可追查。
function failureText(failure: RpcFailure, fallbackKey: string): string {
  const detail = failure.code || failure.message
  const label = t(fallbackKey)
  return detail ? `${label} (${detail})` : label
}

function normalizePinned(value: unknown): PinnedItem | null {
  const data = objectValue(value)
  const text = textValue(data.text).trim()
  if (!text) return null
  return { text, source: textValue(data.source), at: textValue(data.at) }
}

/**
 * 置信度在契约里是 0..1 的比例；若网关直接给百分数（>1）就按百分数展示，
 * 避免把 0.82 之外的 82 渲染成 8200%。
 */
function confidencePercent(value: unknown): number | null {
  const raw = optionalNumber(value)
  if (raw === null) return null
  const percent = raw > 1 ? raw : raw * 100
  return Math.max(0, Math.min(100, Math.round(percent)))
}

/// evidence 可能是证据数组（brief 的形状）或条数；两种都算成条数。
function evidenceCount(value: unknown): number {
  if (Array.isArray(value)) return value.length
  const raw = optionalNumber(value)
  return raw === null || raw <= 0 ? 0 : Math.round(raw)
}

function normalizePreference(value: unknown): PreferenceItem | null {
  const data = objectValue(value)
  const key = textValue(data.key).trim()
  const text = textValue(data.value).trim()
  if (!key && !text) return null
  return {
    key: key || text,
    value: key ? text : '',
    confidence: confidencePercent(data.confidence),
    evidence: evidenceCount(data.evidence),
  }
}

function preferenceText(item: PreferenceItem): string {
  return item.value ? `${item.key}: ${item.value}` : item.key
}

function isBusy(kind: string, text: string): boolean {
  return busy.value === `${kind}:${text}`
}

/// 时间戳可能是秒、毫秒或 ISO 字符串；都归一化成可读本地时间，失败则留空。
function formatAt(raw: string): string {
  if (!raw) return ''
  let stamp: number
  if (/^\d+$/.test(raw)) {
    const numeric = Number(raw)
    stamp = numeric < 1e11 ? numeric * 1000 : numeric
  } else {
    stamp = Date.parse(raw)
  }
  if (!Number.isFinite(stamp)) return ''
  const date = new Date(stamp)
  if (Number.isNaN(date.getTime())) return ''
  return new Intl.DateTimeFormat(undefined, {
    year: 'numeric',
    month: '2-digit',
    day: '2-digit',
    hour: '2-digit',
    minute: '2-digit',
  }).format(date)
}

async function loadPrefs() {
  const data = objectValue(await rpc.call(PREFS_LIST))
  pinned.value = (Array.isArray(data.pinned) ? data.pinned : [])
    .map(normalizePinned)
    .filter((item): item is PinnedItem => item !== null)
  preferences.value = (Array.isArray(data.preferences) ? data.preferences : [])
    .map(normalizePreference)
    .filter((item): item is PreferenceItem => item !== null)
}

async function load() {
  state.value = 'loading'
  errorDetail.value = ''
  actionError.value = ''
  try {
    await rpc.waitForConnection(8000)
    // 字面量门控：这两个方法必须在网关 SUPPORTED_METHODS 里声明，否则面板会
    // 永远打不开（check-rpc-contract 会揪出这类漂移）。
    if (!rpc.supportsMethod('assistant.scope.get') || !rpc.supportsMethod('assistant.prefs.list')) {
      state.value = 'unsupported'
      return
    }
    const scopeData = objectValue(await rpc.call(SCOPE_GET))
    scope.value = textValue(scopeData.scope) === 'project' ? 'project' : 'global'
    workspace.value = textValue(scopeData.workspace) || null
    workspaceDraft.value = workspace.value ?? ''
    await loadPrefs()
    state.value = 'ready'
  } catch (error) {
    errorDetail.value = rpcErrorCode(error)
    state.value = 'error'
  }
}

async function setScope(next: Scope) {
  if (next === scope.value || scopeBusy.value) return
  const draft = workspaceDraft.value.trim()
  // 没有 workspace 路径就不切到 project：不凭空编造路径发给后端。
  if (next === 'project' && !draft) return
  scopeBusy.value = true
  actionError.value = ''
  try {
    const params: Record<string, unknown> = next === 'project'
      ? { scope: 'project', workspace: draft }
      : { scope: 'global' }
    const result = await rpc.call(SCOPE_SET, params)
    const failure = rpcFailure(result)
    if (failure) {
      actionError.value = failureText(failure, 'settings.assistant.scopeSaveFailed')
      return
    }
    // 以后端回包为准（scope.set 与 scope.get 同形状）。
    const data = objectValue(result)
    scope.value = textValue(data.scope) === 'project' ? 'project' : next
    workspace.value = textValue(data.workspace) || null
    workspaceDraft.value = workspace.value ?? draft
  } catch {
    actionError.value = t('settings.assistant.scopeSaveFailed')
  } finally {
    scopeBusy.value = false
  }
}

async function pin(item: PreferenceItem) {
  const text = preferenceText(item)
  if (!text || isBusy('pin', text)) return
  busy.value = `pin:${text}`
  actionError.value = ''
  try {
    const result = await rpc.call(PREFS_PIN, { text })
    const failure = rpcFailure(result)
    if (failure) {
      actionError.value = failureText(failure, 'settings.assistant.pinFailed')
      return
    }
    // 重新读列表而不是本地拼装：钉住清单以后端为准（去重规则由后端决定）。
    await loadPrefs()
  } catch {
    actionError.value = t('settings.assistant.pinFailed')
  } finally {
    busy.value = ''
  }
}

async function unpin(item: PinnedItem) {
  if (!item.text || isBusy('unpin', item.text)) return
  busy.value = `unpin:${item.text}`
  actionError.value = ''
  try {
    const result = await rpc.call(PREFS_UNPIN, { text: item.text })
    const failure = rpcFailure(result)
    if (failure) {
      actionError.value = failureText(failure, 'settings.assistant.unpinFailed')
      return
    }
    await loadPrefs()
  } catch {
    actionError.value = t('settings.assistant.unpinFailed')
  } finally {
    busy.value = ''
  }
}

onMounted(() => {
  void load()
})
</script>

<template>
  <section class="assistant" data-testid="settings-assistant">
    <header class="assistant__head">
      <h3 class="assistant__title">{{ t('settings.assistant.title') }}</h3>
      <p class="assistant__description">{{ t('settings.assistant.description') }}</p>
    </header>

    <div
      v-if="state === 'unsupported'"
      class="assistant__notice"
      role="status"
      data-testid="assistant-unavailable"
    >
      <Icon name="info" :size="18" aria-hidden="true" />
      <div>
        <h4>{{ t('settings.assistant.unavailableTitle') }}</h4>
        <p>{{ t('settings.assistant.unavailableDescription') }}</p>
      </div>
    </div>

    <div v-else-if="state === 'loading'" class="assistant__loading" role="status">
      <LoadingSpinner />
      <span>{{ t('shared.loading') }}</span>
    </div>

    <template v-else>
      <div v-if="state === 'error'" class="assistant__notice assistant__notice--error" role="alert">
        <Icon name="info" :size="18" aria-hidden="true" />
        <div>
          <h4>{{ t('settings.assistant.loadFailed') }}</h4>
          <p v-if="errorDetail">{{ errorDetail }}</p>
          <button
            type="button"
            class="btn btn--ghost assistant__refresh"
            data-testid="assistant-retry"
            @click="load"
          >{{ t('settings.assistant.refresh') }}</button>
        </div>
      </div>

      <div v-if="actionError" class="assistant__notice assistant__notice--error" role="alert">
        <Icon name="info" :size="18" aria-hidden="true" />
        <div>
          <p>{{ actionError }}</p>
        </div>
      </div>

      <div class="assistant__block" data-testid="assistant-scope">
        <div class="assistant__block-head">
          <h4 class="assistant__block-title">{{ t('settings.assistant.scopeTitle') }}</h4>
          <button
            type="button"
            class="btn btn--ghost assistant__icon-btn"
            :aria-label="t('settings.assistant.refresh')"
            :title="t('settings.assistant.refresh')"
            :disabled="scopeBusy"
            data-testid="assistant-refresh"
            @click="load"
          >
            <Icon name="refresh" :size="14" aria-hidden="true" />
          </button>
        </div>
        <div class="assistant__segmented" role="group" :aria-label="t('settings.assistant.scopeTitle')">
          <button
            type="button"
            class="assistant__scope"
            :class="{ 'is-active': scope === 'global' }"
            :aria-pressed="scope === 'global' ? 'true' : 'false'"
            :disabled="scopeBusy"
            data-testid="assistant-scope-global"
            @click="setScope('global')"
          >{{ t('settings.assistant.scopeGlobal') }}</button>
          <button
            type="button"
            class="assistant__scope"
            :class="{ 'is-active': scope === 'project' }"
            :aria-pressed="scope === 'project' ? 'true' : 'false'"
            :disabled="scopeBusy || !workspaceDraft.trim()"
            :title="workspaceDraft.trim() ? undefined : t('settings.assistant.scopeNeedWorkspace')"
            data-testid="assistant-scope-project"
            @click="setScope('project')"
          >{{ t('settings.assistant.scopeProject') }}</button>
        </div>
        <label class="assistant__field">
          <span class="assistant__field-label">{{ t('settings.assistant.scopeWorkspaceLabel') }}</span>
          <input
            v-model="workspaceDraft"
            type="text"
            class="assistant__input"
            :placeholder="t('settings.assistant.scopeWorkspacePlaceholder')"
            :aria-label="t('settings.assistant.scopeWorkspaceLabel')"
            data-testid="assistant-workspace-input"
          />
        </label>
        <p v-if="scope === 'project' && workspace" class="assistant__meta" data-testid="assistant-workspace">
          {{ t('settings.assistant.scopeCurrent', { workspace }) }}
        </p>
        <p v-else-if="!workspaceDraft.trim()" class="assistant__meta">
          {{ t('settings.assistant.scopeNeedWorkspace') }}
        </p>
      </div>

      <div class="assistant__block" data-testid="assistant-pinned">
        <h4 class="assistant__block-title">{{ t('settings.assistant.pinnedTitle') }}</h4>
        <p class="assistant__meta">{{ t('settings.assistant.pinnedDescription') }}</p>
        <ul v-if="pinned.length" class="assistant__list">
          <li
            v-for="(item, index) in pinned"
            :key="item.text + '#' + index"
            class="assistant__row"
            data-testid="assistant-pinned-item"
          >
            <div class="assistant__row-copy">
              <strong class="assistant__row-title">{{ item.text }}</strong>
              <span class="assistant__row-meta">
                <span v-if="item.source">{{ t('settings.assistant.pinnedSource', { source: item.source }) }}</span>
                <span v-if="item.at">{{ formatAt(item.at) }}</span>
              </span>
            </div>
            <button
              type="button"
              class="btn btn--ghost assistant__action"
              :aria-label="t('settings.assistant.unpin') + ': ' + item.text"
              :disabled="isBusy('unpin', item.text)"
              data-testid="assistant-unpin"
              @click="unpin(item)"
            >
              <Icon name="trash" :size="14" aria-hidden="true" />
              {{ t('settings.assistant.unpin') }}
            </button>
          </li>
        </ul>
        <p v-else class="assistant__empty" data-testid="assistant-pinned-empty">
          {{ t('settings.assistant.pinnedEmpty') }}
        </p>
      </div>

      <div class="assistant__block" data-testid="assistant-preferences">
        <h4 class="assistant__block-title">{{ t('settings.assistant.preferencesTitle') }}</h4>
        <p class="assistant__meta">{{ t('settings.assistant.preferencesDescription') }}</p>
        <ul v-if="preferences.length" class="assistant__list">
          <li
            v-for="(item, index) in preferences"
            :key="item.key + '#' + index"
            class="assistant__row"
            data-testid="assistant-preference"
          >
            <div class="assistant__row-copy">
              <strong class="assistant__row-title">
                <span class="assistant__key">{{ item.key }}</span>
                <span v-if="item.value"> = {{ item.value }}</span>
              </strong>
              <span class="assistant__row-meta">
                <span v-if="item.confidence !== null">
                  {{ t('settings.assistant.confidence', { percent: item.confidence }) }}
                </span>
                <span v-else>{{ t('settings.assistant.confidenceUnknown') }}</span>
                <span>{{ t('settings.assistant.evidence', { count: item.evidence }) }}</span>
              </span>
            </div>
            <button
              type="button"
              class="btn btn--ghost assistant__action"
              :aria-label="t('settings.assistant.pin') + ': ' + preferenceText(item)"
              :disabled="isBusy('pin', preferenceText(item))"
              data-testid="assistant-pin"
              @click="pin(item)"
            >
              <Icon name="plus" :size="14" aria-hidden="true" />
              {{ t('settings.assistant.pin') }}
            </button>
          </li>
        </ul>
        <p v-else class="assistant__empty" data-testid="assistant-preferences-empty">
          {{ t('settings.assistant.preferencesEmpty') }}
        </p>
      </div>
    </template>
  </section>
</template>

<style scoped>
.assistant {
  display: flex;
  flex-direction: column;
  gap: var(--sp-4);
  max-width: 880px;
}

.assistant__head {
  display: flex;
  flex-direction: column;
  gap: var(--sp-1);
}

.assistant__title,
.assistant__block-title,
.assistant__notice h4 {
  color: var(--text);
  margin: 0;
}

.assistant__title {
  font-size: var(--fs-lg);
}

.assistant__block-title {
  font-size: var(--fs-md);
}

.assistant__description,
.assistant__meta,
.assistant__empty,
.assistant__row-meta,
.assistant__notice p {
  color: var(--text-muted);
  font-size: var(--fs-sm);
  line-height: 1.5;
  margin: 0;
}

.assistant__loading {
  align-items: center;
  color: var(--text-muted);
  display: flex;
  font-size: var(--fs-sm);
  gap: var(--sp-2);
  min-height: 48px;
}

.assistant__notice {
  align-items: flex-start;
  background: var(--bg-elevated);
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  display: flex;
  gap: var(--sp-3);
  padding: var(--sp-4);
}

.assistant__notice > div {
  display: flex;
  flex-direction: column;
  flex: 1;
  gap: var(--sp-1);
}

.assistant__notice--error {
  background: color-mix(in srgb, var(--danger) 8%, var(--bg-surface));
  border-color: color-mix(in srgb, var(--danger) 32%, var(--border));
}

.assistant__refresh {
  align-self: flex-start;
}

.assistant__block {
  border-top: 1px solid var(--hairline);
  display: flex;
  flex-direction: column;
  gap: var(--sp-2);
  padding-top: var(--sp-3);
}

.assistant__block-head {
  align-items: center;
  display: flex;
  gap: var(--sp-2);
  justify-content: space-between;
}

.assistant__icon-btn {
  min-height: 28px;
  padding: 2px var(--sp-2);
}

.assistant__segmented {
  display: flex;
  gap: var(--sp-2);
}

.assistant__scope {
  background: var(--bg-elevated);
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  color: var(--text-muted);
  cursor: pointer;
  font: inherit;
  font-size: var(--fs-sm);
  min-height: 34px;
  padding: 0 var(--sp-4);
}

.assistant__scope:hover:not(:disabled) {
  background: var(--bg-hover);
  color: var(--text);
}

.assistant__scope.is-active {
  background: color-mix(in srgb, var(--accent) 10%, var(--bg-elevated));
  border-color: var(--accent);
  color: var(--text);
  font-weight: 600;
}

.assistant__scope:focus-visible {
  box-shadow: var(--focus-ring);
  outline: none;
}

.assistant__scope:disabled {
  cursor: not-allowed;
  opacity: 0.55;
}

.assistant__field {
  display: flex;
  flex-direction: column;
  gap: var(--sp-1);
}

.assistant__field-label {
  color: var(--text-dim);
  font-size: var(--fs-xs);
}

.assistant__input {
  background: var(--bg);
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  color: var(--text);
  font: inherit;
  font-family: var(--font-mono);
  font-size: var(--fs-sm);
  min-height: 34px;
  padding: 0 var(--sp-3);
}

.assistant__input:focus-visible {
  border-color: var(--accent);
  box-shadow: var(--focus-ring);
  outline: none;
}

.assistant__list {
  display: flex;
  flex-direction: column;
  gap: var(--sp-2);
  list-style: none;
  margin: 0;
  padding: 0;
}

.assistant__row {
  align-items: center;
  background: var(--bg-elevated);
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  display: flex;
  gap: var(--sp-3);
  justify-content: space-between;
  padding: var(--sp-2) var(--sp-3);
}

.assistant__row-copy {
  display: flex;
  flex-direction: column;
  gap: 2px;
  min-width: 0;
}

.assistant__row-title {
  color: var(--text);
  font-size: var(--fs-sm);
  overflow-wrap: anywhere;
}

.assistant__row-meta {
  display: flex;
  flex-wrap: wrap;
  font-family: var(--font-mono);
  font-size: var(--fs-xs);
  gap: var(--sp-3);
}

.assistant__key {
  color: var(--text-muted);
}

.assistant__action {
  flex-shrink: 0;
  font-size: var(--fs-xs);
  min-height: 30px;
  padding: 4px var(--sp-2);
}

.assistant__row .assistant__action:focus-visible {
  box-shadow: var(--focus-ring);
  outline: none;
}

@media (max-width: 640px) {
  .assistant__row {
    align-items: stretch;
    flex-direction: column;
  }

  .assistant__action {
    align-self: flex-start;
  }
}
</style>
