<script setup lang="ts">
/**
 * Post-mount boot orchestra — continues the CSS splash with a live
 * constellation of agents / subagents / tools. Only runs when splash tier is
 * `full`, once per browser profile (localStorage), and never under reduced motion.
 * Choosing Full again in Appearance clears the seen flag so this can replay.
 */
import { onMounted, onUnmounted, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { useAppStore } from '@/stores/app'
import {
  BOOT_LOG,
  BOOT_NODES,
  BOOT_ORCHESTRA_AUTO_MS,
  BOOT_ORCHESTRA_MIN_MS,
  BOOT_ORCHESTRA_SEEN_KEY,
} from './bootSequence'

const { t } = useI18n()
const appStore = useAppStore()

const visible = ref(false)
const exiting = ref(false)
const ready = ref(false)
const visibleLogs = ref(0)
const litCount = ref(0)

let timers: number[] = []
let onKey: ((e: KeyboardEvent) => void) | null = null

function reducedMotion(): boolean {
  return window.matchMedia('(prefers-reduced-motion: reduce)').matches
}

function hasSeen(): boolean {
  try {
    return localStorage.getItem(BOOT_ORCHESTRA_SEEN_KEY) === '1'
  } catch {
    return false
  }
}

function markSeen(): void {
  try {
    localStorage.setItem(BOOT_ORCHESTRA_SEEN_KEY, '1')
  } catch {
    /* private mode */
  }
}

function clearTimers(): void {
  for (const id of timers) window.clearTimeout(id)
  timers = []
}

function detachKey(): void {
  if (onKey) {
    window.removeEventListener('keydown', onKey)
    onKey = null
  }
}

function dismiss(): void {
  if (exiting.value || !visible.value) return
  markSeen()
  exiting.value = true
  detachKey()
  const exitMs = reducedMotion() ? 80 : 420
  timers.push(window.setTimeout(() => {
    visible.value = false
    exiting.value = false
  }, exitMs))
}

function startSequence(): void {
  if (appStore.bootAnimation !== 'full' || reducedMotion() || hasSeen()) return
  if (visible.value) return

  clearTimers()
  detachKey()
  visible.value = true
  exiting.value = false
  ready.value = false
  visibleLogs.value = 0
  litCount.value = 0

  BOOT_NODES.forEach((_, i) => {
    timers.push(window.setTimeout(() => {
      litCount.value = i + 1
    }, 160 + i * 110))
  })

  BOOT_LOG.forEach((line, i) => {
    timers.push(window.setTimeout(() => {
      visibleLogs.value = i + 1
    }, line.delayMs))
  })

  timers.push(window.setTimeout(() => {
    ready.value = true
  }, BOOT_ORCHESTRA_MIN_MS))

  timers.push(window.setTimeout(() => {
    dismiss()
  }, BOOT_ORCHESTRA_AUTO_MS))

  onKey = (e: KeyboardEvent) => {
    if (e.key === 'Enter' || e.key === 'Escape') dismiss()
  }
  window.addEventListener('keydown', onKey)
}

onMounted(() => {
  startSequence()
})

watch(
  () => appStore.bootAnimation,
  (mode) => {
    if (mode === 'full') startSequence()
  },
)

onUnmounted(() => {
  clearTimers()
  detachKey()
})

function nodeStyle(angle: number, radius: number): Record<string, string> {
  const rad = (angle * Math.PI) / 180
  const x = 50 + Math.cos(rad) * radius * 42
  const y = 50 + Math.sin(rad) * radius * 42
  return { left: `${x}%`, top: `${y}%` }
}
</script>

<template>
  <Teleport to="body">
    <div
      v-if="visible"
      class="boot-orch"
      :class="{ 'is-exiting': exiting }"
      role="dialog"
      aria-modal="true"
      :aria-label="t('boot.orchestraAria')"
      @click.self="dismiss"
    >
      <div class="boot-orch__field" aria-hidden="true">
        <div class="boot-orch__ring" />
        <div class="boot-orch__ring boot-orch__ring--inner" />
        <div
          v-for="(node, i) in BOOT_NODES"
          :key="node.id"
          class="boot-orch__node"
          :class="[`is-${node.kind}`, { 'is-lit': i < litCount }]"
          :style="nodeStyle(node.angle, node.radius)"
        >
          <span class="boot-orch__dot" />
          <span class="boot-orch__label">{{ node.label }}</span>
          <span class="boot-orch__detail">{{ node.detail }}</span>
        </div>
      </div>

      <div class="boot-orch__core">
        <p class="boot-orch__brand">Marguerite</p>
        <p class="boot-orch__tag">{{ t('boot.orchestraTagline') }}</p>
        <div class="boot-orch__bar" aria-hidden="true">
          <span :class="{ 'is-done': ready }" />
        </div>
        <button
          v-if="ready"
          type="button"
          class="boot-orch__enter"
          @click="dismiss"
        >
          {{ t('boot.enterWorkspace') }}
        </button>
        <p v-if="ready" class="boot-orch__hint">{{ t('boot.pressEnter') }}</p>
      </div>

      <ul class="boot-orch__log" aria-hidden="true">
        <li v-for="line in BOOT_LOG.slice(0, visibleLogs)" :key="line.id">
          <span class="boot-orch__ok">ok</span>{{ line.text }}
        </li>
      </ul>
    </div>
  </Teleport>
</template>

<style scoped>
.boot-orch {
  position: fixed;
  inset: 0;
  z-index: 2147482000;
  display: grid;
  place-items: center;
  overflow: hidden;
  background:
    radial-gradient(ellipse 68% 52% at 50% 40%, color-mix(in srgb, var(--accent) 16%, transparent), transparent 70%),
    var(--bg);
  color: var(--text);
  animation: boot-orch-in var(--dur-enter) var(--ease-out) both;
  cursor: pointer;
}

.boot-orch.is-exiting {
  animation: boot-orch-out var(--dur-enter) var(--ease-in) both;
  pointer-events: none;
}

.boot-orch__field {
  position: absolute;
  width: min(560px, 92vw);
  height: min(560px, 92vw);
  top: 48%;
  left: 50%;
  transform: translate(-50%, -54%);
}

.boot-orch__ring {
  position: absolute;
  inset: 10%;
  border-radius: var(--radius-full);
  border: 1px solid color-mix(in srgb, var(--text) 10%, transparent);
  animation: boot-orch-spin calc(var(--dur-pulse) * 5) linear infinite;
}

.boot-orch__ring--inner {
  inset: 26%;
  opacity: 0.65;
  animation-direction: reverse;
  animation-duration: calc(var(--dur-pulse) * 3.6);
}

.boot-orch__node {
  position: absolute;
  transform: translate(-50%, -50%) scale(0.85);
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 2px;
  opacity: 0.2;
  transition:
    opacity var(--dur-enter) var(--ease-out),
    transform var(--dur-enter) var(--ease-spring);
}

.boot-orch__node.is-lit {
  opacity: 1;
  transform: translate(-50%, -50%) scale(1);
}

.boot-orch__dot {
  width: 11px;
  height: 11px;
  border-radius: var(--radius-full);
  background: var(--accent);
  box-shadow: 0 0 0 5px color-mix(in srgb, var(--accent) 16%, transparent);
  animation: boot-orch-pulse var(--dur-pulse) linear infinite;
}

.boot-orch__node.is-subagent .boot-orch__dot { background: var(--ok, var(--accent)); box-shadow: 0 0 0 5px color-mix(in srgb, var(--ok, var(--accent)) 16%, transparent); }
.boot-orch__node.is-tool .boot-orch__dot { background: var(--info, var(--accent)); box-shadow: 0 0 0 5px color-mix(in srgb, var(--info, var(--accent)) 16%, transparent); }
.boot-orch__node.is-memory .boot-orch__dot,
.boot-orch__node.is-retrieval .boot-orch__dot,
.boot-orch__node.is-knowledge .boot-orch__dot { background: var(--warn, var(--accent)); box-shadow: 0 0 0 5px color-mix(in srgb, var(--warn, var(--accent)) 16%, transparent); }

.boot-orch__label {
  font-family: var(--font-mono);
  font-size: 10px;
  letter-spacing: 0.14em;
  text-transform: uppercase;
  color: var(--text);
  white-space: nowrap;
}

.boot-orch__detail {
  font-size: 10px;
  color: var(--text-dim);
  white-space: nowrap;
}

.boot-orch__core {
  position: relative;
  z-index: 2;
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: var(--sp-3);
  text-align: center;
  pointer-events: none;
}

.boot-orch__brand {
  margin: 0;
  font-family: var(--font-display);
  font-size: clamp(2rem, 1.4rem + 2.4vw, 3rem);
  font-weight: 700;
  letter-spacing: 0.02em;
  line-height: 1;
}

.boot-orch__tag {
  margin: 0;
  max-width: 36ch;
  font-size: var(--fs-sm);
  color: var(--text-muted);
}

.boot-orch__bar {
  width: 180px;
  height: 3px;
  margin-top: var(--sp-2);
  border-radius: var(--radius-pill);
  overflow: hidden;
  background: color-mix(in srgb, var(--text) 10%, transparent);
}

.boot-orch__bar > span {
  display: block;
  height: 100%;
  width: 38%;
  border-radius: var(--radius-pill);
  background: var(--accent);
  animation: boot-orch-slide var(--dur-pulse) linear infinite;
  transform-origin: left center;
}

.boot-orch__bar > span.is-done {
  width: 100%;
  animation: none;
}

.boot-orch__enter {
  pointer-events: auto;
  margin-top: var(--sp-3);
  min-height: 36px;
  padding: 0 var(--sp-5);
  border: 1px solid color-mix(in srgb, var(--accent) 45%, transparent);
  border-radius: var(--radius-control);
  background: color-mix(in srgb, var(--accent) 12%, transparent);
  color: var(--accent);
  font-family: var(--font-mono);
  font-size: 11px;
  letter-spacing: 0.22em;
  text-transform: uppercase;
  cursor: pointer;
  transition:
    background var(--dur-fast) var(--ease-standard),
    transform var(--dur-fast) var(--ease-spring);
}

.boot-orch__enter:hover {
  background: color-mix(in srgb, var(--accent) 22%, transparent);
  transform: scale(1.03);
}

.boot-orch__hint {
  margin: 0;
  font-family: var(--font-mono);
  font-size: 10px;
  letter-spacing: 0.12em;
  color: var(--text-dim);
}

.boot-orch__log {
  position: absolute;
  left: var(--sp-5);
  bottom: var(--sp-5);
  margin: 0;
  padding: 0;
  display: flex;
  flex-direction: column;
  gap: 4px;
  font-family: var(--font-mono);
  font-size: 10px;
  color: color-mix(in srgb, var(--text) 45%, transparent);
  pointer-events: none;
}

.boot-orch__log > li {
  list-style: none;
  animation: boot-orch-log var(--dur-base) var(--ease-out) both;
}

.boot-orch__ok {
  color: var(--ok, var(--accent));
  margin-right: 6px;
}

@keyframes boot-orch-in {
  from { opacity: 0; }
  to { opacity: 1; }
}

@keyframes boot-orch-out {
  from { opacity: 1; transform: scale(1); }
  to { opacity: 0; transform: scale(1.03); }
}

@keyframes boot-orch-spin {
  to { transform: rotate(360deg); }
}

@keyframes boot-orch-pulse {
  0%, 100% { opacity: 0.75; }
  50% { opacity: 1; }
}

@keyframes boot-orch-slide {
  0% { transform: translateX(-110%); }
  100% { transform: translateX(320%); }
}

@keyframes boot-orch-log {
  from { opacity: 0; transform: translateX(-6px); }
  to { opacity: 1; transform: translateX(0); }
}

@media (max-width: 720px) {
  .boot-orch__log,
  .boot-orch__detail {
    display: none;
  }
}
</style>
