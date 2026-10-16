<template>
  <Teleport to="body">
    <div
      v-if="enabled"
      ref="petEl"
      class="desktop-pet"
      :class="[
        `is-${behavior}`,
        `mood-${mood}`,
        { 'is-dragging': dragging, 'is-petted': petted, 'is-image': skin.kind === 'image', 'is-flipped': flipped },
      ]"
      :style="petStyle"
      role="img"
      :aria-label="label"
      @pointerdown="onPointerDown"
      @pointerup="onPointerUp"
      @pointercancel="onPointerUp"
      @click="onClick"
      @dblclick="petBounce"
      @contextmenu.prevent="onContextMenu"
    >
      <!-- 情绪气泡 -->
      <div class="pet-bubble" v-if="bubble">{{ bubble }}</div>
      <!-- PSI 意识气泡（连接感高时自动浮现） -->
      <div class="pet-bubble pet-bubble--psi" v-if="psiBubble">{{ psiBubble }}</div>

      <!-- PSI 八维情绪面板（点击徽标展开，只读展示真实引擎状态） -->
      <div v-if="showPsi" class="psi-panel" @pointerdown.stop @click.stop>
        <div class="psi-panel__head">
          <span>PSI 意识引擎 · Viky</span>
          <span class="psi-panel__phi">Φ = {{ psiPhi.toFixed(3) }}</span>
        </div>
        <div class="psi-panel__phase">相位：{{ psi?.circadian ?? '—' }} · 主导情绪：{{ psi?.dominant_mood ?? '—' }}</div>
        <div class="psi-panel__bars">
          <div v-for="(emoji, name) in PSI_EMOJI" :key="name" class="psi-panel__bar-row">
            <span class="psi-panel__bar-label">{{ emoji }} {{ name }}</span>
            <div class="psi-panel__bar-track">
              <div class="psi-panel__bar-fill" :style="{ width: `${Math.round(psiEmotion(name) * 100)}%` }" />
            </div>
            <span class="psi-panel__bar-val">{{ Math.round(psiEmotion(name) * 100) }}%</span>
          </div>
        </div>
      </div>

      <!-- 粒子层 -->
      <div class="pet-particles">
        <span v-for="(p, i) in particles" :key="i" class="pet-particle" :style="p.style">{{ p.emoji }}</span>
      </div>

      <!-- 2D SVG 立体桌宠（矢量分层 + 伪 3D 光照：猫/精灵全数据驱动，免 WebGL）。
           悬停 → 局部聚焦注视（视线/瞳孔幅度放大 + 头部视差）；无 WebGL 也能完整渲染。 -->
      <div
        v-if="skin.kind === 'canvas' || skin.kind === 'model'"
        ref="petSceneEl"
        class="desktop-pet__3d"
        :class="{ 'is-2d': !petScene, 'is-focused': focused }"
        @pointerenter="onStageEnter"
        @pointerleave="onStageLeave"
      >
        <span v-if="!petScene" class="desktop-pet__badge">渲染未启动</span>
      </div>
      <!-- 静态立绘（保留 image 类型兼容；当前皮肤列表已全 3D 化） -->
      <img
        v-else
        class="desktop-pet__img"
        :src="skin.src"
        :alt="skin.name"
        draggable="false"
      />

      <!-- 互动菜单 -->
      <div v-if="menuOpen" class="pet-menu" @pointerdown.stop @click.stop>
        <button @click="feedPet">🍖 投喂</button>
        <button @click="triggerAction('dance')">💃 跳舞</button>
        <button @click="triggerAction('spin')">🌀 转圈</button>
        <button @click="triggerAction('excited')">✨ 开心</button>
        <button @click="triggerAction('sleep')">💤 睡觉</button>
        <button @click="triggerAction('walk')">🚶 散步</button>
        <button class="pet-menu__psi" @click="togglePsiPanel">🧠 PSI 状态</button>
      </div>
    </div>
  </Teleport>
</template>

<script setup lang="ts">
import { computed, nextTick, onMounted, onUnmounted, ref, watch } from 'vue'
import { PetSvg } from './petSvg'
import type { PetMood3D } from './petContract'
import { getCurrentSkin, PET_SKINS, type PetSkin } from './skins'
// 数据驱动模型：猫/精灵全 3D 化 —— 按皮肤 id 取 Spec 热插拔渲染
import { getPetModelSpec, PET_INTERACTION } from './petSpec'
// 契约驱动：后端 mood → PetMood3D（单一来源，未知值收敛 normal，绝不绕过契约）
import { toPetMood3D } from './petContract'
// 交互纯函数：部位热区 / 轻点-拖拽判别 / 松手惯性（数学留在 petPose，渲染层无随机）
import {
  classifyGesture,
  regionFromPoint,
  stepInertia,
  type PetGesture,
  type PetHitRegion,
} from './petPose'
// V2.1 统一契约入口：usePetRpc（pet.status / pet.triggerAction 真实联动）
import { usePetRpc } from '@/rpc/petContract'
// v3 诚实渲染：运行时能力探测（无 WebGL → CSS 2.5D 降级，绝不假装 3D）
import { detectPetRuntime, renderTierOf, type PetRuntimeInfo } from './detectPetRuntime'
// 智慧中枢：RPC 事件 → 情绪/唤醒/经验（与既有行为机正交叠加）
import { getPetBrain } from './petBrain'
import { useRpcStore } from '@/stores/rpc'
// PSI 解包纯函数（含类型），便于单测守护；与 aris_brain `brain.status` 真实载荷对齐
import { unpackPsiBrain, type PsiSnapshot } from './petPsi'
// ── 运行时档位（WebGL/硬件信号仅决定特效预算；SVG 无 WebGL 也完整渲染）──
const runtime = ref<PetRuntimeInfo>(detectPetRuntime())

