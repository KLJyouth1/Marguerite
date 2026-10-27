<template>
  <!-- S5 host-exit evidence card.

       Skeleton choice: `ErrorState.vue`, NOT `.msg-error-card`. The latter looks
       like a better fit but its styles live in a `<style scoped>` block inside
       `SystemMessage.vue`, so those class names are unreachable from any other
       component — reusing them would have produced an unstyled card with
       confident-looking class names. `ErrorState` is a real component with its
       own styles and a `message + retry` contract, which is exactly the shape
       needed here.

       Deliberate omission: `stderr_tail` NEVER appears in the headline. It can
       contain file paths, URLs and usernames, so it is confined to a collapsed
       <details> block. This mirrors the discipline in
       `core/src/model/error_taxonomy.rs`. -->
  <div v-if="visible" class="host-exit">
    <ErrorState :message="heading" :on-retry="onRetry" />

    <!-- Structured facts. Each row is omitted when its value is unknown rather
         than rendered as a placeholder that implies we looked and found
         nothing — "exit code unknown" and "we have no data" are different
         claims. -->
    <dl v-if="facts.length" class="host-exit__facts">
      <template v-for="fact in facts" :key="fact.label">
        <dt class="host-exit__fact-label">{{ fact.label }}</dt>
        <dd class="host-exit__fact-value">{{ fact.value }}</dd>
      </template>
    </dl>

    <!-- stderr tail: collapsed by default and capped for display. The core side
         already caps at 16KB; this cap is about not dumping a wall of text into
         the DOM when the user does expand it. -->
    <details v-if="displayTail" class="host-exit__details">
      <summary class="host-exit__summary">{{ t('chat.errors.hostExitStderrLabel') }}</summary>
      <pre class="host-exit__tail">{{ displayTail }}</pre>
    </details>
  </div>
</template>

<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import ErrorState from '@/components/ErrorState.vue'
import { useHostExitEvidence } from '@/composables/useHostExitEvidence'
import { useToasts } from '@/composables/useToasts'
import { useRpcStore } from '@/stores/rpc'
import { restartDesktopSidecar } from '@/utils/desktopSidecar'
import { HOST_EXITED, localizedChatErrorMessage } from '@/utils/chat/errors'

const { t } = useI18n()
const { pushToast } = useToasts()
const rpc = useRpcStore()

// Injects the App.vue-owned instance; falls back to a local one when mounted
// outside the app shell (same contract as `useGatewayReadiness`).
const { hasEvidence, sidecarPresent, source, exitCode, uptimeMs, stderrTail, errorContext, refresh } =
  useHostExitEvidence()

/** Cap the rendered tail. The full 16KB stays available via the RPC. */
const DISPLAY_TAIL_LIMIT = 2000

// 卡片是「用户可见的问题指示器」，必须反映**当前**状态而非历史记录：
// `exited` 是诊断证据（"曾经退出过"），`sidecar_present` 才是"现在是否在位"。
// 侧车已在位时继续喊「已退出」是假状态（实测：瞬时退出后新侧车已健康，
// 卡片仍常驻，见 useHostExitEvidence.ts 的 sidecar_present 契约注释）。
//
// 【2026-10-23 修复·启动即误报】落盘记录（`source: 'record'`）是**跨重启**的历史：
// 用户一进应用就看到「宿主进程已退出，存活 56 分钟 21 秒」，而那段时间其实是
// **上一次运行**的存活时长。
//
// 第二轮实测还发现 `sidecar_present` 本身不可靠：本机 python 侧车明明在跑
// （21:57:05 启动），网关仍报 `false`（判据只看本进程内的注册表）。所以
// 「历史记录 + 侧车不在位」依然会误报。
//
// 最终判据：**只有本次运行真实观测到的退出**（`source: 'memory'`）才上屏。
// 跨重启的历史记录属于诊断信息，去引擎健康/日志页看，不该做成启动横幅 ——
// 一个可能长达几十秒的假告警，比少一条历史提示的代价大得多。
const visible = computed(() =>
  hasEvidence.value
    && !sidecarPresent.value
    && source.value === 'memory',
)

const heading = computed(() =>
  localizedChatErrorMessage(HOST_EXITED, t('chat.errors.hostExitedGeneric'), errorContext.value),
)

