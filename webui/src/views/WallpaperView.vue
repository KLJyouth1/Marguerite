<template>
  <div class="control-stage wall-stage">
    <header class="control-stage__header">
      <div class="control-stage__title-block">
        <h1 class="control-stage__title">皮肤中心 · 壁纸</h1>
        <p class="control-stage__subtitle">Web UI 插件式壁纸：预设渐变库 / 本地图片与视频壁纸（静态 + 动态）/ 沉浸式主题预览</p>
      </div>
      <div class="control-stage__actions">
        <button class="btn btn--ghost" @click="clearWallpaper">
          <Icon name="x" :size="16" /><span>清除壁纸</span>
        </button>
        <button class="btn btn--primary" :disabled="!currentBg" @click="immersive = true">
          <Icon name="expand" :size="16" /><span>沉浸式预览</span>
        </button>
      </div>
    </header>

    <section v-if="note" class="wall-note">{{ note }}</section>

    <section class="wall-card">
      <div class="wall-card__title">预设壁纸库（内置 6 款：1 款专属影像 + 5 款主题令牌渐变）</div>
      <div class="wall-grid">
        <button v-for="p in presets" :key="p.id" class="wall-tile" :style="{ background: styleOf(p.css) }" @click="applyPreset(p)">
          <span class="wall-tile__label">{{ p.label }}</span>
          <span v-if="activeId === p.id" class="wall-tile__badge">使用中</span>
        </button>
      </div>
    </section>

    <section class="wall-card">
      <div class="wall-card__title">本地壁纸库（选择本机图片 / 视频）</div>
      <p class="wall-card__hint">图片可经 localStorage 持久化（≤ 2MB，刷新后保留）；视频与超限图片仅当前会话生效（浏览器安全限制，不做假持久化）。</p>
      <div class="wall-row">
        <label class="btn btn--ghost wall-file">
          <Icon name="image" :size="16" /><span>选择图片（jpg/png/webp/gif）</span>
          <input type="file" accept="image/*" class="wall-file__input" @change="onImagePick" />
        </label>
        <label class="btn btn--ghost wall-file">
          <Icon name="video" :size="16" /><span>选择视频（mp4/webm，动态壁纸）</span>
          <input type="file" accept="video/mp4,video/webm" class="wall-file__input" @change="onVideoPick" />
        </label>
      </div>
      <p v-if="localInfo" class="wall-card__hint">{{ localInfo }}</p>
    </section>

    <section class="wall-card">
      <div class="wall-card__title">沉浸式主题</div>
      <p class="wall-card__hint">静态壁纸铺满页面背景；沉浸式预览以全屏遮罩展示动态壁纸（视频循环 + 静态图拉伸）。退出即恢复工作界面。</p>
      <div class="wall-row">
        <button class="btn btn--ghost" :disabled="!localWallpaper" @click="applyToBody(localWallpaper)">
          <Icon name="monitor" :size="16" /><span>应用为页面背景</span>
        </button>
      </div>
    </section>

    <section class="wall-card">
      <div class="wall-card__title">面板遮罩（主题 × 壁纸可读性）</div>
      <p class="wall-card__hint">
        数值越大，内容面板越不透明、正文越稳；越小壁纸越明显。图片壁纸会按采样亮度自动抬升下限，
        保证正文对比度不低于 4.5:1（WCAG AA）。
      </p>
      <div class="wall-row wall-veil">
        <input
          v-model.number="veil"
          class="wall-veil__range"
          type="range"
          :min="VEIL_MIN"
          :max="VEIL_MAX"
          step="1"
          aria-label="面板遮罩强度"
          @input="onVeilInput"
        />
        <span class="wall-veil__value">{{ veil }}%</span>
        <span v-if="guardApplied" class="wall-veil__guard">已按对比度守卫抬到 {{ appliedAlpha }}%</span>
      </div>
    </section>

    <div v-if="immersive" class="wall-immersive" @click="immersive = false">
      <video v-if="videoUrl" :src="videoUrl" class="wall-immersive__media" autoplay muted loop playsinline></video>
      <img v-else-if="currentBg" :src="currentBg" class="wall-immersive__media" alt="壁纸预览" />
      <span class="wall-immersive__exit">点击任意处退出沉浸预览</span>
    </div>
  </div>
