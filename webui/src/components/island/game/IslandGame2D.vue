<script setup lang="ts">
/**
 * 2D SVG 降级岛 —— WebGL 不可用/性能不足时的诚实回退（蓝图 §6.2：绝不白屏）。
 * 纯 SVG 低多边形岛，标注三个关键区域；所有用户可见文案走 i18n。
 */
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'
import type { WorldSpec } from './worldSpec'

const props = defineProps<{ world: WorldSpec; dayPhase: 'day' | 'night' }>()
const { t } = useI18n()

const sky = computed(() => (props.dayPhase === 'night' ? 'var(--bg-elevated)' : 'var(--bg-surface)'))
</script>

<template>
  <div class="island2d" role="img" :aria-label="t('islandGame.fallback.aria')">
    <svg viewBox="0 0 400 260" class="island2d__svg" aria-hidden="true">
      <rect x="0" y="0" width="400" height="180" :fill="sky" />
      <!-- 海面 -->
      <ellipse cx="200" cy="220" rx="200" ry="40" fill="var(--info)" opacity="0.6" />
      <!-- 岛体 -->
      <ellipse cx="200" cy="200" rx="120" ry="26" fill="var(--ok)" opacity="0.85" />
      <!-- 秘境花园 -->
      <g transform="translate(140,188)">
        <rect x="-2" y="-14" width="4" height="14" fill="var(--warn)" />
        <polygon points="0,-30 8,-14 -8,-14" fill="var(--ok)" />
        <text x="0" y="-34" text-anchor="middle" class="island2d__label">{{ t('islandGame.region.garden') }}</text>
      </g>
      <!-- 雏菊圣殿 -->
      <g transform="translate(200,184)">
        <circle cx="0" cy="-12" r="9" fill="var(--bg-surface)" />
        <polygon points="0,-24 6,-12 -6,-12" fill="var(--accent)" opacity="0.7" />
        <text x="0" y="-30" text-anchor="middle" class="island2d__label">{{ t('islandGame.region.atrium') }}</text>
      </g>
      <!-- 委托板 -->
      <g transform="translate(258,190)">
        <rect x="-6" y="-10" width="12" height="9" fill="var(--warn)" />
        <text x="0" y="-16" text-anchor="middle" class="island2d__label">{{ t('islandGame.region.board') }}</text>
      </g>
    </svg>
    <p class="island2d__note">{{ t('islandGame.fallback.note') }}</p>
  </div>
</template>

<style scoped>
.island2d {
  display: flex;
  flex-direction: column;
  gap: 0.5rem;
  width: 100%;
  height: 100%;
  align-items: center;
  justify-content: center;
}
.island2d__svg {
  width: 100%;
  height: auto;
  max-height: 70%;
}
.island2d__label {
  font-size: 9px;
  fill: var(--text);
}
.island2d__note {
  margin: 0;
  color: var(--text-muted);
  font-size: 0.8rem;
}
</style>
