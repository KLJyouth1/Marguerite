<template>
  <div
    ref="composerEl"
    class="chat-composer"
    :class="{
      'chat-composer--new-landing': isNewLanding,
      'chat-composer--collapsed': collapsed,
      'chat-composer--floating': floating,
      'chat-composer--docked': !floating,
    }"
  >
    <div class="chat-composer-inner">
      <div v-if="attachments.length > 0" class="chat-collapse-region">
        <div class="chat-attachments">
          <div
            v-for="(att, i) in attachments"
            :key="att.local_id"
            class="attachment-chip"
            :class="{ 'attachment-chip--busy': isAttachmentBusy(att), 'attachment-chip--failed': att.kind === 'failed' }"
            :data-mime="att.mime || ''"
            :title="attachmentTitle(att)"
          >
            <span class="attachment-chip__icon" aria-hidden="true">
              <span v-if="isAttachmentBusy(att)" class="spinner attachment-chip__spinner" />
              <Icon v-else-if="att.kind === 'failed'" name="info" :size="15" />
              <img v-else-if="isImageDisplayAttachment(att) && att.dataUrl" class="attachment-chip__thumb" :src="att.dataUrl" alt="" />
              <Icon v-else :name="attachmentIcon(att)" :size="15" />
            </span>
            <span class="attachment-chip__name">{{ att.name }}</span>
            <span class="attachment-chip__meta">{{ attachmentMeta(att) }}</span>
            <button v-if="att.kind === 'failed' && att.file" class="attachment-action" title="Retry upload" aria-label="Retry upload" @click="emit('retryAttachment', i)">
              <Icon name="refresh" :size="12" />
            </button>
            <button class="attachment-action attachment-remove" :title="t('chat.remove')" :aria-label="t('chat.remove')" @click="emit('removeAttachment', i)">
              <Icon name="x" :size="12" />
            </button>
          </div>
        </div>
      </div>
      <div v-if="contextRefs.length > 0" class="chat-collapse-region">
        <div class="chat-attachments">
          <div
            v-for="r in contextRefs"
            :key="`${r.kind}:${r.id}`"
            class="attachment-chip"
            :title="r.path || r.id"
          >
            <span class="attachment-chip__icon" aria-hidden="true">
              <Icon :name="r.kind === 'folder' ? 'folder' : 'fileText'" :size="15" />
            </span>
            <span class="attachment-chip__name">@{{ r.label }}</span>
            <button
              class="attachment-action attachment-remove"
              :title="t('chat.remove')"
              :aria-label="t('chat.remove')"
              @click="removeRef(r.kind, r.id)"
            >
              <Icon name="x" :size="12" />
            </button>
          </div>
        </div>
      </div>
      <div class="chat-input-panel">
        <div v-if="replanActive" class="chat-collapse-region">
          <div
            class="chat-replan-draft"
            role="status"
            aria-live="polite"
          >
            <span class="chat-replan-draft__icon" aria-hidden="true">
              <Icon name="pencil" :size="14" />
            </span>
            <span class="chat-replan-draft__copy">
              <strong>{{ t('chat.plan.revising') }}</strong>
              {{ t('chat.plan.reviseDraftHint') }}
            </span>
            <button
              type="button"
              class="chat-replan-draft__cancel"
              @click="emit('cancelReplan')"
            >
              {{ t('common.cancel') }}
            </button>
          </div>
        </div>
        <!-- 输入框上方上下文条：工作区 / 分支 / 工作位置（Codex 式胶囊） -->
        <ChatComposerContextBar
          v-if="!collapsed"
          :workspace="projectWorkspace"
          :workspace-status="projectWorkspaceStatus"
          :status-message="projectStatusMessage"
          :can-close-project="canCloseProject"
          :can-choose-project="canChooseProject"
          :project-roots="projectRoots"
          :can-attach-project-roots="canAttachProjectRoots && Boolean(projectWorkspace)"
          :git="workspaceGit"
          @choose-project="emit('chooseProject')"
          @close-project="emit('closeProject')"
          @add-project-root="projectRootsOpen = true"
          @remove-project-root="emit('removeProjectRoot', $event)"
        />
        <div class="chat-input-wrap">
          <textarea
            ref="textareaEl"
            v-model="inputText"
            class="chat-textarea"
            rows="1"
            :placeholder="placeholder"
            :disabled="inputDisabled"
            maxlength="100000"
            :aria-label="t('chat.messageToSend')"
            :aria-describedby="sendBlockedMessage ? 'chat-composer-send-status' : undefined"
            @beforeinput="emit('expand'); emit('beforeinput', $event)"
            @input="emit('input', $event)"
            @keydown="onTextareaKeydown"
            @compositionstart="emit('compositionChange', true)"
            @compositionend="emit('compositionChange', false)"
            @pointerdown="emit('expand')"
            @focus="emit('expand')"
          />
          <ComposerMentionPopover
            v-if="mentionQuery !== null"
            ref="mentionPopoverEl"
            :query="mentionQuery"
            :workspace-id="projectWorkspace?.id"
            @select="applyMention"
          />
        </div>
        <div class="chat-collapse-region chat-collapse-region--footer">
          <div class="chat-input-footer">
          <div class="chat-input-actions chat-input-actions--left">
            <ChatComposerPresets
              v-if="showPresets"
              :run-mode-locked="runModeLocked"
              :model-routing-mode="modelRoutingMode"
              :model-routing-busy="modelRoutingSettingsBusy"
              @open-advanced="openAdvancedFromPresets"
              @set-model-routing-mode="emit('setModelRoutingMode', $event)"
            />
            <ChatModelPicker />
            <!-- 记忆楼层：8 层（7 层记忆 + 经验）可逐层开关，关掉的层不参与检索也不注入 -->
            <MemoryFloorsPopover />
            <div ref="addMenuAnchorEl" class="chat-settings-anchor">
              <button
                class="btn btn--icon btn--ghost chat-plus-btn"
                :class="{ 'is-active': addMenuOpen }"
                :title="t('chat.add')"
                :aria-label="t('chat.add')"
                aria-haspopup="menu"
                :aria-expanded="addMenuOpen ? 'true' : 'false'"
                @click="toggleAddMenu"
              >
                <Icon name="plus" :size="18" />
              </button>
              <ChatComposerAddMenu
                v-if="addMenuOpen"
                :attachments-disabled="replanActive"
                :plan-mode-active="collaborationMode === 'plan'"
                :plan-mode-available="planModeAvailable === true"
                :plan-mode-busy="planModeBusy === true || planModeDisabled === true"
                @activate-plan-mode="emit('setCollaborationMode', 'plan')"
                @attach-files="fileInputEl?.click()"
                @open-commands="emit('openCommands')"
                @open-artifacts="emit('openArtifacts')"
                @open-file-tree="emit('openFileTree')"
                @open-skills="emit('openSkills')"
                @open-plugins="emit('openPlugins')"
                @close="addMenuOpen = false"
              />
            </div>
            <button
              v-if="codingModeEnabled"
              type="button"
              class="chat-coding-mode-chip"
              :title="t('chat.codingMode.disableLabel')"
              :aria-label="t('chat.codingMode.disableLabel')"
              :aria-busy="codingModeSettingsBusy ? 'true' : 'false'"
              :disabled="codingModeSettingsBusy"
              @click="emit('setCodingModeEnabled', false)"
            >
              <span>{{ t('chat.codingMode.activeLabel') }}</span>
              <Icon name="x" :size="12" aria-hidden="true" />
            </button>
            <RunModePanel
              v-model:open="runModePanelOpen"
              :run-mode="runMode"
              :allowed-run-modes="allowedRunModes"
              :safe-setup-available="safeSetupAvailable"
              :safe-setup-state="safeSetupState"
              :run-mode-locked="runModeLocked"
              :run-mode-lock-message="runModeLockMessage"
              :collaboration-mode="collaborationMode || 'default'"
              :plan-mode-available="planModeAvailable === true"
              :plan-mode-busy="planModeBusy === true"
              :plan-mode-disabled="planModeDisabled === true"
              :plan-mode-applies-next-turn="planModeAppliesNextTurn === true"
              :approval-bypass="approvalBypass === true"
              :model-routing-mode="modelRoutingMode"
              :model-routing-settings-busy="modelRoutingSettingsBusy"
              :show-new-badge="showRouterNewBadge"
              @set-run-mode="emit('setRunMode', $event)"
              @set-collaboration-mode="emit('setCollaborationMode', $event)"
              @set-approval-bypass="emit('setApprovalBypass', $event)"
              @set-model-routing-mode="emit('setModelRoutingMode', $event)"
            />
