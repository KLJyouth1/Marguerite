<template>
  <Transition name="cmdp">
  <div
    v-if="open"
    class="cmdp-backdrop"
    role="presentation"
    @mousedown="onBackdrop"
  >
    <section
      ref="dialogRef"
      class="cmdp-dialog"
      role="dialog"
      aria-modal="true"
      :aria-label="t('shared.cmdp.dialogLabel')"
    >
      <div class="cmdp-search">
        <!-- The magnifier sits inside the field so the input reads as the search
             box itself; dismissal stays outside it. -->
        <div class="cmdp-search__field">
          <Icon name="search" :size="16" class="cmdp-search__icon" />
          <input
            v-model="query"
            type="text"
            class="cmdp-search__input"
            :placeholder="t('shared.cmdp.placeholder')"
            role="combobox"
            aria-expanded="true"
            aria-controls="cmdp-listbox"
            aria-autocomplete="list"
            :aria-activedescendant="activeId"
            autocomplete="off"
            spellcheck="false"
            @keydown="onInputKeydown"
          />
        </div>
        <button
          type="button"
          class="cmdp-search__close"
          :aria-label="t('common.close')"
          :title="t('common.close')"
          @click="close()"
        >
          <Icon name="x" :size="15" />
        </button>
      </div>

      <div
        id="cmdp-listbox"
        ref="listRef"
        class="cmdp-list"
        role="listbox"
        :aria-label="t('shared.cmdp.resultsLabel')"
      >
        <p v-if="flatItems.length === 0 && !showSearching" class="cmdp-empty">
          {{ query.trim() ? t('shared.cmdp.noMatches') : t('shared.cmdp.recentsEmpty') }}
        </p>
        <template v-for="group in groups" :key="group.label">
          <template v-if="group.items.length > 0">
            <p class="cmdp-group-label" aria-hidden="true">{{ groupLabel(group.label) }}</p>
            <button
              v-for="item in group.items"
              :id="`cmdp-opt-${item.index}`"
              :key="item.id"
              type="button"
              class="cmdp-option"
              :class="{ 'is-active': item.index === activeIndex }"
              role="option"
              :aria-selected="item.index === activeIndex"
              @click="runItem(item)"
              @mousemove="activeIndex = item.index"
            >
              <Icon v-if="item.icon" :name="item.icon" :size="16" class="cmdp-option__icon" />
              <span class="cmdp-option__body">
                <span class="cmdp-option__label">{{ item.title }}</span>
                <span v-if="item.subtitle" class="cmdp-option__sub">{{ item.subtitle }}</span>
                <!-- eslint-disable-next-line vue/no-v-html — snippet is HTML-escaped in renderSnippet; only <mark> is injected -->
                <span v-if="item.snippetHtml" class="cmdp-option__snippet" v-html="item.snippetHtml"></span>
              </span>
            </button>
          </template>
        </template>
        <p v-if="showSearching" class="cmdp-searching" aria-live="polite">{{ t('shared.cmdp.searching') }}</p>
      </div>
    </section>
  </div>
  </Transition>
</template>

<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { useRouter } from 'vue-router'
import Icon from './Icon.vue'
import { useDialogA11y } from '@/composables/useDialogA11y'
import { useBgm } from '@/composables/useBgm'
import { getGroupedNavigation, getWorkNavigationSection } from '@/router/nav'
import { useRpcStore } from '@/stores/rpc'
import { useActiveMode, switchActiveMode } from '@/composables/useActiveMode'
import { useFreshTaskDraft } from '@/composables/useFreshTaskDraft'
import { sortWorkspacesByRecent, useProjectWorkspaces } from '@/composables/useProjectWorkspaces'
import { useToasts } from '@/composables/useToasts'
import {
  EXPERT_TEAMS,
  loadStoredOrchestration,
  persistOrchestration,
} from '@/composables/chat/orchestration'
import { requestTaskWorkbenchOpen } from '@/workbench/taskItems'
import { requestTerminalWorkbenchOpen } from '@/workbench/terminalItems'
import { requestBrowserWorkbenchOpen } from '@/workbench/browserItems'
import { highlightFtsSnippet } from '@/utils/searchSnippet'
import type { SidebarSection } from '@/composables/useSessions'
import type { IconName } from '@/utils/icons'
import type { MessageSearchHit, SessionSearchHit, SessionsSearchResponse } from '@/types/rpc'

const props = defineProps<{
  open: boolean
  /**
   * Already-loaded sidebar sections. The empty-query state lists these as
   * "Recent tasks" so opening the palette answers "which task?" immediately,
   * with no extra round trip. Destinations and actions stay reachable, but only
   * once the query matches one — an untyped palette full of nav rows is not what
   * a button labelled "search tasks" promises.
   */
  recents: SidebarSection[]
}>()
const emit = defineEmits<{
  (e: 'update:open', value: boolean): void
  (e: 'new-chat'): void
  (e: 'open-settings'): void
  (e: 'toggle-theme'): void
  (e: 'select-session', key: string): void
}>()

const { t } = useI18n()
const router = useRouter()
const rpcStore = useRpcStore()
const { enabled: bgmEnabled, setEnabled: setBgmEnabled } = useBgm()
const { pushToast } = useToasts()
const projectWorkspaces = useProjectWorkspaces()
const freshTaskDraft = useFreshTaskDraft()

