<script setup lang="ts">
/**
 * 岛屿工作空间 —— 智能体运行的 3D 可视化工作台。
 *
 * 数据流（单向、可解释）：
 *   RPC 事件流 → useIslandRuntime（归约）→ buildIslandWorld（纯函数：实体 + 道路）
 *   → computed(world) → Island3D.setWorld（按 id 增量 diff）→ 逐帧 computeIslandFrame（纯函数）
 *
 * 诚实降级：无 WebGL / 上下文丢失 / 规格非法 → 同一份 world 数据换成 2D 示意图并明确标注，
 * 绝不假装 3D；未连接网关时岛屿静止并显示离线胶囊，不编造运行状态。
 */
import { computed, onBeforeUnmount, onMounted, ref, shallowRef, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { useRoute, useRouter } from 'vue-router'
import { useAppStore } from '@/stores/app'
import {
  ISLAND_KIND_META,
  formatDuration,
  statusFromStep,
  type IslandActivityStatus,
  type IslandThemeRole,
} from '@/components/island/islandContract'
import { getIslandSpec, listIslandSpecs, validateIslandSpec } from '@/components/island/islandSpec'
import { buildIslandWorld } from '@/components/island/islandWorld'
import { buildConstructionEntities, constructionDayRange, sampleConstructionPlots } from '@/components/island/constructionWorld'
import { STAGE_LABEL_KEY } from '@/components/island/buildingGrowth'
import { keyboardCameraCommand } from '@/components/island/islandSim'
import { detectIslandRuntime, renderTierOf } from '@/components/island/detectIslandRuntime'
import { useIslandRuntime } from '@/components/island/useIslandRuntime'
import { Island3D, readIslandThemeTokens, type IslandLabelAnchor } from '@/components/island/Island3D'
import { requestIslandWorkbenchOpen } from '@/workbench/islandItems'

/** compact = 工作台面板形态：隐藏页头/图例/操作提示，只保留画布与必要控制 */
defineProps<{ compact?: boolean }>()

const { t } = useI18n()
const route = useRoute()
const router = useRouter()
const app = useAppStore()

/** 渲染档位（一次性探测；fallback 直接走 2D，不创建 WebGL 上下文） */
const runtime = detectIslandRuntime()
const specOptions = listIslandSpecs()
const specId = ref(specOptions[0]?.id ?? 'verdant')
const model = computed(() => getIslandSpec(specId.value))
/** 双保险：模块层已校验内置规格，这里仍挡一次（坏规格拒绝渲染而非带病启动） */
const specErrors = computed(() => validateIslandSpec(model.value))

const canvasRef = ref<HTMLElement | null>(null)
const island = shallowRef<Island3D | null>(null)
const mode = ref<'3d' | '2d'>(runtime.quality === 'fallback' ? '2d' : '3d')
const notice = ref<'none' | 'fallback' | 'context-lost' | 'spec-invalid'>(
  runtime.quality === 'fallback' ? 'fallback' : 'none',
)

const sessionKey = computed(() => (typeof route.query.session === 'string' ? route.query.session : ''))
const { state: runtimeState, connected } = useIslandRuntime({ sessionKey: () => sessionKey.value })

// 施工地块：纯数据驱动（constructionPlots.sample.json）。时间轴回放 = State-From-Time；
// deviceProgress 由设备事件喂入（接入=开工 / 完成=封顶），缺省 {} 即纯时间轴演示。
const constructionPlots = sampleConstructionPlots()
const constructionRange = constructionDayRange(constructionPlots)
const constructionDay = ref(12)
const deviceProgress = ref<Record<string, number>>({})
const constructionEntities = computed(() =>
  buildConstructionEntities(constructionPlots, { day: constructionDay.value, deviceProgress: deviceProgress.value }),
)
const world = computed(() => {
  const w = buildIslandWorld(runtimeState.value, model.value)
  return {
    ...w,
    entities: [...w.entities, ...constructionEntities.value],
    counts: { ...w.counts, construction: constructionEntities.value.length },
  }
})

const selectedId = ref<string | null>(null)
const hoveredId = ref<string | null>(null)
const labels = shallowRef<IslandLabelAnchor[]>([])
const autoDayNight = ref(true)
const dayProgress = ref(0.3)

const selected = computed(() => world.value.entities.find((entity) => entity.id === selectedId.value) ?? null)

/** 每秒一跳的时钟：运行中的工具/步骤耗时实时走高（而不是选中那一刻的静态快照） */
const nowTick = ref(Date.now())
let tickTimer: ReturnType<typeof setInterval> | undefined

/** 详情面板派生数据：已结束取 endedAt，仍活动则取当前时刻 */
const selectedLive = computed(() => {
  const entity = selected.value
  const meta = entity?.meta
  if (!entity || meta?.startedAt === undefined) return { duration: '' }
  const end = meta.endedAt ?? (entity.status === 'active' ? nowTick.value : undefined)
  return { duration: end === undefined ? '' : formatDuration(end - meta.startedAt) }
})

/** 计划步骤的原始状态 → 视觉色调（与建筑灯火同一套口径） */
const stepTone = (status: string) => toneOf(statusFromStep(status))
const counts = computed(() =>
  (Object.keys(ISLAND_KIND_META) as (keyof typeof ISLAND_KIND_META)[])
    .map((kind) => ({ kind, labelKey: ISLAND_KIND_META[kind].labelKey, count: world.value.counts[kind] }))
    .filter((row) => row.count > 0),
)

const runKey = computed(() => {
  const status = runtimeState.value.runStatus
  if (status === 'running') return 'island.status.running'
  if (status === 'queued') return 'island.status.queued'
  if (status === 'failed' || status === 'timeout') return 'island.status.failed'
  return 'island.status.idle'
})

/** 状态 → 视觉色调（3D 灯光、HUD 胶囊、2D 地图共用同一口径） */
const STATUS_TONE: Record<IslandActivityStatus, IslandThemeRole | 'muted'> = {
  idle: 'muted',
  queued: 'warn',
  active: 'accent',
  success: 'ok',
  error: 'danger',
  waiting: 'info',
}
const toneOf = (status: IslandActivityStatus) => STATUS_TONE[status]

const STATUS_KEY: Record<IslandActivityStatus, string> = {
  idle: 'island.status.idle',
  queued: 'island.status.queued',
  active: 'island.status.running',
  success: 'island.status.success',
  error: 'island.status.error',
  waiting: 'island.status.waiting',
}

function titleOf(entityId: string): string {
  return world.value.entities.find((entity) => entity.id === entityId)?.title ?? entityId
}

/* ── 3D 生命周期 ── */

function mountIsland(): void {
  if (!canvasRef.value || island.value) return
  if (mode.value !== '3d') return
  if (specErrors.value.length > 0) {
    console.warn('[island] 规格非法，降级为 2D 示意图:', specErrors.value)
    mode.value = '2d'
    notice.value = 'spec-invalid'
    return
  }
  try {
    island.value = new Island3D({
      container: canvasRef.value,
      spec: model.value,
      quality: renderTierOf(runtime.quality),
      theme: readIslandThemeTokens(),
      onSelect: (id) => {
        selectedId.value = id
      },
      onHover: (id) => {
        hoveredId.value = id
      },
      onLabels: (anchors) => {
        labels.value = anchors
      },
      onContextLost: () => {
        // 诚实降级：上下文丢了就换 2D，并说明原因
        island.value?.dispose()
        island.value = null
        mode.value = '2d'
        notice.value = 'context-lost'
      },
      // 让装饰树木/礁石避开工地（脚手架底下不种树）
      constructionKeepOuts: constructionPlots.map((plot) => ({
        x: plot.x,
        z: plot.z,
        radius: plot.building.footprint + 0.9,
      })),
    })
    island.value.setWorld(world.value)
    island.value.select(selectedId.value)
  } catch (error) {
    console.warn('[island] 3D 初始化失败，降级为 2D 示意图:', error)
    island.value = null
    mode.value = '2d'
    notice.value = 'fallback'
  }
}

watch(world, (next) => {
  island.value?.setWorld(next)
})

watch(selectedId, (id) => {
  island.value?.select(id)
})

watch(() => app.resolvedTheme, () => {
  island.value?.setTheme(readIslandThemeTokens())
})

watch(autoDayNight, (enabled) => {
  island.value?.setAutoDayNight(enabled)
})

watch(dayProgress, (progress) => {
  if (!autoDayNight.value) island.value?.setDayProgress(progress)
})

// 换皮肤（热插拔）：几何依赖 Spec，因此重建场景
watch(specId, () => {
  island.value?.dispose()
  island.value = null
  notice.value = runtime.quality === 'fallback' ? 'fallback' : 'none'
  mode.value = runtime.quality === 'fallback' ? '2d' : '3d'
  mountIsland()
})

onMounted(() => {
  mountIsland()
  tickTimer = setInterval(() => {
    nowTick.value = Date.now()
  }, 1000)
})

onBeforeUnmount(() => {
  if (tickTimer !== undefined) clearInterval(tickTimer)
  island.value?.dispose()
  island.value = null
})

/* ── 交互 ── */

function resetView(): void {
  island.value?.resetView()
}

function focusSelected(): void {
  island.value?.focusEntity(selectedId.value ?? 'core')
}

function pickLabel(anchor: IslandLabelAnchor): void {
  selectedId.value = anchor.id
  island.value?.focusEntity(anchor.id)
}

/**
 * 键盘导航：方向键绕行 / Shift+方向键平移 / `+`-` 缩放 / `0` 复位 / `Esc` 关详情。
 * 键位映射是纯函数（islandSim.keyboardCameraCommand），这里只做「命令 → 场景」的转发。
 */
function onStageKeydown(event: KeyboardEvent): void {
  if (event.key === 'Escape') {
    if (selectedId.value) {
      selectedId.value = null
      event.preventDefault()
    }
    return
  }
  const cmd = keyboardCameraCommand(event.key, event.shiftKey)
  // 无 3D 实例（2D 降级）时不吞键：让方向键回落到页面滚动，别制造「按了没反应」
  if (!cmd || !island.value) return
  // 阻止方向键滚动页面，键盘操作才「留得住」
  event.preventDefault()
  island.value.applyCameraCommand(cmd)
}

function openChat(): void {
  if (!sessionKey.value) return
  router.push({ path: '/chat', query: { session: sessionKey.value } })
}

/** 在工作台侧栏打开同一座岛（跟随当前会话），便于边聊边看 */
function openInWorkbench(): void {
  requestIslandWorkbenchOpen(sessionKey.value || undefined)
}

/* ── 2D 降级地图（与 3D 同一份 world / 同一套 plotFor 坐标）── */

const mapViewBox = computed(() => {
  const extent = model.value.water.radius
  return `${-extent} ${-extent} ${extent * 2} ${extent * 2}`
})

const mapIslandRadius = computed(() => model.value.terrain.radius + model.value.terrain.beachWidth)

const mapPlots = computed(() =>
  world.value.entities.map((entity) => {
    const template = model.value.buildings[entity.kind]
    return {
      id: entity.id,
      title: entity.title,
      tone: toneOf(entity.status),
      x: entity.plot.x,
      z: entity.plot.z,
      size: Math.max(template.width, template.depth) * 0.5,
    }
  }),
)
</script>

<template>
  <section class="island-view" :class="{ 'island-view--compact': compact }">
    <header v-if="!compact" class="island-bar">
      <div class="island-bar__titles">
        <h1 class="island-bar__title">{{ t('island.title') }}</h1>
        <p class="island-bar__subtitle">{{ t('island.subtitle') }}</p>
      </div>
      <div class="island-bar__pills">
        <span class="island-pill" :data-tone="toneOf(runtimeState.runStatus === 'running' ? 'active' : runtimeState.runStatus === 'failed' ? 'error' : 'idle')">
          {{ t(runKey) }}
        </span>
        <span class="island-pill" data-tone="muted">{{ t(runtime.labelKey) }}</span>
        <span v-if="!connected" class="island-pill" data-tone="warn">{{ t('island.status.offline') }}</span>
        <button type="button" class="island-pill island-pill--action" @click="openInWorkbench">
          {{ t('island.controls.openInWorkbench') }}
        </button>
      </div>
    </header>

    <!-- tabindex 让 3D 舞台可聚焦：键盘导航（方向键/缩放/复位/Esc）才有落点，
         同时给键盘与读屏用户一条不依赖指针的操作路径 -->
    <div
      class="island-stage"
      tabindex="0"
      role="application"
      :aria-label="t('island.title')"
      @keydown="onStageKeydown"
    >
      <div v-show="mode === '3d'" ref="canvasRef" class="island-canvas" />
      <div v-if="mode === '3d'" class="island-atmosphere" aria-hidden="true" />

      <!-- 2D 诚实降级：同一份运行数据的示意图（不是 3D 的伪装） -->
      <svg
        v-if="mode === '2d'"
        class="island-map"
        :viewBox="mapViewBox"
        role="img"
        :aria-label="t('island.title')"
      >
        <rect
          class="island-map__water"
          :x="-model.water.radius"
          :y="-model.water.radius"
          :width="model.water.radius * 2"
          :height="model.water.radius * 2"
        />
        <circle class="island-map__island" cx="0" cy="0" :r="mapIslandRadius" />
        <g
          v-for="plot in mapPlots"
          :key="plot.id"
          class="island-map__plot"
          :data-tone="plot.tone"
          @click="pickLabel({ id: plot.id, x: 0, y: 0, visible: true, selected: false })"
        >
          <circle :cx="plot.x" :cy="plot.z" :r="plot.size" />
          <text :x="plot.x" :y="plot.z + plot.size + 3">{{ plot.title }}</text>
        </g>
      </svg>

      <!-- 3D 标签层：跟随真实投影位置（HTML 文本比 Sprite 更清晰、可点击） -->
      <div v-if="mode === '3d'" class="island-labels">
        <button
          v-for="anchor in labels"
          v-show="anchor.visible"
          :key="anchor.id"
          type="button"
          class="island-label"
          :class="{ 'island-label--active': anchor.selected || anchor.id === hoveredId }"
          :style="{ left: anchor.x + 'px', top: anchor.y + 'px' }"
          @click="pickLabel(anchor)"
        >
          {{ titleOf(anchor.id) }}
        </button>
      </div>

      <aside v-if="selected" class="island-detail" :data-tone="toneOf(selected.status)">
        <button type="button" class="island-detail__close" :aria-label="t('island.detail.close')" @click="selectedId = null">×</button>
        <p class="island-detail__kind">{{ t(ISLAND_KIND_META[selected.kind].labelKey) }}</p>
        <h2 class="island-detail__title">{{ selected.title }}</h2>
        <dl class="island-detail__rows">
          <div>
            <dt>{{ t('island.detail.status') }}</dt>
            <dd>{{ t(STATUS_KEY[selected.status]) }}</dd>
          </div>
          <div v-if="selectedLive.duration">
            <dt>{{ t('island.detail.duration') }}</dt>
            <dd>{{ selectedLive.duration }}</dd>
          </div>
          <div v-if="selected.progress !== undefined">
            <dt>{{ t('island.detail.progress') }}</dt>
            <dd>{{ Math.round(selected.progress * 100) }}%</dd>
          </div>
          <template v-if="selected.kind === 'workshop'">
            <div v-if="selected.meta?.toolFlavor">
              <dt>{{ t('island.detail.flavor') }}</dt>
              <dd>{{ t(`island.flavor.${selected.meta.toolFlavor}`) }}</dd>
            </div>
            <div v-if="selected.meta?.toolId">
              <dt>{{ t('island.detail.toolId') }}</dt>
              <dd class="island-detail__mono">{{ selected.meta.toolId }}</dd>
            </div>
          </template>
          <template v-if="selected.kind === 'outpost'">
            <div v-if="selected.meta?.childKey">
              <dt>{{ t('island.detail.childKey') }}</dt>
              <dd class="island-detail__mono">{{ selected.meta.childKey }}</dd>
            </div>
            <div v-if="selected.meta?.rawStatus">
              <dt>{{ t('island.detail.source') }}</dt>
              <dd>{{ selected.meta.rawStatus }}</dd>
            </div>
          </template>
          <template v-if="selected.kind === 'depot'">
            <div v-if="selected.meta?.artifactKind">
              <dt>{{ t('island.detail.artifactKind') }}</dt>
              <dd>{{ selected.meta.artifactKind }}</dd>
            </div>
            <div v-if="selected.meta?.artifactId">
              <dt>{{ t('island.detail.artifactId') }}</dt>
              <dd class="island-detail__mono">{{ selected.meta.artifactId }}</dd>
            </div>
          </template>
          <template v-if="selected.kind === 'beacon' && selected.meta?.ensembleMembers?.length">
            <div>
              <dt>{{ t('island.detail.members') }}</dt>
              <dd>{{ selected.meta.ensembleMembers.filter((m) => m.done).length }}/{{ selected.meta.ensembleMembers.length }}</dd>
            </div>
          </template>
          <template v-if="selected.kind === 'athenaeum'">
            <div v-if="selected.meta?.recallQuery">
              <dt>{{ t('island.detail.recallQuery') }}</dt>
              <dd>{{ selected.meta?.recallQuery }}</dd>
            </div>
            <div v-if="selected.meta?.recallHits !== undefined">
              <dt>{{ t('island.detail.recallHits') }}</dt>
              <dd>{{ selected.meta?.recallHits }}</dd>
            </div>
            <div v-if="selected.meta?.recallTopScore !== undefined">
              <dt>{{ t('island.detail.recallTopScore') }}</dt>
              <dd>{{ selected.meta?.recallTopScore?.toFixed(3) }}</dd>
            </div>
            <div v-if="selected.meta?.reinforced !== undefined">
              <dt>{{ t('island.detail.reinforced') }}</dt>
              <dd>{{ selected.meta?.reinforced }}</dd>
            </div>
            <div v-if="selected.meta?.memoryIds?.length">
              <dt>{{ t('island.detail.memoryIds') }}</dt>
              <dd class="island-detail__mono">{{ selected.meta?.memoryIds?.join(', ') }}</dd>
            </div>
          </template>
          <template v-if="selected.kind === 'construction' && selected.construction">
            <div>
              <dt>{{ t('island.detail.stage') }}</dt>
              <dd>{{ t(STAGE_LABEL_KEY[selected.construction.snapshot.stage]) }}</dd>
            </div>
            <div>
              <dt>{{ t('island.detail.floors') }}</dt>
              <dd>{{ selected.construction.snapshot.visibleFloors }}/{{ selected.construction.config.building.totalFloors }}</dd>
            </div>
            <div>
              <dt>{{ t('island.construction.day') }}</dt>
              <dd>Day {{ constructionDay }}</dd>
            </div>
          </template>
          <template v-if="selected.kind === 'core'">
            <div v-if="selected.meta?.taskTitle">
              <dt>{{ t('island.detail.taskTitle') }}</dt>
              <dd>{{ selected.meta.taskTitle }}</dd>
            </div>
            <div v-if="selected.meta?.strategy">
              <dt>{{ t('island.detail.strategy') }}</dt>
              <dd class="island-detail__mono">{{ selected.meta.strategy }}</dd>
            </div>
            <div>
              <dt>{{ t('island.detail.sessionKey') }}</dt>
              <dd class="island-detail__mono">{{ selected.meta?.sessionKey || '—' }}</dd>
            </div>
          </template>
          <div>
            <dt>{{ t('island.detail.source') }}</dt>
            <dd class="island-detail__mono">{{ selected.detail }}</dd>
          </div>
        </dl>

        <section v-if="selected.kind === 'pier' && selected.meta?.steps?.length" class="island-detail__section">
          <p class="island-detail__section-title">{{ t('island.detail.steps') }}</p>
          <ol class="island-detail__steps">
            <li
              v-for="step in selected.meta.steps"
              :key="step.id"
              class="island-detail__step"
              :data-tone="stepTone(step.status)"
            >
              <span class="island-detail__step-dot" />
              <span class="island-detail__step-title">{{ step.title }}</span>
            </li>
          </ol>
        </section>

        <button
          v-if="sessionKey && selected.kind === 'core'"
          type="button"
          class="island-detail__action"
          @click="openChat"
        >
          {{ t('island.detail.openChat') }}
        </button>
        <button type="button" class="island-detail__action island-detail__action--ghost" @click="focusSelected">
          {{ t('island.controls.focus') }}
        </button>
      </aside>
      <p v-else class="island-detail-empty">{{ t('island.detail.empty') }}</p>

      <div v-if="!compact" class="island-legend">
        <p class="island-legend__title">{{ t('island.legend.title') }}</p>
        <ul class="island-legend__list">
          <li v-for="row in counts" :key="row.kind" class="island-legend__row">
            <span class="island-legend__dot" />
            <span>{{ t(row.labelKey) }}</span>
            <span class="island-legend__count">{{ row.count }}</span>
          </li>
        </ul>
        <p v-if="world.plan" class="island-legend__plan">
          {{ t('island.legend.plan') }} {{ world.plan.done }}/{{ world.plan.total }}
        </p>
      </div>

      <div class="island-controls">
        <button type="button" class="island-controls__btn" @click="resetView">{{ t('island.controls.reset') }}</button>
        <button type="button" class="island-controls__btn" @click="focusSelected">{{ t('island.controls.focus') }}</button>
        <label class="island-controls__toggle">
          <input v-model="autoDayNight" type="checkbox" />
          <span>{{ t('island.controls.autoDayNight') }}</span>
        </label>
        <label v-if="!autoDayNight" class="island-controls__slider">
          <span>{{ t('island.controls.daylight') }}</span>
          <input v-model.number="dayProgress" type="range" min="0" max="1" step="0.01" />
        </label>
        <label class="island-controls__slider">
          <span>{{ t('island.construction.timeline') }}</span>
          <input v-model.number="constructionDay" type="range" :min="constructionRange.min" :max="constructionRange.max" step="0.5" />
        </label>
        <label class="island-controls__toggle">
          <span>{{ t('island.controls.skin') }}</span>
          <select v-model="specId" class="island-controls__select">
            <option v-for="option in specOptions" :key="option.id" :value="option.id">{{ option.label }}</option>
          </select>
        </label>
        <p v-if="!compact" class="island-controls__hint">{{ t('island.controls.hint') }}</p>
      </div>

      <p v-if="notice !== 'none'" class="island-notice">
        {{
          notice === 'context-lost'
            ? t('island.quality.contextLost')
            : notice === 'spec-invalid'
              ? t('island.quality.specInvalid')
              : t('island.quality.fallbackNote')
        }}
      </p>
      <p v-else-if="world.empty" class="island-notice island-notice--calm">{{ t('island.empty.hint') }}</p>
    </div>
  </section>
</template>

<style scoped>
.island-view {
  display: flex;
  flex-direction: column;
  height: 100%;
  min-height: 0;
  background: var(--bg);
  color: var(--text);
}

.island-bar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  padding: 12px 18px;
  border-bottom: 1px solid var(--border);
  background: var(--bg-surface);
}