<ChatEffortSlider
              :model-value="effort"
              :disabled="runModeLocked"
              :routing-on="modelRoutingMode !== 'off'"
              :applies-next-turn="isStreaming"
              @update:model-value="emit('setEffort', $event)"
            />
            <div v-if="props.orchestration" ref="agentPanelAnchorEl" class="chat-settings-anchor">
              <button
                type="button"
                class="btn btn--icon btn--ghost chat-orch-btn"
                :class="{ 'is-active': orchestration?.form === 'subagent' }"
                :title="t('chat.orchestration.btnAssign')"
                :aria-label="t('chat.orchestration.btnAssign')"
                aria-haspopup="dialog"
                :aria-expanded="agentPanelOpen ? 'true' : 'false'"
                data-testid="chat-orchestration-btn-agents"
                @click="toggleAgentPanel"
              >
                <Icon name="agents" :size="17" />
                <span
                  v-if="(orchestration?.agents?.length ?? 0) > 0"
                  class="chat-orch-badge"
                  data-testid="orch-agents-count"
                >{{ orchestration!.agents!.length }}</span>
              </button>
              <ChatAgentPicker
                :open="agentPanelOpen"
                :agents="orchestration?.agents ?? []"
                @update:open="agentPanelOpen = $event"
                @update:agents="applyAgents"
              />
            </div>
            <div v-if="props.orchestration" ref="teamPanelAnchorEl" class="chat-settings-anchor">
              <button
                type="button"
                class="btn btn--icon btn--ghost chat-orch-btn"
                :class="{ 'is-active': orchestration?.form === 'team' || orchestration?.form === 'expert' }"
                :title="t('chat.orchestration.btnTeam')"
                :aria-label="t('chat.orchestration.btnTeam')"
                aria-haspopup="dialog"
                :aria-expanded="teamPanelOpen ? 'true' : 'false'"
                data-testid="chat-orchestration-btn-team"
                @click="toggleTeamPanel"
              >
                <Icon name="boxes" :size="17" />
              </button>
              <ChatTeamPicker
                :open="teamPanelOpen"
                :teams="orchestration?.teams ?? []"
                @update:open="teamPanelOpen = $event"
                @toggle-team="toggleTeam"
                @clear-teams="clearTeams"
              />
            </div>
            <div v-if="props.orchestration" ref="swarmPanelAnchorEl" class="chat-settings-anchor">
              <button
                type="button"
                class="btn btn--icon btn--ghost chat-orch-btn"
                :class="{ 'is-active': orchestration?.swarmOn === true }"
                :title="t('chat.orchestration.btnSwarm')"
                :aria-label="t('chat.orchestration.btnSwarm')"
                aria-haspopup="dialog"
                :aria-expanded="swarmPanelOpen ? 'true' : 'false'"
                data-testid="chat-orchestration-btn-swarm"
                @click="toggleSwarmPanel"
              >
                <Icon name="swarm" :size="17" />
              </button>
              <ChatSwarmPopover
                :open="swarmPanelOpen"
                :swarm-on="orchestration?.swarmOn === true"
                :swarm-size="orchestration?.swarmSize ?? 3"
                @update:open="swarmPanelOpen = $event"
                @update:swarm-on="applySwarmOn"
                @update:swarm-size="applySwarmSize"
              />
            </div>
            <div v-if="props.orchestration" ref="modePanelAnchorEl" class="chat-settings-anchor">
              <button
                type="button"
                class="btn btn--icon btn--ghost chat-orch-btn"
                :class="{ 'is-active': orchestration?.modeKey && orchestration.modeKey !== 'standard_doit' }"
                :title="t('chat.orchestration.capabilitiesTitle')"
                :aria-label="t('chat.orchestration.capabilitiesTitle')"
                aria-haspopup="dialog"
                :aria-expanded="modePanelOpen ? 'true' : 'false'"
                data-testid="chat-orchestration-btn-mode"
                @click="toggleModePanel"
              >
                <Icon name="mode" :size="17" />
              </button>
              <ChatModeCapabilityPanel
                :open="modePanelOpen"
                :mode-key="orchestration?.modeKey ?? 'standard_doit'"
                :capabilities="orchestration?.capabilities ?? []"
                @update:open="modePanelOpen = $event"
                @update:capabilities="applyCapabilities"
              />
            </div>
            <div ref="moreActionsAnchorEl" class="chat-settings-anchor">
              <button
                class="btn btn--icon btn--ghost chat-more-actions-btn"
                :class="{ 'is-active': moreActionsOpen }"
                :title="t('chrome.more')"
                :aria-label="t('chrome.more')"
                aria-haspopup="menu"
                :aria-expanded="moreActionsOpen ? 'true' : 'false'"
                @click="toggleMoreActions"
              >
                <Icon name="moreHorizontal" :size="18" />
              </button>
              <div
                v-if="moreActionsOpen"
                class="chat-more-actions-menu"
                role="menu"
                :aria-label="t('chrome.more')"
              >
                <button
                  type="button"
                  role="menuitem"
                  :class="{ 'is-active': voiceRecording, 'chat-mic--needs-setup': !voiceReady }"
                  :title="voiceReady ? t('chat.recordVoice') : t('chat.voiceUnavailableHint')"
                  :aria-label="voiceReady ? t('chat.recordVoice') : t('chat.voiceUnavailableHint')"
                  :disabled="voiceBusy"
                  @click="triggerVoice"
                >
                  <Icon name="microphone" :size="16" />
                  <span>{{ voiceReady ? t('chat.recordVoice') : t('chat.voiceUnavailableHint') }}</span>
                </button>
                <button
                  type="button"
                  role="menuitem"
                  :aria-label="t('chat.exportMarkdown')"
                  @click="exportConversation"
                >
                  <Icon name="download" :size="16" />
                  <span>{{ t('chat.exportMarkdown') }}</span>
                </button>
                <button
                  v-if="promptCacheKeepaliveAvailable"
                  type="button"
                  role="menuitem"
                  data-testid="chat-composer-action-keepalive"
                  :title="promptCacheKeepaliveSessionReady
                    ? t('chat.promptCacheKeepalive.action')
                    : t('chat.promptCacheKeepalive.unavailableHint')"
                  :aria-label="promptCacheKeepaliveAriaLabel"
                  :disabled="!promptCacheKeepaliveSessionReady"
                  @click="openPromptCacheKeepalive"
                >
                  <Icon name="clock" :size="16" />
                  <span class="chat-more-actions-menu__copy">
                    <span>{{ t('chat.promptCacheKeepalive.action') }}</span>
                    <small v-if="!promptCacheKeepaliveSessionReady">
                      {{ t('chat.promptCacheKeepalive.unavailableHint') }}
                    </small>
                    <small
                      v-else-if="promptCacheKeepaliveStatusText"
                      class="chat-more-actions-menu__keepalive-status"
                      :data-state="promptCacheKeepaliveStatus?.state"
                    >
                      <span class="chat-more-actions-menu__status-dot" aria-hidden="true" />
                      {{ promptCacheKeepaliveStatusText }}
                    </small>
                  </span>
                </button>
              </div>
            </div>
          </div>
          <ChatContextMeter :status="contextStatus ?? null" />
          <div class="chat-input-actions chat-input-actions--right">
            <Transition name="composer-ctl" mode="out-in">
              <button
                v-if="canStop"
                key="stop"
                class="btn btn--icon btn--danger chat-send-btn"
                :title="stopTargetsPlanRun
                  ? t('chat.planRun.stopExecutionEsc')
                  : t('chat.stopResponseEsc')"
                :aria-label="stopTargetsPlanRun
                  ? t('chat.planRun.stopExecution')
                  : t('chat.stopResponse')"
                @click="emit('stop')"
              >
                <Icon name="stop" :size="16" />
              </button>
              <button
                v-else
                key="send"
                class="btn btn--icon btn--primary chat-send-btn"
                :class="{ 'is-ready': hasSendContent && !sendBlockedMessage && !inputDisabled }"
                :title="sendBlockedMessage || sendButtonTitle"
                :aria-label="replanActive ? t('chat.plan.reviseSend') : t('chat.send')"
                :aria-describedby="sendBlockedMessage ? 'chat-composer-send-status' : undefined"
                :disabled="Boolean(sendBlockedMessage) || inputDisabled"
                @click="submitFromButton"
              >
                <Icon name="arrowUp" :size="17" />
              </button>
            </Transition>
          </div>
          </div>
        </div>
      </div>
      <div v-if="sendBlockedMessage" class="chat-collapse-region">
        <p
          id="chat-composer-send-status"
          class="chat-composer-send-status"
          role="status"
          aria-live="polite"
          aria-atomic="true"
        >{{ sendBlockedMessage }}</p>
      </div>
      <div class="chat-collapse-region chat-collapse-region--disclaimer">
        <p class="chat-ai-disclaimer" role="note">{{ t('chat.aiDisclaimer') }}</p>
      </div>
    </div>
    <input
      ref="fileInputEl"
      type="file"
      multiple
      class="hidden"
      @change="emit('fileChange', $event)"
    />
    <!-- 附加项目多选：复用项目选择弹窗的多选模式（只勾选已注册项目，不浏览文件系统）。 -->
    <ProjectWorkspacePickerDialog
      v-if="canAttachProjectRoots && projectWorkspace"
      :open="projectRootsOpen"
      :multi="true"
      :enabled="true"
      :primary-id="primaryProjectId"
      :selected-ids="projectRoots.map(root => root.id)"
      :workspaces="projectRootOptions"
      @close="projectRootsOpen = false"
      @apply="applyProjectRoots"
    />
  </div>
