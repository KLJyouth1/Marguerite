<template>
  <div class="control-stage pet-stage">
    <header class="control-stage__header">
      <div class="control-stage__title-block">
        <h1 class="control-stage__title">{{ effectiveSkin.name }}</h1>
        <p class="control-stage__subtitle">{{ effectiveSkin.description }}（mood 驱动动画）</p>
      </div>
      <div class="control-stage__actions">
        <button class="btn btn--ghost" @click="toggleOverlay" :disabled="!overlaySupported" :title="overlaySupported ? '' : '当前环境不支持桌面悬浮窗（需桌面版）'">
          <Icon name="monitor" :size="16" /><span>{{ overlayOn ? '隐藏桌面桌宠' : '显示桌面桌宠' }}</span>
        </button>
        <button class="btn btn--ghost" @click="load"><Icon name="refresh" :size="16" /><span>刷新</span></button>
      </div>
    </header>
    <p class="pet-desktop-note">桌面桌宠是独立的置顶悬浮窗（任务栏外、可拖拽、双击抚摸）；页内桌宠在 设置 → 宠物 中单独开关。{{ overlayError ? ' 上次操作失败：' + overlayError : '' }}</p>
    <section v-if="error" class="pet-error">{{ error }}</section>

    <!-- 渲染模式 + 3D 视角（多视角是真三维：转相机看真实背面几何） -->
    <section class="pet-view-bar" aria-label="渲染模式与视角">
      <div class="control-segmented" role="group" aria-label="渲染模式">
        <button
          class="control-segmented__btn"
          :class="{ 'is-active': renderMode === 'svg' }"
          type="button"
          @click="setRenderMode('svg')"
        >2.5D 矢量</button>
        <button
          class="control-segmented__btn"
          :class="{ 'is-active': renderMode === '3d' }"
          type="button"
          :disabled="!webglOk"
          :title="webglOk ? '真三维：可切正面 / 45° / 正侧 / 背面' : '当前环境不支持 WebGL'"
          @click="setRenderMode('3d')"
        >3D 立体</button>
      </div>
      <div v-if="renderMode === '3d' && webglOk" class="pet-view-bar__views" role="group" aria-label="视角">
        <button
          v-for="opt in VIEW_OPTIONS"
          :key="opt.key"
          class="pet-view-chip"
          :class="{ 'is-active': view === opt.key }"
          type="button"
          @click="setView(opt.key)"
        >{{ opt.label }}</button>
        <label class="pet-view-auto">
          <input type="checkbox" :checked="autoRotate" @change="toggleAutoRotate(($event.target as HTMLInputElement).checked)" />
          <span>自动旋转</span>
        </label>
      </div>
      <p v-if="!webglOk" class="pet-view-bar__hint">当前环境不支持 WebGL，已回退 2.5D 矢量渲染（形象完整，仅无 3D 视角）。</p>
    </section>

    <section class="pet-card">
      <!-- SVG 矢量舞台（canvas/model 皮肤）；SVG 无 WebGL 依赖，任何环境都完整渲染。
           悬停 → 局部聚焦注视（视线/瞳孔幅度放大） -->
      <div
        v-if="effectiveSkin.kind !== 'image'"
        ref="stage3dEl"
        class="pet3d-stage"
        role="img"
        :aria-label="effectiveSkin.name"
        :class="{ 'is-focused': focused, 'is-3d': renderMode === '3d' && webglOk }"
        @pointerenter="onStageEnter"
        @pointerleave="onStageLeave"
      ></div>
      <div v-else class="pet-image-wrap">
        <img :src="effectiveSkin.src" :alt="effectiveSkin.name" class="pet-image" draggable="false" />
      </div>
      <div class="pet-mood">{{ moodLabel }}</div>
      <div class="pet-stats">能量 {{ energy }}/100 · 好感度 {{ affection }}</div>
      <div class="pet-bar"><div class="pet-bar__fill" :style="{ width: energy + '%', background: energy < 20 ? 'var(--warn)' : 'var(--accent)' }"></div></div>

      <!-- 智慧成长面板（与桌面悬浮宠共享同一个大脑） -->
      <div class="pet-mind">
        <div class="pet-mind__row">
          <span class="pet-mind__lv">Lv.{{ level }}</span>
          <span class="pet-mind__mood">{{ moodLabel }}</span>
          <span class="pet-mind__xp">{{ xp }} / {{ nextLevelAt }} XP</span>
        </div>
        <div class="pet-bar"><div class="pet-bar__fill" :style="{ width: xpProgress + '%' }"></div></div>
        <!-- 智慧统计 v2：宠物真的在「长大」 -->
        <div class="pet-mind__stats">
          <span>✅ 任务 {{ brainFrame.stats.tasksDone }}</span>
          <span>🔧 工具 {{ brainFrame.stats.toolsUsed }}</span>
          <span>📖 观察 {{ brainFrame.stats.tokensSeen.toLocaleString() }} tok</span>
        </div>
        <p class="pet-mind__hint">它注视着你的每一次流式输出与工具调用：任务完成会庆祝攒经验，久未互动就蜷起来打盹。</p>
      </div>

      <!-- Viky · PSI 意识引擎（aris_brain 实时状态，桌宠作为其具身表现层） -->
      <!--
        布局稳定（2026-10-24 真机实测）：
        此前首帧只渲染一行「未连接」，`brain.status` 返回后整块面板插进来，
        把下面的皮肤区推下去约 267px（top 1080 → 1347）。
        现在**不分支 DOM**：加载中与已连接共用同一套元素（8 行维度、同一组 class），
        只替换文本与占位条；meta 行强制单行，避免文本长短引起换行导致的高度抖动。
      -->
      <section
        class="pet-psi"
        :class="{ 'is-loading': psiLoading }"
        v-if="psiLoading || psiEnabled"
        :aria-busy="psiLoading ? 'true' : 'false'"
        :data-testid="psiLoading ? 'pet-psi-skeleton' : 'pet-psi'"
      >
        <div class="pet-psi__head">
          <span class="pet-psi__title">🧠 Viky · PSI 意识引擎</span>
          <span class="pet-psi__phi">{{ psiEnabled ? `Φ ${psiPhi.toFixed(3)}` : 'Φ —' }}</span>
        </div>
        <div class="pet-psi__meta">
          {{ psiEnabled ? `相位节律：${psiCircadian} · 主导情绪：${psiDominant}` : '正在读取引擎状态…' }}
        </div>
        <div class="pet-psi__bars">
          <div v-for="d in psiDims" :key="d.key" class="pet-psi__bar">
            <span class="pet-psi__bar-label">{{ d.emoji }} {{ d.label }}</span>
            <div class="pet-psi__bar-track">
              <div
                class="pet-psi__bar-fill"
                :class="{ 'is-placeholder': !psiEnabled }"
                :style="psiEnabled ? { width: Math.round(d.value * 100) + '%' } : undefined"
              />
            </div>
            <span class="pet-psi__bar-val">{{ psiEnabled ? Math.round(d.value * 100) + '%' : '—' }}</span>
          </div>
        </div>
        <p class="pet-psi__hint">桌宠是 Viky 的具身表现层：八维情感实时驱动其表情，全程只读真实引擎状态。</p>
      </section>
      <p class="pet-psi__off" v-else>PSI 意识引擎（brain.status）未连接，桌宠回退到本地情绪。</p>

      <div class="pet-actions">
        <button class="btn btn--primary" :disabled="busy" @click="onPet"><Icon name="sparkles" :size="16" /><span>顺毛抚摸</span></button>
        <button class="btn btn--secondary" :disabled="busy" @click="onFeed"><Icon name="plus" :size="16" /><span>投喂猫条</span></button>
      </div>
      <div class="pet-hint">能量每 10 秒自然衰减 1；低于 20 进入朦胧态。互动回能并提升好感度。</div>
    </section>

    <!-- 皮肤选择：放在桌宠下方，桌宠本体优先占据首屏；选中态由卡片高亮 + 页头标题呈现 -->
    <section class="pet-skins">
      <h3 class="pet-skins__title">选择皮肤</h3>
      <div class="pet-skins__grid">
        <button
          v-for="s in skins"
          :key="s.id"
          class="pet-skin-card"
          :class="{ 'is-active': s.id === skin.id }"
          @click="selectSkin(s.id)"
        >
          <div
            class="pet-skin-card__preview"
            :style="previewStyle(s)"
          >
            <span class="pet-skin-card__emoji">{{ s.emoji }}</span>
          </div>
          <span class="pet-skin-card__name">{{ s.name }}</span>
        </button>
      </div>
    </section>
  </div>
