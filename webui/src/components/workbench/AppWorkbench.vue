<template>
  <WorkbenchHost
    :enabled="enabled"
    :route-active="routeActive"
    :modal-blocked="surfaceBlocked"
    :aria-label="t('workbench.title')"
    :empty-label="t('workbench.empty')"
    :open-items-label="t('workbench.openItems')"
    :collapse-label="t('workbench.collapse')"
    :close-item-label="t('workbench.closeItem')"
    :resize-label="t('workbench.resize')"
    :pixels-label="t('workbench.pixels')"
    @collapsed="restoreWorkbenchFocus"
    @emptied="restoreWorkbenchFocus"
    @surface-rect="onSurfaceRect"
  >
    <template #title="{ item }">
      <span class="app-workbench__identity">
        <Icon
          v-if="panelHeader(item).icon"
          :name="panelHeader(item).icon!"
          :size="18"
          aria-hidden="true"
        />
        <span class="app-workbench__identity-copy">
          <strong>{{ panelHeader(item).title }}</strong>
          <small v-if="panelHeader(item).subtitle">
            {{ panelHeader(item).subtitle }}
          </small>
        </span>
      </span>
    </template>

    <template #actions="{ item }">
      <!-- 新增面板：终端 / 内置浏览器 / 代码编辑器 -->
      <WorkbenchAddMenu
        @terminal="openTerminal()"
        @browser="openBrowserUrl"
        @editor="openEditor"
        @runtime="openRuntime()"
      />
      <button
        v-if="!item"
        type="button"
        class="btn btn--icon btn--ghost"
        :aria-label="t('workbench.terminal.open')"
        :title="t('workbench.terminal.open')"
        data-testid="open-terminal"
        @click="openTerminal()"
      >
        <Icon name="keyboard" :size="15" />
      </button>
      <select
        v-if="artifactNavigationItems(item).length > 1"
        class="app-workbench__switcher"
        :value="item?.id"
        :aria-label="t('chat.sessionDeliverables')"
        :title="t('chat.sessionDeliverables')"
        data-testid="workbench-artifact-switcher"
        @change="selectNavigationArtifact(item, $event)"
      >
        <option
          v-for="artifact in artifactNavigationItems(item)"
          :key="artifactNavigationId(item, artifact)"
          :value="artifactNavigationId(item, artifact)"
        >
          {{ artifactFileTitle(artifact) }}
        </option>
      </select>
      <template
        v-for="toolbarItem in panelToolbarItems(item)"
        :key="toolbarItem.id"
      >
        <span
          v-if="toolbarItem.kind === 'status'"
          class="app-workbench__warning"
          :title="toolbarItem.label"
          role="status"
        >
          <Icon
            v-if="toolbarItem.icon"
            :name="toolbarItem.icon"
            :size="14"
            aria-hidden="true"
          />
          <span>{{ toolbarItem.text }}</span>
        </span>
        <select
          v-else-if="toolbarItem.kind === 'select'"
          class="app-workbench__switcher app-workbench__mode-select"
          :value="toolbarItem.value"
          :aria-label="toolbarItem.label"
          :title="toolbarItem.label"
          @change="performPanelSelection(item, toolbarItem, $event)"
        >
          <option
            v-for="option in toolbarItem.options"
            :key="option.value"
            :value="option.value"
            :disabled="option.disabled"
          >
            {{ option.label }}
          </option>
          <optgroup
            v-if="toolbarItem.actionOptions?.length"
            :label="toolbarItem.actionGroupLabel"
          >
            <option
              v-for="option in toolbarItem.actionOptions"
              :key="option.value"
              :value="option.value"
              :disabled="option.disabled"
            >
              {{ option.label }}
            </option>
          </optgroup>
        </select>
        <button
          v-else
          type="button"
          class="app-workbench__action"
          :class="{ 'is-active': toolbarItem.pressed }"
          :aria-label="toolbarItem.label"
          :aria-pressed="toolbarItem.pressed"
          :disabled="toolbarItem.disabled"
          :title="toolbarItem.label"
          @click="performPanelAction(item, toolbarItem.id)"
        >
          <Icon :name="toolbarItem.icon" :size="15" aria-hidden="true" />
        </button>
      </template>
    </template>

    <template #panel="{ item, active }">
      <component
        :is="panelComponent(item)"
        v-if="panelComponent(item)"
        :ref="panelRefSetter(item)"
        v-bind="panelProps(item, active, false)"
        @workbench-event="handlePanelEvent(item, $event)"
      />
      <div v-else class="app-workbench__unsupported">
        {{ t('workbench.empty') }}
      </div>
    </template>

    <template #native-surface="{ item, active }">
      <component
        :is="panelComponent(item)"
        v-if="panelComponent(item)"
        :ref="panelRefSetter(item)"
        v-bind="panelProps(item, active, true)"
        @workbench-event="handlePanelEvent(item, $event)"
      />
    </template>
  </WorkbenchHost>

  <!-- 冷启动 / 收起态入口：还没有任何标签时 WorkbenchHost 不挂载（shouldMount
       要求有 activeItem），dock 被收起时标题栏也不可见。这两种情况下右边缘
       常驻悬浮条是唯一入口 —— 必须一直在，否则用户「打开一次再关掉」就再也
       找不到它（2026-10-24 实测缺陷）。 -->
  <WorkbenchAddMenu
    v-if="enabled && routeActive && (store.items.length === 0 || !store.expanded)"
    variant="rail"
    align="start"
    @terminal="openTerminal()"
    @browser="openBrowserUrl"
    @editor="openEditor"
    @runtime="openRuntime()"
  />