</template>

<script setup lang="ts">
import { onMounted, ref } from 'vue'
import Icon from '@/components/Icon.vue'
import {
  WALLPAPER_PANEL_ALPHA_MAX,
  WALLPAPER_PANEL_ALPHA_MIN,
  WALLPAPER_PRESETS,
  applyWallpaperGuarded,
  applyWallpaperVeil,
  clearStoredWallpaper,
  clearWallpaperFromBody,
  clampPanelAlpha,
  isGradient,
  persistImage,
  persistPreset,
  persistWallpaperVeil,
  readStoredWallpaper,
  readWallpaperVeil,
  resolveStoredCss,
  type WallpaperPreset,
} from '@/utils/wallpaper'

type Preset = WallpaperPreset
// 预设库 / 持久化 / 冷启动恢复统一在 `utils/wallpaper.ts`（App.vue 也用它），
// 避免「壁纸页能恢复、其它入口不能」的两套逻辑。
const presets = WALLPAPER_PRESETS

/** 遮罩滑块量程（与 store 的守卫量程同源） */
const VEIL_MIN = WALLPAPER_PANEL_ALPHA_MIN
const VEIL_MAX = WALLPAPER_PANEL_ALPHA_MAX

const activeId = ref('')
const note = ref('')
const localInfo = ref('')
const localWallpaper = ref('')
const videoUrl = ref('')
const currentBg = ref('')
const immersive = ref(false)
const veil = ref(readWallpaperVeil())
const appliedAlpha = ref(veil.value)
const guardApplied = ref(false)

/**
 * 应用壁纸 = 背景 + 对比度守卫后的面板遮罩。
 * 守卫只对图片壁纸生效（渐变壁纸的色值来自主题令牌，对比度天然成立）。
 */
async function applyToBody(bg: string): Promise<void> {
  currentBg.value = bg
  const result = await applyWallpaperGuarded(bg, veil.value)
  appliedAlpha.value = result.applied
  guardApplied.value = result.guard > 0 && result.applied > clampPanelAlpha(veil.value)
}

/** 拖动滑块：即时生效并持久化（不重算壁纸选择，避免重复写 localStorage） */
function onVeilInput(): void {
  veil.value = clampPanelAlpha(veil.value)
  persistWallpaperVeil(veil.value)
  if (currentBg.value) {
    void applyToBody(currentBg.value)
  } else {
    // 没有壁纸时也要记住强度：CSS 变量先写好，等用户选壁纸即按该强度呈现。
    applyWallpaperVeil(veil.value)
  }
}

/// 缩略图用的 `background` 简写。
/// 预设里既有渐变（直接可用）也有图片路径（必须包 `url()`，否则整条声明非法、格子空白）。
function styleOf(css: string): string {
  return isGradient(css) ? css : `url("${css}") center / cover no-repeat`
}

function applyPreset(p: Preset, opts: { silent?: boolean } = {}): void {
  activeId.value = p.id
  localWallpaper.value = p.css
  videoUrl.value = ''
  applyToBody(p.css)
  persistPreset(p.id)
  if (!opts.silent) note.value = '已应用预设壁纸：' + p.label
}

function clearWallpaper(): void {
  activeId.value = ''
  localWallpaper.value = ''
  videoUrl.value = ''
  currentBg.value = ''
  clearWallpaperFromBody()
  clearStoredWallpaper()
  note.value = '壁纸已清除'
}

function onImagePick(e: Event): void {
  const input = e.target as HTMLInputElement
  const file = input.files?.[0]
  if (!file) return
  const reader = new FileReader()
  reader.onload = () => {
    const dataUrl = String(reader.result ?? '')
    videoUrl.value = ''
    localWallpaper.value = dataUrl
    applyToBody(dataUrl)
    const persisted = persistImage(dataUrl)
    localInfo.value = '已选：' + file.name + '（' + Math.round(file.size / 1024) + ' KB）— ' + (persisted ? '已持久化，刷新后保留' : '超过 2MB，仅当前会话生效')
    note.value = '已应用本地图片壁纸'
  }
  reader.readAsDataURL(file)
  input.value = ''
}

