<!-- 输入框顶部上下文条（对齐 ChatGPT / Codex 的胶囊形态）。
     三枚胶囊回答「这次任务在哪做」：
       1. 工作区：当前项目目录（可点击换项目 / 关闭草稿绑定）；
       2. 分支：`workspace.status` 只读解析出的 Git 分支（含本地分支列表）；
       3. 工作位置：本地 / 云端 / 远程 —— 当前版本只有本地执行，另外两项如实标注未接入。
     为什么放在输入框上方而不是底部工具条：这三个选择决定「文件写到哪」，属于每次
     发送前都要确认的前提，与底部的模型/档位（决定"怎么想"）不是一类信息。 -->
<template>
  <div class="ccc">
    <!-- 工作区 -->
    <button
      v-if="canChooseProject"
      type="button"
      class="ccc-chip ccc-chip--button"
      :data-status="workspaceStatus || 'none'"
      :title="workspace?.path || t('chat.context.chooseWorkspace')"
      @click="emit('chooseProject')"
    >
      <Icon name="folder" :size="13" aria-hidden="true" />
      <span class="ccc-chip__text">
        {{ workspace?.name || t('chat.context.chooseWorkspace') }}
      </span>
      <span v-if="statusMessage" class="ccc-chip__status">{{ statusMessage }}</span>
      <Icon name="chevronDown" :size="11" aria-hidden="true" />
    </button>
    <span v-else class="ccc-chip ccc-chip--static" :title="workspace?.path || ''">
      <Icon name="folder" :size="13" aria-hidden="true" />
      <span class="ccc-chip__text">
        {{ workspace?.name || t('chat.context.noWorkspace') }}
      </span>
    </span>
    <button
      v-if="workspace && canCloseProject"
      type="button"
      class="ccc-chip ccc-chip--icon"
      :aria-label="t('workspaces.closeProjectDraft')"
      :title="t('workspaces.closeProjectDraft')"
      @click="emit('closeProject')"
    >
      <Icon name="x" :size="11" aria-hidden="true" />
    </button>

    <!-- 附加项目（P2-9 多根）：只在会话已绑定主项目时出现。
         chip 显示项目名，title 给出完整路径；x 单独移除，不影响主项目。 -->
    <span
      v-for="root in projectRoots"
      :key="root.id"
      class="ccc-chip ccc-chip--root"
      :data-status="root.available ? 'ready' : 'unavailable'"
      :title="root.path"
    >
      <Icon name="folder" :size="13" aria-hidden="true" />
      <span class="ccc-chip__text">{{ root.name }}</span>
      <button
        type="button"
        class="ccc-root-remove"
        :aria-label="t('chat.composer.projectRoots.remove')"
        :title="t('chat.composer.projectRoots.remove')"
        @click="emit('removeProjectRoot', root.id)"
      >
        <Icon name="x" :size="10" aria-hidden="true" />
      </button>
    </span>
    <button
      v-if="canAttachProjectRoots"
      type="button"
      class="ccc-chip ccc-chip--button ccc-chip--add-root"
      :aria-label="t('chat.composer.projectRoots.add')"
      :title="projectRoots.length > 0
        ? t('chat.composer.projectRoots.extraCount', { count: projectRoots.length })
        : t('chat.composer.projectRoots.add')"
      @click="emit('addProjectRoot')"
    >
      <Icon name="plus" :size="12" aria-hidden="true" />
      <span class="ccc-chip__text">{{ t('chat.composer.projectRoots.add') }}</span>
    </button>

    <!-- 分支：只在真实 Git 仓库里出现，绝不猜测 -->
    <div v-if="git" ref="branchAnchorEl" class="ccc-anchor">
      <button
        type="button"
        class="ccc-chip ccc-chip--button"
        :aria-expanded="branchesOpen ? 'true' : 'false'"
        aria-haspopup="listbox"
        :title="git.gitDir || ''"
        @click="branchesOpen = !branchesOpen"
      >
        <Icon name="gitBranch" :size="13" aria-hidden="true" />
        <span class="ccc-chip__text">{{ branchLabel }}</span>
        <Icon name="chevronDown" :size="11" aria-hidden="true" />
      </button>
      <div v-if="branchesOpen" class="ccc-pop" role="listbox">
        <div class="ccc-pop__title">{{ t('chat.context.branchesTitle') }}</div>
        <ul class="ccc-pop__list">
          <li
            v-for="name in git.branches"
            :key="name"
            class="ccc-pop__row"
            :class="{ 'is-current': name === git.branch }"
            role="option"
            :aria-selected="name === git.branch ? 'true' : 'false'"
          >
            <Icon name="check" :size="11" aria-hidden="true" />
            <span>{{ name }}</span>
          </li>
          <li v-if="git.branches.length === 0" class="ccc-pop__empty">
            {{ t('chat.context.branchesEmpty') }}
          </li>
        </ul>
        <div class="ccc-pop__note">{{ t('chat.context.branchesReadonly') }}</div>
      </div>
    </div>

    <!-- 工作位置：本地 / 云端 / 远程（远程 = 直连远程网关，不需要第三方 VPN） -->
    <div class="ccc-location" role="group" :aria-label="t('chat.context.location')">
      <button
        v-for="option in LOCATIONS"
        :key="option.id"
        type="button"
        class="ccc-location__item"
        :class="{ 'is-active': option.id === workLocation, 'is-disabled': option.id === 'cloud' }"
        :disabled="option.id === 'cloud'"
        :aria-pressed="option.id === workLocation ? 'true' : 'false'"
        :title="locationTitle(option.id)"
        @click="chooseLocation(option.id)"
      >
        <Icon :name="option.icon" :size="12" aria-hidden="true" />
        <span>{{ option.id === 'remote' && remoteHost ? remoteHost : t(option.labelKey) }}</span>
      </button>
    </div>

    <!-- 远程网关配置：直连 ws(s)://host/ws，不依赖任何第三方组网工具 -->
    <Teleport to="body">
      <div
        v-if="remoteOpen"
        class="ccc-remote-backdrop"
        @click.self="remoteOpen = false"
      >
        <form class="ccc-remote" @submit.prevent="connectRemote">
          <h3 class="ccc-remote__title">{{ t('chat.context.remoteTitle') }}</h3>
          <p class="ccc-remote__hint">{{ t('chat.context.remoteHint') }}</p>
          <label class="ccc-remote__label" for="ccc-remote-url">
            {{ t('chat.context.remoteUrlLabel') }}
          </label>
          <input
            id="ccc-remote-url"
            v-model="remoteUrl"
            class="ccc-remote__input"
            :placeholder="t('chat.context.remoteUrlPlaceholder')"
            spellcheck="false"
            autocomplete="off"
          >
          <label class="ccc-remote__label" for="ccc-remote-token">
            {{ t('chat.context.remoteTokenLabel') }}
          </label>
          <input
            id="ccc-remote-token"
            v-model="remoteToken"
            class="ccc-remote__input"
            type="password"
            :placeholder="t('chat.context.remoteTokenPlaceholder')"
            autocomplete="off"
          >
          <p v-if="remoteError" class="ccc-remote__error">{{ remoteError }}</p>
          <div class="ccc-remote__actions">
            <button
              v-if="remoteConfigured"
              type="button"
              class="btn btn--sm btn--ghost"
              @click="backToLocal"
            >
              {{ t('chat.context.remoteBack') }}
            </button>
            <span class="ccc-remote__spacer" />
            <button type="button" class="btn btn--sm btn--ghost" @click="remoteOpen = false">
              {{ t('common.cancel') }}
            </button>
            <button type="submit" class="btn btn--sm btn--primary">
              {{ t('chat.context.remoteConnect') }}
            </button>
          </div>
        </form>
      </div>
    </Teleport>
  </div>