</template>

<script lang="ts">
// This SFC has a plain `<script>` block in addition to `<script setup>`; both
// compile into the same module scope, so shared imports are declared here once.
import { useAppStore } from '@/stores/app'
import { useWorkbenchStore } from '@/workbench/store'
import type { WorkbenchItem } from '@/workbench/types'

/**
 * Dock a registered view into the workbench as a keep-alive tab.
 *
 * Docking is additive: it opens an additional instance of the view rather than
 * navigating the main area, so the route-exclusion model is untouched and the
 * docked view survives navigating the main region elsewhere. Returns whether the
 * tab was opened (the store rejects only native-surface overflow, which `view`
 * panels never hit, so this is effectively always true).
 */
export function openViewPanel(opts: { view: string; title: string }): boolean {
  const item: WorkbenchItem = {
    id: `view:${opts.view}`,
    kind: 'view',
    title: opts.title,
    scope: { type: 'app' },
    hostKind: 'dom',
    retention: 'keep-alive',
    payload: { view: opts.view, title: opts.title },
  }
  return useWorkbenchStore().openItem(item)
}
</script>

<script setup lang="ts">
import {
  computed,
  nextTick,
  onBeforeUnmount,
  onMounted,
  watch,
} from 'vue'
import { useI18n } from 'vue-i18n'
import Icon from '@/components/Icon.vue'
import { useArtifactImageLightbox } from '@/composables/chat/useArtifactImageLightbox'
import { useConfirm } from '@/composables/useConfirm'
import { useNativeSurfaceOcclusionState } from '@/composables/useDialogA11y'
import { useToasts } from '@/composables/useToasts'
import { usePlatform } from '@/platform'
import type { NativeWorkbenchSurfaceEvent } from '@/platform/types'
import type { ArtifactPayload } from '@/types/rpc'
import { workbenchPanelRegistry } from '@/workbench/registry'
import {
  artifactWorkbenchItemId,
  createArtifactPreviewWorkbenchItem,
  navigationArtifactsFromWorkbenchItem,
  previewableNavigationArtifactsFromWorkbenchItem,
  sessionKeyFromWorkbenchItem,
} from '@/workbench/artifactItems'
import {
  BROWSER_WORKBENCH_OPEN_EVENT,
  createBrowserWorkbenchItem,
  normalizeBrowserUrl,
  type BrowserWorkbenchOpenEventDetail,
} from '@/workbench/browserItems'
import {
  EDITOR_WORKBENCH_OPEN_EVENT,
  createEditorWorkbenchItem,
  type EditorWorkbenchOpenEventDetail,
} from '@/workbench/editorItems'
import {
  artifactCategory,
  artifactFileTitle,
} from '@/utils/chat/artifacts'
import {
  readPreviewPreferences,
  savePreviewPreferences,
} from '@/utils/workbench/previewPreferences'
import {
  attachWorkbenchRuntime,
  WorkbenchRuntimeManager,
} from '@/workbench/runtime'
import type {
  NativeSurfaceRect,
  WorkbenchComponentEvent,
  WorkbenchPanelHeader,
  WorkbenchPanelRenderState,
  WorkbenchToolbarSelectOption,
  WorkbenchToolbarItem,
} from '@/workbench/types'
import { createArtifactWorkbenchDefinitions } from './artifactWorkbenchProvider'
import { createBrowserWorkbenchDefinition } from './browserWorkbenchProvider'
import { createEditorWorkbenchDefinition } from './editorWorkbenchProvider'
import { createTerminalWorkbenchDefinition } from './terminalWorkbenchProvider'
import { createGoalWorkbenchDefinition } from './goalWorkbenchProvider'
import { createTaskWorkbenchDefinition } from './taskWorkbenchProvider'
import { createIslandWorkbenchDefinition } from './islandWorkbenchProvider'
import { createViewWorkbenchDefinition } from './viewWorkbenchProvider'
import { createRuntimeWorkbenchDefinition } from './runtimeWorkbenchProvider'
import {
  TERMINAL_WORKBENCH_OPEN_EVENT,
  createTerminalWorkbenchItem,
  type TerminalWorkbenchOpenEventDetail,
} from '@/workbench/terminalItems'
import {
  TASK_WORKBENCH_OPEN_EVENT,
  openTaskWorkbenchItem,
  type TaskWorkbenchOpenEventDetail,
} from '@/workbench/taskItems'
import {
  ISLAND_WORKBENCH_OPEN_EVENT,
  openIslandWorkbenchItem,
  type IslandWorkbenchOpenEventDetail,
} from '@/workbench/islandItems'
import {
  RUNTIME_WORKBENCH_OPEN_EVENT,
  createRuntimeWorkbenchItem,
  type RuntimeWorkbenchOpenEventDetail,
} from '@/workbench/runtimeItems'
import WorkbenchHost from './WorkbenchHost.vue'
import WorkbenchAddMenu from './WorkbenchAddMenu.vue'

