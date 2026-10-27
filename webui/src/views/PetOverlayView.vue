<template>
  <!-- V2.1-P2：桌宠悬浮窗独立舞台（透明 body + 无壳）。
       背景 100% 透明，无 UI 骨架——桌面级「仅宠物」体验。
       事件：
         · 手柄按下 → Tauri 原生 startDragging（OS 接管，可拖到屏幕任意位置）
         · 舞台拖拽 → 越过 4px 阈值后同样交给原生拖拽，保留双击抚摸
         · 舞台双击 → triggerAction('pet') 抚摸（与 PetView 语义一致）
         · 舞台右键 → 桌宠自有菜单（互动 / 隐藏 / 设置）
       渲染：优先三维（three 动态加载）；WebGL 不可用/构建失败回退 PetSvg（矢量 + 伪 3D 光照）。
       detectPetRuntime 只调特效预算。 -->
  <div
    class="pet-overlay"
    @contextmenu.prevent="onContextMenu"
  >
    <!-- 拖拽手柄：顶部细条，hover 高亮。原生 startDragging（OS 接管），
         光标不会在拖出窗体边界时丢掉事件。 -->
    <div
      class="pet-overlay__handle"
      aria-hidden="true"
      @pointerdown="onHandlePointerDown"
      :title="t('pet.menu.dragHint')"
    />

    <!-- 舞台（透明背景）：优先真三维，无 WebGL 时回退 2.5D 矢量。
         悬停 → 局部聚焦注视（视线/瞳孔幅度放大） -->
    <div
      ref="stage3dEl"
      class="pet-overlay__stage"
      :class="{ 'is-focused': focused, 'is-3d': pet3d }"
      role="button"
      tabindex="0"
      :aria-label="t('petOverlay.stageAria')"
      aria-haspopup="menu"
      :aria-expanded="menuOpen"
      :title="t('pet.menu.dragHint')"
      @pointerdown="onStagePointerDown"
      @keydown="onStageKeydown"
      @dblclick="onPet"
      @pointerenter="onStageEnter"
      @pointerleave="onStageLeave"
    ></div>
    <!-- 桌宠自有右键菜单（互动 / 隐藏 / 设置）。
         Esc 关闭、↑↓ 在菜单项间移动、打开后焦点落到第一项；
         浏览器默认菜单已在 pet 窗口内全局 preventDefault。 -->
    <transition name="fade">
      <div
        v-if="menuOpen"
        ref="menuEl"
        class="pet-overlay__menu"
        role="menu"
        :aria-label="t('pet.menu.title')"
        :style="{ left: menuPos.x + 'px', top: menuPos.y + 'px' }"
        @contextmenu.prevent
        @click.stop
      >
        <div class="pet-overlay__menu-title">{{ t('pet.menu.title') }}</div>
        <button
          type="button"
          class="pet-overlay__menu-item"
          role="menuitem"
          @click="onInteract"
        >✋ {{ t('pet.menu.interact') }}</button>
        <button
          type="button"
          class="pet-overlay__menu-item"
          role="menuitem"
          @click="hideMe"
        >🚪 {{ t('pet.menu.hide') }}</button>
        <!-- 悬浮窗是独立 webview，没有路由也没拿到主窗口句柄：设置入口如实禁用，
             而不是点开一个空白页假装能设置。 -->
        <button
          type="button"
          class="pet-overlay__menu-item"
          role="menuitem"
          disabled
          aria-disabled="true"
          :title="settingsUnavailableHint"
        >⚙ {{ t('pet.menu.settings') }}</button>
      </div>
    </transition>

    <!-- 心情 & 好感度胶囊（左下悬停显示，不抢视觉焦点） -->
    <div class="pet-overlay__capsule" v-if="statusReady">
      <span class="pet-overlay__mood">{{ moodLabel }}</span>
      <span class="pet-overlay__sep" />
      <span class="pet-overlay__affection">♥ {{ affection }}</span>
      <span class="pet-overlay__energy" :style="{ opacity: energy < 20 ? 1 : 0.75 }">⚡ {{ energy }}</span>
    </div>
  </div>