// ── Viky 数字生命体 · PSI 意识引擎接入 ──
// 桌宠作为 PSI 意识的「具身表现层」：轮询 brain.status 把八维情感实时映射到外观。
// 情绪主导 valence → 快乐/悲伤；arousal → 兴奋/困倦；certainty 低 → 困惑；
// 连接感 connection 高且空闲 → 主动打招呼。全程只读真实引擎状态，绝无模拟。
// PSI 快照类型与解包函数统一放在 ./petPsi（可被单测直接覆盖，防止字段解包回退旧 bug）。
const psi = ref<PsiSnapshot | null>(null)
const psiBubble = ref('')
const showPsi = ref(false)
let psiTimer: ReturnType<typeof setInterval> | null = null
let psiBubbleTimer: ReturnType<typeof setTimeout> | null = null
const PSI_EMOJI: Record<string, string> = {
  valence: '😊',
  arousal: '⚡',
  curiosity: '🔍',
  confidence: '💪',
  connection: '💞',
  growth: '🌱',
  certainty: '🧭',
  existence: '✨',
}
function numOr(v: unknown, fallback: number): number {
  return typeof v === 'number' && Number.isFinite(v) ? v : fallback
}
// Φ 在快照里是格式化字符串（phi_existence），此处安全解析为数值
const psiPhi = computed(() => {
  const v = parseFloat(String(psi.value?.phi_existence ?? '0.5'))
  return Number.isFinite(v) ? v : 0.5
})
function psiEmotion(name: string): number {
  const v = psi.value?.emotion?.[name]
  return typeof v === 'number' ? v : 0.5
}
function applyPsiToPet(): void {
  const emotions = psi.value?.emotion
  if (!emotions) return
  const valence = numOr(emotions.valence, 0.5)
  const arousal = numOr(emotions.arousal, 0.5)
  const certainty = numOr(emotions.certainty, 0.5)
  // 仅在工作/空闲状态下由 PSI 驱动外观，避免与用户操作冲突
  if (behavior.value === 'work' || mood.value === 'working') return
  if (arousal < 0.28) {
    mood.value = 'sleepy'
  } else if (certainty < 0.3) {
    mood.value = 'sad'
    bubble.value = '🤔 有些困惑…'
  } else if (valence > 0.7 && arousal > 0.6) {
    mood.value = 'excited'
  } else if (valence > 0.6) {
    mood.value = 'happy'
  } else if (valence < 0.35) {
    mood.value = 'sad'
  } else {
    mood.value = 'normal'
  }
}
function maybePsiGreeting(): void {
  const connection = psiEmotion('connection')
  const certainty = psiEmotion('certainty')
  const existence = psiEmotion('existence')
  if (psiBubbleTimer) clearTimeout(psiBubbleTimer)
  if (connection > 0.75 && certainty > 0.6 && behavior.value === 'idle') {
    psiBubble.value = `🧠 Φ=${psiPhi.value.toFixed(2)} · 连接感 ${Math.round(connection * 100)}% · 自我存在 ${Math.round(existence * 100)}%`
    psiBubbleTimer = setTimeout(() => { psiBubble.value = '' }, 4000)
  }
}
async function pollPsi(): Promise<void> {
  const rpc = useRpcStore()
  if (rpc.state !== 'connected') return
  try {
    if (!rpc.supportsMethod('brain.status')) return
    const res = await rpc.call<{ ok?: boolean; brain?: PsiSnapshot }>('brain.status', {})
    const b = unpackPsiBrain(res)
    psi.value = b
    if (b && b.emotion) {
      applyPsiToPet()
      maybePsiGreeting()
    }
  } catch {
    /* 引擎未就绪时静默跳过，保持既有行为 */
  }
}
function onPsiBroadcast(e: Event): void {
  const detail = (e as CustomEvent).detail
  // 广播载荷可能是 { brain } 包装或直接就是 snapshot（与 PetView 保持一致），统一由 unpackPsiBrain 处理
  const wrapped = unpackPsiBrain(detail)
  if (wrapped) {
    psi.value = wrapped
    applyPsiToPet()
  }
}
function togglePsiPanel(): void {
  showPsi.value = !showPsi.value
}


const ENABLED_KEY = 'pet.desktopEnabled'
const POSITION_KEY = 'pet.desktop.position'

const enabled = ref(localStorage.getItem(ENABLED_KEY) === 'true')
const pos = ref(readPosition())
const dragging = ref(false)
const petted = ref(false)
const petEl = ref<HTMLElement | null>(null)
const petSceneEl = ref<HTMLElement | null>(null)
const skin = ref<PetSkin>(getCurrentSkin())

// ── 后端联动（V2.1 · 桌宠持久化）──
// 启动拉取真实好感度/心情；交互按后端「真实支持」的动作调 triggerAction；
// 好感度以服务端返回为准（本地乐观更新 + 失败回滚）；失败静默降级为本地，
// 不弹错误、不阻塞渲染（用户偏好：连接状态不可见）。
const petRpc = usePetRpc()
const affection = ref<number | null>(null) // 服务端权威好感度（null = 尚未同步/离线）
let statusTimer: ReturnType<typeof setInterval> | null = null
let affectionInFlight = false // 好感度动作在途守卫：连点/双击不重复触发同一后端调用

/** 后端可用性守卫：未连接或未注册方法时诚实跳过（不假装调用） */
function petRpcReady(): boolean {
  const rpc = useRpcStore()
  return rpc.state === 'connected' && rpc.supportsMethod('pet.triggerAction')
}

/** 用契约把后端 mood 字符串接到 3D 表情；仅在空闲时接管，避免打断用户动作/工作态 */
function applyServerMood(raw: unknown): void {
  if (behavior.value !== 'idle') return
  mood.value = toPetMood3D(raw)
}

