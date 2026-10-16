<script setup lang="ts">
import { computed } from 'vue'
import { slotContributions, uiSlotVersion } from './registry'
import type { UiSlotId } from './types'

const props = defineProps<{ slot: UiSlotId }>()

// `uiSlotVersion()` is read purely so this computed re-evaluates when the
// registry changes; the ordered list itself comes from `slotContributions`.
const contributions = computed(() => {
  uiSlotVersion().value
  return slotContributions(props.slot)
})
</script>

<template>
  <!-- Renders every registered contribution for this slot, in stable order.
       Contributes nothing (not even a wrapper element) when the slot is empty,
       so adding this seam cannot shift the existing layout. -->
  <component
    :is="contribution.component"
    v-for="contribution in contributions"
    :key="contribution.id"
    :data-ui-slot-contribution="contribution.id"
    :data-ui-slot-label-key="contribution.labelKey"
  />
</template>
