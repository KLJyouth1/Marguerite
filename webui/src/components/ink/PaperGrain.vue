<template>
  <!-- 宣纸 noise field (Track F, optional). A static feTurbulence +
       feDisplacementMap grain blended over the ink theme's paper ground. It is
       pre-rendered (no animation) so it never competes with the main thread —
       the empirical finding was that several animated filter layers drop frames,
       so the paper texture stays a single, static SVG. -->
  <svg
    class="ink-paper-grain"
    aria-hidden="true"
    focusable="false"
    xmlns="http://www.w3.org/2000/svg"
    preserveAspectRatio="none"
  >
    <filter id="ink-paper-noise" x="0" y="0" width="100%" height="100%">
      <feTurbulence
        type="fractalNoise"
        baseFrequency="0.82"
        numOctaves="2"
        seed="7"
        stitchTiles="stitch"
        result="noise"
      />
      <feDisplacementMap
        in="noise"
        in2="noise"
        scale="7"
        xChannelSelector="R"
        yChannelSelector="G"
        result="warp"
      />
      <!-- Collapse the noise into a low-alpha black so it reads as fibre, not
           colour; opacity + multiply are applied from CSS below. -->
      <feColorMatrix
        in="warp"
        type="matrix"
        values="0 0 0 0 0  0 0 0 0 0  0 0 0 0 0  0 0 0 0.55 0"
      />
    </filter>
    <rect width="100%" height="100%" filter="url(#ink-paper-noise)" />
  </svg>
</template>

<script setup lang="ts">
// Presentational only. Mounted by App.vue under the "ink" value theme.
</script>

<style scoped>
.ink-paper-grain {
  position: fixed;
  inset: 0;
  width: 100%;
  height: 100%;
  z-index: 0;
  pointer-events: none;
  opacity: 0.05;
  mix-blend-mode: multiply;
}
</style>
