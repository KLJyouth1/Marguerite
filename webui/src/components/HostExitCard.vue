<template>
  <!-- S5 host-exit evidence banner.

       形态：**内容区顶部的细横幅**，不是右下角常驻大卡片。
       真机事故（2026-10-28 视觉审计 18 张截图全部中招）：旧实现用 `ErrorState`
       的居中卡片（padding sp-6 + 重试按钮）钉在右下角，实际尺寸 ~400×180，
       压住缓存面板最后一行、工具页「校验全部」按钮和桌宠。错误指示器不该
       遮挡主内容，也不该用大色块占据阅读区。

       现在的形状：单行标题 + [详情]（默认折叠，展开才出现退出码/stderr）+
       [关闭]（本次运行内不再打扰）。`stderr_tail` 仍然绝不进标题行——它可能
       含路径/URL/用户名（与 `core/src/model/error_taxonomy.rs` 同一纪律）。 -->
  <div v-if="visible" class="host-exit" role="status" data-testid="host-exit-banner">
    <span class="host-exit__icon" aria-hidden="true"><Icon name="info" :size="14" /></span>
    <p class="host-exit__text">{{ bannerText }}</p>
    <button
      v-if="hasDetails"
      type="button"
      class="host-exit__action"
      :aria-expanded="detailsOpen"
      data-testid="host-exit-details"
      @click="detailsOpen = !detailsOpen"
    >{{ detailsOpen ? t('chat.hostExit.hideDetails') : t('chat.hostExit.details') }}</button>
    <button
      v-if="canRestart"
      type="button"
      class="host-exit__action host-exit__action--restart"
      :disabled="retrying"
      data-testid="host-exit-restart"
      @click="onRetry"
    >{{ t('chat.hostExit.restart') }}</button>
    <button
      type="button"
      class="host-exit__dismiss"
      :aria-label="t('chat.hostExit.dismiss')"
      data-testid="host-exit-dismiss"
      @click="dismiss"
    ><Icon name="x" :size="13" /></button>

    <div v-if="detailsOpen && hasDetails" class="host-exit__panel">
      <dl v-if="facts.length" class="host-exit__facts">
        <template v-for="fact in facts" :key="fact.label">
          <dt class="host-exit__fact-label">{{ fact.label }}</dt>
          <dd class="host-exit__fact-value">{{ fact.value }}</dd>
        </template>
      </dl>
      <pre v-if="displayTail" class="host-exit__tail">{{ displayTail }}</pre>
    </div>
  </div>
</template>

<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import Icon from '@/components/Icon.vue'
import { useHostExitEvidence } from '@/composables/useHostExitEvidence'
import { useToasts } from '@/composables/useToasts'
import { useRpcStore } from '@/stores/rpc'
import { restartDesktopSidecar } from '@/utils/desktopSidecar'
import { getTauriInvoke } from '@/utils/tauriBridge'

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
    && source.value === 'memory'
    && !dismissed.value,
)

/** 本次运行内用户手动关掉后不再打扰（侧车恢复/再次退出会重新武装）。 */
const dismissed = ref(false)
const detailsOpen = ref(false)

function dismiss(): void {
  dismissed.value = true
}

// 侧车一旦恢复就重新武装关闭状态：下一次真退出仍要能被看见。
watch(sidecarPresent, (present) => {
  if (present) {
    dismissed.value = false
    detailsOpen.value = false
  }
})

/** 细横幅只放一句人话：诊断信息（退出码/stderr）收进「详情」。 */
const bannerText = computed(() => t('chat.hostExit.bannerTitle'))

/** 有退出码或 stderr 才给「详情」入口，避免出现点了什么都没有的按钮。 */
const hasDetails = computed(() => facts.value.length > 0 || displayTail.value.length > 0)

/**
 * 「重启辅助引擎」只在桌面形态出现：浏览器形态没有 Tauri 命令，按钮点了只会弹
 * 一条「当前形态无法重启」。真机上这个动作确实能救回侧车，因此保留为次按钮
 * （不是旧卡片里那个主色大按钮）。
 */
const canRestart = computed(() => getTauriInvoke() !== null)

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
/* 单行细横幅：内容区顶部、可换行但不占大色块。旧实现是右下角 ErrorState 卡片
   （sp-6 内边距 + 主色重试按钮，实测 ~400×180），18 张截图全部压住主内容。 */
.host-exit {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: var(--sp-2);
  margin: 0;
  padding: 8px var(--sp-3);
  font-size: var(--fs-xs);
  color: var(--text);
  background: color-mix(in srgb, var(--warn) 10%, var(--bg-elevated));
  border: 1px solid color-mix(in srgb, var(--warn) 32%, var(--border));
  border-radius: var(--radius-sm);
}

.host-exit__icon {
  display: inline-flex;
  flex-shrink: 0;
  color: var(--warn);
}

.host-exit__text {
  flex: 1 1 auto;
  min-width: 0;
  margin: 0;
  line-height: 1.45;
  overflow-wrap: anywhere;
}

.host-exit__action,
.host-exit__dismiss {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  flex-shrink: 0;
  padding: 2px var(--sp-2);
  border: 1px solid transparent;
  border-radius: var(--radius-sm);
  background: transparent;
  color: var(--text-muted);
  font-size: var(--fs-xs);
  cursor: pointer;
  transition: var(--transition);
}

.host-exit__action:hover,
.host-exit__dismiss:hover {
  color: var(--text);
  background: var(--bg-hover);
}

.host-exit__action:disabled {
  opacity: 0.55;
  cursor: progress;
}

.host-exit__action:focus-visible,
.host-exit__dismiss:focus-visible {
  outline: none;
  box-shadow: var(--focus-ring);
}

.host-exit__dismiss {
  padding: 2px;
}

/* 详情：占满整行，独立成块，展开也不影响横幅的“细”。 */
.host-exit__panel {
  flex-basis: 100%;
  display: flex;
  flex-direction: column;
  gap: var(--sp-2);
}

.host-exit__facts {
  display: grid;
  grid-template-columns: auto 1fr;
  gap: var(--sp-1) var(--sp-3);
  margin: 0;
  padding: var(--sp-2) var(--sp-3);
  font-size: var(--fs-xs);
  text-align: left;
  background: var(--bg-surface);
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
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

.host-exit__tail {
  max-height: 12rem;
  margin: 0;
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