</template>

<script setup lang="ts">
import { ref, computed, onMounted, onUnmounted, watch, nextTick, reactive } from 'vue'
import {
  usePetRpc,
  PET_SKIN_IDS,
  PET_EXPECTED_SKIN_COUNT,
  type PetSkinId,
  type PetSkinEntry,
  type PetStatus,
} from '@/rpc/petContract'
import { createPetRenderer, type PetRenderer } from '@/components/pet/petRenderer'
import { getPetBrain, type BrainFrame } from '@/components/pet/petBrain'
import { moodMap } from '@/components/pet/petContract'
import { getPetModelSpec } from '@/components/pet/petSpec'
import { detectPetRuntime, renderTierOf } from '@/components/pet/detectPetRuntime'
import { setPetOverlayEnabled } from '@/composables/usePetOverlay'
import { useI18n } from 'vue-i18n'

const { t } = useI18n()
const petRpc = usePetRpc()

/* ─── 皮肤目录 + 状态（与 PetView 同源：R6 单一权威来源 = 后端 pet.list/pet.status）─── */
type UiSkin = Omit<PetSkinEntry, 'kind'> & {
  name: string; emoji: string; kind: 'canvas' | 'model' | 'image'; model?: string; accent: string
}
const FALLBACK: Record<PetSkinId | string, { name: string; emoji: string; accent: string; kind: 'beast' | 'humanoid' }> = {
  cat:    { name: '橘猫小工',   emoji: '🐱', accent: '#F59E0B', kind: 'beast' },
  blue:   { name: '雏菊精灵',   emoji: '🧚‍♀️', accent: '#60A5FA', kind: 'humanoid' },
  pink:   { name: '樱花精灵',   emoji: '🌸', accent: '#F472B6', kind: 'humanoid' },
  purple: { name: '薰衣草精灵', emoji: '💜', accent: '#A78BFA', kind: 'humanoid' },
  green:  { name: '四叶草精灵', emoji: '🍀', accent: '#34D399', kind: 'humanoid' },
}
const DEFAULT_FALLBACK_COLOR = '#60A5FA'
const OFFLINE: UiSkin[] = (PET_SKIN_IDS as readonly string[]).map((id) => {
  const f = FALLBACK[id] ?? { name: id, emoji: '✨', accent: DEFAULT_FALLBACK_COLOR, kind: 'humanoid' }
  return { id, label: f.name, name: f.name, emoji: f.emoji, kind: f.kind === 'beast' ? 'canvas' : 'model', model: id, accent: f.accent, accent_hex: f.accent }
})
const skins = ref<UiSkin[]>(OFFLINE)
const currentSkin = ref<UiSkin>(OFFLINE[0])
const statusReady = computed(() => !!status.value)

/* ─── 舞台（优先真三维；WebGL 不可用/构建失败才回退 2.5D 矢量） ─── */
// 2026-10-24：桌面悬浮窗此前固定用 `PetSvg`，而页内桌宠是 3D ——
// 同一个皮肤在悬浮窗里是「扁的」。现在两处都经 `createPetRenderer` 先试三维。
const stage3dEl = ref<HTMLElement | null>(null)
const pet3d = ref(false)
let sceneToken = 0

/* ─── 渲染器（SVG 随容器自适应，无需 resize；三维自带 ResizeObserver） ─── */
let petScene: PetRenderer | null = null

/** 悬停聚焦：光标进入/离开舞台 → 聚焦权重 0/1（局部注视：视线/瞳孔幅度放大） */
const focused = ref(false)
function onStageEnter() {
  focused.value = true
  petScene?.setHover(1)
}
function onStageLeave() {
  focused.value = false
  petScene?.setHover(0)
}