</template>

<script setup lang="ts">
import { ref, computed, onMounted, onUnmounted, watch, nextTick, type CSSProperties } from 'vue'
import Icon from '@/components/Icon.vue'
// V2.1-P1c 统一契约入口：usePetRpc（替换 useRpcStore 裸调用 + 本地皮肤/状态）
import {
  usePetRpc,
  PET_SKIN_IDS,
  assertPetStatusSymmetry,
  PET_EXPECTED_SKIN_COUNT,
  type PetSkinId,
  type PetAction,
  type PetSkinEntry,
  type PetStatus,
} from '@/rpc/petContract'
// SVG 引擎 + 智慧大脑（与桌面悬浮宠共享同一情绪/记忆）
import { PetSvg } from '@/components/pet/petSvg'
import type { Pet3DScene } from '@/components/pet/pet3d/Pet3DScene'
// 单独的轻量模块：不引入 three，可在 setup 阶段同步探测（避免探测晚到导致布局位移）
import { isWebGLAvailable } from '@/components/pet/pet3d/webglSupport'
import {
  PET_3D_VIEW_LABELS,
  type Pet3DView,
} from '@/components/pet/pet3d/views'
import { getPetBrain, type BrainFrame } from '@/components/pet/petBrain'
// v3 契约层：脑心情→渲染层表情唯一映射（替换本页手写映射，杜绝漏同步）
import { moodMap, type PetMood3D } from '@/components/pet/petContract'
// 数据驱动模型：按皮肤 id 取 Spec（猫/精灵全矢量化）
import { getPetModelSpec } from '@/components/pet/petSpec'
// 运行时档位探测：WebGL/硬件信号只决定特效预算（SVG 无 WebGL 也完整渲染）
import { detectPetRuntime, renderTierOf } from '@/components/pet/detectPetRuntime'
import { useRpcStore } from '@/stores/rpc'
import { ambience } from '@/composables/useAmbience'
import {
  petOverlayAvailable,
  petOverlayPreferred,
  setPetOverlayEnabled,
} from '@/composables/usePetOverlay'

