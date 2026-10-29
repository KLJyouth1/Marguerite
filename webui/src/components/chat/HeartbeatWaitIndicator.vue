<!-- F-09 等待指示器：长等待期间告诉用户「在等什么 / 等了多久」。
     纯展示，不持有任何计时器——秒数来自父级（与 N1 的**单一回合时钟**同源），
     因此绝不会重置软看门狗的内容静默时钟，也不会和旁边的时间读数互相矛盾。 -->
<script setup lang="ts">
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'

const props = withDefaults(defineProps<{
  /** 是否展示等待指示。 */
  visible?: boolean
  /** 相位文案（如「等待模型响应」）；缺省时用通用「仍在运行」。 */
  phaseLabel?: string
  /** 已等待秒数（与回合时钟同源）。 */
  seconds?: number
  /** 是否因合法静默（工具执行中 / 审批挂起 / ensemble）而挂起看门狗。 */
  suspended?: boolean
  /** 挂起原因文案（工具执行中 / 等待审批 / 融合阶段）。 */
  suspendLabel?: string
  /** 是否展示秒数（默认开）。 */
  showSeconds?: boolean
  /**
   * 补充说明（第二行，弱化样式）。用于回答「为什么慢 / 我能做什么」——
   * 真机审计 2026-10-29 P1-3：并行会话里第二页干等 27s，屏幕上只有一个转圈，
   * 用户分不清"卡死"和"在排队"。
   *
   * ⚠️ 它**不进**上面那个 `role="status" aria-live="polite"` 的句子：那行里的秒数
   * 每秒变化，把「已等 Ns」的说明也塞进 live region，读屏会每秒重播一次。
   * 所以这里用 `role="note"`（可被读到，但不主动播报）。
   */
  hint?: string
}>(), {
  visible: false,
  phaseLabel: '',
  seconds: 0,
  suspended: false,
  suspendLabel: '',
  showSeconds: true,
  hint: '',
})

const { t } = useI18n()

const label = computed(() =>
  props.suspended && props.suspendLabel
    ? props.suspendLabel
    : (props.phaseLabel || t('chat.waitIndicator.running')),
)
const secondsText = computed(() => `${Math.max(0, Math.floor(props.seconds || 0))}s`)
</script>

<template>
  <p
    v-if="visible"
    class="chat-wait-indicator"
    :data-suspended="suspended ? 'true' : 'false'"
    data-testid="chat-wait-indicator"
    role="status"
    aria-live="polite"
  >
    <span class="chat-wait-indicator__pulse" aria-hidden="true" />
    <span class="chat-wait-indicator__label">{{ label }}</span>
    <span v-if="showSeconds" class="chat-wait-indicator__seconds" aria-hidden="true">
      · {{ secondsText }}
    </span>
  </p>
  <p
    v-if="visible && hint"
    class="chat-wait-indicator__hint"
    role="note"
    data-testid="chat-wait-indicator-hint"
  >{{ hint }}</p>
</template>

<style scoped>
.chat-wait-indicator {
  display: flex;
  align-items: center;
  gap: 6px;
  margin: 0 0 6px;
  color: var(--text-dim);
  font-size: var(--fs-xs);
  line-height: 1.4;
}
.chat-wait-indicator__pulse {
  width: 7px;
  height: 7px;
  flex: none;
  border-radius: var(--radius-full);
  background: var(--accent);
  animation: chat-wait-pulse 1.4s var(--ease-standard) infinite;
}
.chat-wait-indicator[data-suspended='true'] .chat-wait-indicator__pulse {
  background: var(--warn);
}
.chat-wait-indicator__seconds {
  font-variant-numeric: tabular-nums;
  color: var(--text-dim);
  opacity: 0.8;
}
/* 第二行说明：比主行更弱，且限宽换行 —— 一行 60+ 字的长句在窄侧栏里会撑破布局。 */
.chat-wait-indicator__hint {
  margin: -2px 0 6px 13px;
  max-width: 62ch;
  color: var(--text-dim);
  font-size: var(--fs-xs);
  line-height: 1.45;
  opacity: 0.85;
}
@keyframes chat-wait-pulse {
  0%, 100% { opacity: 0.35; transform: scale(0.85); }
  50% { opacity: 1; transform: scale(1); }
}
@media (prefers-reduced-motion: reduce) {
  .chat-wait-indicator__pulse { animation: none; opacity: 0.8; }
}
</style>