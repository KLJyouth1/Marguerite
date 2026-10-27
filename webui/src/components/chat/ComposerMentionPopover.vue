<!-- 「@」引用候选（对话框内）。
     三类来源：
     1. 上下文引用 —— 网关 `context.references`（文件 / 目录 / 规则 / 历史会话）；
        选中后进入本轮 `chat.send.contextRefs`，由后端渲染成材料拼在用户消息尾部。
     2. 专家库 —— `experts.list`；3. 智能体 —— `agents.list`。
        后两者是「角色指向」，选中只插入 `@名称` 文本（不改变负载），
        整队启动仍走专家中心页的 team.run。 -->
<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import Icon from '@/components/Icon.vue'
import { useRpcStore } from '@/stores/rpc'
import type { ContextRefItem } from '@/composables/chat/contextRefs'

const props = defineProps<{ query: string; workspaceId?: string }>()
const emit = defineEmits<{ select: [payload: { label: string; ref?: ContextRefItem }] }>()

const rpc = useRpcStore()

interface Expert {
  id: string
  label: string
  lineage?: string
  /** 网关返回数组（`gateway/team.rs::experts_list`）；此前声明成 string 属于契约谎言。 */
  expertise?: string[]
}
interface AgentItem {
  id: string
  name?: string
}
interface ContextGroup {
  id: string
  label: string
  items: Array<{ kind: string; id: string; label: string; subtitle?: string; path?: string }>
}
interface Candidate {
  kind: 'expert' | 'agent' | 'ref'
  id: string
  label: string
  sub?: string
  ref?: ContextRefItem
}

const experts = ref<Expert[]>([])
const agents = ref<AgentItem[]>([])
const refCandidates = ref<Candidate[]>([])
const active = ref(0)

const candidates = computed<Candidate[]>(() => {
  const q = props.query.trim().toLowerCase()
  const ex: Candidate[] = experts.value.map((e) => ({
    kind: 'expert',
    id: e.id,
    label: e.label,
    sub: Array.isArray(e.expertise) ? e.expertise.join('、') : (e.expertise || e.lineage),
  }))
  const ag: Candidate[] = agents.value.map((a) => ({
    kind: 'agent',
    id: a.id,
    label: a.name || a.id,
  }))
  const all = [...refCandidates.value, ...ex, ...ag]
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

/**
 * 拉取上下文引用候选。后端对文件/目录做项目根边界校验，这里只负责展示。
 * 失败时清空该组而不是保留上一次结果——避免把过期路径塞进本轮负载。
 */
async function loadRefs() {
  try {
    if (!rpc.isConnected) await rpc.waitForConnection(8_000)
    const res = await rpc.call<{ ok?: boolean; workspace?: string | null; groups?: ContextGroup[] }>(
      'context.references',
      { q: props.query.trim(), workspaceId: props.workspaceId, limit: 6 },
    )
    const groups = res.groups ?? []
    refCandidates.value = groups.flatMap((g) =>
      (g.items ?? []).slice(0, 6).map<Candidate>((it) => ({
        kind: 'ref',
        id: `${g.id}:${it.id}`,
        label: it.label,
        sub: it.subtitle || g.label,
        ref: { kind: it.kind || g.id, id: it.id, label: it.label, path: it.path },
      })),
    )
  } catch {
    refCandidates.value = []
  }
}

let debounce: ReturnType<typeof setTimeout> | null = null
watch(
  () => [props.query, props.workspaceId],
  () => {
    if (debounce) clearTimeout(debounce)
    debounce = setTimeout(() => void loadRefs(), 180)
  },
)

onMounted(() => {
  void load()
  void loadRefs()
})
onBeforeUnmount(() => {
  if (debounce) clearTimeout(debounce)
})

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
  if (c) emit('select', { label: c.label, ref: c.ref })
}
function pick(c: Candidate) {
  emit('select', { label: c.label, ref: c.ref })
}

/** 引用项按来源给图标：目录用 folder，其余用 fileText，专家/智能体沿用原图标。 */
function itemIcon(c: Candidate): 'folder' | 'fileText' | 'sparkles' | 'agents' {
  if (c.kind === 'expert') return 'sparkles'
  if (c.kind === 'agent') return 'agents'
  return c.ref?.kind === 'folder' ? 'folder' : 'fileText'
}

defineExpose({
  move,
  selectActive,
  hasCandidates: () => candidates.value.length > 0,
})
</script>

<template>
  <div class="mention-popover" role="listbox" aria-label="引用文件 / 规则 / 会话 / 专家 / 智能体">
    <div class="mention-popover__head">引用文件 / 规则 / 会话 / 专家 / 智能体</div>
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
          <Icon :name="itemIcon(c)" :size="14" />
          <span class="mention-popover__label">{{ c.label }}</span>
          <span v-if="c.sub" class="mention-popover__sub">{{ c.sub }}</span>
        </button>
      </li>
    </ul>
    <p v-else class="mention-popover__empty">无匹配的文件、规则、会话、专家或智能体</p>
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
