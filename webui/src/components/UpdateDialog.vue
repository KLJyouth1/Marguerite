<script setup lang="ts">
import { onBeforeUnmount, onMounted, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { getPlatform } from '@/platform'
import { getTauriInvoke } from '@/utils/tauriBridge'

// 桌面端自动更新弹窗。与 UpdateBanner（被动横幅）互补：
// - 浏览器端：UpdateBanner 仍显示
// - 桌面端（Tauri）：本组件接管，启动后 5 秒自动检查，有更新时弹窗引导
//
// 调用链：check_for_updates → 有更新 → download_and_install_update（监听
// update://download-progress 事件）→ 下载完弹窗 → restart_app

type DialogPhase = 'idle' | 'available' | 'downloading' | 'downloaded' | 'error'

interface UpdateInfo {
  available: boolean
  version?: string
  date?: string | null
  body?: string | null
  error?: string
}

const platform = getPlatform()
const isDesktop = platform.id === 'desktop'
const { t } = useI18n()

const visible = ref(false)
const phase = ref<DialogPhase>('idle')
const latestVersion = ref('')
const releaseDate = ref('')
const releaseNotes = ref('')
const progress = ref(0)
const errorMsg = ref('')
const ignoredVersion = ref<string | null>(null)

const IGNORE_KEY = 'Marguerite-update-ignored-version'

function readIgnored(): string | null {
  try {
    return localStorage.getItem(IGNORE_KEY)
  } catch {
    return null
  }
}

function setIgnored(version: string) {
  ignoredVersion.value = version
  try {
    localStorage.setItem(IGNORE_KEY, version)
  } catch {
    /* private mode — session-local only */
  }
}

function tauriEventListen(eventName: string, cb: (payload: unknown) => void): (() => void) | null {
  if (typeof window === 'undefined') return null
  const w = window as unknown as Record<string, unknown>
  const tauri = (w.__TAURI__ ?? w.__TAURI_INTERNALS__) as Record<string, unknown> | undefined
  if (!tauri) return null
  const eventApi = (tauri.event ?? (tauri.core as Record<string, unknown> | undefined)?.event) as
    | Record<string, unknown>
    | undefined
  if (!eventApi || typeof eventApi.listen !== 'function') return null
  try {
    const unlistenPromise = (eventApi.listen as (
      event: string,
      handler: (e: { payload: unknown }) => void,
    ) => Promise<() => void>)(eventName, (e) => cb(e.payload))
    // 返回一个同步取消函数（异步 unlisten 在组件卸载时调用即可）
    return () => { void unlistenPromise.then((fn) => fn()) }
  } catch {
    return null
  }
}

let unlistenProgress: (() => void) | null = null
let checkTimer: number | null = null

async function doCheck(): Promise<void> {
  const invoke = getTauriInvoke()
  if (!invoke) return
  try {
    const raw = await invoke('check_for_updates') as UpdateInfo
    if (!raw || raw.available !== true) return
    latestVersion.value = raw.version ?? ''
    releaseDate.value = raw.date ?? ''
    releaseNotes.value = raw.body ?? ''
    if (ignoredVersion.value && ignoredVersion.value === latestVersion.value) return
    phase.value = 'available'
    visible.value = true
  } catch {
    // 网络错误或 updater 未配置——静默，不打扰用户
  }
}

async function doDownload(): Promise<void> {
  const invoke = getTauriInvoke()
  if (!invoke) return
  phase.value = 'downloading'
  progress.value = 0

  unlistenProgress = tauriEventListen('update://download-progress', (payload) => {
    const p = payload as { chunk?: number; total?: number | null }
    if (typeof p.total === 'number' && p.total > 0 && typeof p.chunk === 'number') {
      progress.value = Math.min(100, Math.round((p.chunk / p.total) * 100))
    }
  })

  try {
    await invoke('download_and_install_update')
    phase.value = 'downloaded'
  } catch (err) {
    phase.value = 'error'
    errorMsg.value = err instanceof Error ? err.message : String(err)
  } finally {
    unlistenProgress?.()
    unlistenProgress = null
  }
}

async function doRestart(): Promise<void> {
  const invoke = getTauriInvoke()
  if (!invoke) return
  try {
    await invoke('restart_app')
  } catch {
    /* restart 会立即退出进程，到不了这里 */
  }
}

function dismiss(): void {
  visible.value = false
  phase.value = 'idle'
}

function snooze(): void {
  dismiss()
}

function ignoreVersion(): void {
  setIgnored(latestVersion.value)
  dismiss()
}

onMounted(() => {
  if (!isDesktop) return
  ignoredVersion.value = readIgnored()
  // 启动后 5 秒自动检查，避免 splash 刚结束就弹窗抢注意力
  checkTimer = window.setTimeout(() => void doCheck(), 5000)
})

onBeforeUnmount(() => {
  if (checkTimer !== null) {
    window.clearTimeout(checkTimer)
    checkTimer = null
  }
  unlistenProgress?.()
  unlistenProgress = null
})
</script>

<template>
  <Teleport to="body">
    <div v-if="visible" class="update-dialog-overlay" @click.self="dismiss">
      <div class="update-dialog" role="dialog" aria-modal="true" aria-labelledby="update-dialog-title">
        <!-- 有更新：展示版本信息 -->
        <template v-if="phase === 'available'">
          <p class="update-dialog__eyebrow">{{ t('updates.dialog.newVersion') }}</p>
          <h2 class="update-dialog__version" id="update-dialog-title">v{{ latestVersion }}</h2>
          <p v-if="releaseDate" class="update-dialog__date">{{ releaseDate }}</p>
          <div v-if="releaseNotes" class="update-dialog__notes">{{ releaseNotes }}</div>
          <footer class="update-dialog__actions">
            <button type="button" class="update-dialog__btn update-dialog__btn--ghost" @click="ignoreVersion">
              {{ t('updates.dialog.skipVersion') }}
            </button>
            <button type="button" class="update-dialog__btn update-dialog__btn--ghost" @click="snooze">
              {{ t('updates.dialog.later') }}
            </button>
            <button type="button" class="update-dialog__btn update-dialog__btn--primary" @click="doDownload">
              {{ t('updates.dialog.updateNow') }}
            </button>
          </footer>
        </template>

        <!-- 下载中：进度条 -->
        <template v-else-if="phase === 'downloading'">
          <h2 class="update-dialog__version">{{ t('updates.dialog.downloading') }}</h2>
          <p class="update-dialog__date">v{{ latestVersion }}</p>
          <div class="update-dialog__progress-track">
            <div class="update-dialog__progress-fill" :style="{ width: progress + '%' }"></div>
          </div>
          <p class="update-dialog__progress-label">{{ progress }}%</p>
        </template>

        <!-- 下载完成：提示重启 -->
        <template v-else-if="phase === 'downloaded'">
          <h2 class="update-dialog__version">{{ t('updates.dialog.downloadedTitle') }}</h2>
          <p class="update-dialog__body">{{ t('updates.dialog.downloadedDesc', { version: latestVersion }) }}</p>
          <footer class="update-dialog__actions">
            <button type="button" class="update-dialog__btn update-dialog__btn--ghost" @click="dismiss">
              {{ t('updates.dialog.restartLater') }}
            </button>
            <button type="button" class="update-dialog__btn update-dialog__btn--primary" @click="doRestart">
              {{ t('updates.dialog.restartNow') }}
            </button>
          </footer>
        </template>

        <!-- 错误 -->
        <template v-else-if="phase === 'error'">
          <h2 class="update-dialog__version">{{ t('updates.dialog.failedTitle') }}</h2>
          <p class="update-dialog__body">{{ errorMsg }}</p>
          <footer class="update-dialog__actions">
            <button type="button" class="update-dialog__btn update-dialog__btn--primary" @click="dismiss">
              {{ t('updates.dialog.close') }}
            </button>
          </footer>
        </template>
      </div>
    </div>
  </Teleport>
</template>

<style scoped>
.update-dialog-overlay {
  position: fixed;
  inset: 0;
  z-index: 1200;
  display: grid;
  place-items: center;
  background: var(--scrim);
  backdrop-filter: blur(8px);
}

.update-dialog {
  width: min(92vw, 440px);
  padding: var(--glass-gap);
  border-radius: var(--glass-radius);
  border: 1px solid color-mix(in srgb, var(--glass-edge) transparent);
  background: color-mix(in srgb, var(--bg-elevated) 60%, transparent);
  backdrop-filter: blur(var(--glass-blur)) saturate(var(--glass-saturate));
  -webkit-backdrop-filter: blur(var(--glass-blur)) saturate(var(--glass-saturate));
  box-shadow: var(--shadow-md);
  color: var(--text);
}

.update-dialog__eyebrow {
  margin: 0 0 4px;
  font-size: var(--fs-xs);
  font-weight: 600;
  text-transform: uppercase;
  letter-spacing: 0.06em;
  color: var(--accent-secondary);
}

.update-dialog__version {
  margin: 0 0 4px;
  font-size: var(--fs-xl);
  font-weight: 700;
}

.update-dialog__date {
  margin: 0 0 var(--sp-3);
  font-size: var(--fs-sm);
  color: var(--text-muted);
}

.update-dialog__body {
  margin: 0 0 var(--sp-3);
  font-size: var(--fs-sm);
  line-height: 1.6;
  color: var(--text);
}

.update-dialog__notes {
  margin: 0 0 var(--sp-4);
  max-height: 200px;
  overflow-y: auto;
  padding: var(--sp-3);
  border-radius: var(--radius-md);
  background: color-mix(in srgb, var(--bg-surface) 60%, transparent);
  font-size: var(--fs-sm);
  line-height: 1.6;
  white-space: pre-wrap;
  color: var(--text-muted);
}

.update-dialog__actions {
  display: flex;
  justify-content: flex-end;
  gap: var(--sp-2);
  margin-top: var(--sp-4);
}

.update-dialog__btn {
  padding: var(--sp-2) var(--sp-4);
  border-radius: var(--radius-control);
  border: 1px solid transparent;
  font-size: var(--fs-sm);
  font-weight: 600;
  cursor: pointer;
  transition: opacity var(--dur-fast) var(--ease-standard);
}

.update-dialog__btn--primary {
  background: var(--accent);
  color: var(--accent-foreground);
}

.update-dialog__btn--primary:hover {
  opacity: 0.9;
}

.update-dialog__btn--ghost {
  background: transparent;
  border-color: color-mix(in srgb, var(--border) 60%, transparent);
  color: var(--text);
}

.update-dialog__btn--ghost:hover {
  background: var(--bg-hover);
}

.update-dialog__progress-track {
  margin: var(--sp-3) 0 var(--sp-2);
  height: 8px;
  border-radius: 999px;
  background: color-mix(in srgb, var(--bg-surface) 80%, transparent);
  overflow: hidden;
}

.update-dialog__progress-fill {
  height: 100%;
  border-radius: 999px;
  background: var(--accent);
  transition: width var(--dur-base) var(--ease-out);
}

.update-dialog__progress-label {
  margin: 0;
  text-align: center;
  font-size: var(--fs-sm);
  color: var(--text-muted);
}
</style>
