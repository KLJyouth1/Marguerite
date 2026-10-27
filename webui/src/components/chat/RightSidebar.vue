<template>
  <!-- 右侧侧边栏：tasks 轴 + 历史产物 + 文件更改 -->
  <Transition name="rs-slide">
    <aside
      v-if="open"
      class="rs-panel"
      :class="{ 'is-resizing': resizing }"
      :style="{ '--rs-panel-w': panelWidth + 'px' }"
    >
      <!-- 分割线：会话列 ↔ 面板。拖拽改宽、键盘 ←/→ 微调、宽度持久化。
           focusable 的 separator 在 ARIA 里就是「窗口分隔条」，按 slider 语义暴露取值。 -->
      <div
        class="rs-resizer"
        role="separator"
        aria-orientation="vertical"
        tabindex="0"
        :aria-label="t('layout.panel.resizeHint')"
        :title="t('layout.panel.resizeHint')"
        :aria-valuenow="panelWidth"
        :aria-valuemin="RIGHT_PANEL_MIN_WIDTH"
        :aria-valuemax="panelMaxWidth"
        @pointerdown="startResize"
        @keydown="onResizeKeydown"
      />
      <div class="rs-tabs">
        <div ref="tabScrollRef" class="rs-tabs__scroll" role="tablist" :aria-label="t('chat.rightSidebar.tabsLabel')">
          <div
            v-for="tab in tabs"
            :key="tab.id"
            class="rs-tab"
            :class="{ 'rs-tab--active': activeTab === tab.id }"
            role="tab"
            tabindex="0"
            :aria-selected="activeTab === tab.id ? 'true' : 'false'"
            @click="selectTab(tab.id)"
            @keydown.enter.prevent="selectTab(tab.id)"
            @keydown.space.prevent="selectTab(tab.id)"
          >
            <Icon :name="tab.icon" :size="14" />
            <span>{{ tab.label }}</span>
            <span v-if="tab.count > 0" class="rs-tab__badge">{{ tab.count }}</span>
            <button
              v-if="activeTab === tab.id && openTabs.length > 1"
              type="button"
              class="rs-tab__close"
              :title="t('chat.rightSidebar.closeTab', { tab: tab.label })"
              :aria-label="t('chat.rightSidebar.closeTab', { tab: tab.label })"
              @click.stop="closeTab(tab.id)"
            >
              <Icon name="x" :size="11" />
            </button>
          </div>
        </div>
        <div class="rs-tabs__tools">
          <!-- 侧聊天：默认关闭；点这个按钮等于「打开 / 聚焦」侧聊天轴。
               真正的会话键由 SideChatPanel 在挂载时按主会话键创建并持久化。 -->
          <button
            type="button"
            class="rs-icon-btn rs-sidechat-toggle"
            :class="{ 'rs-icon-btn--active': activeTab === 'sideChat' }"
            data-testid="rs-sidechat-toggle"
            :title="t('chat.sideChat.title')"
            :aria-label="t('chat.sideChat.title')"
            :aria-pressed="activeTab === 'sideChat' ? 'true' : 'false'"
            @click="openSideChat"
          >
            <Icon name="chat" :size="14" />
          </button>
          <div class="rs-tabs__add">
            <button
              ref="addTabBtnRef"
              type="button"
              class="rs-icon-btn"
              :title="t('chat.rightSidebar.addTab')"
              :aria-label="t('chat.rightSidebar.addTab')"
              :aria-expanded="addMenuOpen ? 'true' : 'false'"
              @click="addMenuOpen = !addMenuOpen"
            >
              <Icon name="plus" :size="14" />
            </button>
            <div
              v-if="addMenuOpen"
              ref="addMenuRef"
              class="rs-tabs__add-menu"
              role="menu"
              :aria-label="t('chat.rightSidebar.addTab')"
            >
              <p v-if="closedTabs.length === 0" class="rs-tabs__add-empty">
                {{ t('chat.rightSidebar.addTabEmpty') }}
              </p>
              <button
                v-for="tab in closedTabs"
                :key="tab.id"
                type="button"
                role="menuitem"
                @click="reopenTab(tab.id)"
              >
                <Icon :name="tab.icon" :size="13" />
                {{ tab.label }}
              </button>
            </div>
          </div>
          <button
            type="button"
            class="rs-icon-btn"
            :title="wide ? t('chat.rightSidebar.narrow') : t('chat.rightSidebar.widen')"
            @click="toggleWide"
          >
            <Icon :name="wide ? 'panel-right-open' : 'panel-right-close'" :size="14" />
          </button>
          <button
            type="button"
            class="rs-icon-btn"
            :title="t('layout.panel.collapse')"
            @click="emit('close')"
          >
            <Icon name="x" :size="14" />
          </button>
        </div>
      </div>

      <div class="rs-body">
        <!-- 任务 = 本次运行的实时执行流：工具调用 / 思考 / 文件读写 / 总结 -->
        <div v-if="activeTab === 'tasks'" class="rs-section">
          <div class="rs-run-head">
            <span class="rs-run-live" :class="{ 'is-on': streaming }">
              <i aria-hidden="true" />
              {{ streaming ? t('chat.rightSidebar.runLive') : t('chat.rightSidebar.tabs.tasks') }}
            </span>
            <span class="rs-run-count">{{ tasks.length }}</span>
          </div>
          <div v-if="tasks.length === 0" class="rs-empty">
            <Icon name="listChecks" :size="28" :opacity="0.3" />
            <p>{{ t('chat.rightSidebar.tasksEmpty') }}</p>
          </div>
          <ol v-else class="rs-timeline">
            <li
              v-for="(task, i) in tasks"
              :key="task.id || i"
              class="rs-timeline__item"
              :class="`rs-timeline__item--${task.status}`"
            >
              <span class="rs-timeline__dot" />
              <div class="rs-timeline__content">
                <button
                  type="button"
                  class="rs-timeline__title rs-run-row"
                  :class="{ 'is-open': isRunRowOpen(task, i), 'is-static': !runRowHasBody(task) }"
                  :aria-expanded="runRowHasBody(task) ? (isRunRowOpen(task, i) ? 'true' : 'false') : undefined"
                  :disabled="!runRowHasBody(task)"
                  @click="toggleRunRow(task, i)"
                >
                  <Icon :name="runRowIcon(task)" :size="12" class="rs-run-row__icon" />
                  <span class="rs-run-row__text">{{ runRowTitle(task) }}</span>
                  <span v-if="task.duration" class="rs-timeline__time">{{ task.duration }}</span>
                  <Icon
                    v-if="runRowHasBody(task)"
                    name="chevronDown"
                    :size="12"
                    class="rs-run-row__chevron"
                    :class="{ 'is-open': isRunRowOpen(task, i) }"
                  />
                </button>
                <div v-if="task.detail" class="rs-timeline__detail">{{ task.detail }}</div>
                <div v-if="task.tools && task.tools.length > 0" class="rs-timeline__tools">
                  <span v-for="tool in task.tools" :key="tool" class="rs-tool-chip">
                    <Icon name="play" :size="10" />{{ tool }}
                  </span>
                </div>
                <div v-if="isRunRowOpen(task, i)" class="rs-run-body">
                  <div v-if="task.input" class="rs-run-block">
                    <span class="rs-run-block__label">{{ t('chat.rightSidebar.runInput') }}</span>
                    <pre class="rs-run-pre">{{ task.input }}</pre>
                  </div>
                  <div v-if="task.output" class="rs-run-block">
                    <span class="rs-run-block__label">{{ t('chat.rightSidebar.runOutput') }}</span>
                    <pre class="rs-run-pre">{{ task.output }}</pre>
                  </div>
                </div>
              </div>
            </li>
          </ol>
        </div>

        <!-- 子智能体轴：同一会话内每个 child session 的最新完成状态。
             不做多标签容器；从消息流点击子智能体卡时按 key 过滤。 -->
        <div v-if="activeTab === 'subagents'" class="rs-section">
          <div class="rs-run-head">
            <span class="rs-run-live" :class="{ 'is-on': runningSubagentCount > 0 }">
              <i aria-hidden="true" />
              {{ runningSubagentCount > 0
                ? t('chat.rightSidebar.subagentsLive', { n: runningSubagentCount })
                : t('chat.rightSidebar.tabs.subagents') }}
            </span>
            <span class="rs-run-count">{{ visibleSubagents.length }}</span>
          </div>
          <button
            v-if="subagentFilter"
            type="button"
            class="rs-subagent-filter"
            @click="subagentFilter = ''"
          >
            <Icon name="x" :size="11" />
            {{ t('chat.rightSidebar.subagentFilter', { key: shortSubagentKey(subagentFilter) }) }}
          </button>
          <div v-if="visibleSubagents.length === 0" class="rs-empty">
            <Icon name="boxes" :size="28" :opacity="0.3" />
            <p>{{ subagentFilter
              ? t('chat.rightSidebar.subagentFilterEmpty')
              : t('chat.rightSidebar.subagentsEmpty') }}</p>
          </div>
          <ul v-else class="rs-subagents">
            <li
              v-for="item in visibleSubagents"
              :key="item.key"
              class="rs-subagent"
              :class="`rs-subagent--${item.tone}`"
            >
              <span class="rs-subagent__dot" aria-hidden="true" />
              <div class="rs-subagent__body">
                <div class="rs-subagent__head">
                  <code class="rs-subagent__key" :title="item.key">{{ shortSubagentKey(item.key) }}</code>
                  <span class="rs-subagent__status">{{ subagentStatusLabel(item.tone) }}</span>
                </div>
                <p v-if="item.summary" class="rs-subagent__summary">{{ item.summary }}</p>
                <time v-if="item.ts" class="rs-subagent__time" :datetime="subagentIso(item.ts)">{{ subagentTime(item.ts) }}</time>
              </div>
            </li>
          </ul>
        </div>

        <!-- 历史产物 -->
        <div v-if="activeTab === 'artifacts'" class="rs-section">
          <!-- 诊断条：本次会话已有产物但未载入侧边栏（重连后、索引尚未就绪等）。
               作为兜底入口，避免「生成了文件却看不到」的死路。 -->
          <div v-if="diagnosticList.length > 0" class="rs-diagnostic">
            <Icon name="alertTriangle" :size="14" />
            <span>{{ t('chat.rightSidebar.artifactsDiagnostic', { count: diagnosticList.length }) }}</span>
            <button type="button" class="rs-diagnostic__retry" @click="emit('retryArtifacts')">
              <Icon name="refresh" :size="12" />{{ t('chat.rightSidebar.artifactsReload') }}
            </button>
          </div>

          <div v-if="artifacts.length === 0" class="rs-empty rs-empty--artifacts">
            <Icon name="fileStack" :size="28" :opacity="0.3" />
            <p class="rs-empty__lead">{{ t('chat.rightSidebar.artifactsEmptyLead') }}</p>
            <p class="rs-empty__sub">{{ t('chat.rightSidebar.artifactsEmptySub') }}</p>
            <div class="rs-empty__actions">
              <!-- A running turn owns the session; offering "send a task" here would
                   conflict with it, so surface the running state instead of a dead CTA. -->
              <span v-if="streaming" class="rs-empty__hint">
                {{ t('chat.rightSidebar.artifactsRunningHint') }}
              </span>
              <button v-else type="button" class="rs-empty__btn" @click="emit('compose')">
                <Icon name="pencil" :size="13" />{{ t('chat.rightSidebar.artifactsCompose') }}
              </button>
              <button type="button" class="rs-empty__btn rs-empty__btn--ghost" @click="emit('retryArtifacts')">
                <Icon name="refresh" :size="13" />{{ t('chat.rightSidebar.artifactsReload') }}
              </button>
            </div>
          </div>

          <ul v-else class="rs-artifact-list">
            <li v-for="a in artifacts" :key="artifactKey(a)" class="rs-artifact">
              <span
                class="rs-artifact__type-dot"
                :class="`rs-artifact__type-dot--${statusFor(a)}`"
                :title="statusTitle(a)"
                aria-hidden="true"
              />
              <Icon :name="classify(a).icon" :size="16" :style="{ color: classify(a).colorVar }" />
              <div class="rs-artifact__info">
                <div class="rs-artifact__name" :title="a.name || a.path">{{ a.name || artifactBasename(a.path) }}</div>
                <div class="rs-artifact__path">{{ a.path || formatLabel(a) }}</div>
              </div>
              <span class="rs-artifact__format" :style="{ color: classify(a).colorVar }">
                {{ formatLabel(a) }}
              </span>
              <button
                type="button"
                class="rs-artifact__btn"
                :title="t('chat.rightSidebar.artifactOpenInApp')"
                @click="emit('openArtifact', a)"
              >
                <Icon name="edit" :size="13" />
              </button>
              <button
                type="button"
                class="rs-artifact__btn"
                :title="t('chat.rightSidebar.artifactOpenSystem')"
                @click="emit('openArtifactSystem', a)"
              >
                <Icon name="appWindow" :size="13" />
              </button>
              <div class="rs-artifact__menu-wrap">
                <button
                  type="button"
                  class="rs-artifact__btn"
                  :title="t('chat.rightSidebar.artifactMore')"
                  aria-haspopup="menu"
                  :aria-expanded="openMenuKey === artifactKey(a)"
                  @click.stop="toggleMenu(a)"
                >
                  <Icon name="moreHorizontal" :size="14" />
                </button>
                <ul v-if="openMenuKey === artifactKey(a)" class="rs-artifact__menu" role="menu">
                  <li role="none">
                    <button type="button" role="menuitem" @click="previewArtifact(a)">
                      <Icon name="eye" :size="14" /><span>{{ t('chat.rightSidebar.artifactPreview') }}</span>
                    </button>
                  </li>
                  <li role="none">
                    <button type="button" role="menuitem" @click="editArtifact(a)">
                      <Icon name="edit" :size="14" /><span>{{ t('chat.rightSidebar.artifactEdit') }}</span>
                    </button>
                  </li>
                  <li role="none">
                    <button type="button" role="menuitem" @click="openArtifactSystemAction(a)">
                      <Icon name="appWindow" :size="14" /><span>{{ t('chat.rightSidebar.artifactSystemOpen') }}</span>
                    </button>
                  </li>
                  <li role="none">
                    <button type="button" role="menuitem" @click="copyArtifactPath(a)">
                      <Icon name="copy" :size="14" /><span>{{ t('chat.rightSidebar.artifactCopyPath') }}</span>
                    </button>
                  </li>
                </ul>
              </div>
            </li>
          </ul>
        </div>

        <!-- 文件更改 -->
        <div v-if="activeTab === 'changes'" class="rs-section">
          <div v-if="changes.length === 0" class="rs-empty">
            <Icon name="fork" :size="28" :opacity="0.3" />
            <p>{{ t('chat.rightSidebar.changesEmpty') }}</p>
          </div>
          <ul v-else class="rs-change-list">
            <li v-for="c in changes" :key="c.path" class="rs-change">
              <Icon :name="changeIcon(c)" :size="14" />
              <span class="rs-change__path" :title="c.path">{{ c.name || c.path }}</span>
              <span v-if="c.additions > 0" class="rs-change__add">+{{ c.additions }}</span>
              <span v-if="c.deletions > 0" class="rs-change__del">-{{ c.deletions }}</span>
            </li>
          </ul>
        </div>

        <!-- 文件树：IDE 式项目资源管理器（可展开 / 双击打开 / 右键菜单） -->
        <div v-if="activeTab === 'tree'" class="rs-section rs-section--explorer">
          <FileExplorerPanel :root-path="rootPath || ''" :scope-id="sessionKey" />
        </div>

        <!-- 侧聊天：独立会话键的第二对话。保持挂载（v-show）以免切走标签时
             丢掉正在流式输出的回复；关闭该标签才真正卸载并退订事件。 -->
        <div
          v-if="openTabs.includes('sideChat')"
          v-show="activeTab === 'sideChat'"
          class="rs-section rs-section--sidechat"
        >
          <SideChatPanel :main-session-key="sessionKey || ''" />
        </div>
      </div>
    </aside>
  </Transition>

  <!-- 折叠按钮（侧边栏关闭时显示在右边缘）。
       尺寸从 28×56 放大到 40×96：旧尺寸在 1440 宽屏上只有指甲盖大，用户反馈「按钮太小」。 -->
  <button
    v-if="!open"
    type="button"
    class="rs-toggle"
    :title="t('layout.panel.expand')"
    :aria-label="t('layout.panel.expand')"
    @click="emit('open')"
  >
    <Icon name="panel-right-open" :size="18" />
    <span class="rs-toggle__label" aria-hidden="true">{{ t('layout.panel.expand') }}</span>
  </button>
