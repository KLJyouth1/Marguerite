<!-- 模型思考（reasoning / CoT）的渲染单元。
     三种形态，同一份内容：
       · `embedded`：嵌在活动折叠里，扁平块（无卡片 chrome，长文滚动）；
       · `nested`  ：嵌在活动折叠内的 `details`，去掉重复的左边线；
       · 默认      ：独立 `details` 折叠，历史消息里可回看。
     本次增强（真实数据，不臆造）：
       1. **流式可见**：`live` 时折叠默认展开 + 末尾呼吸点，用户能看着思考长出来；
       2. **体量读数**：字数 / 段落数（由 `part.text` 直接算出，非估算）；
       3. **可带走**：一键复制原始思考文本（长链路排查时刚需）；
       4. 流式结束后自动收起，避免占满屏幕（`open` 只绑 `live`）。 -->
<template>
  <section v-if="embedded && hasText" class="thinking-block">
    <div v-if="!hideSummary" class="thinking-block__header">
      <span v-if="live" class="thinking-pulse" aria-hidden="true" />
      <Icon v-else name="moreHorizontal" :size="12" aria-hidden="true" />
      <span>{{ summary }}</span>
      <span v-if="showMetrics" class="thinking-meta">{{ metrics }}</span>
    </div>
    <div class="thinking-block__body">{{ part.text }}</div>
  </section>
  <!-- `live` is the one prop that only the in-activity usage sets, so it doubles
       as the nested-context marker: inside ActivityDisclosure the activity body
       already draws the fold's left rule, and a second rule here reads as
       doubled chrome. -->
  <details
    v-else-if="hasText"
    class="thinking-fold"
    :class="{ 'thinking-fold--in-activity': nested || live, 'thinking-fold--live': live }"
    :open="open"
  >
    <summary class="thinking-fold__summary">
      <Icon class="thinking-fold__chevron" name="chevronRight" :size="12" />
      <span v-if="live" class="thinking-pulse" aria-hidden="true" />
      <span>{{ summary }}</span>
      <span v-if="showMetrics" class="thinking-meta">{{ metrics }}</span>
      <button
        v-if="canCopy"
        type="button"
        class="thinking-copy"
        :title="copied ? t('chat.reasoning.copied') : t('chat.reasoning.copy')"
        :aria-label="copied ? t('chat.reasoning.copied') : t('chat.reasoning.copy')"
        @click.stop.prevent="copyTrace"
      >
        <Icon :name="copied ? 'check' : 'copy'" :size="11" aria-hidden="true" />
      </button>
    </summary>
    <div ref="bodyEl" class="thinking-fold__body">{{ part.text }}</div>
  </details>
</template>