/** 启动/周期同步真实状态（失败静默，保留本地表现） */
async function syncStatus(): Promise<void> {
  const rpc = useRpcStore()
  if (rpc.state !== 'connected' || !rpc.supportsMethod('pet.status')) return
  try {
    const s = await petRpc.status()
    affection.value = s.affection
    applyServerMood(s.mood)
  } catch {
    /* 引擎未就绪 / 离线 → 静默降级为本地，不弹错误 */
  }
}

/**
 * 好感度类动作（pet=抚摸 / feed=投喂）：后端 `workbench::pet::interact` 真实持久化好感度 + 能量。
 * 本地乐观 +1 → 服务端返回为准 → 失败回滚。celebrate/wake/doze 不改好感度，不走此路径。
 */
async function callAffectionAction(action: 'pet' | 'feed'): Promise<void> {
  if (!petRpcReady() || affectionInFlight) return
  affectionInFlight = true
  const prevAffection = affection.value
  const prevMood = mood.value
  affection.value = (affection.value ?? 0) + 1 // 乐观更新
  try {
    const res = await petRpc.triggerAction(action)
    if (!res.ok) throw new Error(res.error || `动作 ${action} 失败`)
    if (typeof res.affection === 'number') affection.value = res.affection // 服务端权威值
    applyServerMood(res.mood)
  } catch {
    // 失败回滚（不弹错误，连接状态不可见）
    affection.value = prevAffection
    if (behavior.value === 'idle') mood.value = prevMood
  } finally {
    affectionInFlight = false
  }
}

/** 心情类动作（celebrate/wake/doze）：仅持久化后端心情，不动好感度，静默降级 */
async function fireMoodAction(action: 'celebrate' | 'wake' | 'doze'): Promise<void> {
  if (!petRpcReady()) return
  try {
    await petRpc.triggerAction(action)
  } catch {
    /* 静默：心情动作属增强，失败不影响本地表现 */
  }
}

// ── 行为状态机 ──
type Behavior = 'idle' | 'walk' | 'spin' | 'dance' | 'excited' | 'sleep' | 'work' | 'think' | 'sad' | 'celebrate'
type Mood = 'normal' | 'happy' | 'sleepy' | 'working' | 'excited' | 'sad'

const behavior = ref<Behavior>('idle')
const mood = ref<Mood>('normal')
const flipped = ref(false)
const bubble = ref('')
const menuOpen = ref(false)
const particles = ref<{ emoji: string; style: Record<string, string> }[]>([])

let petScene: PetSvg | null = null

interface Point { x: number; y: number }

function readPosition(): Point {
  try {
    const raw = localStorage.getItem(POSITION_KEY)
    if (raw) {
      const parsed = JSON.parse(raw) as Partial<Point>
      if (typeof parsed.x === 'number' && typeof parsed.y === 'number') return { x: parsed.x, y: parsed.y }
    }
  } catch { /* fall through */ }
  return { x: window.innerWidth - 120, y: window.innerHeight - 170 }
}

const PET_W = 100
const PET_H = 110

const petStyle = computed(() => ({
  left: `${pos.value.x}px`,
  top: `${pos.value.y}px`,
  // 走路/工作时用 CSS transition 平滑移动
  transition: behavior.value === 'walk' || behavior.value === 'work'
    ? 'left 1.2s var(--ease-in-out), top 1.2s var(--ease-in-out)'
    : 'none',
}))

const label = computed(() => `Marguerite ${skin.value.name} · 桌面宠`)

let dragStart: Point | null = null
let petStart: Point | null = null
let pettedTimer: ReturnType<typeof setTimeout> | null = null
let behaviorTimer: ReturnType<typeof setTimeout> | null = null
let idleTimer: ReturnType<typeof setTimeout> | null = null
let particleTimer: ReturnType<typeof setTimeout> | null = null

// 交互手势 / 惯性状态
let dragStartAt = 0 // 指针按下时刻（轻点-拖拽判别）
let lastGesture: PetGesture = 'tap' // 最近一次手势（onClick 据此区分轻点与拖动）
let dragVel = { vx: 0, vy: 0 } // 拖拽速度 px/s（松手惯性初速）
let lastSample: { x: number; y: number; t: number } | null = null
let inertiaRaf = 0

/** 可视区内可停留的合法边界（含 8px 边距） */
function petBounds() {
  return {
    minX: 8,
    minY: 8,
    maxX: Math.max(8, window.innerWidth - PET_W - 8),
    maxY: Math.max(8, window.innerHeight - PET_H - 8),
  }
}

/** 持久化落定位置（保持既有「可移动桌宠」行为） */
function persistPosition(): void {
  localStorage.setItem(POSITION_KEY, JSON.stringify(pos.value))
}

// ── 拖拽（含松手惯性滑行）──
function onPointerDown(event: PointerEvent) {
  if (event.button === 2) return // 右键交给 contextmenu
  cancelInertia()
  dragging.value = true
  dragStart = { x: event.clientX, y: event.clientY }
  dragStartAt = performance.now()
  petStart = { ...pos.value }
  dragVel = { vx: 0, vy: 0 }
  lastSample = { x: pos.value.x, y: pos.value.y, t: dragStartAt }
  lastGesture = 'tap'
  petEl.value?.setPointerCapture(event.pointerId)
  document.addEventListener('pointermove', onPointerMove)
  document.addEventListener('pointerup', onPointerUp, { once: true })
  menuOpen.value = false
}

function onPointerMove(event: PointerEvent) {
  if (!dragging.value || !dragStart || !petStart) return
  const dx = event.clientX - dragStart.x
  const dy = event.clientY - dragStart.y
  const bounds = petBounds()
  const nx = clamp(petStart.x + dx, bounds.minX, bounds.maxX)
  const ny = clamp(petStart.y + dy, bounds.minY, bounds.maxY)
  const now = performance.now()
  if (lastSample) {
    const dms = now - lastSample.t
    if (dms > 0) dragVel = { vx: ((nx - lastSample.x) / dms) * 1000, vy: ((ny - lastSample.y) / dms) * 1000 }
  }
  lastSample = { x: nx, y: ny, t: now }
  pos.value = { x: nx, y: ny }
}