async function initStage(): Promise<void> {
  const token = ++sceneToken
  if (currentSkin.value.kind === 'image') { petScene?.dispose(); petScene = null; pet3d.value = false; return }
  await nextTick()
  const container = stage3dEl.value
  if (!container) return
  petScene?.dispose(); petScene = null
  pet3d.value = false
  const runtime = detectPetRuntime()
  const accent = currentSkin.value.accent.startsWith('#')
    ? parseInt(currentSkin.value.accent.slice(1), 16)
    : 0x60a5fa
  try {
    const spec = getPetModelSpec(currentSkin.value.model ?? currentSkin.value.id)
    const result = await createPetRenderer({
      container,
      accent,
      spec,
      quality: renderTierOf(runtime.quality),
    })
    // 异步建模期间可能已换肤/卸载
    if (token !== sceneToken) { result.renderer.dispose(); return }
    petScene = result.renderer
    pet3d.value = result.kind === '3d'
    if (result.fallbackReason) {
      // eslint-disable-next-line no-console
      console.warn('[PetOverlayView] 三维不可用，已回退 2.5D 矢量:', result.fallbackReason)
    }
    // 重建后补上当前悬停态（重挂载不丢失聚焦）
    petScene.setHover(focused.value ? 1 : 0)
  } catch (err) {
    // createPetRenderer 内部已把三维失败降级成 SVG，走到这里说明连 SVG 都没建起来。
    // 不再静默：留 console 证据，否则舞台空白时无法判断是「还在加载」还是「已失败」。
    // eslint-disable-next-line no-console
    console.error('[PetOverlayView] 渲染器构建失败（舞台将保持空白）:', err)
    petScene = null
    pet3d.value = false
  }
}

/* ─── RPC 加载（15s 轮询） ─── */
const status = ref<PetStatus | null>(null)
const busy = ref(false)
async function loadSkins() {
  try {
    const res = await petRpc.list()
    const mapped: UiSkin[] = res.skins.map((entry) => {
      const fb = FALLBACK[entry.id] ?? { name: entry.label, emoji: '✨', accent: entry.accent_hex, kind: entry.kind }
      return {
        ...entry,
        name: entry.label || fb.name,
        emoji: fb.emoji,
        kind: entry.kind === 'beast' ? 'canvas' : 'model',
        model: entry.id,
        accent: entry.accent_hex,
      }
    })
    skins.value = mapped.length ? mapped : OFFLINE
    const match = mapped.find((s) => s.id === currentSkin.value.id) || mapped[0]
    if (match) currentSkin.value = match
    // 换肤由 watch(currentSkinId) 重建舞台；这里不再显式 initStage ——
    // 渲染器**不等待** pet.list（见 onMounted：先用 OFFLINE 目录画出来）。
    // 断言守卫：与后端皮肤数漂移时不打断用户，但记录（console），便于 R5 漂移检测
    if (res.skins.length !== PET_EXPECTED_SKIN_COUNT) {
      // eslint-disable-next-line no-console
      console.warn(`[PetOverlay R5] pet.list 返回 ${res.skins.length} 款皮肤 ≠ 期望 ${PET_EXPECTED_SKIN_COUNT}`)
    }
  } catch { /* 离线兜底：保留 OFFLINE 列表并尝试用默认 cat 渲染 */ }
}
async function loadStatus() {
  try {
    const s = await petRpc.status()
    status.value = s
    retryStatusLeft = 0
    // 若权威状态切换了皮肤 → 重新取 spec + 重建舞台
    const match = skins.value.find((x) => x.id === s.skin)
    if (match && match.id !== currentSkin.value.id) {
      currentSkin.value = match
      void initStage()
    }
  } catch {
    // 悬浮窗冷启动常在 WebSocket 就绪前：失败后短暂重试（最多 5 次，间隔 1.5s），
    // 连接建立后自动补上权威状态（桌面壳首屏体验；也避免 E2E 竞态）。
    if (retryStatusLeft > 0) {
      retryStatusLeft -= 1
      setTimeout(() => void loadStatus(), 1500)
    }
  }
}
let retryStatusLeft = 5

const currentSkinId = computed(() => currentSkin.value.id)
const energy = computed(() => status.value?.energy ?? 100)
const affection = computed(() => status.value?.affection ?? 0)
const moodLabel = computed(() => {
  const pm = status.value?.mood
  const map: Record<string, string> = { normal: '晒太阳', happy: '开心', sleepy: '打盹', working: '专注', excited: '雀跃', sad: '低落' }
  if (pm && map[pm as string]) return map[pm as string]
  return ''
})

