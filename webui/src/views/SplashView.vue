<script setup lang="ts">
import { computed, onMounted, ref, onBeforeUnmount } from 'vue'
import { useRouter } from 'vue-router'
import { useRequest } from '@/composables/useRequest'
import MargheritaCrest from '@/components/splash/MargheritaCrest.vue'

const router = useRouter()

// Runtime version from the same `status` RPC the overview uses; degrades to a
// generic tagline when the gateway is unreachable (never a hard-coded version).
const { data: splashStatus } = useRequest<{ version?: string }>(
  'status',
  undefined,
  { errorLabel: 'Splash status unavailable', toastOnError: false },
)
const versionLine = computed<string>(() =>
  splashStatus.value?.version ? `v${splashStatus.value.version}` : 'agent workspace',
)
const entering = ref(false)
const bgOpacity = ref(1)
const bgBlur = ref(0)
const logoOpacity = ref(0)
const inputOpacity = ref(0)
const progress = ref(0)
const nowTime = ref('')
// Frosted-glass cue: the boot background fades to light frosted glass the moment
// the boot music starts playing.
const frostOn = ref(false)
// Boot audio is an original CC0 program-synthesized track (public/music/dusk-pad),
// never redistributed copyrighted media. See music workbench release hygiene.
const BOOT_AUDIO = '/boot/boot.mp3'
// 启动动画背景视频。
//
// 2026-10-22 第二轮（真机反馈"视频没有播放"）：
// 用户提供的 `bg-v2.mp4` 编码是 **HEVC / H.265（hvc1）**，而 WebView2（Chromium 内核）
// **默认不含 HEVC 解码器**（专利许可原因，依赖系统 HEVC 扩展 + 硬件解码）。
// 实测后果：`<video>` 立刻触发 error → `videoOk=false` → 视频永不显示，
// 用户只看到极光和字幕 —— 正是"没有播放就直接显示字幕"。
//
// 修复：用 ffmpeg 转出两份**所有 Chromium 都能解**的格式，按优先级排列：
//   · `bg-v2-h264.mp4` — H.264 Main / yuv420p / faststart（最高兼容性，主选）
//   · `bg-v2.webm`     — VP9（Chromium 原生支持，体积更小）
// 用 <source> 多源而不是单个 src：浏览器按顺序挑第一个能解的，
// 两个都失败才触发 video 的 error → 回退极光。
//
// 顺带说明为什么不再引用 `/boot/splash-cyber.mp4`：那个文件**从来不存在**
// （注释自承 personal-use 未随包分发），等于启动动画一直没有视频。
const BG_VIDEO_SOURCES = [
  { src: '/splash/bg-v2-h264.mp4', type: 'video/mp4; codecs=avc1.4D401F' },
  { src: '/splash/bg-v2.webm', type: 'video/webm; codecs=vp9' },
]
// The optional background video (personal-use Hero footage, NOT shipped) renders
// only if it actually loads; otherwise the token-tinted aurora is the background.
const videoOk = ref(false)
/**
 * 自动进入的定时器。
 *
 * 2026-10-22 第二轮：原实现固定 6 秒后进；而背景视频是 30 秒，
 * 等于只看到开头 1/5。既然需求是"把这个视频作为启动动画"，
 * 就该给它能被看见的时长（10 秒），同时保留"随时可跳过"。
 *
 * 反向也重要：**视频没播出来时不该让用户等 10 秒**。
 * 所以在 onVideoError 里把等待缩短到 2 秒 —— 盯着一片极光等 10 秒是纯损失。
 */
let autoEnterTimer: number | null = null
/**
 * 是否已经因为"视频全源失败"而缩短过等待。
 *
 * 为什么必须有这个标记（实测到的真缺陷）：`<source>` 的 error 可能在
 * `onMounted` **执行到末尾之前**就触发（浏览器解 source 是异步的，但可能在
 * 同步微任务里就失败）。此时 `onVideoError` 先把定时器缩到 2s，
 * 紧接着 `onMounted` 末尾的 `scheduleAutoEnter(AUTO_ENTER_MS)` 又把它**覆盖回 10s** ——
 * 缩短形同虚设。测试 "视频全源失败时缩短自动进入等待" 正是抓这个。
 */