const props = withDefaults(defineProps<{
  enabled?: boolean
  modalBlocked?: boolean
  routeActive?: boolean
  sessionId?: string
}>(), {
  enabled: true,
  modalBlocked: false,
  routeActive: false,
  sessionId: '',
})

const { t } = useI18n()
const { confirm } = useConfirm()
const { pushToast } = useToasts()
const platform = usePlatform()
const store = useWorkbenchStore()
const appStore = useAppStore()

/* 面板菜单已抽到 `WorkbenchAddMenu`（标题栏与右边缘悬浮条共用一份实现）。 */

/** 其它入口（如产物卡片/命令）请求在工作台打开文件时走同一套建项逻辑。 */
function openEditor(path: string, scopeId?: string, projectPath?: string): void {
  const resolvedScope = (scopeId || store.activeSessionId || props.sessionId || 'app').trim()
  const item = createEditorWorkbenchItem({
    scopeId: resolvedScope,
    path,
    ...(projectPath ? { projectPath } : {}),
  })
  if (!item) {
    pushToast(t('workbench.panels.editorPathInvalid'), { tone: 'warn' })
    return
  }
  if (!store.openItem(item)) pushToast(t('workbench.itemLimitReached'), { tone: 'warn', duration: 6000 })
}

function onEditorWorkbenchOpen(event: Event): void {
  const detail = (event as CustomEvent<EditorWorkbenchOpenEventDetail>).detail
  if (detail && typeof detail.path === 'string') {
    openEditor(detail.path, detail.scopeId, detail.projectPath)
  }
}

