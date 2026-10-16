<script setup lang="ts">
/**
 * StatusLiveRegion — a thin aria-live announcer.
 *
 * Renders an off-screen `<span>` that screen readers read aloud whenever
 * `message` changes. Politeness comes from `useA11y.announceLevel`: callers
 * pass `politeness="assertive"` for approvals/errors and the default
 * `"polite"` for ordinary completions.
 *
 * Self-contained: no store, no router, no i18n, no child components.
 */
import { computed } from 'vue'

import type { AriaLiveMode } from '@/composables/useA11y'

const props = withDefaults(
  defineProps<{
    /** The text to announce. Changing this string triggers the live region. */
    message: string
    /** aria-live politeness. Defaults to polite; assertive interrupts. */
    politeness?: AriaLiveMode
  }>(),
  {
    message: '',
    politeness: 'polite',
  },
)

// role="status" implies polite semantics; assertive announcements use
// role="alert" so assistive tech treats them as urgent.
const role = computed<'status' | 'alert'>(() =>
  props.politeness === 'assertive' ? 'alert' : 'status',
)
</script>

<template>
  <span
    class="a11y-live-region"
    :role="role"
    :aria-live="politeness"
    aria-atomic="true"
  >
    {{ message }}
  </span>
</template>

<style scoped>
/* Visually hidden but available to assistive technology. */
.a11y-live-region {
  position: absolute;
  width: 1px;
  height: 1px;
  margin: -1px;
  padding: 0;
  overflow: hidden;
  clip: rect(0 0 0 0);
  clip-path: inset(50%);
  white-space: nowrap;
  border: 0;
}
</style>
