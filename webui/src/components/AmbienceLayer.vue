<script setup lang="ts">
import { onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { useBgm } from '@/composables/useBgm'
import MusicVisualizer from './music/MusicVisualizer.vue'
import { staticAssetUrl } from '@/utils/staticAsset'

/**
 * Full-interface ambience layer for background music (design §1 — "Constellation
 * / Aurora Glass"). Two fixed fragments, deliberately kept apart:
 *
 *   ┌─ .amb-bg     (z-index 0, BELOW all content) — drifting aurora blobs, a
 *   │               hero-MV god-ray sweep and the audio-reactive constellation
 *   │               canvas (MusicVisualizer).
 *   └─ .amb-scrim  (z-index 9999, ABOVE all chrome) — the frosted-glass wash +
 *                   backdrop blur that frosts the whole workstation the moment
 *               playback starts. It only paints; pointer-events stay none.
 *
 * On `useBgm().playing` flipping true, the scrim ramps (wash 0→24%, blur
 * 0→2px) and the aurora fades to ~0.5 over `--dur-slow`; pausing unwinds the
 * same ramp. `--amb-amp` (now the shared energy level 0..1) is published on
 * :root so the existing signature rim in App.vue and the aurora itself breathe
 * with the track. Honours prefers-reduced-transparency (drop the blur) and
 * prefers-reduced-motion (hold the drift/rays still).
 */

const bgm = useBgm()
const { playing } = bgm
// Shared analysis level (0..1) — works for both the synth pad and file tracks,
// unlike the synth-only RMS amplitude published before.
const energy = bgm.audio.energy

const active = ref(false)
// PROD 由网关 `<base>/static/music/...` 提供；DEV 直取 /music/...
const videoSrc = staticAssetUrl('music/bg-seedance-25.mp4')

let rafId = 0
let lastAmp = -1
function pumpAmp() {
  const a = playing.value ? energy.value : 0
  if (Math.abs(a - lastAmp) > 0.01) {
    lastAmp = a
    document.documentElement.style.setProperty('--amb-amp', a.toFixed(3))
  }
  rafId = requestAnimationFrame(pumpAmp)
}

function syncActive(on: boolean) {
  active.value = on
  document.body.classList.toggle('ambience-on', on)
}

watch(playing, syncActive, { immediate: true })

onMounted(() => {
  syncActive(playing.value)
  rafId = requestAnimationFrame(pumpAmp)
})

onBeforeUnmount(() => {
  if (rafId) cancelAnimationFrame(rafId)
  document.body.classList.remove('ambience-on')
})
</script>

<template>
  <!-- Layer 1: background — 专属 MV 循环视频 + aurora + god-rays + constellation canvas. -->
  <div
    class="amb-bg"
    :class="{ 'amb-bg--active': active }"
    aria-hidden="true"
  >
    <!--
      2026-10-23：Seedance 2.5 生成的 30s / 16:9 专属背景视频（随包分发）。
      只在播放时挂载（`v-if`），暂停即卸载 —— 不让一个常驻解码器白烧 CPU。
      静音：声音来自 BGM 本身；`playsinline` 避免移动端/内嵌内核强制全屏。
    -->
    <video
      v-if="active"
      class="amb-bg__video"
      :src="videoSrc"
      autoplay
      muted
      loop
      playsinline
      preload="auto"
    ></video>
    <div class="amb-bg__aurora">
      <span class="aurora-blob aurora-blob--a"></span>
      <span class="aurora-blob aurora-blob--b"></span>
      <span class="aurora-blob aurora-blob--c"></span>
    </div>
    <div class="amb-bg__rays"></div>
    <MusicVisualizer />
  </div>
  <!-- Layer 2: frosted scrim above every chrome surface. -->
  <div
    class="amb-scrim"
    :class="{ 'amb-scrim--active': active }"
    aria-hidden="true"
  ></div>
</template>

<style scoped>
/* ---- Layer 1: background -------------------------------------------------- */
.amb-bg {
  position: fixed;
  inset: 0;
  z-index: 0;
  overflow: hidden;
  pointer-events: none;
}

/* 专属 MV：低透明度铺底，和 aurora/星座叠加成"极光 + 影像"层。
   用 screen 混合让暗部不压住文字。
   2026-10-24：原来 2px 模糊 + 34% 不透明度 + 上层 scrim 9px，三层叠加后用户
   反馈「磨砂太高、看不清背景」。现在 MV 不再模糊、亮度提到 46%，把模糊预算
   全部让给 scrim 之外的独立层。 */
.amb-bg__video {
  position: absolute;
  inset: 0;
  width: 100%;
  height: 100%;
  object-fit: cover;
  opacity: 0.46;
  filter: saturate(1.12);
  mix-blend-mode: screen;
  transition: opacity var(--dur-slow) var(--ease-standard);
}

.amb-bg__aurora {
  position: absolute;
  inset: -10%;
  filter: blur(60px);
  opacity: 0;
  transition: opacity var(--dur-slow) var(--ease-standard);
}

.amb-bg--active .amb-bg__aurora {
  opacity: calc(0.5 + var(--amb-amp, 0) * 0.35);
}

.aurora-blob {
  position: absolute;
  width: 46vmax;
  height: 46vmax;
  border-radius: 50%;
  mix-blend-mode: screen;
}
/* Each blob carries its own full `animation` shorthand so the motion guard
   (which exempts `infinite` loops) sees one self-contained cadence token. */
.aurora-blob--a {
  top: -8%;
  left: -6%;
  background: radial-gradient(
    circle at 50% 50%,
    color-mix(in srgb, var(--accent) 70%, transparent),
    transparent 70%
  );
  animation: amb-drift calc(var(--dur-pulse) * 14.4) var(--ease-standard) infinite alternate;
}
.aurora-blob--b {
  bottom: -12%;
  right: -8%;
  background: radial-gradient(
    circle at 50% 50%,
    color-mix(in srgb, var(--bloom-accent, var(--accent)) 60%, transparent),
    transparent 70%
  );
  animation: amb-drift calc(var(--dur-pulse) * 17.8) var(--ease-standard) infinite alternate-reverse;
}
.aurora-blob--c {
  top: 30%;
  left: 35%;
  background: radial-gradient(
    circle at 50% 50%,
    color-mix(in srgb, var(--accent-secondary, var(--text-dim)) 55%, transparent),
    transparent 70%
  );
  animation: amb-drift calc(var(--dur-pulse) * 21.1) var(--ease-standard) infinite alternate;
}

@keyframes amb-drift {
  0% { transform: translate3d(0, 0, 0) scale(1); }
  50% { transform: translate3d(4vmax, -3vmax, 0) scale(1.12); }
  100% { transform: translate3d(-3vmax, 4vmax, 0) scale(0.96); }
}

.amb-bg__rays {
  position: absolute;
  inset: 0;
  opacity: 0;
  transition: opacity var(--dur-slow) var(--ease-standard);
  background: conic-gradient(
    from 210deg at 50% -10%,
    transparent 0deg,
    color-mix(in srgb, var(--accent) 22%, transparent) 18deg,
    transparent 40deg,
    color-mix(in srgb, var(--accent) 16%, transparent) 70deg,
    transparent 96deg,
    color-mix(in srgb, var(--bloom-accent, var(--accent)) 20%, transparent) 130deg,
    transparent 160deg,
    transparent 360deg
  );
  animation: amb-rays-spin calc(var(--dur-pulse) * 33) linear infinite;
}
.amb-bg--active .amb-bg__rays {
  opacity: calc(0.3 + var(--amb-amp, 0) * 0.35);
}
@keyframes amb-rays-spin {
  to { transform: rotate(360deg); }
}

/* ---- Layer 2: frosted scrim (above all chrome) -------------------------- */
/* 播放态只做一层很薄的"雾"，让文字不直接压在动的画面上；模糊必须极小。
   2026-10-24：原来 wash 58% + blur 9px，等于把整个界面（连同背景 MV）糊成
   一片蓝灰，用户反馈"磨砂太高、看不清背景"。现在 wash 24% + blur 2px：
   仍能稳住文字对比度，但山脊 / 粒子雏菊 / MV 都看得清。 */
.amb-scrim {
  position: fixed;
  inset: 0;
  z-index: 9999;
  pointer-events: none;
  background: color-mix(in srgb, var(--bg-surface) 0%, transparent);
  backdrop-filter: blur(0px);
  -webkit-backdrop-filter: blur(0px);
  transition:
    background var(--dur-slow) var(--ease-standard),
    backdrop-filter var(--dur-slow) var(--ease-standard),
    -webkit-backdrop-filter var(--dur-slow) var(--ease-standard);
}
.amb-scrim--active {
  background: color-mix(in srgb, var(--bg-surface) 30%, transparent);
  backdrop-filter: blur(2px);
  -webkit-backdrop-filter: blur(2px);
}

/* Reduced transparency: keep the wash, drop the blur. */
@media (prefers-reduced-transparency: reduce) {
  .amb-scrim {
    backdrop-filter: none;
    -webkit-backdrop-filter: none;
  }
}

/* Reduced motion: hold aurora/rays still; the scrim ramp itself stays (it is
   a state change, not motion). */
@media (prefers-reduced-motion: reduce) {
  .aurora-blob,
  .amb-bg__rays {
    animation: none;
  }
  /* 视频是持续运动内容：减动效偏好下直接不显示，避免不适。 */
  .amb-bg__video {
    display: none;
  }
}
</style>