/**
 * 禅模式：收掉工作台 dock 面板，退出时还原**进入前**的展开状态。
 *
 * 放在这里而不是 App.vue：dock 的展开状态归本组件消费的 workbench store 管，
 * "谁持有状态、谁负责收放"应在一处，避免 App.vue 跨两个 store 做编排。
 * 只读 `expanded` 而不用 `isOpen`：`isOpen` 未导出，且它额外要求 host 可用/有选中项，
 * 直接读 `expanded` 才能忠实反映"用户之前是不是把 dock 打开着"。
 */
let dockWasExpandedBeforeZen = false
watch(
  () => appStore.zenMode,
  (on) => {
    if (on) {
      dockWasExpandedBeforeZen = store.expanded
      if (store.expanded) store.setExpanded(false)
    } else if (dockWasExpandedBeforeZen) {
      store.setExpanded(true)
      dockWasExpandedBeforeZen = false
    }
  },
)
const artifactImageLightbox = useArtifactImageLightbox()
const nativeSurfaceOccluded = useNativeSurfaceOcclusionState()
const surfaceBlocked = computed(() => props.modalBlocked || nativeSurfaceOccluded.value)
const baseOrigin = typeof window === 'undefined' ? 'http://localhost' : window.location.origin
const nativeApi = platform.workbench.native
const runtimeManager = new WorkbenchRuntimeManager(workbenchPanelRegistry, {
  nativeWorkbenchApi: nativeApi,
  setExpanded: expanded => {
    store.setExpanded(expanded)
    if (!expanded) restoreWorkbenchFocus()
  },
})
let stopSurfaceEvents: (() => void) | null = null
let detachRuntime: (() => Promise<void>) | null = null

function readAuthToken(): string {
  if (typeof sessionStorage === 'undefined') return ''
  try {
    return sessionStorage.getItem('marguerite.wsToken') || ''
  } catch {
    return ''
  }
}

function confirmWorkbenchPermission(request: {
  permission: string
  requestingOrigin: string
}): Promise<boolean> {
  return confirm({
    title: t('workbench.artifactPreview.permissionTitle'),
    body: t('workbench.artifactPreview.permissionBody', {
      origin: request.requestingOrigin || t('workbench.artifactPreview.unknownOrigin'),
      permission: request.permission,
    }),
    primaryLabel: t('workbench.artifactPreview.permissionAllow'),
    primaryClass: 'btn--primary',
  })
}

/**
 * 在工作台打开内置浏览器。
 *
 * 2026-10-24：不再要求原生表面能力（壳里没有实现），统一走 CDP 镜像面板；
 * 作用域缺省用 `app`，让「没有会话」时也能开一个浏览器面板。
 */
function openBrowserUrl(value: string) {
  const url = normalizeBrowserUrl(value)
  if (!url) {
    pushToast(t('workbench.panels.browserUrlInvalid'), { tone: 'warn' })
    return
  }
  const sessionId = (store.activeSessionId || props.sessionId || 'app').trim()
  const item = createBrowserWorkbenchItem({ scopeId: sessionId, url })
  if (!item) {
    pushToast(t('workbench.panels.browserUrlInvalid'), { tone: 'warn' })
    return
  }
  if (!store.openItem(item)) {
    pushToast(t('workbench.itemLimitReached'), { tone: 'warn', duration: 6000 })
  }
}

function onBrowserWorkbenchOpen(event: Event) {
  const detail = (event as CustomEvent<BrowserWorkbenchOpenEventDetail>).detail
  if (detail && typeof detail.url === 'string') openBrowserUrl(detail.url)
}