const dialogRef = ref<HTMLElement | null>(null)
const listRef = ref<HTMLElement | null>(null)
const query = ref('')
const activeIndex = ref(0)

const open = computed(() => props.open)

// Trap focus, Escape-to-close, and restore focus to the invoker on close. The
// input is the first focusable, so it receives focus on open automatically.
useDialogA11y(dialogRef, open, () => close())

// ---------------------------------------------------------------------------
// Command source — data-driven from the existing nav helpers plus a small set
// of actions. Each entry carries optional keywords for the substring filter.
// ---------------------------------------------------------------------------
type Run = () => void | Promise<void>
interface Command {
  id: string
  title: string
  /** Omitted for task rows: a column of identical chat glyphs is not a signal. */
  icon?: IconName
  keywords: string
  group: string
  /** Secondary line (e.g. a conversation's source surface). */
  subtitle?: string
  /** Pre-sanitized highlight HTML for a transcript snippet (escaped upstream). */
  snippetHtml?: string
  run: Run
}

// Stable group order for the rendered palette. Live switch targets (workspace /
// model / team / agent) sort above the static nav groups so "switch this now"
// beats "go to a page" once the query matches one. Conversation groups sort
// below the static nav/action groups so "go to page" stays the top, instant
// result.
const GROUP_ORDER = [
  'Recents', 'Workspaces', 'Models', 'Teams', 'Agents',
  'Work', 'Operate', 'Observe', 'Settings', 'Extras', 'Actions', 'Modes',
  'Conversations', 'Messages',
] as const

// Display label for a group id. Nav bands resolve through the same nav.* keys
// the sidebar uses, so the palette and the rail always agree per locale.
const GROUP_LABEL_KEYS: Record<string, string> = {
  Recents: 'shared.cmdp.groupRecents',
  Workspaces: 'shared.cmdp.groupWorkspaces',
  Models: 'shared.cmdp.groupModels',
  Teams: 'shared.cmdp.groupTeams',
  Agents: 'shared.cmdp.groupAgents',
  Work: 'shared.cmdp.groupWork',
  // 与侧栏折叠组同名（nav.ts 的 GROUP_TITLES），三档分组在面板与侧栏必须一致。
  Operate: 'nav.groupOperate',
  Observe: 'nav.groupMonitor',
  Settings: 'nav.groupSettings',
  Extras: 'nav.groupExtras',
  Actions: 'shared.cmdp.groupActions',
  Modes: 'shared.cmdp.groupModes',
  Conversations: 'shared.cmdp.groupConversations',
  Messages: 'shared.cmdp.groupMessages',
}
function groupLabel(label: string): string {
  const key = GROUP_LABEL_KEYS[label]
  if (!key) return label
  const translated = t(key)
  // vue-i18n renders the raw key when a locale is missing it (a half-shipped
  // translation would otherwise put `shared.cmdp.groupModes` on screen). The
  // group id is the stable, human-readable fallback.
  return translated === key ? label : translated
}

function navTo(path: string): Run {
  return () => {
    void router.push(path)
  }
}