const displayTail = computed(() => {
  const raw = stderrTail.value.trim()
  if (!raw) return ''
  return raw.length > DISPLAY_TAIL_LIMIT
    ? `${raw.slice(0, DISPLAY_TAIL_LIMIT)}\n…`
    : raw
})

const facts = computed(() => {
  const rows: Array<{ label: string; value: string }> = []
  if (exitCode.value !== null) {
    rows.push({ label: t('chat.errors.hostExitCodeLabel'), value: String(exitCode.value) })
  }
  if (uptimeMs.value !== null) {
    rows.push({
      label: t('chat.errors.hostExitUptimeLabel'),
      value: errorContext.value.uptime_ms,
    })
  }
  return rows
})

function onRetry(): void {
  if (retrying.value) return
  void restart()
}

const retrying = ref(false)

/**
 * 「重试」必须是**重启宿主进程**，而不是再读一次证据。
 *
 * 真机缺陷：宿主退出时 WebSocket 也断了，旧实现 `refresh()` 在未连接时直接
 * return —— 按钮既无动作也无反馈，用户口径是「按了没有用」。
 */
async function restart(): Promise<void> {
  retrying.value = true
  try {
    const result = await restartDesktopSidecar()
    if (!result.ok && result.reason === 'unavailable') {
      // 浏览器形态没有 Tauri 命令可用：如实说明，不假装已重启。
      pushToast(t('chat.hostExit.restartUnavailable'), { tone: 'warn' })
    } else if (!result.ok) {
      pushToast(t('chat.hostExit.restartFailed', { error: result.error || '—' }), { tone: 'danger' })
      return
    } else {
      pushToast(t(result.action === 'already-running'
        ? 'chat.hostExit.alreadyRunning'
        : 'chat.hostExit.restarted'))
    }
    // 重启后等 WS 自动重连，再读一次证据：卡片按「侧车是否在位」自行消失。
    await rpc.waitForConnection(12_000).catch(() => undefined)
    await refresh()
  } finally {
    retrying.value = false
  }
}

// 挂载时补一次读取：状态监听只在 transport 变更时触发，页面在
// 「已重连但还没读证据」的窗口里挂载会看到一份陈旧 payload。
onMounted(() => { void refresh() })

/**
 * 卡片可见期间每 3s 复读一次证据。
 *
 * 真机时序：网关（Rust）先监听、侧车（Python）约 2 秒后才注册。
 * WebView 在侧车注册前就读了证据 → `sidecar_present:false` → 卡片出现；
 * 之后 transport 不再变化，也就没有第二次读取 —— 侧车明明已经好了，
 * 卡片却永远挂在那里（用户看到的就是这张图）。
 *
 * 只在「可见」时轮询，隐藏即停：正常状态下零额外 RPC。
 */
let evidencePoll: ReturnType<typeof setInterval> | null = null

function stopPolling(): void {
  if (evidencePoll !== null) {
    clearInterval(evidencePoll)
    evidencePoll = null
  }
}

watch(visible, (shown) => {
  if (shown && evidencePoll === null) {
    evidencePoll = setInterval(() => { void refresh() }, 3000)
  } else if (!shown) {
    stopPolling()
  }
}, { immediate: true })

onUnmounted(stopPolling)
</script>

<style scoped>
.host-exit {
  display: flex;
  flex-direction: column;
  gap: var(--sp-2);
}

.host-exit__facts {
  display: grid;
  grid-template-columns: auto 1fr;
  gap: var(--sp-1) var(--sp-3);
  margin: 0;
  padding: var(--sp-3);
  font-size: var(--fs-xs);
  text-align: left;
  background: var(--bg-surface);
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
}

.host-exit__fact-label {
  color: var(--text-muted);
  white-space: nowrap;
}

.host-exit__fact-value {
  margin: 0;
  font-family: var(--font-mono);
  color: var(--text);
}

.host-exit__details {
  font-size: var(--fs-xs);
  text-align: left;
}

.host-exit__summary {
  cursor: pointer;
  color: var(--text-muted);
}

.host-exit__tail {
  max-height: 12rem;
  margin: var(--sp-2) 0 0;
  padding: var(--sp-2);
  overflow: auto;
  font-family: var(--font-mono);
  font-size: var(--fs-xs);
  line-height: 1.5;
  white-space: pre-wrap;
  word-break: break-word;
  background: var(--bg-surface);
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
}
</style>