const petRpc = usePetRpc()
const stage3dEl = ref<HTMLElement | null>(null)

// 桌面桌宠悬浮窗（Tauri 独立窗口）：能力取真实桥；Web 端按钮置灰。
const overlaySupported = petOverlayAvailable()
const overlayOn = ref(overlaySupported && petOverlayPreferred())
const overlayError = ref('')
async function toggleOverlay(): Promise<void> {
  overlayError.value = ''
  const next = !overlayOn.value
  const result = await setPetOverlayEnabled(next)
  if (result.ok) overlayOn.value = next
  else overlayError.value = result.error ?? 'unknown'
}

/** 皮肤目录由后端 pet.list 权威供给（R6：单一权威来源），前端本地 PET_SKINS 只作为离线兜底。
 *  注意：PetSkinEntry.kind = 'beast' | 'humanoid'（后端语义）；UiSkin.kind = 'canvas' | 'model' | 'image'（前端渲染分支），
 *  故用 Omit 去掉原 kind，避免 TS 联合冲突。 */
type UiSkin = Omit<PetSkinEntry, 'kind'> & { name: string; emoji: string; description: string; kind: 'canvas' | 'model' | 'image'; model?: string; accent: string; src?: string }
const FALLBACK_EMOJI: Record<PetSkinId | string, string> = {
  cat: '🐱', blue: '🧚‍♀️', pink: '🌸', purple: '💜', green: '🍀',
}
const FALLBACK_LABEL: Record<PetSkinId | string, string> = {
  cat: '橘猫小工', blue: '雏菊精灵', pink: '樱花精灵', purple: '薰衣草精灵', green: '四叶草精灵',
}
const FALLBACK_DESC: Record<PetSkinId | string, string> = {
  cat: '三花猫 · 经典可爱款',
  blue: '雏菊精灵 · 花嫁少女',
  pink: '樱花精灵 · 花瓣公主',
  purple: '薰衣草精灵 · 紫雾少女',
  green: '四叶草精灵 · 幸运少女',
}
function toUiSkin(entry: PetSkinEntry): UiSkin {
  const id = entry.id as PetSkinId
  return {
    ...entry,
    name: entry.label || FALLBACK_LABEL[id] || id,
    emoji: FALLBACK_EMOJI[id] || '✨',
    description: FALLBACK_DESC[id] || `${entry.kind} 皮肤`,
    kind: entry.kind === 'beast' ? 'canvas' : 'model',
    model: entry.id,
    accent: entry.accent_hex,
  }
}
const OFFLINE_SKINS: UiSkin[] = (PET_SKIN_IDS as readonly string[]).map((id) => toUiSkin({
  id, label: FALLBACK_LABEL[id] || id, kind: id === 'cat' ? 'beast' : 'humanoid', accent_hex: id === 'cat' ? '#F59E0B' : id === 'blue' ? '#60A5FA' : id === 'pink' ? '#F472B6' : id === 'purple' ? '#A78BFA' : '#34D399',
}))

const skins = ref<UiSkin[]>(OFFLINE_SKINS)
const skin = ref<UiSkin>(OFFLINE_SKINS[0])

/** 皮肤预览：主题色渐变 + emoji（替代图片素材） */
function previewStyle(s: UiSkin): CSSProperties {
  const accent = s.accent.startsWith('#') ? s.accent : 'var(--accent)'
  return {
    background: `radial-gradient(circle at 50% 36%, color-mix(in srgb, ${accent} 34%, transparent), transparent 72%), linear-gradient(160deg, color-mix(in srgb, ${accent} 14%, var(--bg)) 0%, var(--bg) 100%)`,
  }
}

// SVG 渲染器无 WebGL 依赖：任何环境都能完整渲染，故不再探测 WebGL、也没有 2D 降级分支。
// effectiveSkin 仅用于标题/描述展示（当前皮肤列表全为矢量模型）。
const effectiveSkin = computed<UiSkin>(() => skin.value)

const pet = ref<PetStatus | null>(null)
const busy = ref(false)
const error = ref('')

/** 加载皮肤目录（权威来源：RPC pet.list），失败则静默回退到离线兜底，不打断主流程（R1 诚实但不悲观） */
async function loadSkins(): Promise<void> {
  try {
    const res = await petRpc.list()
    if (res.skins.length !== PET_EXPECTED_SKIN_COUNT) {
      error.value = `[契约守卫] pet.list 皮肤数 ${res.skins.length} ≠ 期望 ${PET_EXPECTED_SKIN_COUNT}，仍以目录显示，已记录漂移。`
    }
    const mapped = res.skins.map(toUiSkin)
    skins.value = mapped.length ? mapped : OFFLINE_SKINS
    // 对齐当前选中皮肤（若后端返回里存在匹配项则用后端权威项，否则保留）
    const match = mapped.find((s) => s.id === skin.value.id) || mapped[0]
    if (match) skin.value = match
  } catch (e) {
    // 离线场景或后端未注册 pet.list：诚实保留 OFFLINE 兜底，错误只写 detail 不阻塞
    const msg = e instanceof Error ? e.message : String(e)
    if (!msg.includes('NOT_IMPLEMENTED')) error.value = `皮肤目录加载失败（已离线兜底）：${msg}`
  }
}