.island-bar__title {
  margin: 0;
  font-size: 16px;
  font-weight: 600;
}

.island-bar__subtitle {
  margin: 2px 0 0;
  font-size: 12px;
  color: var(--text-muted);
}

.island-bar__pills {
  display: flex;
  gap: 8px;
  flex-wrap: wrap;
}

.island-pill {
  padding: 3px 10px;
  border: 1px solid var(--border);
  border-radius: var(--radius-pill);
  font-size: 12px;
  background: var(--bg-surface-2);
}

.island-pill[data-tone='accent'] { color: var(--accent); border-color: var(--accent); }
.island-pill[data-tone='ok'] { color: var(--ok); border-color: var(--ok); }
.island-pill[data-tone='warn'] { color: var(--warn); border-color: var(--warn); }
.island-pill[data-tone='danger'] { color: var(--danger); border-color: var(--danger); }
.island-pill[data-tone='info'] { color: var(--info); border-color: var(--info); }
.island-pill[data-tone='muted'] { color: var(--text-muted); }

.island-pill--action {
  color: var(--accent);
  border-color: var(--accent);
  cursor: pointer;
  transition: background var(--dur-fast) var(--ease-standard);
}

.island-pill--action:hover {
  background: var(--bg-hover);
}

.island-stage {
  position: relative;
  flex: 1;
  min-height: 0;
  overflow: hidden;
}

