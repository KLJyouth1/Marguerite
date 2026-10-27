<!-- 单次工具调用的展示卡：参数与输出分开排版，按载荷类型（JSON / 代码 / 差异 /
     链接 / 纯文本）换不同的渲染方式，并支持复制与超长截断提示。

     为什么要单独一张卡：编排泳道里的工具行此前只有「名字 + 一行参数预览」，
     输出（outputPreview）根本没渲染；同一行里 JSON、命令行、diff、URL 长得一样。
     这里把「调用了什么」与「拿到了什么」都摊开，且默认折叠 —— 过程可见但不喧宾夺主。 -->
<script setup lang="ts">
import { computed, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import Icon from '@/components/Icon.vue'
import { toolIconName } from '@/utils/chat/toolDisplay'
import { diffLineTone, summarizeToolArgs, viewToolPayload } from '@/utils/chat/toolPayload'

const props = defineProps<{
  tool: string
  args?: string
  output?: string
  status: 'running' | 'ok' | 'error'
  elapsedMs?: number | null
  /** 只有一条调用时可直接展开，省一次点击 */
  defaultOpen?: boolean
}>()

const { t } = useI18n()
const open = ref(props.defaultOpen ?? false)

const argsView = computed(() => viewToolPayload(props.args))
const outputView = computed(() => viewToolPayload(props.output))
const argsSummary = computed(() => summarizeToolArgs(props.args))
const iconName = computed(() => toolIconName(props.tool))
const diffLines = computed(() =>
  outputView.value.kind === 'diff' ? outputView.value.body.split('\n') : [],
)
const stateText = computed(() => {
  if (props.status === 'running') return t('chat.tool.running')
  if (props.status === 'error') return t('chat.tool.failed')
  return t('chat.tool.done')
})

const copied = ref<'args' | 'output' | null>(null)
async function copyBlock(kind: 'args' | 'output', text: string): Promise<void> {
  if (!text) return
  try {
    await navigator.clipboard?.writeText(text)
    copied.value = kind
    setTimeout(() => {
      if (copied.value === kind) copied.value = null
    }, 1500)
  } catch {
    // 剪贴板不可用（无权限 / 非安全上下文）：静默降级，不打断阅读
  }
}
</script>

<template>
  <div class="tool-inv" :class="`tool-inv--${status}`" :data-testid="'tool-invocation'">
    <button
      type="button"
      class="tool-inv__head"
      :aria-expanded="open"
      :aria-label="`${tool} ${stateText}`"
      @click="open = !open"
    >
      <Icon :name="iconName" :size="12" class="tool-inv__icon" aria-hidden="true" />
      <code class="tool-inv__name">{{ tool }}</code>
      <span v-if="argsSummary" class="tool-inv__args" :title="argsSummary">{{ argsSummary }}</span>
      <span class="tool-inv__state">{{ stateText }}</span>
      <span v-if="elapsedMs != null" class="tool-inv__ms">{{ elapsedMs }}ms</span>
      <Icon
        name="chevronRight"
        :size="12"
        class="tool-inv__chev"
        :class="{ 'is-open': open }"
        aria-hidden="true"
      />
    </button>

    <div v-if="open" class="tool-inv__body">
      <section v-if="argsView.kind !== 'empty'" class="tool-inv__block">
        <header class="tool-inv__block-head">
          <span>{{ t('chat.orchestration.runToolArgs') }}</span>
          <button type="button" class="tool-inv__copy" @click="copyBlock('args', argsView.body)">
            {{ copied === 'args' ? t('chat.orchestration.runToolCopied') : t('chat.orchestration.runToolCopy') }}
          </button>
        </header>
        <pre v-if="argsView.kind !== 'diff'" class="tool-inv__pre" :data-kind="argsView.kind">{{ argsView.body }}</pre>
        <pre v-else class="tool-inv__pre" data-kind="diff"><span
          v-for="(line, index) in argsView.body.split('\n')"
          :key="index"
          class="tool-inv__line"
          :data-tone="diffLineTone(line)"
        >{{ line }}</span></pre>
        <p v-if="argsView.truncated" class="tool-inv__note">
          {{ t('chat.orchestration.runTruncated', { n: argsView.bytes }) }}
        </p>
      </section>

      <section class="tool-inv__block">
        <header class="tool-inv__block-head">
          <span>{{ t('chat.orchestration.runToolOutput') }}</span>
          <button
            v-if="outputView.kind !== 'empty'"
            type="button"
            class="tool-inv__copy"
            @click="copyBlock('output', outputView.body)"
          >
            {{ copied === 'output' ? t('chat.orchestration.runToolCopied') : t('chat.orchestration.runToolCopy') }}
          </button>
        </header>
        <p v-if="outputView.kind === 'empty'" class="tool-inv__empty">
          {{ status === 'running'
            ? t('chat.orchestration.runToolPending')
            : t('chat.orchestration.runToolNoOutput') }}
        </p>
        <a
          v-else-if="outputView.kind === 'link'"
          class="tool-inv__link"
          :href="outputView.body"
          target="_blank"
          rel="noopener noreferrer"
        >{{ outputView.body }}</a>
        <pre v-else-if="outputView.kind !== 'diff'" class="tool-inv__pre" :data-kind="outputView.kind">{{ outputView.body }}</pre>
        <pre v-else class="tool-inv__pre" data-kind="diff"><span
          v-for="(line, index) in diffLines"
          :key="index"
          class="tool-inv__line"
          :data-tone="diffLineTone(line)"
        >{{ line }}</span></pre>
        <p v-if="outputView.truncated" class="tool-inv__note">
          {{ t('chat.orchestration.runTruncated', { n: outputView.bytes }) }}
        </p>
      </section>
    </div>
  </div>
</template>

<style scoped>
.tool-inv {
  border: 1px solid var(--hairline);
  border-radius: var(--radius-sm);
  background: color-mix(in srgb, var(--bg-surface-2) 55%, transparent);
  overflow: hidden;
}

.tool-inv--running { border-color: color-mix(in srgb, var(--accent) 34%, var(--hairline)); }
.tool-inv--error { border-color: color-mix(in srgb, var(--danger) 38%, var(--hairline)); }

.tool-inv__head {
  display: flex;
  align-items: center;
  gap: 6px;
  width: 100%;
  min-width: 0;
  padding: 4px 8px;
  border: 0;
  background: none;
  color: var(--text-muted);
  font-size: 10.5px;
  text-align: left;
  cursor: pointer;
}

.tool-inv__head:hover { background: var(--bg-hover); }

.tool-inv__icon { flex-shrink: 0; color: var(--text-dim); }
.tool-inv--ok .tool-inv__icon { color: var(--ok, var(--accent)); }
.tool-inv--error .tool-inv__icon { color: var(--danger); }
.tool-inv--running .tool-inv__icon { color: var(--accent); }

.tool-inv__name {
  flex-shrink: 0;
  color: var(--text);
  font-size: 10.5px;
  font-weight: 600;
}

.tool-inv__args {
  min-width: 0;
  flex: 1;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  /* 小字号文本统一用 muted：dim 在深色主题下约 4.2:1，低于 WCAG AA 的 4.5:1。 */
  color: var(--text-muted);
}

.tool-inv__state { flex-shrink: 0; color: var(--text-muted); }
.tool-inv--ok .tool-inv__state { color: var(--ok, var(--accent)); }
.tool-inv--error .tool-inv__state { color: var(--danger); }
.tool-inv--running .tool-inv__state { color: var(--accent); }

.tool-inv__ms {
  flex-shrink: 0;
  color: var(--text-muted);
  font-variant-numeric: tabular-nums;
}

.tool-inv__chev {
  flex-shrink: 0;
  color: var(--text-dim);
  transition: transform var(--dur-fast) var(--ease-standard);
}
.tool-inv__chev.is-open { transform: rotate(90deg); }

.tool-inv__body {
  display: flex;
  flex-direction: column;
  gap: 6px;
  padding: 0 8px 8px;
}

.tool-inv__block-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
  margin-bottom: 3px;
  color: var(--text-muted);
  font-size: 10px;
  letter-spacing: .04em;
  text-transform: uppercase;
}

