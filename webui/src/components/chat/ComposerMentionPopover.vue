<!-- 「@」专家 / 智能体引用候选（对话框内）。
     数据源复用桌面网关原生 RPC：experts.list（预设专家库）+ agents.list（预设智能体）。
     纯只读候选：选中后由 ChatComposer 把 `@名称` 插入输入框，不改动 chat.send 负载。 -->
<script setup lang="ts">
import { computed, onMounted, ref, watch } from 'vue'
import Icon from '@/components/Icon.vue'
import { useRpcStore } from '@/stores/rpc'

const props = defineProps<{ query: string }>()
const emit = defineEmits<{ select: [label: string] }>()

const rpc = useRpcStore()

interface Expert {
  id: string
  label: string
  lineage?: string
  expertise?: string
}
interface AgentItem {
  id: string
  name?: string
}
interface Candidate {
  kind: 'expert' | 'agent'
  id: string
  label: string
  sub?: string
}

const experts = ref<Expert[]>([])
const agents = ref<AgentItem[]>([])
const active = ref(0)

const candidates = computed<Candidate[]>(() => {
  const q = props.query.trim().toLowerCase()
  const ex: Candidate[] = experts.value.map((e) => ({
    kind: 'expert',
    id: e.id,
    label: e.label,
    sub: e.expertise || e.lineage,
  }))
  const ag: Candidate[] = agents.value.map((a) => ({
    kind: 'agent',
    id: a.id,
    label: a.name || a.id,
  }))
  const all = [...ex, ...ag]
  if (!q) return all
  return all.filter((c) => `${c.label} ${c.id}`.toLowerCase().includes(q))
})

async function load() {
  try {
    if (!rpc.isConnected) await rpc.waitForConnection(8_000)
    const [e, a] = await Promise.all([
      rpc.call<{ experts: Expert[] }>('experts.list', {}),
      rpc.call<{ agents: AgentItem[] }>('agents.list', {}),
    ])
    experts.value = e.experts ?? []
    agents.value = a.agents ?? []
  } catch {
    /* 无专家库时静默降级：候选为空，不阻塞输入 */
  }
}

onMounted(load)

// 候选随 query 收窄时把高亮归零，避免高亮停留在越界索引上。
watch(candidates, () => {
  active.value = 0
})

function move(delta: number) {
  const n = candidates.value.length
  if (!n) return
  active.value = (active.value + delta + n) % n
}
function selectActive() {
  const c = candidates.value[active.value]
  if (c) emit('select', c.label)
}
function pick(c: Candidate) {
  emit('select', c.label)
}

defineExpose({
  move,
  selectActive,
  hasCandidates: () => candidates.value.length > 0,
})
</script>

<template>
  <div class="mention-popover" role="listbox" aria-label="引用专家 / 智能体">
    <div class="mention-popover__head">引用专家 / 智能体</div>
    <ul v-if="candidates.length" class="mention-popover__list">
      <li v-for="(c, i) in candidates" :key="`${c.kind}:${c.id}`">
        <button
          type="button"
          role="option"
          :aria-selected="i === active"
          class="mention-popover__item"
          :class="{ 'is-active': i === active }"
          @mousedown.prevent
          @mouseenter="active = i"
          @click="pick(c)"
        >
          <Icon :name="c.kind === 'expert' ? 'sparkles' : 'agents'" :size="14" />
          <span class="mention-popover__label">{{ c.label }}</span>
          <span v-if="c.sub" class="mention-popover__sub">{{ c.sub }}</span>
        </button>
      </li>
    </ul>
    <p v-else class="mention-popover__empty">无匹配的专家或智能体</p>
  </div>
</template>

<style scoped>
.mention-popover {
  position: absolute;
  left: 0;
  bottom: calc(100% + 6px);
  width: min(340px, calc(100vw - 32px));
  max-height: 300px;
  overflow: hidden;
  display: flex;
  flex-direction: column;
  border: 1px solid var(--border-strong);
  border-radius: var(--radius-md);
  background: var(--bg-elevated, var(--bg-surface));
  box-shadow: var(--shadow-xl);
  z-index: 40;
}
.mention-popover__head {
  padding: 7px 10px;
  border-bottom: 1px solid var(--border);
  color: var(--text-dim);
  font-size: var(--fs-xs);
  font-weight: 700;
  letter-spacing: 0.02em;
}
.mention-popover__list {
  margin: 0;
  padding: 4px;
  list-style: none;
  overflow-y: auto;
}
.mention-popover__item {
  display: flex;
  align-items: center;
  gap: 8px;
  width: 100%;
  padding: 7px 9px;
  border: 0;
  border-radius: var(--radius-sm);
  background: transparent;
  color: var(--text-muted);
  font: inherit;
  font-size: var(--fs-sm);
  text-align: left;
  cursor: pointer;
}
.mention-popover__item.is-active,
.mention-popover__item:hover {
  background: var(--bg-hover);
  color: var(--text);
}
.mention-popover__label {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.mention-popover__sub {
  margin-left: auto;
  max-width: 45%;
  overflow: hidden;
  color: var(--text-dim);
  font-size: var(--fs-xs);
  text-overflow: ellipsis;
  white-space: nowrap;
}
.mention-popover__empty {
  margin: 0;
  padding: 12px;
  color: var(--text-dim);
  font-size: var(--fs-xs);
  text-align: center;
}
</style>