/* 键盘焦点环：只在键盘操作时出现（:focus-visible），指针点击不打扰视觉。
   用主题的 --focus-ring（与其他视图一致），不写裸色值。 */
.island-stage:focus {
  outline: none;
}

.island-stage:focus-visible {
  box-shadow: var(--focus-ring);
  outline: none;
}

.island-canvas {
  position: absolute;
  inset: 0;
}

.island-map {
  display: block;
  width: 100%;
  height: 100%;
}

.island-map__water {
  fill: var(--bg-surface-2);
}

.island-map__island {
  fill: color-mix(in srgb, var(--ok) 26%, var(--bg-surface));
  stroke: var(--border);
  stroke-width: 0.4;
}

.island-map__plot {
  cursor: pointer;
}

.island-map__plot circle {
  fill: currentColor;
  opacity: 0.85;
}

.island-map__plot text {
  fill: var(--text-muted);
  font-size: 2.6px;
  text-anchor: middle;
}

.island-map__plot[data-tone='accent'] { color: var(--accent); }
.island-map__plot[data-tone='ok'] { color: var(--ok); }
.island-map__plot[data-tone='warn'] { color: var(--warn); }
.island-map__plot[data-tone='danger'] { color: var(--danger); }
.island-map__plot[data-tone='info'] { color: var(--info); }
.island-map__plot[data-tone='muted'] { color: var(--text-dim); }

