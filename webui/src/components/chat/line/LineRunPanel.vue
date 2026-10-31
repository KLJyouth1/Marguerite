<template>
  <div v-if="line.lastRun.value" class="lrp" data-testid="line-run-panel">
    <LineRunCard
      :run="line.lastRun.value"
      :catalog="line.catalog.value"
      :retro="line.retro.value"
      :retro-loading="line.retroLoading.value"
      @load-retro="onLoadRetro"
    />
    <FiveWhyCard
      :open="retroOpen"
      :retro="line.retro.value"
      :loading="line.retroLoading.value"
      :run-id="retroRunId"
      :error="retroErrorText"
      @close="retroOpen = false"
      @retry="onLoadRetro"
    />
  </div>
</template>

<script setup lang="ts">
// 产线回执面板（LineRunPanel）—— 回执卡 + 复盘单的一对装配，ChatView 只需挂一个标签。
//
// 它存在的理由是那条"点了没反应"缺陷：`loadRetro` 失败时把 `retro` 归零、原因写进
// `lastRunError`，若父级只在成功时开单，复盘按钮就会静默吞掉点击。所以本面板在
// **成功与失败两条路径上都开单**——失败时把原因经 `error` prop 喂给 FiveWhyCard
// （LINE_* 码在这里过 `lineCodeText` 翻译，界面永远不该看到裸机器码）。
// `lastRunError` 同时承载派单错误，因此只有当单子绑定的那次运行没有产出复盘
// （`retro === null` 且单子已打开）时才透传，别的运行状态的错误不进复盘单。
import { computed, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'

import FiveWhyCard from './FiveWhyCard.vue'
import LineRunCard from './LineRunCard.vue'
import { lineCodeText } from '@/composables/line/lineCode'
import type { LineWorkflow } from '@/composables/line/useLineWorkflow'

const props = defineProps<{
  line: LineWorkflow
}>()

const { t } = useI18n()

const retroOpen = ref(false)
/** 单子最近一次请求的 runId：与传给 loadRetro 的同一个值，不做事后 `lastRun` 反查。 */
const retroRunId = ref('')

// 换了一次运行就关掉旧单：复盘单讲的是"那一次"的证据，新回执进来还挂着上一次
// 的单子，等于让两份证据共用一个标题（面板在父级按 `lastRun` 存在与否渲染，
// 连续两次运行之间不会自动卸载，所以必须显式关）。
watch(
  () => props.line.lastRun.value?.runId,
  () => {
    retroOpen.value = false
    retroRunId.value = ''
  },
)

async function onLoadRetro(runId: string): Promise<void> {
  retroRunId.value = runId
  await props.line.loadRetro(runId)
  // 成败都开单：失败不是"什么都不发生"的理由。
  retroOpen.value = true
}

const retroErrorText = computed<string | null>(() => {
  if (!retroOpen.value) return null
  if (props.line.retro.value !== null) return null
  const raw = props.line.lastRunError.value
  return raw ? lineCodeText(raw, t) || null : null
})
</script>

<style scoped>
.lrp {
  display: flex;
  flex-direction: column;
  gap: var(--sp-2);
}
</style>
