<template>
  <div class="twin-stage">
    <!-- 顶栏：标题 + 诚实标注 -->
    <header class="twin-header">
      <h2 class="twin-title">{{ t('deviceTwin.title', '设备 3D 数字孪生') }}</h2>
      <div class="twin-badges">
        <span class="badge badge-warn">{{ t('deviceTwin.teaching', '教学模拟 · 非实测') }}</span>
        <span v-if="!configured" class="badge badge-info">{{ t('deviceTwin.demo', '演示数据 · 未连接真实设备') }}</span>
      </div>
    </header>

    <!-- 视图状态切换（六态） -->
    <nav class="twin-nav" role="tablist" :aria-label="t('deviceTwin.viewLabel', '视图状态')">
      <button
        v-for="s in VIEW_STATES"
        :key="s"
        class="nav-btn"
        :class="{ active: snapshot.state === s }"
        :disabled="s === 'inspect'"
        role="tab"
        :aria-selected="snapshot.state === s"
        @click="goState(s)"
      >
        {{ t(`deviceTwin.state.${s}`, stateLabelFallback(s)) }}
      </button>
    </nav>

    <div class="twin-body">
      <!-- 画布区：3D 或 2D 降级 -->
      <div class="twin-canvas" :class="{ 'is-2d': quality === 'fallback' }">
        <template v-if="quality === 'fallback'">
          <!-- 2D 降级：真实 DOM 示意图，不是静态图 -->
          <div class="fallback-schematic" aria-hidden="true">
            <div class="fridge-outline" :class="{ open: snapshot.state === 'open' }">
              <div class="fridge-door upper" :style="doorStyle('upperDoor')"></div>
              <div class="fridge-door lower" :style="doorStyle('lowerDoor')"></div>
              <div class="fridge-body"></div>
            </div>
            <p class="fallback-note">{{ t('deviceTwin.fallback', '精简模式：WebGL 不可用，显示 2D 示意图') }}</p>
          </div>
        </template>
        <template v-else>
          <div ref="webglHost" class="webgl-host"></div>
        </template>

        <!-- 选中部件标签 -->
        <div v-if="selectedMeta" class="part-toast">
          {{ selectedMeta.name }} · {{ selectedMeta.english }}
        </div>
      </div>

      <!-- 右侧面板槽位：部件档案 + 热仿真读数 -->
      <aside class="twin-panel">
        <section class="panel-section">
          <h3 class="panel-heading">{{ t('deviceTwin.partInfo', '部件档案') }}</h3>
          <template v-if="selectedMeta">
            <div class="part-card">
              <div class="part-name">{{ selectedMeta.name }} <span class="part-en">{{ selectedMeta.english }}</span></div>
              <p class="part-intro">{{ selectedMeta.intro }}</p>
              <dl class="part-facts">
                <div v-for="([k, v], i) in selectedMeta.facts" :key="i" class="fact-row">
                  <dt>{{ k }}</dt><dd>{{ v }}</dd>
                </div>
              </dl>
            </div>
          </template>
          <p v-else class="panel-empty">{{ t('deviceTwin.selectHint', '点击部件查看档案') }}</p>
        </section>

        <section class="panel-section">
          <h3 class="panel-heading">{{ t('deviceTwin.thermal', '热仿真（教学）') }}</h3>
          <dl class="thermal-readout">
            <div class="fact-row">
              <dt>{{ t('deviceTwin.cabTemp', '箱内温度') }}</dt>
              <dd>{{ thermal.T_cab.toFixed(1) }}°C</dd>
            </div>
            <div class="fact-row">
              <dt>{{ t('deviceTwin.condTemp', '冷凝器温度') }}</dt>
              <dd>{{ thermal.T_cond.toFixed(1) }}°C</dd>
            </div>
            <div class="fact-row">
              <dt>{{ t('deviceTwin.compressor', '压缩机') }}</dt>
              <dd>{{ thermal.compressorOn ? t('deviceTwin.on', '运行') : t('deviceTwin.off', '停机') }}</dd>
            </div>
            <div class="fact-row">
              <dt>{{ t('deviceTwin.energy', '累计耗电') }}</dt>
              <dd>{{ thermal.cumulative.energyWh.toFixed(2) }} Wh</dd>
            </div>
          </dl>
        </section>
      </aside>
    </div>
  </div>
</template>

<script setup lang="ts">
import { computed, onMounted, onBeforeUnmount, ref, shallowRef } from 'vue'
import { useI18n } from 'vue-i18n'
import { VIEW_STATES, createViewStateMachine, type DeviceViewState, type ViewStateSnapshot } from './model/ViewStateMachine'
import { createKinematicsController, type KinematicsState } from './model/KinematicsController'
import { createLumpedThermalModel, type ThermalState } from './sim/LumpedThermalModel'
import { MetadataCatalog } from './model/MetadataCatalog'
import type { PartMetadata } from './model/PartNode'
import { FRIDGE_ROOT } from './devices/fridge/FridgeModel'
import { detectWebGL } from '../pet/detectPetRuntime'
import { DeviceTwinScene, type TwinQuality } from './scene/DeviceTwinScene'

