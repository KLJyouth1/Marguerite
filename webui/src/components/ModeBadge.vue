<!-- 顶栏全局模式徽标（3×3 用户模式收敛，P0-2）。
     单一事实源 = 3×3 用户模式格位（activeUserMode，来自网关 mode.get.userMode /
     mode.changed 广播）。这消除了过去两条并存的模式轴：
       · 旧轴：顶栏徽标的五个运行时 kind（plan/agent/ensemble/bloom/omni-transcend）
       · 新轴：输入框上方 3×3 网格的九档用户模式（强度 × 工作态）
     徽标曾自带一份 MODE_KIND_LIST、只渲染 kind 别名，于是同一页面上「顶栏写 Agent、
     网格高亮标准·方案」互相矛盾；切换一次还会在两条轴之间来回改写。

     现在的约定：
       · 徽标**只渲染** activeUserMode；kind 仅作**兼容读取**——载荷里没有 userMode
         时用 userModeKeyForKind(kind) 反推格位（老网关/老载荷也能显示正确的九档标签）。
       · 徽标菜单与输入框的 ModeMatrix **同源**：同样九格、同样 switchUserMode(key)
         下发 { userMode }，由网关 from_user_mode 派生真正的执行档。
       · 右键轮换保留历史「五站」手感，但落点一律换算成九档格位再下发，
         不再引入第二个写入路径（见 cycleMode 注释）。
     切换按钮统一走 switchUserMode（乐观前先确认：失败保留旧值并显式报错）。 -->
<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from 'vue'
import Icon from '@/components/Icon.vue'
import { useRouter } from 'vue-router'
import { useActiveMode } from '@/composables/useActiveMode'
import {
  MODE_KINDS,
  normalizeModeKind,
  USER_MODE_INTENSITY,
  USER_MODE_META,
  USER_MODE_WORKING,
  userModeKeyForKind,
  type UserModeInfo,
} from '@/stores/activeMode'
import { userModeHint, userModeLabel } from '@/utils/userModeText'
import { t } from '@/i18n'

const router = useRouter()
const { activeMode, activeUserMode, switchError, switching, switchUserMode } = useActiveMode()

/**
 * Legacy runtime kind, kept ONLY as a compatibility read.
 *
 * The gateway payload carries both `kind` (execution tier, Rust `ModeKind`) and
 * `userMode` (the 3×3 slot). `kind` is what `from_user_mode` derived, so it is the
 * honest answer to "what is actually running" — but it is never written from here.
 */
const compatKind = computed(() =>
  normalizeModeKind(activeMode.value?.kind ?? activeMode.value?.mode),
)

/**
 * The 3×3 slot to display. `activeUserMode` wins; a payload without it falls back
 * to the canonical reverse map so an older gateway still lands on the right cell
 * instead of showing a dash.
 */
const currentKey = computed(() => {
  if (activeUserMode.value?.key) return activeUserMode.value.key
  return compatKind.value ? userModeKeyForKind(compatKind.value) : ''
})

const currentInfo = computed<UserModeInfo | null>(() => {
  const meta = USER_MODE_META[currentKey.value] as UserModeInfo | undefined
  return meta ?? null
})

const currentLabel = computed(() => (currentInfo.value ? userModeLabel(currentInfo.value.key) : '—'))

// 切换入口的「在哪切」说明只作为 tooltip 存在：它此前由 ModeHintBar 常驻渲染，
// 于是每个会话页面上都挂着一句 11px 灰字提示（真机实测），属于噪声。
const badgeTitle = computed(() => {
  if (!currentInfo.value) return String(t('chrome.modeMenu'))
  const parts = [String(t('chrome.modeBadge', { mode: currentLabel.value })), currentInfo.value.key]
  // 技术名（枚举值）保留在 tooltip 里，方便高级用户与网关日志核对。
  if (compatKind.value) parts.push(compatKind.value)
  parts.push(String(t('modeHintBar.gotoTopbarBadge')))
  return parts.join(' · ')
})

const open = ref(false)

// Honest switch feedback: a failed `mode.switch` keeps the old badge and shows
// this error instead of pretending the switch happened.
const failed = computed(() => Boolean(switchError.value))
const failureText = computed(() => String(t('chrome.modeSwitchFailed')))
const busy = computed(() => switching.value === true)

