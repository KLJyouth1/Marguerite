<!-- 输入区脚注的上下文用量表（M1 Task 5）。
     数据来自网关 `usage.status` 的 `sessions[i].contextStatus`（Task 1/2 的载荷）：
     已用 / 窗口 / 工具占比 / 压力档位 / 窗口来源。

     判据纪律：没有 `contextStatus`、或窗口未知（0）时**整块不渲染**——
     渲染一个 0% 会教用户误读成「窗口很空」，比不显示更糟。
     窗口来自预算回退（`windowSource=budget_fallback`）时必须多一句说明，
     不能把 7500 的预算伪装成模型的真实 10k 窗口。 -->
<template>
  <div
    v-if="visible"
    class="ctx-meter"
    :class="`ctx-meter--${tier}`"
    data-testid="context-meter"
    :title="aria"
    :aria-label="aria"
  >
    <span class="ctx-meter__used" data-testid="context-meter-used">{{ usedK }}k</span>
    <span class="ctx-meter__sep" aria-hidden="true">/</span>
    <span class="ctx-meter__window">{{ windowK }}k</span>
    <span class="ctx-meter__pct">({{ pct }}%)</span>
    <span v-if="toolK !== null" class="ctx-meter__tool" data-testid="context-meter-tool">
      {{ t('chat.contextMeter.tool', { k: toolK }) }}
    </span>
    <span
      v-if="windowApprox"
      class="ctx-meter__approx"
      data-testid="context-meter-window-approx"
    >{{ t('chat.contextMeter.windowApprox') }}</span>
  </div>
</template>

<script setup lang="ts">
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'
import type { ContextStatus } from '@/composables/chat/useChatUsageWidget'

const props = defineProps<{ status?: ContextStatus | null }>()

const { t } = useI18n()

const TIERS = ['normal', 'watch', 'advise', 'compact'] as const
type Tier = typeof TIERS[number]
const DEFAULT_LADDER: [number, number, number] = [0.6, 0.75, 0.85]

/** 取第一个「有限且 > 0」的数值（camel / snake 双拼兼容）。 */
function pickNumber(...values: unknown[]): number | null {
  for (const value of values) {
    const n = Number(value)
    if (Number.isFinite(n) && n > 0) return n
  }
  return null
}

const used = computed(() =>
  pickNumber(props.status?.contextTokens, props.status?.context_tokens) ?? 0,
)
const windowTokens = computed(() =>
  pickNumber(props.status?.contextWindowTokens, props.status?.context_window_tokens) ?? 0,
)
const toolTokens = computed(() =>
  pickNumber(props.status?.toolTokens, props.status?.tool_tokens),
)

const pressure = computed<number | null>(() => {
  const raw = Number(props.status?.pressure)
  if (Number.isFinite(raw)) return Math.min(1, Math.max(0, raw))
  // 网关没给 pressure 时按同一口径自算（used / window），不猜。
  if (windowTokens.value > 0) {
    return Math.min(1, used.value / windowTokens.value)
  }
  return null
})

const visible = computed(() =>
  props.status != null && windowTokens.value > 0 && pressure.value !== null,
)
const pct = computed(() => Math.round((pressure.value ?? 0) * 100))
const usedK = computed(() => Math.round(used.value / 100) / 10)
const windowK = computed(() => Math.round(windowTokens.value / 100) / 10)
const toolK = computed(() =>
  toolTokens.value === null ? null : Math.round(toolTokens.value / 100) / 10,
)

function sanitizeLadder(raw: unknown): [number, number, number] {
  if (!Array.isArray(raw) || raw.length !== 3) return DEFAULT_LADDER
  const [a, b, c] = raw.map(Number)
  const ok = [a, b, c].every(v => Number.isFinite(v) && v > 0 && v <= 1)
    && a < b && b < c
  return ok ? [a, b, c] : DEFAULT_LADDER
}

const tier = computed<Tier>(() => {
  const explicit = String(props.status?.tier ?? '').trim()
  if ((TIERS as readonly string[]).includes(explicit)) return explicit as Tier
  // 网关没给档位时用同一把尺子算（与 core `context::pressure::tier_for` 同口径）。
  const [watch, advise, compact] = sanitizeLadder(props.status?.ladder)
  const p = pressure.value ?? 0
  if (p >= compact) return 'compact'
  if (p >= advise) return 'advise'
  if (p >= watch) return 'watch'
  return 'normal'
})

const windowApprox = computed(() => {
  const source = String(props.status?.windowSource ?? props.status?.window_source ?? '')
  return source === 'budget_fallback'
})

const aria = computed(() => t('chat.contextMeter.aria', {
  used: usedK.value,
  window: windowK.value,
  pct: pct.value,
  tier: t(`chat.contextMeter.tier.${tier.value}`),
}))
</script>

<style scoped>
.ctx-meter {
  display: inline-flex;
  align-items: center;
  gap: 0.3rem;
  min-height: 22px;
  min-width: 0;
  padding: 0 0.5rem;
  border: 1px solid var(--border);
  border-radius: var(--radius-pill);
  background: color-mix(in srgb, var(--bg-elevated) 70%, transparent);
  color: var(--text-muted);
  font-size: var(--fs-xs);
  font-variant-numeric: tabular-nums;
  white-space: nowrap;
}

.ctx-meter__used {
  color: var(--text);
  font-weight: 600;
}

.ctx-meter__sep,
.ctx-meter__pct,
.ctx-meter__tool,
.ctx-meter__approx {
  color: var(--text-dim);
}

.ctx-meter--watch {
  border-color: color-mix(in srgb, var(--warn) 32%, var(--border));
}

.ctx-meter--advise {
  border-color: color-mix(in srgb, var(--warn) 50%, var(--border));
  background: color-mix(in srgb, var(--warn) 8%, transparent);
}

.ctx-meter--advise .ctx-meter__used {
  color: var(--warn);
}

.ctx-meter--compact {
  border-color: color-mix(in srgb, var(--danger) 50%, var(--border));
  background: color-mix(in srgb, var(--danger) 10%, transparent);
}

.ctx-meter--compact .ctx-meter__used {
  color: var(--danger);
}

@media (max-width: 720px) {
  .ctx-meter__tool,
  .ctx-meter__approx {
    display: none;
  }
}
</style>