function clamp(v: number, min: number, max: number): number {
  return Math.min(Math.max(v, min), Math.max(min, max))
}

function onPointerUp(event?: PointerEvent) {
  if (!dragging.value) return
  dragging.value = false
  document.removeEventListener('pointermove', onPointerMove)
  const start = dragStart
  const dx = event && start ? event.clientX - start.x : 0
  const dy = event && start ? event.clientY - start.y : 0
  const dtMs = start ? performance.now() - dragStartAt : 0
  lastGesture = classifyGesture(dx, dy, dtMs)
  const speed = Math.hypot(dragVel.vx, dragVel.vy)
  dragStart = null
  petStart = null
  lastSample = null
  // 拖动松手且仍有速度 → 惯性滑行；轻点/静止 → 直接落定持久化
  if (lastGesture === 'drag' && speed > PET_INTERACTION.inertia.minSpeed) {
    startInertia()
  } else {
    dragVel = { vx: 0, vy: 0 }
    persistPosition()
  }
}

/** 松手惯性滑行：纯函数 stepInertia 逐帧推进，减速落定后持久化位置 */
function startInertia() {
  cancelInertia()
  const bounds = petBounds()
  let body = { x: pos.value.x, y: pos.value.y, vx: dragVel.vx, vy: dragVel.vy }
  let prev = performance.now()
  const step = () => {
    const now = performance.now()
    const dt = (now - prev) / 1000
    prev = now
    const next = stepInertia(body, dt, bounds, PET_INTERACTION.inertia)
    body = next.body
    pos.value = { x: body.x, y: body.y }
    if (Math.abs(body.vx) > 1) flipped.value = body.vx < 0 // 朝向随水平速度
    if (next.resting) {
      inertiaRaf = 0
      dragVel = { vx: 0, vy: 0 }
      persistPosition()
      return
    }
    inertiaRaf = requestAnimationFrame(step)
  }
  inertiaRaf = requestAnimationFrame(step)
}

function cancelInertia() {
  if (inertiaRaf) cancelAnimationFrame(inertiaRaf)
  inertiaRaf = 0
}

// ── 点击互动：部位热区（头/背/尾）→ 轻点反应（与拖动区分）──
function onClick(event: MouseEvent) {
  if (dragging.value) return
  if (lastGesture === 'drag') return // 拖动结束后的 click 不当作轻点
  const rect = petEl.value?.getBoundingClientRect()
  const ny = rect && rect.height > 0 ? (event.clientY - rect.top) / rect.height : 0.5
  reactToRegion(regionFromPoint(ny))
}

/** 部位轻点反应：纯函数姿态（PetSvg.poke）+ 既有 mood 通道。
 *  头/背轻点 = 抚摸 → 后端 `pet` 真实持久化好感度；尾部无对应后端动作 → 仅本地甩尾。 */
function reactToRegion(region: PetHitRegion) {
  petScene?.poke(region)
  if (region === 'tail') {
    spawnParticles(['✨', '💫'], 4)
    return
  }
  mood.value = 'happy'
  petted.value = true
  spawnParticles(['♡', '♥'], 4)
  void callAffectionAction('pet')
  if (pettedTimer) clearTimeout(pettedTimer)
  pettedTimer = setTimeout(() => {
    petted.value = false
    if (behavior.value === 'idle') mood.value = 'normal'
  }, 900)
}

function petBounce() {
  petted.value = true
  mood.value = 'happy'
  brain.petted() // 智慧中枢：抚摸 → 庆祝 + 经验成长
  spawnParticles(['♡', '♥', '♡'], 5)
  petScene?.jump()
  petScene?.burst(0xff88aa, 8)
  void callAffectionAction('pet') // 后端真实持久化好感度（乐观更新 + 失败回滚）
  if (pettedTimer) clearTimeout(pettedTimer)
  pettedTimer = setTimeout(() => {
    petted.value = false
    if (behavior.value === 'idle') mood.value = 'normal'
  }, 1500)
}

/** 投喂（右键菜单）→ 后端 `feed` 真实持久化好感度 + 能量（乐观更新 + 失败回滚） */
function feedPet() {
  menuOpen.value = false
  petted.value = true
  mood.value = 'happy'
  spawnParticles(['🍖', '🐟', '✨'], 6)
  petScene?.jump()
  petScene?.burst(0xffbb66, 8)
  void callAffectionAction('feed')
  if (pettedTimer) clearTimeout(pettedTimer)
  pettedTimer = setTimeout(() => {
    petted.value = false
    if (behavior.value === 'idle') mood.value = 'normal'
  }, 1200)
}

function onContextMenu(e: MouseEvent) {
  e.preventDefault()
  menuOpen.value = !menuOpen.value
  setTimeout(() => {
    if (menuOpen.value) {
      const close = () => { menuOpen.value = false; document.removeEventListener('click', close) }
      document.addEventListener('click', close)
    }
  }, 0)
}

// ── 行为控制 ──
function setBehavior(b: Behavior, durationMs: number) {
  behavior.value = b
  if (behaviorTimer) clearTimeout(behaviorTimer)
  behaviorTimer = setTimeout(() => {
    behavior.value = 'idle'
    if (mood.value !== 'working') mood.value = 'normal'
  }, durationMs)
}

