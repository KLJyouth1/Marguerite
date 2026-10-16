<script setup lang="ts">
/**
 * 3D 开放世界容器（P0 主交付物）。
 *
 * 职责：
 * 1. WebGL 可用 → 挂载 IslandWorld3DScene（纯 three），WASD + PointerLock 鼠标漫游；
 *    WebGL 不可用 → 降级 IslandGame2D.vue（2D SVG fallback，props world+dayPhase）。
 * 2. 四档模型档位切换器 UI（tierProfile 消费方）：粒子预算/生成节律/NPC 对话深度/
 *    极光强度/时间流速全部随档位缩放。
 * 3. 游戏↔Agent 双向绑定（useWorld3dBridge）：拾取道具 → chat.send 委托；运行时事件
 *    → 场景脉冲可视化。HUD 诚实显示连接状态与待发队列。
 *
 * 为什么程序化几何体而非外部素材？见 world3d/terrain.ts 文件头。
 */
import { computed, onBeforeUnmount, onMounted, ref, shallowRef, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { detectWebGL } from '@/components/pet/detectPetRuntime'
import { IslandWorld3DScene, type FocusedProp, type PickupEvent, type World3dHud } from './world3d/scene'
import { useWorld3dBridge } from './world3d/useWorld3dBridge'
import {
  tierProfile,
  tierForSnapshot,
  type TierProfile,
  type World3dTier,
} from './world3d/tierProfile'
import { npcReplyFor, pickupGlowStrength, type NpcDialogueLine } from './world3d/agentBinding'
import { propExpReward, type PropKind, type PropSourceCounts } from './world3d/propsSpec'
import type { EngineSnapshot } from './game/gameLogic'
import IslandGame2D from './game/IslandGame2D.vue'
import type { WorldSpec } from './game/worldSpec'
import { loadIslandGlbAssets } from './world3d/glbAssets'
import {
  glbHotReload,
  type GlbIssue,
  type GlbLoadResult,
  type GlbLoadStatus,
} from './world3d/glbHotReload'

const { t } = useI18n()

const props = defineProps<{
  world: WorldSpec
  dayPhase: 'day' | 'night'
  engine: EngineSnapshot
  /** 真实运行时来源（工具/产物/记忆/委托/编队）；无数据 → 岛上无道具（诚实） */
  propSources: PropSourceCounts
}>()

const emit = defineEmits<{
  'pickup-complete': [event: PickupEvent, exp: number]
  'tier-change': [tier: World3dTier]
}>()

// ── WebGL 探测（复用桌宠探测器；不可用 → 2D fallback）──────────────
const gl = detectWebGL()
const use3d = gl !== 'none'
const reduceMotion =
  typeof window !== 'undefined' &&
  typeof window.matchMedia === 'function' &&
  window.matchMedia('(prefers-reduced-motion: reduce)').matches

// ── 四档模型档位（tierProfile 消费）─────────────────────────────────
const TIERS: World3dTier[] = ['flagship', 'efficient', 'sprint', 'omni']
const currentTier = ref<World3dTier>(tierForSnapshot(props.engine))
const profile = computed<TierProfile>(() => tierProfile(currentTier.value))

// 引擎快照变化 → 自动跟随档位（用户未手动覆盖时）
watch(
  () => props.engine,
  (e) => {
    currentTier.value = tierForSnapshot(e)
  },
  { deep: true },
)

function selectTier(tier: World3dTier): void {
  currentTier.value = tier
  scene.value?.setProfile(tierProfile(tier))
  emit('tier-change', tier)
}

// ── 3D 场景挂载 ─────────────────────────────────────────────────────
const canvasRef = ref<HTMLCanvasElement | null>(null)
const scene = shallowRef<IslandWorld3DScene | null>(null)
const hud = ref<World3dHud>({ fps: 60, drawCalls: 0, focus: null, position: [0, 0, 0] })

function mountScene(): void {
  if (!use3d || !canvasRef.value) return
  const s = new IslandWorld3DScene(canvasRef.value, profile.value, reduceMotion)
  s.setHudListener((h: World3dHud) => (hud.value = h))
  s.setPickupListener((e: PickupEvent) => handlePickup(e))
  s.setPropSources(props.propSources, `world-${props.world.revision}-${props.world.seed}`)
  scene.value = s
}

watch(
  () => props.propSources,
  (src) => {
    scene.value?.setPropSources(src, `world-${props.world.revision}-${props.world.seed}`)
  },
  { deep: true },
)

// ── GLB 资产状态（程序化几何 / 已热加载 N 件 / 校验失败）───────────────
// 诚实显示：本机无 Blender → GLB 缺失 → 场景继续用程序化几何；绝不假装
// 「AI 已产出资产」。状态取自 glbHotReload 单例（scene 与容器共用同一资产表）。
interface GlbState {
  status: GlbLoadStatus
  revision: number
  loaded: number
  errors: GlbIssue[]
}
const glbState = ref<GlbState>({
  status: glbHotReload.getStatus(),
  revision: glbHotReload.getRevision(),
  loaded: 0,
  errors: glbHotReload.getLastErrors(),
})

function syncGlbState(result?: GlbLoadResult): void {
  glbState.value = {
    status: glbHotReload.getStatus(),
    revision: glbHotReload.getRevision(),
    loaded: result?.loaded.length ?? glbState.value.loaded,
    errors: glbHotReload.getLastErrors(),
  }
}

const glbBadgeText = computed<string>(() => {
  switch (glbState.value.status) {
    case 'ready':
      return t('islandGame.glb.statusLoaded', {
        count: glbState.value.loaded,
        rev: glbState.value.revision,
      })
    case 'failed':
      return t('islandGame.glb.statusFailed')
    default:
      return t('islandGame.glb.statusProcedural')
  }
})

const glbBadgeClass = computed<string>(() =>
  glbState.value.status === 'ready' ? 'chip--ok' : 'chip--muted',
)

const glbRevisionTitle = computed<string>(
  () => `${t('islandGame.glb.revisionLabel')}: ${glbState.value.revision}`,
)

// ── 双向绑定（游戏 ↔ Agent）──────────────────────────────────────────
// ⚠ 顺序不可颠倒：useWorld3dBridge 会在 setup 期间**立刻**调用 sessionKey()
// （useWorld3dBridge.ts:80 `ref(options.sessionKey?.() ?? null)`），所以
// bridgeSessionKey 必须先声明——否则触发 TDZ 报错
// 「Cannot access 'bridgeSessionKey' before initialization」，整个组件 setup 崩掉。
const bridgeSessionKey = ref<string | null>(null)
const bridge = useWorld3dBridge({ sessionKey: () => bridgeSessionKey.value })

// Agent→游戏：场景脉冲（最近事件 → 场景特效）
watch(bridge.lastPulse, (pulse) => {
  if (pulse) scene.value?.applyPulse(pulse)
})

function handlePickup(e: PickupEvent): void {
  const exp = propExpReward(e.kind)
  bridge.dispatchPickup(
    { kind: e.kind, interaction: interactionOf(e.kind), name: e.name },
    currentTier.value,
  )
  emit('pickup-complete', e, exp)
}

/** 拾取道具 → 交互语义（propsSpec 契约：拾取类别 → 委托类别） */
function interactionOf(kind: PropKind): 'analyze' | 'explain' | 'insight' | 'quest' | 'brainstorm' {
  switch (kind) {
    case 'code_crystal': return 'analyze'
    case 'doc_scroll': return 'explain'
    case 'data_fruit': return 'insight'
    case 'quest_beacon': return 'quest'
    case 'inspiration_bloom': return 'brainstorm'
  }
}

// ── AI NPC 对话（档位深度驱动条数）────────────────────────────────
const npcTopic = ref('')
const npcLines = ref<NpcDialogueLine[]>([])
function talkToNpc(): void {
  npcLines.value = npcReplyFor(npcTopic.value || props.world.islandName, currentTier.value)
}

// ── 拾取特效强度（HUD 显示神力辉光）────────────────────────────────
const glowStrength = computed(() => pickupGlowStrength(currentTier.value))

// ── 生命周期 ─────────────────────────────────────────────────────────
onMounted(() => {
  mountScene()
  // GLB 热加载：fire-and-forget —— 加载/校验失败只更新状态、绝不阻断渲染
  // （本机无 Blender，GLB 尚未导出属预期状态，场景继续用程序化几何）。
  // scene 已订阅 glbHotReload，加载成功后会自动热替换道具几何。
  void loadIslandGlbAssets().then(syncGlbState, syncGlbState)
})
onBeforeUnmount(() => {
  scene.value?.dispose()
  scene.value = null
})

// ── PointerLock 漫游（点击画布锁定指针）────────────────────────────
function onCanvasClick(): void {
  scene.value?.requestPointerLock()
}

// 键盘 E 拾取提示（HUD 显示面向中的道具）
const focusedProp = computed<FocusedProp | null>(() => hud.value.focus)
</script>

<template>
  <div class="world3d">
    <!-- 3D 渲染（WebGL 可用） -->
    <div v-if="use3d" class="world3d__stage">
      <canvas
        ref="canvasRef"
        class="world3d__canvas"
        role="img"
        :aria-label="t('islandGame.world3d.canvasAria')"
        @click="onCanvasClick"
      ></canvas>

      <!-- HUD 悬浮层 -->
      <div class="world3d__hud">
        <div class="hud__stats">
          <span class="chip">{{ hud.fps }} fps · {{ hud.drawCalls }} dc</span>
          <span class="chip chip--muted">
            {{ t('islandGame.world3d.tierLabel') }}:
            {{ t('islandGame.world3d.tier.' + currentTier) }}
          </span>
          <span class="chip" :class="bridge.connected.value ? 'chip--ok' : 'chip--muted'">
            {{ bridge.connected.value ? t('islandGame.world3d.connected') : t('islandGame.world3d.offline') }}
            <template v-if="bridge.outbox.value.length > 0">
              · {{ bridge.outbox.value.length }} {{ t('islandGame.world3d.pending') }}
            </template>
          </span>
          <span class="chip" :class="glbBadgeClass" :title="glbRevisionTitle">
            {{ glbBadgeText }}
          </span>
          <span v-if="glbState.errors.length > 0" class="chip chip--muted">
            {{ t('islandGame.glb.errorsLabel') }}: {{ glbState.errors[0].code }}
          </span>
        </div>

        <!-- 面向道具提示（E 拾取） -->
        <div v-if="focusedProp" class="hud__focus">
          <span class="chip chip--accent">
            [E] {{ t('islandGame.world3d.pickup') }}:
            {{ t('islandGame.world3d.prop.' + focusedProp.kind + '.name') }}
            <small>({{ focusedProp.name }})</small>
          </span>
        </div>

        <!-- 神力辉光指示（拾取特效强度） -->
        <div class="hud__glow" :style="{ opacity: glowStrength }">
          <span class="chip chip--accent">✦ {{ t('islandGame.world3d.glow') }} {{ (glowStrength * 100).toFixed(0) }}%</span>
        </div>
      </div>

      <!-- 控制提示 -->
      <p class="world3d__controls" tabindex="0">
        {{ t('islandGame.world3d.controls') }}
      </p>
    </div>

    <!-- 2D fallback（WebGL 不可用） -->
    <div v-else class="world3d__fallback">
      <IslandGame2D :world="props.world" :day-phase="props.dayPhase" />
      <p class="world3d__note">{{ t('islandGame.world3d.fallbackNote') }}</p>
    </div>

    <!-- 四档档位切换器 -->
    <section class="world3d__tiers">
      <h3 class="tiers__title">{{ t('islandGame.world3d.tierLabel') }}</h3>
      <div class="tiers__grid">
        <button
          v-for="tier in TIERS"
          :key="tier"
          class="tier__btn"
          :class="{ 'tier__btn--active': currentTier === tier }"
          @click="selectTier(tier)"
        >
          <strong>{{ t('islandGame.world3d.tier.' + tier) }}</strong>
          <small>
            {{ tierProfile(tier).particleBudget }} {{ t('islandGame.world3d.particles') }} ·
            {{ tierProfile(tier).npcDialogueDepth }}× {{ t('islandGame.world3d.npcDepth') }}
          </small>
        </button>
      </div>
    </section>

    <!-- AI NPC 对话（档位驱动复杂度） -->
    <section class="world3d__npc">
      <h3 class="npc__title">{{ t('islandGame.world3d.npcTitle') }}</h3>
      <input
        v-model="npcTopic"
        class="npc__input"
        :placeholder="t('islandGame.world3d.npcPlaceholder')"
        @keyup.enter="talkToNpc"
      />
      <button class="btn" @click="talkToNpc">{{ t('islandGame.world3d.talk') }}</button>
      <div v-if="npcLines.length > 0" class="npc__dialogue">
        <p v-for="(line, i) in npcLines" :key="i" class="npc__line">
          <strong>{{ line.speaker }}:</strong> {{ line.text }}
        </p>
        <small class="npc__disclaimer">{{ t('islandGame.world3d.npcDisclaimer') }}</small>
      </div>
    </section>
  </div>
</template>

<style scoped>
.world3d {
  display: flex;
  flex-direction: column;
  gap: 12px;
}

.world3d__stage {
  position: relative;
  width: 100%;
  height: 480px;
  border-radius: var(--radius-card);
  overflow: hidden;
  background: var(--bg-elevated);
}

.world3d__canvas {
  width: 100%;
  height: 100%;
  cursor: crosshair;
}

.world3d__hud {
  position: absolute;
  inset: 0;
  pointer-events: none;
  padding: 12px;
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.hud__stats {
  display: flex;
  gap: 6px;
  flex-wrap: wrap;
}

.hud__focus {
  margin-top: auto;
}

.hud__glow {
  align-self: flex-end;
}

.world3d__controls {
  position: absolute;
  bottom: 8px;
  left: 12px;
  font-size: 12px;
  color: var(--text-secondary);
  background: var(--scrim);
  padding: 4px 8px;
  border-radius: var(--radius-xs);
}

.world3d__fallback {
  position: relative;
}

.world3d__note {
  margin-top: 8px;
  font-size: 13px;
  color: var(--text-secondary);
}

.world3d__tiers {
  padding: 12px;
  background: var(--bg-surface);
  border-radius: var(--radius-card);
}

.tiers__title {
  margin: 0 0 8px 0;
  font-size: 14px;
  font-weight: 600;
}

.tiers__grid {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(140px, 1fr));
  gap: 8px;
}

.tier__btn {
  display: flex;
  flex-direction: column;
  gap: 2px;
  padding: 10px;
  background: var(--bg-elevated);
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  cursor: pointer;
  text-align: left;
}

.tier__btn--active {
  border-color: var(--accent);
  background: var(--accent-deep);
}

.world3d__npc {
  padding: 12px;
  background: var(--bg-surface);
  border-radius: var(--radius-card);
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.npc__title {
  margin: 0;
  font-size: 14px;
  font-weight: 600;
}

.npc__input {
  padding: 8px;
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  background: var(--bg-elevated);
}

.npc__dialogue {
  display: flex;
  flex-direction: column;
  gap: 4px;
  padding: 8px;
  background: var(--bg-elevated);
  border-radius: var(--radius-sm);
}

.npc__line {
  margin: 0;
  font-size: 13px;
}

.npc__disclaimer {
  margin-top: 4px;
  font-size: 11px;
  color: var(--text-dim);
}
</style>
