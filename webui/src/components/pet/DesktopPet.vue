<template>
  <Teleport to="body">
    <div
      v-if="enabled"
      ref="petEl"
      class="desktop-pet"
      :class="{ 'is-dragging': dragging, 'is-petted': petted }"
      :style="style"
      role="img"
      :aria-label="label"
      @pointerdown="onPointerDown"
      @pointerup="onPointerUp"
      @pointercancel="onPointerUp"
      @dblclick="petBounce"
    >
      <div class="desktop-pet__whale">
        <div class="desktop-pet__body"></div>
        <div class="desktop-pet__tail"></div>
        <div class="desktop-pet__eye"></div>
      </div>
      <span class="desktop-pet__bubble" v-if="petted">♡</span>
    </div>
  </Teleport>
</template>

<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from 'vue'

const ENABLED_KEY = 'pet.desktopEnabled'
const POSITION_KEY = 'pet.desktop.position'

const enabled = ref(localStorage.getItem(ENABLED_KEY) === 'true')
const pos = ref(readPosition())
const dragging = ref(false)
const petted = ref(false)
const petEl = ref<HTMLElement | null>(null)

interface Point { x: number; y: number }

function readPosition(): Point {
  try {
    const raw = localStorage.getItem(POSITION_KEY)
    if (raw) {
      const parsed = JSON.parse(raw) as Partial<Point>
      if (typeof parsed.x === 'number' && typeof parsed.y === 'number') return { x: parsed.x, y: parsed.y }
    }
  } catch { /* fall through to default */ }
  return { x: window.innerWidth - 120, y: window.innerHeight - 170 }
}

const style = computed(() => ({
  left: `${pos.value.x}px`,
  top: `${pos.value.y}px`,
}))

const label = 'Marguerite whale pet'

let dragStart: Point | null = null
let petStart: Point | null = null
let pettedTimer: ReturnType<typeof setTimeout> | null = null

function onPointerDown(event: PointerEvent) {
  dragging.value = true
  dragStart = { x: event.clientX, y: event.clientY }
  petStart = { ...pos.value }
  petEl.value?.setPointerCapture(event.pointerId)
  document.addEventListener('pointermove', onPointerMove)
  document.addEventListener('pointerup', onPointerUp, { once: true })
}

function onPointerMove(event: PointerEvent) {
  if (!dragging.value || !dragStart || !petStart) return
  const dx = event.clientX - dragStart.x
  const dy = event.clientY - dragStart.y
  const next = {
    x: clamp(petStart.x + dx, 8, window.innerWidth - 96),
    y: clamp(petStart.y + dy, 8, window.innerHeight - 120),
  }
  pos.value = next
}

function clamp(value: number, min: number, max: number): number {
  return Math.min(Math.max(value, min), Math.max(min, max))
}

function onPointerUp() {
  if (!dragging.value) return
  dragging.value = false
  dragStart = null
  petStart = null
  document.removeEventListener('pointermove', onPointerMove)
  localStorage.setItem(POSITION_KEY, JSON.stringify(pos.value))
}

function petBounce() {
  petted.value = true
  if (pettedTimer) clearTimeout(pettedTimer)
  pettedTimer = setTimeout(() => { petted.value = false }, 1200)
}

function onStorage(event: StorageEvent) {
  if (event.key === ENABLED_KEY) {
    enabled.value = localStorage.getItem(ENABLED_KEY) === 'true'
  }
}

onMounted(() => window.addEventListener('storage', onStorage))
onUnmounted(() => {
  window.removeEventListener('storage', onStorage)
  document.removeEventListener('pointermove', onPointerMove)
  if (pettedTimer) clearTimeout(pettedTimer)
})
</script>

<style scoped>
/* 桌面桌宠：悬浮、可拖拽、主题令牌取色（零裸 hex，过 check:architecture） */
.desktop-pet {
  align-items: flex-end;
  cursor: grab;
  display: flex;
  flex-direction: column;
  position: fixed;
  touch-action: none;
  user-select: none;
  z-index: 400;
}

.desktop-pet.is-dragging { cursor: grabbing; }

.desktop-pet__whale {
  height: 64px;
  position: relative;
  width: 88px;
  animation: pet-bob 3.2s ease-in-out infinite;
}

.desktop-pet__body {
  background: var(--accent);
  border: 2px solid var(--border-strong);
  border-radius: 60% 60% 46% 46%;
  height: 52px;
  left: 12px;
  position: absolute;
  top: 6px;
  width: 64px;
}

.desktop-pet__tail {
  background: var(--accent);
  border: 2px solid var(--border-strong);
  border-radius: 0 100% 100% 0;
  height: 26px;
  left: 2px;
  position: absolute;
  top: 18px;
  transform: rotate(-24deg);
  width: 20px;
}

.desktop-pet__eye {
  background: var(--bg);
  border-radius: 50%;
  height: 8px;
  position: absolute;
  right: 20px;
  top: 24px;
  width: 8px;
}

.desktop-pet__bubble {
  background: var(--bg-surface);
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  color: var(--accent);
  font-size: var(--fs-md);
  margin-bottom: 4px;
  padding: 2px 8px;
}

.desktop-pet.is-petted .desktop-pet__whale {
  animation: pet-bounce var(--dur-base) var(--ease-out) 2;
}

@keyframes pet-bob {
  0%, 100% { transform: translateY(0); }
  50% { transform: translateY(-8px); }
}

@keyframes pet-bounce {
  0%, 100% { transform: translateY(0); }
  50% { transform: translateY(-14px); }
}

@media (prefers-reduced-motion: reduce) {
  .desktop-pet__whale { animation: none; }
  .desktop-pet.is-petted .desktop-pet__whale { animation: none; }
}
</style>
