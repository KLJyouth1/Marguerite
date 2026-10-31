<template>
  <section class="attn-rail" :aria-label="t('attention.label')">
    <!-- Collapsed pill — the only permanently visible part, so it has to answer
         one question at a glance: "is anything waiting on me, and how many?"
         The kind dots answer "what sort" without opening anything. -->
    <button
      v-if="!expanded"
      type="button"
      class="attn-pill"
      :aria-label="t('attention.expand')"
      aria-expanded="false"
      aria-controls="attn-panel"
      v-bind="pillMotion"
      @click="emit('toggle')"
    >
      <svg
        viewBox="0 0 24 24"
        width="15"
        height="15"
        fill="none"
        stroke="currentColor"
        stroke-width="1.8"
        stroke-linecap="round"
        aria-hidden="true"
        focusable="false"
      >
        <path d="M18 8a6 6 0 1 0-12 0c0 7-3 8-3 8h18s-3-1-3-8" />
        <path d="M13.7 20a2 2 0 0 1-3.4 0" />
      </svg>
      <span :key="motionKey" class="attn-pill__count">{{ total }}</span>
      <span class="attn-pill__kinds" aria-hidden="true">
        <span
          v-for="kind in activeKinds"
          :key="kind"
          class="attn-dot"
          :class="`attn-dot--${kind}`"
        />
      </span>
    </button>

    <!-- Expanded panel. Non-modal on purpose: a corner surface that trapped
         focus would steal the caret the user was about to type with. -->
    <div v-else id="attn-panel" class="attn-panel" v-bind="panelMotion">
      <header class="attn-panel__head">
        <p class="attn-panel__title">{{ t('attention.label') }}</p>
        <button
          type="button"
          class="attn-act"
          :aria-label="t('attention.collapse')"
          :title="t('attention.collapse')"
          @click="emit('toggle')"
        >
          {{ t('attention.collapseShort') }}
        </button>
      </header>
      <ul v-if="items.length" class="attn-panel__list">
        <li v-for="item in items" :key="item.id" class="attn-item">
          <span class="attn-dot attn-item__dot" :class="`attn-dot--${item.kind}`" aria-hidden="true" />
          <span class="attn-item__body">
            <button type="button" class="attn-item__hit" @click="emit('action', 'open', item)">
              <span class="attn-item__title">{{ item.title }}</span>
              <span v-if="item.detail" class="attn-item__detail">{{ item.detail }}</span>
            </button>
            <span class="attn-item__meta">
              <span>{{ t(`attention.kind.${item.kind}`) }}</span>
              <span aria-hidden="true"> · </span>
              <span class="attn-item__age">{{ ageLabel(item.at) }}</span>
            </span>
          </span>
        </li>
      </ul>
      <p v-else class="attn-empty">{{ t('attention.empty') }}</p>
    </div>
  </section>
</template>

<script setup lang="ts">
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'

import {
  type AttentionItem,
  type AttentionKind,
  type RankedAttention,
} from '@/composables/ui/attentionQueue'
import { motionAttrs, type MotionCue } from '@/composables/ui/motionGrammar'

const props = defineProps<{
  /** Already ranked and sliced by the caller. */
  items: RankedAttention[]
  /** Per-kind counts across *all* items, not just the visible slice. */
  counts: Record<AttentionKind, number>
  /** Debt across all items; drives the pill badge. */
  total: number
  expanded: boolean
  /** Bumped by the caller whenever a cue should replay. */
  motionKey: number
  /** Cue derived from the last count transition; null means "stay still". */
  cue: MotionCue | null
  reducedMotion: boolean
  /** Now in epoch ms, injected so the age label stays a pure function of props. */
  now: number
}>()

const emit = defineEmits<{
  (e: 'toggle'): void
  (e: 'action', action: 'open', item: AttentionItem): void
}>()

const { t } = useI18n()

const activeKinds = computed<AttentionKind[]>(() =>
  (Object.keys(props.counts) as AttentionKind[]).filter((kind) => props.counts[kind] > 0),
)

// The pill carries the arrival cue (its number changed); the panel carries the
// settle cue (it opened). Both read the same grammar module, so the two
// surfaces can never disagree about what "new" feels like.
const pillMotion = computed(() => motionAttrs(props.cue, props.reducedMotion))
const panelMotion = computed(() =>
  motionAttrs(props.cue === 'refuse' ? 'refuse' : 'settle', props.reducedMotion),
)

function ageLabel(at: number): string {
  const minutes = Math.max(0, Math.floor((props.now - at) / 60_000))
  if (minutes < 1) return t('attention.age.now')
  if (minutes < 60) return t('attention.age.minutes', { n: minutes })
  return t('attention.age.hours', { n: Math.floor(minutes / 60) })
}
</script>