</template>

<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import Icon from '@/components/Icon.vue'
import type { IconName } from '@/utils/icons'
import ChatComposerAddMenu from '@/components/chat/ChatComposerAddMenu.vue'
import ChatModelPicker from '@/components/chat/ChatModelPicker.vue'
import MemoryFloorsPopover from '@/components/chat/MemoryFloorsPopover.vue'
import ChatComposerPresets from '@/components/chat/ChatComposerPresets.vue'
import ChatEffortSlider from '@/components/chat/ChatEffortSlider.vue'
import ChatAgentPicker from '@/components/chat/ChatAgentPicker.vue'
import ChatComposerContextBar from '@/components/chat/ChatComposerContextBar.vue'
import ChatModeCapabilityPanel from '@/components/chat/ChatModeCapabilityPanel.vue'
import ChatSwarmPopover from '@/components/chat/ChatSwarmPopover.vue'
import ChatTeamPicker from '@/components/chat/ChatTeamPicker.vue'
import ComposerMentionPopover from '@/components/chat/ComposerMentionPopover.vue'
import {
  addContextRef,
  removeContextRef,
  useContextRefs,
  type ContextRefItem,
} from '@/composables/chat/contextRefs'
import RunModePanel from '@/components/chat/RunModePanel.vue'
import ChatContextMeter from '@/components/chat/ChatContextMeter.vue'
import type { ContextStatus } from '@/composables/chat/useChatUsageWidget'
import ProjectWorkspacePickerDialog from '@/components/ProjectWorkspacePickerDialog.vue'
import {
  EXPERT_TEAMS,
  type OrchestrationState,
} from '@/composables/chat/orchestration'
import type { Attachment } from '@/types/chat'
import type { ProjectWorkspaceItem } from '@/types/rpc'
import type { EffortLevel } from '@/types/effort'
import type { ModelRoutingMode } from '@/types/modelRouting'
import type { SandboxRunMode, SandboxSetupState } from '@/types/sandbox'
import type { CollaborationMode } from '@/types/plans'
import type { PromptCacheKeepaliveStatus } from '@/types/promptCacheKeepalive'
import type { WorkspaceGitSummary } from '@/composables/sessions/useWorkspaceStatus'
import { isAttachmentBusy, isImageDisplayAttachment } from '@/utils/chat/attachments'
import { useActiveMode } from '@/composables/useActiveMode'

interface ChatComposerExpose {
  composerElement: () => HTMLElement | null
  canCollapse: () => boolean
  focusTextarea: () => void
  isTextareaFocused: () => boolean
  resizeTextarea: () => void
}

const props = withDefaults(defineProps<{
  attachments: Attachment[]
  busySendMode: 'queue' | 'steer'
  hasSendContent: boolean
  isStreaming: boolean
  canStop: boolean
  stopTargetsPlanRun?: boolean
  isNewLanding: boolean
  placeholder: string
  sendButtonTitle: string
  sendBlockedMessage?: string
  inputDisabled?: boolean
  runMode: SandboxRunMode
  allowedRunModes: SandboxRunMode[]
  safeSetupAvailable?: boolean
  /** 沙箱 setup 真实状态（未 ready → 安全模式显示「待安装」但仍可点击）。 */
  safeSetupState?: SandboxSetupState | null
  runModeLocked: boolean
  runModeLockMessage: string
  modelRoutingMode: ModelRoutingMode
  modelRoutingSettingsBusy: boolean
  /** 推理等级（off/normal/deep/max）：底栏滑块的当前档位，由父级持久化。 */
  effort?: EffortLevel
  codingModeEnabled?: boolean
  codingModeSettingsBusy?: boolean
  voiceBusy: boolean
  voiceRecording: boolean
  voiceReady: boolean
  projectWorkspace?: { id: string; name: string; path: string } | null
  projectWorkspaceStatus?: 'none' | 'resolving' | 'ready' | 'unavailable' | 'removed' | 'unknown' | 'error'
  projectStatusMessage?: string
  /** 工作区 Git 摘要（只读）：由 `workspace.status` 一并解析，用于分支胶囊。 */
  workspaceGit?: WorkspaceGitSummary | null
  canCloseProject?: boolean
  canChooseProject?: boolean
  /** 会话级附加项目（不含主项目）：chips 展示，每个可单独移除。 */
  projectRoots?: Array<{ id: string; name: string; path: string; available: boolean }>
  /** 附加项目候选项（工作区清单），供多选弹窗渲染 name/path/可用性。 */
  projectRootOptions?: ProjectWorkspaceItem[]
  /** 是否显示「+ 附加项目」入口（可读取工作区清单时为 true）。 */
  canAttachProjectRoots?: boolean
  /** 主项目 id：多选弹窗里置灰且不可取消。 */
  primaryProjectId?: string
  planModeAvailable?: boolean
  collaborationMode?: CollaborationMode
  planModeBusy?: boolean
  planModeDisabled?: boolean
  planModeAppliesNextTurn?: boolean
  /** P2-3：当前会话是否已开启审批旁路（会话级提权，见 RunModePanel）。 */
  approvalBypass?: boolean
  /** M1 T5：网关 `usage.status` 的 contextStatus；null/缺失时脚注整块不渲染。 */
  contextStatus?: ContextStatus | null
  replanActive?: boolean
  promptCacheKeepaliveAvailable?: boolean
  promptCacheKeepaliveSessionReady?: boolean
  promptCacheKeepaliveStatus?: PromptCacheKeepaliveStatus | null
  /** Collapsed to a single-line input (floating-composer retract). */
  collapsed?: boolean
  /** Floating-composer toggle: false docks the panel (solid surface, no
      glass) even though the composer still renders in the normal layout. */
  floating?: boolean
/** D-02：在底栏渲染 3×3 模式选择胶囊（仅 ChatView 开启）。 */
  showPresets?: boolean
  /** P2·E：对话编排状态。form 非 single 时，父级把当前输入提交给 orchestrate.run。 */
  orchestration?: OrchestrationState
}>(), {
  canChooseProject: true,
  canAttachProjectRoots: false,
  primaryProjectId: '',
  projectRootOptions: () => [],
  projectRoots: () => [],
  codingModeEnabled: false,
  codingModeSettingsBusy: false,
  effort: 'off',
  inputDisabled: false,
  safeSetupAvailable: false,
  floating: false,
  showPresets: false,
})