function triggerAction(b: Behavior) {
  menuOpen.value = false
  switch (b) {
    case 'dance':
      mood.value = 'excited'
      setBehavior('dance', 3000)
      spawnParticles(['♪', '♫', '♬', '✨'], 8)
      petScene?.dance(3)
      petScene?.burst(0xffcc66, 10)
      break
    case 'spin':
      setBehavior('spin', 1200)
      petScene?.spin()
      break
    case 'excited':
      mood.value = 'happy'
      setBehavior('excited', 1500)
      spawnParticles(['✨', '⭐', '💫'], 6)
      petScene?.jump()
      petScene?.burst(0x88ddff, 8)
      void fireMoodAction('celebrate') // 同步后端心情（excited）
      break
    case 'sleep':
      mood.value = 'sleepy'
      setBehavior('sleep', 5000)
      bubble.value = '💤'
      void fireMoodAction('doze') // 同步后端心情（sleepy）
      setTimeout(() => { bubble.value = '' }, 5000)
      break
    case 'walk':
      walkToRandom()
      break
  }
}

// ── 自主行为 ──
function walkToRandom() {
  const targetX = clamp(pos.value.x + (Math.random() - 0.5) * 300, 8, window.innerWidth - PET_W - 8)
  const targetY = clamp(pos.value.y + (Math.random() - 0.3) * 100, 8, window.innerHeight - PET_H - 8)
  flipped.value = targetX < pos.value.x
  behavior.value = 'walk'
  mood.value = 'normal'
  pos.value = { x: targetX, y: targetY }
  petScene?.setWalking(true)
  petScene?.setFacing(flipped.value ? -1 : 1)
  if (behaviorTimer) clearTimeout(behaviorTimer)
  behaviorTimer = setTimeout(() => {
    behavior.value = 'idle'
    petScene?.setWalking(false)
    scheduleIdleBehavior()
  }, 1500)
}

function scheduleIdleBehavior() {
  if (idleTimer) clearTimeout(idleTimer)
  const delay = 8000 + Math.random() * 15000
  idleTimer = setTimeout(() => {
    if (behavior.value !== 'idle' || mood.value === 'working') {
      scheduleIdleBehavior()
      return
    }
    const r = Math.random()
    if (r < 0.35) {
      walkToRandom()
    } else if (r < 0.5) {
      triggerAction('spin')
      setTimeout(() => scheduleIdleBehavior(), 1500)
    } else if (r < 0.6) {
      triggerAction('dance')
      setTimeout(() => scheduleIdleBehavior(), 3500)
    } else if (r < 0.72) {
      // 打盹
      mood.value = 'sleepy'
      behavior.value = 'sleep'
      bubble.value = '💤'
      behaviorTimer = setTimeout(() => {
        behavior.value = 'idle'
        mood.value = 'normal'
        bubble.value = ''
        scheduleIdleBehavior()
      }, 4000)
    } else {
      scheduleIdleBehavior()
    }
  }, delay)
}

// ── 粒子效果 ──
function spawnParticles(emojis: string[], count: number) {
  const newParticles: { emoji: string; style: Record<string, string> }[] = []
  for (let i = 0; i < count; i++) {
    const emoji = emojis[Math.floor(Math.random() * emojis.length)]
    const x = 20 + Math.random() * 60
    const delay = Math.random() * 0.5
    const dur = 1 + Math.random() * 1
    newParticles.push({
      emoji,
      style: {
        left: `${x}%`,
        animationDelay: `${delay}s`,
        animationDuration: `${dur}s`,
      },
    })
  }
  particles.value = newParticles
  if (particleTimer) clearTimeout(particleTimer)
  particleTimer = setTimeout(() => { particles.value = [] }, 2500)
}

// ── 任务事件 ──
function onTaskEvent(e: Event) {
  const detail = (e as CustomEvent).detail as { state: string }
  if (detail.state === 'working') {
    mood.value = 'working'
    behavior.value = 'work'
    bubble.value = '💭'
    if (behaviorTimer) clearTimeout(behaviorTimer)
    if (idleTimer) clearTimeout(idleTimer)
  } else if (detail.state === 'done') {
    mood.value = 'excited'
    behavior.value = 'celebrate'
    bubble.value = '✨'
    spawnParticles(['🎉', '✨', '⭐', '♡'], 10)
    petScene?.dance(3)
    petScene?.burst(0xffcc44, 16)
    petScene?.jump()
    behaviorTimer = setTimeout(() => {
      behavior.value = 'idle'
      mood.value = 'normal'
      bubble.value = ''
      scheduleIdleBehavior()
    }, 3500)
  } else if (detail.state === 'cancelled') {
    mood.value = 'sad'
    behavior.value = 'sad'
    bubble.value = '😅'
    behaviorTimer = setTimeout(() => {
      behavior.value = 'idle'
      mood.value = 'normal'
      bubble.value = ''
      scheduleIdleBehavior()
    }, 2500)
  }
}

function onToolEvent(e: Event) {
  if (mood.value !== 'working') return
  const detail = (e as CustomEvent).detail as { name: string }
  // 工具调用时偶尔跳一下
  if (Math.random() < 0.3) {
    petted.value = true
    petScene?.jump()
    setTimeout(() => { petted.value = false }, 600)
  }
  // 浏览器/电脑操控类工具显示特殊气泡
  if (detail.name?.includes('browser') || detail.name?.includes('computer')) {
    bubble.value = '🖥️'
    setTimeout(() => { if (mood.value === 'working') bubble.value = '💭' }, 1500)
  }
}

// ── 存储/皮肤事件 ──
function onStorage(event: StorageEvent) {
  if (event.key === ENABLED_KEY) {
    enabled.value = localStorage.getItem(ENABLED_KEY) === 'true'
  }
  if (event.key === 'pet.skin') {
    skin.value = getCurrentSkin()
    initPetScene()
  }
}

function onSkinChanged(e: Event) {
  const id = (e as CustomEvent).detail as string
  skin.value = PET_SKINS.find((s) => s.id === id) ?? PET_SKINS[0]
  initPetScene()
}

