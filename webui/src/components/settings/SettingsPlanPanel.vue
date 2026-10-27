<script setup lang="ts">
// 计划执行配置面板（Phase 1：把 `plan.*` 暴露到 WebUI，含 `require_approval` 审批闸门）。
//
// 数据源刻意**不复用** `config.get/patch`：那一对是 PY_FIRST —— 侧车在线时读写的是
// Python 侧的 `config.toml`，而**计划执行器读的是 `~/.marguerite/config.yaml`**。
// 用它们写 `plan.*` 会得到「开关已开启、执行时没有闸门」的静默失效。
// 因此这里走 Rust-first 的 `config.plan.get` / `config.plan.set`（同一份 config.yaml），
// 并在写入后如实展示后端返回的 `restartRequired`（执行器配置是进程级缓存）。
import { computed, onMounted, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import ControlSwitch from '@/components/ControlSwitch.vue'
import { useRpcStore } from '@/stores/rpc'

const { t } = useI18n()
const rpc = useRpcStore()

interface PlanConfig {
  auto_decompose: boolean
  require_approval: boolean
  max_steps: number
  checkpoint_interval_steps: number
  heartbeat_interval_s: number
  step_timeout_secs: number
}
interface PlanPayload {
  ok?: boolean
  plan?: Partial<PlanConfig>
  source?: string
  error?: string
  changed?: boolean
  restartRequired?: boolean
}

const plan = ref<PlanConfig | null>(null)
const source = ref('')
const error = ref('')
const notice = ref('')
const busy = ref(false)

/// 后端是否真的提供这一对方法（浏览器直连旧网关时可能没有 → 如实提示，不假装能改）
const available = computed(() => rpc.supportsMethod('config.plan.get'))

async function load() {
  error.value = ''
  notice.value = ''
  if (!available.value) return
  try {
    const r = await rpc.call<PlanPayload>('config.plan.get', {})
    if (r?.error) {
      error.value = r.error
      return
    }
    plan.value = r.plan as PlanConfig
    source.value = String(r.source || '')
  } catch (e) {
    error.value = e instanceof Error ? e.message : String(e)
    plan.value = null
  }
}

/// 写单个字段：后端只回 `changed` / `restartRequired`，UI 据此如实提示（不臆测已生效）。
async function save(patch: Record<string, unknown>) {
  if (busy.value) return
  busy.value = true
  error.value = ''
  notice.value = ''
  try {
    const r = await rpc.call<PlanPayload>('config.plan.set', patch)
    if (r?.error) {
      error.value = r.error
      return
    }
    if (r.plan) plan.value = r.plan as PlanConfig
    notice.value = r.restartRequired
      ? t('settings.plan.restartRequired')
      : r.changed
        ? t('settings.plan.saved')
        : t('settings.plan.unchanged')
  } catch (e) {
    error.value = e instanceof Error ? e.message : String(e)
    void load() // 失败时回读真实值，避免界面停在乐观值上
  } finally {
    busy.value = false
  }
}

function setBool(key: 'auto_decompose' | 'require_approval', value: boolean) {
  if (plan.value) plan.value[key] = value
  void save({ [key]: value })
}

function setNumber(key: 'max_steps' | 'step_timeout_secs' | 'heartbeat_interval_s', value: number) {
  if (!Number.isFinite(value) || value < 0) {
    void load()
    return
  }
  if (plan.value) plan.value[key] = Math.round(value)
  void save({ [key]: Math.round(value) })
}

onMounted(load)
</script>

<template>
  <section class="control-section" data-testid="settings-plan">
    <div class="control-section__head">
      <h3 class="control-section__title">{{ t('settings.plan.title') }}</h3>
      <p class="control-section__desc">{{ t('settings.plan.desc') }}</p>
    </div>

    <p v-if="!available" class="plan-notice" role="status">{{ t('settings.plan.unavailable') }}</p>
    <template v-else>
      <p v-if="error" class="plan-notice plan-notice--error" role="alert">{{ error }}</p>
      <p v-else-if="notice" class="plan-notice" role="status">{{ notice }}</p>
      <p v-if="source" class="plan-source">{{ t('settings.plan.source', { source }) }}</p>

      <label v-if="plan" class="control-row">
        <div class="control-row__label-block">
          <span class="control-row__label">{{ t('settings.plan.requireApprovalLabel') }}</span>
          <span class="control-row__desc">{{ t('settings.plan.requireApprovalDesc') }}</span>
        </div>
        <div class="control-row__control">
          <ControlSwitch
            name="plan_require_approval"
            :checked="plan.require_approval"
            :disabled="busy"
            :aria-label="t('settings.plan.requireApprovalLabel')"
            @change="(v) => setBool('require_approval', v)"
          />
        </div>
      </label>

      <label v-if="plan" class="control-row">
        <div class="control-row__label-block">
          <span class="control-row__label">{{ t('settings.plan.autoDecomposeLabel') }}</span>
          <span class="control-row__desc">{{ t('settings.plan.autoDecomposeDesc') }}</span>
        </div>
        <div class="control-row__control">
          <ControlSwitch
            name="plan_auto_decompose"
            :checked="plan.auto_decompose"
            :disabled="busy"
            :aria-label="t('settings.plan.autoDecomposeLabel')"
            @change="(v) => setBool('auto_decompose', v)"
          />
        </div>
      </label>

      <label v-if="plan" class="control-row">
        <div class="control-row__label-block">
          <span class="control-row__label">{{ t('settings.plan.maxStepsLabel') }}</span>
          <span class="control-row__desc">{{ t('settings.plan.maxStepsDesc') }}</span>
        </div>
        <div class="control-row__control">
          <input
            class="control-input control-input--narrow"
            name="plan_max_steps"
            type="number"
            min="1"
            step="1"
            inputmode="numeric"
            :value="plan.max_steps"
            :disabled="busy"
            :aria-label="t('settings.plan.maxStepsLabel')"
            @change="(e) => setNumber('max_steps', Number((e.target as HTMLInputElement).value))"
          >
        </div>
      </label>

      <label v-if="plan" class="control-row">
        <div class="control-row__label-block">
          <span class="control-row__label">{{ t('settings.plan.stepTimeoutLabel') }}</span>
          <span class="control-row__desc">{{ t('settings.plan.stepTimeoutDesc') }}</span>
        </div>
        <div class="control-row__control">
          <input
            class="control-input control-input--narrow"
            name="plan_step_timeout_secs"
            type="number"
            min="0"
            step="10"
            inputmode="numeric"
            :value="plan.step_timeout_secs"
            :disabled="busy"
            :aria-label="t('settings.plan.stepTimeoutLabel')"
            @change="(e) => setNumber('step_timeout_secs', Number((e.target as HTMLInputElement).value))"
          >
        </div>
      </label>

      <label v-if="plan" class="control-row">
        <div class="control-row__label-block">
          <span class="control-row__label">{{ t('settings.plan.heartbeatLabel') }}</span>
        </div>
        <div class="control-row__control">
          <input
            class="control-input control-input--narrow"
            name="plan_heartbeat_interval_s"
            type="number"
            min="1"
            step="5"
            inputmode="numeric"
            :value="plan.heartbeat_interval_s"
            :disabled="busy"
            :aria-label="t('settings.plan.heartbeatLabel')"
            @change="(e) => setNumber('heartbeat_interval_s', Number((e.target as HTMLInputElement).value))"
          >
        </div>
      </label>
    </template>
  </section>
</template>

<style scoped>
.plan-notice {
  border: 1px solid color-mix(in srgb, var(--warn) 35%, var(--border));
  border-radius: var(--radius-sm);
  color: var(--warn);
  font-size: var(--fs-xs);
  margin: 0 0 var(--sp-2) var(--sp-1);
  padding: var(--sp-2) var(--sp-3);
}

.plan-notice--error {
  border-color: color-mix(in srgb, var(--danger) 35%, var(--border));
  color: var(--danger);
}

.plan-source {
  color: var(--text-dim);
  font-size: var(--fs-xs);
  margin: 0 0 var(--sp-2) var(--sp-1);
}
</style>
