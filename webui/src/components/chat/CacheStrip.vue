<!-- W5.6 缓存指标条（诚实口径）。
     为什么需要它：`cache.status` 里有两把不同的尺子 ——
       · 服务端前缀覆盖 = cache_read token / 归一后总输入 token（token 占比）；
       · 合格请求命中 = 至少命中一次的请求 / 达到 provider 最小可缓存长度的请求（请求占比）。
     旧 UI 把它们（以及本地 prefix 命中率）用 `Math.max(...)` 合成一个数，量纲不同 → 虚报。
     这里三个数**分开**显示，tooltip 写明分子/分母；只有旧网关（无新字段）才回退到
     `server_cache_hit_rate`（W5.1 定义为 coverage 别名），并显式打 `legacy` 标记。

     文案：优先走 i18n key（`chat.cacheStrip.*`），en 字典尚未收录时回退到英文常量。
     用动态 key（`te(key) ? t(key) : fallback`）是为了不触发 check-i18n 的
     「字面量 key 必须在 en 中定义」硬失败；locale 由其它智能体维护，待补 key 后自动生效。
-->
<template>
  <div class="cache-strip" data-testid="cache-strip">
    <span
      class="cache-strip__metric"
      data-testid="cache-strip-coverage"
      :title="coverageHint"
    >
      <span class="cache-strip__label">{{ coverageLabel }}</span>
      <span class="cache-strip__value">{{ formatPct(coveragePct) }}</span>
    </span>
    <span
      class="cache-strip__metric"
      data-testid="cache-strip-request-hit"
      :title="requestHitHint"
    >
      <span class="cache-strip__label">{{ requestHitLabel }}</span>
      <span class="cache-strip__value">{{ formatPct(requestHitRatePct) }}</span>
    </span>
    <span
      v-if="clientHitRatePct !== null && clientHitRatePct !== undefined"
      class="cache-strip__metric"
      data-testid="cache-strip-client"
      :title="clientHint"
    >
      <span class="cache-strip__label">{{ clientLabel }}</span>
      <span class="cache-strip__value">{{ formatPct(clientHitRatePct) }}</span>
    </span>
    <span
      v-if="legacy"
      class="cache-strip__legacy"
      data-testid="cache-strip-legacy"
      :title="legacyHint"
    >
      {{ legacyBadge }}
    </span>
  </div>
</template>

<script setup lang="ts">
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'

const LEGACY_HINT =
  'This gateway only reports server_cache_hit_rate, which is a server prefix coverage alias. Request-level hit rate is not reported and is never inferred.'

const props = withDefaults(
  defineProps<{
    /** 服务端前缀覆盖 0..100；null = 网关未上报 */
    coveragePct: number | null
    /** 合格请求命中率 0..100；null = 网关未上报 */
    requestHitRatePct: number | null
    /** 本地前缀缓存命中率 0..100（独立量纲，仅作对照） */
    clientHitRatePct?: number | null
    /** true = 只有旧 `server_cache_hit_rate`，coverage 是别名、请求命中率不可得 */
    legacy?: boolean
  }>(),
  { clientHitRatePct: null, legacy: false },
)

const { t, te } = useI18n()

/** 有 key 用翻译；没有时用英文回退（不写死 key 字面量，见文件头）。 */
function text(key: string, fallback: string): string {
  return te(key) ? t(key) : fallback
}

function formatPct(value: number | null | undefined): string {
  if (typeof value !== 'number' || !Number.isFinite(value)) {
    return text('chat.cacheStrip.unavailable', '—')
  }
  // 0..100 的百分数；最多一位小数，避免把噪声渲染成精确值。
  return `${Math.round(value * 10) / 10}%`
}

const coverageLabel = computed(() =>
  props.legacy
    ? text('chat.cacheStrip.legacyCoverage', 'Cache hit rate (legacy)')
    : text('chat.cacheStrip.coverage', 'Server prefix coverage'),
)
const coverageHint = computed(() => {
  if (props.legacy) return text('chat.cacheStrip.legacyHint', LEGACY_HINT)
  return text(
    'chat.cacheStrip.coverageHint',
    'Server prefix coverage = cache_read tokens / total input tokens. Anthropic total = input + cache_read + cache_creation; OpenAI/DeepSeek total = prompt_tokens (cache_read is already a subset).',
  )
})
const requestHitLabel = computed(() =>
  text('chat.cacheStrip.requestHitRate', 'Eligible request hit rate'),
)
const requestHitHint = computed(() =>
  props.legacy
    ? text('chat.cacheStrip.legacyHint', LEGACY_HINT)
    : text(
        'chat.cacheStrip.requestHitRateHint',
        "Eligible request hit rate = requests with cache_read > 0 / requests at or above the provider's minimum cacheable length (DeepSeek 1024, Anthropic 1024 or 2048 by model, OpenAI 1024 tokens).",
      ),
)
const clientLabel = computed(() => text('chat.cacheStrip.clientHitRate', 'Local prefix hits'))
const clientHint = computed(() =>
  text(
    'chat.cacheStrip.clientHitRateHint',
    "Local prefix cache = hits / lookups in this gateway's exact-prefix cache (independent of server-side caching; never mixed into the server metrics).",
  ),
)
const legacyBadge = computed(() => text('chat.cacheStrip.legacyBadge', 'legacy'))
const legacyHint = computed(() => text('chat.cacheStrip.legacyHint', LEGACY_HINT))
</script>

<style scoped>
.cache-strip {
  display: inline-flex;
  align-items: center;
  flex-wrap: wrap;
  gap: 0.25rem 0.75rem;
  font-size: var(--fs-xs);
  color: var(--text-dim);
  font-variant-numeric: tabular-nums;
}

.cache-strip__metric {
  display: inline-flex;
  align-items: baseline;
  gap: 0.3rem;
  white-space: nowrap;
}

.cache-strip__label {
  color: var(--text-dim);
}

.cache-strip__value {
  color: var(--text);
  font-weight: 600;
}

.cache-strip__legacy {
  padding: 0 0.4rem;
  border: 1px solid color-mix(in srgb, var(--warn) 45%, var(--border));
  border-radius: var(--radius-pill);
  background: color-mix(in srgb, var(--warn) 12%, transparent);
  color: var(--warn);
  font-size: var(--fs-xs);
  line-height: 1.5;
  letter-spacing: 0.04em;
  text-transform: uppercase;
}
</style>