.tool-inv__copy {
  padding: 0 6px;
  border: 1px solid var(--hairline);
  border-radius: var(--radius-pill);
  background: transparent;
  color: var(--text-muted);
  font-size: 9.5px;
  cursor: pointer;
}
.tool-inv__copy:hover { color: var(--accent); border-color: var(--accent); }

.tool-inv__pre {
  margin: 0;
  max-height: 220px;
  overflow: auto;
  padding: 6px 8px;
  border-radius: var(--radius-sm);
  background: var(--bg);
  color: var(--text-muted);
  font-family: var(--font-mono, ui-monospace, monospace);
  font-size: 10.5px;
  line-height: 1.5;
  white-space: pre;
  tab-size: 2;
}

.tool-inv__pre[data-kind='json'] { color: var(--text); }
.tool-inv__pre[data-kind='text'] { white-space: pre-wrap; word-break: break-word; }
.tool-inv__pre[data-kind='diff'] { white-space: pre; }

.tool-inv__line { display: block; }
.tool-inv__line[data-tone='add'] { color: var(--ok, var(--accent)); }
.tool-inv__line[data-tone='del'] { color: var(--danger); }
.tool-inv__line[data-tone='hunk'] { color: var(--info); }
.tool-inv__line[data-tone='meta'] { color: var(--text-muted); }
.tool-inv__line[data-tone='context'] { color: var(--text-muted); }

.tool-inv__empty { margin: 0; color: var(--text-muted); font-size: 10.5px; }
.tool-inv__note { margin: 2px 0 0; color: var(--text-muted); font-size: 9.5px; }

.tool-inv__link {
  color: var(--accent);
  font-size: 10.5px;
  word-break: break-all;
  text-decoration: underline;
  text-underline-offset: 2px;
}

@media (prefers-reduced-motion: reduce) {
  .tool-inv__chev { transition: none; }
}
</style>