// Build the full command set once per open (nav helpers are platform-filtered
// and cheap; recomputing on open keeps it correct after platform changes).
const allCommands = computed<Command[]>(() => {
  const out: Command[] = []

  // Work: the pinned rail destinations, from the single Work-band helper so the
  // palette tracks the route taxonomy instead of a hardcoded path list (which
  // silently dropped promoted routes and double-listed demoted ones).
  for (const item of getWorkNavigationSection()) {
    const searchAliases = item.path === '/usage' ? ' usage 用量' : ''
    out.push({
      id: `nav:${item.path}`,
      title: item.title,
      icon: item.icon,
      keywords: `${item.title} ${item.path}${searchAliases}`.toLowerCase(),
      group: 'Work',
      run: navTo(item.path),
    })
    // Channels is the second route in the Skills & Channels hub. Keep its
    // direct command beside the pinned hub entry instead of burying it among
    // Overview's operational subpages.
    if (item.path === '/skills') {
      const title = t('nav.channels')
      out.push({
        id: 'nav:/channels',
        title,
        icon: 'channels',
        keywords: `${title} /channels`.toLowerCase(),
        group: 'Work',
        run: navTo('/channels'),
      })
    }
  }

  // Diagnostic Logs remains directly reachable without promoting it back into
  // the rail; the Overview command already opens Status.
  const demoted: Array<{ path: string; name: string; icon: IconName; group: string }> = [
    { path: '/logs', name: 'logs', icon: 'logs', group: 'Observe' },
  ]
  for (const item of demoted) {
    const title = t(`nav.${item.name}`)
    out.push({
      id: `nav:${item.path}`,
      title,
      icon: item.icon,
      keywords: `${title} ${item.path}`.toLowerCase(),
      group: item.group,
      run: navTo(item.path),
    })
  }

  // 二级分组目的地（引擎健康 / 编辑器 / 终端 / 浏览器驾驶舱 / 应用自控 / 专家中心…）。
  //
  // 这些页面只在侧栏折叠组里，此前命令面板只索引 Work 主 rail + /logs ⇒ 用户按
  // Ctrl+K 搜「应用自控」「浏览器」全部「无匹配项」，页面等于不可达。这里按侧栏
  // 同一份 `getGroupedNavigation()` 补齐，标题/图标与侧栏逐字一致；按 id 去重，
  // 避免 /logs 这类既在 demoted 又进分组的条目出现两次。
  const known = new Set(out.map(command => command.id))
  for (const group of getGroupedNavigation()) {
    for (const item of group.items) {
      const id = `nav:${item.path}`
      if (known.has(id)) continue
      known.add(id)
      out.push({
        id,
        title: item.title,
        icon: item.icon,
        keywords: `${item.title} ${item.path}`.toLowerCase(),
        group: group.id,
        run: navTo(item.path),
      })
    }
  }

  // The Channels hub entry opens the workspace; this separate action lands in
  // its add-channel compose takeover.
  const channelSetupTitle = t('nav.channelSetup')
  out.push({
    id: 'nav:/channels?compose=1',
    title: channelSetupTitle,
    icon: 'channels',
    keywords: `${channelSetupTitle} channels setup config add /channels`.toLowerCase(),
    group: 'Actions',
    run: navTo('/channels?compose=1'),
  })

  // Actions: app-level commands that are not routes.
  out.push(
    {
      id: 'action:new-chat',
      title: t('shared.cmdp.actionNewChat'),
      icon: 'plus',
      keywords: 'new chat conversation compose start',
      group: 'Actions',
      run: () => emit('new-chat'),
    },
    {
      id: 'action:settings',
      title: t('shared.cmdp.actionOpenSettings'),
      icon: 'settings',
      keywords: 'settings preferences configure options',
      group: 'Actions',
      run: () => emit('open-settings'),
    },
    {
      id: 'action:toggle-theme',
      title: t('shared.cmdp.actionToggleTheme'),
      icon: 'monitor',
      keywords: 'theme dark light appearance toggle',
      group: 'Actions',
      run: () => emit('toggle-theme'),
    },
    // Background-music gate: writes the useBgm singleton directly (no App-level
    // routing or handler to reuse, unlike the emit-based actions above). The
    // title tracks the current state via the reactive `enabled` ref.
    {
      id: 'action:toggle-bgm',
      title: bgmEnabled.value ? t('shared.cmdp.actionBgmDisable') : t('shared.cmdp.actionBgmEnable'),
      icon: 'music',
      keywords: 'music bgm background sound audio 音乐 背景音乐',
      group: 'Actions',
      run: () => setBgmEnabled(!bgmEnabled.value),
    },
    // Task-progress panel: docks the delivery-pipeline workbench panel beside
    // the current view. This is the production caller for
    // `createTaskWorkbenchItem` (Gap4) — distinct from the /task route, which is
    // the full-page surface. Dispatches a window event the AppWorkbench listens
    // for, mirroring the terminal/browser panel open paths.
    {
      id: 'action:open-task-panel',
      title: t('task.title'),
      icon: 'listChecks',
      keywords: 'task progress workbench panel steps deliverables 任务 进度 交付物',
      group: 'Actions',
      run: () => {
        requestTaskWorkbenchOpen()
      },
    },
    // 侧栏面板的冷启动入口：工作台在没有任何标签时是**不挂载**的（标题栏的「+」
    // 也就点不到）。调色板这两条会先建出一个面板项，把工作台挂起来，
    // 之后标题栏的「+」就能继续加编辑器/浏览器。
    {
      id: 'action:open-workbench-terminal',
      title: t('workbench.panels.terminal'),
      icon: 'terminal',
      keywords: 'terminal shell console workbench panel 终端 沙箱 工作台',
      group: 'Actions',
      run: () => {
        requestTerminalWorkbenchOpen('')
      },
    },
    {
      id: 'action:open-workbench-browser',
      title: t('workbench.panels.browser'),
      icon: 'languages',
      keywords: 'browser web preview workbench panel 浏览器 内置 预览',
      group: 'Actions',
      run: () => {
        requestBrowserWorkbenchOpen('https://example.com')
      },
    },
  )

  // Modes: switch the runtime mode from anywhere (upgrade-masterplan §6 阶段 D).
  // Reads the shared activeMode so the current mode is always marked; switching
  // goes through switchActiveMode, which the live `mode.changed` frame confirms
  // app-wide.
  //
  // NOTE: `useActiveMode()` must be called during setup (not lazily inside a
  // computed) — `activeMode` is a module-level computed that resolves the Pinia
  // store on evaluation, and evaluating it outside a component instance throws
  // "getActivePinia() was called but there was no active Pinia".
  const activeModeRef = useActiveMode().activeMode
  const currentMode = activeModeRef.value?.kind
    ?? activeModeRef.value?.mode
    ?? ''
  const MODE_COMMANDS: Array<{ kind: string; label: string }> = [
    { kind: 'plan', label: 'Plan' },
    { kind: 'agent', label: 'Agent' },
    { kind: 'ensemble', label: 'Ensemble' },
    { kind: 'bloom', label: 'Bloom' },
    { kind: 'omni-transcend', label: 'Omni Transcend' },
  ]
  for (const m of MODE_COMMANDS) {
    const isActive = m.kind === currentMode
    out.push({
      id: `mode:${m.kind}`,
      title: t('shared.cmdp.actionModeSwitch', { mode: m.label }),
      icon: 'mode',
      keywords: `mode switch ${m.label} ${m.kind} 模式 切换`.toLowerCase(),
      group: 'Modes',
      subtitle: isActive ? t('shared.cmdp.modeActive') : undefined,
      run: () => void switchActiveMode(m.kind),
    })
  }

  return out
})

