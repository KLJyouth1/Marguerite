<template>
  <div class="empty-state">
    <p v-if="showIntro" class="empty-state__greeting">{{ greeting }}</p>
    <p v-if="agentName" class="empty-state__agent">
      <span class="empty-state__agent-dot" aria-hidden="true"></span>
      {{ t('chat.mainAgent') }} · {{ agentName }}
    </p>
    <p v-if="showIntro" class="empty-state__lead">{{ t('chat.emptyLead') }}</p>
    <ul class="empty-state__constellation" aria-hidden="true">
      <li v-for="node in orbitNodes" :key="node">{{ node }}</li>
    </ul>
    <div
      v-if="!suppressed"
      class="empty-state__chips"
      role="group"
      :aria-label="t('chat.suggestedTasks')"
      :aria-disabled="disabled || undefined"
    >
      <button
        v-for="chip in chips"
        :key="chip.label"
        type="button"
        class="empty-state__chip"
        :disabled="disabled"
        @click="emit('pick', chip.prompt)"
      >{{ chip.label }}</button>
    </div>
    <!-- W8.4: reopen a recent project straight from the landing page. Data and
         action reuse the shared project-workspace store (`workspaces.list` +
         `workspaces.open`), so no new RPC surface is introduced here. -->
    <div
      v-if="!suppressed && recentWorkspaces.length > 0"
      class="empty-state__recent"
      role="group"
      :aria-label="t('workspaces.recentTitle')"
    >
      <span class="empty-state__recent-label">{{ t('workspaces.recentTitle') }}</span>
      <button
        v-for="workspace in recentWorkspaces"
        :key="workspace.id"
        type="button"
        class="empty-state__recent-chip"
        :title="workspace.path"
        :disabled="disabled || switchingWorkspaceId === workspace.id"
        @click="selectRecentWorkspace(workspace)"
      >{{ workspace.name }}</button>
    </div>
  </div>
</template>