// ── SVG 桌宠初始化（矢量渲染，无 WebGL 依赖；档位只影响特效预算，不降级形象）──
function initPetScene() {
  // 静态立绘（当前列表已全模型化）不需要矢量场景
  if (skin.value.kind === 'image') {
    petScene?.dispose()
    petScene = null
    return
  }
  nextTick(() => {
    if (!petSceneEl.value) return
    petScene?.dispose()
    petScene = null
    const accentHex = skin.value.accent.startsWith('#')
      ? parseInt(skin.value.accent.slice(1), 16)
      : 0x9ec5e8
    try {
      // 数据驱动造型：兽形/人形由 Spec 决定外观（热插拔，渲染代码零改动）
      const spec = getPetModelSpec(skin.value.model ?? skin.value.id)
      petScene = new PetSvg({
        container: petSceneEl.value,
        accent: accentHex,
        spec,
        quality: renderTierOf(runtime.value.quality),
      })
      petScene.setMood(mood.value as PetMood3D)
      petScene.setFacing(flipped.value ? -1 : 1)
      // 重建后补上当前悬停态（光标可能一直停在它身上，换皮肤不应丢失聚焦）
      petScene.setHover(focused.value ? 1 : 0)
    } catch (e) {
      // 构造失败（如环境缺 document）→ 置空并由模板如实标注，不假装已渲染
      console.warn('[PetSvg] 创建失败:', e)
      petScene = null
    }
  })
}

// ── 鼠标视线追踪 ──
function onMouseMove(e: MouseEvent) {
  if (!petScene || !petEl.value) return
  const rect = petEl.value.getBoundingClientRect()
  const cx = rect.left + rect.width / 2
  const cy = rect.top + rect.height / 2
  petScene.lookAt(
    ((e.clientX - cx) / window.innerWidth) * 2,
    ((e.clientY - cy) / window.innerHeight) * 2,
  )
}

// ── 悬停聚焦（光标停在它身上 → 局部注视 + 镜头轻微推近）──
const focused = ref(false)
function onStageEnter() {
  focused.value = true
  petScene?.setHover(1)
}
function onStageLeave() {
  focused.value = false
  petScene?.setHover(0)
}

// ── mood / behavior 变化同步到 3D ──
watch(mood, (m) => { petScene?.setMood(m as PetMood3D) })
watch(flipped, (f) => { petScene?.setFacing(f ? -1 : 1) })

onMounted(() => {
  window.addEventListener('storage', onStorage)
  window.addEventListener('pet-skin-changed', onSkinChanged)
  window.addEventListener('pet-task', onTaskEvent)
  window.addEventListener('pet-tool', onToolEvent)
  window.addEventListener('mousemove', onMouseMove)
  initPetScene()
  scheduleIdleBehavior()
  mountBrain()
  // 后端联动：启动拉取真实好感度/心情（失败静默），并周期刷新以反映衰减
  void syncStatus()
  statusTimer = setInterval(() => { void syncStatus() }, 15000)
})

// ── 智慧中枢接线（正交增量）：RPC 总线事件喂养大脑，成长系统持久化 ──
const brain = getPetBrain()
let brainTimer: ReturnType<typeof setInterval> | null = null
let unsubs: Array<() => void> = []

function mountBrain(): void {
  const rpc = useRpcStore()
  const on = (event: string, feed: Parameters<typeof brain.feed>[0]) => {
    try { unsubs.push(rpc.on(event, () => brain.feed(feed))) } catch { /* rpc 未就绪时跳过 */ }
  }
  on('session.event.text_delta', 'text_delta')
  on('session.event.tool_use_start', 'tool_start')
  on('session.event.tool_result', 'tool_result')
  on('session.event.warning', 'warning')
  on('session.event.task_group.done', 'done')
  on('session.event.plan_delta', 'plan')
  on('task.queued', 'queued')
  // PSI 意识引擎接入：订阅 psi_broadcast 实时事件 + 轮询兜底
  try {
    unsubs.push(rpc.on('psi_broadcast', onPsiBroadcast))
  } catch { /* rpc 未就绪时跳过 */ }
  void pollPsi()
  psiTimer = setInterval(() => { void pollPsi() }, 5000)
  // 大脑时钟：空闲判定/唤醒衰减 → 驱动既有打盹分支
  brainTimer = setInterval(() => {
    const f = brain.tick(0.5)
    if (f.mood === 'sleepy' && behavior.value === 'idle' && mood.value !== 'working') {
      triggerAction('sleep')
    }
    // 规划态 → 托腮沉思（think 行为）
    if (f.mood === 'thinking' && behavior.value === 'idle' && mood.value !== 'working') {
      mood.value = 'normal'
      behavior.value = 'think'
      if (behaviorTimer) clearTimeout(behaviorTimer)
      behaviorTimer = setTimeout(() => {
        if (behavior.value === 'think') {
          behavior.value = 'idle'
          scheduleIdleBehavior()
        }
      }, 6000)
    }
    // 情境语言 v2：周期性地让宠物「说话」
    const remark = brain.remark()
    if (remark) {
      bubble.value = `${remark.emoji} ${remark.text}`
      if (bubbleTimer) clearTimeout(bubbleTimer)
      bubbleTimer = setTimeout(() => {
        if (bubble.value.startsWith(remark.emoji)) bubble.value = ''
      }, 4500)
    }
  }, 500)
}

let bubbleTimer: ReturnType<typeof setTimeout> | null = null

function unmountBrain(): void {
  if (brainTimer) clearInterval(brainTimer)
  brainTimer = null
  if (psiTimer) clearInterval(psiTimer)
  psiTimer = null
  if (psiBubbleTimer) clearTimeout(psiBubbleTimer)
  psiBubbleTimer = null
  if (bubbleTimer) clearTimeout(bubbleTimer)
  bubbleTimer = null
  for (const off of unsubs) { try { off() } catch { /* 已断开 */ } }
  unsubs = []
}

function unmountNow() {
  unmountBrain()
}