/* ─── 互动 ─── */
async function onPet() {
  // 拖拽结束后的那次 dblclick 是副作用，不是"抚摸"意图。
  if (swallowNextPet) {
    swallowNextPet = false
    return
  }
  if (busy.value) return
  busy.value = true
  brain.petted()
  // 心情动作联动（V2.2-PetAnim）：抚摸 → celebrate/excited → 开心蹦跳
  petScene?.triggerMoodAction('excited')
  try {
    const res = await petRpc.triggerAction('pet')
    if (res.ok && typeof res.energy === 'number' && typeof res.affection === 'number' && res.mood) {
      status.value = { ...(status.value ?? currentAsFallback()), energy: res.energy, affection: res.affection, mood: res.mood } as PetStatus
    } else {
      void loadStatus()
    }
  } finally { busy.value = false }
}
function currentAsFallback(): Record<string, unknown> {
  return { skin: currentSkin.value.id, kind: currentSkin.value.kind === 'canvas' ? 'beast' : 'humanoid', mood: 'normal', energy: 100, affection: 0, running: true, accent_hex: currentSkin.value.accent }
}

/* ─── 拖拽 ───
 * 用户实测「桌宠拖不动 / 拖不出窗体」的根因（两层，缺一不可）：
 *
 * 1) 旧实现只有顶部 16px 手柄能拖，按着宠物本体拖完全不动 —— 命中区域太小；
 * 2) 手柄把 `startDragging()` 包在同步 try/catch 里并直接 return：
 *    `startDragging()` 返回 Promise，权限不足（本窗口不在 capabilities 的
 *    `windows` 名单里 → ACL 拒绝）时是 **reject**，同步 try/catch 接不住；
 *    而函数已经 return，下面的回退实现永远不会执行 → 装包后「怎么拖都不动」。
 * 3) 即便走回退分支（逐帧 invoke `pet_overlay_move`），也没有 pointer capture：
 *    IPC 慢半拍时光标就跑出窗体，pointermove 立刻停止 —— 观感正是
 *    「拖到窗体边上就卡住，出不去」。
 *
 * 现在：手柄按下立即原生拖拽（OS 接管鼠标）；舞台越过 4px 阈值后转原生拖拽
 * （保留双击抚摸）；原生不可用才回退，并补上 setPointerCapture。
 */
const DRAG_THRESHOLD = 4
interface StageDragState {
  mx: number
  my: number
  wx: number
  wy: number
  moved: boolean
  /** 已交给 OS 原生拖拽：后续 pointermove 不再逐帧改窗口 */
  native: boolean
  pointerId: number
}
let stageDrag: StageDragState | null = null
/** 拖拽刚结束时置为真，用来吞掉尾随的 dblclick（否则松手会莫名抚摸一次）。 */
let swallowNextPet = false

/** 原生窗口拖拽。返回是否已接管（Promise reject 时才回退）。 */
function startNativeDrag(ev: PointerEvent): boolean {
  const w = getWindow()
  if (!w?.startDragging) return false
  try {
    const pending = w.startDragging()
    if (pending && typeof (pending as Promise<void>).then === 'function') {
      // 关键：ACL 拒绝是 reject —— 必须 .catch，否则既不报错也不移动
      void (pending as Promise<void>).catch(() => beginFallbackDrag(ev))
    }
    return true
  } catch {
    return false
  }
}

/** 回退实现：没有原生拖拽能力时，逐帧把窗口挪到指针新位置。 */
function beginFallbackDrag(ev: PointerEvent) {
  ev.preventDefault()
  const target = ev.currentTarget as HTMLElement | null
  stageDrag = {
    mx: ev.clientX,
    my: ev.clientY,
    wx: window.screenX,
    wy: window.screenY,
    moved: false,
    native: false,
    pointerId: ev.pointerId,
  }
  try { target?.setPointerCapture?.(ev.pointerId) } catch { /* 捕获失败也能靠 window 级监听 */ }
  window.addEventListener('pointermove', onStagePointerMove, { capture: true })
  window.addEventListener('pointerup', endStageDrag, { once: true, capture: true })
  window.addEventListener('pointercancel', endStageDrag, { once: true, capture: true })
}