/** 九个格位，行主序（强度 × 工作态），与输入框 ModeMatrix 的顺序一致。 */
function cells(): UserModeInfo[] {
  const out: UserModeInfo[] = []
  for (const i of USER_MODE_INTENSITY) {
    for (const w of USER_MODE_WORKING) {
      const meta = USER_MODE_META[`${i}_${w}`] as UserModeInfo | undefined
      if (meta) out.push(meta)
    }
  }
  return out
}

function toggleMenu() {
  open.value = !open.value
}

function closeMenu() {
  open.value = false
}

async function pick(key: string) {
  closeMenu()
  if (!key || key === currentKey.value) return
  await switchUserMode(key)
}

function goControlCenter() {
  closeMenu()
  void router.push('/control-center')
}

/**
 * 旧交互兼容：右键快速轮换。
 *
 * 保留历史「五站」顺序（plan/agent/ensemble/bloom/omni-transcend 各自的格位），
 * 但**只按九档键下发**：轮换是最后一个还认识旧 kind 列表的入口，把 kind 换算成
 * userMode 再走 switchUserMode，就不会出现「右键切了一次、网格高亮没动」。
 */
function cycleMode() {
  const order = MODE_KINDS.map((k) => userModeKeyForKind(k))
  const idx = order.indexOf(currentKey.value)
  const next = order[(idx + 1) % order.length]
  void switchUserMode(next)
}

function onDocClick(e: MouseEvent) {
  if (!open.value) return
  const host = (e.target as HTMLElement)?.closest?.('.mode-badge')
  if (!host) closeMenu()
}

function onKeydown(e: KeyboardEvent) {
  if (e.key === 'Escape') closeMenu()
}

onMounted(() => {
  document.addEventListener('click', onDocClick)
  document.addEventListener('keydown', onKeydown)
})
onUnmounted(() => {
  document.removeEventListener('click', onDocClick)
  document.removeEventListener('keydown', onKeydown)
})
</script>

<template>
  <div class="mode-badge-wrap">
    <button
      class="mode-badge"
      :class="{ 'mode-badge--busy': busy, 'mode-badge--error': failed }"
      type="button"
      :title="badgeTitle"
      :aria-label="badgeTitle"
      :aria-expanded="open"
      :aria-busy="busy"
      @click="toggleMenu"
      @contextmenu.prevent="cycleMode"
    >
      <span
        class="mode-badge__dot"
        :class="currentInfo ? `mode-badge__dot--${currentInfo.working}` : 'mode-badge__dot--unknown'"
      />
      <Icon name="mode" :size="14" />
      <span class="mode-badge__label">{{ currentLabel }}</span>
      <Icon name="chevronDown" :size="12" class="mode-badge__caret" />
    </button>

    <p v-if="failed" class="mode-badge__error" role="alert" :title="failureText">
      <Icon name="alertTriangle" :size="12" aria-hidden="true" />
      <span class="mode-badge__error-text">{{ failureText }}</span>
    </p>

    <Transition name="mode-menu">
      <div v-if="open" class="mode-menu" role="menu" :aria-label="t('chrome.modeMenu')">
        <div class="mode-menu__head">
          <span class="mode-menu__title">{{ t('chrome.modeMenuTitle') }}</span>
          <span class="mode-menu__current">{{ t('chrome.modeMenuCurrent', { mode: currentLabel }) }}</span>
        </div>
        <div class="mode-menu__grid" role="group" :aria-label="t('chrome.modeMenuTitle')">
          <button
            v-for="cell in cells()"
            :key="cell.key"
            type="button"
            class="mode-menu__item"
            :class="{ 'mode-menu__item--active': cell.key === currentKey }"
            role="menuitemradio"
            :aria-checked="cell.key === currentKey"
            :title="`${cell.key} · ${userModeHint(cell.key)}`"
            @click="pick(cell.key)"
          >
            <span class="mode-menu__item-dot" :class="`mode-badge__dot--${cell.working}`" />
            <span class="mode-menu__item-main">
              <span class="mode-menu__item-label">{{ userModeLabel(cell.key) }}</span>
              <span class="mode-menu__item-hint">{{ userModeHint(cell.key) }}</span>
            </span>
          </button>
        </div>
        <button type="button" class="mode-menu__foot" role="menuitem" @click="goControlCenter">
          <Icon name="settings" :size="13" />
          <span>{{ t('chrome.modeMenuOpenHub') }}</span>
        </button>
      </div>
    </Transition>
  </div>