/** 加载当前宠物状态（字段对称守卫：不对称直接抛） */
async function load(): Promise<void> {
  try {
    const status = await petRpc.status()
    pet.value = status
    // 同步选中皮肤到权威状态（若 pet.status.skin 改变，则 UI 跟随切换）
    const match = skins.value.find((s) => s.id === status.skin)
    if (match && match.id !== skin.value.id) {
      skin.value = match
      void nextTick(initStage)
    }
  } catch (e) {
    error.value = e instanceof Error ? e.message : String(e)
  }
}

/** 切换皮肤：调用 pet.setSkin → 成功后更新本地选中并重建矢量舞台 */
async function selectSkin(id: PetSkinId | string) {
  busy.value = true
  error.value = ''
  try {
    const res = await petRpc.setSkin(id)
    if (!res.ok) {
      error.value = res.error || '切换皮肤失败'
      return
    }
    const match = skins.value.find((s) => s.id === res.skin)
    if (match) {
      skin.value = match
      if (skin.value.kind !== 'image') void nextTick(initStage)
    }
  } catch (e) {
    error.value = e instanceof Error ? e.message : String(e)
  } finally {
    busy.value = false
  }
}

/** 触发动作（抚摸 / 投喂 / 庆祝…）：统一走 triggerAction → 成功后拉取最新状态 + 驱动大脑 */
async function trigger(action: PetAction) {
  busy.value = true
  error.value = ''
  try {
    const res = await petRpc.triggerAction(action)
    if (!res.ok) {
      error.value = res.error || `动作 ${action} 失败`
      return
    }
    // 环境音开启时给动作一个轻量合成音（无音频文件）；未开启时是空操作。
    if (action === 'pet') ambience.chime('pet')
    else if (action === 'feed') ambience.chime('feed')
    // 触发后字段同步到本地视图（避免额外 round-trip；缺失字段则 load() 补齐）
    if (res.mood || typeof res.energy === 'number' || typeof res.affection === 'number') {
      const base: Record<string, unknown> = { ...(pet.value ?? { skin: skin.value.id, kind: skin.value.kind, mood: 'normal', energy: 100, affection: 0, running: true, accent_hex: skin.value.accent }) }
      if (res.mood) base.mood = res.mood
      if (typeof res.energy === 'number') base.energy = res.energy
      if (typeof res.affection === 'number') base.affection = res.affection
      try { pet.value = assertPetStatusSymmetry(base) } catch { void load() }
    } else {
      void load()
    }
  } catch (e) {
    error.value = e instanceof Error ? e.message : String(e)
  } finally {
    busy.value = false
  }
}

function onPet() {
  brain.petted()
  void trigger('pet')
}
function onFeed() {
  brain.feed('done')
  void trigger('feed')
}

// ── Viky 数字生命体 · PSI 意识引擎接入（作为 viky 载体）──
// 桌宠是 PSI 意识的「具身表现层」：轮询 brain.status（aris_brain.rs 暴露的 RPC），
// 把八维情感实时映射到 3D 桌宠表情与 PSI 状态面板。全程只读真实引擎状态，绝无模拟。
// 引擎未就绪（未连接 / 不支持 brain.status / brain.status 抛错）时静默跳过，回退到本地 petBrain 的既有情绪。
interface PsiSnapshot {
  emotion?: Record<string, number>
  dominant_mood?: string
  circadian?: string
  phi_existence?: string | number
  [k: string]: unknown
}
const psi = ref<PsiSnapshot | null>(null)
/** 首帧骨架：`brain.status` 尚未返回时为 true（避免面板晚到造成的纵向布局位移） */
const psiLoading = ref(true)
/** 骨架的最长等待时间点：连不上时到点退回「未连接」，不让骨架永久挂着 */
let psiLoadDeadline = 0
const psiMood = ref<PetMood3D>('normal')

const psiEnabled = computed(() => !!psi.value && !!psi.value.emotion)
const psiPhi = computed(() => {
  const v = parseFloat(String(psi.value?.phi_existence ?? '0.5'))
  return Number.isFinite(v) ? v : 0.5
})
const psiDominant = computed(() => psi.value?.dominant_mood ?? '—')
const psiCircadian = computed(() => psi.value?.circadian ?? '—')
const PSI_DIMS: { key: string; label: string; emoji: string }[] = [
  { key: 'valence', label: '愉悦', emoji: '😊' },
  { key: 'arousal', label: '唤醒', emoji: '⚡' },
  { key: 'curiosity', label: '好奇', emoji: '🔍' },
  { key: 'confidence', label: '自信', emoji: '💪' },
  { key: 'connection', label: '连接', emoji: '💞' },
  { key: 'growth', label: '成长', emoji: '🌱' },
  { key: 'certainty', label: '确定感', emoji: '🧭' },
  { key: 'existence', label: '存在感', emoji: '✨' },
]
const psiDims = computed(() =>
  PSI_DIMS.map((d) => ({ ...d, value: Number(psi.value?.emotion?.[d.key] ?? 0.5) })),
)

function numOr(v: unknown, fallback: number): number {
  return typeof v === 'number' && Number.isFinite(v) ? v : fallback
}

