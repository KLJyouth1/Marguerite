<template>
  <Teleport to="body">
    <div
      v-if="open && enabled && (multi || (phase !== 'closed' && phase !== 'native-picking'))"
      class="modal-overlay"
      @click="closeDialog"
    >
      <section
        ref="dialogRef"
        class="modal project-picker"
        role="dialog"
        aria-modal="true"
        :aria-label="dialogTitle"
        @click.stop
      >
        <header class="project-picker__header">
          <h3>{{ dialogTitle }}</h3>
          <button
            class="btn btn--icon btn--ghost"
            :aria-label="t('common.close')"
            @click="closeDialog"
          >
            <Icon name="x" :size="15" />
          </button>
        </header>

        <template v-if="multi">
          <p class="project-picker__scope">{{ t('chat.composer.projectRoots.hint') }}</p>
          <div
            class="project-picker__entries project-picker__roots"
            role="group"
            :aria-label="dialogTitle"
          >
            <button
              v-for="workspace in workspaces"
              :key="workspace.id"
              type="button"
              class="project-picker__root"
              :class="{
                'is-selected': isRootSelected(workspace.id),
                'is-primary': workspace.id === primaryId,
              }"
              :disabled="workspace.id === primaryId"
              :aria-pressed="workspace.id === primaryId || isRootSelected(workspace.id) ? 'true' : 'false'"
              :title="workspace.path"
              @click="toggleRoot(workspace.id)"
            >
              <span class="project-picker__root-mark" aria-hidden="true">
                <Icon
                  :name="workspace.id === primaryId || isRootSelected(workspace.id) ? 'check' : 'plus'"
                  :size="14"
                />
              </span>
              <span class="project-picker__root-copy">
                <span class="project-picker__root-name">{{ workspace.name }}</span>
                <span class="project-picker__root-path">{{ workspace.path }}</span>
              </span>
              <span
                v-if="workspace.id === primaryId"
                class="project-picker__root-badge"
              >{{ t('chat.composer.projectRoots.primary') }}</span>
              <span
                v-else-if="!workspace.available"
                class="project-picker__root-badge project-picker__root-badge--warn"
              >{{ t('workspaces.unavailable') }}</span>
            </button>
            <p v-if="workspaces.length === 0" class="project-picker__scope">
              {{ t('chat.composer.projectRoots.empty') }}
            </p>
          </div>
          <footer class="project-picker__footer">
            <button class="btn btn--ghost" @click="closeDialog">
              {{ t('common.cancel') }}
            </button>
            <button
              class="btn btn--primary project-picker__choose"
              @click="applySelection"
            >
              <Icon name="check" :size="15" />
              {{ t('chat.composer.projectRoots.apply') }}
            </button>
          </footer>
        </template>

        <template v-else-if="phase === 'desktop-error'">
          <p class="project-picker__error" role="alert">{{ error }}</p>
          <footer class="project-picker__footer">
            <button class="btn btn--ghost" @click="closeDialog">
              {{ t('common.cancel') }}
            </button>
            <button class="btn btn--primary" @click="retryNativePicker">
              {{ t('workspaces.retryDirectoryPicker') }}
            </button>
          </footer>
        </template>

        <template v-else-if="phase === 'recent-loading'">
          <p class="project-picker__scope" role="status">{{ t('workspaces.recentLoading') }}</p>
          <footer class="project-picker__footer">
            <button class="btn btn--ghost" @click="closeDialog">
              {{ t('common.cancel') }}
            </button>
            <!-- 列表 RPC 挂起时仍要有出口：直接去选目录，而不是只能关窗。 -->
            <button
              class="btn btn--ghost project-picker__action"
              :disabled="systemPickerBusy"
              @click="browseFoldersWhileLoading"
            >
              <Icon name="folder" :size="14" />
              {{ t('workspaces.browseFolders') }}
            </button>
          </footer>
        </template>

        <template v-else-if="phase === 'recent-ready'">
          <p class="project-picker__scope">{{ t('workspaces.recentTitle') }}</p>
          <div
            ref="recentListRef"
            class="project-picker__entries project-picker__roots"
            role="listbox"
            tabindex="0"
            :aria-label="t('workspaces.recentTitle')"
            @keydown.enter.prevent="chooseRecent()"
          >
            <button
              v-for="workspace in recentWorkspaces"
              :key="workspace.id"
              type="button"
              role="option"
              class="project-picker__root"
              :class="{ 'is-selected': workspace.path === recentSelectedPath }"
              :aria-selected="workspace.path === recentSelectedPath"
              :title="workspace.path"
              @click="selectRecent(workspace.path)"
              @dblclick="chooseRecent(workspace.path)"
              @keydown.enter.prevent.stop="chooseRecent(workspace.path)"
              @keydown.space.prevent.stop="selectRecent(workspace.path)"
            >
              <span class="project-picker__root-mark" aria-hidden="true">
                <Icon
                  :name="workspace.path === recentSelectedPath ? 'check' : 'folder'"
                  :size="14"
                />
              </span>
              <span class="project-picker__root-copy">
                <span class="project-picker__root-name">{{ workspace.name }}</span>
                <span class="project-picker__root-path">{{ workspace.path }}</span>
              </span>
              <span
                v-if="workspace.pinned"
                class="project-picker__root-badge"
                :title="t('workspaces.pin')"
                :aria-label="t('workspaces.pin')"
              >
                <Icon name="skills" :size="11" />
              </span>
              <span
                v-else-if="!workspace.available"
                class="project-picker__root-badge project-picker__root-badge--warn"
              >{{ t('workspaces.unavailable') }}</span>
            </button>
          </div>
          <p v-if="error" class="project-picker__error" role="alert">{{ error }}</p>
          <footer class="project-picker__footer">
            <button class="btn btn--ghost" @click="closeDialog">
              {{ t('common.cancel') }}
            </button>
            <button
              class="btn btn--ghost project-picker__action"
              :disabled="systemPickerBusy"
              @click="browseFolders"
            >
              <Icon name="folder" :size="14" />
              {{ t('workspaces.browseFolders') }}
            </button>
            <button
              class="btn btn--primary project-picker__choose"
              :disabled="!recentSelectedPath"
              @click="chooseRecent()"
            >
              <Icon name="check" :size="15" />
              {{ t('workspaces.chooseSelectedDirectory') }}
            </button>
          </footer>
        </template>

        <template v-else>
          <p class="project-picker__scope">{{ t('workspaces.webPickerScope') }}</p>
          <div class="project-picker__path">
            <button
              type="button"
              class="btn btn--ghost project-picker__action project-picker__parent"
              :disabled="!parentDirectory"
              @click="browse(parentDirectory || undefined)"
            >
              <Icon name="arrowUp" :size="14" />
              {{ t('workspaces.parentDirectory') }}
            </button>
            <input
              ref="pathInputRef"
              v-model="locationDraft"
              type="text"
              :aria-label="t('workspaces.projectPath')"
              :placeholder="t('workspaces.pathPlaceholder')"
              @keydown.enter.prevent="browse(locationDraft)"
            />
            <button
              v-if="systemPickerAvailable"
              type="button"
              class="btn btn--ghost project-picker__action project-picker__browse"
              :disabled="systemPickerBusy"
              @click="openSystemPicker"
            >
              <Icon name="folder" :size="14" />
              {{ t('workspaces.browse') }}
            </button>
          </div>
          <p v-if="error" class="project-picker__error" role="alert">{{ error }}</p>
          <div class="project-picker__browser">
            <div class="project-picker__browser-toolbar">
              <div class="project-picker__create">
                <button
                  v-if="!creatingDirectory"
                  type="button"
                  class="btn btn--ghost project-picker__action project-picker__create-trigger"
                  :disabled="webLoading"
                  @click="beginCreateDirectory"
                >
                  <Icon name="plus" :size="14" />
                  {{ t('workspaces.newDirectory') }}
                </button>
                <form
                  v-else
                  class="project-picker__create-form"
                  @submit.prevent="createDirectory"
                >
                  <input
                    ref="newDirectoryInputRef"
                    v-model="newDirectoryName"
                    type="text"
                    :aria-label="t('workspaces.newDirectoryName')"
                    :placeholder="t('workspaces.newDirectoryName')"
                    autocomplete="off"
                    @keydown.esc.prevent="cancelCreateDirectory"
                  />
                  <button
                    type="submit"
                    class="btn btn--primary"
                    :disabled="creatingDirectoryBusy || !newDirectoryName.trim()"
                  >
                    {{ t('workspaces.createDirectory') }}
                  </button>
                  <button
                    type="button"
                    class="btn btn--ghost"
                    :disabled="creatingDirectoryBusy"
                    @click="cancelCreateDirectory"
                  >
                    {{ t('common.cancel') }}
                  </button>
                </form>
              </div>
            </div>
            <div
              class="project-picker__entries"
              role="listbox"
              :aria-busy="webLoading"
            >
              <button
                v-for="entry in directories"
                :key="entry.path"
                type="button"
                role="option"
                :aria-selected="selectedDirectory === entry.path"
                class="project-picker__entry"
                :class="{ 'is-selected': selectedDirectory === entry.path }"
                @click="selectDirectory(entry.path)"
                @dblclick="browse(entry.path)"
                @keydown.enter.prevent.stop="browse(entry.path)"
                @keydown.space.prevent.stop="selectDirectory(entry.path)"
              >
                <Icon name="folder" :size="15" />
                <span>{{ entry.name }}</span>
              </button>
            </div>
          </div>
          <footer class="project-picker__footer">
            <button class="btn btn--ghost" @click="closeDialog">
              {{ t('common.cancel') }}
            </button>
            <!-- 最近列表读取失败后回落到浏览器：保留一条回到最近列表的重试入口。 -->
            <button
              v-if="recentFailed && !startInBrowser"
              class="btn btn--ghost project-picker__action"
              @click="retryRecentList"
            >
              <Icon name="refresh" :size="14" />
              {{ t('workspaces.retryRecentList') }}
            </button>
            <button
              class="btn btn--primary project-picker__choose"
              :disabled="!canChoose"
              @click="choose"
            >
              <Icon name="check" :size="15" />
              {{ t('workspaces.chooseSelectedDirectory') }}
            </button>
          </footer>
        </template>
      </section>
    </div>
  </Teleport>