function onHandlePointerDown(ev: PointerEvent) {
  if (ev.button !== 0) return
  if (startNativeDrag(ev)) return
  beginFallbackDrag(ev)
}

function onStagePointerDown(ev: PointerEvent) {
  if (ev.button !== 0) return // 右键交给 contextmenu
  stageDrag = {
    mx: ev.clientX,
    my: ev.clientY,
    wx: window.screenX,
    wy: window.screenY,
    moved: false,
    native: false,
    pointerId: ev.pointerId,
  }
  window.addEventListener('pointermove', onStagePointerMove, { capture: true })
  window.addEventListener('pointerup', endStageDrag, { once: true, capture: true })
  window.addEventListener('pointercancel', endStageDrag, { once: true, capture: true })
}

function onStagePointerMove(ev: PointerEvent) {
  if (!stageDrag || stageDrag.native) return
  const dx = ev.clientX - stageDrag.mx
  const dy = ev.clientY - stageDrag.my
  if (!stageDrag.moved) {
    if (Math.hypot(dx, dy) < DRAG_THRESHOLD) return
    stageDrag.moved = true
    // 越过阈值 → 交给 OS 原生拖拽：指针跨出窗体也不会断流
    if (startNativeDrag(ev)) {
      stageDrag.native = true
      return
    }
  }
  // 窗口随指针移动：宠物可以被拖到屏幕任意位置（含拖出原来的窗体范围）。
  moveWindow(stageDrag.wx + dx, stageDrag.wy + dy)
}

function endStageDrag() {
  if (stageDrag?.moved) swallowNextPet = true
  const drag = stageDrag
  stageDrag = null
  window.removeEventListener('pointermove', onStagePointerMove, { capture: true })
  if (!drag) return
  try { stage3dEl.value?.releasePointerCapture?.(drag.pointerId) } catch { /* 已随 pointerup 释放 */ }
}
interface WindowBridge {
  startDragging?: () => Promise<void> | void
  setIgnoreCursorEvents?: (flag: boolean) => Promise<void> | void
  invoke?: (cmd: string, args?: Record<string, unknown>) => Promise<unknown>
}
// Tauri API 仅当实际运行于 Tauri 壳时才存在。Web 侧编译时不安装 @tauri-apps 包（桌面 GUI 用自己的 package.json），
// 所以这里做「存在即调用、不存在即 skip」的弱式探测，避免 vue-tsc 因模块缺失报错（R1 诚实降级）。
// eslint-disable-next-line @typescript-eslint/no-explicit-any
declare const __TAURI__: any
// eslint-disable-next-line @typescript-eslint/no-explicit-any
function getTauriGlobal(): any {
  // Tauri v2/v1 分别暴露 window.__TAURI__ 或 window.__TAURI_INTERNALS__；都不代表错误。
  const w = window as unknown as Record<string, unknown>
  return w.__TAURI__ ?? w.__TAURI_INTERNALS__ ?? undefined
}
function getWindow(): WindowBridge | null {
  const tauri = getTauriGlobal()
  if (!tauri) return null
  try {
    // 优先用 window 模块（若桌面壳打包时已注入全局），否则走 event 驱动的 invoke
    // eslint-disable-next-line @typescript-eslint/no-unsafe-member-access
    const winApi: unknown = tauri.window ?? tauri.getCurrentWindow
    // eslint-disable-next-line @typescript-eslint/no-unsafe-member-access
    const coreApi: unknown = tauri.core ?? tauri.invoke ?? tauri.event
    const getCurrentWindow =
      (typeof winApi === 'object' && winApi && 'getCurrentWindow' in (winApi as object))
        ? (winApi as { getCurrentWindow?: () => { startDragging?: () => Promise<void> | void } }).getCurrentWindow
        : undefined
    const invokeFn: ((cmd: string, args?: Record<string, unknown>) => Promise<unknown>) | undefined =
      typeof coreApi === 'object' && coreApi && 'invoke' in (coreApi as object)
        // eslint-disable-next-line @typescript-eslint/no-explicit-any
        ? (coreApi as any).invoke?.bind(coreApi)
        : (typeof tauri.invoke === 'function' ? tauri.invoke.bind(tauri) : undefined)
    return {
      startDragging: getCurrentWindow
        ? () => { const w = getCurrentWindow(); if (w && typeof w.startDragging === 'function') return w.startDragging() }
        : undefined,
      invoke: invokeFn,
    }
  } catch { return null }
}
function moveWindow(x: number, y: number) {
  const w = getWindow()
  if (!w?.invoke) return
  // 回退路径：失败（旧壳 / ACL）时不要抛未处理拒绝，原生拖拽仍是首选
  void Promise.resolve(w.invoke('pet_overlay_move', { x, y })).catch(() => {})
}