// 八维情感 → 桌宠 3D 表情（valence 主导快乐/悲伤，arousal 主导兴奋/困倦，certainty 低 → 困惑）。
function applyPsiSnapshot(raw: PsiSnapshot | null): void {
  psi.value = raw
  if (!raw || !raw.emotion) {
    psiMood.value = 'normal'
    return
  }
  const e = raw.emotion
  const valence = numOr(e.valence, 0.5)
  const arousal = numOr(e.arousal, 0.5)
  const certainty = numOr(e.certainty, 0.5)
  let m: PetMood3D = 'normal'
  if (arousal < 0.28) m = 'sleepy'
  else if (certainty < 0.3) m = 'sad'
  else if (valence > 0.7 && arousal > 0.6) m = 'excited'
  else if (valence > 0.6) m = 'happy'
  else if (valence < 0.35) m = 'sad'
  else m = 'normal'
  psiMood.value = m
}

async function pollPsi(): Promise<void> {
  const rpc = getRpcStore_()
  if (!rpc || rpc.state !== 'connected' || !rpc.supportsMethod('brain.status')) {
    // 还没连上 / 方法表还没下发：保持骨架（结构占位，不产生位移），
    // 超过等待窗口才退回「未连接」，避免永久骨架。
    if (psiLoadDeadline && Date.now() > psiLoadDeadline) psiLoading.value = false
    return
  }
  try {
    const res = await rpc.call<{ ok?: boolean; brain?: PsiSnapshot }>('brain.status', {})
    applyPsiSnapshot(res?.brain ?? null)
  } catch {
    /* 引擎未就绪时静默跳过，保持本地情绪 */
  } finally {
    psiLoading.value = false
  }
}

// 订阅 psi_broadcast 实时事件（与轮询兜底互补）；广播载荷可能是 {brain} 或直接是 snapshot。
function onPsiBroadcast(e: Event): void {
  const detail = (e as CustomEvent).detail as { brain?: PsiSnapshot } | PsiSnapshot | null
  const wrapped = detail && typeof detail === 'object' && 'brain' in detail
    ? (detail as { brain?: PsiSnapshot }).brain
    : (detail as PsiSnapshot | null)
  if (wrapped && typeof wrapped === 'object') {
    applyPsiSnapshot(wrapped)
    psiLoading.value = false
  }
}

/** 渲染模式：2.5D 矢量（默认低门槛）/ 3D 立体（真三维 + 多视角） */
const RENDER_MODE_KEY = 'pet.renderMode'
const renderMode = ref<'svg' | '3d'>('svg')
/**
 * WebGL 可用性：**setup 阶段同步探测**（不是 onMounted 里异步等 3D 模块）。
 *
 * 此前探测依赖 `await import(Pet3DScene)`（模块里带 three），结果约 200ms 后才回来：
 * 页面先按「不支持 WebGL」渲染（显示回退提示、隐藏视角按钮），探测完再切换，
 * 工具条换形导致桌宠卡上移 29px（真机 Layout Instability 实测 0.173 分… 单条 0.0077，
 * 发生在加载后约 200ms）。同步探测后首帧就是终态。
 */
const webglOk = ref(isWebGLAvailable())
{
  let saved: string | null = null
  try { saved = localStorage.getItem(RENDER_MODE_KEY) } catch { /* 忽略 */ }
  renderMode.value = saved === 'svg' || !webglOk.value ? 'svg' : '3d'
}
const view = ref<Pet3DView>('front')
const autoRotate = ref(false)
const VIEW_OPTIONS: Array<{ key: Exclude<Pet3DView, 'free'>; label: string }> = [
  { key: 'front', label: PET_3D_VIEW_LABELS.front },
  { key: 'threeQuarter', label: PET_3D_VIEW_LABELS.threeQuarter },
  { key: 'side', label: PET_3D_VIEW_LABELS.side },
  { key: 'back', label: PET_3D_VIEW_LABELS.back },
]

let petScene: PetSvg | Pet3DScene | null = null
/** 3D 专属实例（视角/自动旋转只对它生效） */
let pet3d: Pet3DScene | null = null
/** 舞台重建令牌：异步 await 之后用令牌丢弃过期的旧构建 */
let stageToken = 0

/**
 * 悬停聚焦：光标进入/离开舞台 → 聚焦权重 0/1。
 * 权重驱动 `PetSvg.setHover`：视线/瞳孔幅度放大（局部注视）。
 * `focused` 同步到舞台的 `is-focused` 状态位，供 e2e/诊断断言真实交互已生效。
 */
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
  const token = ++stageToken
  // 静态立绘（当前列表已全矢量化）走 img 分支，不需矢量场景
  if (effectiveSkin.value.kind === 'image') {
    petScene?.dispose()
    petScene = null
    pet3d = null
    return
  }
  if (!stage3dEl.value) return
  petScene?.dispose()
  petScene = null
  pet3d = null
  // 运行时档位：只调特效预算（模糊半径 / 粒子数），SVG 无 WebGL 也完整渲染
  const runtime = detectPetRuntime()
  // 皮肤主题色：model 皮肤自带 hex accent；canvas 猫用全局 --accent
  const accentRaw = effectiveSkin.value.accent.startsWith('#')
    ? effectiveSkin.value.accent
    : getComputedStyle(document.documentElement).getPropertyValue('--accent').trim()
  const accent = accentRaw.startsWith('#') ? parseInt(accentRaw.slice(1), 16) : 0x60a5fa
  // 数据驱动造型：canvas 猫 / model 精灵由 Spec 决定外观（热插拔，渲染代码零改动）
  const spec = getPetModelSpec(effectiveSkin.value.model ?? effectiveSkin.value.id)

  if (renderMode.value === '3d' && webglOk.value) {
    try {
      // 动态 import：3D 运行时（three）不进首屏包，只有真正切到 3D 才加载
      const mod = await import('@/components/pet/pet3d/Pet3DScene')
      if (token !== stageToken || !stage3dEl.value) return
      const scene = new mod.Pet3DScene({
        container: stage3dEl.value,
        accent,
        spec,
        quality: renderTierOf(runtime.quality),
      })
      scene.setView(view.value)
      scene.setAutoRotate(autoRotate.value)
      scene.setHover(focused.value ? 1 : 0)
      pet3d = scene
      petScene = scene
      return
    } catch (e) {
      // 构造失败（驱动/上下文丢失）→ 如实回退矢量渲染并说明原因，绝不留下空白舞台
      webglOk.value = false
      renderMode.value = 'svg'
      error.value = `3D 渲染初始化失败，已回退 2.5D 矢量：${e instanceof Error ? e.message : String(e)}`
    }
  }

  try {
    petScene = new PetSvg({
      container: stage3dEl.value,
      accent,
      spec,
      quality: renderTierOf(runtime.quality),
    })
    // 重建后补上当前悬停态（光标可能一直停在舞台上，重挂载不应丢失聚焦）
    petScene.setHover(focused.value ? 1 : 0)
  } catch (e) {
    // 构建期异常（如环境缺 document）→ 如实上报，不假装已渲染
    error.value = e instanceof Error ? e.message : String(e)
  }
}