</template>

<script setup lang="ts">
import { computed, nextTick, onScopeDispose, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import Icon from '@/components/Icon.vue'
import { useDialogA11y } from '@/composables/useDialogA11y'
import {
  normalizeWorkspace,
  sortWorkspacesByRecent,
  type NormalizedProjectWorkspace,
} from '@/composables/useProjectWorkspaces'
import { getPlatform } from '@/platform'
import { useRpcStore } from '@/stores/rpc'
import type {
  ProjectWorkspaceItem,
  ProjectWorkspacesResponse,
  SandboxPathEntry,
  SandboxPathListResponse,
} from '@/types/rpc'

type PickerPhase =
  | 'closed'
  | 'native-picking'
  | 'desktop-error'
  | 'recent-loading'
  | 'recent-ready'
  | 'web-loading'
  | 'web-ready'
  | 'web-error'

const props = withDefaults(defineProps<{
  open: boolean
  sessionKey?: string
  initialPath?: string
  enabled?: boolean
  /** 多选模式：列出已注册项目供勾选（不浏览文件系统）。 */
  multi?: boolean
  /** 多选模式下的已选附加项目 id（主根由 `primaryId` 单独给出）。 */
  selectedIds?: string[]
  /** 多选模式下的主项目 id：置灰且不可取消。 */
  primaryId?: string
  /** 多选模式的项目清单（来自 `workspaces.list`）。 */
  workspaces?: ProjectWorkspaceItem[]
  /** 创建项目时传 true：直接进入目录浏览器，不展示「最近打开」（旧目录不是新项目源）。 */
  startInBrowser?: boolean
}>(), {
  enabled: true,
  sessionKey: '',
  multi: false,
  selectedIds: () => [],
  primaryId: '',
  workspaces: () => [],
  startInBrowser: false,
})
const emit = defineEmits<{
  close: []
  choose: [path: string]
  /** 多选模式确认：完整的附加项目 id 集合（不含主根）。 */
  apply: [ids: string[]]
}>()
const { t } = useI18n()
const rpc = useRpcStore()
const dialogRef = ref<HTMLElement | null>(null)
const pathInputRef = ref<HTMLInputElement | null>(null)
const recentListRef = ref<HTMLDivElement | null>(null)
const newDirectoryInputRef = ref<HTMLInputElement | null>(null)
const phase = ref<PickerPhase>('closed')
const currentDirectory = ref('')
const selectedDirectory = ref('')
const locationDraft = ref('')
const parentDirectory = ref<string | null>(null)
const entries = ref<SandboxPathEntry[]>([])
const error = ref('')
const creatingDirectory = ref(false)
const creatingDirectoryBusy = ref(false)
const systemPickerBusy = ref(false)
const systemPickerAvailable = ref(false)
const newDirectoryName = ref('')
const multiSelection = ref<string[]>([])
const recentWorkspaces = ref<NormalizedProjectWorkspace[]>([])
const recentSelectedPath = ref('')
const recentFailed = ref(false)
let openEpoch = 0
let browseSequence = 0

const dialogTitle = computed(() => (
  props.multi ? t('chat.composer.projectRoots.dialogTitle') : t('workspaces.chooseProject')
))

const webLoading = computed(() => phase.value === 'web-loading')
// 帮助函数：避免 TS 在 `await` 之后仍把 `phase.value` 收窄成赋值时的字面量。
function phaseIs(value: PickerPhase): boolean {
  return phase.value === value
}
const directories = computed(() =>
  entries.value.filter(entry => entry.kind === 'directory' && entry.selectable),
)
const canChoose = computed(() => {
  if (webLoading.value) return false
  const selected = selectedDirectory.value.trim()
  if (!selected) return false
  if (selected === currentDirectory.value) return true
  return directories.value.some(entry => entry.path === selected)
})

function errorMessage(cause: unknown): string {
  return cause instanceof Error ? cause.message : String(cause)
}

function isAbsoluteLocation(path: string): boolean {
  return path.startsWith('/')
    || path.startsWith('\\')
    || /^[A-Za-z]:[\\/]/.test(path)
}

function ownsRequest(epoch: number, sequence: number): boolean {
  return props.open
    && props.enabled
    && epoch === openEpoch
    && sequence === browseSequence
}

async function browse(target?: string, epoch = openEpoch) {
  if (!props.open || !props.enabled || epoch !== openEpoch) return
  const sequence = ++browseSequence
  const normalized = target?.trim() || ''
  const params: Record<string, string> = {
    sessionKey: props.sessionKey,
    kind: 'workspace',
  }
  if (normalized) {
    params.path = normalized
    if (!isAbsoluteLocation(normalized) && currentDirectory.value) {
      params.basePath = currentDirectory.value
    }
  }

  phase.value = 'web-loading'
  error.value = ''
  try {
    const response = await rpc.call<SandboxPathListResponse>(
      'sandbox.path.list',
      params,
    )
    if (!ownsRequest(epoch, sequence)) return
    const resolved = String(response.currentPath || response.path || '').trim()
    if (!resolved) throw new Error('Gateway returned an empty directory path.')
    currentDirectory.value = resolved
    locationDraft.value = resolved
    selectedDirectory.value = resolved
    parentDirectory.value = response.parentPath ?? null
    systemPickerAvailable.value = response.systemPickerAvailable === true
    entries.value = Array.isArray(response.entries) ? response.entries : []
    phase.value = 'web-ready'
  } catch (cause) {
    if (!ownsRequest(epoch, sequence)) return
    error.value = errorMessage(cause)
    phase.value = 'web-error'
  }
}

function selectDirectory(path: string) {
  selectedDirectory.value = path
}

function selectRecent(path: string) {
  recentSelectedPath.value = path
}

/** 确认最近工作区：回车/双击/主按钮都走这一条出口。 */
function chooseRecent(path = recentSelectedPath.value) {
  if (!props.enabled) return
  const selected = path.trim()
  if (!selected) return
  const row = recentWorkspaces.value.find(item => item.path === selected)
  // 目录已被删除的工作区不能确认：`workspaces.open` 会失败，静默关窗只会让人
  // 以为打开成功。就地给出原因，把选择留在弹窗里。
  if (row && row.available === false) {
    recentSelectedPath.value = selected
    error.value = t('workspaces.unavailableRecentSelection', { name: row.name })
    return
  }
  invalidateAndClose()
  emit('choose', selected)
}

/**
 * 打开即加载「最近打开」。列表非空 → 默认选中第一条 **可用** 的（最近打开的），
 * 不再弹原生目录选择器；只有首次使用（列表为空）才自动弹一次。
 */
async function loadRecent(epoch: number) {
  if (!props.open || !props.enabled || epoch !== openEpoch) return
  phase.value = 'recent-loading'
  error.value = ''
  recentFailed.value = false
  try {
    const response = await rpc.call<ProjectWorkspacesResponse>('workspaces.list')
    // 用户可能已经转去原生选择器/目录浏览器：迟到的列表结果不得抢回界面。
    if (epoch !== openEpoch || !props.open || phase.value !== 'recent-loading') return
    const rows = Array.isArray(response?.workspaces) ? response.workspaces : []
    recentWorkspaces.value = sortWorkspacesByRecent(
      rows
        .map(normalizeWorkspace)
        .filter((item): item is NormalizedProjectWorkspace => item !== null),
    ).slice(0, 50)
    recentSelectedPath.value = recentWorkspaces.value.find(item => item.available)?.path || ''
    if (recentWorkspaces.value.length === 0) {
      // 首次使用：保留一次自动原生选择器（无原生能力时 runNativePicker 回退到网页浏览器）。
      await runNativePicker(epoch)
      return
    }
    phase.value = 'recent-ready'
    await nextTick()
    if (epoch === openEpoch && props.open) recentListRef.value?.focus()
  } catch (cause) {
    if (epoch !== openEpoch || !props.open || phase.value !== 'recent-loading') return
    // 列表读不到时退到既有的目录浏览器：不把「读取失败」伪装成「首次使用」去弹原生选择器。
    const message = errorMessage(cause)
    recentFailed.value = true
    await browse(props.initialPath?.trim() || undefined, epoch)
    if (epoch === openEpoch && props.open && phaseIs('web-ready')) error.value = message
  }
}

function retryRecentList() {
  if (!props.open || !props.enabled) return
  void loadRecent(openEpoch)
}

/** 列表还在加载时用户想去选目录：放弃这次列表请求，走既有的原生/浏览器兜底。 */
function browseFoldersWhileLoading() {
  if (!props.open || !props.enabled || systemPickerBusy.value) return
  const epoch = openEpoch
  systemPickerBusy.value = true
  void runNativePicker(epoch).finally(() => {
    if (epoch === openEpoch) systemPickerBusy.value = false
  })
}

function browseFolders() {
  if (!props.open || !props.enabled || systemPickerBusy.value) return
  const epoch = openEpoch
  systemPickerBusy.value = true
  void runNativePicker(epoch, {
    keepOpenOnCancel: true,
    initialPath: recentSelectedPath.value || undefined,
  }).finally(() => {
    if (epoch === openEpoch) systemPickerBusy.value = false
  })
}

/** 主项目永远不可取消；其余项目点击切换勾选。 */
function isRootSelected(id: string): boolean {
  return multiSelection.value.includes(id)
}

function toggleRoot(id: string) {
  if (!id || id === props.primaryId) return
  multiSelection.value = isRootSelected(id)
    ? multiSelection.value.filter(candidate => candidate !== id)
    : [...multiSelection.value, id]
}

function applySelection() {
  if (!props.enabled) return
  invalidateAndClose()
  emit('apply', multiSelection.value.filter(id => id !== props.primaryId))
}

async function beginCreateDirectory() {
  creatingDirectory.value = true
  newDirectoryName.value = ''
  await nextTick()
  newDirectoryInputRef.value?.focus()
}

function cancelCreateDirectory() {
  if (creatingDirectoryBusy.value) return
  creatingDirectory.value = false
  newDirectoryName.value = ''
}

async function createDirectory() {
  if (!props.enabled) return
  const name = newDirectoryName.value.trim()
  if (!name || !currentDirectory.value || creatingDirectoryBusy.value) return
  const epoch = openEpoch
  creatingDirectoryBusy.value = true
  error.value = ''
  try {
    const response = await rpc.call<{ path: string }>(
      'sandbox.path.create-directory',
      {
        sessionKey: props.sessionKey,
        parentPath: currentDirectory.value,
        name,
        kind: 'workspace',
      },
    )
    if (!props.open || epoch !== openEpoch) return
    const createdPath = String(response.path || '').trim()
    if (!createdPath) throw new Error('Gateway returned an empty directory path.')
    creatingDirectory.value = false
    newDirectoryName.value = ''
    await browse(createdPath)
  } catch (cause) {
    if (!props.open || epoch !== openEpoch) return
    error.value = t('workspaces.createDirectoryFailed', {
      error: errorMessage(cause),
    })
  } finally {
    if (epoch === openEpoch) creatingDirectoryBusy.value = false
  }
}

function invalidateAndClose() {
  openEpoch += 1
  browseSequence += 1
  phase.value = 'closed'
}

function closeDialog() {
  invalidateAndClose()
  emit('close')
}

function choose() {
  if (!props.enabled || !canChoose.value) return
  const selected = selectedDirectory.value.trim()
  invalidateAndClose()
  emit('choose', selected)
}

async function openSystemPicker() {
  if (!props.open || !props.enabled || systemPickerBusy.value) return
  const epoch = openEpoch
  systemPickerBusy.value = true
  error.value = ''
  const params: Record<string, string> = {
    sessionKey: props.sessionKey,
    kind: 'workspace',
  }
  if (currentDirectory.value) params.initialPath = currentDirectory.value
  try {
    const choice = await rpc.call<{ path: string | null }>(
      'sandbox.path.pick',
      params,
    )
    if (epoch !== openEpoch || !props.open) return
    const selected = String(choice?.path || '').trim()
    if (!selected) return
    invalidateAndClose()
    emit('choose', selected)
  } catch (cause) {
    if (epoch !== openEpoch || !props.open) return
    error.value = t('workspaces.directoryPickerFailed', {
      error: errorMessage(cause),
    })
  } finally {
    if (epoch === openEpoch) systemPickerBusy.value = false
  }
}

async function runNativePicker(
  epoch: number,
  options: { keepOpenOnCancel?: boolean; initialPath?: string } = {},
) {
  if (!props.enabled) return
  const initialPath = (options.initialPath ?? props.initialPath ?? '').trim() || undefined
  const nativePicker = getPlatform().files.chooseProjectDirectory
  if (typeof nativePicker !== 'function') {
    await nextTick()
    if (epoch !== openEpoch || !props.open) return
    pathInputRef.value?.focus()
    await browse(initialPath, epoch)
    return
  }

  phase.value = 'native-picking'
  error.value = ''
  try {
    const choice = await nativePicker({ initialPath })
    if (epoch !== openEpoch || !props.open) return
    if (choice?.path) {
      const selected = choice.path
      invalidateAndClose()
      emit('choose', selected)
    } else if (options.keepOpenOnCancel) {
      // 从「浏览文件夹…」进来时取消：回到最近列表，而不是把整个对话框关掉。
      phase.value = 'recent-ready'
      await nextTick()
      if (epoch === openEpoch && props.open) recentListRef.value?.focus()
    } else {
      closeDialog()
    }
  } catch (cause) {
    if (epoch !== openEpoch || !props.open) return
    error.value = t('workspaces.directoryPickerFailed', {
      error: errorMessage(cause),
    })
    if (options.keepOpenOnCancel) {
      phase.value = 'recent-ready'
      await nextTick()
      if (epoch === openEpoch && props.open) recentListRef.value?.focus()
    } else {
      phase.value = 'desktop-error'
    }
  }
}

function retryNativePicker() {
  void runNativePicker(openEpoch)
}

watch(
  () => props.open,
  async (open) => {
    openEpoch += 1
    browseSequence = 0
    currentDirectory.value = ''
    selectedDirectory.value = ''
    locationDraft.value = props.initialPath?.trim() || ''
    parentDirectory.value = null
    entries.value = []
    error.value = ''
    creatingDirectory.value = false
    creatingDirectoryBusy.value = false
    systemPickerBusy.value = false
    systemPickerAvailable.value = false
    newDirectoryName.value = ''
    multiSelection.value = [...(props.selectedIds || [])]
    recentWorkspaces.value = []
    recentSelectedPath.value = ''
    recentFailed.value = false
    if (!open) {
      phase.value = 'closed'
      return
    }
    if (!props.enabled) {
      phase.value = 'closed'
      return
    }
    if (props.multi) {
      // 多选只是勾选已注册项目，不碰文件系统，也不弹原生目录选择器。
      phase.value = 'web-ready'
      return
    }
    const epoch = openEpoch
    if (props.startInBrowser) {
      // 创建项目：「最近打开」里全是旧目录，不能当成新项目源。
      await browse(props.initialPath?.trim() || undefined, epoch)
      return
    }
    await loadRecent(epoch)
  },
  { immediate: true },
)

watch(
  () => props.enabled,
  enabled => {
    if (enabled || !props.open) return
    invalidateAndClose()
    emit('close')
  },
)

// v-if 卸载后迟到的 RPC 响应仍可能触发 runNativePicker（首个版本的 epoch 只在
// close/reopen 时递增）。卸载时同样作废所有在途请求。
onScopeDispose(() => {
  openEpoch += 1
  browseSequence += 1
  phase.value = 'closed'
})

useDialogA11y(
  dialogRef,
  computed(
    () => props.open
      && props.enabled
      && (props.multi || (phase.value !== 'closed' && phase.value !== 'native-picking')),
  ),
  closeDialog,
  { initialFocus: pathInputRef },
)
</script>

<style scoped>
.modal-overlay {
  position: fixed;
  inset: 0;
  z-index: 1106; /* Layer: modal-dialog */
  display: grid;
  place-items: center;
  background: var(--scrim);
}
.project-picker {
  width: min(92vw, 620px);
  max-height: min(78vh, 620px);
  display: flex;
  flex-direction: column;
  gap: var(--sp-3);
  padding: var(--sp-5);
  border: 1px solid var(--border);
  border-radius: var(--radius-modal);
  background: var(--bg-surface);
}
.project-picker__header,
.project-picker__footer {
  display: flex;
  align-items: center;
  gap: var(--sp-2);
}
.project-picker__header h3 { flex: 1; margin: 0; }
.project-picker__scope { margin: 0; color: var(--text-muted); font-size: var(--fs-sm); }
.project-picker__path {
  display: grid;
  grid-template-columns: auto minmax(0, 1fr) auto;
  align-items: center;
  gap: var(--sp-2);
}
.project-picker__path input {
  width: 100%;
  min-width: 0;
  min-height: 38px;
}
.project-picker__action.btn--ghost {
  min-height: 38px;
  padding: 7px 11px;
  border-color: color-mix(in srgb, var(--border) 88%, transparent);
  background: var(--bg-elevated);
  color: var(--text);
  box-shadow: inset 0 1px 0 color-mix(in srgb, var(--text) 5%, transparent);
}
.project-picker__action.btn--ghost:not(:disabled):hover {
  border-color: color-mix(in srgb, var(--accent) 38%, var(--border));
  background: var(--bg-hover);
}
.project-picker__action.btn--ghost:not(:disabled):active,
.project-picker__choose.btn--primary:not(:disabled):active {
  transform: scale(0.98);
}
.project-picker__action :deep(svg),
.project-picker__choose :deep(svg) {
  flex: 0 0 auto;
}
.project-picker__browser {
  min-height: 180px;
  display: flex;
  flex: 1;
  flex-direction: column;
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  overflow: hidden;
  background: var(--bg-surface);
}
.project-picker__browser-toolbar {
  min-height: 43px;
  display: flex;
  align-items: center;
  justify-content: flex-end;
  padding: 5px var(--sp-2);
  border-bottom: 1px solid color-mix(in srgb, var(--border) 76%, transparent);
  background: color-mix(in srgb, var(--bg-elevated) 60%, var(--bg-surface));
}
.project-picker__entries {
  min-height: 0;
  flex: 1;
  overflow-x: hidden;
  overflow-y: auto;
  overscroll-behavior: contain;
  scrollbar-gutter: stable;
  padding: var(--sp-1);
}
.project-picker__create {
  width: 100%;
  min-height: 32px;
  display: flex;
  align-items: center;
  justify-content: flex-end;
}
.project-picker__create-trigger {
  gap: var(--sp-1);
  font-weight: 600;
}
.project-picker__create-trigger.project-picker__action {
  min-height: 32px;
  padding: 4px 9px;
  background: var(--bg-surface);
  color: var(--text-muted);
  font-size: var(--fs-xs);
}
.project-picker__create-form {
  width: 100%;
  display: flex;
  align-items: center;
  gap: var(--sp-2);
}
.project-picker__create-form input {
  flex: 1;
  min-width: 0;
}
.project-picker__entry {
  width: 100%;
  display: flex;
  gap: var(--sp-2);
  align-items: center;
  padding: var(--sp-2);
  border: 0;
  border-radius: var(--radius-sm);
  background: transparent;
  color: var(--text);
  text-align: left;
}
.project-picker__entry:hover,
.project-picker__entry.is-selected { background: var(--bg-hover); }
.project-picker__roots {
  gap: var(--sp-1);
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  background: var(--bg-surface);
}
.project-picker__root {
  width: 100%;
  min-height: 44px;
  display: flex;
  align-items: center;
  gap: var(--sp-2);
  padding: var(--sp-2);
  border: 0;
  border-radius: var(--radius-sm);
  background: transparent;
  color: var(--text);
  text-align: left;
  cursor: pointer;
}
.project-picker__root:hover,
.project-picker__root:focus-visible { background: var(--bg-hover); }
.project-picker__root:focus-visible { outline: 0; box-shadow: var(--focus-ring); }
.project-picker__root:disabled { cursor: default; opacity: 0.72; }
.project-picker__root.is-selected,
.project-picker__root.is-primary {
  background: color-mix(in srgb, var(--accent) 12%, transparent);
}
.project-picker__root-mark {
  flex: 0 0 auto;
  display: inline-flex;
  color: var(--text-dim);
}
.project-picker__root.is-selected .project-picker__root-mark,
.project-picker__root.is-primary .project-picker__root-mark { color: var(--accent); }
.project-picker__root-copy {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 2px;
}
.project-picker__root-name {
  overflow: hidden;
  color: var(--text);
  font-size: var(--fs-sm);
  text-overflow: ellipsis;
  white-space: nowrap;
}
.project-picker__root-path {
  overflow: hidden;
  color: var(--text-dim);
  font-size: var(--fs-xs);
  text-overflow: ellipsis;
  white-space: nowrap;
}
.project-picker__root-badge {
  flex: 0 0 auto;
  padding: 1px 6px;
  border-radius: var(--radius-pill);
  background: color-mix(in srgb, var(--accent) 16%, transparent);
  color: var(--text);
  font-size: var(--fs-xs);
}
.project-picker__root-badge--warn {
  background: color-mix(in srgb, var(--warn, var(--accent)) 20%, transparent);
}
.project-picker__error { margin: 0; color: var(--danger); font-size: var(--fs-sm); }
.project-picker__footer { justify-content: flex-end; }
.project-picker__choose {
  min-height: 38px;
  padding-inline: var(--sp-4);
  box-shadow: 0 3px 10px color-mix(in srgb, var(--accent) 20%, transparent);
}

@media (max-width: 560px) {
  .project-picker__path {
    grid-template-columns: minmax(0, 1fr) auto;
  }
  .project-picker__parent {
    grid-column: 1 / -1;
    justify-self: start;
  }
}
</style>
