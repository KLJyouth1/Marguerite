<script setup lang="ts">
/**
 * 岛屿工作台面板 —— 复用 IslandView 的紧凑形态（同一份 runtime → world → 3D 管线）。
 *
 * 单一事实源：运行态归约、岛屿世界构建、WebGL 生命周期都在 IslandView/Island3D，
 * 面板只负责「换一个容器尺寸」。紧凑形态隐藏页头与图例（工作台已提供 tab 标题与侧栏），
 * 但保留视角复位/聚焦/昼夜控制 —— 那些是看运行状态必需的。
 */
import IslandView from '@/views/IslandView.vue'
import type { WorkbenchItem } from '@/workbench/types'

defineProps<{ item: WorkbenchItem }>()
</script>

<template>
  <div class="island-panel">
    <IslandView compact />
  </div>
</template>

<style scoped>
.island-panel {
  height: 100%;
  min-height: 0;
  overflow: hidden;
}
</style>