/** 切换渲染模式（持久化；3D 不可用时按钮已被禁用，这里再守一道） */
function setRenderMode(mode: 'svg' | '3d'): void {
  if (mode === '3d' && !webglOk.value) return
  if (renderMode.value === mode) return
  renderMode.value = mode
  try { localStorage.setItem(RENDER_MODE_KEY, mode) } catch { /* 忽略 */ }
  error.value = ''
  void initStage()
}

/** 预设视角（正面 / 45° / 正侧 / 背面） */
function setView(next: Exclude<Pet3DView, 'free'>): void {
  view.value = next
  autoRotate.value = false
  pet3d?.setView(next)
  pet3d?.setAutoRotate(false)
}

function toggleAutoRotate(on: boolean): void {
  autoRotate.value = on
  pet3d?.setAutoRotate(on)
}

let raf = 0
let lastT = performance.now() / 1000
let psiTimer: ReturnType<typeof setInterval> | null = null
let psiUnsub: (() => void) | null = null

/** 获取 rpcStore 的兼容入口：petRpc 仅封装 pet 方法，PSI/通用事件仍需底层 rpc 实例。
 *  V2.1 设计原则：契约接口内部持有 rpc，对外仍暴露轻量入口，避免多实例耦合。
 *
 *  【2026-10-23 修复】旧实现用 `require('@/stores/rpc')` —— Vite 产物里没有 `require`，
 *  ReferenceError 被 catch 吞掉后永远返回 null，于是 `brain.status` 轮询与
 *  `psi_broadcast` 订阅**从未生效**（PSI 面板长期显示默认 0.5）。改为静态 import。 */
function getRpcStore_() {
  try {
    return useRpcStore()
  } catch {
    return null
  }
}

const brain = getPetBrain()
const brainFrame = ref<BrainFrame>(brain.frame())
const level = computed(() => brainFrame.value.level)
const xp = computed(() => brainFrame.value.xp)
const nextLevelAt = computed(() => level.value * 50)
const xpProgress = computed(() => Math.round(((xp.value % 50) / 50) * 100))

const energy = computed(() => pet.value?.energy ?? 100)
const affection = computed(() => pet.value?.affection ?? 0)
const moodLabel = computed(() => {
  // 优先显示后端返回的 mood（若与 PetMood 枚举对齐则转中文），否则显示 brainFrame 情绪
  const pm = pet.value?.mood
  if (pm) {
    const map: Record<string, string> = { normal: '眯眼晒太阳', happy: '开心摇尾', sleepy: '蜷成一团', working: '专注巡逻', excited: '蹦蹦跳跳', sad: '垂头丧气' }
    return map[pm] || String(pm)
  }
  switch (brainFrame.value.mood) {
    case 'celebrate': return '开箱庆祝！'
    case 'working': return '专注巡逻'
    case 'thinking': return '托腮沉思'
    case 'laughing': return '被逗乐了'
    case 'attentive': return '目不转睛'
    case 'curious': return '好奇张望'
    case 'alert': return '耳朵竖起'
    case 'sleepy': return '蜷成一团'
    default: return '眯眼晒太阳'
  }
})

function loop() {
  const t = performance.now() / 1000
  const dt = Math.min(0.1, Math.max(0.001, t - lastT))
  lastT = t
  brainFrame.value = brain.tick(dt)
  // PSI 意识引擎优先驱动表情（作为 viky 载体）；未连接时回退到 petBrain 情绪映射
  if (psiEnabled.value) {
    petScene?.setMood(psiMood.value)
  } else {
    // v3：单一事实源（petContract.moodMap），不再手写本地映射
    petScene?.setMood(moodMap[brainFrame.value.mood])
  }
  raf = requestAnimationFrame(loop)
}

// 页面鼠标 → 视线追踪（智慧感核心：它看着你）
function onPageMouseMove(e: MouseEvent) {
  const nx = (e.clientX / window.innerWidth) * 2 - 1
  const ny = (e.clientY / window.innerHeight) * 2 - 1
  petScene?.lookAt(nx, ny)
}