<script setup lang="ts">
import { computed, getCurrentInstance, onMounted, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { useRpcCall } from '@/composables/useRpc'
import { useRpcStore } from '@/stores/rpc'
import { useFreshTaskDraft } from '@/composables/useFreshTaskDraft'
import {
  sortWorkspacesByRecent,
  useProjectWorkspaces,
} from '@/composables/useProjectWorkspaces'
import { useToasts } from '@/composables/useToasts'
import { optionalSessionRpcCallOptions } from '@/composables/chat/sessionBootstrapAdmission'

const { t } = useI18n()

interface CapabilityStatus {
  searchConfigured?: boolean
  imageGenerationConfigured?: boolean
  imageGenerationEnabled?: boolean
}

const props = withDefaults(defineProps<{
  agentId: string
  /**
   * 当前主智能体展示名（由 ChatView 注入）。
   * 有意做成 prop 而非在本组件里读 `useAgentOptions()`：本组件是纯展示层，
   * 一旦自己连 store，所有只挂载它的单测都必须补 Pinia 与 RPC mock——
   * 这正是本次改动踩到的回归（4 个用例因 `getActivePinia()` 全红）。
   */
  agentName?: string
  suppressed?: boolean
  disabled?: boolean
  /** 落地页 hero 已承担标题与副句时置 false，避免两段大标题互相竞争。 */
  showIntro?: boolean
}>(), {
  agentName: '',
  suppressed: false,
  disabled: false,
  showIntro: true,
})

const emit = defineEmits<{
  pick: [text: string]
}>()

// ---------------------------------------------------------------------------
// W8.4 recent-workspace shortcut.
//
// Deliberately lazy and guarded: this component stays renderable without any
// app context (its unit tests mount it standalone), so the store wiring is only
// created when an app-level Pinia exists. Both calls reuse the shared project
// workspace store — no new RPC methods — and ChatView already reacts to the
// `freshTaskDraft` signal, so no prop/event plumbing is needed here.
// ---------------------------------------------------------------------------
const RECENT_WORKSPACE_LIMIT = 3
const recentWorkspaces = ref<Array<{ id: string; name: string; path: string }>>([])
const switchingWorkspaceId = ref('')

// The guard checks the *mounted app's* installed Pinia (`app.use(pinia)`), not
// the ambient active one: unit tests mount this component standalone and a
// module-global active Pinia left by another suite must not make it warn about
// a missing injection.
const appPinia = getCurrentInstance()?.appContext.config.globalProperties.$pinia
const workspaceApi = appPinia
  ? (() => {
      const rpc = useRpcStore()
      const projectWorkspaces = useProjectWorkspaces()
      const freshTaskDraft = useFreshTaskDraft()
      const { pushToast } = useToasts()

      async function load() {
        // `canChooseProject` = local owner + `workspaces.list` + `workspaces.open`;
        // without it the shortcut could not actually switch anywhere.
        if (!rpc.canChooseProject || !rpc.supportsMethod('workspaces.list')) return
        try {
          const list = await projectWorkspaces.loadWorkspaces()
          recentWorkspaces.value = sortWorkspacesByRecent(list).slice(0, RECENT_WORKSPACE_LIMIT)
        } catch (err) {
          // A failed list must not take the suggestion chips down with it; the
          // project picker surfaces the actionable error.
          console.warn('[EmptyStateChips] workspaces.list failed:', err)
        }
      }

      async function select(workspace: { id: string; name: string; path: string }) {
        if (switchingWorkspaceId.value) return
        switchingWorkspaceId.value = workspace.id
        try {
          // Same "open project" path the palette and picker use; idempotent for
          // an already-registered folder and records `last_opened_at`.
          await projectWorkspaces.openWorkspace(workspace.path)
          freshTaskDraft.requestFreshTask(props.agentId || 'main', workspace.id)
        } catch (err) {
          const detail = err instanceof Error ? err.message : String(err)
          pushToast(t('shared.cmdp.workspaceSwitchFailed', { error: detail }), { tone: 'danger' })
        } finally {
          switchingWorkspaceId.value = ''
        }
      }

      return { load, select, rpc }
    })()
  : null

// The capability table arrives with the WS handshake; a landing mounted while
// still "connecting" must pick the list up once the gateway is ready instead of
// staying empty until the next navigation.
if (workspaceApi) {
  watch(
    () => [workspaceApi.rpc.state, workspaceApi.rpc.canChooseProject] as const,
    ([state, canChoose]) => {
      if (state === 'connected' && canChoose && recentWorkspaces.value.length === 0) {
        void workspaceApi.load()
      }
    },
  )
}

function selectRecentWorkspace(workspace: { id: string; name: string; path: string }) {
  if (!workspaceApi) return
  void workspaceApi.select(workspace)
}

onMounted(() => {
  if (workspaceApi) void workspaceApi.load()
})

interface SuggestionChip {
  label: string
  prompt: string
}

function ordinaryChip(label: string): SuggestionChip {
  return { label, prompt: label }
}

const buildGameChip = computed<SuggestionChip>(() => ({
  label: t('chat.chips.buildGame'),
  prompt: t('chat.chips.buildGamePrompt'),
}))

const FALLBACK_CHIPS = computed(() => [
  buildGameChip.value,
  ordinaryChip(t('chat.chips.summarizeWebpage')),
  ordinaryChip(t('chat.chips.planWeek')),
])

const capabilityStatus = useRpcCall<CapabilityStatus>(
  'onboarding.status',
  undefined,
  { callOptions: optionalSessionRpcCallOptions },
)

const greeting = computed(() => {
  const hour = new Date().getHours()
  if (hour >= 5 && hour < 12) return t('chat.greetingMorning')
  if (hour >= 12 && hour < 18) return t('chat.greetingAfternoon')
  return t('chat.greetingEvening')
})

const orbitNodes = computed(() => [
  t('chat.orbit.agent'),
  t('chat.orbit.subagent'),
  t('chat.orbit.tools'),
  t('chat.orbit.memory'),
  t('chat.orbit.knowledge'),
])

const chips = computed(() => {
  // 主智能体（`main` = 高级开发工程师「严工」）用专家包自带的三个入口，
  // 而不是通用的「做个游戏 / 总结网页 / 规划一周」。
  //
  // 为什么：人设换了、落地页却在推荐写作/游戏任务，用户第一眼看到的引导与
  // 智能体的实际专长不一致 —— 这是「专家包接进来了但没接完」。三条提示词
  // 直接取自专家包 `quickPrompts`（与桌面端人设同源），措辞保持"你要干什么"
  // 而不是"我能干什么"，让用户一点就进入该专家的工作模式。
  if (props.agentId === 'main') {
    return [
      { label: t('chat.chips.diagnoseProject'), prompt: t('chat.chips.diagnoseProjectPrompt') },
      { label: t('chat.chips.auditEdgeCases'), prompt: t('chat.chips.auditEdgeCasesPrompt') },
      { label: t('chat.chips.researchFirst'), prompt: t('chat.chips.researchFirstPrompt') },
    ]
  }
  const status = capabilityStatus.data.value
  if (!status) return FALLBACK_CHIPS.value
  const derived: SuggestionChip[] = []
  if (status.searchConfigured) derived.push(ordinaryChip(t('chat.chips.searchAiNews')))
  if (status.imageGenerationConfigured && status.imageGenerationEnabled !== false) {
    derived.push(ordinaryChip(t('chat.chips.generateImage')))
  }
  derived.push(
    ordinaryChip(t('chat.chips.summarizeWebpage')),
    buildGameChip.value,
  )
  if (derived.length < 3) derived.push(ordinaryChip(t('chat.chips.planWeek')))
  return derived.slice(0, 4)
})
</script>

<style scoped>
.empty-state {
  /* The landing wrapper disables pointer events; the chips need them back. */
  pointer-events: auto;
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: var(--sp-2);
  position: relative;
  text-align: center;
  max-width: 36rem;
  margin-inline: auto;
}

/* Dawn halo behind the greeting — themes without --atmosphere-dawn simply
   render no halo. Kept behind text via z-index, never intercepts input. */
.empty-state::before {
  background: var(--atmosphere-dawn, transparent);
  border-radius: var(--radius-full);
  content: '';
  filter: blur(48px);
  inset: -34% -16%;
  opacity: 0.55;
  pointer-events: none;
  position: absolute;
  z-index: -1;
}

.empty-state__greeting {
  animation: empty-state-greeting-reveal calc(var(--dur-base) * 2.5) var(--ease-out) both;
  margin: var(--sp-2) 0 0;
  font-family: var(--font-display);
  font-size: clamp(1.75rem, 1rem + 1.8vw, 2.25rem);
  font-weight: 600;
  letter-spacing: var(--track-display);
  color: var(--text);
}

.empty-state__agent {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  margin: 0;
  font-family: var(--font-mono);
  font-size: 11px;
  letter-spacing: 0.04em;
  color: var(--text-muted);
  animation: empty-state-greeting-reveal calc(var(--dur-base) * 2.5) var(--ease-out) both;
  animation-delay: 30ms;
}
.empty-state__agent-dot {
  width: 5px;
  height: 5px;
  border-radius: var(--radius-full);
  background: var(--accent);
  box-shadow: 0 0 0 3px color-mix(in srgb, var(--accent) 18%, transparent);
}

.empty-state__lead {
  margin: 0;
  max-width: 38ch;
  font-size: var(--fs-sm);
  color: var(--text-dim);
  line-height: 1.5;
  animation: empty-state-greeting-reveal calc(var(--dur-base) * 2.5) var(--ease-out) both;
  animation-delay: 40ms;
}

.empty-state__constellation {
  display: flex;
  flex-wrap: wrap;
  justify-content: center;
  gap: 6px;
  margin: var(--sp-2) 0 0;
  padding: 0;
  list-style: none;
}

.empty-state__constellation > li {
  padding: 3px 9px;
  border-radius: var(--radius-pill);
  border: 1px solid color-mix(in srgb, var(--accent) 22%, var(--border));
  background: color-mix(in srgb, var(--accent) 6%, transparent);
  font-family: var(--font-mono);
  font-size: 10px;
  letter-spacing: 0.12em;
  text-transform: uppercase;
  color: var(--text-muted);
  animation: empty-state-greeting-reveal calc(var(--dur-base) * 2.5) var(--ease-out) both;
}

.empty-state__constellation > li:nth-child(1) { animation-delay: 60ms; }
.empty-state__constellation > li:nth-child(2) { animation-delay: 100ms; }
.empty-state__constellation > li:nth-child(3) { animation-delay: 140ms; }
.empty-state__constellation > li:nth-child(4) { animation-delay: 180ms; }
.empty-state__constellation > li:nth-child(5) { animation-delay: 220ms; }

@keyframes empty-state-greeting-reveal {
  from {
    filter: blur(5px);
    opacity: 0;
    transform: translateY(7px);
  }
  to {
    filter: blur(0);
    opacity: 1;
    transform: translateY(0);
  }
}

.empty-state__chips {
  display: flex;
  flex-wrap: wrap;
  justify-content: center;
  gap: var(--sp-2);
  margin-top: var(--sp-4);
  /* Reserve one chip row so late capability resolution cannot shift layout. */
  min-height: 2.25rem;
}

.empty-state__chip {
  display: inline-flex;
  align-items: center;
  min-height: 2.25rem;
  padding: 0.375rem 0.875rem;
  border: 1px solid var(--border);
  border-radius: var(--radius-full);
  background: var(--bg-elevated);
  font: inherit;
  font-size: 0.8125rem;
  color: var(--text-muted);
  cursor: pointer;
  transition: background var(--transition), border-color var(--transition), color var(--transition);
}

.empty-state__chip:hover {
  background: var(--bg-hover);
  border-color: var(--border-strong);
  color: var(--text);
}

.empty-state__chip:disabled {
  cursor: default;
  opacity: 0.72;
}

.empty-state__chip:disabled:hover {
  background: var(--bg-elevated);
  border-color: var(--border);
  color: var(--text-muted);
}

.empty-state__chip:focus-visible {
  outline: none;
  box-shadow: var(--focus-ring);
}

/* W8.4 recent projects: a quieter row than the suggestion chips — it is a
   resume affordance, not the primary call to action. */
.empty-state__recent {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  justify-content: center;
  gap: var(--sp-2);
  margin-top: var(--sp-1);
}

.empty-state__recent-label {
  font-family: var(--font-mono);
  font-size: 10px;
  letter-spacing: 0.12em;
  text-transform: uppercase;
  color: var(--text-dim);
}

.empty-state__recent-chip {
  display: inline-flex;
  align-items: center;
  max-width: 16rem;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  padding: 0.2rem 0.7rem;
  border: 1px solid color-mix(in srgb, var(--accent) 18%, var(--border));
  border-radius: var(--radius-full);
  background: transparent;
  font: inherit;
  font-size: 0.75rem;
  color: var(--text-muted);
  cursor: pointer;
  transition: background var(--transition), border-color var(--transition), color var(--transition);
}

.empty-state__recent-chip:hover {
  background: color-mix(in srgb, var(--accent) 8%, transparent);
  border-color: color-mix(in srgb, var(--accent) 36%, var(--border));
  color: var(--text);
}

.empty-state__recent-chip:disabled {
  cursor: default;
  opacity: 0.72;
}

.empty-state__recent-chip:focus-visible {
  outline: none;
  box-shadow: var(--focus-ring);
}

@media (max-width: 768px) {
  .empty-state__chip {
    min-height: 2.75rem;
  }

  .empty-state__chips {
    min-height: 2.75rem;
  }
}

@media (prefers-reduced-motion: reduce) {
  .empty-state__greeting,
  .empty-state__lead,
  .empty-state__constellation > li {
    animation: none;
  }

  .empty-state__chip {
    transition: none;
  }
}
</style>