function onVideoPick(e: Event): void {
  const input = e.target as HTMLInputElement
  const file = input.files?.[0]
  if (!file) return
  if (videoUrl.value) URL.revokeObjectURL(videoUrl.value)
  const url = URL.createObjectURL(file)
  videoUrl.value = url
  localWallpaper.value = url
  activeId.value = ''
  localInfo.value = '已选视频（动态壁纸）：' + file.name + '（' + Math.round(file.size / 1024) + ' KB）— 仅当前会话生效，点击「沉浸式预览」循环播放'
  note.value = '已加载动态壁纸（视频循环）'
  input.value = ''
}

onMounted(() => {
  // 先把遮罩强度写到 CSS 变量：即使没有壁纸，面板默认值也与用户选择一致。
  applyWallpaperVeil(veil.value)
  const stored = readStoredWallpaper()
  if (!stored) return
  const css = resolveStoredCss(stored)
  if (!css) {
    // 预设已下线（改版）：清掉脏数据，避免每次进来都空转。
    clearStoredWallpaper()
    return
  }
  if (stored.type === 'preset') {
    const preset = presets.find(p => p.id === stored.id)
    if (preset) {
      // 静默恢复：只还原背景与「使用中」徽标，不弹「已应用」提示。
      applyPreset(preset, { silent: true })
      localInfo.value = '已恢复上次选择的预设壁纸：' + preset.label
      return
    }
  }
  localWallpaper.value = css
  localInfo.value = '已恢复上次会话的本地图片壁纸'
  applyToBody(css)
})
</script>

<style scoped>
.wall-stage { padding: 24px 28px; max-width: 1100px; margin: 0 auto; }
.wall-note { margin: 12px 0; padding: 10px 14px; background: color-mix(in srgb, var(--accent) 10%, var(--card)); border: 1px solid color-mix(in srgb, var(--accent) 25%, transparent); border-radius: var(--radius-sm); color: var(--accent); font-size: 12px; }
.wall-card { margin-top: 16px; padding: 14px; background: var(--card); border: 1px solid var(--hairline); border-radius: var(--radius-md); }
.wall-card__title { font-size: 13px; font-weight: 600; margin-bottom: 8px; }
.wall-card__hint { font-size: 11px; color: var(--text-dim); margin: 6px 0 0; }
.wall-grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(170px, 1fr)); gap: 10px; }
.wall-tile { position: relative; height: 96px; border-radius: var(--radius-md); border: 1px solid var(--hairline); cursor: pointer; overflow: hidden; transition: transform var(--dur-fast), border-color var(--dur-fast); }
.wall-tile:hover { transform: translateY(-2px); border-color: var(--accent); }
.wall-tile__label { position: absolute; left: 8px; bottom: 8px; font-size: 11px; color: var(--text); background: color-mix(in srgb, var(--bg) 60%, transparent); padding: 2px 8px; border-radius: 999px; }
.wall-tile__badge { position: absolute; top: 6px; right: 6px; font-size: 10px; padding: 2px 7px; border-radius: 999px; background: color-mix(in srgb, var(--accent) 20%, var(--bg)); color: var(--accent); }
.wall-row { display: flex; gap: 10px; flex-wrap: wrap; align-items: center; margin-top: 8px; }
.wall-veil__range { flex: 1 1 220px; min-width: 160px; accent-color: var(--accent); }
.wall-veil__value { font-family: var(--font-mono); font-size: 12px; color: var(--text); min-width: 40px; }
.wall-veil__guard { font-size: 11px; color: var(--warn); }
.wall-file { position: relative; cursor: pointer; }
.wall-file__input { position: absolute; inset: 0; opacity: 0; cursor: pointer; }
.wall-immersive { position: fixed; inset: 0; z-index: 10000; background: var(--bg); display: flex; align-items: center; justify-content: center; cursor: zoom-out; }
.wall-immersive__media { max-width: 100%; max-height: 100%; object-fit: contain; }
.wall-immersive__exit { position: absolute; bottom: 18px; left: 50%; transform: translateX(-50%); font-size: 12px; color: var(--text-dim); background: color-mix(in srgb, var(--bg) 70%, transparent); padding: 4px 12px; border-radius: 999px; }
</style>