const emit = defineEmits<{
  beforeinput: [event: InputEvent]
  compositionChange: [value: boolean]
  fileChange: [event: Event]
  input: [event: Event]
  keydown: [event: KeyboardEvent]
  removeAttachment: [index: number]
  retryAttachment: [index: number]
  send: []
  setBusySendMode: [mode: 'queue' | 'steer']
  setRunMode: [mode: SandboxRunMode]
  setModelRoutingMode: [mode: ModelRoutingMode]
  setEffort: [level: EffortLevel]
  setCodingModeEnabled: [enabled: boolean]
  setCollaborationMode: [mode: CollaborationMode]
  setApprovalBypass: [enabled: boolean]
  cancelReplan: []
  voiceInput: []
  voiceSetup: []
  exportMarkdown: []
  stop: []
  chooseProject: []
  closeProject: []
  /** 多选弹窗确认：完整的附加项目 id 集合（不含主项目）。 */
  applyProjectRoots: [ids: string[]]
  /** 移除单个附加项目 chip。 */
  removeProjectRoot: [id: string]
  /** 「+」菜单 → 打开斜杠命令清单 / 产物面板 / 工作区文件树 / 技能页 / 插件页。 */
  openCommands: []
  openArtifacts: []
  openFileTree: []
  openSkills: []
  openPlugins: []
  openPromptCacheKeepalive: []
  refreshPromptCacheKeepalive: []
/** Request the parent to restore the full (expanded) composer. */
  expand: []
  /** P2·E：编排状态补丁（面板单选/开关 → 父级合并）。form 推导在本组件 handler 完成。 */
  updateOrchestration: [patch: Partial<OrchestrationState>]
}>()

const { t } = useI18n()

const inputText = defineModel<string>({ required: true })
const composerEl = ref<HTMLElement | null>(null)
const textareaEl = ref<HTMLTextAreaElement | null>(null)
const fileInputEl = ref<HTMLInputElement | null>(null)
// 附加项目多选弹窗的开合是纯 UI 状态；附加项目集合本身由 ChatView 单例持有。
const projectRootsOpen = ref(false)

function applyProjectRoots(ids: string[]) {
  projectRootsOpen.value = false
  emit('applyProjectRoots', ids)
}
// 输入区浮层是**单槽**状态：同一时刻只允许一个浮层存在。
//
// 此前这里是 7 个独立的 `ref(false)`，「互斥」靠零散 watch 手工维护，只覆盖了
// Add / More / RunMode 三者。后果在真机上可见：先开「执行模式与路由」，再点
// 「模式与能力」，两个面板会同时挂在输入区上方互相叠压、下缘被裁切。
// 改成单槽后互斥是**结构性的**（写入即抢占），不再依赖某处补 watch。
type ComposerPopover = 'add' | 'runMode' | 'more' | 'agent' | 'team' | 'swarm' | 'mode'

const activePopover = ref<ComposerPopover | null>(null)

/** 把一个具名浮层映射成 `v-model:open` 可用的可写布尔。 */
function popoverOpen(name: ComposerPopover) {
  return computed({
    get: () => activePopover.value === name,
    set: (open: boolean) => {
      if (open) {
        activePopover.value = name
      } else if (activePopover.value === name) {
        activePopover.value = null
      }
    },
  })
}

const addMenuOpen = popoverOpen('add')
const runModePanelOpen = popoverOpen('runMode')
const moreActionsOpen = popoverOpen('more')
// P2·E 编排四面板：子智能体指派 / 团队专家 / 蜂群 / 模式能力。
const agentPanelOpen = popoverOpen('agent')
const teamPanelOpen = popoverOpen('team')
const swarmPanelOpen = popoverOpen('swarm')
const modePanelOpen = popoverOpen('mode')
const agentPanelAnchorEl = ref<HTMLElement | null>(null)
const teamPanelAnchorEl = ref<HTMLElement | null>(null)
const swarmPanelAnchorEl = ref<HTMLElement | null>(null)
const modePanelAnchorEl = ref<HTMLElement | null>(null)
// P3 打通：模式矩阵 ↔ 编排面板共享同一全局 3×3 状态源（activeUserMode）。
const { activeUserMode } = useActiveMode()

// ── 「@」引用（对话框内）─────────────────────────────────────────────────
// 输入 `@` 触发候选浮层：文件/目录/规则/历史会话会登记为本轮引用
// （随 `chat.send.contextRefs` 上行，由后端渲染成材料）；专家/智能体只插入
// `@名称` 文本作为角色指向，整队启动仍走专家中心页 team.run。
const mentionPopoverEl = ref<InstanceType<typeof ComposerMentionPopover> | null>(null)
const mentionQuery = ref<string | null>(null)

function updateMentionTrigger() {
  const m = /(^|\s)@([^\s@]*)$/.exec(inputText.value)
  mentionQuery.value = m ? m[2] : null
}
watch(inputText, updateMentionTrigger)

function onTextareaKeydown(event: KeyboardEvent) {
  // IME 组字期间 Enter/Tab/方向键属于输入法（确认候选、翻页），不是「选中 @ 候选」
  // 或「发送」——父级快捷键已挡，但这里的 @ 浮层分支在 emit 之前，必须同样挡掉，
  // 否则中文输入法里打 `@张三` 一按回车就把半截候选塞进输入框。
  if (event.isComposing || event.keyCode === 229) return
  if (mentionQuery.value !== null && mentionPopoverEl.value) {
    if (event.key === 'ArrowDown') {
      event.preventDefault()
      mentionPopoverEl.value.move(1)
      return
    }
    if (event.key === 'ArrowUp') {
      event.preventDefault()
      mentionPopoverEl.value.move(-1)
      return
    }
    if ((event.key === 'Enter' || event.key === 'Tab') && mentionPopoverEl.value.hasCandidates()) {
      event.preventDefault()
      mentionPopoverEl.value.selectActive()
      return
    }
    if (event.key === 'Escape') {
      event.preventDefault()
      mentionQuery.value = null
      return
    }
  }
  emit('keydown', event)
}

function applyMention(payload: { label: string; ref?: ContextRefItem }) {
  const text = inputText.value
  const m = /(^|\s)@([^\s@]*)$/.exec(text)
  if (m) {
    inputText.value = `${text.slice(0, m.index)}${m[1]}@${payload.label} `
  }
  // 文件/目录/规则/会话：登记为本轮引用（发送时随 chat.send.contextRefs 上行，
  // 后端把材料拼在用户消息尾部）。专家/智能体只是角色指向，不入引用负载。
  if (payload.ref) addContextRef(payload.ref)
  mentionQuery.value = null
  void nextTick(() => textareaEl.value?.focus())
}

const contextRefs = useContextRefs()
function removeRef(kind: string, id: string) {
  removeContextRef(kind, id)
}
const promptCacheKeepaliveStatusText = computed(() => {
  const status = props.promptCacheKeepaliveStatus
  if (!status || status.state === 'off') return ''
  return t(`chat.promptCacheKeepalive.states.${status.state}`)
})
const promptCacheKeepaliveAriaLabel = computed(() => {
  const action = t('chat.promptCacheKeepalive.action')
  return promptCacheKeepaliveStatusText.value
    ? `${action}. ${promptCacheKeepaliveStatusText.value}`
    : action
})

// "NEW" badge on the routing control — the single-model AI router is now the
// default, so flag it until the user first opens the control, then never again.
const ROUTER_NEW_BADGE_KEY = 'marguerite.composer.routerNewBadgeSeen'
const routerNewBadgeSeen = ref(false)
try {
  routerNewBadgeSeen.value = localStorage.getItem(ROUTER_NEW_BADGE_KEY) === '1'
} catch { /* localStorage unavailable */ }
const showRouterNewBadge = computed(() => !routerNewBadgeSeen.value)
function dismissRouterNewBadge() {
  if (routerNewBadgeSeen.value) return
  routerNewBadgeSeen.value = true
  try {
    localStorage.setItem(ROUTER_NEW_BADGE_KEY, '1')
  } catch { /* localStorage unavailable */ }
}
const addMenuAnchorEl = ref<HTMLElement | null>(null)
const moreActionsAnchorEl = ref<HTMLElement | null>(null)

const anyPopoverOpen = computed(() => activePopover.value !== null)

// 「@」候选与输入区浮层互斥：单槽状态，直接清空即可。
watch(mentionQuery, (q) => {
  if (q !== null) activePopover.value = null
})

function eventInsideRoot(event: PointerEvent, root: HTMLElement | null): boolean {
  if (!root) return false
  const path = typeof event.composedPath === 'function' ? event.composedPath() : []
  if (path.includes(root)) return true
  return event.target instanceof Node && root.contains(event.target)
}

function closeOpenPopoversFromOutside(event: PointerEvent) {
  if (addMenuOpen.value && !eventInsideRoot(event, addMenuAnchorEl.value)) {
    addMenuOpen.value = false
  }
  if (
    moreActionsOpen.value &&
    !eventInsideRoot(event, moreActionsAnchorEl.value)
  ) {
    moreActionsOpen.value = false
  }
  if (agentPanelOpen.value && !eventInsideRoot(event, agentPanelAnchorEl.value)) {
    agentPanelOpen.value = false
  }
  if (teamPanelOpen.value && !eventInsideRoot(event, teamPanelAnchorEl.value)) {
    teamPanelOpen.value = false
  }
  if (swarmPanelOpen.value && !eventInsideRoot(event, swarmPanelAnchorEl.value)) {
    swarmPanelOpen.value = false
  }
  if (modePanelOpen.value && !eventInsideRoot(event, modePanelAnchorEl.value)) {
    modePanelOpen.value = false
  }
}