function openTerminal(scopeId?: string): void {
  const item = createTerminalWorkbenchItem({
    scopeId: scopeId || store.activeSessionId || props.sessionId || 'app',
  })
  if (!item) {
    pushToast(t('workbench.terminal.failedOpen'), { tone: 'warn' })
    return
  }
  if (!store.openItem(item)) {
    pushToast(t('workbench.itemLimitReached'), { tone: 'warn', duration: 6000 })
  }
}

function onTerminalWorkbenchOpen(event: Event) {
  const detail = (event as CustomEvent<TerminalWorkbenchOpenEventDetail>).detail
  openTerminal(detail?.scopeId)
}

/**
 * Open the task-progress panel (Gap4). Wired to a real caller — the
 * {@link TASK_WORKBENCH_OPEN_EVENT} window event dispatched by
 * `requestTaskWorkbenchOpen` (CommandPalette "Task progress" action) — so
 * `createTaskWorkbenchItem` is no longer dead code.
 */
function openTask(scopeId?: string): void {
  const resolved = scopeId || store.activeSessionId || props.sessionId || undefined
  if (!openTaskWorkbenchItem(store, resolved)) {
    pushToast(t('workbench.itemLimitReached'), { tone: 'warn', duration: 6000 })
  }
}

function onTaskWorkbenchOpen(event: Event) {
  const detail = (event as CustomEvent<TaskWorkbenchOpenEventDetail>).detail
  openTask(detail?.scopeId)
}

/**
 * 岛屿面板入口（由 `requestIslandWorkbenchOpen` 派发，例如 /island 页头的「在工作台打开」）：
 * 与任务面板同一套 scope 解析口径 —— 显式 scope → 当前活跃会话 → 宿主传入的会话。
 */
function openIsland(scopeId?: string): void {
  const resolved = scopeId || store.activeSessionId || props.sessionId || undefined
  if (!openIslandWorkbenchItem(store, resolved)) {
    pushToast(t('workbench.itemLimitReached'), { tone: 'warn', duration: 6000 })
  }
}

function onIslandWorkbenchOpen(event: Event) {
  const detail = (event as CustomEvent<IslandWorkbenchOpenEventDetail>).detail
  openIsland(detail?.scopeId)
}

/**
 * 「运行态」面板：Harness 侧的只读观测面（决策 / 执行环境 / 上下文 / 工具面 / 质量门）。
 * scope 解析口径与终端一致：显式 → 当前活跃会话 → 宿主会话；都没有时用 `app`
 * （未绑定会话时也能看本机环境）。
 */
function openRuntime(scopeId?: string): void {
  const resolved = (scopeId || store.activeSessionId || props.sessionId || 'app').trim()
  const item = createRuntimeWorkbenchItem({
    scopeId: resolved,
    title: t('workbench.panels.runtime'),
  })
  if (!item) {
    pushToast(t('workbench.itemLimitReached'), { tone: 'warn' })
    return
  }
  if (!store.openItem(item)) pushToast(t('workbench.itemLimitReached'), { tone: 'warn', duration: 6000 })
}

function onRuntimeWorkbenchOpen(event: Event) {
  const detail = (event as CustomEvent<RuntimeWorkbenchOpenEventDetail>).detail
  openRuntime(detail?.scopeId)
}