let timer: ReturnType<typeof setInterval> | null = null

/**
 * 启动阶段：DOM 就绪后建舞台。
 * WebGL 探测与默认渲染模式已在 setup 里同步完成（首帧即终态），
 * 因此这里不再有「先建 SVG、再切 3D」的二次构建。
 */
async function bootstrapStage(): Promise<void> {
  // WebGL 与渲染模式已在 setup 里同步定好（首帧即终态），这里只等 DOM 就绪后建场景。
  // 注意：three 仍然按需加载（initStage 内部 `await import`），不进首屏包。
  await nextTick()
  void initStage()
}

onMounted(() => {
  // V2.1-P1c：两步加载（① 皮肤目录 ② 当前状态），失败均诚实兜底
  void loadSkins().then(() => void load())
  void bootstrapStage()
  raf = requestAnimationFrame(loop)
  window.addEventListener('mousemove', onPageMouseMove)
  timer = setInterval(() => void load(), 15000)
  // Viky · PSI 意识引擎：轮询兜底 + 实时广播订阅
  psiLoadDeadline = Date.now() + 15000
  void pollPsi()
  psiTimer = setInterval(() => void pollPsi(), 5000)
  const rpc = getRpcStore_()
  psiUnsub = rpc?.on?.('psi_broadcast', onPsiBroadcast) ?? null
})
onUnmounted(() => {
  cancelAnimationFrame(raf)
  window.removeEventListener('mousemove', onPageMouseMove)
  if (timer) clearInterval(timer)
  if (psiTimer) clearInterval(psiTimer)
  if (psiUnsub) psiUnsub()
  petScene?.dispose()
  petScene = null
})

watch(() => skin.value.id, () => {
  void nextTick(initStage)
})
</script>

<style scoped>
.pet-stage { padding: 24px 28px; max-width: 900px; margin: 0 auto; }
.pet-error { margin: 12px 0; padding: 10px 14px; background: color-mix(in srgb, var(--danger) 10%, var(--card)); border: 1px solid color-mix(in srgb, var(--danger) 30%, transparent); border-radius: var(--radius-sm); color: var(--danger); font-size: 12px; }

.pet-skins { margin-top: 16px; }
.pet-skins__title { font-size: 14px; font-weight: 600; margin: 0 0 10px; color: var(--text); }
.pet-skins__grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(120px, 1fr)); gap: 10px; }
.pet-skin-card {
  display: flex; flex-direction: column; align-items: center; gap: 6px;
  padding: 10px 8px; background: var(--card); border: 2px solid var(--hairline);
  border-radius: var(--radius-md); cursor: pointer; transition: all var(--dur-fast) var(--ease-out);
}
.pet-skin-card:hover { border-color: color-mix(in srgb, var(--accent) 40%, var(--hairline)); transform: translateY(-2px); }
.pet-skin-card.is-active { border-color: var(--accent); box-shadow: 0 0 0 3px color-mix(in srgb, var(--accent) 15%, transparent); }
.pet-skin-card__preview {
  width: 66px; height: 75px; display: flex; align-items: center; justify-content: center;
  border-radius: var(--radius-sm); overflow: hidden;
  background: radial-gradient(circle at 50% 40%, color-mix(in srgb, var(--accent) 8%, var(--bg)), var(--bg));
}
.pet-skin-card__preview canvas, .pet-skin-card__preview img { max-width: 100%; max-height: 100%; }
.pet-skin-card__preview img { object-fit: contain; }
.pet-skin-card__name { font-size: 12px; color: var(--text); text-align: center; }

.pet-card { margin-top: 14px; padding: 18px; background: var(--card); border: 1px solid var(--hairline); border-radius: var(--radius-md); text-align: center; }
.fairy-canvas { max-width: 100%; border-radius: var(--radius-md); background: radial-gradient(circle at 50% 40%, color-mix(in srgb, var(--accent) 12%, var(--bg)), var(--bg)); }
.pet3d-stage { position: relative; width: 100%; height: 380px; border-radius: var(--radius-md); background: radial-gradient(circle at 50% 38%, color-mix(in srgb, var(--accent) 14%, var(--bg)), var(--bg)); overflow: hidden; }
/* 矮窗口（如 1024×680 安装版）：压缩区块间距与舞台高度，保证不滚动就能看到桌宠本体 */
@media (max-height: 780px) {
  .pet-stage { gap: var(--sp-5); }
  /* 实测（1024×680）：舞台 top≈390，313px 高时下沿 703 超出视口 23px；
     压到 280px 上限后下沿≈648，留出余量吸收异步布局位移。 */
  .pet3d-stage { height: min(38vh, 280px); }
}
/* 3D 模式：底色压深一档 + 底部渐暗，给浅色角色一个明确的轮廓背景（白底白宠会糊成一片） */
.pet3d-stage.is-3d {
  background:
    radial-gradient(circle at 50% 30%, color-mix(in srgb, var(--accent) 30%, var(--bg)) 0%, transparent 62%),
    linear-gradient(180deg, color-mix(in srgb, var(--text) 4%, var(--bg)) 0%, color-mix(in srgb, var(--text) 16%, var(--bg)) 100%);
}
.pet3d-stage :deep(canvas.pet-3d-canvas) { touch-action: none; cursor: grab; }
.pet3d-stage :deep(canvas.pet-3d-canvas:active) { cursor: grabbing; }