watch(anyPopoverOpen, (open) => {
  if (open) {
    document.addEventListener('pointerdown', closeOpenPopoversFromOutside, true)
  } else {
    document.removeEventListener('pointerdown', closeOpenPopoversFromOutside, true)
  }
}, { immediate: true })

onBeforeUnmount(() => {
  document.removeEventListener('pointerdown', closeOpenPopoversFromOutside, true)
})

// 首次展开统一模式面板时消除「NEW」角标（原在路由按钮上）。
// 浮层互斥已由单槽状态保证，这里不再手工关闭其它面板。
watch(runModePanelOpen, (isOpen) => {
  if (isOpen) dismissRouterNewBadge()
})

// 「高级 ▾」：打开既有 RunModePanel（内含全量的 RunMode / PlanMode / ModelRouting）。
function openAdvancedFromPresets() {
  runModePanelOpen.value = true
}

function closeAllPopovers() {
  activePopover.value = null
  mentionQuery.value = null
}

/// 发送按钮：先收起所有浮层再提交。
///
/// 否则蜂群浮层会留在原地盖住刚出现的编排泳道卡片右侧（真机可见），
/// 用户以为「卡片没显示全」，实际是被浮层遮住了。
function submitFromButton() {
  closeAllPopovers()
  emit('send')
}

// The retract animation collapses the footer with overflow clipping; any open
// menu would be clipped/vanished mid-flight, so close menus the moment the
// composer starts collapsing.
watch(() => props.collapsed, (collapsed) => {
  if (collapsed) closeAllPopovers()
})

function toggleMoreActions() {
  const opening = activePopover.value !== 'more'
  activePopover.value = opening ? 'more' : null
  if (
    opening
    && props.promptCacheKeepaliveAvailable
    && props.promptCacheKeepaliveSessionReady
  ) {
    emit('refreshPromptCacheKeepalive')
  }
}

/** 点同一个控件开/关；点另一个控件直接抢占（单槽状态的语义）。 */
function togglePopover(name: ComposerPopover) {
  activePopover.value = activePopover.value === name ? null : name
}

function toggleAddMenu() {
  togglePopover('add')
}

// ── P2·E 编排四面板：互斥由单槽状态保证 ─────────────────────────────
function toggleAgentPanel() {
  togglePopover('agent')
}
function toggleTeamPanel() {
  togglePopover('team')
}
function toggleSwarmPanel() {
  togglePopover('swarm')
}
function toggleModePanel() {
  togglePopover('mode')
}

// ── P2·E 编排状态补丁：面板单选/开关 → updateOrchestration ─────────
// form 由「最后一次选择」即时推导，保证任意时刻 form 与面板状态一致：
//   agents 非空 → subagent；选团队 → team/expert；开蜂群 → swarm；
//   对应面板清空/关闭 → 回落 single（原 chat.send 路径）。
function applyOrchestration(patch: Partial<OrchestrationState>) {
  emit('updateOrchestration', patch)
}

function applyAgents(agents: string[]) {
  const patch: Partial<OrchestrationState> = { agents }
  if (agents.length > 0) {
    patch.form = 'subagent'
  } else if (props.orchestration?.form === 'subagent') {
    patch.form = 'single'
  }
  applyOrchestration(patch)
}

/** 团队多选：点已选项 = 取消；至少选一个后按「是否含专家团」推导 team/expert。 */
function toggleTeam(teamId: string) {
  const current = props.orchestration?.teams ?? []
  const next = current.includes(teamId)
    ? current.filter(id => id !== teamId)
    : [...current, teamId].slice(0, 8)
  applyTeams(next)
}

function clearTeams() {
  applyTeams([])
}

function applyTeams(teams: string[]) {
  if (teams.length === 0) {
    applyOrchestration({ teams: [], form: 'single', confirmed: false })
    return
  }
  const isExpert = teams.some(id => EXPERT_TEAMS.some(t => t.id === id))
  applyOrchestration({
    teams,
    form: isExpert ? 'expert' : 'team',
    confirmed: false,
  })
}

function applySwarmOn(on: boolean) {
  const patch: Partial<OrchestrationState> = { swarmOn: on }
  if (on) {
    patch.form = 'swarm'
  } else if (props.orchestration?.form === 'swarm') {
    patch.form = 'single'
  }
  applyOrchestration(patch)
}

function applySwarmSize(size: number) {
  applyOrchestration({ swarmSize: size })
}

// P3 打通（反向）：外部模式切换（徽标菜单 / ModeMatrix / 路由保存）经
// `session.event.mode.changed` 广播更新 activeUserMode；这里把最新格位回填到
// 编排面板，保证「任一界面操作 → 全工作台一致」。
// 2026-10-24：能力面板不再画 3×3 矩阵（与「模式」弹层重复），档位只在 ModeMatrix 改。
watch(activeUserMode, (um) => {
  if (um?.key && props.orchestration && um.key !== props.orchestration.modeKey) {
    emit('updateOrchestration', { modeKey: um.key })
  }
})

function applyCapabilities(capabilities: string[]) {
  applyOrchestration({ capabilities })
}

function triggerVoice() {
  moreActionsOpen.value = false
  if (props.voiceReady) {
    emit('voiceInput')
  } else {
    emit('voiceSetup')
  }
}

function exportConversation() {
  moreActionsOpen.value = false
  emit('exportMarkdown')
}

function openPromptCacheKeepalive() {
  if (!props.promptCacheKeepaliveSessionReady) return
  moreActionsOpen.value = false
  emit('openPromptCacheKeepalive')
}

function attachmentIcon(att: Attachment): IconName {
  return isImageDisplayAttachment(att) ? 'image' : 'fileText'
}

function attachmentMeta(att: Attachment): string {
  if (att.kind === 'failed') return att.error ? `FAILED · ${att.error}` : 'FAILED'
  const mime = att.mime || ''
  const subtype = mime.includes('/') ? mime.split('/')[1] : mime
  const label = subtype ? subtype.toUpperCase() : 'FILE'
  const size = typeof att.size === 'number'
    ? `${Math.max(1, Math.round(att.size / 1024))} KB`
    : ''
  return [label, size].filter(Boolean).join(' · ')
}

function attachmentTitle(att: Attachment): string {
  if (att.kind === 'failed') {
    return att.error ? `${att.name}: ${att.error}` : `${att.name}: failed`
  }
  return att.name
}

function composerElement(): HTMLElement | null {
  return composerEl.value
}

function canCollapse(): boolean {
  const activeElement = document.activeElement
  return !anyPopoverOpen.value
    && (
      !activeElement
      || activeElement === textareaEl.value
      || !composerEl.value?.contains(activeElement)
    )
}

function documentCanReceiveFocus(): boolean {
  return document.visibilityState === 'visible' && document.hasFocus()
}

function focusTextarea() {
  // Programmatic focus must never reactivate a background/minimized browser
  // window. Recheck inside nextTick because the page can lose focus between
  // scheduling and execution (GitHub issue 382).
  if (!documentCanReceiveFocus()) return
  nextTick(() => {
    if (documentCanReceiveFocus()) textareaEl.value?.focus()
  })
}

function isTextareaFocused(): boolean {
  return document.activeElement === textareaEl.value
}

function resizeTextarea() {
  nextTick(() => {
    const ta = textareaEl.value
    if (!ta) return
    ta.style.height = 'auto'
    ta.style.height = Math.min(ta.scrollHeight, 160) + 'px'
  })
}

defineExpose<ChatComposerExpose>({
  composerElement,
  canCollapse,
  focusTextarea,
  isTextareaFocused,
  resizeTextarea,
})
</script>

<style scoped>
.hidden {
  display: none !important;
}

.chat-composer {
  padding: 0.75rem 1.5rem 1.875rem;
  border-top: 0;
  background: var(--bg-surface);
  flex-shrink: 0;
}

.chat-composer--floating {
  /* No bottom-bar band: leave breathing room around the glass card while the
     disabled preference retains the established docked surface exactly. */
  padding: 0.5rem 1.5rem 1.25rem;
  background: transparent;
}

