<template>
  <!-- Nothing needs a human → render nothing. A peripheral surface that is
       always on screen becomes wallpaper; the whole point is that its presence
       is the signal. -->
  <AttentionShelf
    v-if="visible"
    :items="ranked"
    :counts="counts"
    :total="total"
    :expanded="expanded"
    :motion-key="motionKey"
    :cue="cue"
    :reduced-motion="reducedMotion"
    :now="now"
    @toggle="toggle"
    @action="onAction"
  />
</template>

<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { useRouter } from 'vue-router'

import AttentionShelf from '@/components/ui/AttentionShelf.vue'
import {
  attentionCounts,
  attentionDebt,
  dedupeAttention,
  rankAttention,
  type AttentionItem,
} from '@/composables/ui/attentionQueue'
import { approvalsToAttention, createFirstSeenTracker } from '@/composables/ui/attentionSources'
import {
  cueFromCounts,
  prefersReducedMotion,
  type MotionCue,
} from '@/composables/ui/motionGrammar'
import { useAppStore } from '@/stores/app'

/** How many rows the corner will show before the user should work the queue. */
const VISIBLE_LIMIT = 5
/** Age labels only need minute resolution; 30s keeps them honest without churn. */
const CLOCK_MS = 30_000

const appStore = useAppStore()
const router = useRouter()

const firstSeen = createFirstSeenTracker()
const now = ref(Date.now())
const expanded = ref(false)
const reducedMotion = ref(prefersReducedMotion())
const cue = ref<MotionCue | null>(null)
const motionKey = ref(0)

const items = computed<AttentionItem[]>(() =>
  approvalsToAttention(appStore.pendingApprovals, firstSeen, now.value),
)
const ranked = computed(() => rankAttention(dedupeAttention(items.value), now.value, VISIBLE_LIMIT))
const counts = computed(() => attentionCounts(items.value))
const total = computed(() => attentionDebt(items.value))
const visible = computed(() => total.value > 0)

let clock: ReturnType<typeof setInterval> | undefined
let media: MediaQueryList | undefined

function startClock(): void {
  if (clock !== undefined) return
  clock = setInterval(() => {
    now.value = Date.now()
  }, CLOCK_MS)
}

function stopClock(): void {
  if (clock === undefined) return
  clearInterval(clock)
  clock = undefined
}

function onMotionPreferenceChange(event: MediaQueryListEvent): void {
  reducedMotion.value = event.matches
}

// The clock is only alive while there is something to age. An idle workbench
// must not hold an interval — this is a desktop app that may sit open all day.
watch(
  total,
  (next) => {
    if (next > 0) startClock()
    else stopClock()
  },
  { immediate: true },
)

// Motion fires on the *transition*, never on the value: `cueFromCounts` returns
// null when nothing changed, and only then do we leave `cue` alone.
watch(total, (next, prev) => {
  const nextCue = cueFromCounts(prev, next)
  if (!nextCue) return
  cue.value = nextCue
  motionKey.value += 1
})

// An empty shelf cannot stay expanded — otherwise resolving the last approval
// leaves an invisible panel holding the expanded flag for the next arrival.
watch(total, (next) => {
  if (next === 0) expanded.value = false
})

function toggle(): void {
  expanded.value = !expanded.value
}

function onAction(action: 'open', item: AttentionItem): void {
  // The shelf routes attention; it never resolves a decision. Approving a tool
  // execution from a 340px corner — without the command text, the permission
  // matrix or the diff — is a safety regression, so `approve`/`reject` are not
  // offered here at all. The signature stays narrow on purpose: widening it is
  // the moment someone ships a one-click approval.
  if (action !== 'open') return

  if (item.target?.kind === 'approval') {
    appStore.requestApprovalFocus({
      approvalId: item.target.approvalId,
      sessionKey: item.target.sessionKey,
    })
    // Same URL shape the sidebar uses (App.vue `switchToSession`: path `/chat`
    // plus `?session=`). Re-stated here rather than imported because importing a
    // function out of a 96 KB single-file component would couple the shelf to the
    // shell; if that shape ever changes, `check-attention-shelf` catches the drift.
    void router.push({ path: '/chat', query: { session: item.target.sessionKey } })
    expanded.value = false
    return
  }

  if (item.target?.kind === 'route') {
    void router.push(item.target.path)
    expanded.value = false
    return
  }

  // Unroutable item: land on Sessions, whose attention strip carries the same
  // pending count (the retired `/approvals` page already redirects there).
  void router.push('/sessions')
}

function onKeydown(event: KeyboardEvent): void {
  // Alt+. mirrors the OS convention for "show me what needs doing".
  if (!event.altKey || event.key !== '.') return
  if (total.value === 0) return
  event.preventDefault()
  toggle()
}

onMounted(() => {
  window.addEventListener('keydown', onKeydown)
  if (typeof window.matchMedia === 'function') {
    media = window.matchMedia('(prefers-reduced-motion: reduce)')
    media.addEventListener?.('change', onMotionPreferenceChange)
  }
})

onBeforeUnmount(() => {
  window.removeEventListener('keydown', onKeydown)
  media?.removeEventListener?.('change', onMotionPreferenceChange)
  stopClock()
})
</script>