/* ─── 右键菜单（桌宠自有菜单） ─── */
const MENU_W = 208
const MENU_H = 152
const menuOpen = ref(false)
const menuEl = ref<HTMLElement | null>(null)
const menuPos = reactive({ x: 0, y: 0 })
/** 设置入口缺失的如实说明（悬浮窗没有路由，也拿不到主窗口句柄）。 */
const settingsUnavailableHint = computed(
  () => `${t('pet.menu.settings')}：桌宠悬浮窗没有独立设置入口，请在主窗口「设置 → 宠物」中调整`,
)

function openMenuAt(x: number, y: number) {
  // 贴边时把菜单收回可视区（窗体只有 360×420，别让菜单被裁掉）
  menuPos.x = Math.max(6, Math.min(x, window.innerWidth - MENU_W - 6))
  menuPos.y = Math.max(6, Math.min(y, window.innerHeight - MENU_H - 6))
  menuOpen.value = true
  void nextTick(() => {
    menuEl.value?.querySelector<HTMLButtonElement>('button:not([disabled])')?.focus()
  })
}

function onContextMenu(ev: MouseEvent) {
  ev.preventDefault()
  openMenuAt(ev.clientX, ev.clientY)
}

function closeMenu(restoreFocus = false) {
  if (!menuOpen.value) return
  menuOpen.value = false
  if (restoreFocus) stage3dEl.value?.focus()
}

/** 键盘打开菜单：Enter / 空格 / 菜单键 / Shift+F10（与原生右键等价）。 */
function onStageKeydown(ev: KeyboardEvent) {
  const wantsMenu =
    ev.key === 'ContextMenu' ||
    (ev.key === 'F10' && ev.shiftKey) ||
    (ev.key === 'Enter' && !menuOpen.value) ||
    (ev.key === ' ' && !menuOpen.value)
  if (!wantsMenu) return
  ev.preventDefault()
  const rect = stage3dEl.value?.getBoundingClientRect()
  openMenuAt(
    rect ? rect.left + rect.width / 2 : window.innerWidth / 2,
    rect ? rect.top + 24 : 24,
  )
}

/** 菜单内键盘导航：Esc 关闭并回到舞台，↑↓ 循环移动焦点。 */
function onMenuKeydown(ev: KeyboardEvent) {
  if (!menuOpen.value) return
  if (ev.key === 'Escape') {
    ev.preventDefault()
    closeMenu(true)
    return
  }
  if (ev.key !== 'ArrowDown' && ev.key !== 'ArrowUp') return
  const items = Array.from(
    menuEl.value?.querySelectorAll<HTMLButtonElement>('button:not([disabled])') ?? [],
  )
  if (items.length === 0) return
  ev.preventDefault()
  const current = items.indexOf(document.activeElement as HTMLButtonElement)
  const step = ev.key === 'ArrowDown' ? 1 : -1
  items[(current + step + items.length) % items.length]?.focus()
}

function onInteract() {
  closeMenu()
  void onPet()
}

