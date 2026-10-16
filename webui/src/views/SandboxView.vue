<script setup lang="ts">
/**
 * 微缩沙盘 —— 闭环高铁运行图 + 动态对象状态卡（任务书 B）。
 *
 * 数据流：
 *   时间轴 simTime（秒）→ schedule.trainStateAt/nextTrainState（纯函数）
 *   → SandboxScene.setTime（three 渲染，State From Time）
 *   → describeTrainCard（纯函数）→ EntityStateCard（可复用）
 *
 * 诚实降级：无 WebGL / 上下文丢失 → 同一份 schedule 数据画 2D SVG 环线，不假装 3D。
 * 本视图不接路由（由协调者注册）；不改任何既有文件。
 */
import { computed, onBeforeUnmount, onMounted, ref, shallowRef, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import EntityStateCard, { type EntityCardTone } from '@/components/EntityStateCard.vue'
import { SandboxScene, type SandboxPick } from './sandbox/SandboxScene'
import {
  LOOP_LENGTH,
  LOOP_PERIOD,
  nextTrainState,
  trainStateAt,
  TRAIN_COUNT,
} from './sandbox/schedule'
import { describeTrainCard } from './sandbox/cardModel'

const { t } = useI18n()

const containerRef = ref<HTMLElement | null>(null)
const scene = shallowRef<SandboxScene | null>(null)
const mode = ref<'3d' | '2d'>('3d')
const notice = ref<'none' | 'fallback' | 'context-lost'>('none')

const simTime = ref(0)
const playing = ref(true)
const speed = ref(1)
const dayFactor = ref(1)
const rainOn = ref(false)
const selectedIndex = ref<number | null>(null)

let rafId = 0
let lastT = 0

/* ── 选中对象的状态卡 ── */
const selectedCard = computed(() => {
  if (selectedIndex.value === null) return null
  const now = trainStateAt(selectedIndex.value, simTime.value)
  const next = nextTrainState(selectedIndex.value, simTime.value)
  const copy = describeTrainCard(now, next)
  return {
    title: t('sandbox.train.label', { n: selectedIndex.value + 1 }),
    kindLabel: t(copy.kindLabelKey),
    nowLabel: t(copy.nowKey),
    nowDetail: copy.nowDetailKey ? t(copy.nowDetailKey) : undefined,
    nextLabel: t(copy.nextKey),
    nextDetail: copy.nextDetailKey ? t(copy.nextDetailKey) : undefined,
    tone: copy.tone as EntityCardTone,
    closeLabel: t('sandbox.detail.close'),
  }
})

/* ── 2D 降级：同一份 schedule 的示意图 ── */
const trainDots2d = computed(() =>
  Array.from({ length: TRAIN_COUNT }, (_, i) => {
    const s = trainStateAt(i, simTime.value)
    const angle = (s.distance / LOOP_LENGTH) * Math.PI * 2
    // 椭圆环线示意
    return {
      id: i,
      x: 160 + 120 * Math.sin(angle),
      y: 110 + 70 * Math.cos(angle),
      dwell: s.phase === 'dwell',
    }
  }),
)

/* ── 生命周期 ── */
function mountScene(): void {
  if (!containerRef.value || scene.value) return
  try {
    scene.value = new SandboxScene({
      container: containerRef.value,
      onSelect: (p: SandboxPick | null) => {
        selectedIndex.value = p ? p.index : null
        scene.value?.select(p ? p.index : null)
      },
      onContextLost: () => {
        scene.value?.dispose()
        scene.value = null
        mode.value = '2d'
        notice.value = 'context-lost'
      },
    })
    scene.value.setDayFactor(dayFactor.value)
    scene.value.setRain(rainOn.value)
    scene.value.setTime(simTime.value)
    scene.value.select(selectedIndex.value)
  } catch (error) {
    console.warn('[sandbox] 3D 初始化失败，降级 2D 示意图:', error)
    scene.value = null
    mode.value = '2d'
    notice.value = 'fallback'
  }
}

function advance(now: number): void {
  rafId = requestAnimationFrame(advance)
  const dt = Math.min((now - lastT) / 1000, 0.1)
  lastT = now
  if (playing.value) {
    simTime.value = (simTime.value + dt * speed.value) % LOOP_PERIOD
  }
  scene.value?.setTime(simTime.value)
  if (selectedIndex.value !== null) {
    // 光环跟随列车
    scene.value?.select(selectedIndex.value)
  }
}

onMounted(() => {
  mountScene()
  lastT = performance.now()
  rafId = requestAnimationFrame(advance)
})

onBeforeUnmount(() => {
  cancelAnimationFrame(rafId)
  scene.value?.dispose()
  scene.value = null
})

watch(dayFactor, (v) => scene.value?.setDayFactor(v))
watch(rainOn, (v) => scene.value?.setRain(v))

/* ── 交互 ── */
function onScrub(): void {
  // 时间轴拖到哪，状态就在哪（State From Time）
  scene.value?.setTime(simTime.value)
}

function resetView(): void {
  // 相机复位由场景内部状态管理；这里仅重置时间到起点
  simTime.value = 0
  scene.value?.setTime(0)
}

const speedOptions = [0.5, 1, 2, 4]
</script>

<template>
  <section class="sb-view">
    <header class="sb-bar">
      <div class="sb-bar__titles">
        <h1 class="sb-bar__title">{{ t('sandbox.title') }}</h1>
        <p class="sb-bar__subtitle">{{ t('sandbox.subtitle') }}</p>
      </div>
    </header>

    <div class="sb-stage" tabindex="0" role="application" :aria-label="t('sandbox.title')">
      <div v-show="mode === '3d'" ref="containerRef" class="sb-canvas" />

      <!-- 2D 诚实降级：同一运行图的 SVG 环线 -->
      <svg v-if="mode === '2d'" class="sb-map" viewBox="0 0 320 220" role="img" :aria-label="t('sandbox.title')">
        <ellipse cx="160" cy="110" rx="120" ry="70" fill="none" stroke="var(--border)" stroke-width="2" />
        <circle cx="160" cy="40" r="5" fill="var(--accent)" />
        <circle cx="160" cy="180" r="5" fill="var(--ok)" />
        <circle
          v-for="d in trainDots2d"
          :key="d.id"
          :cx="d.x"
          :cy="d.y"
          r="6"
          :fill="d.dwell ? 'var(--ok)' : 'var(--accent)'"
        />
      </svg>

      <EntityStateCard
        v-if="selectedCard"
        class="sb-card"
        :title="selectedCard.title"
        :kind-label="selectedCard.kindLabel"
        :now-label="selectedCard.nowLabel"
        :now-detail="selectedCard.nowDetail"
        :next-label="selectedCard.nextLabel"
        :next-detail="selectedCard.nextDetail"
        :tone="selectedCard.tone"
        :close-label="selectedCard.closeLabel"
        @close="selectedIndex = null; scene?.select(null)"
      />
      <p v-else class="sb-empty">{{ t('sandbox.empty.pickTrain') }}</p>

      <div class="sb-controls">
        <button type="button" class="sb-btn" @click="playing = !playing">
          {{ playing ? t('sandbox.controls.pause') : t('sandbox.controls.play') }}
        </button>
        <label class="sb-field">
          <span>{{ t('sandbox.controls.speed') }}</span>
          <select v-model.number="speed" class="sb-select">
            <option v-for="s in speedOptions" :key="s" :value="s">{{ s }}×</option>
          </select>
        </label>
        <label class="sb-field sb-field--grow">
          <span>{{ t('sandbox.controls.timeline') }}</span>
          <input v-model.number="simTime" type="range" min="0" :max="LOOP_PERIOD" step="0.5" @input="onScrub" />
        </label>
        <label class="sb-field">
          <span>{{ t('sandbox.controls.dayNight') }}</span>
          <input v-model.number="dayFactor" type="range" min="0" max="1" step="0.01" />
        </label>
        <label class="sb-field">
          <input v-model="rainOn" type="checkbox" />
          <span>{{ t('sandbox.controls.rain') }}</span>
        </label>
        <button type="button" class="sb-btn" @click="resetView">{{ t('sandbox.controls.resetView') }}</button>
      </div>

      <p v-if="notice !== 'none'" class="sb-notice">
        {{ notice === 'context-lost' ? t('sandbox.quality.contextLost') : t('sandbox.quality.fallbackNote') }}
      </p>
    </div>
  </section>
</template>

<style scoped>
.sb-view {
  display: flex;
  flex-direction: column;
  height: 100%;
  min-height: 0;
  background: var(--bg);
  color: var(--text);
}

.sb-bar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--sp-3);
  padding: var(--sp-3) var(--sp-4);
  border-bottom: 1px solid var(--border);
  background: var(--bg-surface);
}