let autoEnterShrunk = false
const AUTO_ENTER_MS = 10_000
const AUTO_ENTER_NO_VIDEO_MS = 2_000

function scheduleAutoEnter(ms: number) {
  if (autoEnterTimer !== null) clearTimeout(autoEnterTimer)
  autoEnterTimer = window.setTimeout(enter, ms)
}

function onVideoReady() {
  videoOk.value = true
}
function onVideoError() {
  videoOk.value = false
  // 所有候选源都解不开时不干等：缩短自动进入时间，并记住"已缩短"，
  // 避免 onMounted 末尾的默认调度把它覆盖回去。
  autoEnterShrunk = true
  scheduleAutoEnter(AUTO_ENTER_NO_VIDEO_MS)
}
const audioCtx: { value: AudioContext | null } = { value: null }
const audioEl: { value: HTMLAudioElement | null } = { value: null }
const rafId: { value: number | null } = { value: null }
const analyser: { value: AnalyserNode | null } = { value: null }
const waveform = ref<number[]>(new Array(64).fill(0))

let clockTimer: number | null = null

function tickClock() {
  const d = new Date()
  const hh = String(d.getHours()).padStart(2, '0')
  const mm = String(d.getMinutes()).padStart(2, '0')
  const ss = String(d.getSeconds()).padStart(2, '0')
  nowTime.value = `${hh}:${mm}:${ss}`
}

function tickWaveform() {
  if (analyser.value) {
    const buf = new Uint8Array(analyser.value.frequencyBinCount)
    analyser.value.getByteFrequencyData(buf)
    const n = 64
    const step = Math.floor(buf.length / n)
    const arr: number[] = []
    for (let i = 0; i < n; i++) arr.push(buf[i * step] / 255)
    waveform.value = arr
  }
  rafId.value = requestAnimationFrame(tickWaveform)
}

function enter() {
  if (entering.value) return
  entering.value = true
  // Fade out audio
  if (audioEl.value) {
    const a = audioEl.value
    const fade = setInterval(() => {
      a.volume = Math.max(0, a.volume - 0.05)
      if (a.volume <= 0.01) { a.pause(); clearInterval(fade) }
    }, 50)
  }
  setTimeout(() => router.replace('/chat'), 600)
}

/**
 * Esc 跳过（AC4）。
 *
 * 为什么必须有键盘出口：窗口是 `decorations: false`，标题栏由 webui 自绘
 * （`TitleBarControls`），而启动动画期间外壳被隐藏 ⇒ **没有任何关闭/拖动按钮**。
 * 如果此时程序卡住，只有键盘出口能让用户脱身。这条不是"锦上添花"，
 * 是这套"覆盖整个壳"设计的必要安全阀。
 */
function onKeydown(e: KeyboardEvent) {
  if (e.key === 'Escape') {
    e.preventDefault()
    enter()
  }
}

/**
 * 启动动画期间隐藏外壳（侧栏 / 顶栏 / 侧栏拖柄），实现"覆盖整个壳"（G1）。
 *
 * 为什么用 `html[data-splash]` + CSS 而不是在 App.vue 写 `v-if`：
 *   · App.vue 的 sidebar/topbar 是多根模板的顶层节点，加 v-if 要改结构；
 *   · 更关键的是**有其它代码依赖这些 DOM 存在**（布局测量、快捷键作用域等），
 *     直接不渲染会让它们拿到 null。用 CSS `display:none` 保留 DOM，
 *     同时元素**不可聚焦、不进无障碍树**——正是这里要的语义。
 */
function showSplashChrome() {
  document.documentElement.dataset.splash = '1'
  window.addEventListener('keydown', onKeydown)
}
function hideSplashChrome() {
  delete document.documentElement.dataset.splash
  window.removeEventListener('keydown', onKeydown)
}