// ---------------------------------------------------------------------------
// Live switch providers — workspace / model / team / agent.
//
// Loaded lazily when the palette opens (never at boot) and only for methods the
// connected gateway declares; a failed load degrades to "no results" for that
// group instead of blocking the rest of the palette.
//
// Why the palette owns the switching itself instead of emitting to App: these
// are the same state transitions the sidebar / model picker / composer already
// perform, and each one has an existing owner we reuse — `workspaces.open` +
// `useProjectWorkspaces`, `onboarding.llmProfile.activate` (the model picker's
// activation path), `persistOrchestration` (the composer's team selection),
// and the shared `freshTaskDraft` signal + `?agent=` draft route.
// ---------------------------------------------------------------------------
interface PaletteModelRow {
  provider: string
  model: string
  label: string
  /** models.json profile id (Rust profile rows only); catalog rows have none. */
  profileId: string
  active: boolean
}

interface ModelsListResponse {
  models?: Array<Record<string, unknown>>
  profiles?: Array<Record<string, unknown>>
  current?: { provider?: unknown; model?: unknown } | null
}

interface ConfigSnapshot {
  llm?: { provider?: unknown; model?: unknown } | null
  llm_profile_rows?: Array<Record<string, unknown>>
}

interface TeamCompositionRow {
  id?: unknown
  name?: unknown
  label?: unknown
  goal?: unknown
}

interface AgentListResponse {
  agents?: Array<{ id?: unknown; name?: unknown; description?: unknown }>
}

function supports(method: string): boolean {
  return typeof rpcStore.supportsMethod === 'function' && rpcStore.supportsMethod(method)
}

function errorText(err: unknown): string {
  return err instanceof Error ? err.message : String(err)
}

function str(value: unknown): string {
  return typeof value === 'string' ? value.trim() : ''
}

async function selectWorkspace(workspace: { id: string; name: string; path: string }): Promise<void> {
  try {
    // `workspaces.open` is the shared "open this project" path (idempotent for
    // an already-registered folder; W8.1 refreshes its last_opened_at).
    if (rpcStore.canChooseProject) await projectWorkspaces.openWorkspace(workspace.path)
    freshTaskDraft.requestFreshTask('main', workspace.id)
    await router.push({ path: '/chat/new', query: { agent: 'main', project: workspace.id } })
  } catch (err) {
    pushToast(t('shared.cmdp.workspaceSwitchFailed', { error: errorText(err) }), { tone: 'danger' })
  }
}

async function loadWorkspaceCommands(): Promise<Command[]> {
  // Listing must gate on the *open* capability, not just on `workspaces.list`:
  // a gateway that can list but cannot open would otherwise render switch rows
  // whose action silently does nothing. `canChooseProject` = owner + list + open.
  if (!rpcStore.canChooseProject || !supports('workspaces.list')) return []
  try {
    const list = await projectWorkspaces.loadWorkspaces()
    // W8.4: the palette is a "switch to a recent project" affordance, so cap the
    // group at the 5 most recent (pinned first) — the full-management list lives
    // in the project picker dialog. `sortWorkspacesByRecent` applies the same
    // rule as the backend and keeps old gateways (no `lastOpenedAt`) stable.
    return sortWorkspacesByRecent(list).slice(0, 5).map(workspace => ({
      id: `workspace:${workspace.id}`,
      title: workspace.name,
      icon: 'folder' as IconName,
      keywords: `${workspace.name} ${workspace.path} ws workspace workspaces 工作区 空间`.toLowerCase(),
      group: 'Workspaces',
      subtitle: workspace.path,
      run: () => selectWorkspace(workspace),
    }))
  } catch (err) {
    console.warn('[CommandPalette] workspaces.list failed:', errorText(err))
    return []
  }
}

async function activateModel(row: PaletteModelRow): Promise<void> {
  if (row.active) return
  try {
    // Same activation RPC the chat model picker uses; it writes models.json and
    // mirrors config.yaml, then publishes `models.changed` app-wide.
    await rpcStore.call('onboarding.llmProfile.activate', {
      ...(row.profileId ? { profileId: row.profileId } : {}),
      providerId: row.provider,
      model: row.model,
    })
  } catch (err) {
    pushToast(t('shared.cmdp.modelSwitchFailed', { error: errorText(err) }), { tone: 'danger' })
  }
}

function modelRowFromEntry(raw: Record<string, unknown>): PaletteModelRow | null {
  const provider = str(raw.provider)
  const model = str(raw.model) || str(raw.id) || str(raw.name)
  if (!provider || !model) return null
  return {
    provider,
    model,
    // Rust profile rows carry `kind`; provider catalog rows (pyengine) do not.
    profileId: typeof raw.kind === 'string' ? str(raw.id) : '',
    label: str(raw.label) || str(raw.name) || model,
    active: raw.active === true,
  }
}