.chat-composer--new-landing {
  width: min(calc(100% - 48px), 820px);
  margin: 0 auto;
  padding: 0;
  background: transparent;
}

.chat-composer-inner {
  width: min(100%, var(--composer-col, 820px));
  margin: 0 auto;
}

.chat-project-chip {
  flex: 0 1 auto;
  min-width: 0;
  width: fit-content;
  max-width: min(220px, 42vw);
  display: inline-flex;
  align-items: center;
  gap: 5px;
  min-height: 30px;
  padding: 3px 6px;
  border: 0;
  border-radius: var(--radius-control);
  background: transparent;
  color: var(--text-muted);
  font-size: var(--fs-xs);
}
.chat-project-chip > .icon {
  flex: 0 0 auto;
  color: var(--text-dim);
}
.chat-project-chip__name {
  flex: 0 1 auto;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-weight: 400;
}
.chat-project-chip__status {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  color: var(--warning, var(--text-muted));
  font-size: var(--fs-xs);
}
.chat-project-chip button {
  flex: 0 0 auto;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  padding: 2px;
  border: 0;
  border-radius: var(--radius-full);
  background: transparent;
  color: inherit;
  cursor: pointer;
}
.chat-project-chip button:hover,
.chat-project-chip button:focus-visible {
  outline: 0;
  background: var(--bg-hover);
  color: var(--text);
}
.chat-project-chip[data-status="unavailable"],
.chat-project-chip[data-status="removed"],
.chat-project-chip[data-status="error"] {
  background: color-mix(in srgb, var(--warn) 7%, transparent);
}
.chat-coding-mode-chip {
  flex: 0 0 auto;
  min-height: 30px;
  display: inline-flex;
  align-items: center;
  gap: 4px;
  padding: 3px 7px 3px 9px;
  border: 1px solid color-mix(in srgb, var(--accent) 28%, transparent);
  border-radius: var(--radius-full);
  background: color-mix(in srgb, var(--accent) 9%, transparent);
  color: var(--accent);
  font: inherit;
  font-size: var(--fs-xs);
  font-weight: 650;
  line-height: 1;
  cursor: pointer;
  transition:
    border-color var(--dur-fast),
    background var(--dur-fast),
    color var(--dur-fast);
}
.chat-coding-mode-chip:hover,
.chat-coding-mode-chip:focus-visible {
  outline: 0;
  border-color: color-mix(in srgb, var(--accent) 48%, transparent);
  background: color-mix(in srgb, var(--accent) 15%, transparent);
  color: var(--accent-hover);
}
.chat-coding-mode-chip:focus-visible {
  box-shadow: var(--focus-ring);
}
.chat-coding-mode-chip:disabled {
  cursor: default;
  opacity: var(--state-disabled-opacity);
}
.chat-project-choose {
  flex-shrink: 0;
  max-width: 100%;
  display: inline-flex;
  align-items: center;
  gap: 5px;
  min-height: 30px;
  padding: 3px 7px;
  border: 0;
  border-radius: var(--radius-full);
  background: transparent;
  color: var(--text-muted);
  font-size: var(--fs-xs);
  font-weight: 600;
  cursor: pointer;
  transition:
    color var(--dur-fast),
    background var(--dur-fast),
    transform var(--dur-fast);
}
.chat-project-choose > .icon:first-child { color: var(--accent); }
.chat-project-choose__chevron {
  opacity: 0.55;
  transition: opacity var(--dur-fast);
}
.chat-project-choose:hover,
.chat-project-choose:focus-visible {
  color: var(--text);
  background: color-mix(in srgb, var(--accent) 7%, transparent);
}
.chat-project-choose:hover .chat-project-choose__chevron,
.chat-project-choose:focus-visible .chat-project-choose__chevron {
  opacity: 0.9;
}

.chat-composer--new-landing .chat-composer-inner {
  width: 100%;
}

.chat-ai-disclaimer {
  margin: 0.5rem 0 0;
  color: var(--text-muted);
  font-size: var(--fs-xs);
  line-height: 1.5;
  text-align: center;
}

/* Transcript content deliberately scrolls underneath the floating dock. The
   input panel owns its glass surface, but the disclaimer sits outside that
   panel; give the note its own content-sized surface so translated/wrapped
   copy never collides visually with a message passing behind it. */
.chat-composer--floating .chat-collapse-region--disclaimer > .chat-ai-disclaimer {
  width: fit-content;
  max-width: 100%;
  margin-inline: auto;
  padding: 0.25rem 0.75rem;
  border: 1px solid color-mix(in srgb, var(--border) 68%, transparent);
  border-radius: var(--radius-full);
  background: color-mix(in srgb, var(--bg-surface) 92%, transparent);
  box-shadow: 0 8px 20px -16px color-mix(in srgb, var(--text) 32%, transparent);
  -webkit-backdrop-filter: blur(16px) saturate(125%);
  backdrop-filter: blur(16px) saturate(125%);
}

@supports not ((-webkit-backdrop-filter: blur(1px)) or (backdrop-filter: blur(1px))) {
  .chat-composer--floating .chat-collapse-region--disclaimer > .chat-ai-disclaimer {
    background: var(--bg-surface);
  }
}

@media (prefers-reduced-transparency: reduce) {
  .chat-composer--floating .chat-collapse-region--disclaimer > .chat-ai-disclaimer {
    background: var(--bg-surface);
    -webkit-backdrop-filter: none;
    backdrop-filter: none;
  }
}

.chat-composer-send-status {
  margin: 0.5rem 0 0;
  color: var(--warning, var(--text-muted));
  font-size: var(--fs-sm);
  line-height: 1.5;
  text-align: center;
}

.chat-attachments {
  display: flex;
  flex-wrap: wrap;
  gap: 0.375rem;
  margin-bottom: 0.5rem;
}

.attachment-chip {
  display: inline-flex;
  align-items: center;
  gap: 0.375rem;
  padding: 0.25rem 0.5rem;
  max-width: min(100%, 360px);
  background: var(--bg-elevated);
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  font-size: 0.8125rem;
}

.attachment-chip--busy {
  opacity: 0.7;
}

.attachment-chip--failed {
  border-color: color-mix(in srgb, var(--danger) 38%, var(--border));
  background: color-mix(in srgb, var(--danger) 8%, var(--bg-elevated));
}

.attachment-chip--failed .attachment-chip__icon,
.attachment-chip--failed .attachment-chip__meta {
  color: var(--danger);
}

.attachment-chip__icon {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 16px;
  height: 16px;
  color: var(--text-muted);
}

.attachment-chip__thumb {
  width: 16px;
  height: 16px;
  border-radius: var(--radius-sm);
  object-fit: cover;
}

.attachment-chip__spinner {
  width: 12px;
  height: 12px;
  border: 2px solid var(--text-muted);
  border-top-color: transparent;
  border-radius: 50%;
  animation: spin 0.8s linear infinite;
}

