<template>
  <!-- V2.1-P2：桌宠悬浮窗独立舞台（透明 body + 无壳）。
       背景 100% 透明，无 UI 骨架——桌面级「仅宠物」体验。
       事件：
         · 顶栏 drag-handle 拖拽 → 上报 Tauri：pet_overlay_move（经 ipc 桥）
         · 舞台双击 → triggerAction('pet') 抚摸（与 PetView 语义一致）
         · 舞台右键 → 显示简化菜单（换肤/隐藏/置顶信息）
       渲染：PetSvg（SVG 矢量 + 伪 3D 光照）+ detectPetRuntime（档位只调特效预算）。 -->
  <div
    class="pet-overlay"
    @contextmenu.prevent="onContextMenu"
  >
    <!-- 拖拽手柄：16px 高顶部细条，半透明淡蓝，hover 时高亮提示可拖动 -->
    <div
      class="pet-overlay__handle"
      @mousedown="startDrag"
      title="拖拽移动桌宠 · 双击抚摸 · 右键菜单"
    />

    <!-- SVG 舞台（透明背景）；SVG 无 WebGL 依赖，任何环境都完整渲染。
         悬停 → 局部聚焦注视（视线/瞳孔幅度放大） -->
    <div
      ref="stage3dEl"
      class="pet-overlay__stage"
      :class="{ 'is-focused': focused }"
      role="img"
      aria-label="Marguerite 桌面桌宠"
      @dblclick="onPet"
      @pointerenter="onStageEnter"
      @pointerleave="onStageLeave"
    ></div>
    <!-- 右键菜单（Tauri Window 级菜单一部分；此处保留 DOM 菜单，供 Web 用户态测试） -->
    <transition name="fade">
      <div
        v-if="menuOpen"
        class="pet-overlay__menu"
        :style="{ left: menuPos.x + 'px', top: menuPos.y + 'px' }"
        @click.stop
      >
        <div class="pet-overlay__menu-title">桌宠菜单</div>
        <button class="pet-overlay__menu-item" @click="onPet">✋ 顺毛抚摸</button>
        <button class="pet-overlay__menu-item" @click="onFeed">🐟 投喂猫条</button>
        <button class="pet-overlay__menu-item" @click="openPetView">🖥 打开设置面板</button>
        <div class="pet-overlay__menu-sep" />
        <div class="pet-overlay__menu-sub">切换皮肤</div>
        <div class="pet-overlay__menu-skins">
          <button
            v-for="s in skinButtons"
            :key="s.id"
            class="pet-overlay__skin"
            :class="{ 'is-active': s.id === currentSkinId }"
            :style="previewSkinStyle(s)"
            :title="s.name"
            @click="selectSkin(s.id)"
          >
            {{ s.emoji }}
          </button>
        </div>
        <div class="pet-overlay__menu-sep" />
        <button class="pet-overlay__menu-item is-danger" @click="hideMe">🚪 隐藏悬浮窗</button>
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
import { ref, computed, onMounted, onUnmounted, watch, nextTick, reactive, type CSSProperties } from 'vue'
import {
  usePetRpc,
  PET_SKIN_IDS,
  PET_EXPECTED_SKIN_COUNT,
  type PetSkinId,
  type PetSkinEntry,
  type PetStatus,
} from '@/rpc/petContract'
import { PetSvg } from '@/components/pet/petSvg'
import { getPetBrain, type BrainFrame } from '@/components/pet/petBrain'
import { moodMap } from '@/components/pet/petContract'
import { getPetModelSpec } from '@/components/pet/petSpec'
import { detectPetRuntime, renderTierOf } from '@/components/pet/detectPetRuntime'

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

/* ─── 舞台（SVG 渲染器；无 WebGL 依赖，任何环境都完整渲染） ─── */
const stage3dEl = ref<HTMLElement | null>(null)
function previewSkinStyle(s: UiSkin): CSSProperties {
  const a = s.accent.startsWith('#') ? s.accent : '#60A5FA'
  return { background: `radial-gradient(circle at 50% 40%, color-mix(in srgb, ${a} 34%, transparent), transparent 76%)` }
}

/* ─── 矢量渲染器（SVG 随容器自适应，无需 resize） ─── */
let petScene: PetSvg | null = null

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