async function loadModelCommands(): Promise<Command[]> {
  const rows = new Map<string, PaletteModelRow>()
  let currentProvider = ''
  let currentModel = ''

  // Profile rows first: they carry the unambiguous profileId that activation
  // needs when one provider owns several archives.
  if (supports('config.get')) {
    try {
      const cfg = await rpcStore.call<ConfigSnapshot>('config.get')
      currentProvider = str(cfg?.llm?.provider)
      currentModel = str(cfg?.llm?.model)
      for (const raw of cfg?.llm_profile_rows || []) {
        const row = modelRowFromEntry(raw)
        if (row) rows.set(`${row.provider}:${row.model}`, row)
      }
    } catch (err) {
      console.warn('[CommandPalette] config.get failed:', errorText(err))
    }
  }

  // `models.list` also works offline (Rust fallback) and adds provider catalog
  // entries the profile list may not include yet.
  if (supports('models.list')) {
    try {
      const res = await rpcStore.call<ModelsListResponse>('models.list', {})
      currentProvider = str(res?.current?.provider) || currentProvider
      currentModel = str(res?.current?.model) || currentModel
      const list = Array.isArray(res?.models)
        ? res.models
        : (Array.isArray(res?.profiles) ? res.profiles : [])
      for (const raw of list) {
        const row = modelRowFromEntry(raw)
        if (row && !rows.has(`${row.provider}:${row.model}`)) {
          rows.set(`${row.provider}:${row.model}`, row)
        }
      }
    } catch (err) {
      console.warn('[CommandPalette] models.list failed:', errorText(err))
    }
  }

  for (const row of rows.values()) {
    if (!row.active && row.provider === currentProvider && row.model === currentModel) {
      row.active = true
    }
  }

  return [...rows.values()].map(row => ({
    id: `model:${row.provider}:${row.model}`,
    title: row.label,
    icon: 'model' as IconName,
    keywords: `${row.label} ${row.provider} ${row.model} model models 模型`.toLowerCase(),
    group: 'Models',
    subtitle: row.active
      ? `${row.provider} · ${row.model} — ${t('shared.cmdp.modelActive')}`
      : `${row.provider} · ${row.model}`,
    run: () => activateModel(row),
  }))
}

function selectTeam(id: string, label: string): void {
  try {
    const stored = loadStoredOrchestration()
    const teams = stored.teams.includes(id) ? stored.teams : [...stored.teams, id].slice(0, 8)
    // Same rule the composer applies: expert teams switch the form to `expert`,
    // custom compositions to `team`.
    const form = EXPERT_TEAMS.some(team => team.id === id) ? 'expert' : 'team'
    persistOrchestration({ ...stored, teams, form, confirmed: false })
    freshTaskDraft.requestFreshTask('main')
    void router.push({ path: '/chat/new', query: { agent: 'main' } })
    pushToast(t('shared.cmdp.teamSelected', { name: label }), { tone: 'ok' })
  } catch (err) {
    pushToast(t('shared.cmdp.teamSelectFailed', { error: errorText(err) }), { tone: 'danger' })
  }
}

async function loadTeamCommands(): Promise<Command[]> {
  const out: Command[] = []
  const seen = new Set<string>()
  if (supports('team.compositions.list')) {
    try {
      const res = await rpcStore.call<{ items?: TeamCompositionRow[]; compositions?: TeamCompositionRow[] }>(
        'team.compositions.list',
        {},
      )
      const list = Array.isArray(res?.items)
        ? res.items
        : (Array.isArray(res?.compositions) ? res.compositions : [])
      for (const raw of list) {
        const id = str(raw.id) || str(raw.name)
        const label = str(raw.label) || str(raw.name) || id
        if (!id || seen.has(id)) continue
        seen.add(id)
        out.push({
          id: `team:${id}`,
          title: label,
          icon: 'boxes' as IconName,
          keywords: `${label} ${id} ${str(raw.goal)} team teams 团队 专家团`.toLowerCase(),
          group: 'Teams',
          subtitle: str(raw.goal) || id,
          run: () => selectTeam(id, label),
        })
      }
    } catch (err) {
      console.warn('[CommandPalette] team.compositions.list failed:', errorText(err))
    }
  }
  // The built-in expert catalogue is a static registry (same list the composer
  // renders), so those teams stay switchable even when compositions fail.
  for (const team of EXPERT_TEAMS) {
    if (seen.has(team.id)) continue
    seen.add(team.id)
    out.push({
      id: `team:${team.id}`,
      title: team.label,
      icon: 'boxes' as IconName,
      keywords: `${team.label} ${team.id} team teams 团队 专家团`.toLowerCase(),
      group: 'Teams',
      subtitle: team.id,
      run: () => selectTeam(team.id, team.label),
    })
  }
  return out
}

function selectAgent(id: string): void {
  // Reuses the existing agent-chat entry (`/chat/new?agent=…`) plus the shared
  // fresh-task signal so clicking twice on the same draft still starts over.
  freshTaskDraft.requestFreshTask(id)
  void router.push({ path: '/chat/new', query: { agent: id } })
}

async function loadAgentCommands(): Promise<Command[]> {
  if (!supports('agents.list')) return []
  try {
    const res = await rpcStore.call<AgentListResponse>('agents.list', {})
    return (res?.agents || [])
      .map(raw => ({
        id: str(raw.id),
        name: str(raw.name) || str(raw.id),
        description: str(raw.description),
      }))
      .filter(agent => Boolean(agent.id))
      .map(agent => ({
        id: `agent:${agent.id}`,
        title: agent.name,
        icon: 'agents' as IconName,
        keywords: `${agent.name} ${agent.id} agent agents 智能体 代理 专家`.toLowerCase(),
        group: 'Agents',
        subtitle: agent.description || agent.id,
        run: () => selectAgent(agent.id),
      }))
  } catch (err) {
    console.warn('[CommandPalette] agents.list failed:', errorText(err))
    return []
  }
}