</template>

<script setup lang="ts">
import { computed, nextTick, onMounted, onUnmounted, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import Icon from '@/components/Icon.vue'
import type { IconName } from '@/utils/icons'
import { useDocumentEvent } from '@/composables/useDocumentEvent'
import type { ArtifactPayload } from '@/types/rpc'
import type { SubagentPanelEntry, SubagentTone } from '@/utils/chat/subagent'
import { absoluteTime, isoTime } from '@/utils/messageTime'
import { classifyArtifact, type ArtifactKindDescriptor } from '@/utils/artifactKind'
import { useToasts } from '@/composables/useToasts'
import {
  RIGHT_PANEL_DEFAULT_WIDTH,
  RIGHT_PANEL_MIN_WIDTH,
  usePanelResize,
} from '@/composables/usePanelResize'
import FileExplorerPanel from './FileExplorerPanel.vue'
import SideChatPanel from './SideChatPanel.vue'

export interface RsTask {
  title: string
  detail?: string
  status: 'running' | 'done' | 'error' | 'pending'
  tools?: string[]
  duration?: string
  /** 行类型：决定左侧图标与默认是否展开 */
  kind?: 'tool' | 'thinking' | 'summary' | 'step'
  /** 入参（命令行 / 文件路径 / JSON 摘要） */
  input?: string
  /** 出参（命令输出 / 文件内容摘录 / 工具结果） */
  output?: string
  /** 稳定 key（工具调用用 toolId，思考/总结用固定前缀） */
  id?: string
}

export type RsArtifact = ArtifactPayload & { openable?: boolean }

export interface RsChange {
  path: string
  name?: string
  additions: number
  deletions: number
  kind?: 'created' | 'modified' | 'deleted'
}

/** 子智能体轴的一行：来自会话内 subagent_completion 消息，不新增 RPC。 */
export type RsSubagent = SubagentPanelEntry

const props = defineProps<{
  open: boolean
  tasks: RsTask[]
  subagents?: RsSubagent[]
  artifacts: RsArtifact[]
  changes: RsChange[]
  /** 诊断：有产物但未载入侧边栏（如重连后索引未就绪）。 */
  diagnostics?: string[]
  /** 是否正在流式输出（当前 turn 实时写入产物）。 */
  streaming?: boolean
  /** 流式期间经 session.event.artifact 出现的产物 key 集合。 */
  liveKeys?: string[]
  /** 后端已有更新版本、侧边栏仍指向旧内容的产物 key 集合。 */
  staleKeys?: string[]
  /**
   * 当前会话的工作区根目录。
   * 文件树此前只发 `{ path: '' }`，后端按 `USERPROFILE` → `HOME` 回退，
   * 于是「工作区目录」实际列的是 C:\Users\<user>（AppData/Cookies/Desktop…），
   * 不是用户正在做的项目。这里把真实根目录透给 `sandbox.path.list` 的 `basePath`。
   */
  rootPath?: string
  /** 当前会话键（项目树打开文件时归属到该会话作用域） */
  sessionKey?: string
  /** 外部请求切换标签（输入框「+」→ 我的产物 / 工作区文件 / 侧聊天）。 */
  focusTab?: 'tasks' | 'subagents' | 'artifacts' | 'changes' | 'tree' | 'sideChat'
  /** 每次请求递增：同一个标签连续点两次也要生效。 */
  focusSeq?: number
  /** 从消息流点击子智能体卡时传入 child session key；空串表示不过滤。 */
  focusSubagentKey?: string
}>()

const emit = defineEmits<{
  close: []
  open: []
  openArtifact: [a: RsArtifact]
  openArtifactSystem: [a: RsArtifact]
  retryArtifacts: []
  compose: []
}>()

const { t } = useI18n()

// ── 运行流：可展开行 ──
// 默认只展开「正在跑」和「出错」的行：整段输出全铺开会把侧栏变成一堵墙，
// 但用户要能一眼看到失败原因与实时输出，所以这两类默认打开。
const openRunRows = ref<Set<string>>(new Set())
function runRowKey(task: RsTask, index: number): string {
  // 优先稳定 id；没有 id 的历史步骤用「kind:title」，**不用下标** ——
  // 流式追加会把下标整体挪位，用下标会让"刚收起的那条"变成另一条。
  // `index` 只在标题也为空时兜底（理论上不会发生）。
  return task.id || `${task.kind || 'step'}:${task.title || index}`
}
function runRowHasBody(task: RsTask): boolean {
  return Boolean(task.input || task.output)
}
function isRunRowOpen(task: RsTask, index: number): boolean {
  if (!runRowHasBody(task)) return false
  const key = runRowKey(task, index)
  if (openRunRows.value.has(key)) return true
  return (task.status === 'running' || task.status === 'error') && !openRunRows.value.has(`closed:${key}`)
}
function toggleRunRow(task: RsTask, index: number) {
  if (!runRowHasBody(task)) return
  const key = runRowKey(task, index)
  const next = new Set(openRunRows.value)
  if (isRunRowOpen(task, index)) {
    next.delete(key)
    next.add(`closed:${key}`)
  } else {
    next.delete(`closed:${key}`)
    next.add(key)
  }
  openRunRows.value = next
}
function runRowIcon(task: RsTask): IconName {
  switch (task.kind) {
    case 'thinking': return 'zap'
    case 'summary': return 'sparkles'
    case 'tool': return 'terminal'
    default: return 'play'
  }
}
/** 思考/总结两类的标题由展示层本地化（数据层只带 kind，不写死任何语言） */
function runRowTitle(task: RsTask): string {
  if (task.kind === 'thinking') return t('chat.rightSidebar.runThinking')
  if (task.kind === 'summary') return t('chat.rightSidebar.runSummary')
  return task.title
}

const ALL_TAB_IDS = ['tasks', 'subagents', 'artifacts', 'changes', 'tree', 'sideChat'] as const
type RsTabId = (typeof ALL_TAB_IDS)[number]
/**
 * 侧聊天刻意**不在**默认打开的集合里：它要消耗一个独立会话，
 * 不该在每次打开右栏时就悄悄创建一个。缺失时由工具按钮 / 加号菜单打开。
 */
const DEFAULT_TAB_IDS: RsTabId[] = ['tasks', 'subagents', 'artifacts', 'changes', 'tree']
const TABS_STORAGE_KEY = 'marguerite-right-sidebar-tabs'

function isRsTabId(value: unknown): value is RsTabId {
  return typeof value === 'string' && (ALL_TAB_IDS as readonly string[]).includes(value)
}

/** 恢复打开的轴与活动轴；缺省（首次使用 / 数据损坏）回到除侧聊天外的全部打开。 */
function loadSidebarTabs(): { open: RsTabId[]; active: RsTabId } {
  const fallback = { open: [...DEFAULT_TAB_IDS], active: 'tasks' as RsTabId }
  try {
    const raw = localStorage.getItem(TABS_STORAGE_KEY)
    if (!raw) return fallback
    const parsed = JSON.parse(raw) as { open?: unknown; active?: unknown }
    const open = Array.isArray(parsed.open)
      ? parsed.open.filter(isRsTabId).filter((id, index, list) => list.indexOf(id) === index)
      : []
    if (open.length === 0) return fallback
    const first = open[0] as RsTabId
    const active = isRsTabId(parsed.active) && open.includes(parsed.active) ? parsed.active : first
    return { open, active }
  } catch {
    return fallback
  }
}

function saveSidebarTabs(state: { open: RsTabId[]; active: RsTabId }) {
  try {
    localStorage.setItem(TABS_STORAGE_KEY, JSON.stringify(state))
  } catch {
    /* private mode */
  }
}

const storedTabs = loadSidebarTabs()
const activeTab = ref<RsTabId>(storedTabs.active)
const openTabs = ref<RsTabId[]>(storedTabs.open)
const addMenuOpen = ref(false)
const addMenuRef = ref<HTMLElement | null>(null)
const addTabBtnRef = ref<HTMLButtonElement | null>(null)
const tabScrollRef = ref<HTMLElement | null>(null)
const subagentFilter = ref('')

// 外部「+」菜单请求的标签切换：按 focusSeq 触发，避免同标签重复点击失效。
watch(
  () => [props.focusTab, props.focusSeq] as const,
  ([tab]) => {
    if (!tab) return
    ensureTabOpen(tab)
    activeTab.value = tab
    if (tab === 'subagents' && !props.focusSubagentKey) subagentFilter.value = ''
  },
)
// 子智能体卡点击：打开对应轴并按 child session 过滤；与 focusSeq 同步，
// 这样连续点同一张卡也会重新聚焦（清掉用户手动清除过的过滤）。
watch(
  () => [props.focusSubagentKey, props.focusSeq] as const,
  ([key]) => {
    if (!key) return
    ensureTabOpen('subagents')
    activeTab.value = 'subagents'
    subagentFilter.value = key
  },
  { immediate: true },
)
watch([openTabs, activeTab], () => {
  saveSidebarTabs({ open: openTabs.value, active: activeTab.value })
})
watch(activeTab, () => {
  void nextTick(() => {
    const el = tabScrollRef.value?.querySelector<HTMLElement>('.rs-tab--active')
    if (el && typeof el.scrollIntoView === 'function') {
      el.scrollIntoView({ block: 'nearest', inline: 'nearest' })
    }
  })
})

useDocumentEvent('pointerdown', event => {
  if (!addMenuOpen.value) return
  const target = event.target as Node | null
  if (target && (addMenuRef.value?.contains(target) || addTabBtnRef.value?.contains(target))) return
  addMenuOpen.value = false
})
useDocumentEvent('keydown', event => {
  if (event.key !== 'Escape' || !addMenuOpen.value) return
  event.stopPropagation()
  addMenuOpen.value = false
})

function ensureTabOpen(tab: RsTabId) {
  if (!openTabs.value.includes(tab)) openTabs.value = [...openTabs.value, tab]
}

function closeTab(tab: RsTabId) {
  if (openTabs.value.length <= 1) return
  const index = openTabs.value.indexOf(tab)
  const next = openTabs.value.filter(id => id !== tab)
  openTabs.value = next
  if (activeTab.value === tab) {
    activeTab.value = next[Math.max(0, index - 1)] ?? next[0] as RsTabId
  }
  if (tab === 'subagents') subagentFilter.value = ''
  addMenuOpen.value = false
}

function reopenTab(tab: RsTabId) {
  ensureTabOpen(tab)
  activeTab.value = tab
  addMenuOpen.value = false
  if (tab === 'subagents' && props.focusSubagentKey) subagentFilter.value = props.focusSubagentKey
}

function selectTab(tab: RsTabId) {
  ensureTabOpen(tab)
  activeTab.value = tab
  if (tab === 'subagents') subagentFilter.value = ''
}

/** 工具按钮 / 外部 focus：打开（或聚焦）侧聊天轴。 */
function openSideChat() {
  ensureTabOpen('sideChat')
  activeTab.value = 'sideChat'
}

const visibleSubagents = computed(() => (
  subagentFilter.value
    ? (props.subagents ?? []).filter(item => item.key === subagentFilter.value)
    : (props.subagents ?? [])
))
const runningSubagentCount = computed(() => (
  (props.subagents ?? []).filter(item => item.tone === 'run').length
))

function shortSubagentKey(key: string): string {
  return key.length > 34 ? `${key.slice(0, 16)}…${key.slice(-12)}` : key
}

function subagentStatusLabel(tone: SubagentTone): string {
  if (tone === 'ok') return t('chat.subagentCard.statusDone')
  if (tone === 'err') return t('chat.subagentCard.statusFailed')
  if (tone === 'run') return t('chat.subagentCard.statusRunning')
  return t('chat.subagentCard.statusIdle')
}

function subagentTime(ts: string | number | null | undefined): string {
  return absoluteTime(ts)
}

function subagentIso(ts: string | number | null | undefined): string {
  return isoTime(ts)
}
// Persist width across mounts so navigating away and back keeps the chosen
// layout instead of resetting to the narrow column.
// 宽度改由 usePanelResize 统一管理（拖拽 / 键盘 / clamp / localStorage），
// 会话列在 ChatView 里读同一份宽度向左让位。
const {
  width: panelWidth,
  maxWidth: panelMaxWidth,
  resizing,
  startResize,
  onResizeKeydown,
  setWidth,
} = usePanelResize()
/** 「加宽」按钮现在只是把宽度设成 420px（再点回到默认 300px），不再走独立 class。 */
const wide = computed(() => panelWidth.value >= 400)
function toggleWide() {
  setWidth(wide.value ? RIGHT_PANEL_DEFAULT_WIDTH : 420)
}

// Tab labels are localized: the artefacts tab used to be hard-coded Chinese and
// its empty state called the same thing "文件", contradicting the tab wording.
const allTabs = computed(() => [
  { id: 'tasks' as const, label: t('chat.rightSidebar.tabs.tasks'), icon: 'listChecks' as IconName, count: props.tasks.filter(task => task.status === 'running').length },
  { id: 'subagents' as const, label: t('chat.rightSidebar.tabs.subagents'), icon: 'boxes' as IconName, count: (props.subagents ?? []).length },
  { id: 'artifacts' as const, label: t('chat.rightSidebar.tabs.artifacts'), icon: 'fileText' as IconName, count: props.artifacts.length },
  { id: 'changes' as const, label: t('chat.rightSidebar.tabs.changes'), icon: 'fork' as IconName, count: props.changes.length },
  { id: 'tree' as const, label: t('chat.rightSidebar.tabs.tree'), icon: 'listTree' as IconName, count: 0 },
  { id: 'sideChat' as const, label: t('chat.sideChat.title'), icon: 'chat' as IconName, count: 0 },
])
const tabs = computed(() => allTabs.value.filter(tab => openTabs.value.includes(tab.id)))
const closedTabs = computed(() => allTabs.value.filter(tab => !openTabs.value.includes(tab.id)))

// 文件树已抽到 `FileExplorerPanel`（IDE 式：懒加载展开、双击打开编辑器、
// 右键菜单走真实 RPC）。这里不再保留「进入目录 + 返回键」的浏览器式实现。

/** Stable key: prefer id, then download_url, then path+name. */
function artifactKey(a: RsArtifact): string {
  return String(a.id || a.download_url || `${a.path}|${a.name}`)
}

function artifactBasename(path?: string): string {
  const tail = String(path || '').split(/[\\/]/).pop() || 'file'
  return tail.split(/[?#]/)[0]
}

/**
 * Diagnostics arrive optional from the parent; normalise once so the template
 * never has to guard, and an absent prop reads as "nothing wrong" rather than
 * throwing on `.length`.
 */
const diagnosticList = computed(() => props.diagnostics ?? [])

/** Cache the classifier per artifact so the template's triple call stays cheap. */
const classifyCache = new WeakMap<object, ArtifactKindDescriptor>()
function classify(a: RsArtifact): ArtifactKindDescriptor {
  const cached = classifyCache.get(a)
  if (cached) return cached
  const name = a.name || artifactBasename(a.path || '')
  const result = classifyArtifact(name, a.mime || a.content_type)
  classifyCache.set(a, result)
  return result
}

function formatLabel(a: RsArtifact): string {
  const d = classify(a)
  if (d.format) return d.format
  return d.label
}

function changeIcon(c: RsChange): IconName {
  if (c.kind === 'created') return 'plus'
  if (c.kind === 'deleted') return 'trash'
  return 'edit'
}

const { pushToast } = useToasts()

type ArtifactStatus = 'ready' | 'updating' | 'stale'

// 用 Set 包裹父级传入的 key 集合，把每行状态查询降到 O(1)。
const liveKeysSet = computed(() => new Set(props.liveKeys ?? []))
const staleKeysSet = computed(() => new Set(props.staleKeys ?? []))

/** 派生单行的状态点语义：更新中 > 已过期 > 就绪。 */
function statusFor(a: RsArtifact): ArtifactStatus {
  if (props.streaming && liveKeysSet.value.has(artifactKey(a))) return 'updating'
  if (staleKeysSet.value.has(artifactKey(a))) return 'stale'
  return 'ready'
}

const STATUS_LABEL_KEYS: Record<ArtifactStatus, string> = {
  ready: 'chat.rightSidebar.statusReady',
  updating: 'chat.rightSidebar.statusUpdating',
  stale: 'chat.rightSidebar.statusStale',
}
function statusTitle(a: RsArtifact): string {
  return String(t(STATUS_LABEL_KEYS[statusFor(a)]))
}

// 「更多」菜单：同一时刻仅允许展开一行，点击外部或按 Escape 收起。
const openMenuKey = ref<string | null>(null)
function toggleMenu(a: RsArtifact) {
  const key = artifactKey(a)
  openMenuKey.value = openMenuKey.value === key ? null : key
}
function closeMenu() {
  openMenuKey.value = null
}
function onDocumentClick(e: MouseEvent) {
  // 点击落在某行菜单容器内时不关闭，交由各菜单项自行处理。
  const target = e.target as HTMLElement | null
  if (target && target.closest('.rs-artifact__menu-wrap')) return
  closeMenu()
}
function onDocumentKeydown(e: KeyboardEvent) {
  if (e.key === 'Escape') closeMenu()
}
onMounted(() => {
  document.addEventListener('click', onDocumentClick)
  document.addEventListener('keydown', onDocumentKeydown)
})
onUnmounted(() => {
  document.removeEventListener('click', onDocumentClick)
  document.removeEventListener('keydown', onDocumentKeydown)
})

function previewArtifact(account: RsArtifact) {
  emit('openArtifact', account)
  closeMenu()
}
function editArtifact(account: RsArtifact) {
  emit('openArtifact', account)
  closeMenu()
}
function openArtifactSystemAction(account: RsArtifact) {
  emit('openArtifactSystem', account)
  closeMenu()
}
function copyArtifactPath(account: RsArtifact) {
  const text = account.path || account.name || ''
  if (!text) {
    closeMenu()
    return
  }
  navigator.clipboard.writeText(text).then(
    () => pushToast(String(t('chat.rightSidebar.artifactCopied')), { tone: 'ok', duration: 1600 }),
    () => pushToast(String(t('chat.rightSidebar.artifactCopyFailed')), { tone: 'danger', duration: 2000 }),
  )
  closeMenu()
}
</script>

<style scoped>
.rs-toggle {
  position: fixed;
  /* 工作台展开时右移工作台宽度，避免两者重叠（实测右侧栏会盖住工作台标题栏） */
  right: var(--workbench-inset-right, 0);
  top: 50%;
  transform: translateY(-50%);
  z-index: 90;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 4px;
  width: 40px;
  height: 96px;
  border: 1px solid var(--border);
  border-right: none;
  border-radius: var(--radius-md) 0 0 var(--radius-md);
  background: var(--bg-elevated);
  color: var(--text-muted);
  cursor: pointer;
  transition: background var(--dur-fast, 120ms) var(--ease-standard), color var(--dur-fast, 120ms) var(--ease-standard);
}

.rs-toggle__label {
  font-size: 9.5px;
  line-height: 1.15;
  letter-spacing: 0.08em;
  writing-mode: vertical-rl;
}
.rs-toggle:hover {
  background: var(--bg-hover);
  color: var(--text);
}

.rs-panel {
  position: fixed;
  right: var(--workbench-inset-right, 0);
  top: 0;
  bottom: 0;
  /* 宽度由 usePanelResize 控制（clamp 280px..55vw + 持久化），不再写死 300px */
  width: var(--rs-panel-w, 300px);
  z-index: 95;
  display: flex;
  flex-direction: column;
  background: var(--bg-elevated);
  border-left: 1px solid var(--border);
  box-shadow: -4px 0 24px color-mix(in srgb, var(--bg) 50%, transparent);
}

/* 分割线：贴在面板左缘，6px 宽热区（视觉 1px 线 + 两侧对称的 hover 高亮）。
   拖动时禁止选中文本与继承 cursor。 */
.rs-resizer {
  position: absolute;
  left: 0;
  top: 0;
  bottom: 0;
  width: 6px;
  transform: translateX(-50%);
  z-index: 2;
  cursor: col-resize;
  touch-action: none;
  background: transparent;
  border-radius: var(--radius-pill);
  transition: background var(--dur-fast, 120ms) var(--ease-standard);
}
.rs-resizer::before {
  content: '';
  position: absolute;
  left: 50%;
  top: 0;
  bottom: 0;
  width: 1px;
  transform: translateX(-50%);
  background: var(--border);
}
.rs-resizer:hover,
.rs-resizer:focus-visible,
.rs-panel.is-resizing .rs-resizer {
  background: color-mix(in srgb, var(--accent) 28%, transparent);
  outline: none;
}
.rs-resizer:focus-visible { box-shadow: var(--focus-ring); }
/* 拖动中：面板与指针同帧跟手，别再加过渡 */
.rs-panel.is-resizing { user-select: none; }

.rs-tabs {
  display: flex;
  align-items: center;
  gap: 2px;
  padding: 8px 8px 0;
  border-bottom: 1px solid var(--border);
}
.rs-tabs__scroll {
  display: flex;
  align-items: center;
  gap: 2px;
  flex: 1;
  min-width: 0;
  overflow-x: auto;
  scrollbar-width: none;
}
.rs-tabs__scroll::-webkit-scrollbar { display: none; }
.rs-tabs__tools { display: flex; align-items: center; flex-shrink: 0; }
.rs-tabs__add { position: relative; display: flex; }
.rs-tabs__add-menu {
  position: absolute;
  right: 0;
  top: calc(100% + 4px);
  z-index: 6;
  display: flex;
  flex-direction: column;
  gap: 2px;
  min-width: 150px;
  padding: 4px;
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  background: var(--bg-elevated);
  box-shadow: var(--shadow-md);
}
.rs-tabs__add-menu button {
  display: flex;
  align-items: center;
  gap: 6px;
  width: 100%;
  padding: 6px 8px;
  border: 0;
  border-radius: var(--radius-sm);
  background: transparent;
  color: var(--text);
  font-size: var(--fs-xs);
  text-align: left;
  cursor: pointer;
}
.rs-tabs__add-menu button:hover { background: var(--bg-hover); }
.rs-tabs__add-empty {
  margin: 0;
  padding: 6px 8px;
  color: var(--text-dim);
  font-size: var(--fs-xs);
}
.rs-tab__close {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 16px;
  height: 16px;
  margin-left: 2px;
  padding: 0;
  border: 0;
  border-radius: var(--radius-xs);
  background: transparent;
  color: var(--text-dim);
  cursor: pointer;
}
.rs-tab__close:hover { background: var(--bg-hover); color: var(--text); }
.rs-tab:focus-visible { outline: none; box-shadow: var(--focus-ring); }
.rs-tab {
  display: flex;
  align-items: center;
  flex-shrink: 0;
  gap: 5px;
  padding: 6px 10px;
  border: none;
  border-radius: var(--radius-sm) var(--radius-sm) 0 0;
  background: transparent;
  color: var(--text-muted);
  font-size: 12px;
  cursor: pointer;
  white-space: nowrap;
  transition: background var(--dur-fast, 120ms), color var(--dur-fast, 120ms);
}
.rs-tab:hover { background: var(--bg-hover); color: var(--text); }
.rs-tab--active {
  color: var(--accent);
  border-bottom: 2px solid var(--accent);
}
.rs-tab__badge {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  min-width: 16px;
  height: 16px;
  padding: 0 4px;
  border-radius: var(--radius-md);
  background: var(--accent);
  color: var(--text);
  font-size: 10px;
  font-weight: 600;
}
.rs-tabs__spacer { flex: 1; }
.rs-icon-btn {
  display: flex; align-items: center; justify-content: center;
  width: 26px; height: 26px;
  border: none; border-radius: var(--radius-sm);
  background: transparent; color: var(--text-muted);
  cursor: pointer; transition: background var(--dur-fast, 120ms);
}
.rs-icon-btn:hover { background: var(--bg-hover); color: var(--text); }
.rs-icon-btn--active { color: var(--accent); background: var(--bg-hover); }

.rs-body { flex: 1; overflow-y: auto; padding: 8px; }
.rs-section { display: flex; flex-direction: column; gap: 6px; }
/* 项目树自己带滚动：容器给足高度，列表在内部滚动（不再整页跟着滚） */
.rs-section--explorer { height: 100%; min-height: 0; }
.rs-section--explorer :deep(.fx) { min-height: 320px; }
/* 侧聊天同样自己带滚动（消息区 + 输入框固定），容器只负责占满高度 */
.rs-section--sidechat { height: 100%; min-height: 0; }

.rs-empty {
  display: flex; flex-direction: column; align-items: center; justify-content: center;
  gap: 8px; padding: 40px 16px; color: var(--text-dim); font-size: 12px; text-align: center;
}
.rs-empty--artifacts { padding: 28px 16px; }
.rs-empty__lead { color: var(--text-muted); font-size: 12px; font-weight: 500; }
.rs-empty__sub { font-size: 11px; line-height: 1.5; max-width: 220px; }
.rs-empty__actions { display: flex; gap: 6px; margin-top: 4px; }
.rs-empty__hint {
  color: var(--text-muted);
  font-size: 11px;
  line-height: 1.5;
  max-width: 220px;
}
.rs-empty__btn {
  display: inline-flex; align-items: center; gap: 4px;
  padding: 6px 12px; border: 1px solid var(--accent);
  border-radius: var(--radius-md);
  background: var(--accent); color: var(--text);
  font-size: 12px; cursor: pointer;
  transition: background var(--dur-fast, 120ms) var(--ease-standard), transform var(--dur-fast, 120ms) var(--ease-standard);
}
.rs-empty__btn:hover { background: var(--accent-hover); }
.rs-empty__btn:active { transform: translateY(1px); }
.rs-empty__btn--ghost {
  background: transparent; color: var(--text-muted); border-color: var(--border);
}
.rs-empty__btn--ghost:hover { background: var(--bg-hover); color: var(--text); }

.rs-diagnostic {
  display: flex; align-items: center; gap: 6px;
  padding: 6px 10px; border-radius: var(--radius-md);
  background: color-mix(in srgb, var(--warn) 12%, transparent);
  border: 1px solid color-mix(in srgb, var(--warn) 40%, transparent);
  color: var(--text-muted); font-size: 11px;
}
.rs-diagnostic__retry {
  margin-left: auto; display: inline-flex; align-items: center; gap: 3px;
  padding: 2px 8px; border: 1px solid color-mix(in srgb, var(--warn) 50%, transparent);
  border-radius: var(--radius-sm); background: transparent; color: var(--text);
  font-size: 11px; cursor: pointer;
  transition: background var(--dur-fast, 120ms) var(--ease-standard);
}
.rs-diagnostic__retry:hover { background: color-mix(in srgb, var(--warn) 22%, transparent); }

/* Timeline */
.rs-timeline { list-style: none; margin: 0; padding: 0 0 0 16px; position: relative; }
.rs-timeline::before {
  content: ''; position: absolute; left: 5px; top: 4px; bottom: 4px;
  width: 1px; background: var(--border);
}
.rs-timeline__item { position: relative; padding: 6px 0 10px 14px; }
.rs-timeline__dot {
  position: absolute; left: -16px; top: 10px; width: 10px; height: 10px;
  border-radius: 50%; border: 2px solid var(--border);
  background: var(--bg-elevated);
}
.rs-timeline__item--running .rs-timeline__dot {
  border-color: var(--accent);
  background: var(--accent);
  animation: rs-pulse 1.2s ease-in-out infinite;
}
.rs-timeline__item--done .rs-timeline__dot { border-color: var(--ok); background: var(--ok); }
.rs-timeline__item--error .rs-timeline__dot { border-color: var(--danger); background: var(--danger); }
@keyframes rs-pulse { 0%,100% { opacity: 1; } 50% { opacity: 0.4; } }

.rs-timeline__title { font-size: 12px; font-weight: 500; color: var(--text); }
.rs-timeline__detail { font-size: 11px; color: var(--text-muted); margin-top: 2px; word-break: break-word; }
.rs-timeline__tools { display: flex; flex-wrap: wrap; gap: 4px; margin-top: 4px; }
.rs-tool-chip {
  display: inline-flex; align-items: center; gap: 3px;
  padding: 1px 6px; border-radius: var(--radius-xs);
  background: color-mix(in srgb, var(--accent) 12%, transparent); color: var(--accent);
  font-size: 10px; font-family: var(--font-mono, monospace);
}
.rs-timeline__time { font-size: 10px; color: var(--text-dim); }

/* ── 运行流（任务 tab） ───────────────────────────────────────── */
.rs-run-head {
  display: flex; align-items: center; gap: 8px;
  padding: 2px 4px 4px;
  font-size: 11px; color: var(--text-dim);
}
.rs-run-live { display: inline-flex; align-items: center; gap: 6px; font-weight: 600; }
.rs-run-live i {
  width: 6px; height: 6px; border-radius: var(--radius-pill);
  background: var(--text-dim);
}
.rs-run-live.is-on { color: var(--ok); }
.rs-run-live.is-on i {
  background: var(--ok);
  animation: rs-pulse var(--dur-pulse) var(--ease-standard) infinite;
}
.rs-run-count { margin-left: auto; font-variant-numeric: tabular-nums; opacity: 0.7; }

.rs-run-row {
  display: flex; align-items: center; gap: 6px;
  width: 100%; padding: 0; border: 0; background: transparent;
  font: inherit; text-align: left; cursor: pointer;
}
.rs-run-row.is-static { cursor: default; }
.rs-run-row:not(.is-static):hover .rs-run-row__text { color: var(--accent); }
.rs-run-row__icon { flex: 0 0 auto; color: var(--text-dim); }
.rs-run-row__text {
  min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
}
.rs-run-row__chevron {
  flex: 0 0 auto; margin-left: auto; color: var(--text-dim);
  transition: transform var(--dur-fast) var(--ease-standard);
}
.rs-run-row__chevron.is-open { transform: rotate(180deg); }

.rs-run-body { margin-top: 6px; display: flex; flex-direction: column; gap: 6px; }
.rs-run-block { display: flex; flex-direction: column; gap: 2px; }
.rs-run-block__label {
  font-size: 10px; font-weight: 700; letter-spacing: 0.04em;
  text-transform: uppercase; color: var(--text-dim);
}
.rs-run-pre {
  margin: 0; padding: 6px 8px;
  max-height: 220px; overflow: auto;
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  background: var(--bg-surface);
  color: var(--text);
  font-family: var(--font-mono, monospace);
  font-size: 11px; line-height: 1.5;
  white-space: pre-wrap; word-break: break-word;
  overscroll-behavior: contain;
}

/* Artifacts */
.rs-artifact-list { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 4px; }
.rs-artifact {
  display: flex; align-items: center; gap: 8px;
  padding: 8px 10px; border-radius: var(--radius-md);
  background: var(--card);
  border: 1px solid var(--border);
  transition: background var(--dur-fast, 120ms) var(--ease-standard), border-color var(--dur-fast, 120ms) var(--ease-standard);
}
.rs-artifact:hover { background: var(--bg-hover); border-color: var(--border-strong); }
.rs-artifact__type-dot {
  width: 6px; height: 6px; border-radius: 50%; flex-shrink: 0;
  background: var(--ok);
  transition: background var(--dur-fast, 120ms) var(--ease-standard);
}
.rs-artifact__type-dot--ready { background: var(--ok); }
.rs-artifact__type-dot--updating {
  background: var(--accent);
  animation: rs-pulse 1.2s ease-in-out infinite;
}
.rs-artifact__type-dot--stale { background: var(--warn); }
.rs-artifact__info { flex: 1; min-width: 0; }
.rs-artifact__name { font-size: 12px; color: var(--text); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
.rs-artifact__path { font-size: 10px; color: var(--text-dim); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
.rs-artifact__format {
  font-size: 10px; font-weight: 600; font-family: var(--font-mono, monospace);
  letter-spacing: 0.04em; flex-shrink: 0;
}
.rs-artifact__btn {
  display: flex; align-items: center; justify-content: center;
  width: 26px; height: 26px; border: none; border-radius: var(--radius-sm);
  background: transparent; color: var(--text-muted); cursor: pointer; flex-shrink: 0;
  transition: background var(--dur-fast, 120ms) var(--ease-standard), color var(--dur-fast, 120ms) var(--ease-standard);
}
.rs-artifact__btn:hover { background: var(--accent); color: var(--text); }
.rs-artifact__btn:focus-visible { outline: none; box-shadow: var(--focus-ring); }

.rs-artifact__menu-wrap { position: relative; flex-shrink: 0; }
.rs-artifact__menu {
  position: absolute; right: 0; top: calc(100% + 4px); z-index: 10;
  min-width: 150px; margin: 0; padding: 4px; list-style: none;
  background: var(--bg-elevated);
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  box-shadow: var(--shadow-lg);
}
.rs-artifact__menu button {
  display: flex; align-items: center; gap: 8px; width: 100%;
  padding: 7px 8px; border: none; border-radius: var(--radius-sm);
  background: transparent; color: var(--text); font-size: 12px; cursor: pointer;
  transition: background var(--dur-fast, 120ms) var(--ease-standard);
}
.rs-artifact__menu button:hover { background: var(--bg-hover); }
.rs-artifact__menu button:focus-visible { outline: none; box-shadow: var(--focus-ring); }

/* Changes */
.rs-change-list { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 2px; }
.rs-change {
  display: flex; align-items: center; gap: 6px;
  padding: 5px 8px; border-radius: var(--radius-sm); font-size: 12px;
}
.rs-change:hover { background: var(--bg-hover); }
.rs-change__path { flex: 1; color: var(--text); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; font-family: var(--font-mono, monospace); font-size: 11px; }
.rs-change__add { color: var(--ok); font-size: 11px; font-weight: 600; font-family: var(--font-mono, monospace); }
.rs-change__del { color: var(--danger); font-size: 11px; font-weight: 600; font-family: var(--font-mono, monospace); }

/* File tree */
.rs-tree-toolbar { display: flex; align-items: center; gap: 6px; padding: 4px 6px; }
.rs-tree-path { flex: 1; font-size: 11px; color: var(--text-muted); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; font-family: var(--font-mono, monospace); }
.rs-tree-list { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 1px; }
.rs-tree-row {
  display: flex; align-items: center; gap: 8px; width: 100%;
  padding: 5px 8px; border: none; border-radius: var(--radius-sm);
  background: transparent; color: var(--text); font-size: 12px; cursor: pointer; text-align: left;
  transition: background var(--dur-fast, 120ms) var(--ease-standard);
}
.rs-tree-row:hover { background: var(--bg-hover); }
.rs-tree-row--file { cursor: default; }
.rs-tree-name { flex: 1; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
.rs-icon-btn:disabled { opacity: 0.4; cursor: default; }

/* Subagents axis */
.rs-subagent-filter {
  display: inline-flex; align-items: center; gap: 4px; align-self: flex-start;
  margin: 2px 0 4px; padding: 3px 8px;
  border: 1px solid var(--border); border-radius: var(--radius-pill);
  background: var(--bg-surface); color: var(--text-muted);
  font-size: var(--fs-xs); cursor: pointer;
  transition: color var(--dur-fast) var(--ease-standard), border-color var(--dur-fast) var(--ease-standard);
}
.rs-subagent-filter:hover { color: var(--text); border-color: var(--border-strong); }
.rs-subagents { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: var(--sp-2); }
.rs-subagent {
  display: flex; gap: var(--sp-2); padding: var(--sp-2);
  border: 1px solid var(--border); border-radius: var(--radius-md);
  background: var(--bg-surface);
}
.rs-subagent__dot {
  flex-shrink: 0; width: 6px; height: 6px; margin-top: 5px;
  border-radius: var(--radius-pill); background: var(--text-dim);
}
.rs-subagent--ok .rs-subagent__dot { background: var(--ok); }
.rs-subagent--err .rs-subagent__dot { background: var(--danger); }
.rs-subagent--run .rs-subagent__dot { background: var(--accent); }
.rs-subagent__body { min-width: 0; flex: 1; display: flex; flex-direction: column; gap: 3px; }
.rs-subagent__head { display: flex; align-items: center; gap: var(--sp-2); min-width: 0; }
.rs-subagent__key {
  min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
  color: var(--text-muted); font-family: var(--font-mono, monospace); font-size: var(--fs-xs);
}
.rs-subagent__status { margin-left: auto; flex-shrink: 0; color: var(--text-dim); font-size: var(--fs-xs); }
.rs-subagent--ok .rs-subagent__status { color: var(--ok); }
.rs-subagent--err .rs-subagent__status { color: var(--danger); }
.rs-subagent--run .rs-subagent__status { color: var(--accent); }
.rs-subagent__summary {
  margin: 0; color: var(--text); font-size: var(--fs-xs); line-height: 1.5;
  white-space: pre-wrap; overflow: hidden;
  display: -webkit-box; -webkit-line-clamp: 4; -webkit-box-orient: vertical;
}
.rs-subagent__time { color: var(--text-dim); font-size: var(--fs-xs); }

.rs-slide-enter-active, .rs-slide-leave-active { transition: transform var(--dur-normal, 200ms) var(--ease-standard); }
.rs-slide-enter-from, .rs-slide-leave-to { transform: translateX(100%); }

@media (prefers-reduced-motion: reduce) {
  .rs-timeline__item--running .rs-timeline__dot { animation: none; }
  .rs-artifact__type-dot--updating { animation: none; }
  .rs-slide-enter-active, .rs-slide-leave-active { transition: none; }
}
</style>