onUnmounted(() => {
  window.removeEventListener('storage', onStorage)
  window.removeEventListener('pet-skin-changed', onSkinChanged)
  window.removeEventListener('pet-task', onTaskEvent)
  window.removeEventListener('pet-tool', onToolEvent)
  window.removeEventListener('mousemove', onMouseMove)
  document.removeEventListener('pointermove', onPointerMove)
  cancelInertia()
  unmountNow()
  if (statusTimer) clearInterval(statusTimer)
  if (pettedTimer) clearTimeout(pettedTimer)
  if (behaviorTimer) clearTimeout(behaviorTimer)
  if (idleTimer) clearTimeout(idleTimer)
  if (particleTimer) clearTimeout(particleTimer)
  petScene?.dispose()
  petScene = null
})
</script>

<style scoped>
.desktop-pet {
  align-items: center;
  cursor: grab;
  display: flex;
  flex-direction: column;
  position: fixed;
  touch-action: none;
  user-select: none;
  z-index: 801;
  will-change: transform, left, top;
  perspective: 600px;
}

.desktop-pet.is-dragging { cursor: grabbing; }

/* 3D 容器 */
.desktop-pet__3d {
  width: 100px;
  height: 110px;
  position: relative;
  filter: drop-shadow(0 6px 12px color-mix(in srgb, var(--text) 25%, transparent));
}

/* R1 诚实降级：无 WebGL 时的 CSS 2.5D —— 呼吸/眨眼用 CSS 动画，不假装 3D */
.desktop-pet__3d.is-2d {
  background:
    radial-gradient(circle at 50% 42%, color-mix(in srgb, var(--accent) 16%, transparent), transparent 70%);
  animation: pet2dBreathe 2.8s var(--ease-in-out) infinite;
}
@keyframes pet2dBreathe {
  0%, 100% { transform: scaleY(1) scaleX(1); }
  50% { transform: scaleY(0.97) scaleX(1.02); }
}
.desktop-pet__badge {
  position: absolute;
  left: 50%;
  bottom: 2px;
  transform: translateX(-50%);
  font-size: 9px;
  line-height: 1;
  padding: 2px 6px;
  border-radius: var(--radius-pill);
  color: var(--text-dim);
  background: color-mix(in srgb, var(--card) 88%, transparent);
  border: 1px solid var(--hairline);
  white-space: nowrap;
}
@media (prefers-reduced-motion: reduce) {
  .desktop-pet__3d.is-2d { animation: none; }
}

/* 图片皮肤翻转 */
.desktop-pet.is-flipped .desktop-pet__img {
  transform: scaleX(-1);
}

.desktop-pet__img {
  display: block;
  height: 100px;
  width: auto;
  object-fit: contain;
  filter: drop-shadow(0 4px 12px color-mix(in srgb, var(--text) 35%, transparent));
  -webkit-user-drag: none;
  transform-origin: bottom center;
  transition: transform var(--dur-base) var(--ease-out);
}

/* ── 情绪气泡 ── */
.pet-bubble {
  background: var(--bg-surface);
  border: 1px solid var(--border);
  border-radius: var(--radius-lg);
  font-size: 16px;
  margin-bottom: 2px;
  padding: 1px 7px;
  animation: bubble-pop var(--dur-enter) var(--ease-out);
}

@keyframes bubble-pop {
  0% { transform: scale(0); opacity: 0; }
  70% { transform: scale(1.15); }
  100% { transform: scale(1); opacity: 1; }
}

/* ── 粒子 ── */
.pet-particles {
  pointer-events: none;
  position: absolute;
  inset: 0;
  overflow: visible;
}
.pet-particle {
  position: absolute;
  bottom: 60%;
  font-size: 14px;
  animation: particle-float 1.5s var(--ease-out) forwards; /* motion-allow: particle cadence */
  pointer-events: none;
}
@keyframes particle-float {
  0% { transform: translateY(0) scale(0.5); opacity: 1; }
  100% { transform: translateY(-50px) rotate(30deg) scale(1.2); opacity: 0; }
}

/* ── 右键菜单 ── */
.pet-menu {
  position: absolute;
  bottom: 100%;
  left: 50%;
  transform: translateX(-50%);
  display: flex;
  flex-direction: column;
  gap: 2px;
  background: var(--bg-surface);
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  box-shadow: 0 4px 16px color-mix(in srgb, var(--text) 15%, transparent);
  padding: 4px;
  margin-bottom: 6px;
  z-index: 401;
  min-width: 90px;
}
.pet-menu button {
  background: none;
  border: none;
  border-radius: var(--radius-xs);
  cursor: pointer;
  font-size: 12px;
  padding: 5px 10px;
  text-align: left;
  white-space: nowrap;
  color: var(--text);
}
.pet-menu button:hover {
  background: color-mix(in srgb, var(--accent, cornflowerblue) 15%, transparent);
}

/* ── 图片皮肤的 3D 透视动画（3D 桌宠由 Three.js 自行渲染动画，不应用 CSS 关键帧） ── */
.is-idle .desktop-pet__img {
  animation: pet-bob 3.2s var(--ease-in-out) infinite;
}
@keyframes pet-bob {
  0%, 100% { transform: translateY(0) rotateY(0deg); }
  50% { transform: translateY(-6px) rotateY(8deg); }
}
.is-idle.is-flipped .desktop-pet__img {
  animation: pet-bob-flip 3.2s var(--ease-in-out) infinite;
}
@keyframes pet-bob-flip {
  0%, 100% { transform: translateY(0) rotateY(180deg); }
  50% { transform: translateY(-6px) rotateY(172deg); }
}

.is-walk .desktop-pet__img {
  animation: pet-walk 0.4s var(--ease-in-out) infinite;
}
@keyframes pet-walk {
  0%, 100% { transform: translateY(0) rotateY(0deg) rotateZ(-3deg); }
  50% { transform: translateY(-5px) rotateY(15deg) rotateZ(3deg); }
}
.is-walk.is-flipped .desktop-pet__img {
  animation: pet-walk-flip 0.4s var(--ease-in-out) infinite;
}
@keyframes pet-walk-flip {
  0%, 100% { transform: translateY(0) rotateY(180deg) rotateZ(-3deg); }
  50% { transform: translateY(-5px) rotateY(165deg) rotateZ(3deg); }
}