/* 渲染模式 + 3D 视角工具条 */
.pet-view-bar {
  display: flex; flex-wrap: wrap; align-items: center; gap: 10px;
  margin-top: 14px; padding: 10px 12px;
  background: var(--card); border: 1px solid var(--hairline); border-radius: var(--radius-md);
}
.pet-view-bar__views { display: flex; flex-wrap: wrap; align-items: center; gap: 6px; }
.pet-view-chip {
  padding: 5px 12px; font-size: 12px; color: var(--text-dim);
  background: color-mix(in srgb, var(--bg) 70%, transparent);
  border: 1px solid var(--hairline); border-radius: 999px; cursor: pointer;
  transition: all var(--dur-fast) var(--ease-out);
}
.pet-view-chip:hover { color: var(--text); border-color: color-mix(in srgb, var(--accent) 45%, var(--hairline)); }
.pet-view-chip.is-active {
  color: var(--accent);
  border-color: var(--accent);
  background: color-mix(in srgb, var(--accent) 12%, var(--bg));
}
.pet-view-auto { display: inline-flex; align-items: center; gap: 6px; font-size: 12px; color: var(--text-dim); cursor: pointer; }
.pet-view-auto input { accent-color: var(--accent); }
.pet-view-bar__hint { flex-basis: 100%; margin: 2px 0 0; font-size: 11px; color: var(--warn); }
.pet-mind { margin-top: 14px; padding: 12px 14px; border-radius: var(--radius-md); background: color-mix(in srgb, var(--accent) 6%, var(--card)); border: 1px solid var(--hairline); text-align: left; }
.pet-mind__row { display: flex; align-items: center; gap: 10px; }
.pet-mind__lv { font-weight: 800; font-size: 16px; color: var(--accent); }
.pet-mind__mood { flex: 1; font-size: 13px; color: var(--text-muted); }
.pet-mind__xp { font-family: var(--font-mono, monospace); font-size: 12px; color: var(--text-dim); }
.pet-mind__hint { margin: 8px 0 0; font-size: 12px; line-height: 1.5; color: var(--text-dim); }
.pet-mind__stats { margin-top: 8px; display: flex; gap: 14px; font-size: 12px; color: var(--text-muted); font-variant-numeric: tabular-nums; }
.pet-image-wrap { display: flex; justify-content: center; }
.pet-image { max-height: 360px; max-width: 100%; border-radius: var(--radius-md); filter: drop-shadow(0 4px 16px color-mix(in srgb, var(--text) 30%, transparent)); }
.pet-mood { margin-top: 10px; font-weight: 600; font-size: 15px; }
.pet-stats { margin-top: 6px; font-size: 13px; color: var(--text-dim); }
.pet-bar { margin: 12px auto; width: 320px; max-width: 90%; height: 8px; background: color-mix(in srgb, var(--text) 10%, var(--card)); border-radius: 999px; overflow: hidden; }
.pet-bar__fill { height: 100%; border-radius: 999px; }
.pet-actions { display: flex; gap: 10px; justify-content: center; }
.pet-hint { margin-top: 12px; font-size: 12px; color: var(--text-dim); }

/* ── Viky · PSI 意识引擎卡片 ── */
.pet-psi { margin-top: 14px; padding: 12px 14px; border-radius: var(--radius-md); background: color-mix(in srgb, var(--accent) 6%, var(--card)); border: 1px solid var(--hairline); text-align: left; }
.pet-psi__head { display: flex; align-items: center; justify-content: space-between; }
.pet-psi__title { font-weight: 700; font-size: 13px; color: var(--text); }
.pet-psi__phi { font-weight: 800; font-size: 16px; color: var(--accent); font-variant-numeric: tabular-nums; }
/* 单行固定：加载态与就绪态的文案长度不同，换行会让整块高度抖动（真机实测 29px） */
.pet-psi__meta {
  margin-top: 2px;
  font-size: 11.5px;
  color: var(--text-dim);
  line-height: 18px;
  height: 18px;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.pet-psi__bars { margin-top: 10px; display: flex; flex-direction: column; gap: 5px; }
.pet-psi__bar { display: flex; align-items: center; gap: 8px; font-size: 11.5px; line-height: 18px; height: 18px; }
.pet-psi__bar-label { width: 78px; flex: none; color: var(--text-muted); }
.pet-psi__bar-track { flex: 1; height: 7px; border-radius: var(--radius-xs); background: color-mix(in srgb, var(--text) 12%, transparent); overflow: hidden; }
.pet-psi__bar-fill { height: 100%; border-radius: var(--radius-xs); background: linear-gradient(90deg, var(--info), var(--accent)); transition: width var(--dur-base) var(--ease-out); }
/* 骨架条：结构占位，不暗示任何真实数值（宽度固定半格 + 弱化配色） */
.pet-psi__bar-fill.is-placeholder { width: 50%; background: var(--hairline); opacity: 0.6; }
.pet-psi__bar-val { width: 34px; text-align: right; color: var(--text-dim); font-variant-numeric: tabular-nums; }
.pet-psi__hint { margin: 8px 0 0; font-size: 11.5px; line-height: 1.5; color: var(--text-dim); }
.pet-psi__off { margin-top: 12px; font-size: 12px; color: var(--text-dim); text-align: center; }
</style>
