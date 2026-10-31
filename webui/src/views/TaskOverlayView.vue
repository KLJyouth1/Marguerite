<template>
  <!-- P0-4：任务悬浮球（Tauri 独立透明窗口 120×120）。
       与 /pet-overlay 同构：无外壳、全透明背景，只渲染一个紧凑球体。
       有运行中任务 → 显示数量 + 旋转指示；无任务 → 静态小圆点。
       点击球体 → 经 Tauri 命令 task_overlay_hide 隐藏自己（不弹窗、不暴露连接状态）。 -->
  <div class="task-overlay">
    <div
      class="task-overlay__ball"
      :class="{ 'is-running': running }"
      role="button"
      tabindex="0"
      :title="t('taskOverlay.hint')"
      :aria-label="running ? t('taskOverlay.aria', { count }) : t('taskOverlay.idle')"
      @click="hideOverlay"
      @keydown.enter="hideOverlay"
      @keydown.space.prevent="hideOverlay"
    >
      <span v-if="running" class="task-overlay__count">{{ count }}</span>
      <span v-else class="task-overlay__dot" />
    </div>
  </div>
</template>

<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { useRpcStore } from '@/stores/rpc'
import { SESSION_LIST_VIEW, sessionRunStatus } from '@/composables/useSessions'
import { nextOverlayPollMs } from '@/utils/chat/overlayPoll'
import type { SessionsListResponse } from '@/types/rpc'

const { t } = useI18n()
const rpc = useRpcStore()

// 网关 run-status 词表里的「活动」态（单一事实源：utils/chat/runStatus.ts）。
const ACTIVE_RUN_STATUSES = new Set(['running', 'queued', 'approval_pending'])
const EVENT_DEBOUNCE_MS = 400

const count = ref(0)
const running = computed(() => count.value > 0)

let pollTimer: ReturnType<typeof setTimeout> | null = null
let debounceTimer: ReturnType<typeof setTimeout> | null = null
let unsubs: Array<() => void> = []

/**
 * 兜底轮询自我续期：间隔按「有没有任务在跑 / 连没连上」现算，
 * 空闲时放慢到 60 s 而不是撤掉（理由与数字见 `utils/chat/overlayPoll`）。
 */
function schedulePoll(): void {
  if (pollTimer) clearTimeout(pollTimer)
  pollTimer = setTimeout(() => {
    pollTimer = null
    void refresh().finally(schedulePoll)
  }, nextOverlayPollMs(running.value, rpc.isConnected))
}

/**
 * 统计运行中的任务：只读 `sessions.list`（SUPPORTED_METHODS 已声明），
 * 逐行用会话契约的 `sessionRunStatus` 归一化，避免自造状态词表。
 * 连接失败 / 方法不可达 → 中性空态（0），不弹窗、不显示连接状态。
 */
async function refresh(): Promise<void> {
  try {
    await rpc.waitForConnection(8_000)
    const data = await rpc.call<SessionsListResponse>('sessions.list', {
      limit: 200,
      view: SESSION_LIST_VIEW,
    })
    const rows = data?.sessions ?? data?.keys ?? []
    count.value = rows.reduce((total, row) => {
      if (typeof row === 'string') return total
      return ACTIVE_RUN_STATUSES.has(sessionRunStatus(row)) ? total + 1 : total
    }, 0)
  } catch {
    count.value = 0
  }
}

function scheduleRefresh(): void {
  if (debounceTimer) clearTimeout(debounceTimer)
  debounceTimer = setTimeout(() => {
    debounceTimer = null
    void refresh()
  }, EVENT_DEBOUNCE_MS)
}

/* Tauri API 只在桌面壳里存在；Web 侧未安装 @tauri-apps 包，
   因此沿用 PetOverlayView 的弱探测方式：存在即调用、不存在即跳过。 */
interface TauriInvokeApi {
  core?: { invoke?: (cmd: string, args?: Record<string, unknown>) => Promise<unknown> }
  invoke?: (cmd: string, args?: Record<string, unknown>) => Promise<unknown>
}