async function hideMe() {
  closeMenu()
  // 复用既有「桌面悬浮窗开关」能力（自带 Web 端诚实降级）
  const res = await setPetOverlayEnabled(false)
  if (res.ok) return
  const w = getWindow()
  if (w?.invoke) {
    try {
      await w.invoke('pet_overlay_hide')
      return
    } catch { /* 落到下面的如实提示 */ }
  }
  alert('当前环境没有桌面壳，无法隐藏悬浮窗；桌面端可通过托盘菜单重新显示。')
}

/** pet 窗口内全局禁用浏览器默认菜单（右键只出桌宠自己的菜单）。 */
function suppressNativeContextMenu(ev: MouseEvent) {
  ev.preventDefault()
}

/* ─── 大脑 + 动画循环（和 PetView 共用同一数学：R6 单一权威 moodMap） ─── */
const brain = getPetBrain()
const brainFrame = ref<BrainFrame>(brain.frame())
let raf = 0
let lastT = performance.now() / 1000
function loop() {
  const t = performance.now() / 1000
  const dt = Math.min(0.1, Math.max(0.001, t - lastT)); lastT = t
  brainFrame.value = brain.tick(dt)
  petScene?.setMood(moodMap[brainFrame.value.mood])
  raf = requestAnimationFrame(loop)
}

let pollTimer: ReturnType<typeof setInterval> | null = null
onMounted(async () => {
  // 页面背景透明：悬浮窗 HTML 由统一网关托管，此处强制 body/html 透明 + 无滚动条
  const html = document.documentElement
  const body = document.body
  html.style.background = 'transparent'
  body.style.background = 'transparent'
  body.style.overflow = 'hidden'
  html.style.overflow = 'hidden'

  // 先画：渲染器不依赖任何 RPC。旧顺序是 `await loadSkins()` 之后才 initStage，
  // 于是 pet.list 慢/握手未就绪时舞台会长时间空白（e2e 实测 15s 等不到渲染器）。
  // OFFLINE 目录已含全部皮肤元数据；权威皮肤返回后由 watch 重建。
  void initStage()
  await loadSkins()
  void loadStatus()
  raf = requestAnimationFrame(loop)
  pollTimer = setInterval(() => void loadStatus(), 15000)

  // 外部点击关闭菜单
  window.addEventListener('mousedown', onOutsideMenu)
  // 菜单键盘：Esc 关闭 / ↑↓ 移动焦点
  window.addEventListener('keydown', onMenuKeydown)
  // 整个 pet 窗口禁用浏览器默认右键菜单
  window.addEventListener('contextmenu', suppressNativeContextMenu)
})
function onOutsideMenu(ev: MouseEvent) {
  if (!menuOpen.value) return
  const target = ev.target as HTMLElement | null
  if (!target?.closest('.pet-overlay__menu')) menuOpen.value = false
}
onUnmounted(() => {
  cancelAnimationFrame(raf)
  if (pollTimer) clearInterval(pollTimer)
  petScene?.dispose(); petScene = null
  window.removeEventListener('mousedown', onOutsideMenu)
  window.removeEventListener('keydown', onMenuKeydown)
  window.removeEventListener('contextmenu', suppressNativeContextMenu)
  window.removeEventListener('pointermove', onStagePointerMove, { capture: true })
})

/* 皮肤切换 → 重建 3D 舞台 */
watch(currentSkinId, () => { void initStage() })
</script>

<style scoped>
.pet-overlay {
  position: fixed; inset: 0; margin: 0; padding: 0;
  width: 100vw; height: 100vh;
  background: transparent; color: var(--text);
  user-select: none; -webkit-user-select: none;
  font-family: var(--font-sans, system-ui);
}

/* 拖拽手柄：顶部细条，微透，hover 高亮 */
.pet-overlay__handle {
  position: absolute; top: 2px; left: 12px; right: 12px; height: 18px;
  background: color-mix(in srgb, var(--accent) 18%, transparent);
  border-radius: 999px; cursor: grab; backdrop-filter: blur(6px);
  opacity: 0.55; transition: opacity var(--dur-fast, 120ms) var(--ease-out);
  border: 1px solid color-mix(in srgb, var(--accent) 30%, transparent);
}
.pet-overlay__handle:hover { opacity: 1; cursor: grabbing; background: color-mix(in srgb, var(--accent) 32%, transparent); }