const props = defineProps<{
  /** 是否已连接真实设备 RPC（false = 演示模式） */
  configured?: boolean
}>()

const { t } = useI18n()
const configured = computed(() => Boolean(props.configured))

// ── 状态机 ──
const vsm = createViewStateMachine()
const snapshot = shallowRef<ViewStateSnapshot>(vsm.snapshot())
const kinematics = createKinematicsController()
const kinState = shallowRef<KinematicsState>(kinematics.snapshot())

// ── 部件档案（MetadataCatalog 从 FridgeModel 装配树索引全部节点） ──
const catalog = new MetadataCatalog(FRIDGE_ROOT)
const selectedMeta = ref<PartMetadata | null>(null)

// ── 热仿真 ──
const thermalModel = createLumpedThermalModel()
const thermal = shallowRef<ThermalState>(thermalModel.snapshot())

// ── WebGL 探测（同步在 setup 阶段跑，与 IslandView 同范式） ──
// 首渲染就要走对分支：若先 fallback 再在 onMounted 里 flip，v-if/v-else 切换与挂载竞态，
// webglHost ref 还没建出来就 mountScene() → 静默 return → canvas 0 个。
type Quality = TwinQuality | 'fallback'

function detectQuality(): Quality {
  if (typeof window === 'undefined' || typeof navigator === 'undefined') return 'fallback'
  const gl = detectWebGL()
  if (gl === 'none') return 'fallback'
  if (gl === 'webgl1') return 'low'
  const cores = navigator.hardwareConcurrency > 0 ? navigator.hardwareConcurrency : 4
  return cores >= 4 ? 'high' : 'medium'
}

const quality = ref<Quality>(detectQuality())

function stateLabelFallback(s: DeviceViewState): string {
  const map: Record<DeviceViewState, string> = {
    product: '整机', open: '开合', explode: '爆炸',
    section: '剖面', cycle: '循环', inspect: '特写',
  }
  return map[s] ?? s
}

function goState(to: DeviceViewState): void {
  if (to === 'inspect') return
  // 进爆炸/剖面前联锁自动复位
  if (to === 'explode' || to === 'section') {
    kinematics.prepareForExplode()
  } else {
    kinematics.unlock()
  }
  const r = vsm.requestTransition(to)
  if (r.ok) snapshot.value = vsm.snapshot()
}

function doorStyle(group: 'upperDoor' | 'lowerDoor') {
  const k = kinState.value.doors[group]
  return { transform: `translateX(${k * 30}%) rotateY(${k * 20}deg)` }
}

// ── 3D 场景实例 ──
const webglHost = ref<HTMLDivElement | null>(null)
let scene: DeviceTwinScene | null = null

function handleSelect(partId: string | null): void {
  if (!partId) {
    selectedMeta.value = null
    return
  }
  const meta = catalog.get(partId)
  selectedMeta.value = meta
  scene?.select(partId)
}

function mountScene(): void {
  if (!webglHost.value || quality.value === 'fallback') return
  try {
    scene = new DeviceTwinScene({
      container: webglHost.value,
      quality: quality.value,
      onSelect: handleSelect,
      onContextLost: handleContextLost,
    })
  } catch (err) {
    // WebGL 构造失败（驱动崩溃/上下文丢失）→ 诚实降级到 2D，不白屏
    console.warn('[device-twin] WebGL init failed, falling back to 2D:', err)
    scene?.dispose()
    scene = null
    quality.value = 'fallback'
  }
}

function handleContextLost(): void {
  scene?.dispose()
  scene = null
  quality.value = 'fallback'
}

// ── 帧循环：驱动状态机 + 热仿真 + 喂给 3D 场景 ──
let rafId = 0
let lastTime = 0
function frame(now: number): void {
  rafId = requestAnimationFrame(frame)
  const dt = Math.min(0.05, (now - lastTime) / 1000 || 0.016)
  lastTime = now
  vsm.tick(dt)
  kinState.value = kinematics.tick(dt)
  snapshot.value = vsm.snapshot()
  // 热仿真：按真实时间推进（教学模式 1s 物理时间 ≈ 1s 墙钟）
  thermal.value = thermalModel.step(dt)
  // 把缓动后的运动状态喂给 three 场景
  scene?.setFrameState({
    state: snapshot.value.state,
    progress: snapshot.value.progress,
    kinematics: kinState.value,
  })
}