for (const definition of createArtifactWorkbenchDefinitions({
  authToken: readAuthToken,
  baseOrigin,
  confirmPermission: confirmWorkbenchPermission,
  confirmRemoteResources: () => confirm({
    title: t('workbench.artifactPreview.remoteResourcesConfirmTitle'),
    body: t('workbench.artifactPreview.remoteResourcesConfirmBody'),
    primaryLabel: t('workbench.artifactPreview.remoteResourcesConfirmAction'),
    primaryClass: 'btn--primary',
  }),
  currentSessionId: () => store.activeSessionId || props.sessionId,
  getPreviewPreferences: () => readPreviewPreferences(platform),
  openArtifact: (artifact, sessionKey, navigationArtifacts) => {
    if (artifactCategory(artifact) === 'visual') {
      artifactImageLightbox.open({
        artifact,
        navigationArtifacts,
        sessionKey,
      })
      return
    }
    const opened = store.openItem(createArtifactPreviewWorkbenchItem({
      artifact,
      navigationArtifacts,
      nativeHtml: Boolean(
        platform.capabilities.hasNativeWorkbenchSurfaces
        && platform.workbench.native,
      ),
      sessionKey,
    }))
    if (!opened) {
      pushToast(t('workbench.itemLimitReached'), { tone: 'warn', duration: 6000 })
    }
  },
  platform,
  previewLeasesEnabled: true,
  pushToast: (message, options) => pushToast(message, options),
  savePreviewPreferences: preferences => savePreviewPreferences(platform, preferences),
  showFullPreviewNotice: () => pushToast(
    t('workbench.artifactPreview.fullModeNotice'),
    { tone: 'info', duration: 9000 },
  ),
  t: (key, params) => String(t(key, params || {})),
})) {
  workbenchPanelRegistry.register(definition, { replace: true })
}
workbenchPanelRegistry.register(createBrowserWorkbenchDefinition({
  t: (key, params) => String(t(key, params || {})),
}), { replace: true })
// 内置编辑器（kind 'file'）：IDE 式单文件标签，读/写走 workspace.file.*
workbenchPanelRegistry.register(createEditorWorkbenchDefinition(), { replace: true })
workbenchPanelRegistry.register(createTerminalWorkbenchDefinition(), { replace: true })
workbenchPanelRegistry.register(createGoalWorkbenchDefinition(), { replace: true })
workbenchPanelRegistry.register(createTaskWorkbenchDefinition(), { replace: true })
workbenchPanelRegistry.register(createIslandWorkbenchDefinition(), { replace: true })
workbenchPanelRegistry.register(createRuntimeWorkbenchDefinition({
  subtitle: t('workbench.runtime.subtitle'),
}), { replace: true })
workbenchPanelRegistry.register(createViewWorkbenchDefinition(), { replace: true })
detachRuntime = attachWorkbenchRuntime(store, runtimeManager)

function panelComponent(item: WorkbenchItem) {
  return workbenchPanelRegistry.resolve(item)?.component || null
}

function panelRenderState(
  item: WorkbenchItem,
  active: boolean,
  nativeSurface: boolean,
): WorkbenchPanelRenderState {
  return {
    active,
    hostAvailable: store.hostAvailable,
    nativeSurface,
    runtimeState: runtimeManager.getRenderState(item.id),
  }
}

function panelProps(
  item: WorkbenchItem,
  active: boolean,
  nativeSurface: boolean,
): Readonly<Record<string, unknown>> {
  return workbenchPanelRegistry.resolve(item)?.getProps?.(
    item,
    panelRenderState(item, active, nativeSurface),
  ) || {}
}

function artifactNavigationItems(
  item: WorkbenchItem | null,
): readonly ArtifactPayload[] {
  return previewableNavigationArtifactsFromWorkbenchItem(item)
}

function artifactNavigationId(
  item: WorkbenchItem | null,
  artifact: ArtifactPayload,
): string {
  return artifactWorkbenchItemId(sessionKeyFromWorkbenchItem(item), artifact)
}

function selectNavigationArtifact(
  item: WorkbenchItem | null,
  event: Event,
) {
  if (!item) return
  const select = event.currentTarget as HTMLSelectElement
  const artifact = artifactNavigationItems(item).find(
    candidate => artifactNavigationId(item, candidate) === select.value,
  )
  if (!artifact || select.value === item.id) return
  const navigationArtifacts = navigationArtifactsFromWorkbenchItem(item)
  const sessionKey = sessionKeyFromWorkbenchItem(item)
  const opened = store.openItem(createArtifactPreviewWorkbenchItem({
    artifact,
    navigationArtifacts,
    nativeHtml: Boolean(
      platform.capabilities.hasNativeWorkbenchSurfaces
      && platform.workbench.native,
    ),
    sessionKey,
  }))
  if (!opened) {
    pushToast(t('workbench.itemLimitReached'), { tone: 'warn', duration: 6000 })
  }
}

