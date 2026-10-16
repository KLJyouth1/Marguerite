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
}>(), {
  visible: false,
  phaseLabel: '',
  seconds: 0,
  suspended: false,
  suspendLabel: '',
  showSeconds: true,
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
@keyframes chat-wait-pulse {
  0%, 100% { opacity: 0.35; transform: scale(0.85); }
  50% { opacity: 1; transform: scale(1); }
}
@media (prefers-reduced-motion: reduce) {
  .chat-wait-indicator__pulse { animation: none; opacity: 0.8; }
}
</style>