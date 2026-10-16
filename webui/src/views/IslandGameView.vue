<script setup lang="ts">
/**
 * 岛屿开放世界 MVP（路由 /control/island-game）。
 *
 * 它是一个绑定真实引擎状态的「空间兼游戏」：
 * - 3D 岛（秘境花园 + 雏菊圣殿 + 委托板）可 WASD 漫游，WebGL 不可用自动降级 2D SVG；
 * - 模式符文/星塔点灯透出真实 RPC（mode.get / mode.spec），方法缺失优雅降级；
 * - 委托状态机 + EXP/称号 + 4 精灵编队 + 昼夜跟随系统时间；
 * - 描述文本 → WorldSpec → 校验 → 热加载可回滚。
 *
 * 存档（委托/EXP/称号/编队/采集）合法持久化在 localStorage；引擎状态永不造假。
 */
import { computed, nextTick, onBeforeUnmount, onMounted, ref, shallowRef, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { detectWebGL } from '@/components/pet/detectPetRuntime'
import { IslandGameScene, type IslandGameTier, type SceneHud } from '@/components/island/game/IslandGameScene'
import { useEngineBridge } from '@/components/island/game/useEngineBridge'
import {
  advanceQuest,
  completeQuest,
  dayPhaseOf,
  defaultSave,
  deriveWeather,
  loadSave,
  partyCap,
  persistSave,
  type GameSave,
  type PartyClass,
  PARTY_CLASSES,
} from '@/components/island/game/gameLogic'
import {
  defaultWorldSpec,
  specFromDescription,
  validateWorldSpec,
  type WorldSpec,
} from '@/components/island/game/worldSpec'
import IslandGame2D from '@/components/island/game/IslandGame2D.vue'
import IslandWorld3D from '@/components/island/IslandWorld3D.vue'
import { useIslandRuntime } from '@/components/island/useIslandRuntime'
import { propSourceCountsOf } from '@/components/island/world3d/propsSpec'
import type { PickupEvent } from '@/components/island/world3d/scene'

const { t } = useI18n()

// ── 运行能力探测（复用桌宠 detectWebGL）──────────────────────────────
const gl = detectWebGL()
const reduceMotion =
  typeof window !== 'undefined' &&
  typeof window.matchMedia === 'function' &&
  window.matchMedia('(prefers-reduced-motion: reduce)').matches
const tier: IslandGameTier = gl === 'webgl2' ? (reduceMotion ? 'medium' : 'high') : gl === 'webgl1' ? 'low' : 'low'
const use3d = gl !== 'none'

/**
 * 舞台视图模式：3D 开放世界（world3d，three.js）↔ 经典岛屿（本视图原生的
 * IslandGameScene）。两者并存而非替换——经典视图绑定模式符文/编队/世界热加载，
 * 3D 开放世界绑定四档档位 + GLB 资产热加载 + 道具委托；WebGL 不可用时只剩 2D SVG。
 *
 * 默认进 3D 开放世界：用户目标是「岛屿变成真正的空间兼游戏」，经典视图保留为
 * 一键可回退的对照实现（不删代码、不改行为）。
 */
const viewMode = ref<'world3d' | 'classic'>(use3d ? 'world3d' : 'classic')

// ── 岛屿运行时（真实 RPC 事件归约 → 3D 岛上道具来源）────────────────
// 诚实性红线：道具存在与否完全由真实运行数据决定（无数据则岛上无道具）。
const { state: islandRuntime } = useIslandRuntime()
const propSources = computed(() => propSourceCountsOf(islandRuntime.value))

// ── 游戏存档（客户端合法持久化）─────────────────────────────────────
const save = ref<GameSave>(loadSave() ?? defaultSave())
watch(save, (s) => persistSave(s), { deep: true })

/** 3D 世界拾取道具 → 记入本视图的 EXP（与经典视图共用同一份存档） */
function onPickupComplete(_event: PickupEvent, exp: number): void {
  if (!Number.isFinite(exp) || exp <= 0) return
  save.value = { ...save.value, exp: save.value.exp + exp }
}

// ── 世界（可热加载/回滚）────────────────────────────────────────────
const world = ref<WorldSpec>(defaultWorldSpec())
const previousWorld = ref<WorldSpec | null>(null)
const draft = ref('')
const genErrors = ref<string[]>([])

// ── 引擎状态桥接（真实 RPC）─────────────────────────────────────────
const { snapshot, refresh, lastError } = useEngineBridge()

// ── 昼夜（跟随系统真实时间）─────────────────────────────────────────
const phase = ref(dayPhaseOf())
const weather = computed(() => deriveWeather(snapshot.value, new Date()))

// ── 经典岛屿场景挂载（仅 classic 模式；3D 开放世界由 IslandWorld3D 自管）──
const canvasRef = ref<HTMLCanvasElement | null>(null)
const scene = shallowRef<IslandGameScene | null>(null)
const hud = ref<SceneHud>({ fps: 60, drawCalls: 0 })

let clockTimer: ReturnType<typeof setInterval> | undefined

function mountScene(): void {
  if (!use3d || scene.value || !canvasRef.value) return
  const s = new IslandGameScene(canvasRef.value, tier, reduceMotion)
  s.setHudListener((h: SceneHud) => (hud.value = h))
  s.setWorld(world.value)
  s.setDayPhase(phase.value)
  s.setModeKind(snapshot.value.modeKind)
  s.setParty(assignedPartyCount.value)
  scene.value = s
}

function disposeScene(): void {
  scene.value?.dispose()
  scene.value = null
}

function syncScene(): void {
  const s = scene.value
  if (!s) return
  s.setModeKind(snapshot.value.modeKind)
  s.setParty(assignedPartyCount.value)
  s.setDayPhase(phase.value)
}

// 切换视图模式：进 classic 才创建 IslandGameCanvas（nextTick 等 canvas 出 DOM），
// 离开即 dispose——两种模式绝不同时持有渲染上下文（WebGL 上下文有数量上限）。
watch(viewMode, async (mode) => {
  if (mode === 'classic' && use3d) {
    await nextTick()
    mountScene()
  } else {
    disposeScene()
  }
})

// ── 委托操作 ───────────────────────────────────────────────────────
function advance(id: string): void {
  save.value = advanceQuest(save.value, id)
}
function turnIn(id: string): void {
  const res = completeQuest(save.value, id)
  save.value = res.save
}

// ── 编队（4 精灵位）────────────────────────────────────────────────
function cycleElf(slot: number): void {
  const party = [...save.value.party]
  const current = party[slot]
  if (!current) {
    party[slot] = { slot, cls: PARTY_CLASSES[0] as PartyClass }
  } else {
    const idx = PARTY_CLASSES.indexOf(current.cls)
    party[slot] = { slot, cls: PARTY_CLASSES[(idx + 1) % PARTY_CLASSES.length] as PartyClass }
  }
  save.value = { ...save.value, party, revision: save.value.revision + 1 }
}
const assignedPartyCount = computed(() => save.value.party.filter(Boolean).length)
const cap = computed(() => partyCap(snapshot.value))

// ── 世界生成闭环 ────────────────────────────────────────────────────
function generate(): void {
  const candidate = specFromDescription(draft.value)
  const errs = validateWorldSpec(candidate)
  if (errs.length) {
    genErrors.value = errs // 校验失败：不热加载，保留当前世界
    return
  }
  genErrors.value = []
  previousWorld.value = world.value
  world.value = candidate
  scene.value?.setWorld(candidate)
}
function rollback(): void {
  if (!previousWorld.value) return
  world.value = previousWorld.value
  scene.value?.setWorld(world.value)
  previousWorld.value = null
}

const titlesUnlocked = computed(() => save.value.titles)

onMounted(() => {
  // 默认进 3D 开放世界时不创建经典场景（省一份 WebGL 上下文与渲染循环）。
  if (viewMode.value === 'classic') mountScene()
  void refresh()
  clockTimer = setInterval(() => {
    phase.value = dayPhaseOf()
    void refresh()
  }, 15000)
})
onBeforeUnmount(() => {
  if (clockTimer) clearInterval(clockTimer)
  scene.value?.dispose()
  scene.value = null
})

watch([snapshot, phase], syncScene)
</script>

<template>
  <main class="island-game" :aria-busy="false">
    <header class="island-game__head">
      <div>
        <h1 class="island-game__title">{{ t('islandGame.title') }}</h1>
        <p class="island-game__sub">{{ t('islandGame.subtitle') }}</p>
      </div>
      <div class="island-game__chips">
        <span class="chip" :class="snapshot.connected ? 'chip--ok' : 'chip--muted'">
          {{ snapshot.connected ? t('islandGame.engine.online') : t('islandGame.engine.offline') }}
        </span>
        <span class="chip">{{ t('islandGame.engine.mode') }}: {{ snapshot.modeKind ?? t('islandGame.engine.unknown') }}</span>
        <span class="chip">{{ t('islandGame.engine.shards') }}: {{ snapshot.maxSwarmShards ?? t('islandGame.engine.unknown') }}</span>
        <span class="chip chip--muted">{{ t('islandGame.weather.' + weather) }} · {{ t('islandGame.day.' + phase) }}</span>
        <span class="chip chip--muted" v-if="use3d && viewMode === 'classic'">{{ hud.fps }} fps · {{ t('islandGame.tier.' + tier) }}</span>
      </div>
    </header>

    <section class="island-game__body">
      <div class="island-game__stage">
        <!-- 3D 开放世界（three.js）：四档档位 + GLB 资产热加载 + 道具委托 -->
        <IslandWorld3D
          v-if="viewMode === 'world3d' && use3d"
          :world="world"
          :day-phase="phase"
          :engine="snapshot"
          :prop-sources="propSources"
          @pickup-complete="onPickupComplete"
        />
        <canvas v-else-if="use3d" ref="canvasRef" class="island-game__canvas" role="img" :aria-label="t('islandGame.canvasAria')"></canvas>
        <IslandGame2D v-else :world="world" :day-phase="phase" />
        <div v-if="use3d" class="island-game__viewmode" role="group" :aria-label="t('islandGame.viewMode.aria')">
          <button
            class="btn"
            :class="{ 'btn--primary': viewMode === 'world3d' }"
            :aria-pressed="viewMode === 'world3d'"
            @click="viewMode = 'world3d'"
          >
            {{ t('islandGame.viewMode.world3d') }}
          </button>
          <button
            class="btn"
            :class="{ 'btn--primary': viewMode === 'classic' }"
            :aria-pressed="viewMode === 'classic'"
            @click="viewMode = 'classic'"
          >
            {{ t('islandGame.viewMode.classic') }}
          </button>
        </div>
        <p class="island-game__controls" tabindex="0">{{ t('islandGame.controls') }}</p>
      </div>

      <aside class="island-game__panel">
        <h2 class="panel__title">{{ t('islandGame.quest.title') }}</h2>
        <p class="panel__line">EXP {{ save.exp }} · Lv {{ save.level }}</p>
        <ul class="quest__list">
          <li v-for="q in save.quests" :key="q.id" class="quest__item">
            <div class="quest__row">
              <span class="quest__label">{{ q.label }}</span>
              <span class="chip chip--muted">{{ t('islandGame.quest.status.' + q.status) }}</span>
            </div>
            <div class="quest__row quest__row--actions">
              <span class="quest__exp">{{ q.expReward }} EXP</span>
              <button v-if="q.status !== 'completed'" class="btn" @click="advance(q.id)">
                {{ q.status === 'verifying' ? t('islandGame.quest.turnIn') : t('islandGame.quest.advance') }}
              </button>
              <button v-if="q.status === 'verifying'" class="btn btn--primary" @click="turnIn(q.id)">
                {{ t('islandGame.quest.complete') }}
              </button>
            </div>
          </li>
        </ul>

        <h2 class="panel__title">{{ t('islandGame.titles.title') }}</h2>
        <p class="panel__line">
          <template v-if="titlesUnlocked.length">
            <span v-for="id in titlesUnlocked" :key="id" class="chip chip--accent">{{ t('islandGame.titles.' + id) }}</span>
          </template>
          <template v-else>{{ t('islandGame.titles.none') }}</template>
        </p>

        <h2 class="panel__title">{{ t('islandGame.party.title') }} ({{ assignedPartyCount }}/{{ cap }})</h2>
        <div class="party__slots" role="group" :aria-label="t('islandGame.party.aria')">
          <button
            v-for="slot in 4"
            :key="slot"
            class="party__slot"
            :aria-label="t('islandGame.party.slotAria', { n: slot })"
            @click="cycleElf(slot - 1)"
          >
            {{ save.party[slot - 1] ? t('islandGame.party.' + save.party[slot - 1]!.cls) : '＋' }}
          </button>
        </div>
      </aside>

      <aside class="island-game__panel">
        <h2 class="panel__title">{{ t('islandGame.worldgen.title') }}</h2>
        <label class="worldgen__label" for="island-gen">{{ t('islandGame.worldgen.label') }}</label>
        <textarea id="island-gen" v-model="draft" class="worldgen__input" rows="3" :placeholder="t('islandGame.worldgen.placeholder')"></textarea>
        <div class="worldgen__actions">
          <button class="btn btn--primary" @click="generate">{{ t('islandGame.worldgen.apply') }}</button>
          <button class="btn" :disabled="!previousWorld" @click="rollback">{{ t('islandGame.worldgen.rollback') }}</button>
        </div>
        <p class="worldgen__name">{{ world.islandName }} · {{ t('islandGame.theme.' + world.theme) }}</p>
        <ul v-if="genErrors.length" class="worldgen__errors" role="alert">
          <li v-for="(e, i) in genErrors" :key="i">{{ e }}</li>
        </ul>
        <p class="panel__line panel__line--muted">{{ lastError ? t('islandGame.engine.offline') : t('islandGame.worldgen.hint') }}</p>
      </aside>
    </section>
  </main>
</template>

<style scoped>
.island-game {
  display: flex;
  flex-direction: column;
  gap: 0.75rem;
  padding: 1rem;
  height: 100%;
  box-sizing: border-box;
  overflow: auto;
}
.island-game__head {
  display: flex;
  flex-wrap: wrap;
  justify-content: space-between;
  gap: 0.5rem;
  align-items: flex-end;
}
.island-game__title {
  margin: 0;
  font-size: 1.2rem;
  color: var(--text);
}
.island-game__sub {
  margin: 0.15rem 0 0;
  color: var(--text-muted);
  font-size: 0.8rem;
}
.island-game__chips {
  display: flex;
  flex-wrap: wrap;
  gap: 0.35rem;
}
.chip {
  font-size: 0.72rem;
  padding: 0.15rem 0.5rem;
  border-radius: 999px;
  background: var(--bg-surface);
  color: var(--text-muted);
  border: 1px solid var(--border);
}
.chip--ok { color: var(--ok); }
.chip--muted { opacity: 0.8; }
.chip--accent { color: var(--accent); }

.island-game__body {
  display: grid;
  grid-template-columns: minmax(0, 2fr) minmax(220px, 1fr);
  gap: 0.75rem;
  flex: 1;
  min-height: 0;
}
.island-game__stage {
  position: relative;
  border-radius: var(--radius-card);
  overflow: hidden;
  border: 1px solid var(--border);
  background: var(--bg-surface);
  min-height: 360px;
}
.island-game__canvas {
  width: 100%;
  height: 100%;
  display: block;
}
.island-game__controls {
  position: absolute;
  left: 0.5rem;
  bottom: 0.4rem;
  margin: 0;
  font-size: 0.72rem;
  color: var(--text);
  background: var(--scrim);
  padding: 0.15rem 0.5rem;
  border-radius: var(--radius-sm);
}
/* 视图模式切换（3D 开放世界 ↔ 经典岛屿）：浮在舞台左上，不占布局高度 */
.island-game__viewmode {
  position: absolute;
  top: 0.5rem;
  left: 0.5rem;
  display: flex;
  gap: 0.3rem;
  z-index: 1;
}
.island-game__panel {
  display: flex;
  flex-direction: column;
  gap: 0.4rem;
  border: 1px solid var(--border);
  border-radius: var(--radius-card);
  padding: 0.75rem;
  background: var(--bg-surface);
  overflow: auto;
}
.panel__title {
  margin: 0.25rem 0 0;
  font-size: 0.85rem;
  color: var(--text);
}
.panel__line { margin: 0; font-size: 0.78rem; color: var(--text-muted); }
.panel__line--muted { opacity: 0.75; }
.quest__list { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 0.4rem; }
.quest__item {
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  padding: 0.4rem;
}
.quest__row { display: flex; justify-content: space-between; align-items: center; gap: 0.4rem; }
.quest__row--actions { margin-top: 0.3rem; }
.quest__label { font-size: 0.8rem; color: var(--text); }
.quest__exp { font-size: 0.72rem; color: var(--text-muted); }
.btn {
  font-size: 0.75rem;
  padding: 0.25rem 0.6rem;
  border-radius: var(--radius-sm);
  border: 1px solid var(--border);
  background: var(--bg);
  color: var(--text);
  cursor: pointer;
}
.btn--primary { background: var(--accent); color: var(--accent-foreground); border-color: transparent; }
.btn:disabled { opacity: 0.5; cursor: not-allowed; }
.party__slots { display: flex; gap: 0.4rem; }
.party__slot {
  width: 2.4rem;
  height: 2.4rem;
  border-radius: var(--radius-md);
  border: 1px dashed var(--border);
  background: var(--bg);
  color: var(--text-muted);
  cursor: pointer;
}
.worldgen__label { font-size: 0.75rem; color: var(--text-muted); }
.worldgen__input {
  width: 100%;
  box-sizing: border-box;
  border-radius: var(--radius-sm);
  border: 1px solid var(--border);
  padding: 0.4rem;
  font: inherit;
  font-size: 0.8rem;
}
.worldgen__actions { display: flex; gap: 0.4rem; }
.worldgen__name { margin: 0; font-size: 0.78rem; color: var(--text); }
.worldgen__errors { margin: 0; padding-left: 1rem; color: var(--danger); font-size: 0.72rem; }
</style>