.is-spin .desktop-pet__img {
  animation: pet-spin-3d 1.1s linear infinite;
}
@keyframes pet-spin-3d {
  0% { transform: rotateY(0deg); }
  100% { transform: rotateY(360deg); }
}

.is-dance .desktop-pet__img {
  animation: pet-dance-3d 0.5s var(--ease-in-out) infinite;
}
@keyframes pet-dance-3d {
  0%, 100% { transform: translateY(0) rotateY(-15deg) rotateZ(-8deg) scale(1); }
  25% { transform: translateY(-10px) rotateY(15deg) rotateZ(8deg) scale(1.05); }
  50% { transform: translateY(-4px) rotateY(-10deg) rotateZ(-5deg) scale(1); }
  75% { transform: translateY(-12px) rotateY(20deg) rotateZ(6deg) scale(1.08); }
}

.is-excited .desktop-pet__img {
  animation: pet-excited-3d 0.35s var(--ease-in-out) infinite;
}
@keyframes pet-excited-3d {
  0%, 100% { transform: translateY(0) rotateY(0deg) scale(1); }
  50% { transform: translateY(-8px) rotateY(20deg) scale(1.08); }
}

.is-sleep .desktop-pet__img {
  animation: pet-sleep 2.8s var(--ease-in-out) infinite;
  opacity: 0.85;
}
@keyframes pet-sleep {
  0%, 100% { transform: translateY(2px) scale(0.97); }
  50% { transform: translateY(0) scale(1); }
}

.is-work .desktop-pet__img {
  animation: pet-work-3d 1.8s var(--ease-in-out) infinite;
}
@keyframes pet-work-3d {
  0%, 100% { transform: translateY(0) rotateY(-5deg) rotateZ(-2deg); }
  50% { transform: translateY(-3px) rotateY(5deg) rotateZ(2deg); }
}

.is-think .desktop-pet__img {
  animation: pet-think-3d 2s var(--ease-in-out) infinite;
}
@keyframes pet-think-3d {
  0%, 100% { transform: rotateY(-8deg) rotateZ(-3deg) translateY(0); }
  50% { transform: rotateY(8deg) rotateZ(3deg) translateY(-2px); }
}

.is-sad .desktop-pet__img {
  animation: pet-sad 2s var(--ease-in-out) infinite;
  opacity: 0.7;
}
@keyframes pet-sad {
  0%, 100% { transform: translateY(2px) rotateX(5deg) rotateZ(-2deg); }
  50% { transform: translateY(4px) rotateX(10deg) rotateZ(2deg); }
}

.is-celebrate .desktop-pet__img {
  animation: pet-celebrate-3d 0.6s var(--ease-in-out) infinite;
}
@keyframes pet-celebrate-3d {
  0% { transform: translateY(0) scale(1) rotateY(0deg); }
  25% { transform: translateY(-14px) scale(1.1) rotateY(-20deg) rotateZ(-10deg); }
  50% { transform: translateY(-6px) scale(1.05) rotateY(0deg); }
  75% { transform: translateY(-14px) scale(1.1) rotateY(20deg) rotateZ(10deg); }
  100% { transform: translateY(0) scale(1) rotateY(0deg); }
}

.is-petted .desktop-pet__img {
  animation: pet-bounce var(--dur-enter) var(--ease-out) 2 !important;
}
@keyframes pet-bounce {
  0%, 100% { transform: translateY(0); }
  50% { transform: translateY(-14px); }
}

/* ── PSI 意识气泡 ── */
.pet-bubble--psi {
  border-color: color-mix(in srgb, var(--accent) 60%, transparent);
  color: var(--accent);
  font-weight: 600;
  white-space: nowrap;
}

/* ── PSI 八维情绪面板 ── */
.psi-panel {
  position: absolute;
  bottom: 100%;
  left: 50%;
  transform: translateX(-50%);
  width: 210px;
  background: var(--bg-surface);
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  box-shadow: 0 8px 28px color-mix(in srgb, var(--text) 20%, transparent);
  padding: 10px 12px;
  margin-bottom: 8px;
  z-index: 402;
  font-size: 11px;
  color: var(--text);
}
.psi-panel__head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  font-weight: 700;
  font-size: 12px;
  margin-bottom: 2px;
}
.psi-panel__phi {
  color: var(--accent);
  font-variant-numeric: tabular-nums;
}
.psi-panel__phase {
  color: var(--text-dim);
  margin-bottom: 8px;
  font-size: 10.5px;
}
.psi-panel__bars {
  display: flex;
  flex-direction: column;
  gap: 4px;
}
.psi-panel__bar-row {
  display: flex;
  align-items: center;
  gap: 6px;
}
.psi-panel__bar-label {
  width: 74px;
  flex: none;
  text-transform: capitalize;
}
.psi-panel__bar-track {
  flex: 1;
  height: 6px;
  border-radius: var(--radius-xs);
  background: color-mix(in srgb, var(--text) 14%, transparent);
  overflow: hidden;
}
.psi-panel__bar-fill {
  height: 100%;
  border-radius: var(--radius-xs);
  background: linear-gradient(90deg, var(--info), var(--accent));
  transition: width var(--dur-base) var(--ease-out);
}
.psi-panel__bar-val {
  width: 30px;
  text-align: right;
  color: var(--text-dim);
  font-variant-numeric: tabular-nums;
}
.pet-menu__psi {
  border-top: 1px solid var(--border) !important;
  color: var(--accent) !important;
  font-weight: 600;
}

@media (prefers-reduced-motion: reduce) {
  .desktop-pet__img { animation: none !important; }
}
</style>