function panelHeader(item: WorkbenchItem | null): WorkbenchPanelHeader {
  if (!item) return { title: '' }
  return workbenchPanelRegistry.resolve(item)?.getHeader?.(
    item,
    panelRenderState(
      item,
      item.id === store.activeItemId,
      item.hostKind === 'native-webcontents',
    ),
  ) || { title: item.title }
}

function panelToolbarItems(
  item: WorkbenchItem | null,
): readonly WorkbenchToolbarItem[] {
  if (!item) return []
  return workbenchPanelRegistry.resolve(item)?.getToolbarItems?.(
    item,
    panelRenderState(
      item,
      item.id === store.activeItemId,
      item.hostKind === 'native-webcontents',
    ),
  ) || []
}

function setPanelHandle(item: WorkbenchItem, value: unknown) {
  runtimeManager.setComponentHandle(item, value)
}

function panelRefSetter(item: WorkbenchItem): (value: unknown) => void {
  return value => setPanelHandle(item, value)
}

function handlePanelEvent(item: WorkbenchItem, event: WorkbenchComponentEvent) {
  if (event.type === 'request-collapse') {
    store.setExpanded(false)
    restoreWorkbenchFocus()
    return
  }
  runtimeManager.handleComponentEvent(item, event)
}

function performPanelAction(item: WorkbenchItem | null, actionId: string) {
  if (item) runtimeManager.performAction(item, actionId)
}

function performPanelSelection(
  item: WorkbenchItem | null,
  toolbarItem: Extract<WorkbenchToolbarItem, { kind: 'select' }>,
  event: Event,
) {
  const select = event.currentTarget as HTMLSelectElement
  const selectedValue = select.value
  const selected = [
    ...toolbarItem.options,
    ...(toolbarItem.actionOptions || []),
  ].find((option: WorkbenchToolbarSelectOption) => option.value === selectedValue)

  // Keep the control on the effective mode while the async lease replacement
  // runs. The render state selects the new value on success.
  select.value = toolbarItem.value
  if (!item || !selected || selectedValue === toolbarItem.value) return
  runtimeManager.performAction(item, selected.actionId)
}

function restoreWorkbenchFocus() {
  void nextTick(() => {
    const candidates = document.querySelectorAll<HTMLElement>([
      '[data-testid="chat-session-action-deliverables"]',
      '[data-testid="chat-header-primary-action"][data-action="deliverables"]',
      '.chat-textarea',
    ].join(','))
    const target = [...candidates].find(element => element.getClientRects().length > 0)
    target?.focus({ preventScroll: true })
  })
}

function onSurfaceRect(rect: NativeSurfaceRect) {
  runtimeManager.handleSurfaceRect(rect)
}

function onNativeSurfaceEvent(event: NativeWorkbenchSurfaceEvent) {
  runtimeManager.handleNativeSurfaceEvent(event)
}

watch(
  () => [props.routeActive, props.sessionId] as const,
  ([routeActive, sessionId]) => {
    if (routeActive) store.setSessionScope(sessionId || null)
  },
  { immediate: true },
)

watch(
  () => props.enabled,
  enabled => {
    if (!enabled) store.reset()
  },
)

onMounted(() => {
  if (nativeApi) stopSurfaceEvents = nativeApi.onSurfaceEvent(onNativeSurfaceEvent)
  window.addEventListener(BROWSER_WORKBENCH_OPEN_EVENT, onBrowserWorkbenchOpen)
  window.addEventListener(EDITOR_WORKBENCH_OPEN_EVENT, onEditorWorkbenchOpen)
  window.addEventListener(TERMINAL_WORKBENCH_OPEN_EVENT, onTerminalWorkbenchOpen)
  window.addEventListener(TASK_WORKBENCH_OPEN_EVENT, onTaskWorkbenchOpen)
  window.addEventListener(ISLAND_WORKBENCH_OPEN_EVENT, onIslandWorkbenchOpen)
  window.addEventListener(RUNTIME_WORKBENCH_OPEN_EVENT, onRuntimeWorkbenchOpen)
})