.island-labels {
  position: absolute;
  inset: 0;
  pointer-events: none;
}

.island-label {
  position: absolute;
  transform: translate(-50%, -100%);
  padding: 2px 8px;
  border: 1px solid var(--border);
  border-radius: var(--radius-pill);
  background: var(--bg-elevated);
  color: var(--text-muted);
  font-size: 11px;
  line-height: 1.4;
  pointer-events: auto;
  cursor: pointer;
  transition: color var(--dur-fast) var(--ease-standard), border-color var(--dur-fast) var(--ease-standard);
}

.island-label:hover,
.island-label--active {
  color: var(--accent);
  border-color: var(--accent);
}

.island-detail,
.island-legend,
.island-controls {
  position: absolute;
  background: var(--bg-elevated);
  border: 1px solid var(--border);
  border-radius: var(--radius-card);
  padding: 12px 14px;
}

.island-detail {
  top: 14px;
  left: 14px;
  width: 240px;
}

.island-detail__kind {
  margin: 0;
  font-size: 11px;
  color: var(--text-dim);
}

.island-detail__title {
  margin: 4px 0 10px;
  font-size: 14px;
  font-weight: 600;
  word-break: break-all;
}

.island-detail__rows {
  display: grid;
  gap: 6px;
  margin: 0;
  font-size: 12px;
}

