<script setup lang="ts">
/**
 * 三档模型分层 + 成本台账（四端展示之 WebUI）。
 *
 * 数据源：`model.tier.get`（档位决策 + 预算 + 各档候选链）与 `model.ledger`
 * （会话预算四要素）。两者与 CLI `marguerite model tier`、TUI 状态栏同源
 * （都读 SessionState），避免三端各算一套、数字对不上。
 *
 * 为什么把「锁定档位」也放进来：只展示不操作，用户看到「均衡档」却无法解释
 * 为什么没用旗舰档；这里把 tier_reasons 一并显示，并提供显式锁定入口。
 */
import { computed, onMounted, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { useRpcStore } from '@/stores/rpc'

type TierName = 'flagship' | 'balanced' | 'turbo'

interface TierBudget {
  tokens_cap: number
  usd_cap: number
  tokens_used: number
  usd_used: number
  remaining_ratio: number
  state: string
  /** 计量口径来源：用户配过预算时是台账，否则是会话态 */
  budget_source?: 'usage_ledger' | 'session_state'
  budget_session?: string
}

interface TierData {
  locked_tier: string | null
  thinking_effort: string
  effective_tier: string
  tier_reasons: string[]
  budget: TierBudget
  tiers: Record<TierName, string[]>
}

const rpc = useRpcStore()
const { t } = useI18n()
const data = ref<TierData | null>(null)
const loading = ref(false)
const saving = ref(false)
const error = ref<string | null>(null)

const TIERS = computed<Array<{ id: TierName; label: string; hint: string }>>(() => [
  {
    id: 'flagship',
    label: t('settings.tierLedger.tierFlagship'),
    hint: t('settings.tierLedger.hintFlagship'),
  },
  {
    id: 'balanced',
    label: t('settings.tierLedger.tierBalanced'),
    hint: t('settings.tierLedger.hintBalanced'),
  },
  {
    id: 'turbo',
    label: t('settings.tierLedger.tierTurbo'),
    hint: t('settings.tierLedger.hintTurbo'),
  },
])

const stateLabel = computed<Record<string, string>>(() => ({
  healthy: t('settings.tierLedger.stateHealthy'),
  degraded: t('settings.tierLedger.stateDegraded'),
  emergency: t('settings.tierLedger.stateEmergency'),
  // 如实描述**实际**行为：耗尽时生效档已降到极速档，而不是「拒绝新请求」
  //（硬拒绝会在任务中途打断工作，故刻意不做；极速档就是成本天花板）。
  depleted: t('settings.tierLedger.stateDepleted'),
}))

/** 后端返回的是英文档名，展示层需要本地化，未知档名原样透出。 */
function tierLabel(id: string): string {
  const map: Record<string, string> = {
    flagship: t('settings.tierLedger.tierFlagship'),
    balanced: t('settings.tierLedger.tierBalanced'),
    turbo: t('settings.tierLedger.tierTurbo'),
  }
  return map[id] ?? id
}

/** 计量口径来源：让「改了预算却没看到变化」这类困惑有一行可核对的解释。 */
const budgetSourceLabel = computed(() => {
  const src = data.value?.budget.budget_source
  if (src === 'usage_ledger') {
    const sess = data.value?.budget.budget_session
    return sess ? t('settings.tierLedger.sourceLedger', { session: sess }) : t('settings.tierLedger.sourceLedgerNoSession')
  }
  if (src === 'session_state') return t('settings.tierLedger.sourceSession')
  return ''
})

function pct(v: number | undefined): string {
  return `${Math.round((v ?? 0) * 100)}%`
}

function money(v: number | undefined): string {
  return `$${(v ?? 0).toFixed(2)}`
}

async function load() {
  loading.value = true
  error.value = null
  try {
    await rpc.waitForConnection()
    const d = await rpc.call<TierData>('model.tier.get')
    data.value = d
  } catch (e) {
    error.value = e instanceof Error ? e.message : String(e)
  } finally {
    loading.value = false
  }
}

async function lock(tier: TierName | 'auto') {
  saving.value = true
  error.value = null
  try {
    await rpc.waitForConnection()
    await rpc.call('model.tier.set', { tier })
    await load()
  } catch (e) {
    error.value = e instanceof Error ? e.message : String(e)
  } finally {
    saving.value = false
  }
}

onMounted(load)
</script>

<template>
  <section class="control-section tierledger" aria-labelledby="tierledger-title">
    <div class="control-section__head">
      <h3 id="tierledger-title" class="control-section__title">{{ t('settings.tierLedger.title') }}</h3>
      <p class="control-section__desc">
        {{ t('settings.tierLedger.description') }}
      </p>
    </div>

    <div v-if="error" class="tierledger__banner" role="alert">{{ error }}</div>
    <div v-if="loading && !data" class="tierledger__empty">{{ t('settings.tierLedger.loading') }}</div>

    <template v-if="data">
      <div class="tierledger__status">
        <div class="tierledger__tier" :data-tier="data.effective_tier">
          <span class="tierledger__tier-label">{{ t('settings.tierLedger.effectiveTier') }}</span>
          <strong class="tierledger__tier-value">{{ tierLabel(data.effective_tier) }}</strong>
          <span class="tierledger__lock">
            {{ data.locked_tier ? t('settings.tierLedger.locked', { tier: tierLabel(data.locked_tier) }) : t('settings.tierLedger.autoSelect') }}
            · {{ t('settings.tierLedger.thinking', { effort: data.thinking_effort }) }}
          </span>
        </div>
        <div class="tierledger__budget-head">
          <span>{{ t('settings.tierLedger.budgetState', { state: stateLabel[data.budget.state] ?? data.budget.state }) }}</span>
          <span class="tierledger__remaining">{{ t('settings.tierLedger.remaining', { pct: pct(data.budget.remaining_ratio) }) }}</span>
        </div>
      </div>

      <div
        class="tierledger__bar"
        role="progressbar"
        :aria-label="t('settings.tierLedger.remainingAria')"
        aria-valuemin="0"
        aria-valuemax="100"
        :aria-valuenow="Math.round((data.budget.remaining_ratio ?? 0) * 100)"
        :data-state="data.budget.state"
      >
        <span
          class="tierledger__bar-fill"
          :style="{ width: `${Math.max(0, Math.min(100, (data.budget.remaining_ratio ?? 0) * 100))}%` }"
        />
      </div>

      <dl class="tierledger__nums">
        <div><dt>{{ t('settings.tierLedger.tokenLabel') }}</dt><dd>{{ (data.budget.tokens_used ?? 0).toLocaleString() }} / {{ (data.budget.tokens_cap ?? 0).toLocaleString() }}</dd></div>
        <div><dt>{{ t('settings.tierLedger.costLabel') }}</dt><dd>{{ money(data.budget.usd_used) }} / {{ money(data.budget.usd_cap) }}</dd></div>
        <div><dt>{{ t('settings.tierLedger.remainingLabel') }}</dt><dd>{{ pct(data.budget.remaining_ratio) }}</dd></div>
        <div v-if="budgetSourceLabel"><dt>{{ t('settings.tierLedger.sourceLabel') }}</dt><dd>{{ budgetSourceLabel }}</dd></div>
      </dl>

      <div v-if="data.tier_reasons?.length" class="tierledger__reasons">
        <span class="tierledger__reasons-title">{{ t('settings.tierLedger.whyThisTier') }}</span>
        <ul>
          <li v-for="(reason, i) in data.tier_reasons" :key="i">{{ reason }}</li>
        </ul>
      </div>

      <div class="tierledger__lock-actions" role="group" :aria-label="t('settings.tierLedger.lockGroupAria')">
        <button
          type="button"
          class="btn btn--sm"
          :class="{ 'is-active': !data.locked_tier }"
          :aria-pressed="!data.locked_tier"
          :disabled="saving"
          @click="lock('auto')"
        >{{ t('settings.tierLedger.autoButton') }}</button>
        <button
          v-for="tier in TIERS"
          :key="tier.id"
          type="button"
          class="btn btn--sm"
          :class="{ 'is-active': data.locked_tier === tier.id }"
          :aria-pressed="data.locked_tier === tier.id"
          :title="tier.hint"
          :disabled="saving"
          @click="lock(tier.id)"
        >{{ tier.label }}</button>
        <button type="button" class="btn btn--sm btn--ghost" :disabled="loading || saving" @click="load">{{ t('settings.tierLedger.refresh') }}</button>
      </div>

      <div class="tierledger__chains">
        <div v-for="tier in TIERS" :key="tier.id" class="tierledger__chain">
          <span class="tierledger__chain-name">{{ tier.label }}</span>
          <code class="tierledger__chain-models">
            {{ (data.tiers?.[tier.id] ?? []).join(' → ') || t('settings.tierLedger.notConfigured') }}
          </code>
        </div>
      </div>
    </template>
  </section>
</template>

<style scoped>
.tierledger__banner {
  border: 1px solid color-mix(in srgb, var(--danger) 45%, var(--border));
  background: color-mix(in srgb, var(--danger) 8%, transparent);
  color: var(--danger);
  border-radius: var(--radius-sm);
  padding: 8px 10px;
  font-size: var(--fs-xs);
}
.tierledger__empty {
  color: var(--text-dim);
  font-size: var(--fs-sm);
  padding: var(--sp-3) 0;
}
.tierledger__status {
  display: flex;
  flex-wrap: wrap;
  justify-content: space-between;
  gap: var(--sp-3);
  align-items: baseline;
  margin-top: var(--sp-2);
}
.tierledger__tier {
  display: flex;
  align-items: baseline;
  gap: 8px;
}
.tierledger__tier-label,
.tierledger__lock {
  color: var(--text-dim);
  font-size: var(--fs-xs);
}
.tierledger__tier-value {
  font-size: var(--fs-lg);
  text-transform: capitalize;
  color: var(--accent);
}
.tierledger__tier[data-tier='turbo'] .tierledger__tier-value { color: var(--ok); }
.tierledger__tier[data-tier='flagship'] .tierledger__tier-value { color: var(--warn); }
.tierledger__budget-head {
  display: flex;
  gap: var(--sp-3);
  font-size: var(--fs-xs);
  color: var(--text-muted);
}
.tierledger__remaining { font-variant-numeric: tabular-nums; }
.tierledger__bar {
  height: 6px;
  border-radius: var(--radius-xs);
  background: var(--bg-elevated);
  overflow: hidden;
  margin: 8px 0;
}
.tierledger__bar-fill {
  display: block;
  height: 100%;
  background: var(--ok);
  transition: width var(--dur-enter) var(--ease-standard);
}
.tierledger__bar[data-state='degraded'] .tierledger__bar-fill { background: var(--warn); }
.tierledger__bar[data-state='emergency'] .tierledger__bar-fill,
.tierledger__bar[data-state='depleted'] .tierledger__bar-fill { background: var(--danger); }
.tierledger__nums {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(140px, 1fr));
  gap: var(--sp-2) var(--sp-4);
  margin: 0 0 var(--sp-2);
  font-size: var(--fs-xs);
}
.tierledger__nums > div { display: flex; justify-content: space-between; gap: 8px; }
.tierledger__nums dt { color: var(--text-dim); }
.tierledger__nums dd { margin: 0; font-variant-numeric: tabular-nums; }
.tierledger__reasons {
  font-size: var(--fs-xs);
  color: var(--text-muted);
  margin-bottom: var(--sp-2);
}
.tierledger__reasons-title {
  color: var(--text-dim);
  font-weight: 600;
}
.tierledger__reasons ul {
  margin: 4px 0 0;
  padding-left: 18px;
  display: flex;
  flex-direction: column;
  gap: 2px;
}
.tierledger__lock-actions {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
  margin-bottom: var(--sp-2);
}
.tierledger__lock-actions .btn.is-active {
  border-color: var(--accent);
  color: var(--accent);
}
.tierledger__chains {
  display: flex;
  flex-direction: column;
  gap: 4px;
  font-size: var(--fs-xs);
}
.tierledger__chain {
  display: grid;
  grid-template-columns: 42px 1fr;
  gap: 8px;
  align-items: baseline;
}
.tierledger__chain-name { color: var(--text-dim); }
.tierledger__chain-models {
  color: var(--text-muted);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
</style>
