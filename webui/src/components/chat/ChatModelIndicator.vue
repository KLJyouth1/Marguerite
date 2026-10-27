<!-- P3 全域打通 · 聊天页主模型指示。
     为什么需要它：切换模型/部署本地模型后，聊天页此前**完全没有反馈**——用户不知道
     当前这条消息会发给谁。这不是"少一个装饰"，而是让"模型三要素（provider/model/
     endpoint）"在用户最常驻的界面里不可见，与 P3「任何一处变更全台一致」直接冲突。
     实现纪律：
       · 单一事实源 = `config.get` 的 `llm`（由后端 `effective_model_fields` 对齐 models.json），
         不在前端另算；
       · 事件驱动 = 订阅 `models.changed`（本地中心/CLI/设置页任一改动都会广播）；
       · 降级安全 = RPC 不可用或字段缺失时**整块不渲染**，绝不显示"未知模型"这类噪声；
       · 可访问 = 用 `title` + `aria-label` 给全量信息，视觉上只占一枚小胶囊。 -->
<template>
  <RouterLink
    v-if="visible"
    class="model-indicator"
    :to="{ path: '/control/settings', query: { section: 'provider' } }"
    :title="tooltip"
    :aria-label="tooltip"
  >
    <span class="model-indicator__dot" :class="{ 'model-indicator__dot--local': isLocal }" aria-hidden="true" />
    <span class="model-indicator__name">{{ modelName }}</span>
    <span v-if="providerName" class="model-indicator__provider">{{ providerName }}</span>
    <span v-if="profileCount" class="model-indicator__count" :title="profilesTooltip">{{ profileCount }}</span>
  </RouterLink>
</template>

<script setup lang="ts">
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'
import { RouterLink } from 'vue-router'
import { useLlmProfiles } from '@/composables/useLlmProfiles'
import { useRpcStore } from '@/stores/rpc'

const { t } = useI18n()
const rpc = useRpcStore()
const { active, profileCount } = useLlmProfiles()

const provider = computed(() => active.value.provider)
const modelName = computed(() => active.value.model)
const baseUrl = computed(() => active.value.baseUrl)

/** 本地/回环端点：用一个小圆点区分"离线可跑"，与设置页 `is_keyless_endpoint` 同语义。 */
const isLocal = computed(() => {
  const p = provider.value.toLowerCase()
  const u = baseUrl.value.toLowerCase()
  return p === 'ollama' || /localhost|127\.0\.0\.1|\[::1\]|0\.0\.0\.0/.test(u)
})

/** 三者皆空时不渲染：老网关 / RPC 失败 / 尚未配置都走这里，不给用户噪声。 */
const visible = computed(() => Boolean(modelName.value) && rpc.state === 'connected')

const providerName = computed(() => (provider.value ? provider.value : ''))

const tooltip = computed(() =>
  t('chat.modelIndicator.tooltip', {
    model: modelName.value,
    provider: provider.value || '—',
    endpoint: baseUrl.value || '—',
  }),
)

const profilesTooltip = computed(() =>
  t('chat.modelIndicator.profilesTooltip', { count: profileCount.value }),
)
</script>

<style scoped>
.model-indicator {
  display: inline-flex;
  align-items: center;
  gap: 0.375rem;
  max-width: 22rem;
  padding: 0.125rem 0.5rem;
  border: 1px solid var(--border);
  border-radius: var(--radius-pill);
  background: transparent;
  color: var(--text-dim);
  font-size: 0.75rem;
  line-height: 1.5;
  text-decoration: none;
  transition:
    border-color var(--dur-fast) var(--ease-standard),
    color var(--dur-fast) var(--ease-standard);
}
.model-indicator:hover,
.model-indicator:focus-visible {
  border-color: var(--accent);
  color: var(--text);
}
.model-indicator:focus-visible {
  outline: none;
  box-shadow: var(--focus-ring);
}

/* 本地端点用强调色，云端用中性色：一眼区分"要不要联网/花不花钱"。 */
.model-indicator__dot {
  flex: 0 0 auto;
  width: 6px;
  height: 6px;
  border-radius: var(--radius-pill);
  background: var(--text-dim);
}
.model-indicator__dot--local {
  background: var(--accent);
}

.model-indicator__name {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-variant-numeric: tabular-nums;
}

.model-indicator__provider {
  flex: 0 0 auto;
  padding-left: 0.375rem;
  border-left: 1px solid var(--border);
  color: var(--text-dim);
  opacity: 0.85;
}

.model-indicator__count {
  flex: 0 0 auto;
  min-width: 1.125rem;
  padding: 0 0.25rem;
  border-radius: var(--radius-pill);
  background: color-mix(in srgb, var(--accent) 16%, transparent);
  color: var(--accent);
  font-size: 0.6875rem;
  line-height: 1.25rem;
  text-align: center;
  font-variant-numeric: tabular-nums;
}
</style>