onMounted(async () => {
  showSplashChrome()
  clockTimer = window.setInterval(tickClock, 1000)
  tickClock()

  // Sequence: logo fade in at 400ms, audio starts at 1200ms, bg fades+blurs 1500-4500ms, input at 3500ms
  setTimeout(() => { logoOpacity.value = 1 }, 400)

  setTimeout(async () => {
    try {
      const el = new Audio(BOOT_AUDIO)
      el.loop = true
      el.volume = 0
      audioEl.value = el
      await el.play()
      // Fade the whole boot background to light frosted glass now that music plays.
      frostOn.value = true
      // Ramp volume to 0.3
      const ramp = setInterval(() => {
        el.volume = Math.min(0.3, el.volume + 0.03)
        if (el.volume >= 0.3) clearInterval(ramp)
      }, 60)

      // Wire up visualizer
      const ctx = new AudioContext()
      const src = ctx.createMediaElementSource(el)
      const an = ctx.createAnalyser()
      an.fftSize = 256
      src.connect(an)
      an.connect(ctx.destination)
      analyser.value = an
      tickWaveform()
    } catch {
      // Autoplay blocked — silent splash still works
    }
  }, 1200)

  // Fallback: even if audio is autoplay-blocked/missing, the frosted-glass aurora
  // still activates with the visual transition so the splash never looks unlit.
  setTimeout(() => {
    frostOn.value = true
  }, 1500)

  // Background transition: 100% -> 70%, blur 0 -> 2px over 3s.
  //
  // 2026-10-22：原值是 100% -> 25% + blur 18px。视频换成 bg-v2.mp4 之后，
  // 那组终值会把视频糊成一片几乎看不见的背景（18px 模糊 + 25% 透明度），
  // 与"把视频作为启动动画"的诉求相反。改为轻度过场：视频清晰可见，
  // 中央品牌字仍可读（顶部/底部渐变压暗提供了对比度）。
  setTimeout(() => {
    const start = performance.now()
    const dur = 3000
    const step = (t: number) => {
      const p = Math.min(1, (t - start) / dur)
      bgOpacity.value = 1 - 0.3 * p
      bgBlur.value = 2 * p
      progress.value = p * 100
      if (p < 1) requestAnimationFrame(step)
    }
    requestAnimationFrame(step)
  }, 1500)

  setTimeout(() => { inputOpacity.value = 1 }, 3500)

  // 自动进入（可被 onVideoError 缩短，见 scheduleAutoEnter）。
  // 若视频已经全源失败（autoEnterShrunk），不要覆盖成默认的 10s。
  if (!autoEnterShrunk) scheduleAutoEnter(AUTO_ENTER_MS)
})

onBeforeUnmount(() => {
  hideSplashChrome()
  if (autoEnterTimer !== null) clearTimeout(autoEnterTimer)
  if (clockTimer) clearInterval(clockTimer)
  if (rafId.value) cancelAnimationFrame(rafId.value)
  if (audioEl.value) { audioEl.value.pause(); audioEl.value = null }
  if (audioCtx.value) audioCtx.value.close().catch(() => {})
})
</script>