.attachment-chip__name {
  font-weight: 500;
  max-width: 150px;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.attachment-chip__meta {
  color: var(--text-dim);
  font-size: 0.6875rem;
  min-width: 0;
  max-width: 150px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.attachment-action {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  flex: 0 0 16px;
  padding: 0;
  width: 16px;
  height: 16px;
  background: none;
  border: none;
  cursor: pointer;
  color: var(--text-muted);
  font-size: 0.875rem;
}

.attachment-action:hover {
  color: var(--text);
}

.chat-input-panel {
  display: flex;
  flex-direction: column;
  min-height: 128px;
  border: 1px solid var(--border-strong);
  border-radius: var(--radius-modal);
  background: var(--bg-surface);
  box-shadow: var(--shadow-xs);
  position: relative;
}

/* Glass is opt-in. ChatComposer is also reused outside ChatView, where an
   opaque panel remains the compatibility-safe default. */
.chat-composer--floating .chat-input-panel {
  background: color-mix(in srgb, var(--bg-surface) 72%, transparent);
  -webkit-backdrop-filter: blur(16px) saturate(140%);
  backdrop-filter: blur(16px) saturate(140%);
  box-shadow: var(--shadow-lg);
}

@supports not ((-webkit-backdrop-filter: blur(1px)) or (backdrop-filter: blur(1px))) {
  .chat-composer--floating .chat-input-panel {
    background: var(--bg-surface);
  }
}

@media (prefers-reduced-transparency: reduce) {
  .chat-composer--floating .chat-input-panel {
    background: var(--bg-surface);
    -webkit-backdrop-filter: none;
    backdrop-filter: none;
  }
}

.chat-replan-draft {
  display: grid;
  grid-template-columns: auto minmax(0, 1fr) auto;
  align-items: center;
  gap: var(--sp-2);
  padding: var(--sp-2) var(--sp-3);
  border-bottom: 1px solid color-mix(in srgb, var(--accent) 24%, var(--border));
  border-radius: var(--radius-modal) var(--radius-modal) 0 0;
  background: color-mix(in srgb, var(--accent) 6%, transparent);
  color: var(--text-muted);
  font-size: var(--fs-xs);
}

.chat-replan-draft__icon {
  display: inline-flex;
  color: var(--accent);
}

.chat-replan-draft__copy {
  min-width: 0;
}

.chat-replan-draft__copy strong {
  margin-right: var(--sp-1);
  color: var(--text);
}

.chat-replan-draft__cancel {
  padding: 2px 7px;
  border: 1px solid transparent;
  border-radius: var(--radius-md);
  background: transparent;
  color: var(--text-muted);
  cursor: pointer;
  font: inherit;
}

.chat-replan-draft__cancel:hover,
.chat-replan-draft__cancel:focus-visible {
  border-color: var(--border);
  background: var(--bg-hover);
  color: var(--text);
  outline: none;
}

.chat-replan-draft__cancel:focus-visible {
  box-shadow: var(--focus-ring);
}

.chat-composer--new-landing .chat-input-panel {
  min-height: 168px;
  border-color: var(--border);
  border-radius: var(--radius-modal);
  box-shadow: var(--shadow-lg);
}

/* Floating-composer retract: collapse to a single-line input with nothing
   else. Every region animates (opacity/height/transform) instead of snapping,
   and the composer's own padding tightens so the bar reads as one slim line. */
.chat-composer {
  transition: padding var(--dur-enter) cubic-bezier(0.4, 0, 0.2, 1);
}

.chat-composer--floating.chat-composer--collapsed {
  padding-bottom: 0.5rem;
}

/* Floating-composer toggle off (docked layout): the panel is a solid surface
   — no glass to read the transcript through. !important beats apple-modern's
   `#app .chat .chat-input-panel:focus-within` id-scoped surface. */
.chat-composer--docked .chat-input-panel {
  background: var(--bg-surface) !important;
  -webkit-backdrop-filter: none !important;
  backdrop-filter: none !important;
}

/* A one-row grid animates to zero without imposing an arbitrary max-height on
   attachments or status copy. Expanded content therefore keeps its natural
   height, including on narrow screens and with many attachments. */
.chat-collapse-region {
  display: grid;
  grid-template-rows: minmax(0, 1fr);
  opacity: 1;
  overflow: hidden;
  transition:
    grid-template-rows var(--dur-enter) cubic-bezier(0.4, 0, 0.2, 1),
    opacity var(--dur-base) var(--ease-out),
    transform var(--dur-enter) cubic-bezier(0.4, 0, 0.2, 1),
    visibility 0s linear;
}

.chat-collapse-region > * {
  min-height: 0;
}

/* Composer menus are positioned above their anchors and must escape the
   animation wrapper while the footer is fully expanded. */
.chat-composer:not(.chat-composer--collapsed) .chat-collapse-region--footer {
  overflow: visible;
}

.chat-composer--collapsed .chat-collapse-region {
  grid-template-rows: minmax(0, 0fr);
  opacity: 0;
  transform: translateY(6px);
  visibility: hidden;
  pointer-events: none;
  overflow: hidden;
  transition:
    grid-template-rows var(--dur-enter) cubic-bezier(0.4, 0, 0.2, 1),
    opacity var(--dur-base) var(--ease-out),
    transform var(--dur-enter) cubic-bezier(0.4, 0, 0.2, 1),
    visibility 0s linear var(--dur-enter);
}

.chat-input-panel {
  /* !important: theme files (apple-modern) declare their own min-height and
     transition with higher specificity; the retract must win either way. */
  transition:
    min-height var(--dur-enter) cubic-bezier(0.4, 0, 0.2, 1),
    border-color var(--dur-base) var(--ease-out),
    box-shadow var(--dur-base) var(--ease-out) !important;
}

.chat-composer--collapsed .chat-input-panel {
  min-height: 0 !important;
}

.chat-textarea {
  transition:
    min-height var(--dur-enter) cubic-bezier(0.4, 0, 0.2, 1),
    max-height var(--dur-enter) cubic-bezier(0.4, 0, 0.2, 1),
    padding var(--dur-enter) cubic-bezier(0.4, 0, 0.2, 1);
}

.chat-composer--collapsed .chat-textarea {
  min-height: 0;
  max-height: 2.5rem;
  padding: 0.4375rem 1rem;
  overflow-y: hidden;
}

.chat-composer--new-landing .chat-input-panel:focus-within {
  border-color: var(--border-focus);
  box-shadow: var(--shadow-xl);
}

.chat-input-footer,
.chat-input-actions {
  display: flex;
  align-items: center;
}

.chat-input-footer {
  justify-content: space-between;
  gap: 0.75rem;
  padding: 0.25rem 0.625rem 0.625rem;
}

.chat-input-actions {
  gap: 0.25rem;
  min-width: 0;
}

.chat-settings-anchor {
  position: relative;
  display: inline-flex;
}

.chat-more-actions-menu {
  position: absolute;
  z-index: 20;
  bottom: calc(100% + 0.5rem);
  left: 0;
  width: max-content;
  min-width: 210px;
  padding: 0.375rem;
  border: 1px solid color-mix(in srgb, var(--border) 72%, transparent);
  border-radius: var(--radius-lg);
  background: color-mix(in srgb, var(--bg-surface) 96%, transparent);
  box-shadow: var(--shadow-lg);
  backdrop-filter: blur(16px);
}

.chat-more-actions-menu button {
  display: flex;
  align-items: center;
  width: 100%;
  min-height: 36px;
  gap: 0.625rem;
  padding: 0.5rem 0.625rem;
  border: 0;
  border-radius: var(--radius-control);
  background: transparent;
  color: var(--text-muted);
  font: inherit;
  font-size: var(--fs-sm);
  text-align: left;
  cursor: pointer;
}

.chat-more-actions-menu button:hover,
.chat-more-actions-menu button:focus-visible,
.chat-more-actions-menu button.is-active {
  outline: 0;
  background: var(--bg-hover);
  color: var(--text);
}

.chat-more-actions-menu button:disabled {
  cursor: default;
  opacity: var(--state-disabled-opacity);
}

.chat-more-actions-menu__copy,
.chat-more-actions-menu__copy small {
  display: block;
}

.chat-more-actions-menu__copy small {
  margin-top: 2px;
  color: var(--text-dim);
  font-size: var(--fs-xs);
}

.chat-more-actions-menu__keepalive-status {
  align-items: center;
  display: flex !important;
  gap: 0.375rem;
}

.chat-more-actions-menu__status-dot {
  width: 0.375rem;
  height: 0.375rem;
  flex: 0 0 auto;
  border-radius: 50%;
  background: var(--text-dim);
}

.chat-more-actions-menu__keepalive-status[data-state='scheduled']
  .chat-more-actions-menu__status-dot,
.chat-more-actions-menu__keepalive-status[data-state='probing']
  .chat-more-actions-menu__status-dot {
  background: var(--accent);
}

.chat-more-actions-menu__keepalive-status[data-state='stopped']
  .chat-more-actions-menu__status-dot {
  background: var(--danger);
}

.chat-input-actions--right {
  flex-shrink: 0;
}

.chat-input-wrap {
  flex: 1;
  min-width: 0;
  display: flex;
  position: relative;
}

.chat-textarea {
  width: 100%;
  min-height: 68px;
  max-height: 160px;
  padding: 1rem 1rem 0.375rem;
  border: 0;
  border-radius: 0;
  background: transparent;
  color: var(--text);
  font-size: 0.9375rem;
  line-height: 1.5;
  resize: none;
  outline: none;
  font-family: inherit;
}

.chat-composer--new-landing .chat-textarea {
  min-height: 108px;
  padding: 1.25rem 1.5rem 0.5rem;
  font-size: 1rem;
}

.chat-textarea:focus {
  border-color: transparent;
  box-shadow: none;
}

.chat-input-panel:focus-within {
  border-color: var(--border-focus);
  box-shadow: var(--shadow-sm);
}

.btn--icon {
  width: 34px;
  height: 34px;
  min-width: 34px;
  min-height: 34px;
  border-radius: var(--radius-full);
  padding: 0;
}

.chat-plus-btn {
  color: var(--text-muted);
}

.btn--ghost.is-active {
  background: color-mix(in srgb, var(--ok) 12%, var(--bg-surface));
  color: var(--ok);
}

/* Voice not configured: keep the button clickable (it routes to setup) but
   dim it so it still reads as "not active"; brighten on hover to invite it. */
.chat-mic--needs-setup {
  opacity: var(--state-disabled-opacity);
}

.chat-mic--needs-setup:hover {
  opacity: 1;
}

.chat-model-routing-btn {
  position: relative;
  border-color: transparent;
  background: transparent;
  color: var(--text-muted);
}

.chat-model-routing-btn__new {
  position: absolute;
  top: -3px;
  right: -5px;
  padding: 1px 4px;
  border-radius: 999px;
  background: var(--accent);
  color: var(--bg-surface);
  font-size: 8px;
  font-weight: 600;
  line-height: 1.25;
  letter-spacing: 0.04em;
  text-transform: uppercase;
  pointer-events: none;
}

.chat-model-routing-btn.btn--ghost:not(:disabled):hover {
  border-color: color-mix(in srgb, var(--accent) 18%, transparent);
  background: color-mix(in srgb, var(--accent) 6%, var(--bg-surface));
  color: var(--accent);
}

.chat-model-routing-btn--off.btn--ghost:not(:disabled):hover {
  border-color: color-mix(in srgb, var(--text-dim) 14%, transparent);
  background: color-mix(in srgb, var(--text-dim) 6%, var(--bg-surface));
  color: var(--text-muted);
}

.chat-model-routing-btn.btn--ghost.is-active {
  border-color: color-mix(in srgb, var(--accent) 24%, transparent);
  background: color-mix(in srgb, var(--accent) 9%, var(--bg-surface));
  color: var(--accent);
}

.chat-model-routing-btn--off.btn--ghost.is-active {
  border-color: color-mix(in srgb, var(--text-dim) 18%, transparent);
  background: color-mix(in srgb, var(--text-dim) 8%, var(--bg-surface));
  color: var(--text-muted);
}

.chat-model-routing-btn--squilla_router.btn--ghost.is-active::after {
  content: "";
  position: absolute;
  left: 12px;
  right: 12px;
  bottom: 6px;
  height: 2px;
  border-radius: var(--radius-full);
  background: color-mix(in srgb, var(--accent) 62%, transparent);
}

.chat-model-routing-btn--llm_ensemble.btn--ghost.is-active {
  border-color: color-mix(in srgb, var(--accent) 30%, transparent);
  background: color-mix(in srgb, var(--accent) 11%, var(--bg-surface));
}

.chat-model-routing-btn--llm_ensemble.btn--ghost.is-active::before,
.chat-model-routing-btn--llm_ensemble.btn--ghost.is-active::after {
  content: "";
  position: absolute;
  bottom: 6px;
  width: 6px;
  height: 2px;
  border-radius: var(--radius-full);
  background: color-mix(in srgb, var(--accent) 62%, transparent);
}

.chat-model-routing-btn--llm_ensemble.btn--ghost.is-active::before {
  left: 10px;
}

.chat-model-routing-btn--llm_ensemble.btn--ghost.is-active::after {
  right: 10px;
}

.chat-run-mode-btn {
  --run-mode-tone: var(--text-muted);
  --run-mode-tint: transparent;
  --run-mode-border: transparent;
  --run-mode-marker: var(--text-dim);
  position: relative;
  border-color: var(--run-mode-border);
  background: var(--run-mode-tint);
  color: var(--run-mode-tone);
}

.chat-run-mode-btn::after {
  content: "";
  position: absolute;
  right: 7px;
  bottom: 7px;
  width: 6px;
  height: 6px;
  border-radius: var(--radius-full);
  background: var(--run-mode-marker);
  box-shadow: 0 0 0 2px var(--bg-surface);
}

.chat-run-mode-btn--safe {
  --run-mode-tone: var(--ok);
  --run-mode-tint: color-mix(in srgb, var(--ok) 12%, var(--bg-surface));
  --run-mode-border: color-mix(in srgb, var(--ok) 34%, transparent);
  --run-mode-marker: var(--ok);
}

.chat-run-mode-btn--full {
  --run-mode-tone: color-mix(in srgb, var(--warn) 72%, var(--text-muted));
  --run-mode-tint: color-mix(in srgb, var(--warn) 5%, var(--bg-surface));
  --run-mode-border: color-mix(in srgb, var(--warn) 18%, transparent);
  --run-mode-marker: color-mix(in srgb, var(--warn-fill) 70%, var(--text-dim));
}

.chat-run-mode-btn.btn--ghost:not(:disabled):hover,
.chat-run-mode-btn.btn--ghost.is-active {
  border-color: var(--run-mode-border);
  background: color-mix(in srgb, var(--run-mode-marker) 16%, var(--bg-surface));
  color: var(--run-mode-tone);
}

.chat-run-mode-btn.is-locked {
  opacity: 0.46;
  filter: grayscale(0.6);
  cursor: default;
}

.chat-run-mode-lock-tip {
  position: absolute;
  bottom: calc(100% + 10px);
  left: 50%;
  z-index: 220;
  width: max-content;
  max-width: min(240px, calc(100vw - 24px));
  padding: 7px 10px;
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  background: var(--bg-elevated);
  color: var(--text);
  box-shadow: var(--shadow-md);
  font-size: var(--fs-xs);
  font-weight: 500;
  line-height: 1.35;
  white-space: nowrap;
  opacity: 0;
  visibility: hidden;
  pointer-events: none;
  transform: translate(-50%, 3px) scale(0.98);
  transform-origin: bottom center;
  transition:
    opacity var(--dur-fast) var(--ease-out),
    transform var(--dur-fast) var(--ease-out),
    visibility 0s linear var(--dur-fast);
}

.chat-run-mode-lock-tip::after {
  content: "";
  position: absolute;
  top: 100%;
  left: 50%;
  width: 7px;
  height: 7px;
  border-right: 1px solid var(--border);
  border-bottom: 1px solid var(--border);
  background: var(--bg-elevated);
  transform: translate(-50%, -4px) rotate(45deg);
}

.chat-run-mode-anchor:hover > .chat-run-mode-lock-tip {
  opacity: 1;
  visibility: visible;
  transform: translate(-50%, 0) scale(1);
  transition-delay: var(--dur-base), var(--dur-base), 0s;
}

.chat-send-btn.btn--primary {
  background: var(--bg-hover);
  color: var(--text-dim);
  border-color: var(--bg-hover);
}

.chat-send-btn.btn--primary:hover {
  background: var(--bg-hover);
  border-color: var(--bg-hover);
}

.chat-send-btn.btn--primary.is-ready {
  background: var(--accent);
  border-color: var(--accent);
  color: var(--accent-foreground);
}

.chat-send-btn.btn--primary.is-ready:hover {
  background: var(--accent-hover);
  border-color: var(--accent-hover);
}

@keyframes spin {
  to { transform: rotate(360deg); }
}

/* Streaming-only controls (Stop, Queue/Steer) ease in/out at turn boundaries
   instead of popping into the action cluster. */
.composer-ctl-enter-active,
.composer-ctl-leave-active {
  transition: opacity var(--dur-fast) var(--ease-out),
              transform var(--dur-fast) var(--ease-out);
}
.composer-ctl-enter-from,
.composer-ctl-leave-to {
  opacity: 0;
  transform: scale(0.9);
}

@media (prefers-reduced-motion: reduce) {
  .chat-composer,
  .chat-collapse-region,
  .chat-input-panel,
  .chat-textarea {
    transition: none !important;
  }

  .chat-run-mode-lock-tip {
    transition: none;
  }

  .composer-ctl-enter-active,
  .composer-ctl-leave-active {
    transition: none;
  }
}

@media (max-width: 768px) {
  .chat-composer {
    padding: 0.5rem 0.75rem;
  }

  .chat-composer--floating {
    padding: 0.5rem 0.75rem calc(0.875rem + env(safe-area-inset-bottom, 0px));
  }

  .chat-composer--floating.chat-composer--collapsed {
    padding-bottom: calc(0.5rem + env(safe-area-inset-bottom, 0px));
  }
}

@media (hover: none) {
  .chat-run-mode-lock-tip {
    display: none;
  }
}
</style>
