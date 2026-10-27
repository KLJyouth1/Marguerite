<script setup lang="ts">
/**
 * M3-D2 FinOps：会话预算阈值面板。
 * usage.budget.get / usage.budget.set 读写会话预算；超阈值后 core 触发
 * budget.warning 升级事件（审计日志可见）并在此高亮。
 */
import { onMounted, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { useRpcStore } from '@/stores/rpc'

const { t } = useI18n()

interface BudgetState {
  ok: boolean
  session: string
  threshold_tokens: number | null
  threshold_usd: number | null
  total_tokens: number
  input_tokens: number
  output_tokens: number
  cost_usd: number
  currency: string
  exceeded: boolean
  error?: string
}

const rpc = useRpcStore()
const session = ref('default')
const state = ref<BudgetState | null>(null)
const loading = ref(false)
const saving = ref(false)
const error = ref<string | null>(null)
const message = ref<string | null>(null)
const formTokens = ref<number | null>(null)
const formUsd = ref<number | null>(null)

async function refresh() {
  loading.value = true
  error.value = null
  try {
    await rpc.waitForConnection()
    const data = await rpc.call<BudgetState>('usage.budget.get', { session: session.value })
    if (data?.ok === false) {
      error.value = data.error ?? t('settings.budget.readFailed')
    } else {
      state.value = data
      formTokens.value = data.threshold_tokens
      formUsd.value = data.threshold_usd
    }
  } catch (e) {
    error.value = e instanceof Error ? e.message : String(e)
  } finally {
    loading.value = false
  }
}

async function save() {
  if (saving.value) return
  saving.value = true
  error.value = null
  message.value = null
  try {
    const params: Record<string, unknown> = { session: session.value }
    if (formTokens.value != null) params.thresholdTokens = formTokens.value
    if (formUsd.value != null) params.thresholdUsd = formUsd.value
    const data = await rpc.call<{ ok: boolean; error?: string }>('usage.budget.set', params)
    if (data.ok) {
      message.value = t('settings.budget.saved')
    } else {
      error.value = data.error ?? t('settings.budget.saveFailed')
    }
    await refresh()
  } catch (e) {
    error.value = e instanceof Error ? e.message : String(e)
  } finally {
    saving.value = false
  }
}

async function clearBudget() {
  saving.value = true
  error.value = null
  message.value = null
  try {
    const data = await rpc.call<{ ok: boolean; error?: string }>('usage.budget.set', {
      session: session.value,
      thresholdTokens: 0,
      thresholdUsd: 0,
    })
    if (data.ok) {
      message.value = t('settings.budget.cleared')
    } else {
      error.value = data.error ?? t('settings.budget.clearFailed')
    }
    await refresh()
  } catch (e) {
    error.value = e instanceof Error ? e.message : String(e)
  } finally {
    saving.value = false
  }
}

onMounted(refresh)
</script>

<template>
  <section class="control-section">
    <div class="control-section__head">
      <h3 class="control-section__title">{{ t('settings.budget.title') }}</h3>
      <p class="control-section__desc">
        {{ t('settings.budget.description') }}
      </p>
    </div>
    <div v-if="error" class="budget-banner budget-banner--err" role="alert">{{ error }}</div>
    <div v-else-if="message" class="budget-banner budget-banner--ok" role="status">{{ message }}</div>
    <div v-if="state" class="budget-summary" :data-testid="state.exceeded ? 'budget-exceeded' : 'budget-ok'">
      <div class="budget-summary__item">
        <span class="budget-summary__label">{{ t('settings.budget.session') }}</span>
        <span class="budget-summary__value">{{ state.session }}</span>
      </div>
      <div class="budget-summary__item">
        <span class="budget-summary__label">{{ t('settings.budget.usedTokens') }}</span>
        <span class="budget-summary__value">{{ state.total_tokens.toLocaleString() }}</span>
      </div>
      <div class="budget-summary__item">
        <span class="budget-summary__label">{{ t('settings.budget.usedCost') }}</span>
        <span class="budget-summary__value">${{ state.cost_usd.toFixed(4) }} {{ state.currency }}</span>
      </div>
      <div class="budget-summary__item">
        <span class="budget-summary__label">{{ t('settings.budget.threshold') }}</span>
        <span class="budget-summary__value">
          {{ state.threshold_tokens != null ? `${state.threshold_tokens.toLocaleString()} tok` : '—' }}
          / {{ state.threshold_usd != null ? `$${state.threshold_usd}` : '—' }}
        </span>
      </div>
      <div class="budget-summary__item">
        <span class="budget-summary__label">{{ t('settings.budget.status') }}</span>
        <span class="budget-status" :class="state.exceeded ? 'budget-status--exceeded' : 'budget-status--ok'">
          {{ state.exceeded ? t('settings.budget.exceeded') : t('settings.budget.normal') }}
        </span>
      </div>
    </div>
    <div v-else-if="loading" class="budget-empty">{{ t('settings.budget.loading') }}</div>
    <div class="budget-form">
      <input
        v-model.number="formTokens"
        class="control-input control-input--narrow"
        type="number"
        min="0"
        :placeholder="t('settings.budget.tokensPlaceholder')"
        :aria-label="t('settings.budget.tokensAria')"
      />
      <input
        v-model.number="formUsd"
        class="control-input control-input--narrow"
        type="number"
        min="0"
        step="0.001"
        :placeholder="t('settings.budget.usdPlaceholder')"
        :aria-label="t('settings.budget.usdAria')"
      />
      <button type="button" class="btn" :disabled="saving || loading" @click="save">{{ t('settings.budget.save') }}</button>
      <button type="button" class="btn btn--ghost" :disabled="saving || loading" @click="clearBudget">{{ t('settings.budget.clear') }}</button>
    </div>
  </section>
</template>

<style scoped>
.budget-banner {
  border-radius: var(--radius-sm);
  font-size: var(--fs-sm);
  margin: var(--sp-2) 0;
  padding: var(--sp-2) var(--sp-3);
}
.budget-banner--err { background: color-mix(in srgb, var(--danger) 15%, var(--bg-surface)); color: var(--danger); }
.budget-banner--ok { background: color-mix(in srgb, var(--ok) 15%, var(--bg-surface)); color: var(--ok); }
.budget-summary {
  background: var(--bg-surface);
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  display: grid;
  gap: var(--sp-2);
  grid-template-columns: repeat(auto-fit, minmax(9rem, 1fr));
  margin: var(--sp-3) 0;
  padding: var(--sp-3);
}
.budget-summary__item { display: flex; flex-direction: column; gap: 2px; }
.budget-summary__label { color: var(--text-dim); font-size: var(--fs-xs); }
.budget-summary__value { font-size: var(--fs-sm); font-weight: 600; }
.budget-status {
  border-radius: var(--radius-sm);
  display: inline-block;
  font-size: var(--fs-xs);
  font-weight: 600;
  padding: 2px var(--sp-2);
  width: fit-content;
}
.budget-status--ok { background: color-mix(in srgb, var(--ok) 18%, var(--bg-surface)); color: var(--ok); }
.budget-status--exceeded { background: color-mix(in srgb, var(--danger) 18%, var(--bg-surface)); color: var(--danger); }
.budget-empty { color: var(--text-muted); font-size: var(--fs-sm); padding: var(--sp-3) 0; }
.budget-form {
  display: flex;
  flex-wrap: wrap;
  gap: var(--sp-2);
}
</style>