<template>
  <div class="splash-root" @click="enter">
    <!-- Optional background video: personal-use Hero footage is NOT shipped, so it
         renders only when the file actually loads; the aurora below is the
         always-present fallback (v-if hides a 404 <video> so there is no broken box). -->
    <video
      v-show="videoOk"
      class="splash-bg"
      autoplay
      muted
      loop
      playsinline
      preload="auto"
      :style="{ opacity: bgOpacity, filter: `blur(${bgBlur}px)` }"
      @loadeddata="onVideoReady"
      @canplay="onVideoReady"
      @error="onVideoError"
    >
      <!-- 多源按兼容性排序：H.264（WebView2/所有 Chromium 必解）→ VP9/WebM。
           全部失败才回退极光。
           ⚠️ HTML 陷阱：用多个 <source> 时，**全部候选失败不会触发 <video> 的
           error 事件**（video 元素只把 networkState 置为 NETWORK_NO_SOURCE）。
           所以回退判据必须挂在**最后一个 source** 上 —— 只挂 video@error 会永远收不到，
           表现就是"视频区一片空白且永不回退"。 -->
      <source
        v-for="(s, i) in BG_VIDEO_SOURCES"
        :key="s.src"
        :src="s.src"
        :type="s.type"
        @error="i === BG_VIDEO_SOURCES.length - 1 ? onVideoError() : undefined"
      />
    </video>

    <!-- Hero-MV aurora: drifting token-tinted blobs + god-ray sweep. Purely
         decorative and token-coloured so it degrades gracefully when bg.mp4 is
         missing and carries no colour literals (CI colour guard). -->
    <div class="splash-aurora" :class="{ 'splash-aurora--on': frostOn }">
      <span class="aurora-blob aurora-blob--a"></span>
      <span class="aurora-blob aurora-blob--b"></span>
      <span class="aurora-blob aurora-blob--c"></span>
      <div class="splash-rays"></div>
    </div>

    <!-- Light frosted-glass veil: fades in as the boot music starts. -->
    <div class="splash-frost" :class="{ 'splash-frost--on': frostOn }"></div>

    <!-- Dark gradient overlays -->
    <div class="splash-top-grad"></div>
    <div class="splash-bottom-grad"></div>

    <!-- 跳过按钮（右上角，透明底）。
         这是启动动画期间**唯一的显式出口**：窗口 decoratons:false、外壳被隐藏，
         若没有它，用户只能等 6 秒自动进入。键盘可达（真实 <button> 语义），
         并配 Esc（见 onKeydown）。视觉上刻意低调：默认近乎透明，仅在
         hover / focus-visible 时浮现，避免抢走视频的注意力。 -->
    <button
      type="button"
      class="splash-skip"
      data-testid="splash-skip"
      aria-label="跳过启动动画"
      @click.stop="enter"
    >
      跳过 SKIP
    </button>

    <!-- HUD: top-left -->
    <div class="hud hud-tl">
      <div class="hud-title">MARGUERITE</div>
      <div class="hud-sub">{{ versionLine }}</div>
    </div>

    <!-- HUD: top-right -->
    <div class="hud hud-tr">
      <div class="hud-time">{{ nowTime }}</div>
      <div class="hud-bpm">150 BPM</div>
      <div class="hud-progress">
        <div class="hud-progress-bar" :style="{ width: progress + '%' }"></div>
      </div>
    </div>

    <!-- HUD: bottom-left -->
    <div class="hud hud-bl">
      <div class="hud-lr">
        <span>L</span><div class="lr-bar"></div><div class="lr-bar"></div><span>R</span>
      </div>
      <div class="hud-sessions">SESSIONS 03</div>
    </div>

    <!-- HUD: bottom-right -->
    <div class="hud hud-br">
      <div class="hud-dolby">DOLBY ATMOS · BINAURAL</div>
      <div class="hud-flight">ALT 00515 · V 6.8 M/S</div>
    </div>

    <!-- Center: 徽记 + 字幕。
         2026-10-22 第二轮（需求「字幕背景要改变」）：
         原来字幕直接压在视频上，`text-shadow` 只在纯色底上够用；视频一旦真的开始
         播放（HEVC 已转 H.264 修复），亮场景会让白字完全糊掉。现在给字幕一个
         **独立的承载层**：极窄的内衬底 + 上下渐隐边 + 细描边，保证任何画面下都可读，
         同时不挡住视频（承载层只有字幕那么宽，不是整块蒙版）。 -->
    <div class="splash-center" :style="{ opacity: logoOpacity }">
      <MargheritaCrest class="splash-crest" :size="92" />
      <div class="splash-logo">MARGUERITE</div>
      <div class="splash-tagline">许愿式智能体工作台</div>
      <div class="splash-input" :style="{ opacity: inputOpacity }">
        ✨ 说点什么...
      </div>
    </div>

    <!-- Bottom waveform -->
    <div class="splash-wave">
      <div
        v-for="(v, i) in waveform"
        :key="i"
        class="wave-bar"
        :style="{ height: Math.max(2, v * 48) + 'px' }"
      ></div>
    </div>
  </div>
</template>

<style scoped>
.splash-root {
  position: fixed;
  inset: 0;
  background: var(--bg-surface);
  overflow: hidden;
  cursor: pointer;
  font-family: 'JetBrains Mono', 'SF Mono', Consolas, monospace;
  color: var(--text);
  user-select: none;
}