function initStage() {
  if (currentSkin.value.kind === 'image') { petScene?.dispose(); petScene = null; return }
  if (!stage3dEl.value) return
  petScene?.dispose(); petScene = null
  const runtime = detectPetRuntime()
  const accent = currentSkin.value.accent.startsWith('#')
    ? parseInt(currentSkin.value.accent.slice(1), 16)
    : 0x60a5fa
  try {
    const spec = getPetModelSpec(currentSkin.value.model ?? currentSkin.value.id)
    petScene = new PetSvg({ container: stage3dEl.value, accent, spec, quality: renderTierOf(runtime.quality) })
    // 重建后补上当前悬停态（重挂载不丢失聚焦）
    petScene.setHover(focused.value ? 1 : 0)
  } catch {
    // 构建期异常：SVG 无 WebGL 依赖，不存在「精简模式」降级分支
    petScene = null
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
    if (!match && mapped.length > 0) currentSkin.value = mapped[0]
    void nextTick(initStage)
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
      void nextTick(initStage)
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
const skinButtons = computed(() => skins.value)

/* ─── 互动 ─── */
async function onPet() {
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
async function onFeed() {
  if (busy.value) return
  busy.value = true
  brain.feed('done')
  petScene?.triggerMoodAction('excited')
  try {
    const res = await petRpc.triggerAction('feed')
    if (res.ok && typeof res.energy === 'number' && typeof res.affection === 'number' && res.mood) {
      status.value = { ...(status.value ?? currentAsFallback()), energy: res.energy, affection: res.affection, mood: res.mood } as PetStatus
    } else {
      void loadStatus()
    }
  } finally { busy.value = false }
}
async function selectSkin(id: PetSkinId | string) {
  if (busy.value) return
  busy.value = true
  try {
    const res = await petRpc.setSkin(id)
    if (res.ok) {
      const match = skins.value.find((s) => s.id === res.skin)
      if (match) { currentSkin.value = match; void nextTick(initStage) }
    }
  } finally { busy.value = false; menuOpen.value = false }
}
function currentAsFallback(): Record<string, unknown> {
  return { skin: currentSkin.value.id, kind: currentSkin.value.kind === 'canvas' ? 'beast' : 'humanoid', mood: 'normal', energy: 100, affection: 0, running: true, accent_hex: currentSkin.value.accent }
}

/* ─── 拖拽（浏览器原生鼠标事件 + Tauri 拖拽移动：pet_overlay_move） ─── */
let dragStart: { mx: number; my: number; wx: number; wy: number } | null = null
function startDrag(ev: MouseEvent) {
  // 走 Tauri Window.startDragging() 若可用（桌面端最顺滑）；否则回退自实现
  const w = getWindow()
  if (w?.startDragging) {
    try { w.startDragging(); return } catch { /* 回退自实现 */ }
  }
  dragStart = {
    mx: ev.clientX, my: ev.clientY,
    wx: window.screenX, wy: window.screenY,
  }
  window.addEventListener('mousemove', onDragMove, { capture: true })
  window.addEventListener('mouseup', endDrag, { once: true, capture: true })
  ev.preventDefault()
}
function onDragMove(ev: MouseEvent) {
  if (!dragStart) return
  const dx = ev.clientX - dragStart.mx
  const dy = ev.clientY - dragStart.my
  const nx = dragStart.wx + dx
  const ny = dragStart.wy + dy
  moveWindow(nx, ny)
}
function endDrag() {
  dragStart = null
  window.removeEventListener('mousemove', onDragMove, { capture: true })
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
  void w.invoke('pet_overlay_move', { x, y })
}

/* ─── 右键菜单 ─── */
const menuOpen = ref(false)
const menuPos = reactive({ x: 0, y: 0 })
function onContextMenu(ev: MouseEvent) {
  menuPos.x = Math.min(ev.clientX, window.innerWidth - 220)
  menuPos.y = Math.min(ev.clientY, window.innerHeight - 340)
  menuOpen.value = true
}
function hideMe() {
  menuOpen.value = false
  const w = getWindow()
  if (!w?.invoke) {
    // Web 测试态：直接关闭窗口（诚实提示无 Tauri）
    alert('Web 模式下请关闭当前标签页；桌面端可通过托盘菜单再开启。')
    return
  }
  void w.invoke('pet_overlay_hide')
}
function openPetView() {
  menuOpen.value = false
  window.open('/control/pet', '_blank')
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

  await loadSkins()
  void loadStatus()
  void nextTick(initStage)
  raf = requestAnimationFrame(loop)
  pollTimer = setInterval(() => void loadStatus(), 15000)

  // 外部点击关闭菜单
  window.addEventListener('mousedown', onOutsideMenu)
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
  window.removeEventListener('mousemove', onDragMove, { capture: true })
})

/* 皮肤切换 → 重建 3D 舞台 */
watch(currentSkinId, () => { void nextTick(initStage) })
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
  box-shadow:
    0 20px 60px -20px color-mix(in srgb, var(--accent) 40%, color-mix(in srgb, var(--scrim, var(--text)) 60%, transparent)),
    0 1px 0 0 color-mix(in srgb, var(--accent) 22%, transparent);
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
.pet-overlay__menu-sub { padding: 6px 8px 4px; font-size: 11px; color: var(--text-dim); letter-spacing: 0.03em; text-transform: uppercase; }
.pet-overlay__menu-item {
  width: 100%; text-align: left; padding: 6px 8px; border-radius: var(--radius-sm);
  border: none; background: transparent; color: var(--text); cursor: pointer;
  transition: background var(--dur-fast) var(--ease-out);
}
.pet-overlay__menu-item:hover { background: color-mix(in srgb, var(--accent) 14%, var(--card)); }
.pet-overlay__menu-item.is-danger { color: var(--danger); }
.pet-overlay__menu-item.is-danger:hover { background: color-mix(in srgb, var(--danger) 12%, var(--card)); }
.pet-overlay__menu-sep { height: 1px; margin: 6px 6px; background: var(--hairline); opacity: 0.6; }
.pet-overlay__menu-skins { display: grid; grid-template-columns: repeat(5, 1fr); gap: 6px; padding: 2px 4px 4px; }
.pet-overlay__skin {
  aspect-ratio: 1 / 1; border-radius: var(--radius-md);
  border: 1px solid var(--hairline); cursor: pointer;
  display: flex; align-items: center; justify-content: center; font-size: 18px;
  transition: transform var(--dur-fast) var(--ease-out);
}
.pet-overlay__skin:hover { transform: translateY(-1px); }
.pet-overlay__skin.is-active { border-color: var(--accent); box-shadow: 0 0 0 2px color-mix(in srgb, var(--accent) 20%, transparent); }

/* 过渡 */
.fade-enter-active, .fade-leave-active { transition: opacity var(--dur-fast) var(--ease-out), transform var(--dur-fast) var(--ease-out); }
.fade-enter-from, .fade-leave-to { opacity: 0; transform: translateY(-4px); }
</style>
