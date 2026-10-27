<template>
  <!--
    玛格丽特 · 星夜的三维山脊背景。
    只在 `marguerite-night` 主题下挂载；WebGL 不可用时由 App.vue 退回 Canvas 星空。
  -->
  <div ref="host" class="marguerite-ridge" aria-hidden="true" />
</template>

<script setup lang="ts">
import { onBeforeUnmount, onMounted, ref } from 'vue'
import { RidgeScene } from './ridgeScene'

const host = ref<HTMLElement | null>(null)
let scene: RidgeScene | null = null

onMounted(() => {
  if (!host.value) return
  scene = new RidgeScene(host.value)
  scene.start()
})

onBeforeUnmount(() => {
  scene?.dispose()
  scene = null
})
</script>

<style scoped>
/* 与 MargueriteSky 同一套层级约定：负 z-index 落在 #app 的内容之下、body 背景之上 */
.marguerite-ridge {
  position: fixed;
  inset: 0;
  z-index: -1;
  pointer-events: none;
  contain: strict;
}

.marguerite-ridge :deep(canvas) {
  display: block;
  width: 100%;
  height: 100%;
}
</style>