.splash-bg {
  position: absolute;
  inset: 0;
  width: 100%;
  height: 100%;
  object-fit: cover;
  transition: opacity var(--dur-fast) linear;
}

.splash-top-grad {
  position: absolute;
  top: 0; left: 0; right: 0;
  height: 25%;
  background: linear-gradient(to bottom, color-mix(in srgb, var(--bg-surface) 80%, transparent), transparent);
}
.splash-bottom-grad {
  position: absolute;
  bottom: 0; left: 0; right: 0;
  height: 25%;
  background: linear-gradient(to top, color-mix(in srgb, var(--bg-surface) 85%, transparent), transparent);
}

/* ---- Hero-MV aurora + god-rays ------------------------------------------ */
.splash-aurora {
  position: absolute;
  inset: -10%;
  filter: blur(60px);
  opacity: 0;
  transition: opacity var(--dur-slow) var(--ease-standard);
  z-index: 1;
}
.splash-aurora--on {
  opacity: 0.6;
}
.aurora-blob {
  position: absolute;
  width: 46vmax;
  height: 46vmax;
  border-radius: 50%;
  mix-blend-mode: screen;
  animation: splash-drift 26s var(--ease-standard) infinite alternate;
}
.aurora-blob--a {
  top: -8%; left: -6%;
  background: radial-gradient(circle at 50% 50%,
    color-mix(in srgb, var(--accent) 70%, transparent), transparent 70%);
}
.aurora-blob--b {
  bottom: -12%; right: -8%;
  background: radial-gradient(circle at 50% 50%,
    color-mix(in srgb, var(--bloom-accent, var(--accent)) 60%, transparent), transparent 70%);
  animation: splash-drift calc(var(--dur-pulse) * 17.8) var(--ease-standard) infinite alternate-reverse;
}
.aurora-blob--c {
  top: 30%; left: 35%;
  background: radial-gradient(circle at 50% 50%,
    color-mix(in srgb, var(--accent-secondary, var(--text-dim)) 55%, transparent), transparent 70%);
  animation: splash-drift calc(var(--dur-pulse) * 21.1) var(--ease-standard) infinite alternate;
}
@keyframes splash-drift {
  0% { transform: translate3d(0, 0, 0) scale(1); }
  50% { transform: translate3d(4vmax, -3vmax, 0) scale(1.12); }
  100% { transform: translate3d(-3vmax, 4vmax, 0) scale(0.96); }
}
.splash-rays {
  position: absolute;
  inset: 0;
  background: conic-gradient(from 210deg at 50% -10%,
    transparent 0deg,
    color-mix(in srgb, var(--accent) 22%, transparent) 18deg,
    transparent 40deg,
    color-mix(in srgb, var(--accent) 16%, transparent) 70deg,
    transparent 96deg,
    color-mix(in srgb, var(--bloom-accent, var(--accent)) 20%, transparent) 130deg,
    transparent 160deg,
    transparent 360deg);
  animation: splash-rays-spin 60s linear infinite;
  opacity: 0.7;
}
@keyframes splash-rays-spin {
  to { transform: rotate(360deg); }
}

/* ---- Light frosted-glass veil (boot) ----------------------------------- */
.splash-frost {
  position: absolute;
  inset: 0;
  z-index: 1;
  opacity: 0;
  pointer-events: none;
  background: color-mix(in srgb, var(--bg-surface) 55%, transparent);
  backdrop-filter: blur(14px);
  -webkit-backdrop-filter: blur(14px);
  transition: opacity var(--dur-slow) var(--ease-standard),
    backdrop-filter var(--dur-slow) var(--ease-standard),
    -webkit-backdrop-filter var(--dur-slow) var(--ease-standard);
}
.splash-frost--on {
  opacity: 1;
}
@media (prefers-reduced-transparency: reduce) {
  .splash-frost {
    backdrop-filter: none;
    -webkit-backdrop-filter: none;
  }
}
@media (prefers-reduced-motion: reduce) {
  .aurora-blob, .splash-rays { animation: none; }
}