</template>

<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import Icon from '@/components/Icon.vue'
import type { IconName } from '@/utils/icons'
import type { WorkspaceGitSummary } from '@/composables/sessions/useWorkspaceStatus'

const { t } = useI18n()

const props = withDefaults(defineProps<{
  workspace?: { id: string; name: string; path: string } | null
  workspaceStatus?: string
  statusMessage?: string
  canCloseProject?: boolean
  canChooseProject?: boolean
  /** 附加项目（不含主项目），由 ChatView 的会话级状态派生。 */
  projectRoots?: Array<{ id: string; name: string; path: string; available: boolean }>
  /** 是否显示「+ 附加项目」入口（需要能读取工作区清单）。 */
  canAttachProjectRoots?: boolean
  git?: WorkspaceGitSummary | null
}>(), {
  workspace: null,
  workspaceStatus: 'none',
  statusMessage: '',
  canCloseProject: false,
  canChooseProject: false,
  projectRoots: () => [],
  canAttachProjectRoots: false,
  git: null,
})

const emit = defineEmits<{
  chooseProject: []
  closeProject: []
  addProjectRoot: []
  removeProjectRoot: [id: string]
}>()

const branchesOpen = ref(false)
const branchAnchorEl = ref<HTMLElement | null>(null)

const branchLabel = computed(() => {
  if (!props.git) return ''
  if (props.git.branch) return props.git.branch
  return props.git.detached ? t('chat.context.detached') : t('chat.context.branchesEmpty')
})

