<!-- 团队/专家团选择面板（可多选、可取消）。
     设计要点（用户反馈）：旧实现只能单选，且点击已选项不会取消 —— 选了团队就无法回到
     「不编排」；也没有多选，无法同时启用「全栈工程 + 安全合规」这类组合。
     现在：
       · 点击已选项 = 取消该团队；
       · 支持多选，后端把所选团队的名册合并、按角色去重后执行；
       · 面板底部给出「已选 N 个」与「全部取消」。 -->
<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import Icon from '@/components/Icon.vue'
import { useI18n } from 'vue-i18n'
import { useRpcStore } from '@/stores/rpc'
import { EXPERT_TEAMS } from '@/composables/chat/orchestration'

const props = defineProps<{
  open: boolean
  /** 当前已选团队 id（team.compositions.list 的 id/name 或 EXPERT_TEAMS 的 id）。 */
  teams: string[]
}>()

const emit = defineEmits<{
  'update:open': [value: boolean]
  toggleTeam: [id: string]
  clearTeams: []
}>()

const { t } = useI18n()
const rpc = useRpcStore()

interface TeamComposition {
  id?: string
  name: string
  label?: string
  goal?: string
  members?: Array<{ role?: string; brief?: string }>
}

const customTeams = ref<TeamComposition[]>([])
let compositionLoaded = false

function loadCompositions() {
  if (compositionLoaded || !rpc.supportsMethod('team.compositions.list')) return
  compositionLoaded = true
  rpc.call<{ items?: TeamComposition[]; compositions?: TeamComposition[] }>('team.compositions.list', {})
    .then((res) => {
      // 后端字段是 `items`（旧前端误读 `compositions`，导致自定义团队永远为空）。
      const list = Array.isArray(res?.items)
        ? res.items
        : (Array.isArray(res?.compositions) ? res.compositions : [])
      customTeams.value = list
    })
    .catch(() => {
      // 静默降级：仅显示专家团，不影响编排入口。
    })
}

// 面板首次打开时载入一次（避免常驻 RPC）。
// 注意必须 watch：组件在面板关闭时就已挂载，只在 setup 里判断 `props.open`
// 会让自定义团队永远为空（真机表现为「自定义团队」分组从不出现）。
if (props.open) loadCompositions()
watch(
  () => props.open,
  (open) => {
    if (open) loadCompositions()
  },
)

const selected = computed(() => new Set(props.teams))

function teamKey(item: TeamComposition): string {
  return item.id || item.name
}

function toggle(key: string) {
  emit('toggleTeam', key)
}
</script>

<template>
  <div v-if="open" class="orch-popover" role="dialog" :aria-label="t('chat.orchestration.teamTitle')">
    <div class="orch-popover__head">
      <Icon name="boxes" :size="14" />
      <span>{{ t('chat.orchestration.teamTitle') }}</span>
      <span v-if="teams.length" class="orch-popover__count">
        {{ t('chat.orchestration.teamSelected', { n: teams.length }) }}
      </span>
    </div>
    <div class="orch-popover__body">
      <p class="orch-popover__hint orch-popover__hint--top">
        {{ t('chat.orchestration.teamMultiHint') }}
      </p>

      <p class="orch-popover__section" v-if="customTeams.length">
        {{ t('chat.orchestration.customTeamsTitle') }}
        <span class="orch-popover__count">{{ customTeams.length }}</span>
      </p>
      <button
        v-for="item in customTeams"
        :key="`custom-${teamKey(item)}`"
        type="button"
        class="orch-option orch-team"
        :class="{ 'is-on': selected.has(teamKey(item)) }"
        :aria-pressed="selected.has(teamKey(item)) ? 'true' : 'false'"
        @click="toggle(teamKey(item))"
      >
        <span class="orch-option__check" aria-hidden="true">
          <Icon v-if="selected.has(teamKey(item))" name="check" :size="12" />
        </span>
        <span class="orch-option__copy">
          <strong>{{ item.label || item.name }}</strong>
          <small v-if="item.goal">{{ item.goal }}</small>
          <small v-if="item.members?.length">
            {{ t('chat.orchestration.runMemberCount', { n: item.members.length }) }}
          </small>
        </span>
      </button>

      <p class="orch-popover__section">
        {{ t('chat.orchestration.expertTeamsTitle') }}
        <span class="orch-popover__count">{{ EXPERT_TEAMS.length }}</span>
      </p>
      <button
        v-for="item in EXPERT_TEAMS"
        :key="item.id"
        type="button"
        class="orch-option orch-team"
        :class="{ 'is-on': selected.has(item.id) }"
        :aria-pressed="selected.has(item.id) ? 'true' : 'false'"
        @click="toggle(item.id)"
      >
        <span class="orch-option__check" aria-hidden="true">
          <Icon v-if="selected.has(item.id)" name="check" :size="12" />
        </span>
        <span class="orch-option__copy">
          <strong>{{ item.label }}</strong>
        </span>
      </button>

      <div class="orch-popover__foot">
        <span v-if="!teams.length" class="orch-popover__foot-hint">
          {{ t('chat.orchestration.teamNone') }}
        </span>
        <button
          v-else
          type="button"
          class="orch-popover__clear"
          @click="emit('clearTeams')"
        >
          {{ t('chat.orchestration.teamClear') }}
        </button>
      </div>
    </div>
  </div>