.hud {
  position: absolute;
  font-size: 11px;
  letter-spacing: 0.15em;
  color: color-mix(in srgb, var(--text) 70%, transparent);
  text-transform: uppercase;
  z-index: 2;
}
.hud-tl { top: 28px; left: 32px; }
/* 顶右 HUD 让位给跳过按钮（按钮占 20..48 的高度带），否则两者会重叠。 */
.hud-tr { top: 64px; right: 32px; text-align: right; }
.hud-bl { bottom: 28px; left: 32px; }
.hud-br { bottom: 28px; right: 32px; text-align: right; }

.hud-title { font-size: 13px; font-weight: 600; color: var(--text); letter-spacing: 0.3em; }
.hud-sub { font-size: 10px; margin-top: 4px; color: color-mix(in srgb, var(--text) 40%, transparent); }
.hud-time { font-size: 13px; color: var(--text); }
.hud-bpm { font-size: 10px; margin-top: 4px; color: color-mix(in srgb, var(--accent) 90%, transparent); }
.hud-progress {
  width: 120px; height: 2px; background: color-mix(in srgb, var(--text) 15%, transparent);
  margin-top: 8px; margin-left: auto;
}
.hud-progress-bar {
  height: 100%; background: var(--accent); transition: width var(--dur-fast) linear;
}
.hud-lr { display: flex; align-items: center; gap: 6px; font-size: 10px; }
.lr-bar { width: 40px; height: 2px; background: color-mix(in srgb, var(--text) 30%, transparent); }
.hud-sessions { font-size: 10px; margin-top: 6px; color: color-mix(in srgb, var(--text) 40%, transparent); }
.hud-dolby { font-size: 10px; color: color-mix(in srgb, var(--text) 40%, transparent); }
.hud-flight { font-size: 10px; margin-top: 4px; color: color-mix(in srgb, var(--accent) 70%, transparent); }

.splash-center {
  position: absolute;
  top: 50%; left: 50%;
  transform: translate(-50%, -50%);
  text-align: center;
  z-index: 3;
  /* 字幕承载层（需求「字幕背景要改变」）。
     视频真的开始播放后，亮场景会让纯白字彻底糊掉 —— text-shadow 只能救纯色底。
     这里用**窄内衬 + 上下渐隐**：底衬只包住字幕本身（不是整屏蒙版，视频仍完整可见），
     两侧羽化到透明，看起来像一条压在画面上的光带而不是一块灰色标签。 */
  display: flex;
  flex-direction: column;
  align-items: center;
  padding: 34px 76px 30px;
  /* 用 token，不带 px 兜底 —— radius 守卫把 `var(--x, 24px)` 的兜底值判为
     硬编码（它对，兜底值就是第二份真源）。该 token 由 base.css 定义，必定存在。 */
  border-radius: var(--radius-xl);
  background:
    radial-gradient(
      120% 140% at 50% 45%,
      color-mix(in srgb, var(--bg-surface) 62%, transparent) 0%,
      color-mix(in srgb, var(--bg-surface) 34%, transparent) 45%,
      transparent 78%
    );
  /* 上下细边只作为"承载"的暗示，不做封闭框 —— 封闭框会像弹窗 */
  box-shadow:
    inset 0 1px 0 color-mix(in srgb, var(--text) 12%, transparent),
    inset 0 -1px 0 color-mix(in srgb, var(--text) 8%, transparent);
  backdrop-filter: blur(2px);
  -webkit-backdrop-filter: blur(2px);
}

.splash-crest {
  margin-bottom: 22px;
  filter: drop-shadow(0 6px 18px color-mix(in srgb, var(--bg-surface) 55%, transparent));
}

.splash-logo {
  font-size: 42px;
  font-weight: 200;
  /* 字距从 0.5em 收到 0.34em：monogram 已经在视觉上承担了"标记"的角色，
     文字再拉那么开会显得松散。 */
  letter-spacing: 0.34em;
  color: var(--text);
  text-shadow:
    0 0 40px color-mix(in srgb, var(--accent) 50%, transparent),
    0 1px 2px color-mix(in srgb, var(--bg-surface) 70%, transparent);
  transition: opacity var(--dur-enter) var(--ease-out);
  margin-left: 0.34em;
}