type WorkLocation = 'local' | 'cloud' | 'remote'
const LOCATIONS: Array<{ id: WorkLocation; labelKey: string; icon: IconName }> = [
  { id: 'local', labelKey: 'chat.context.locationLocal', icon: 'monitor' },
  { id: 'cloud', labelKey: 'chat.context.locationCloud', icon: 'cloud' },
  { id: 'remote', labelKey: 'chat.context.locationRemote', icon: 'boxes' },
]

const WS_URL_KEY = 'marguerite.wsUrl'
const WS_TOKEN_KEY = 'marguerite.wsToken'

/**
 * 「远程」= 直连**远程 Marguerite 网关**（ws(s)://host/ws）。
 *
 * 为什么这么做：本项目自带 SSH 远程协同与网关协议，不需要让用户安装 Radmin VPN
 * 之类的第三方组网工具。切到远程后整个前端（对话、文件、终端）都由那台机器执行。
 * 云端厂商运行时还没有，因此 `cloud` 仍然禁用并给出原因。
 */
function readRemoteUrl(): string {
  try {
    return (localStorage.getItem(WS_URL_KEY) || '').trim()
  } catch {
    return ''
  }
}

function defaultWsUrl(): string {
  const proto = location.protocol === 'https:' ? 'wss:' : 'ws:'
  return `${proto}//${location.host}/ws`
}

