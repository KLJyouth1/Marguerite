<script setup lang="ts">
/**
 * RunNodeCard — distinctive live tile for agents, subagents, tools, memory.
 * Interaction container (not decorative chrome): status pulse + primary action.
 */
import type { RunNodeKind, RunNodeStatus } from './runNodeTypes'

const props = withDefaults(defineProps<{
  title: string
  kind: RunNodeKind
  status?: RunNodeStatus
  subtitle?: string
  meta?: string
  detail?: string
  index?: number
}>(), {
  status: 'idle',
  index: 0,
})

defineEmits<{ activate: [] }>()

const kindLabel: Record<RunNodeKind, string> = {
  agent: 'Agent',
  subagent: 'Subagent',
  tool: 'Tool',
  memory: 'Memory',
  retrieval: 'Retrieval',
  knowledge: 'Knowledge',
  fleet: 'Fleet',
}
</script>

<template>
  <button
    type="button"
    class="rnc"
    :class="[`is-${props.status}`, `kind-${props.kind}`]"
    :style="{ '--i': String(props.index) }"
    @click="$emit('activate')"
  >
    <header class="rnc__head">
      <span class="rnc__pulse" aria-hidden="true" />
      <span class="rnc__kind">{{ kindLabel[kind] }}</span>
      <span class="rnc__status">{{ status }}</span>
    </header>
    <h3 class="rnc__title">{{ title }}</h3>
    <p v-if="subtitle" class="rnc__sub">{{ subtitle }}</p>
    <p v-if="detail" class="rnc__detail">{{ detail }}</p>
    <footer v-if="meta" class="rnc__meta">{{ meta }}</footer>
  </button>
</template>

<style scoped>
.rnc {
  --rnc-tone: var(--accent);
  position: relative;
  display: flex;
  flex-direction: column;
  align-items: flex-start;
  gap: 6px;
  width: 100%;
  min-height: 148px;
  padding: var(--sp-4);
  border: 1px solid color-mix(in srgb, var(--rnc-tone) 22%, var(--border));
  border-radius: var(--radius-card);
  background:
    linear-gradient(
      155deg,
      color-mix(in srgb, var(--rnc-tone) 10%, transparent),
      transparent 42%
    ),
    var(--bg-surface);
  color: var(--text);
  text-align: left;
  cursor: pointer;
  overflow: hidden;
  animation: rnc-rise var(--dur-enter) var(--ease-out) both;
  animation-delay: calc(var(--i, 0) * 40ms);
  transition:
    border-color var(--dur-fast) var(--ease-standard),
    transform var(--dur-fast) var(--ease-spring),
    box-shadow var(--dur-fast) var(--ease-standard);
}

.rnc:hover {
  border-color: color-mix(in srgb, var(--rnc-tone) 55%, var(--border));
  transform: translateY(-2px);
  box-shadow: var(--elev-2, var(--shadow-md));
}

.rnc:focus-visible {
  outline: none;
  box-shadow: var(--focus-ring);
}

.rnc.kind-subagent { --rnc-tone: var(--ok, var(--accent)); }
.rnc.kind-tool { --rnc-tone: var(--info, var(--accent)); }
.rnc.kind-memory,
.rnc.kind-retrieval,
.rnc.kind-knowledge { --rnc-tone: var(--warn, var(--accent)); }
.rnc.kind-fleet { --rnc-tone: var(--accent); }

.rnc.is-running { --rnc-tone: var(--accent); }
.rnc.is-ok { --rnc-tone: var(--ok, var(--accent)); }
.rnc.is-warn { --rnc-tone: var(--warn, var(--accent)); }
.rnc.is-error { --rnc-tone: var(--danger, var(--accent)); }

.rnc__head {
  display: flex;
  align-items: center;
  gap: 8px;
  width: 100%;
}

.rnc__pulse {
  width: 8px;
  height: 8px;
  border-radius: var(--radius-full);
  background: var(--rnc-tone);
  box-shadow: 0 0 0 3px color-mix(in srgb, var(--rnc-tone) 18%, transparent);
}

.rnc.is-running .rnc__pulse {
  animation: rnc-pulse var(--dur-pulse) linear infinite;
}

.rnc__kind {
  font-family: var(--font-mono);
  font-size: 10px;
  letter-spacing: 0.14em;
  text-transform: uppercase;
  color: var(--text-dim);
}

.rnc__status {
  margin-left: auto;
  font-family: var(--font-mono);
  font-size: 10px;
  letter-spacing: 0.08em;
  text-transform: uppercase;
  color: var(--rnc-tone);
}

.rnc__title {
  margin: var(--sp-1) 0 0;
  font-family: var(--font-display);
  font-size: var(--fs-lg);
  font-weight: 700;
  letter-spacing: -0.01em;
  line-height: 1.15;
}

.rnc__sub,
.rnc__detail {
  margin: 0;
  font-size: var(--fs-xs);
  color: var(--text-muted);
  line-height: 1.45;
}

.rnc__meta {
  margin-top: auto;
  padding-top: var(--sp-2);
  font-family: var(--font-mono);
  font-size: 10px;
  color: var(--text-dim);
}

@keyframes rnc-rise {
  from { opacity: 0; transform: translateY(10px); }
  to { opacity: 1; transform: translateY(0); }
}

@keyframes rnc-pulse {
  0%, 100% { opacity: 0.65; transform: scale(1); }
  50% { opacity: 1; transform: scale(1.2); }
}

@media (prefers-reduced-motion: reduce) {
  .rnc,
  .rnc.is-running .rnc__pulse {
    animation: none;
  }
}
</style>