onBeforeUnmount(() => {
  window.removeEventListener(BROWSER_WORKBENCH_OPEN_EVENT, onBrowserWorkbenchOpen)
  window.removeEventListener(EDITOR_WORKBENCH_OPEN_EVENT, onEditorWorkbenchOpen)
  window.removeEventListener(TERMINAL_WORKBENCH_OPEN_EVENT, onTerminalWorkbenchOpen)
  window.removeEventListener(TASK_WORKBENCH_OPEN_EVENT, onTaskWorkbenchOpen)
  window.removeEventListener(ISLAND_WORKBENCH_OPEN_EVENT, onIslandWorkbenchOpen)
  window.removeEventListener(RUNTIME_WORKBENCH_OPEN_EVENT, onRuntimeWorkbenchOpen)
  stopSurfaceEvents?.()
  stopSurfaceEvents = null
  if (detachRuntime) void detachRuntime()
  detachRuntime = null
})
</script>

<style scoped>
.app-workbench__identity {
  display: flex;
  min-width: 0;
  align-items: center;
  gap: var(--sp-2);
  color: var(--text-dim);
}

.app-workbench__identity-copy {
  display: grid;
  min-width: 0;
  gap: 1px;
}

.app-workbench__identity-copy strong,
.app-workbench__identity-copy small,
.app-workbench__collection-title {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.app-workbench__identity-copy strong,
.app-workbench__collection-title {
  color: var(--text);
  font-size: var(--fs-sm);
  font-weight: 600;
}

.app-workbench__identity-copy small {
  color: var(--text-dim);
  font-size: var(--fs-xs);
  font-weight: 400;
}

.app-workbench__action {
  display: inline-flex;
  width: 30px;
  height: 30px;
  align-items: center;
  justify-content: center;
  padding: 0;
  border: 0;
  border-radius: var(--radius-md);
  background: transparent;
  color: var(--text-dim);
  cursor: pointer;
}

.app-workbench__switcher {
  width: min(150px, 24vw);
  min-width: 84px;
  height: 30px;
  padding: 0 24px 0 var(--sp-2);
  border: 1px solid transparent;
  border-radius: var(--radius-md);
  background-color: transparent;
  color: var(--text-dim);
  cursor: pointer;
  font: inherit;
  font-size: var(--fs-xs);
  text-overflow: ellipsis;
}

.app-workbench__switcher:hover {
  border-color: var(--border);
  background-color: var(--bg-hover);
  color: var(--text);
}

.app-workbench__switcher:focus-visible {
  border-color: var(--accent);
  outline: 2px solid var(--accent);
  outline-offset: -2px;
}

.app-workbench__mode-select {
  width: min(174px, 30vw);
  border-color: var(--border);
  background-color: var(--bg-surface);
  color: var(--text);
  font-weight: 600;
}

.app-workbench__action:hover {
  background: var(--bg-hover);
  color: var(--text);
}

.app-workbench__action.is-active {
  background: color-mix(in srgb, var(--accent) 12%, transparent);
  color: var(--accent);
}

.app-workbench__action:focus-visible {
  outline: 2px solid var(--accent);
  outline-offset: -2px;
}

.app-workbench__warning {
  display: inline-flex;
  min-width: 0;
  align-items: center;
  gap: var(--sp-1);
  color: var(--warn);
  font-size: var(--fs-xs);
}

.app-workbench__warning span {
  overflow: hidden;
  max-width: 150px;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.app-workbench__unsupported {
  display: grid;
  min-height: 100%;
  place-items: center;
  padding: var(--sp-5);
  color: var(--text-dim);
  font-size: var(--fs-sm);
}

@media (max-width: 600px) {
  .app-workbench__switcher {
    width: min(128px, 35vw);
  }

  .app-workbench__warning span {
    display: none;
  }
}
</style>
