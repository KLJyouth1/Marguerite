<script setup lang="ts">
/**
 * Generic view panel — renders any view registered in the view map below so a
 * route-owned view can be docked into the workbench without the workbench layer
 * needing to know about routes.
 *
 * Docking is additive: the panel hosts its own instance of the view, so the
 * main area stays free to navigate and the route-exclusion model is untouched.
 */
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'
import type { Component } from 'vue'
import PluginsView from '@/views/PluginsView.vue'

const props = defineProps<{ view: string; title?: string }>()

/**
 * Registry of dockable views, keyed by the `view` name carried in the workbench
 * item payload. Append-only: add an entry here to make a view dockable.
 */
const VIEW_MAP: Readonly<Record<string, Component>> = {
  plugins: PluginsView,
}

const { t } = useI18n()
const resolved = computed<Component | null>(() => VIEW_MAP[props.view] || null)
</script>

<template>
  <div class="view-panel">
    <component :is="resolved" v-if="resolved" />
    <div v-else class="view-panel__empty">{{ t('workbench.empty') }}</div>
  </div>
</template>

<style scoped>
.view-panel {
  display: flex;
  min-height: 0;
  height: 100%;
  flex: 1;
  flex-direction: column;
  overflow: auto;
}

.view-panel__empty {
  display: grid;
  min-height: 100%;
  place-items: center;
  padding: var(--sp-5);
  color: var(--text-dim);
  font-size: var(--fs-sm);
  text-align: center;
}
</style>