onMounted(() => {
  // quality 已在 setup 同步探测，首渲染即走对分支，webglHost ref 此时已就位
  mountScene()
  lastTime = performance.now()
  rafId = requestAnimationFrame(frame)
})

onBeforeUnmount(() => {
  cancelAnimationFrame(rafId)
  scene?.dispose()
  scene = null
})
</script>

<style scoped>
.twin-stage {
  display: flex;
  flex-direction: column;
  height: 100%;
  background: var(--bg-surface);
  color: var(--text);
  font-family: var(--font-sans);
}

.twin-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: var(--sp-3) var(--sp-4);
  border-bottom: 1px solid var(--border);
}

.twin-title {
  font-size: var(--fs-lg);
  font-weight: var(--fw-eyebrow);
  letter-spacing: var(--label-track);
}

.twin-badges { display: flex; gap: var(--sp-2); }

.badge {
  padding: var(--sp-1) var(--sp-2);
  border-radius: var(--radius-pill);
  font-size: var(--fs-xs);
  font-weight: var(--fw-quiet);
}
.badge-warn {
  background: var(--warn-fill);
  color: var(--warn);
}
.badge-info {
  background: var(--info-fill);
  color: var(--info);
}

.twin-nav {
  display: flex;
  gap: var(--sp-1);
  padding: var(--sp-2) var(--sp-4);
  border-bottom: 1px solid var(--border);
}

.nav-btn {
  padding: var(--sp-2) var(--sp-3);
  border: 1px solid var(--border);
  border-radius: var(--radius-control);
  background: var(--bg-hover);
  color: var(--text-muted);
  cursor: pointer;
  transition: var(--dur-base) var(--ease-standard);
  font-size: var(--fs-sm);
}
.nav-btn:hover { background: var(--bg-elevated); }
.nav-btn.active {
  background: var(--accent);
  color: var(--accent-foreground);
  border-color: var(--accent);
}
.nav-btn:disabled { opacity: var(--state-disabled-opacity); cursor: not-allowed; }

.twin-body {
  display: grid;
  grid-template-columns: 1fr 280px;
  flex: 1;
  min-height: 0;
}

.twin-canvas {
  position: relative;
  min-height: 320px;
  background: var(--bg);
}
.webgl-host { width: 100%; height: 100%; }

.fallback-schematic {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: var(--sp-4);
  height: 100%;
}

.fridge-outline {
  position: relative;
  width: 120px;
  height: 280px;
}
.fridge-body {
  position: absolute;
  inset: 0;
  border: 2px solid var(--text-dim);
  border-radius: var(--radius-sm);
  background: var(--bg-hover);
}
.fridge-door {
  position: absolute;
  left: 0;
  width: 100%;
  border: 2px solid var(--text-muted);
  border-radius: var(--radius-sm);
  background: var(--bg-elevated);
  transition: transform var(--dur-enter) var(--ease-out);
  transform-origin: left center;
}
.fridge-door.upper { top: 0; height: 58%; }
.fridge-door.lower { bottom: 0; height: 40%; }

.fallback-note {
  font-size: var(--fs-xs);
  color: var(--text-dim);
}

.part-toast {
  position: absolute;
  bottom: var(--sp-3);
  left: var(--sp-3);
  padding: var(--sp-2) var(--sp-3);
  background: var(--bg-elevated);
  border: 1px solid var(--border);
  border-radius: var(--radius-card);
  font-size: var(--fs-sm);
}

.twin-panel {
  border-left: 1px solid var(--border);
  padding: var(--sp-3);
  overflow-y: auto;
  display: flex;
  flex-direction: column;
  gap: var(--sp-4);
}

.panel-heading {
  font-size: var(--fs-xs);
  font-weight: var(--fw-eyebrow);
  letter-spacing: var(--eyebrow-track);
  text-transform: uppercase;
  color: var(--text-dim);
  margin-bottom: var(--sp-2);
}

.part-card {
  padding: var(--sp-3);
  background: var(--bg-hover);
  border-radius: var(--radius-card);
}
.part-name { font-size: var(--fs-md); font-weight: 600; }
.part-en { font-size: var(--fs-xs); color: var(--text-dim); margin-left: var(--sp-1); }
.part-intro {
  font-size: var(--fs-sm);
  color: var(--text-muted);
  margin-top: var(--sp-2);
  line-height: 1.5;
}
.part-facts { margin-top: var(--sp-2); }
.fact-row {
  display: flex;
  justify-content: space-between;
  padding: var(--sp-1) 0;
  font-size: var(--fs-sm);
}
.fact-row dt { color: var(--text-dim); }
.fact-row dd { color: var(--text); font-family: var(--font-mono); }

.panel-empty {
  font-size: var(--fs-sm);
  color: var(--text-dim);
}

.thermal-readout {
  padding: var(--sp-2);
  background: var(--bg-hover);
  border-radius: var(--radius-card);
}
</style>