.island-detail__rows div {
  display: flex;
  justify-content: space-between;
  gap: 8px;
}

.island-detail__rows dt {
  color: var(--text-dim);
}

.island-detail__rows dd {
  margin: 0;
  color: var(--text);
  text-align: right;
  word-break: break-all;
}

.island-detail[data-tone='accent'] .island-detail__kind { color: var(--accent); }
.island-detail[data-tone='danger'] .island-detail__kind { color: var(--danger); }
.island-detail[data-tone='ok'] .island-detail__kind { color: var(--ok); }
.island-detail[data-tone='warn'] .island-detail__kind { color: var(--warn); }
.island-detail[data-tone='info'] .island-detail__kind { color: var(--info); }

.island-detail__close {
  position: absolute;
  top: 6px;
  right: 8px;
  padding: 2px 6px;
  border: none;
  border-radius: var(--radius-control);
  background: transparent;
  color: var(--text-dim);
  font-size: 15px;
  line-height: 1;
  cursor: pointer;
}

.island-detail__close:hover {
  color: var(--text);
  background: var(--bg-hover);
}

.island-detail__mono {
  font-size: 11px;
  letter-spacing: 0.01em;
  color: var(--text-muted);
}

.island-detail__section {
  margin-top: 10px;
  padding-top: 10px;
  border-top: 1px solid var(--hairline);
}

