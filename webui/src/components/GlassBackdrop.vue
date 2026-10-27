<template>
  <!--
    液态玻璃的背景层：云朵 / 海面 / 沙丘三套「大块柔和纹理」。
    - 只用超大径向渐变 + 大半径模糊，**没有任何细碎噪点/纹理**（细碎纹理是廉价感的主因）；
    - 每个色块自带 filter，动画只改 transform ⇒ 模糊结果被缓存，逐帧只做合成；
    - 场景由 `useGlassScene` 统一管理（参数面板可切），持久化在 localStorage；
    - `prefers-reduced-motion` 下停止漂移，画面保持静态但层次不变。
  -->
  <div class="glass-backdrop" :data-scene="glassScene" aria-hidden="true">
    <div class="glass-backdrop__field">
      <i class="glass-blob glass-blob--1" />
      <i class="glass-blob glass-blob--2" />
      <i class="glass-blob glass-blob--3" />
      <i class="glass-blob glass-blob--4" />
      <i class="glass-blob glass-blob--5" />
    </div>
    <div class="glass-backdrop__glow" />
    <div class="glass-backdrop__vignette" />
  </div>
</template>

<script setup lang="ts">
import { glassScene } from '@/composables/useGlassScene'
</script>

<style scoped>
.glass-backdrop {
  position: fixed;
  inset: 0;
  overflow: hidden;
  pointer-events: none;
  contain: strict;
  /* 底色不是纯色：即使色块全部静止，也有一层极缓的冷色渐变托底
     （用户明确禁止"纯色背景"；这层负责在任何视口比例下都保持大块柔和过渡）。
     2026-10-24：色块中心常常落在视口外，屏幕上只剩渐变尾巴 —— 于是这里补三处
     **固定落在屏内**的光池，保证面板后面永远有可被折射的色与光。 */
  background:
    radial-gradient(70% 52% at 18% 6%,
      color-mix(in srgb, var(--accent) 30%, transparent) 0%,
      transparent 62%),
    radial-gradient(64% 48% at 86% 90%,
      color-mix(in srgb, var(--accent-secondary) 22%, transparent) 0%,
      transparent 64%),
    radial-gradient(90% 60% at 50% 118%,
      color-mix(in srgb, var(--queued) 16%, transparent) 0%,
      transparent 70%),
    var(--bg);
}

.glass-backdrop__field {
  position: absolute;
  inset: 0;
  transform: translateZ(0);
}

/* 色块：大、软、少。半径 40vw+ 的模糊椭圆是「云/海/沙丘」的公共语汇。 */
.glass-blob {
  position: absolute;
  display: block;
  border-radius: 50%;
  filter: blur(72px);
  /* 1.0：面板的暗纱压到 52% 后，背景必须"够亮够彩"，玻璃才有东西可折射；
     0.85 时透上来只剩一层灰雾。 */
  opacity: 1;
  will-change: transform;
  animation: glass-drift-a 96s cubic-bezier(0.42, 0, 0.58, 1) infinite;
}

.glass-blob--1 {
  width: 82vw;
  height: 64vh;
  left: -12vw;
  top: -18vh;
}

.glass-blob--2 {
  width: 68vw;
  height: 56vh;
  right: -14vw;
  top: 2vh;
  animation: glass-drift-b 118s cubic-bezier(0.42, 0, 0.58, 1) infinite;
  animation-delay: -22s;
}

.glass-blob--3 {
  width: 74vw;
  height: 58vh;
  left: 10vw;
  bottom: -22vh;
  animation: glass-drift-c 132s cubic-bezier(0.42, 0, 0.58, 1) infinite;
  animation-delay: -48s;
}

.glass-blob--4 {
  width: 52vw;
  height: 44vh;
  right: 4vw;
  bottom: -8vh;
  filter: blur(84px);
  animation: glass-drift-a 108s cubic-bezier(0.42, 0, 0.58, 1) infinite;
  animation-delay: -64s;
}

.glass-blob--5 {
  width: 46vw;
  height: 40vh;
  left: 34vw;
  top: 20vh;
  filter: blur(96px);
  opacity: 0.5;
  animation: glass-drift-b 142s cubic-bezier(0.42, 0, 0.58, 1) infinite;
  animation-delay: -12s;
}