<script setup lang="ts">
import { computed, nextTick, onMounted, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import Icon from '@/components/Icon.vue'
import type { ChatPart } from '@/types/parts'

const { t } = useI18n()

const props = defineProps<{
  part: Extract<ChatPart, { type: 'reasoning' }>
  embedded?: boolean
  /** Nested inside the outer activity disclosure. */
  nested?: boolean
  /** Streaming turn: label the fold "Thinking · Ns" (matching the live
   * elapsed ticker) instead of the settled "Thought for …" wording. */
  live?: boolean
  /** The parent disclosure already names this content, so avoid a second
   * "thought" label when reasoning is embedded in a Plan process. */
  hideSummary?: boolean
}>()

const summary = computed(() => {
  const seconds = props.part.seconds || 0
  if (props.live) return t('chat.thinkingForSeconds', { seconds })
  if (seconds < 1) return t('chat.thoughtProcess')
  if (seconds < 60) return t('chat.thoughtForSeconds', { seconds })
  return t('chat.thoughtForMinutes', { minutes: Math.floor(seconds / 60), seconds: seconds % 60 })
})

/** 思考体量：字数与段落数，全部由文本本身算出（不做任何推断）。 */
const charCount = computed(() => props.part.text.trim().length)
/** 纯空白文本 = 没有思考内容：整块不渲染，避免出现点开空无一物的「思考」折叠。 */
const hasText = computed(() => charCount.value > 0)
const blockCount = computed(
  () => props.part.text.split(/\n\s*\n/).filter((s) => s.trim().length > 0).length,
)
const showMetrics = computed(() => charCount.value > 0)
const metrics = computed(() =>
  blockCount.value > 1
    ? t('chat.reasoning.metricsBlocks', { chars: charCount.value, blocks: blockCount.value })
    : t('chat.reasoning.metrics', { chars: charCount.value }),
)

const canCopy = computed(() => !props.live && charCount.value > 0)

/**
 * 折叠默认状态。
 *
 * 反馈（真机）：用户只看到「思考了 7 秒 · 65 字」，正文要再点一次才出来，
 * 于是普遍以为"模型没有思考内容"。短思考（≤600 字）本就该直接可读，
 * 长思考才值得默认收起 —— 默认展开会让一次长链路思考占满整屏。
 * 流式期间始终展开（能看着它长出来）。
 */
const SHORT_THINKING_CHARS = 600
const open = ref(props.live || (charCount.value > 0 && charCount.value <= SHORT_THINKING_CHARS))
watch(
  () => props.live,
  (live) => {
    if (live) open.value = true
    else if (charCount.value > SHORT_THINKING_CHARS) open.value = false
  },
)

const copied = ref(false)
let copiedTimer: ReturnType<typeof setTimeout> | null = null

async function copyTrace() {
  try {
    await navigator.clipboard?.writeText(props.part.text)
    copied.value = true
    if (copiedTimer) clearTimeout(copiedTimer)
    copiedTimer = setTimeout(() => {
      copied.value = false
    }, 1600)
  } catch {
    // 剪贴板被拒（无权限/非安全上下文）：保持静默，不弹错打断阅读。
  }
}

// 流式期间把视口钉在最新一行，否则用户看到的是开头那几句（内容一直在追加）。
const bodyEl = ref<HTMLElement | null>(null)
function stickToBottom() {
  const el = bodyEl.value
  if (el) el.scrollTop = el.scrollHeight
}
watch(
  () => props.part.text,
  () => {
    if (!props.live) return
    void nextTick(stickToBottom)
  },
)
onMounted(() => {
  if (props.live) void nextTick(stickToBottom)
})
</script>

<style scoped>
.thinking-block {
  min-width: 0;
}
.thinking-block__header {
  display: flex;
  align-items: center;
  gap: 0.375rem;
  margin-bottom: 0.25rem;
  color: var(--text-dim);
  font-size: 0.75rem;
  line-height: 1.5;
}
.thinking-block__body {
  color: var(--text-muted);
  font-size: 0.8125rem;
  line-height: 1.55;
  white-space: pre-wrap;
  word-break: break-word;
  max-height: 16rem;
  overflow-y: auto;
  padding-right: 0.5rem;
}

/* 思考体量读数：右对齐、低对比，只做「这轮想了多少」的标尺。 */
.thinking-meta {
  margin-left: auto;
  flex: 0 0 auto;
  color: var(--text-dim);
  font-variant-numeric: tabular-nums;
  opacity: 0.85;
}

/* 流式呼吸点：唯一一条 ambient 循环，周期复用 --dur-pulse。 */
.thinking-pulse {
  flex: 0 0 auto;
  width: 6px;
  height: 6px;
  border-radius: var(--radius-pill);
  background: var(--accent);
  animation: thinking-breathe var(--dur-pulse) var(--ease-standard) infinite;
}
@keyframes thinking-breathe {
  0%, 100% { opacity: 0.25; transform: scale(0.85); }
  50% { opacity: 1; transform: scale(1.15); }
}

.thinking-copy {
  flex: 0 0 auto;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 18px;
  height: 18px;
  margin-left: 0.25rem;
  padding: 0;
  border: 1px solid transparent;
  border-radius: var(--radius-xs);
  background: transparent;
  color: var(--text-dim);
  cursor: pointer;
  transition:
    color var(--dur-fast) var(--ease-standard),
    border-color var(--dur-fast) var(--ease-standard);
}
.thinking-copy:hover,
.thinking-copy:focus-visible {
  outline: 0;
  border-color: var(--border);
  color: var(--text);
}
.thinking-copy:focus-visible {
  box-shadow: var(--focus-ring);
}

/* Reasoning disclosure — self-contained fold (chevron, focus ring, capped
 * scrolling body) used for settled history rows and reusable as-is for the
 * live streaming fold, kept local so this part needs no shared sheet. */
.thinking-fold { margin: 0 0 0.5rem; font-size: 0.8125rem; color: var(--text-dim); }
.thinking-fold__summary {
  display: inline-flex; align-items: center; gap: 0.375rem;
  padding: 0.125rem 0.25rem; border-radius: var(--radius-sm);
  cursor: pointer; list-style: none; color: var(--text-dim); line-height: 1.5;
}
.thinking-fold__summary::-webkit-details-marker { display: none; }
.thinking-fold__summary:hover { color: var(--text-muted); }
.thinking-fold__summary:focus-visible {
  outline: none;
  box-shadow: var(--focus-ring);
}
.thinking-fold__chevron { flex-shrink: 0; transition: transform var(--dur-fast) var(--ease-standard); }
.thinking-fold[open] > .thinking-fold__summary .thinking-fold__chevron { transform: rotate(90deg); }
.thinking-fold__body {
  margin: 0.25rem 0 0.375rem; padding: 0.375rem 0.75rem;
  border-left: 2px solid var(--border); color: var(--text-muted);
  line-height: 1.55; white-space: pre-wrap; word-break: break-word;
  max-height: 16rem; overflow-y: auto;
}
/* Nested inside the activity fold the outer 1px frame already rules the left
 * edge, so the fold's own 2px rule (and the padding that indents from it)
 * would draw two parallel lines — drop both for that variant only. */
.thinking-fold--in-activity > .thinking-fold__body { border-left: none; padding-left: 0; }
/* 流式时给左轨一点强调色，让「正在思考」与「已完成的思考」一眼可分。 */
.thinking-fold--live > .thinking-fold__body {
  border-left-color: color-mix(in srgb, var(--accent) 55%, var(--border));
}
@media (prefers-reduced-motion: reduce) {
  .thinking-fold__chevron { transition: none; }
  .thinking-pulse { animation: none; opacity: 0.8; }
}
</style>
