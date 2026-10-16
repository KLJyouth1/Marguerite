<script setup lang="ts">
/**
 * WP6 — device building card (thin, presentational).
 *
 * Renders one normalized `DeviceNode` as a simplified building card for the
 * island workspace. Props-only: it takes a node and renders it, it does NOT
 * read the store and does NOT mutate the three.js scene.
 *
 * Wiring point (honest boundary, not yet done):
 *   Island3D.ts builds Mesh objects from the entities produced by
 *   `buildIslandWorld()`. A real 3D "device building" would be added there as a
 *   new `IslandEntityKind` once the device-bus RPC is wired into
 *   `useIslandRuntime`. Until then this card is the 2D preview / fallback
 *   surface for that future node.
 */
import { computed } from 'vue'

import type { DeviceNode } from '../../composables/useDevicesPanel'

const props = defineProps<{ node: DeviceNode }>()

// Hazard band → semantic theme role (mirrors islandContract's IslandThemeRole).
// Colour is never the only signal: the text label is rendered alongside the badge.
const HAZARD_ROLE = { L0: 'ok', L1: 'info', L2: 'warn', L3: 'danger' } as const
type HazardRole = (typeof HAZARD_ROLE)[keyof typeof HAZARD_ROLE]

const hazardRole = computed<HazardRole>(() => HAZARD_ROLE[props.node.hazardLevel])
</script>

<template>
  <article
    class="device-node"
    :class="[
      `device-node--${props.node.hazardLevel.toLowerCase()}`,
      `device-node--${hazardRole}`,
      { 'device-node--offline': !props.node.online },
    ]"
    :data-icon="props.node.icon"
    data-testid="device-island-node"
  >
    <span class="device-node__badge" aria-hidden="true">{{ props.node.label }}</span>

    <div class="device-node__body">
      <span class="device-node__name" :title="props.node.name">{{ props.node.name }}</span>
      <span class="device-node__meta">
        <span class="device-node__dot" aria-hidden="true"></span>
        <span class="device-node__transport">{{ props.node.transport || '—' }}</span>
      </span>
    </div>

    <span class="device-node__state">{{ props.node.online ? 'online' : 'offline' }}</span>
  </article>
</template>

<style scoped>
.device-node {
  display: flex;
  align-items: center;
  gap: 0.625rem;
  padding: 0.5rem 0.75rem;
  background: var(--bg-elevated);
  border: 1px solid var(--border);
  border-radius: var(--radius-card);
  transition:
    border-color var(--dur-base) var(--ease-standard),
    opacity var(--dur-base) var(--ease-standard);
}

.device-node--offline {
  opacity: 0.6;
}

.device-node__badge {
  flex-shrink: 0;
  padding: 0.125rem 0.5rem;
  border: 1px solid transparent;
  border-radius: var(--radius-pill);
  font-size: 0.75rem;
  line-height: 1.2;
  white-space: nowrap;
}

.device-node--ok.device-node--ok {
  background: color-mix(in srgb, var(--ok) 16%, var(--bg-surface));
  color: var(--ok);
  border-color: color-mix(in srgb, var(--ok) 35%, var(--border));
}

.device-node--info.device-node--info {
  background: color-mix(in srgb, var(--info) 16%, var(--bg-surface));
  color: var(--info);
  border-color: color-mix(in srgb, var(--info) 35%, var(--border));
}

.device-node--warn.device-node--warn {
  background: color-mix(in srgb, var(--warn) 16%, var(--bg-surface));
  color: var(--warn);
  border-color: color-mix(in srgb, var(--warn) 35%, var(--border));
}

.device-node--danger.device-node--danger {
  background: color-mix(in srgb, var(--danger) 12%, var(--bg-surface));
  color: var(--danger);
  border-color: color-mix(in srgb, var(--danger) 30%, var(--border));
}

.device-node__body {
  display: flex;
  flex-direction: column;
  min-width: 0;
  flex: 1 1 auto;
}

.device-node__name {
  color: var(--text);
  font-weight: 600;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.device-node__meta {
  display: flex;
  align-items: center;
  gap: 0.375rem;
  color: var(--text-muted);
  font-size: 0.75rem;
  min-width: 0;
}

.device-node__transport {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.device-node__dot {
  flex-shrink: 0;
  width: 0.5rem;
  height: 0.5rem;
  border-radius: 50%;
  background: var(--ok);
}

.device-node--offline .device-node__dot {
  background: var(--text-dim);
}

.device-node__state {
  flex-shrink: 0;
  color: var(--text-dim);
  font-size: 0.75rem;
  white-space: nowrap;
}

/* Mobile / narrow viewport: keep the card from overflowing by wrapping the
   badge onto its own line and truncating the name instead of stretching. */
@media (max-width: 480px) {
  .device-node {
    flex-wrap: wrap;
    gap: 0.375rem;
    padding: 0.5rem 0.625rem;
  }

  .device-node__body {
    flex: 1 1 100%;
    order: 3;
  }

  .device-node__state {
    margin-left: auto;
  }
}
</style>