const dynamicCommands = ref<Command[]>([])
const paletteLoading = ref(false)
let dynamicLoadToken = 0

async function loadDynamicSources(): Promise<void> {
  const token = ++dynamicLoadToken
  paletteLoading.value = true
  try {
    const [workspaces, models, teams, agents] = await Promise.all([
      loadWorkspaceCommands(),
      loadModelCommands(),
      loadTeamCommands(),
      loadAgentCommands(),
    ])
    if (token !== dynamicLoadToken) return
    dynamicCommands.value = [...workspaces, ...models, ...teams, ...agents]
  } finally {
    if (token === dynamicLoadToken) paletteLoading.value = false
  }
}

// Case-insensitive substring filter on title + keywords. Only consulted for a
// non-empty query: the untyped palette lists recent tasks instead (see
// visibleCommands), so destinations surface by name rather than by default.
const searchableCommands = computed<Command[]>(() => [
  ...allCommands.value,
  ...dynamicCommands.value,
])

const filtered = computed<Command[]>(() => {
  const q = query.value.trim().toLowerCase()
  if (!q) return []
  return searchableCommands.value.filter(
    (cmd) => cmd.title.toLowerCase().includes(q) || cmd.keywords.includes(q),
  )
})

// ---------------------------------------------------------------------------
// Conversation search — async, debounced, server-side over titles + transcript
// content (sessions.search). Results append below the static nav/action groups.
// ---------------------------------------------------------------------------
const sessionHits = ref<SessionSearchHit[]>([])
const messageHits = ref<MessageSearchHit[]>([])
const searching = ref(false)
// One "loading" line covers both providers: transcript search and the live
// switch lists (workspace/model/team/agent) load asynchronously after open.
// The untyped resting state must not read "Searching tasks…" while it is just
// prefetching, so the provider flag only counts once a query exists.
const showSearching = computed(
  () => searching.value || (paletteLoading.value && query.value.trim().length > 0),
)
let debounceTimer: ReturnType<typeof setTimeout> | null = null
// Monotonic token so a slow response from an earlier keystroke can't overwrite
// the results of a later one.
let searchToken = 0

function clearResults() {
  sessionHits.value = []
  messageHits.value = []
  searching.value = false
}

async function runSearch(q: string) {
  const token = ++searchToken
  searching.value = true
  try {
    const res = await rpcStore.call<SessionsSearchResponse>('sessions.search', { query: q, limit: 12 })
    if (token !== searchToken) return
    sessionHits.value = res?.sessions ?? []
    messageHits.value = res?.messages ?? []
  } catch {
    if (token !== searchToken) return
    sessionHits.value = []
    messageHits.value = []
  } finally {
    if (token === searchToken) searching.value = false
  }
}

// Conversation search is async + debounced and never blocks typing: the nav /
// action filter above is synchronous, so "go to page" stays instant regardless
// of search latency. A 2-char floor avoids firing on a lone ASCII letter (which
// matches almost everything), but a single CJK/non-ASCII character is a whole
// word, so allow length-1 when the query is non-ASCII.
const MIN_SEARCH_LEN = 2
const NON_ASCII = /[^\x00-\x7F]/
function shouldSearch(q: string): boolean {
  return q.length >= MIN_SEARCH_LEN || NON_ASCII.test(q)
}
watch(query, (q) => {
  const trimmed = q.trim()
  if (debounceTimer) clearTimeout(debounceTimer)
  if (!shouldSearch(trimmed)) {
    searchToken++ // cancel any in-flight result
    clearResults()
    return
  }
  debounceTimer = setTimeout(() => runSearch(trimmed), 180)
})

const conversationCommands = computed<Command[]>(() => {
  const out: Command[] = []
  for (const hit of sessionHits.value) {
    out.push({
      id: `session:${hit.key}`,
      title: hit.title || t('shared.cmdp.untitledChat'),
      keywords: '',
      group: 'Conversations',
      subtitle: hit.surface && hit.surface !== 'webchat' ? hit.surface : undefined,
      run: () => emit('select-session', hit.key),
    })
  }
  messageHits.value.forEach((hit, i) => {
    out.push({
      id: `message:${hit.key}:${hit.createdAt ?? i}:${i}`,
      title: hit.title || t('shared.cmdp.untitledChat'),
      keywords: '',
      group: 'Messages',
      snippetHtml: highlightFtsSnippet(hit.snippet || ''),
      run: () => emit('select-session', hit.key),
    })
  })
  return out
})

// Recent tasks, flattened from the sidebar sections the app already loaded.
// Capped so the untyped palette stays a short "pick up where you left off" list
// rather than a scroll of the whole ledger; subagent rows (depth > 0) are folded
// under their parent in the sidebar and would read as duplicates here.
const RECENTS_LIMIT = 8
const recentCommands = computed<Command[]>(() => {
  const out: Command[] = []
  const seen = new Set<string>()
  for (const section of props.recents) {
    for (const row of section.rows) {
      if (out.length >= RECENTS_LIMIT) return out
      if (row.rowKind !== 'session' || row.depth > 0 || seen.has(row.key)) continue
      seen.add(row.key)
      out.push({
        id: `recent:${row.key}`,
        title: row.title || t('shared.cmdp.untitledChat'),
        keywords: '',
        group: 'Recents',
        run: () => emit('select-session', row.key),
      })
    }
  }
  return out
})