</template>

<style scoped>
.orch-popover {
  position: absolute;
  right: 0;
  bottom: calc(100% + 6px);
  width: min(320px, calc(100vw - 32px));
  border: 1px solid var(--border-strong);
  border-radius: var(--radius-md);
  background: var(--bg-elevated, var(--bg-surface));
  box-shadow: var(--shadow-xl);
  z-index: 40;
  overflow: hidden;
}
.orch-popover__head {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 8px 12px;
  border-bottom: 1px solid var(--border);
  color: var(--text-dim);
  font-size: var(--fs-xs);
  font-weight: 700;
  letter-spacing: 0.02em;
}
.orch-popover__body {
  padding: 10px 12px 12px;
  max-height: min(520px, calc(100dvh - 14rem));
  overflow-y: auto;
  overscroll-behavior: contain;
  scrollbar-width: thin;
  scrollbar-color: var(--border-strong) transparent;
}
.orch-popover__section {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
  margin: 6px 0 4px;
  color: var(--text-dim);
  font-size: var(--fs-xs);
  font-weight: 600;
}
.orch-popover__count {
  color: var(--text-dim);
  font-weight: 500;
  font-variant-numeric: tabular-nums;
}
.orch-option {
  display: flex;
  align-items: flex-start;
  gap: 8px;
  width: 100%;
  padding: 7px 8px;
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  background: transparent;
  color: var(--text-muted);
  font: inherit;
  text-align: left;
  cursor: pointer;
  transition:
    border-color var(--dur-fast) var(--ease-standard),
    background var(--dur-fast) var(--ease-standard),
    color var(--dur-fast) var(--ease-standard);
}
.orch-option + .orch-option {
  margin-top: 6px;
}
.orch-option:hover,
.orch-option:focus-visible {
  outline: 0;
  border-color: color-mix(in srgb, var(--accent) 45%, var(--border));
  background: var(--bg-hover);
}
.orch-option.is-on {
  border-color: var(--accent);
  background: color-mix(in srgb, var(--accent) 9%, transparent);
  color: var(--text);
}
.orch-option__check {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  flex: 0 0 18px;
  width: 18px;
  height: 18px;
  margin-top: 1px;
  border: 1px solid var(--border-strong);
  border-radius: var(--radius-sm);
  color: var(--accent);
}
.orch-option.is-on .orch-option__check {
  border-color: var(--accent);
  background: var(--accent);
  color: var(--bg-surface);
}
.orch-option__copy {
  display: flex;
  flex-direction: column;
  gap: 2px;
  min-width: 0;
}
.orch-option__copy strong {
  color: var(--text);
  font-size: var(--fs-sm);
  font-weight: 650;
}
.orch-option__copy small {
  color: var(--text-dim);
  font-size: var(--fs-xs);
  line-height: 1.4;
}
.orch-popover__hint {
  margin: 10px 0 0;
  padding-top: 8px;
  border-top: 1px solid var(--border);
  color: var(--text-dim);
  font-size: var(--fs-xs);
  line-height: 1.5;
}
.orch-popover__hint--top {
  margin: 0 0 4px;
  padding: 0;
  border-top: 0;
}
.orch-popover__foot {
  display: flex;
  justify-content: flex-end;
  margin-top: 10px;
  padding-top: 8px;
  border-top: 1px solid var(--border);
}
.orch-popover__foot-hint {
  color: var(--text-dim);
  font-size: var(--fs-xs);
}
.orch-popover__clear {
  padding: 4px 10px;
  border: 1px solid var(--border);
  border-radius: var(--radius-pill);
  background: transparent;
  color: var(--text-muted);
  font-size: var(--fs-xs);
  cursor: pointer;
}
.orch-popover__clear:hover,
.orch-popover__clear:focus-visible {
  outline: 0;
  border-color: color-mix(in srgb, var(--accent) 45%, var(--border));
  color: var(--text);
}
</style>