.splash-tagline {
  margin-top: 14px;
  font-size: 13px;
  letter-spacing: 0.4em;
  /* 从 50% 提到 76%：承载层已经保证了背景对比度，副标题不该再靠低透明度"显得高级"，
     那只会让它在视频上更难读。 */
  color: color-mix(in srgb, var(--text) 76%, transparent);
  transition: opacity var(--dur-enter) var(--ease-out);
  margin-left: 0.4em;
}
.splash-input {
  margin-top: 48px;
  display: inline-block;
  padding: 14px 40px;
  border: 1px solid color-mix(in srgb, var(--accent) 40%, transparent);
  border-radius: var(--radius-pill);
  font-size: 14px;
  letter-spacing: 0.1em;
  color: color-mix(in srgb, var(--text) 80%, transparent);
  background: color-mix(in srgb, var(--bg-surface) 30%, transparent);
  backdrop-filter: blur(20px);
  transition: opacity var(--dur-enter) var(--ease-out), border-color var(--dur-fast) var(--ease-standard);
  animation: pulse-border var(--dur-pulse) var(--ease-standard) infinite;
}
@keyframes pulse-border {
  0%, 100% { border-color: color-mix(in srgb, var(--accent) 30%, transparent); }
  50% { border-color: color-mix(in srgb, var(--accent) 80%, transparent); }
}

.splash-wave {
  position: absolute;
  bottom: 80px;
  left: 50%;
  transform: translateX(-50%);
  display: flex;
  align-items: flex-end;
  gap: 2px;
  height: 48px;
  z-index: 2;
}
.wave-bar {
  width: 2px;
  background: var(--accent);
  transition: height var(--dur-fast) linear;
  border-radius: var(--radius-sm);
  opacity: 0.8;
}

/* ---- 跳过按钮（右上角，透明）------------------------------------------ */
/* 全部颜色走 token / color-mix —— 裸色值会被 check-webui-colors 判违规，
   也会在深色主题下用错明度。默认态刻意接近透明，不抢视频注意力。 */
.splash-skip {
  position: absolute;
  top: 20px;
  right: 24px;
  z-index: 4;
  padding: 7px 15px;
  background: transparent;
  border: 1px solid color-mix(in srgb, var(--text) 22%, transparent);
  border-radius: var(--radius-pill);
  color: color-mix(in srgb, var(--text) 62%, transparent);
  font-family: inherit;
  font-size: 11px;
  letter-spacing: 0.12em;
  cursor: pointer;
  transition:
    color var(--dur-fast) var(--ease-standard),
    border-color var(--dur-fast) var(--ease-standard),
    background-color var(--dur-fast) var(--ease-standard);
}
/* hover 与 focus-visible 同等对待：键盘用户必须看到和鼠标用户一样明确的反馈。 */
.splash-skip:hover,
.splash-skip:focus-visible {
  color: var(--text);
  border-color: color-mix(in srgb, var(--accent) 55%, transparent);
  background-color: color-mix(in srgb, var(--accent) 14%, transparent);
  outline: none;
}
/* 用 ring 而不是 outline 保证在视频上也有足够对比度。 */
.splash-skip:focus-visible {
  box-shadow: 0 0 0 2px color-mix(in srgb, var(--accent) 70%, transparent);
}
@media (prefers-reduced-motion: reduce) {
  .splash-skip { transition: none; }
}

/* ---- 启动动画期间隐藏外壳（"覆盖整个壳"的实现）-------------------------
   为什么在 SplashView 里用 :global 而不是改 App.vue：
     · App.vue 的 sidebar/topbar 是多根模板的顶层节点，加 v-if 要改结构，且
       有其它代码依赖这些 DOM 存在（布局测量 / 快捷键作用域）——直接不渲染
       会让它们拿到 null。
     · `display:none` 保留 DOM，同时元素**不可聚焦、不进无障碍树**，
       正是"启动动画期间不应能操作外壳"要的语义。
   属性由 showSplashChrome() 在挂载时设置、卸载时清除，不污染其它路由。 */
:global(html[data-splash='1'] .sidebar),
:global(html[data-splash='1'] .topbar),
:global(html[data-splash='1'] .sidebar-resizer) {
  display: none;
}
</style>
