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
      <div class="wall-card__title">预设壁纸库（内置 5 款，主题令牌渐变，零外部资源）</div>
      <div class="wall-grid">
        <button v-for="p in presets" :key="p.id" class="wall-tile" :style="{ background: p.css }" @click="applyPreset(p)">
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

interface Preset { id: string; label: string; css: string }
const presets: Preset[] = [
  { id: 'aurora', label: '极光 · 蓝橙', css: 'linear-gradient(135deg, color-mix(in srgb, var(--accent) 85%, var(--bg)) 0%, color-mix(in srgb, var(--accent) 25%, var(--bg)) 45%, var(--bg) 100%)' },
  { id: 'dusk', label: '黄昏 · 墨彩', css: 'linear-gradient(160deg, var(--bg) 0%, color-mix(in srgb, var(--warn) 22%, var(--bg)) 55%, color-mix(in srgb, var(--accent) 12%, var(--bg)) 100%)' },
  { id: 'ink', label: '水墨 · 青灰', css: 'linear-gradient(120deg, var(--bg) 20%, color-mix(in srgb, var(--info) 16%, var(--bg)) 60%, var(--card) 100%)' },
  { id: 'ember', label: '余烬 · 暖橙', css: 'linear-gradient(145deg, color-mix(in srgb, var(--warn) 30%, var(--bg)) 0%, var(--bg) 70%)' },
  { id: 'night', label: '夜航 · 深蓝', css: 'radial-gradient(120% 90% at 20% 10%, color-mix(in srgb, var(--info) 30%, var(--bg)) 0%, var(--bg) 60%)' },
]

const STORAGE_KEY = 'marguerite.wallpaper'
const MAX_PERSIST_BYTES = 2 * 1024 * 1024
const activeId = ref('')
const note = ref('')
const localInfo = ref('')
const localWallpaper = ref('')
const videoUrl = ref('')
const currentBg = ref('')
const immersive = ref(false)

function applyToBody(bg: string): void {
  currentBg.value = bg
  document.body.style.backgroundImage = (bg.startsWith('linear') || bg.startsWith('radial')) ? bg : 'url("' + bg + '")'
  document.body.style.backgroundSize = 'cover'
  document.body.style.backgroundAttachment = 'fixed'
}

function applyPreset(p: Preset): void {
  activeId.value = p.id
  localWallpaper.value = p.css
  videoUrl.value = ''
  applyToBody(p.css)
  note.value = '已应用预设壁纸：' + p.label
}

function clearWallpaper(): void {
  activeId.value = ''
  localWallpaper.value = ''
  videoUrl.value = ''
  currentBg.value = ''
  document.body.style.backgroundImage = ''
  document.body.style.backgroundSize = ''
  document.body.style.backgroundAttachment = ''
  try { localStorage.removeItem(STORAGE_KEY) } catch { /* 忽略 */ }
  note.value = '壁纸已清除'
}

function persistImage(dataUrl: string): boolean {
  const bytes = Math.ceil((dataUrl.length - dataUrl.indexOf(',') - 1) * 3 / 4)
  if (bytes > MAX_PERSIST_BYTES) return false
  try { localStorage.setItem(STORAGE_KEY, dataUrl); return true } catch { return false }
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
  try {
    const saved = localStorage.getItem(STORAGE_KEY)
    if (saved) {
      localWallpaper.value = saved
      localInfo.value = '已恢复上次会话的本地图片壁纸'
      applyToBody(saved)
    }
  } catch { /* 忽略 */ }
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
.wall-file { position: relative; cursor: pointer; }
.wall-file__input { position: absolute; inset: 0; opacity: 0; cursor: pointer; }
.wall-immersive { position: fixed; inset: 0; z-index: 10000; background: var(--bg); display: flex; align-items: center; justify-content: center; cursor: zoom-out; }
.wall-immersive__media { max-width: 100%; max-height: 100%; object-fit: contain; }
.wall-immersive__exit { position: absolute; bottom: 18px; left: 50%; transform: translateX(-50%); font-size: 12px; color: var(--text-dim); background: color-mix(in srgb, var(--bg) 70%, transparent); padding: 4px 12px; border-radius: 999px; }
</style>