</template>

<style scoped>
.mode-badge-wrap {
  position: relative;
  display: inline-flex;
}
.mode-badge {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  height: 30px;
  padding: 0 10px;
  border-radius: 999px;
  border: 1px solid var(--border);
  background: var(--bg-elevated);
  color: var(--text);
  font-size: 12.5px;
  font-weight: 600;
  cursor: pointer;
  transition: background var(--dur-fast) var(--ease-standard), border-color var(--dur-fast) var(--ease-standard);
}
.mode-badge:hover {
  border-color: var(--accent);
}
.mode-badge__caret {
  opacity: 0.6;
}
.mode-badge__dot {
  width: 8px;
  height: 8px;
  border-radius: 50%;
  background: var(--text-dim);
  flex: none;
}
/* 颜色跟随「工作态」三列（与输入框 ModeMatrix 的列语义一致），强度档由标签文字承载。 */
.mode-badge__dot--ask { background: var(--info); }
.mode-badge__dot--plan { background: var(--accent); }
.mode-badge__dot--doit { background: var(--warn); }
.mode-badge__label { white-space: nowrap; }
.mode-badge--busy { opacity: 0.75; }
.mode-badge--error { border-color: var(--danger); }
.mode-badge__error {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  margin: 0 0 0 6px;
  max-width: 220px;
  color: var(--danger);
  font-size: 11px;
  line-height: 1.2;
}
.mode-badge__error-text {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.mode-menu {
  position: absolute;
  top: calc(100% + 6px);
  right: 0;
  min-width: 300px;
  background: var(--bg-surface);
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  box-shadow: 0 10px 32px color-mix(in srgb, var(--text) 18%, transparent);
  padding: 6px;
  z-index: 960;
  display: flex;
  flex-direction: column;
  gap: 2px;
}
.mode-menu__head {
  display: flex;
  align-items: baseline;
  justify-content: space-between;
  gap: 8px;
  padding: 4px 8px 6px;
  border-bottom: 1px solid var(--border);
  margin-bottom: 4px;
}
.mode-menu__title { font-size: 12px; font-weight: 700; }
.mode-menu__current { font-size: 11px; color: var(--text-dim); }
.mode-menu__grid {
  display: grid;
  grid-template-columns: repeat(3, minmax(0, 1fr));
  gap: 2px;
}
.mode-menu__item {
  display: flex;
  align-items: flex-start;
  gap: 6px;
  padding: 7px 8px;
  border: none;
  border-radius: var(--radius-sm);
  background: transparent;
  color: var(--text);
  text-align: left;
  cursor: pointer;
  font: inherit;
}
.mode-menu__item:hover {
  background: color-mix(in srgb, var(--accent) 14%, transparent);
}
.mode-menu__item--active {
  background: color-mix(in srgb, var(--accent) 20%, transparent);
}
.mode-menu__item-dot {
  width: 8px;
  height: 8px;
  border-radius: 50%;
  margin-top: 4px;
  flex: none;
}
.mode-menu__item-main { display: flex; flex-direction: column; gap: 1px; min-width: 0; }
.mode-menu__item-label { font-size: 12.5px; font-weight: 600; white-space: nowrap; }
.mode-menu__item-hint {
  font-size: 10.5px;
  color: var(--text-dim);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.mode-menu__foot {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 8px;
  border: none;
  border-top: 1px solid var(--border);
  border-radius: 0 0 var(--radius-sm) var(--radius-sm);
  background: transparent;
  color: var(--text-dim);
  font: inherit;
  font-size: 12px;
  cursor: pointer;
  margin-top: 4px;
}
.mode-menu__foot:hover {
  color: var(--accent);
}

.mode-menu-enter-active,
.mode-menu-leave-active {
  transition: opacity var(--dur-fast) var(--ease-standard), transform var(--dur-fast) var(--ease-standard);
  transform-origin: top right;
}
.mode-menu-enter-from,
.mode-menu-leave-to {
  opacity: 0;
  transform: scale(0.96) translateY(-4px);
}
</style>