/* ── 场景一：云朵（月光白 + 极光蓝，最安静的一套） ───────────────── */
[data-scene='clouds'] .glass-blob--1 {
  background: radial-gradient(closest-side, color-mix(in srgb, var(--text) 46%, transparent), transparent);
}
[data-scene='clouds'] .glass-blob--2 {
  background: radial-gradient(closest-side, color-mix(in srgb, var(--accent) 72%, transparent), transparent);
}
[data-scene='clouds'] .glass-blob--3 {
  background: radial-gradient(closest-side, color-mix(in srgb, var(--queued) 52%, transparent), transparent);
}
[data-scene='clouds'] .glass-blob--4 {
  background: radial-gradient(closest-side, color-mix(in srgb, var(--accent-secondary) 48%, transparent), transparent);
}
[data-scene='clouds'] .glass-blob--5 {
  background: radial-gradient(closest-side, color-mix(in srgb, var(--text) 28%, transparent), transparent);
}

/* ── 场景二：海面（深海蓝 + 薄荷水面 + 一条低平的光带） ───────────── */
[data-scene='sea'] .glass-blob--1 {
  background: radial-gradient(closest-side, color-mix(in srgb, var(--accent-deep) 58%, transparent), transparent);
}
[data-scene='sea'] .glass-blob--2 {
  background: radial-gradient(closest-side, color-mix(in srgb, var(--accent-secondary) 42%, transparent), transparent);
}
[data-scene='sea'] .glass-blob--3 {
  background: radial-gradient(closest-side, color-mix(in srgb, var(--accent) 40%, transparent), transparent);
}
[data-scene='sea'] .glass-blob--4 {
  background: radial-gradient(closest-side, color-mix(in srgb, var(--info) 30%, transparent), transparent);
}
[data-scene='sea'] .glass-blob--5 {
  background: radial-gradient(closest-side, color-mix(in srgb, var(--accent-secondary) 34%, transparent), transparent);
}
[data-scene='sea'] .glass-backdrop__glow {
  background: linear-gradient(180deg,
    transparent 46%,
    color-mix(in srgb, var(--accent-secondary) 18%, transparent) 58%,
    transparent 72%);
  opacity: 1;
}

/* ── 场景三：沙丘（暖沙 + 冷影，两色对照，仍不喧哗） ─────────────── */
[data-scene='dunes'] .glass-blob--1 {
  background: radial-gradient(closest-side, color-mix(in srgb, var(--warn) 46%, transparent), transparent);
}
[data-scene='dunes'] .glass-blob--2 {
  background: radial-gradient(closest-side, color-mix(in srgb, var(--accent) 36%, transparent), transparent);
}
[data-scene='dunes'] .glass-blob--3 {
  background: radial-gradient(closest-side, color-mix(in srgb, var(--warn) 36%, transparent), transparent);
}
[data-scene='dunes'] .glass-blob--4 {
  background: radial-gradient(closest-side, color-mix(in srgb, var(--accent-deep) 40%, transparent), transparent);
}
[data-scene='dunes'] .glass-blob--5 {
  background: radial-gradient(closest-side, color-mix(in srgb, var(--text) 18%, transparent), transparent);
}
[data-scene='dunes'] .glass-backdrop__glow {
  background: linear-gradient(180deg,
    transparent 52%,
    color-mix(in srgb, var(--warn) 16%, transparent) 62%,
    transparent 76%);
  opacity: 1;
}

/* 可选的"地平线柔光"：只有海面/沙丘启用（云朵要更空） */
.glass-backdrop__glow {
  position: absolute;
  inset: 0;
  opacity: 0;
  pointer-events: none;
}

/* 暗角：把四周压暗，保证贴边文字（侧栏/顶栏）始终清晰 */
.glass-backdrop__vignette {
  position: absolute;
  inset: 0;
  pointer-events: none;
  background:
    radial-gradient(124% 92% at 50% 6%,
      transparent 52%,
      color-mix(in srgb, var(--bg) 34%, transparent) 100%),
    linear-gradient(90deg,
      color-mix(in srgb, var(--bg) 26%, transparent) 0%,
      transparent 16%);
}

@keyframes glass-drift-a {
  0% { transform: translate3d(0, 0, 0) scale(1); }
  50% { transform: translate3d(3.2%, -2.4%, 0) scale(1.07); }
  100% { transform: translate3d(0, 0, 0) scale(1); }
}

@keyframes glass-drift-b {
  0% { transform: translate3d(0, 0, 0) scale(1.04); }
  50% { transform: translate3d(-4%, 3%, 0) scale(0.98); }
  100% { transform: translate3d(0, 0, 0) scale(1.04); }
}

@keyframes glass-drift-c {
  0% { transform: translate3d(1%, 0, 0) scale(1); }
  50% { transform: translate3d(-2.6%, -3.4%, 0) scale(1.09); }
  100% { transform: translate3d(1%, 0, 0) scale(1); }
}

@media (prefers-reduced-motion: reduce) {
  .glass-blob {
    animation: none;
  }
}
</style>