.island-detail__section-title {
  margin: 0 0 6px;
  font-size: 11px;
  color: var(--text-dim);
}

.island-detail__steps {
  display: grid;
  gap: 4px;
  margin: 0;
  padding: 0;
  max-height: 132px;
  overflow: auto;
  list-style: none;
  font-size: 12px;
}

.island-detail__step {
  display: flex;
  align-items: center;
  gap: 6px;
  color: var(--text-muted);
}

.island-detail__step-dot {
  flex: 0 0 auto;
  width: 6px;
  height: 6px;
  border-radius: var(--radius-pill);
  background: currentColor;
}

.island-detail__step-title {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.island-detail__step[data-tone='accent'] { color: var(--accent); }
.island-detail__step[data-tone='ok'] { color: var(--ok); }
.island-detail__step[data-tone='warn'] { color: var(--warn); }
.island-detail__step[data-tone='danger'] { color: var(--danger); }
.island-detail__step[data-tone='info'] { color: var(--info); }
.island-detail__step[data-tone='muted'] { color: var(--text-dim); }

.island-detail-empty {
  position: absolute;
  top: 14px;
  left: 14px;
  margin: 0;
  padding: 6px 12px;
  border: 1px dashed var(--border);
  border-radius: var(--radius-pill);
  background: var(--bg-elevated);
  color: var(--text-dim);
  font-size: 12px;
}

/* 画布氛围层：四周轻微暗角，把注意力收拢到岛上（保留雾感，不遮挡交互） */
.island-atmosphere {
  position: absolute;
  inset: 0;
  pointer-events: none;
  background:
    radial-gradient(125% 95% at 50% 46%, transparent 52%, color-mix(in srgb, var(--bg) 62%, transparent) 100%);
}

.island-detail__action {
  margin-top: 12px;
  width: 100%;
  padding: 6px 10px;
  border: 1px solid var(--accent);
  border-radius: var(--radius-control);
  background: transparent;
  color: var(--accent);
  font-size: 12px;
  cursor: pointer;
}

.island-detail__action--ghost {
  margin-top: 6px;
  border-color: var(--border);
  color: var(--text-muted);
}

.island-legend {
  top: 14px;
  right: 14px;
  min-width: 168px;
}

.island-legend__title {
  margin: 0 0 8px;
  font-size: 11px;
  color: var(--text-dim);
}

.island-legend__list {
  display: grid;
  gap: 4px;
  margin: 0;
  padding: 0;
  list-style: none;
  font-size: 12px;
}

.island-legend__row {
  display: flex;
  align-items: center;
  gap: 8px;
  color: var(--text-muted);
}

.island-legend__dot {
  width: 7px;
  height: 7px;
  border-radius: var(--radius-pill);
  background: var(--accent);
}

.island-legend__count {
  margin-left: auto;
  color: var(--text);
  font-variant-numeric: tabular-nums;
}

.island-legend__plan {
  margin: 8px 0 0;
  font-size: 12px;
  color: var(--accent);
}

.island-controls {
  bottom: 14px;
  left: 14px;
  right: 14px;
  display: flex;
  align-items: center;
  gap: 10px;
  flex-wrap: wrap;
  font-size: 12px;
}

.island-controls__btn {
  padding: 5px 12px;
  border: 1px solid var(--border);
  border-radius: var(--radius-control);
  background: var(--bg-surface-2);
  color: var(--text);
  font-size: 12px;
  cursor: pointer;
}

.island-controls__toggle,
.island-controls__slider {
  display: flex;
  align-items: center;
  gap: 6px;
  color: var(--text-muted);
}

.island-controls__select {
  padding: 3px 8px;
  border: 1px solid var(--border);
  border-radius: var(--radius-control);
  background: var(--bg-surface-2);
  color: var(--text);
  font-size: 12px;
}

.island-controls__hint {
  margin: 0 0 0 auto;
  color: var(--text-dim);
}

.island-notice {
  position: absolute;
  bottom: 68px;
  left: 50%;
  transform: translateX(-50%);
  margin: 0;
  padding: 6px 14px;
  border: 1px solid var(--warn);
  border-radius: var(--radius-pill);
  background: var(--bg-elevated);
  color: var(--warn);
  font-size: 12px;
}

.island-notice--calm {
  border-color: var(--border);
  color: var(--text-muted);
}

/* 工作台面板形态：窄容器下压缩内边距与浮层尺寸 */
.island-view--compact .island-detail {
  width: 200px;
  padding: 10px 12px;
}

.island-view--compact .island-controls {
  bottom: 10px;
  left: 10px;
  right: 10px;
  padding: 10px 12px;
  gap: 8px;
}

.island-view--compact .island-notice {
  bottom: 62px;
  max-width: calc(100% - 24px);
  text-align: center;
}

@media (max-width: 720px) {
  .island-legend {
    display: none;
  }

  .island-detail {
    width: 200px;
  }
}
</style>