/* SVG 舞台：透明底色径向渐变聚焦宠物，画布外全透明 */
.pet-overlay__stage {
  position: absolute; left: 0; right: 0; top: 14px; bottom: 14px;
  border-radius: var(--radius-modal); /* radius-allow: SVG 舞台圆形聚焦需要接近圆度，--radius-modal(24px) 近似 32px */
  background:
    radial-gradient(ellipse at 50% 48%, color-mix(in srgb, var(--accent) 14%, color-mix(in srgb, var(--text) 3%, transparent)) 0%, var(--bg-transparent, transparent) 70%);
  overflow: hidden;
  /* 整卡可拖（2026-10-25）：光标本身就是"这里能拖"的唯一可见提示 */
  cursor: grab;
  box-shadow:
    0 20px 60px -20px color-mix(in srgb, var(--accent) 40%, color-mix(in srgb, var(--scrim, var(--text)) 60%, transparent)),
    0 1px 0 0 color-mix(in srgb, var(--accent) 22%, transparent);
}

/* 三维画布只负责渲染：双击抚摸 / 右键菜单 / 拖拽手柄都由外层接收。
   否则画布自己的 pointerdown 会把「抚摸宠物」变成「旋转模型」。 */
.pet-overlay__stage:active {
  cursor: grabbing;
}
/* 键盘可达：Tab 聚焦到舞台时给一条可见焦点环（不抢鼠标态视觉） */
.pet-overlay__stage:focus-visible {
  outline: 2px solid color-mix(in srgb, var(--accent) 60%, transparent);
  outline-offset: -2px;
}

.pet-overlay__stage.is-3d :deep(canvas) {
  pointer-events: none;
}

/* 心情 & 好感度胶囊（左下） */
.pet-overlay__capsule {
  position: absolute; left: 14px; bottom: 18px;
  display: inline-flex; align-items: center; gap: 8px;
  padding: 4px 10px; border-radius: 999px;
  background: color-mix(in srgb, var(--card) 78%, transparent);
  backdrop-filter: blur(6px);
  border: 1px solid color-mix(in srgb, var(--hairline) 70%, transparent);
  font-size: 11.5px; color: var(--text-muted);
  font-variant-numeric: tabular-nums;
}
.pet-overlay__sep { width: 1px; height: 10px; background: var(--hairline); opacity: 0.5; }
.pet-overlay__energy { color: var(--accent); font-weight: 600; }
.pet-overlay__affection { color: var(--danger); font-weight: 600; }

/* 右键菜单 */
.pet-overlay__menu {
  position: absolute; z-index: 10; min-width: 200px;
  padding: 8px; border-radius: var(--radius-lg);
  background: color-mix(in srgb, var(--card) 96%, transparent);
  border: 1px solid var(--hairline);
  backdrop-filter: blur(14px);
  box-shadow: 0 18px 40px -12px color-mix(in srgb, var(--scrim, var(--text)) 60%, transparent);
  font-size: 12.5px;
}
.pet-overlay__menu-title { font-weight: 700; padding: 4px 8px 8px; color: var(--text); font-size: 12px; }
.pet-overlay__menu-item {
  width: 100%; text-align: left; padding: 6px 8px; border-radius: var(--radius-sm);
  border: none; background: transparent; color: var(--text); cursor: pointer;
  transition: background var(--dur-fast) var(--ease-out);
}
.pet-overlay__menu-item:hover { background: color-mix(in srgb, var(--accent) 14%, var(--card)); }
.pet-overlay__menu-item:disabled {
  color: var(--text-dim);
  cursor: not-allowed;
}
.pet-overlay__menu-item:disabled:hover { background: transparent; }
.pet-overlay__menu-sep { height: 1px; margin: 6px 6px; background: var(--hairline); opacity: 0.6; }

/* 过渡 */
.fade-enter-active, .fade-leave-active { transition: opacity var(--dur-fast) var(--ease-out), transform var(--dur-fast) var(--ease-out); }
.fade-enter-from, .fade-leave-to { opacity: 0; transform: translateY(-4px); }
</style>