// Untyped: recent tasks only. Typed: nav/action matches (instant, synchronous)
// followed by conversation matches (async), so nothing is orphaned but the
// resting state stays about tasks.
const visibleCommands = computed<Command[]>(() => (
  query.value.trim()
    ? [...filtered.value, ...conversationCommands.value]
    : recentCommands.value
))

interface FlatItem extends Command {
  index: number
}

// Flatten in group order, assigning each visible item a stable roving index so
// ↑/↓ and aria-activedescendant address a single sequence across groups.
const flatItems = computed<FlatItem[]>(() => {
  const items: FlatItem[] = []
  let index = 0
  for (const label of GROUP_ORDER) {
    for (const cmd of visibleCommands.value) {
      if (cmd.group !== label) continue
      items.push({ ...cmd, index: index++ })
    }
  }
  return items
})

const groups = computed(() =>
  GROUP_ORDER.map((label) => ({
    label,
    items: flatItems.value.filter((item) => item.group === label),
  })).filter((group) => group.items.length > 0),
)

const activeId = computed(() => {
  const item = flatItems.value[activeIndex.value]
  return item ? `cmdp-opt-${item.index}` : undefined
})

// Reset query + selection whenever the palette opens; clear any conversation
// results and cancel in-flight searches on close so the next open starts clean.
// `immediate` covers the already-open mount (App renders it with v-if inside)
// so the live switch providers load on the very first paint too.
watch(open, (isOpen) => {
  if (isOpen) {
    query.value = ''
    activeIndex.value = 0
    clearResults()
    void loadDynamicSources()
  } else {
    if (debounceTimer) clearTimeout(debounceTimer)
    searchToken++
    clearResults()
  }
}, { immediate: true })

// Capabilities and the method table arrive with the WS handshake: a palette
// opened during "connecting" would otherwise stay empty until reopened.
watch(
  () => rpcStore.state,
  (state) => {
    if (state === 'connected' && open.value) void loadDynamicSources()
  },
)

// Clamp the active index whenever the visible set shrinks under the cursor
// (filter narrowing or async results arriving/clearing).
watch(flatItems, () => {
  if (activeIndex.value >= flatItems.value.length) {
    activeIndex.value = Math.max(0, flatItems.value.length - 1)
  }
})

onBeforeUnmount(() => {
  if (debounceTimer) clearTimeout(debounceTimer)
})

function scrollActiveIntoView() {
  void nextTick(() => {
    const el = listRef.value?.querySelector<HTMLElement>('.cmdp-option.is-active')
    el?.scrollIntoView({ block: 'nearest' })
  })
}

function move(delta: number) {
  const count = flatItems.value.length
  if (count === 0) return
  activeIndex.value = (activeIndex.value + delta + count) % count
  scrollActiveIntoView()
}

function onInputKeydown(e: KeyboardEvent) {
  if (e.key === 'ArrowDown') {
    e.preventDefault()
    move(1)
  } else if (e.key === 'ArrowUp') {
    e.preventDefault()
    move(-1)
  } else if (e.key === 'Enter') {
    e.preventDefault()
    const item = flatItems.value[activeIndex.value]
    if (item) runItem(item)
  }
  // Escape is handled by useDialogA11y (document-level), so it is intentionally
  // not intercepted here.
}

function runItem(item: Command) {
  close()
  // Live switches are async (RPC + navigation); never let a rejection escape
  // as an unhandled rejection — each action surfaces its own toast on failure.
  void Promise.resolve(item.run()).catch((err) => {
    console.warn('[CommandPalette] command failed:', item.id, err)
  })
}

function close() {
  emit('update:open', false)
}

function onBackdrop(e: MouseEvent) {
  if (e.target === e.currentTarget) close()
}
</script>

<style scoped>
.cmdp-backdrop {
  position: fixed;
  inset: 0;
  /* Modal tier (above the fixed sidebar at 200) so the scrim dims the rail and
     the palette reads as a true modal — raised above Settings (300). */
  z-index: 320;
  display: flex;
  align-items: flex-start;
  justify-content: center;
  padding: 12vh 16px 16px;
  background: var(--scrim);
}

.cmdp-dialog {
  /* Give task titles and message snippets enough horizontal room without
     turning the palette into a full-page surface. */
  width: min(560px, calc(100vw - 32px));
  max-height: min(600px, calc(100vh - 18vh));
  display: flex;
  flex-direction: column;
  overflow: hidden;
  background: var(--bg-surface);
  border: 1px solid var(--border);
  border-radius: var(--radius-modal);
  box-shadow: var(--shadow-lg);
}

/* Open/close motion: scrim fades, dialog slides up + fades (exit a tier faster).
   Matches the modal idiom (SettingsDialog settings-pop). */
.cmdp-enter-active { transition: opacity var(--dur-base) var(--ease-out); }
.cmdp-leave-active { transition: opacity var(--dur-fast) var(--ease-in); }
.cmdp-enter-from,
.cmdp-leave-to { opacity: 0; }
.cmdp-enter-active .cmdp-dialog {
  transition: transform var(--dur-base) var(--ease-out), opacity var(--dur-base) var(--ease-out);
}
.cmdp-leave-active .cmdp-dialog {
  transition: transform var(--dur-fast) var(--ease-in), opacity var(--dur-fast) var(--ease-in);
}
.cmdp-enter-from .cmdp-dialog { opacity: 0; transform: translateY(8px); }
.cmdp-leave-to .cmdp-dialog { opacity: 0; transform: translateY(6px); }

