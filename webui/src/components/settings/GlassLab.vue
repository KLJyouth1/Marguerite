<template>
  <!--
    玻璃参数面板：把液态玻璃主题里所有"尺寸/材质"参数集中成一组可拖动的数。
    改一个数字 → 立刻写进 <html> 的 --glass-* → 全应用即时预览 → 自动记住。
    底部把当前参数导出成 JSON（含 CSS 变量映射），可直接贴进 tokens.css 复现。
  -->
  <section class="glass-lab" data-testid="glass-lab">
    <header class="glass-lab__head">
      <div>
        <h3 class="glass-lab__title">{{ t('glassLab.title') }}</h3>
        <p class="glass-lab__desc">{{ t('glassLab.desc') }}</p>
      </div>
      <button class="btn btn--ghost glass-lab__reset" type="button" data-testid="glass-lab-reset" @click="onReset">
        {{ t('glassLab.reset') }}
      </button>
    </header>

    <div class="glass-lab__scenes">
      <span class="glass-lab__scenes-label">{{ t('glassLab.scene') }}</span>
      <button
        v-for="scene in GLASS_SCENES"
        :key="scene"
        class="glass-lab__chip"
        :class="{ 'is-active': glassScene === scene }"
        type="button"
        :data-testid="`glass-lab-scene-${scene}`"
        :aria-pressed="glassScene === scene"
        @click="setGlassScene(scene)"
      >
        {{ t(`glassLab.scene_${scene}`) }}
      </button>
    </div>

    <div class="glass-lab__grid">
      <label v-for="def in GLASS_PARAM_DEFS" :key="def.key" class="glass-lab__row">
        <span class="glass-lab__label">{{ t(`glassLab.${def.labelKey}`) }}</span>
        <input
          class="glass-lab__range"
          type="range"
          :min="def.min"
          :max="def.max"
          :step="def.step"
          :value="glassParams[def.key]"
          :data-testid="`glass-lab-range-${def.key}`"
          :aria-label="t(`glassLab.${def.labelKey}`)"
          @input="onInput(def.key, $event)"
        >
        <output class="glass-lab__value">{{ glassParams[def.key] }}{{ def.unit === 'px' ? 'px' : '%' }}</output>
      </label>
    </div>

    <details class="glass-lab__export">
      <summary>{{ t('glassLab.export') }}</summary>
      <pre class="glass-lab__json" data-testid="glass-lab-json">{{ exportedJson }}</pre>
    </details>
  </section>
</template>

<script setup lang="ts">
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'
import { GLASS_SCENES, glassScene, setGlassScene } from '@/composables/useGlassScene'
import {
  GLASS_PARAM_DEFS,
  exportGlassParams,
  glassParams,
  resetGlassParams,
  setGlassParam,
} from '@/composables/useGlassLab'

const { t } = useI18n()

const exportedJson = computed(() => {
  // 依赖 glassParams 的每个键，保证拖滑块时 JSON 同步更新
  void Object.values(glassParams).join(',')
  return exportGlassParams()
})

function onInput(key: string, event: Event): void {
  const value = Number((event.target as HTMLInputElement).value)
  setGlassParam(key, value)
}

function onReset(): void {
  resetGlassParams()
}
</script>

<style scoped>
.glass-lab {
  background: var(--bg-elevated);
  border: 1px solid var(--border);
  border-radius: var(--radius-card);
  padding: 16px 18px;
  display: flex;
  flex-direction: column;
  gap: 14px;
}

.glass-lab__head {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 12px;
}

.glass-lab__title {
  margin: 0;
  font-size: 0.95rem;
  font-weight: 600;
  color: var(--text);
}

.glass-lab__desc {
  margin: 4px 0 0;
  font-size: 0.8rem;
  line-height: 1.6;
  color: var(--text-muted);
  max-width: 62ch;
}

.glass-lab__reset {
  flex: 0 0 auto;
  font-size: 0.8rem;
}

.glass-lab__scenes {
  display: flex;
  align-items: center;
  gap: 8px;
  flex-wrap: wrap;
}

.glass-lab__scenes-label {
  font-size: 0.8rem;
  color: var(--text-muted);
  margin-right: 2px;
}

.glass-lab__chip {
  border: 1px solid var(--border);
  border-radius: var(--radius-pill);
  background: transparent;
  color: var(--text-muted);
  font-size: 0.78rem;
  padding: 4px 12px;
  cursor: pointer;
  transition: color var(--dur-fast) var(--ease-standard), border-color var(--dur-fast) var(--ease-standard);
}

.glass-lab__chip:hover {
  color: var(--text);
  border-color: var(--border-strong);
}

.glass-lab__chip.is-active {
  color: var(--accent-foreground);
  background: var(--accent);
  border-color: var(--accent);
}

.glass-lab__grid {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(260px, 1fr));
  gap: 10px 20px;
}

.glass-lab__row {
  display: grid;
  grid-template-columns: minmax(84px, auto) 1fr 56px;
  align-items: center;
  gap: 10px;
}

.glass-lab__label {
  font-size: 0.8rem;
  color: var(--text-muted);
  white-space: nowrap;
}

.glass-lab__range {
  width: 100%;
  accent-color: var(--accent);
}

.glass-lab__value {
  font-size: 0.78rem;
  color: var(--text);
  font-variant-numeric: tabular-nums;
  text-align: right;
}

.glass-lab__export {
  border-top: 1px solid var(--hairline);
  padding-top: 10px;
}

.glass-lab__export summary {
  cursor: pointer;
  font-size: 0.8rem;
  color: var(--text-muted);
}

.glass-lab__json {
  margin: 10px 0 0;
  max-height: 240px;
  overflow: auto;
  background: var(--bg);
  border: 1px solid var(--border);
  border-radius: var(--radius-control);
  padding: 12px;
  font-size: 0.74rem;
  line-height: 1.55;
  color: var(--text-muted);
  user-select: text;
}
</style>