const remoteUrlSaved = ref(readRemoteUrl())
const remoteConfigured = computed(() => Boolean(remoteUrlSaved.value))
const remoteHost = computed(() => {
  const raw = remoteUrlSaved.value
  if (!raw) return ''
  try {
    return new URL(raw).host
  } catch {
    return raw.replace(/^wss?:\/\//, '').replace(/\/.*$/, '')
  }
})
const workLocation = ref<WorkLocation>(remoteConfigured.value ? 'remote' : 'local')

const remoteOpen = ref(false)
const remoteUrl = ref(remoteUrlSaved.value || defaultWsUrl())
const remoteToken = ref('')
const remoteError = ref('')

function locationTitle(id: WorkLocation): string {
  if (id === 'local') return t('chat.context.locationLocalHint')
  if (id === 'cloud') return t('chat.context.locationUnavailable')
  return remoteConfigured.value
    ? t('chat.context.remoteConnectedHint', { host: remoteHost.value })
    : t('chat.context.remoteHintShort')
}

function chooseLocation(next: WorkLocation) {
  if (next === 'cloud') return
  if (next === 'local') {
    workLocation.value = 'local'
    if (remoteConfigured.value) {
      remoteOpen.value = true
      remoteError.value = ''
    }
    return
  }
  remoteUrl.value = remoteUrlSaved.value || defaultWsUrl()
  remoteError.value = ''
  remoteOpen.value = true
}

function connectRemote(): void {
  const raw = remoteUrl.value.trim()
  let parsed: URL
  try {
    parsed = new URL(raw)
  } catch {
    remoteError.value = t('chat.context.remoteInvalid')
    return
  }
  if (parsed.protocol !== 'ws:' && parsed.protocol !== 'wss:') {
    remoteError.value = t('chat.context.remoteInvalid')
    return
  }
  try {
    localStorage.setItem(WS_URL_KEY, parsed.toString())
    if (remoteToken.value.trim()) {
      sessionStorage.setItem(WS_TOKEN_KEY, remoteToken.value.trim())
    }
  } catch {
    remoteError.value = t('chat.context.remoteStorageFailed')
    return
  }
  // 连接参数在页面加载时读取：必须重载才能建立新的 WS 会话。
  location.reload()
}

function backToLocal(): void {
  try {
    localStorage.removeItem(WS_URL_KEY)
    sessionStorage.removeItem(WS_TOKEN_KEY)
  } catch {
    /* 存储不可用时也只能尽力而为 */
  }
  location.reload()
}

function onDocumentPointerDown(event: PointerEvent) {
  if (!branchesOpen.value) return
  const anchor = branchAnchorEl.value
  if (anchor && event.target instanceof Node && anchor.contains(event.target)) return
  branchesOpen.value = false
}

function onKeydown(event: KeyboardEvent) {
  if (event.key === 'Escape' && branchesOpen.value) branchesOpen.value = false
}

onMounted(() => {
  document.addEventListener('pointerdown', onDocumentPointerDown)
  document.addEventListener('keydown', onKeydown)
})
onBeforeUnmount(() => {
  document.removeEventListener('pointerdown', onDocumentPointerDown)
  document.removeEventListener('keydown', onKeydown)
})
</script>

<style scoped>
.ccc {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 0.375rem;
  padding: 0 0.125rem 0.375rem;
  min-width: 0;
}
.ccc-anchor { position: relative; min-width: 0; }

.ccc-chip {
  display: inline-flex;
  align-items: center;
  gap: 0.3rem;
  max-width: 16rem;
  min-height: 26px;
  padding: 0.15rem 0.5rem;
  border: 1px solid var(--border);
  border-radius: var(--radius-pill);
  background: transparent;
  color: var(--text-muted);
  font-size: 0.75rem;
  line-height: 1.4;
  white-space: nowrap;
}
.ccc-chip__text {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.ccc-chip__status {
  color: var(--warn, var(--text-dim));
  font-size: 0.6875rem;
}
.ccc-chip--static { cursor: default; }
.ccc-chip--button { cursor: pointer; }
.ccc-chip--button:hover,
.ccc-chip--button:focus-visible {
  outline: 0;
  border-color: color-mix(in srgb, var(--accent) 45%, var(--border));
  color: var(--text);
}
.ccc-chip--button:focus-visible { box-shadow: var(--focus-ring); }
.ccc-chip--icon { padding: 0.15rem 0.35rem; }
.ccc-chip--root { padding-right: 0.2rem; }
.ccc-chip--root[data-status='unavailable'] {
  border-color: color-mix(in srgb, var(--warn, var(--accent)) 55%, var(--border));
}
.ccc-root-remove {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 16px;
  height: 16px;
  padding: 0;
  border: 0;
  border-radius: var(--radius-pill);
  background: transparent;
  color: var(--text-dim);
  cursor: pointer;
}
.ccc-root-remove:hover,
.ccc-root-remove:focus-visible {
  outline: 0;
  background: color-mix(in srgb, var(--text) 12%, transparent);
  color: var(--text);
}
.ccc-chip[data-status='unavailable'],
.ccc-chip[data-status='removed'],
.ccc-chip[data-status='error'] {
  border-color: color-mix(in srgb, var(--warn, var(--accent)) 55%, var(--border));
  color: var(--text);
}

.ccc-pop {
  position: absolute;
  left: 0;
  bottom: calc(100% + 6px);
  z-index: 40;
  width: 15rem;
  padding: 0.5rem;
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  background: var(--surface-2);
  box-shadow: var(--shadow-lg);
}
.ccc-pop__title {
  padding: 0.125rem 0.25rem 0.375rem;
  color: var(--text-dim);
  font-size: 0.6875rem;
  letter-spacing: 0.04em;
  text-transform: uppercase;
}
.ccc-pop__list {
  margin: 0;
  padding: 0;
  list-style: none;
  max-height: 14rem;
  overflow-y: auto;
}
.ccc-pop__row {
  display: flex;
  align-items: center;
  gap: 0.375rem;
  padding: 0.25rem 0.375rem;
  border-radius: var(--radius-sm);
  color: var(--text-muted);
  font-size: 0.75rem;
}
.ccc-pop__row > svg { visibility: hidden; flex: 0 0 auto; }
.ccc-pop__row.is-current { color: var(--text); background: var(--surface-3); }
.ccc-pop__row.is-current > svg { visibility: visible; color: var(--accent); }
.ccc-pop__empty { padding: 0.25rem 0.375rem; color: var(--text-dim); font-size: 0.75rem; }
.ccc-pop__note {
  margin-top: 0.375rem;
  padding-top: 0.375rem;
  border-top: 1px solid var(--border);
  color: var(--text-dim);
  font-size: 0.6875rem;
  line-height: 1.45;
}

.ccc-location {
  display: inline-flex;
  align-items: center;
  gap: 1px;
  padding: 1px;
  border: 1px solid var(--border);
  border-radius: var(--radius-pill);
}
.ccc-location__item {
  display: inline-flex;
  align-items: center;
  gap: 0.25rem;
  padding: 0.15rem 0.45rem;
  border: 0;
  border-radius: var(--radius-pill);
  background: transparent;
  color: var(--text-dim);
  font-size: 0.6875rem;
  line-height: 1.5;
  cursor: pointer;
}
.ccc-location__item.is-active {
  background: color-mix(in srgb, var(--accent) 16%, transparent);
  color: var(--text);
}
.ccc-location__item.is-disabled { cursor: not-allowed; opacity: 0.55; }
.ccc-location__item:focus-visible { outline: 0; box-shadow: var(--focus-ring); }

/* 远程网关配置弹层 */
.ccc-remote-backdrop {
  position: fixed;
  z-index: 1400;
  inset: 0;
  display: grid;
  place-items: center;
  background: color-mix(in srgb, var(--scrim) 42%, transparent);
}
.ccc-remote {
  display: grid;
  width: min(420px, calc(100vw - 32px));
  gap: 8px;
  padding: 16px;
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  background: var(--bg-elevated, var(--bg-surface));
  box-shadow: var(--shadow-lg);
}
.ccc-remote__title { margin: 0; color: var(--text); font-size: 14px; font-weight: 600; }
.ccc-remote__hint { margin: 0; color: var(--text-dim); font-size: 11.5px; line-height: 1.55; }
.ccc-remote__label { color: var(--text-dim); font-size: 11px; }
.ccc-remote__input {
  width: 100%;
  padding: 6px 8px;
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  background: var(--bg);
  color: var(--text);
  font: inherit;
  font-size: 12.5px;
}
.ccc-remote__error { margin: 0; color: var(--danger); font-size: 11.5px; }
.ccc-remote__actions { display: flex; align-items: center; gap: 6px; }
.ccc-remote__spacer { flex: 1; }

@media (max-width: 640px) {
  .ccc-chip { max-width: 9rem; }
}
</style>