function invokeTauri(cmd: string): void {
  const scope = window as unknown as {
    __TAURI__?: TauriInvokeApi
    __TAURI_INTERNALS__?: TauriInvokeApi
  }
  const api = scope.__TAURI__ ?? scope.__TAURI_INTERNALS__
  if (!api) return
  const invoke = api.core?.invoke
    ? api.core.invoke.bind(api.core)
    : api.invoke?.bind(api)
  if (!invoke) return
  void Promise.resolve(invoke(cmd)).catch(() => {})
}

function hideOverlay(): void {
  invokeTauri('task_overlay_hide')
}

onMounted(() => {
  // 悬浮窗背景必须完全透明（网关托管的页面，无外壳）
  const html = document.documentElement
  const body = document.body
  html.style.background = 'transparent'
  body.style.background = 'transparent'
  html.style.overflow = 'hidden'
  body.style.overflow = 'hidden'

  void refresh().finally(schedulePoll)
  unsubs = [
    rpc.on('sessions.changed', scheduleRefresh),
    rpc.on('task.queued', scheduleRefresh),
    rpc.on('task.running', scheduleRefresh),
    rpc.on('task.succeeded', scheduleRefresh),
    rpc.on('task.failed', scheduleRefresh),
    rpc.on('task.cancelled', scheduleRefresh),
    rpc.on('task.timeout', scheduleRefresh),
  ]
})

onUnmounted(() => {
  if (pollTimer) clearTimeout(pollTimer)
  pollTimer = null
  if (debounceTimer) clearTimeout(debounceTimer)
  unsubs.forEach((off) => off())
  unsubs = []
})
</script>

<style scoped>
.task-overlay {
  position: fixed;
  inset: 0;
  display: flex;
  align-items: center;
  justify-content: center;
  background: transparent;
  color: var(--text);
  user-select: none;
  -webkit-user-select: none;
  font-family: var(--font-sans, system-ui);
}

.task-overlay__ball {
  position: relative;
  width: 84px;
  height: 84px;
  border-radius: var(--radius-full);
  display: flex;
  align-items: center;
  justify-content: center;
  background: color-mix(in srgb, var(--bg-surface) 88%, transparent);
  border: 1px solid color-mix(in srgb, var(--text) 12%, transparent);
  box-shadow: var(--shadow-lg);
  cursor: pointer;
  transition: transform var(--dur-fast) var(--ease-out), box-shadow var(--dur-fast) var(--ease-out);
}
.task-overlay__ball:hover {
  transform: scale(1.06);
  box-shadow: var(--shadow-xl);
}
.task-overlay__ball:active {
  transform: scale(0.96);
}
.task-overlay__ball:focus-visible {
  outline: 2px solid color-mix(in srgb, var(--accent) 60%, transparent);
  outline-offset: 3px;
}

/* 运行中：绕球体一圈的旋转指示（仅 transform，不触发布局） */
.task-overlay__ball.is-running::after {
  content: '';
  position: absolute;
  inset: -4px;
  border-radius: var(--radius-full);
  border: 2px solid transparent;
  border-top-color: var(--accent);
  border-right-color: color-mix(in srgb, var(--accent) 40%, transparent);
  animation: task-overlay-spin var(--dur-pulse) linear infinite;
}

.task-overlay__count {
  font-size: 22px;
  font-weight: 700;
  line-height: 1;
  color: var(--accent);
  font-variant-numeric: tabular-nums;
}

/* 空闲：静态小圆点（不转动、不呼吸） */
.task-overlay__dot {
  width: 10px;
  height: 10px;
  border-radius: 50%;
  background: color-mix(in srgb, var(--text-muted) 55%, transparent);
}

@keyframes task-overlay-spin {
  from { transform: rotate(0deg); }
  to { transform: rotate(360deg); }
}

/* 尊重系统「减少动态效果」偏好：停转、去过渡，信息仍完整可读 */
@media (prefers-reduced-motion: reduce) {
  .task-overlay__ball {
    transition: none;
  }
  .task-overlay__ball.is-running::after {
    animation: none;
    border-right-color: var(--accent);
  }
}
</style>