.cmdp-search {
  display: flex;
  align-items: center;
  gap: var(--sp-2);
  padding: 12px 14px;
}

/* Barely-there fill so the field reads as a field without becoming a slab: a
   hairline border does the work, the tint only hints at depth. The magnifier
   lives inside it rather than floating in the dialog's padding. */
.cmdp-search__field {
  display: flex;
  align-items: center;
  gap: var(--sp-2);
  flex: 1;
  min-width: 0;
  min-height: 44px;
  padding: 0 12px;
  background: var(--bg-surface);
  border: 1px solid var(--border);
  border-radius: var(--radius-control);
  transition: border-color var(--dur-fast), background var(--dur-fast);
}
.cmdp-search__field:focus-within {
  border-color: color-mix(in srgb, var(--accent) 70%, var(--border));
}

.cmdp-search__field:focus-within .cmdp-search__icon {
  color: var(--accent);
}

.cmdp-search__icon {
  color: var(--text-muted);
  flex-shrink: 0;
}

.cmdp-search__field > input.cmdp-search__input,
.cmdp-search__field > input.cmdp-search__input:focus {
  appearance: none;
  background: transparent;
  border: 0;
  border-radius: 0;
  box-shadow: none;
  color: var(--text);
  flex: 1;
  font-family: var(--font-sans);
  font-size: var(--fs-sm);
  font-weight: 400;
  line-height: 20px;
  min-height: 42px;
  min-width: 0;
  outline: none;
  padding: 0;
}

.cmdp-search__input::placeholder {
  color: var(--text-muted);
}

/* Explicit dismiss outside the field: the palette is a task picker people open
   and abandon, and Escape alone is not a visible way out. */
.cmdp-search__close {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 30px;
  height: 30px;
  flex-shrink: 0;
  padding: 0;
  background: transparent;
  border: 0;
  border-radius: var(--radius-sm);
  color: var(--text-muted);
  cursor: pointer;
  transition: background var(--dur-fast), color var(--dur-fast);
}
.cmdp-search__close:hover {
  background: var(--bg-hover);
  color: var(--text);
}
.cmdp-search__close:focus-visible {
  box-shadow: 0 0 0 2px color-mix(in srgb, var(--accent) 18%, transparent);
  outline: 1px solid var(--accent);
  outline-offset: -1px;
}

.cmdp-list {
  min-height: 0;
  overflow-y: auto;
  overscroll-behavior: contain;
  padding: 8px;
}

.cmdp-empty {
  margin: 0;
  padding: 18px 10px;
  text-align: center;
  color: var(--text-muted);
  font-size: var(--fs-sm);
}

.cmdp-group-label {
  margin: var(--sp-2) 0 2px;
  padding: 0 var(--sp-2);
  font-size: var(--fs-xs);
  font-weight: 400;
  line-height: 18px;
  color: var(--text-muted);
}
.cmdp-group-label:first-child {
  margin-top: 0;
}

.cmdp-option {
  display: flex;
  align-items: center;
  gap: var(--sp-2);
  width: 100%;
  /* Roomier rows now that each one is a single line of title: the list is a
     "pick one" surface, so touchable spacing beats packing more in. */
  min-height: 42px;
  padding: var(--sp-2) var(--sp-3);
  border: 1px solid transparent;
  border-radius: var(--radius-sm);
  background: transparent;
  color: var(--text);
  font-family: var(--font-sans);
  font-size: var(--fs-sm);
  line-height: 20px;
  /* Regular weight: the active row is marked by its tint, not by getting
     bolder, so a list of task titles stays quiet to read down. */
  font-weight: 400;
  text-align: left;
  cursor: pointer;
}

.cmdp-option__icon {
  flex-shrink: 0;
  color: var(--text-muted);
}

.cmdp-option__body {
  display: flex;
  flex: 1;
  flex-direction: column;
  gap: 1px;
  min-width: 0;
}

.cmdp-option__label {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

/* Conversation result secondary lines: muted, lighter, single-line ellipsis. */
.cmdp-option__sub,
.cmdp-option__snippet {
  color: var(--text-muted);
  font-size: var(--fs-xs);
  font-weight: 400;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.cmdp-option__snippet :deep(.cmdp-mark) {
  background: color-mix(in srgb, var(--accent) 22%, transparent);
  border-radius: var(--radius-xs);
  color: var(--text);
  padding: 0 1px;
}

.cmdp-searching {
  margin: 0;
  padding: 10px;
  color: var(--text-muted);
  font-size: var(--fs-sm);
}


/* Lark marks the highlighted row with a plain neutral fill — no leading bar, no
   tinted border, no recoloured text. The row is being pointed at, not flagged;
   brand colour stays on the action you press, not on where the cursor is. */
.cmdp-option.is-active {
  background: var(--bg-hover);
  border-color: transparent;
}
.cmdp-option.is-active .cmdp-option__icon {
  color: var(--text-muted);
}

@media (max-width: 768px) {
  .cmdp-backdrop {
    padding: 8vh 12px 12px;
  }
}
</style>