.sb-bar__title {
  margin: 0;
  font-size: var(--fs-lg);
  font-weight: 600;
}

.sb-bar__subtitle {
  margin: 2px 0 0;
  font-size: var(--fs-xs);
  color: var(--text-muted);
}

.sb-stage {
  position: relative;
  flex: 1;
  min-height: 0;
  overflow: hidden;
}

.sb-stage:focus { outline: none; }
.sb-stage:focus-visible { box-shadow: var(--focus-ring); outline: none; }

.sb-canvas {
  position: absolute;
  inset: 0;
}

.sb-map {
  display: block;
  width: 100%;
  height: 100%;
}

.sb-card {
  position: absolute;
  top: var(--sp-3);
  left: var(--sp-3);
  z-index: 2;
}

.sb-empty {
  position: absolute;
  top: var(--sp-3);
  left: var(--sp-3);
  margin: 0;
  padding: var(--sp-1) var(--sp-3);
  border: 1px dashed var(--border);
  border-radius: var(--radius-pill);
  background: var(--bg-elevated);
  color: var(--text-dim);
  font-size: var(--fs-xs);
}

.sb-controls {
  position: absolute;
  bottom: var(--sp-3);
  left: var(--sp-3);
  right: var(--sp-3);
  display: flex;
  align-items: center;
  gap: var(--sp-3);
  flex-wrap: wrap;
  padding: var(--sp-2) var(--sp-3);
  background: var(--bg-elevated);
  border: 1px solid var(--border);
  border-radius: var(--radius-card);
  font-size: var(--fs-xs);
}

.sb-btn {
  padding: var(--sp-1) var(--sp-3);
  border: 1px solid var(--border);
  border-radius: var(--radius-control);
  background: var(--bg-surface-2);
  color: var(--text);
  font-size: var(--fs-xs);
  cursor: pointer;
}

.sb-btn:hover { background: var(--bg-hover); }

.sb-field {
  display: flex;
  align-items: center;
  gap: var(--sp-2);
  color: var(--text-muted);
}

.sb-field--grow { flex: 1 1 200px; }

.sb-field input[type='range'] { width: 160px; }

.sb-select {
  padding: 2px var(--sp-1);
  border: 1px solid var(--border);
  border-radius: var(--radius-control);
  background: var(--bg-surface-2);
  color: var(--text);
  font-size: var(--fs-xs);
}

.sb-notice {
  position: absolute;
  bottom: 72px;
  left: 50%;
  transform: translateX(-50%);
  margin: 0;
  padding: var(--sp-1) var(--sp-3);
  border: 1px solid var(--warn);
  border-radius: var(--radius-pill);
  background: var(--bg-elevated);
  color: var(--warn);
  font-size: var(--fs-xs);
}

@media (max-width: 720px) {
  .sb-field--grow input[type='range'] { width: 110px; }
}
</style>
