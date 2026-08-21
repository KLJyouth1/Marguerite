<template>
  <div
    class="chat"
    :class="{
      'chat--new-landing': isNewChatLanding,
      'chat--meta-setup': Boolean(setupState),
      'chat--drag-over': threadDragOver,
      'chat--plan-questionnaire-open': Boolean(dockedPlanQuestionnaire),
      'chat--composer-floating': composerFxEnabled && !isNewChatLanding,
      'chat--composer-collapsed': composerCollapsed && composerFxEnabled && !isNewChatLanding,
    }"
    @dragenter="onChatDragEnter"
    @dragover="onChatDragOver"
    @dragleave="onChatDragLeave"
    @drop="onChatDrop"
  >
    <div v-if="threadDragOver" class="chat-drop-overlay" role="status" aria-live="polite" aria-atomic="true">
      <div class="chat-drop-overlay__frame" aria-hidden="true"></div>
      <div class="chat-drop-overlay__beacon">
        <span class="chat-drop-overlay__glyph" aria-hidden="true">
          <Icon name="fileText" :size="30" />
          <Icon class="chat-drop-overlay__plus" name="plus" :size="12" />
        </span>
        <span class="chat-drop-overlay__copy">
          <span class="chat-drop-overlay__title">{{ t('chat.dropOverlayTitle') }}</span>
          <span class="chat-drop-overlay__hint">{{ t('chat.dropOverlayHint') }}</span>
        </span>
      </div>
    </div>

    <!-- Chat owns the actions; App owns their stable, in-flow shell position. -->
    <Teleport to="#app-route-header">
      <ChatHeaderActions
        v-if="!isNewChatLanding"
        ref="chatHeaderActionsRef"
        :title="currentChatTitle"
        :session-key="sessionKey"
        :copy-state="sessionCopyState"
        :copy-icon="sessionCopyIcon"
        :copy-live-text="sessionCopyLiveText"
        :deliverable-count="headerDeliverableCount"
        :file-changes-visible="fileChangesVisible"
        :share-mode="shareMode"
        :shareable-message-count="shareableMessageCount"
        :compact-busy="compactInFlight"
        @open-deliverables="openDeliverables"
        @open-file-changes="fileChangesOpen = true"
        @start-share="startShareMode"
        @copy-session-key="onSessionCopyClick"
        @compact-context="requestManualCompact"
      />
    </Teleport>

    <!-- Thread -->
    <div class="chat-body">
      <ParticleField />
      <!-- Share-mode banner stays pinned above the scrolling thread. -->
      <div
        v-if="shareMode"
        ref="shareBannerRef"
        class="chat-share-banner"
        tabindex="-1"
        role="group"
        :aria-label="t('chat.shareSelectedMessages')"
        data-testid="share-banner"
      >
        <span class="chat-share-banner__hint">{{ t('chat.shareBannerHint') }}</span>
        <span class="chat-share-banner__count" role="status" aria-live="polite">{{ t('chat.shareSelectedCount', { count: selectedShareCount }) }}</span>
        <button
          type="button"
          class="chat-share-btn chat-share-btn--save"
          :disabled="selectedShareCount === 0 || shareSaving"
          :title="selectedShareCount === 0 ? t('chat.shareSelectAtLeastOne') : t('chat.shareSavePngHint')"
          @click="saveShareImage"
        >
          <Icon name="download" :size="14" />
          <span>{{ shareSaving ? t('chat.saving') : t('chat.savePng') }}</span>
        </button>
        <button type="button" class="chat-share-btn" :title="t('chat.shareCancelHint')" @click="endShareMode">
          {{ t('common.cancel') }}
        </button>
      </div>
      <div class="chat-thread-shell">
        <ChatForkTransitionOverlay
          v-if="forkTransition"
          :fork="forkTransition"
          @retry="retryForkTransition"
          @return-to-parent="returnToForkParent"
        />
        <div
          ref="threadRef"
          class="chat-thread"
          role="region"
          tabindex="0"
          :aria-label="t('chat.conversation')"
          :aria-busy="isStreaming || forkInFlight"
          @scroll="onThreadScroll"
          @wheel.passive="onThreadWheel"
          @touchmove.passive="markThreadScrollIntent('either')"
          @pointerdown="markThreadScrollIntent('either')"
          @pointermove="onThreadPointerMove"
          @keydown="onThreadScrollKeydown"
        >
        <template v-if="isNewChatLanding">
          <div class="chat-landing-brand" :aria-label="t('chat.newChatBrand')">
            <EmptyStateChips
              :key="landingAgentId"
              :agent-id="landingAgentId"
              :suppressed="landingSuggestionsHidden"
              :disabled="landingSuggestionsDisabled"
              @pick="applyLandingSuggestion"
            />
          </div>
        </template>
        <ChatSessionRecoveryStatus
          v-if="!forkTransition && visibleHistoryRecoveryState"
          :key="`${sessionKey}:history`"
          :state="visibleHistoryRecoveryState"
          @retry="retryHistory"
        />
        <ChatSessionRecoveryStatus
          v-if="!forkTransition && liveRecoveryState"
          :key="`${sessionKey}:live`"
          :state="liveRecoveryState"
          @retry="retryLive"
        />
        <div
          v-if="!forkTransition && showConfirmedEmptySession"
          class="chat-empty"
        >
          {{ t('chat.noMessagesYet') }}
        </div>
        <HistoryLoadSentinel
          v-if="!isNewChatLanding && !forkTransition"
          :scroll-container="threadRef"
          :has-more="historyState.hasMore"
          :loading="historyState.loadingEarlier"
          :blocked="historyState.loading"
          :error="historyState.loadEarlierError"
          :canonical-available="historyState.canonicalAvailable"
          :canonical-complete="historyState.canonicalComplete"
          :cursor="historyState.oldestCursor"
          :session-key="sessionKey"
          @load-earlier="loadEarlierHistory"
          @retry="retryHistory"
        />

        <div
          class="chat-message-surface"
          :class="{ 'chat-message-surface--preview': forkTransition }"
          :inert="forkTransition ? true : undefined"
          :aria-hidden="forkTransition ? 'true' : undefined"
          :data-preview-session="forkTransition?.parentKey"
        >
        <ChatMessageList
          :messages="forkTransition?.previewMessages || renderedMessages"
          :session-key="forkTransition?.parentKey || sessionKey"
          :auth-token="readAuthToken()"
          :artifact-navigation-items="sessionArtifacts"
          :workbench-enabled="workbenchEnabled"
          :share-mode="shareMode"
          :selected-message-ids="selectedShareMessageIds"
          :strip-time-prefix="stripTimePrefix"
          :render-markdown="renderMarkdown"
          :fmt-tok="fmtTok"
          :tool-call-groups="toolCallGroups"
          :is-tool-group-open="isToolGroupOpen"
          :is-tool-item-open="isToolItemOpen"
          :tool-group-status-text="toolGroupStatusText"
          :tool-status-text="toolStatusText"
          :tool-secondary-text="toolSecondaryText"
          :copy-message="copyMessage"
          :download-attachment="downloadAttachment"
          :fork-busy="forkInFlight"
          :plan-action-pending="planCardPendingAction"
          :plan-actions-disabled="planActionsDisabled"
          :is-streaming="isStreaming"
          @fork-conversation="forkConversation"
          @edit-message="editMessage"
          @regenerate-message="regenerateMessage"
          @toggle-share-message="toggleShareMessage"
          @download-artifact="downloadArtifact"
          @open-artifact="openArtifact"
          @toggle-tool-group="toggleToolGroup"
          @toggle-tool-item="toggleToolItem"
          @show-tool-result="showToolResultModal"
          @resolve-interrupt="resolveInterrupt"
          @extend-interrupt="extendInterrupt"
          @clarify-submit="submitClarify"
          @clarify-dismiss="dismissClarify"
          @resume-sandbox="resumeSandbox"
          @plan-implement-current="implementCurrentPlan"
          @plan-implement-new="implementPlanInNewTask"
          @plan-replan="beginPlanRevision"
        >
          <template #router-strip="{ message: msg }">
            <RouterFxStrip v-if="shouldRenderRouterStrip(msg)" :message="msg" />
          </template>
        </ChatMessageList>
        </div>

        <!-- Manual or turn-boundary compaction has no assistant turn to own
             it. Keep one quiet transcript maintenance row instead of a
             floating success card or a second task-like animation. -->
        <div
          v-if="compactStatus.visible && !compactStatus.compactionId"
          class="chat-compaction-event"
          :class="{
            'chat-compaction-event--running': compactStatus.isBusy,
            'chat-compaction-event--failed': compactStatus.tone === 'err',
          }"
          data-testid="compaction-event"
          :data-compaction-id="compactStatus.compactionId"
          :data-status="compactStatus.status"
          :data-source="compactStatus.source"
          :data-durability="compactStatus.durability"
          data-placement="turn-boundary"
          :role="compactStatus.tone === 'err' ? 'alert' : 'status'"
          :aria-live="compactStatus.tone === 'err' ? 'assertive' : 'polite'"
          aria-atomic="true"
        >
          <span class="chat-compaction-event__marker" aria-hidden="true" />
          <span class="chat-compaction-event__title">{{ compactStatus.message }}</span>
          <span v-if="compactStatus.detail" class="chat-compaction-event__detail">
            {{ compactStatus.detail }}
          </span>
        </div>

        <PlanCard
          v-if="currentPlan && !currentPlanInHistory"
          :plan="currentPlan"
          :disabled="planActionsDisabled"
          :pending-action="planCardPendingAction"
          @implement-current="implementCurrentPlan"
          @implement-new="implementPlanInNewTask"
          @replan="beginPlanRevision"
        />

        <!-- Pre-reveal router phase: shown only before the live activity owns
             the turn. Once activity is visible, execution status becomes
             the single primary progress surface. -->
        <RouterFxStrip
          v-if="routerStripReserve"
          class="router-fx-reserve"
          :message="routerStripReserve"
          aria-hidden="true"
        />

        <!-- MetaSkill run cards: preflight checkpoint + progress ribbon,
             grouped per run_id above the live activity area. -->
        <template v-for="runId in metaRuns.ribbonOrder.value" :key="`meta-${runId}`">
          <MetaPreflightCard
            v-if="metaRuns.preflights.value.has(runId)"
            :state="metaRuns.preflights.value.get(runId)!.state"
            :phase="metaRuns.preflights.value.get(runId)!.phase"
            :error-text="metaRuns.preflights.value.get(runId)!.errorText"
            @action="metaRuns.onPreflightAction"
          />
          <MetaRibbon
            v-if="metaRuns.ribbons.value.has(runId)"
            :run="metaRuns.ribbons.value.get(runId)!"
            @action="metaRuns.onRibbonAction"
            @chip-select="metaRuns.onChipSelect"
          />
        </template>

        <!-- Streaming AI message: activity stays open while the turn is live.
             Gateway-marked intermediate text remains in the transcript, while
             gateway-marked answer text streams below the activity boundary. -->
        <!-- No blanket aria-live here: the phase label inside ActivityDisclosure
             is the single live announcement point, so streaming DOM churn (tool
             rows, answer tokens) is not read out mutation-by-mutation. -->
        <div v-if="isStreaming && streamBubble && answerRevealOpen" class="msg-ai" data-history-role="assistant">
          <div class="msg-ai-main">
            <ActivityDisclosure
              default-open
              :lifecycle="liveAnswerPart ? 'answering' : 'working'"
              :step-count="executionDockRun?.status === 'running' ? 0 : liveActivityStepCount"
              :failure-count="liveActivityFailureCount"
              :phase-label="liveActivityPhaseLabel"
              :elapsed-label="streamPhaseElapsed"
              :stale="streamActivityStale"
            >
              <!-- Reasoning remains available as a flat, secondary disclosure,
                   rendered by the same part component settled turns use so the
                   chevron affordance and wording stay consistent; `live`
                   selects the streaming "Thinking · Ns" label. -->
              <ReasoningPart v-if="liveReasoningPart" :part="liveReasoningPart" live />

              <AssistantActivityTimeline
                v-if="
                  liveActivityTimelineItems.length
                  || liveActivityProjection.statusSteps.length
                "
                variant="checklist"
                :projection="liveActivityProjection"
                :timeline-items="liveActivityTimelineItems"
                :state-scope="liveToolStateScope"
                :is-tool-group-open="isToolGroupOpen"
                :is-tool-item-open="isToolItemOpen"
                :tool-group-status-text="toolGroupStatusText"
                :tool-status-text="toolStatusText"
                :tool-secondary-text="toolSecondaryText"
                :tool-elapsed-text="liveToolElapsed"
                @toggle-group="toggleToolGroup"
                @toggle-item="toggleToolItem"
                @show-result="showToolResultModal"
              >
                <template #interrupt="{ part }">
                  <InterruptPart
                    v-if="part.resolution"
                    :part="part"
                    timeline
                    @resolve="resolveInterrupt"
                    @extend="extendInterrupt"
                    @clarify-submit="(fields, request) => submitClarify(fields, request)"
                    @clarify-dismiss="dismissClarify"
                  />
                </template>
              </AssistantActivityTimeline>
            </ActivityDisclosure>

            <!-- The gateway marks text as intermediate or answer. Only the
                 semantic answer span streams below the activity boundary; no
                 timeout or draft heuristic is involved. -->
            <div v-if="liveAnswerPart" class="live-answer">
              <TextPart
                :part="liveAnswerPart"
                :sources="[]"
              />
            </div>
            <span
              v-if="liveAnswerPart && !streamActivityStale"
              class="stream-caret"
              aria-hidden="true"
            />

            <ChatArtifactList
              :artifacts="liveArtifacts"
              :navigation-artifacts="sessionArtifacts"
              :session-key="sessionKey"
              :auth-token="readAuthToken()"
              :prefer-workbench="workbenchEnabled"
              @download="downloadArtifact"
              @open="openArtifact"
            />

          </div>
        </div>

        <!-- Pending controls stay outside the collapsible activity timeline at
             the live edge. A resolution removes this card and leaves its compact
             outcome in chronological history. -->
        <InterruptPart
          v-for="part in livePendingInterruptParts"
          :key="part.key"
          :part="part"
          @resolve="resolveInterrupt"
          @extend="extendInterrupt"
          @clarify-submit="(fields, request) => submitClarify(fields, request)"
          @clarify-dismiss="dismissClarify"
        />

        <!-- Soft long-running banner: content events crossed the high watchdog
             threshold while no backend-deadline-owned phase (tool, approval,
             ensemble) explains it. "Keep waiting" suppresses this silence
             episode; "Interrupt" uses the composer stop path. -->
        <ChatStallNotice
          v-if="stallActive"
          :seconds="stallSeconds"
          @wait="stallWatchdog.dismiss()"
          @interrupt="onStop"
        />

        <!-- Thinking indicator -->
        <div v-if="thinkingVisible && answerRevealOpen" class="msg-ai thinking" role="status" aria-live="polite">
          <div class="msg-ai-main">
            <div class="thinking-status">
              <span class="stream-activity-dot" aria-hidden="true" />
              <span class="thinking-elapsed activity-shimmer" aria-live="off">{{ thinkingText }}</span>
            </div>
          </div>
        </div>

        <!-- Legacy standalone approval / clarify block. The interrupt parts now
             carry these through the fold (InterruptPart over the same cards), so
             this side-list only renders on the foldLiveTurn=0 rollback branch —
             the one-flag kill switch — to avoid a double-render. Kept for one
             release as the rollback lever, mirroring the foldLiveTurn discipline. -->
        <template v-if="foldLiveTurnMode === false">
          <!-- In-thread approval cards: blocked runs ask for a decision here -->
          <ApprovalCard
            v-for="entry in approvalEntries"
            :key="entry.approval.id"
            :approval="entry.approval"
            :resolution="entry.resolution"
            :busy="approvalBusyIds.has(entry.approval.id)"
            :error="entry.error"
            @allow-once="resolveApproval(entry, 'allow-once')"
            @allow-always="resolveApproval(entry, 'allow-always')"
            @deny="resolveApproval(entry, 'deny')"
            @extend="extendInterrupt(entry.approval.id)"
          />

          <!-- In-thread clarify card: pending agent questions render as a form -->
          <ClarifyCard
            v-if="pendingClarify"
            :request="pendingClarify"
            :submitted="clarifySubmitted"
            :busy="clarifyBusy"
            :error="clarifyError"
            @submit="submitClarify"
            @dismiss="dismissClarify"
          />
        </template>
        </div>
        <ConversationMinimap
          v-if="!isNewChatLanding && !shareMode && !forkTransition"
          :messages="renderedMessages"
          :scroll-container="threadRef"
          :strip-time-prefix="stripTimePrefix"
          :session-key="sessionKey"
          :history-has-more="historyState.hasMore"
          @navigate="onHistoryNavigate"
          @navigate-end="onHistoryNavigateEnd"
        />
      </div>
    </div>

    <MetaSkillSetupCard
      v-if="setupState"
      :state="setupState"
      :provider-navigation-pending="metaSetupProviderNavigationPending"
      @confirm="confirmSetup"
      @retry="retrySetup"
      @cancel="cancelSetup"
      @configure="openMetaSetupProviderSettings"
    />
    <!-- Composer dock: positioning context so the slash menu anchors directly
         above the composer in any layout. The new-chat landing centers the
         composer instead of pinning it to the bottom, so the menu must not
         anchor to the chat container's bottom edge. -->
    <div class="chat-composer-dock">
    <!-- Durable execution progress belongs to the work surface, not to the
         transcript. Keeping it immediately above the composer also lets a
         future goal driver reuse this dock across multiple turns. -->
    <Transition name="plan-run-dock">
      <div v-if="executionDockRun" class="plan-run-dock">
        <PlanRunRibbon
          :run="executionDockRun"
          :cancel-busy="planActionPending === 'cancel-run'"
          :disabled="planModeBusy || planActionPending !== null"
          @cancel="cancelActivePlanRun"
          @focus-return="focusComposerAfterPlanRun"
        />
      </div>
    </Transition>
    <!-- Jump-to-latest: floats above the composer once the reader has scrolled up
         off the live edge, so a long streaming answer is never lost below the fold. -->
    <Transition name="jump-latest">
      <button
        v-if="showJumpToLatest"
        type="button"
        class="chat-jump-latest"
        :aria-label="t('chat.jumpToLatest')"
        :title="t('chat.jumpToLatest')"
        @click="jumpToLatest"
      >
        <Icon name="chevronRight" :size="14" class="chat-jump-latest__icon" />
        <span>{{ t('chat.latest') }}</span>
      </button>
    </Transition>
    <!-- Slash command menu -->
    <div v-if="slashOpen" class="chat-slash">
      <div
        v-for="(cmd, i) in filteredSlashCmds"
        :key="cmd.cmd"
        class="chat-slash-item"
        :class="{ 'chat-slash-item--active': i === slashIdx }"
        @click="completeSlashCmd(cmd)"
      >
        <span class="chat-slash-cmd">{{ cmd.cmd }}</span>
        <span
          v-if="cmd.metaStatus === 'needs_setup'"
          class="chat-slash-status"
        >{{ t('chat.metaRuns.needsSetup') }}</span>
        <span class="chat-slash-desc" :title="cmd.desc">{{ cmd.desc }}</span>
      </div>
    </div>

    <PendingQueue
      :items="pendingQueue"
      :max-pending="maxPending"
      :image-blocked-message="queuedImageSendBlockedMessage"
      :steer-available="sameTurnSteerAvailable"
      @clear="clearPendingQueue"
      @edit="editPendingMessage"
      @remove="removePendingChip"
      @steer="steerPendingMessage"
    />

    <div
      v-if="dockedPlanQuestionnaire"
      class="plan-questionnaire-dock"
      @wheel="handlePlanQuestionnaireWheel"
    >
      <ClarifyCard
        :request="dockedPlanQuestionnaire"
        :submitted="clarifySubmitted"
        :busy="clarifyBusy"
        :error="clarifyError"
        :docked="true"
        @submit="submitClarify"
      />
    </div>

    <ChatComposer
      ref="composerRef"
      v-model="inputText"
      :attachments="pendingAttachments"
      :busy-send-mode="busySendMode"
      :has-send-content="composerHasSendContent"
      :is-streaming="isStreaming"
      :can-stop="canStop"
      :stop-targets-plan-run="composerStopsPlanRun"
      :is-new-landing="isNewChatLanding"
      :placeholder="composerPlaceholder"
      :send-button-title="sendButtonTitle"
      :send-blocked-message="composerSendBlockedMessage"
      :input-disabled="Boolean(dockedPlanQuestionnaire) || Boolean(forkTransition)"
      :run-mode="runMode"
      :allowed-run-modes="composerAllowedRunModes"
      :safe-setup-available="composerSafeSetupAvailable"
      :run-mode-locked="runModeLocked"
      :run-mode-lock-message="t('chat.composer.runModeLocked')"
      :model-routing-mode="modelRoutingMode"
      :model-routing-settings-busy="modelRoutingSettingsBusy"
      :coding-mode-enabled="codingModeEnabled"
      :coding-mode-settings-busy="codingModeSettingsBusy"
      :voice-busy="voiceBusy"
      :voice-recording="voiceRecording"
      :voice-ready="voiceReady"
      :project-workspace="activeWorkspace"
      :project-workspace-status="activeWorkspaceStatus"
      :project-status-message="activeProjectStatusMessage"
      :can-close-project="isDraftRoute() && pendingWorkspaceId !== null"
      :can-choose-project="rpc.canChooseProject"
      :plan-mode-available="planUiAvailable"
      :collaboration-mode="collaboration.mode"
      :plan-mode-busy="planModeBusy"
      :plan-mode-disabled="planActionPending !== null"
      :plan-mode-applies-next-turn="planModeAppliesNextTurn"
      :replan-active="replanActive"
      :prompt-cache-keepalive-available="promptCacheKeepaliveAvailable"
      :prompt-cache-keepalive-session-ready="promptCacheKeepaliveSessionReady"
      :prompt-cache-keepalive-status="promptCacheKeepaliveStatus"
      :collapsed="composerCollapsed && composerFxEnabled && !isNewChatLanding"
      :floating="composerFxEnabled && !isNewChatLanding"
      @expand="expandComposer"
      @composition-change="composing = $event"
      @beforeinput="onTextareaBeforeInput"
      @file-change="onFileInputChange"
      @input="onTextareaInput"
      @keydown="onTextareaKeydown"
      @remove-attachment="removeAttachment"
      @retry-attachment="retryAttachment"
      @set-busy-send-mode="busySendMode = $event"
      @set-run-mode="setComposerRunMode"
      @set-model-routing-mode="setComposerModelRoutingMode"
      @set-coding-mode-enabled="setComposerCodingModeEnabled"
      @set-collaboration-mode="setCollaborationMode"
      @cancel-replan="cancelPlanRevision"
      @voice-input="onVoiceInput"
      @voice-setup="onVoiceSetup"
      @export-markdown="exportMarkdown"
      @send="onComposerSend"
      @stop="onComposerStop"
      @choose-project="openProjectPicker"
      @close-project="closeProjectDraft"
      @open-prompt-cache-keepalive="promptCacheKeepaliveOpen = true"
      @refresh-prompt-cache-keepalive="void refreshPromptCacheKeepaliveStatus()"
    />
    <SandboxSetupDialog
      :open="composerSandboxSetupOpen"
      :pending="sandboxSetupPending"
      :outcome="sandboxSetupOutcome"
      @cancel="cancelComposerSandboxSetup"
      @background="runComposerSandboxSetupInBackground"
      @confirm="void confirmComposerSandboxSetup()"
    />
    <ProjectWorkspacePickerDialog
      v-if="rpc.canChooseProject"
      :open="projectPickerOpen"
      :enabled="rpc.canChooseProject"
      :session-key="sessionKey"
      :initial-path="activeWorkspace?.path"
      @close="projectPickerOpen = false"
      @choose="chooseProjectPath"
    />
    </div>

    <ToolResultModal
      :open="toolResultModal.open"
      :title="toolResultModal.title"
      :content="toolResultModal.content"
      :context="toolResultModal.context"
      @close="toolResultModal.open = false"
    />

    <DeliverablesDrawer
      :open="deliverablesOpen"
      :artifacts="sessionArtifacts"
      :session-key="sessionKey"
      :auth-token="readAuthToken()"
      @close="closeDeliverables"
      @download="downloadArtifact"
    />

    <FileChangesDrawer
      :open="fileChangesOpen"
      @close="fileChangesOpen = false"
    />

    <RightSidebar
      :open="rightSidebarOpen"
      :tasks="sidebarTasks"
      :artifacts="sessionArtifacts"
      :changes="sidebarChanges"
      :diagnostics="sidebarDiagnostics"
      :streaming="isStreaming"
      :live-keys="liveKeys"
      :stale-keys="staleKeys"
      @close="rightSidebarOpen = false"
      @open="rightSidebarOpen = true"
      @open-artifact="onSidebarOpenArtifact"
      @open-artifact-system="onSidebarOpenArtifactSystem"
      @retry-artifacts="reloadSidebarArtifacts"
      @compose="focusComposer"
    />

    <!-- Epic A 内联编辑 / 预览覆盖层：从产物列表"在应用内编辑/预览"进入 -->
    <Transition name="aie-overlay">
      <div v-if="editingArtifact" class="aie-overlay" @keydown.esc="editingArtifact = null">
        <div class="aie-overlay__backdrop" @click="editingArtifact = null" />
        <div
          class="aie-overlay__panel"
          role="dialog"
          aria-modal="true"
          aria-label="产物编辑与预览"
        >
          <ArtifactInlineEditor
            :artifact="editingArtifact"
            :rpc="rpc"
            :session-key="currentSessionKey"
            :auth-token="() => rpcAuthToken()"
            :base-origin="baseOrigin"
            @close="editingArtifact = null"
            @saved="onArtifactInlineSaved"
            @open-system="onSidebarOpenArtifactSystem"
          />
        </div>
      </div>
    </Transition>

    <SharePreviewModal
      :open="!!sharePreview"
      :image-url="sharePreview?.url || ''"
      :filename="sharePreview?.filename || ''"
      :theme="shareTheme"
      :copy-supported="copySupported"
      :busy="shareSaving"
      @close="closeSharePreview"
      @download="onShareDownload"
      @copy="onShareCopy"
      @set-theme="onShareSetTheme"
    />

    <PromptCacheKeepaliveDialog
      v-if="promptCacheKeepaliveAvailable"
      :open="promptCacheKeepaliveOpen"
      :session-key="sessionKey"
      @close="promptCacheKeepaliveOpen = false"
      @saved="onPromptCacheKeepaliveSaved"
    />

    <!-- Persistent completion announcer: the live block's role="status" phase
         label unmounts with the block when streaming ends, so on its own the
         settle would never reach a screen reader. This region stays mounted
         across the streaming boundary; it fills when a live turn settles and
         clears when the next turn starts so repeat turns announce again. -->
    <span class="chat-turn-settled-announcer" role="status" aria-live="polite">{{ turnSettledAnnouncement }}</span>
  </div>
</template>

<script setup lang="ts">
import { computed, onMounted, onUnmounted, nextTick, ref, watch, watchEffect } from 'vue'
import { useI18n } from 'vue-i18n'
import { useRouter } from 'vue-router'
import { storeToRefs } from 'pinia'
import { useRpcStore } from '@/stores/rpc'
import { useRpcCall } from '@/composables/useRpc'
import { useAppStore } from '@/stores/app'
import { useSandboxSetupStore } from '@/stores/sandboxSetup'
import { useWorkbenchStore } from '@/workbench/store'
import { usePlatform } from '@/platform'
import ApprovalCard from '@/components/chat/ApprovalCard.vue'
import ActivityDisclosure from '@/components/chat/ActivityDisclosure.vue'
import AssistantActivityTimeline from '@/components/chat/AssistantActivityTimeline.vue'
import ChatArtifactList from '@/components/chat/ChatArtifactList.vue'
import ChatForkTransitionOverlay from '@/components/chat/ChatForkTransitionOverlay.vue'
import ChatHeaderActions from '@/components/chat/ChatHeaderActions.vue'
import PromptCacheKeepaliveDialog from '@/components/chat/PromptCacheKeepaliveDialog.vue'
import DeliverablesDrawer from '@/components/chat/DeliverablesDrawer.vue'
import FileChangesDrawer from '@/components/chat/FileChangesDrawer.vue'
import RightSidebar from '@/components/chat/RightSidebar.vue'
import ArtifactInlineEditor from '@/components/chat/ArtifactInlineEditor.vue'
import type { RsArtifact } from '@/components/chat/RightSidebar.vue'
import ChatComposer from '@/components/chat/ChatComposer.vue'
import ProjectWorkspacePickerDialog from '@/components/ProjectWorkspacePickerDialog.vue'
import ChatMessageList from '@/components/chat/ChatMessageList.vue'
import ChatSessionRecoveryStatus from '@/components/chat/ChatSessionRecoveryStatus.vue'
import ChatStallNotice from '@/components/chat/ChatStallNotice.vue'
import ClarifyCard from '@/components/chat/ClarifyCard.vue'
import ConversationMinimap from '@/components/chat/ConversationMinimap.vue'
import EmptyStateChips from '@/components/chat/EmptyStateChips.vue'
import InterruptPart from '@/components/chat/parts/InterruptPart.vue'
import ReasoningPart from '@/components/chat/parts/ReasoningPart.vue'
import TextPart from '@/components/chat/parts/TextPart.vue'
import MetaPreflightCard from '@/components/chat/MetaPreflightCard.vue'
import MetaRibbon from '@/components/chat/MetaRibbon.vue'
import MetaSkillSetupCard from '@/components/chat/MetaSkillSetupCard.vue'
import PendingQueue from '@/components/chat/PendingQueue.vue'
import PlanCard from '@/components/chat/PlanCard.vue'
import PlanRunRibbon from '@/components/chat/PlanRunRibbon.vue'
import RouterFxStrip from '@/components/chat/RouterFxStrip.vue'
import SharePreviewModal from '@/components/chat/SharePreviewModal.vue'
import SandboxSetupDialog from '@/components/sandbox/SandboxSetupDialog.vue'
import ToolResultModal from '@/components/chat/ToolResultModal.vue'
import Icon from '@/components/Icon.vue'
import HistoryLoadSentinel from '@/components/HistoryLoadSentinel.vue'
import ParticleField from '@/components/ParticleField.vue'
import { useChatApprovals } from '@/composables/chat/useChatApprovals'
import { useChatAttachments } from '@/composables/chat/useChatAttachments'
import { useChatCompaction } from '@/composables/chat/useChatCompaction'
import { useChatComposerShortcuts } from '@/composables/chat/useChatComposerShortcuts'
import { useChatDraftPersistence } from '@/composables/chat/useChatDraftPersistence'
import { useChatElevatedMode } from '@/composables/chat/useChatElevatedMode'
import { useChatFeatureToggles } from '@/composables/chat/useChatFeatureToggles'
import { useChatHistory } from '@/composables/chat/useChatHistory'
import { useChatMarkdownExport } from '@/composables/chat/useChatMarkdownExport'
import { useChatMessageActions } from '@/composables/chat/useChatMessageActions'
import {
  createChatMetaDraftRecovery,
  listServerMetaDrafts,
  queryServerMetaDrafts,
} from '@/composables/chat/useChatMetaDraftRecovery'
import {
  useChatPendingQueue,
} from '@/composables/chat/useChatPendingQueue'
import { useChatShareExport } from '@/composables/chat/useChatShareExport'
import { useMediaQuery } from '@/composables/chat/useMediaQuery'
import {
  fmtTok,
  truncate,
  useChatRenderedMessages,
} from '@/composables/chat/useChatRenderedMessages'
import { useChatRouterDecisionRuntime } from '@/composables/chat/useChatRouterDecisionRuntime'
import { useChatAnswerReveal } from '@/composables/chat/useChatAnswerReveal'
import { useChatRpcEventHandlers } from '@/composables/chat/useChatRpcEventHandlers'
import { useChatRpcSubscriptions } from '@/composables/chat/useChatRpcSubscriptions'
import { useChatSend, type ChatSendOutcome } from '@/composables/chat/useChatSend'
import {
  effectiveComposerRunMode,
} from '@/composables/chat/composerRunMode'
import { useSandboxSetupRecovery } from '@/composables/chat/useSandboxSetupRecovery'
import { useChatStallWatchdog } from '@/composables/chat/useChatStallWatchdog'
import { useArtifactImageLightbox } from '@/composables/chat/useArtifactImageLightbox'
import { useMetaRuns } from '@/composables/chat/useMetaRuns'
import { useMetaSkillSetup } from '@/composables/chat/useMetaSkillSetup'
import { useChatPlans } from '@/composables/chat/useChatPlans'
import {
  shouldCanonicalizeInitialDraftRoute,
  useChatSessionRoute,
} from '@/composables/chat/useChatSessionRoute'
import { useChatRunModePreference } from '@/composables/chat/useChatRunModePreference'
import { useChatSessionBootstrap } from '@/composables/chat/useChatSessionBootstrap'
import { autoSendDraftIsUnchanged } from '@/composables/chat/sessionBootstrapContract'
import { normalizeRunStatus, sessionRunStatus } from '@/composables/chat/useChatRunStatus'
import {
  isSubagentCompletionMessage,
  planActionErrorLabel,
  shouldRenderRouterStrip,
  transcriptCompactionState,
} from '@/composables/chat/chatViewUtils'
import {
  acquireSessionBootstrapAdmission,
  claimSessionBootstrapAdmission,
  optionalSessionRpcCallOptions,
  runModeWriteRpcCallOptions,
  sandboxSetupRpcCallOptions,
} from '@/composables/chat/sessionBootstrapAdmission'
import { useChatSessionRuntime } from '@/composables/chat/useChatSessionRuntime'
import { useChatSessionSubscription } from '@/composables/chat/useChatSessionSubscription'
import {
  useChatSlashCommands,
  type DurableMetaDraft,
} from '@/composables/chat/useChatSlashCommands'
import { useChatStream } from '@/composables/chat/useChatStream'
import { useComposerFloatingPreference } from '@/composables/useComposerFloatingPreference'
import { useChatTextRendering } from '@/composables/chat/useChatTextRendering'
import { useChatUsageWidget } from '@/composables/chat/useChatUsageWidget'
import { useSessionArtifacts } from '@/composables/chat/useSessionArtifacts'
import { useArtifactActions } from '@/composables/chat/useArtifactActions'
import { useVoiceInput } from '@/composables/chat/useVoiceInput'
import { useDocumentEvent } from '@/composables/useDocumentEvent'
import { hasOpenDialogLayer } from '@/composables/useDialogA11y'
import { useToasts } from '@/composables/useToasts'
import { usePartialTextRestore } from '@/composables/chat/usePartialTextRestore'
import { useConfirm } from '@/composables/useConfirm'
import { useProjectWorkspaces } from '@/composables/useProjectWorkspaces'
import {
  createDraftProjectHydrationGuard,
  useActiveProjectWorkspace,
} from '@/composables/useActiveProjectWorkspace'
import { useFreshTaskDraft } from '@/composables/useFreshTaskDraft'
import type {
  Attachment,
  ChatMaintenanceEvent,
  ChatPendingItem,
  ChatRenderedMessage,
  ChatRunStatusSource,
  ChatStreamTimelineItem,
  HiddenControlDispatchResult,
  ToolResultContext,
} from '@/types/chat'
// The fork orchestration helpers moved into useChatForkController; ChatView only
// still owns the generation guard it hands to that controller.
import { createForkTransitionLifetime } from '@/utils/chat/forkTransition'
import { liveToolElapsedText } from '@/utils/chat/liveToolElapsedText'
import type {
  MetaDraftDiscardResponse,
  SessionEventPayload,
  SessionMessagesSnapshotResponse,
} from '@/types/rpc'
import type { ModelRoutingMode } from '@/types/modelRouting'
import {
  isRecognizedSandboxRunMode,
  normalizeSandboxRunMode,
  type SandboxRunMode,
} from '@/types/sandbox'
import type { ChatPart } from '@/types/parts'
// 本地类型与纯函数（已从本文件抽出到 views/chat/，降低单文件装配层体积）
import type {
  RpcAuthPayload,
} from './chat/chat-view.types'
import { activeSnapshot, readAuthToken, useChatViewState } from './chat'
// 控制器阶段（C 阶段）：从 ChatView.vue 装配层抽出的本地编排函数，按域聚合为 controller composable。
import { useChatPromptCacheController } from '@/composables/chat/useChatPromptCacheController'
import { useChatToolResultController } from '@/composables/chat/useChatToolResultController'
import { useChatDragClipboardController } from '@/composables/chat/useChatDragClipboardController'
import { useChatExecutionDockController } from '@/composables/chat/useChatExecutionDockController'
import { useChatComposerController } from '@/composables/chat/useChatComposerController'
import { useChatComposerInputController } from '@/composables/chat/useChatComposerInputController'
import { useChatMetaController } from '@/composables/chat/useChatMetaController'
import { useChatArtifactShareController } from '@/composables/chat/useChatArtifactShareController'
import { useChatForkController } from '@/composables/chat/useChatForkController'
import { useChatRunModeController } from '@/composables/chat/useChatRunModeController'
import { useChatProjectController } from '@/composables/chat/useChatProjectController'
import type {
  CollaborationMode,
  PlanCardAction,
  PlanCardActionTarget,
} from '@/types/plans'
import {
  artifactFromWorkbenchItem,
  createArtifactPreviewWorkbenchItem,
} from '@/workbench/artifactItems'
import {
  artifactUsesWorkbenchPreview,
} from '@/utils/workbench/artifactPreview'
import {
  persistDeferredMetaDraft,
  takeDeferredMetaDrafts,
} from '@/utils/chat/metaDraftOutbox'
import { listPendingMetaDiscards } from '@/utils/chat/metaDiscardOutbox'
import { createHistoryNavigationScrollLock } from '@/utils/chat/historyNavigationScrollLock'
import {
  createComposerRetractionController,
  type ComposerScrollIntent,
} from '@/utils/chat/composerRetraction'
import {
  PENDING_STREAM_TASK_ID,
  STOPPED_STREAM_TASK_ID,
  isCurrentSessionPayload as payloadIsCurrentSession,
} from '@/utils/chat/streamEvents'
import { copyTextWithFallback, shareCopyImageSupported } from '@/utils/browser'
import { useCopyFeedback } from '@/composables/chat/useCopyFeedback'
import { recordSessionNavigationDiag } from '@/utils/chat/sessionNavigationDiag'
import {
  toolCallGroups,
  toolGroupStatusText,
  toolSecondaryText,
  toolStatusText,
} from '@/utils/chat/toolDisplay'
import {
  hasSendableModelInputImageAttachment,
  isSendableAttachment,
} from '@/utils/chat/attachments'
import { isShareableChatMessage } from '@/utils/chat/messageIdentity'
import { agentIdFromSessionKey } from '@/utils/chat/sessionKeys'
import { shouldDisableLandingSuggestions } from '@/utils/chat/landingSuggestions'
import { handoffPlanQuestionnaireWheel } from '@/utils/chat/planQuestionnaireWheel'
import { clearAssistantActivityExpansionState } from '@/utils/chat/activityDisclosureState'
import {
  resolveChatHistoryRecoveryState,
  shouldShowConfirmedEmptySession,
  visibleChatHistoryRecoveryState,
} from '@/utils/chat/sessionLoadState'
import {
  isSemanticActivityStatusStep,
  projectAssistantActivityTimeline,
  splitLiveAssistantTimeline,
} from '@/utils/chat/assistantActivity'

/* ── Types ── (已抽出到 ./chat/chat-view.types) ──────────────────────── */

// 工具结果弹窗状态：首个迁入共享状态层的孤立单元。
const { t } = useI18n()
const viewState = useChatViewState(t)
const {
  toolResultModal,
  inputText,
  composing,
  aborted,
  projectPickerOpen,
  landingPrefilled,
  pendingAutoSend,
  pendingAutoSendSessionKey,
  threadRef,
  composerRef,
  composerCollapsed,
  chatHeaderActionsRef,
  sessionKey,
  promptCacheKeepaliveOpen,
  promptCacheKeepaliveStatus,
  composerRevision,
  autoScroll,
  messages,
  forkTransition,
  lastHeaderRole,
  lastHeaderDay,
  threadDragOver,
  shareMode,
  shareSaving,
  selectedShareMessageIds,
  shareBannerRef,
  sharePreview,
  shareTheme,
  activeRunModeLock,
  composerSandboxSetupOpen,
  runStatus,
  currentEpoch,
  lastStreamSeq,
  activeTaskGroups,
  activeStreamTaskId,
  activeStreamSessionKey,
  pendingSessionIntent,
  pendingForkBeforeMessageId,
  interruptState,
  pendingQueueOwnerContext,
  compactInFlight,
  metaSetupProviderNavigationPending,
  turnSettledAnnouncement,
  executionDockRun,
  deliverablesOpen,
} = viewState

// 工具结果弹窗控制器（C 阶段抽出）。
const { showToolResultModal } = useChatToolResultController(viewState)
// Composer 辅助控制器（C 阶段 Batch 8 安全切片）：scrollToBottom /
// lastUserMessageText / scrollToStepCard / focusPendingApprovalCard。
// 装配在 viewState 解构后、首个引用方（chatStream @~1169）之前。
const {
  scrollToBottom,
  lastUserMessageText,
  scrollToStepCard,
  focusPendingApprovalCard,
} = useChatComposerController(viewState)

/* ── Stores / Router ───────────────────────────────────────────────── */

const rpc = useRpcStore()
const fileChangesOpen = ref(false)
const fileChangesVisible = computed(() => rpc.supportsMethod('workspace.file_changes.list'))

// ── 右侧侧边栏 ──
const RIGHT_SIDEBAR_OPEN_KEY = 'marguerite.chat.rightSidebarOpen'
// Persist across route changes: ChatView must not reset the panel to closed
// on every mount (issue #A — RightSidebar state should survive navigation).
const rightSidebarOpen = ref(localStorage.getItem(RIGHT_SIDEBAR_OPEN_KEY) === '1')
watch(rightSidebarOpen, (open) => {
  try { localStorage.setItem(RIGHT_SIDEBAR_OPEN_KEY, open ? '1' : '0') } catch { /* ignore */ }
})
const sidebarTasks = computed((): Array<{ title: string; detail?: string; status: 'running' | 'done' | 'error' | 'pending'; tools?: string[]; duration?: string }> => {
  const tasks: Array<{ title: string; detail?: string; status: 'running' | 'done' | 'error' | 'pending'; tools?: string[]; duration?: string }> = []
  // Feed the live orchestrator step stream (in-flight tool calls) so the task
  // axis reflects real-time execution, not just committed history.
  for (const tc of chatStream.streamToolCalls.value) {
    tasks.push({
      title: `调用工具：${tc.displayName || tc.name}`,
      status: (tc.isRunning ? 'running' : (tc.isError ? 'error' : 'done')) as 'running' | 'done' | 'error' | 'pending',
      tools: [tc.name],
    })
  }
  for (const msg of messages.value) {
    if (msg.role !== 'assistant') continue
    if (msg.reasoning?.text) {
      tasks.push({ title: '深度思考', status: 'done' as const, detail: String(msg.reasoning.text).slice(0, 100) })
    }
    if (msg.timeline) {
      for (const seg of msg.timeline) {
        const s = seg as Record<string, unknown>
        tasks.push({
          title: String(s.label || s.kind || s.text || '步骤'),
          detail: s.detail ? String(s.detail) : undefined,
          status: String(s.status || 'done') as 'running' | 'done' | 'error' | 'pending',
          duration: s.duration ? String(s.duration) : undefined,
        })
      }
    }
    if (msg.tool_calls) {
      for (const tc of msg.tool_calls) {
        const toolName = String(tc.name || tc.tool_name || tc.tool || 'tool')
        tasks.push({
          title: `调用工具：${toolName}`,
          detail: typeof tc.input === 'object' ? JSON.stringify(tc.input).slice(0, 120) : String(tc.input || ''),
          status: (tc.error ? 'error' : (tc.result ? 'done' : 'running')) as 'running' | 'done' | 'error' | 'pending',
          tools: [toolName],
        })
      }
    }
  }
  return tasks
})
const sidebarChanges = ref<Array<{ path: string; name?: string; additions: number; deletions: number; kind?: 'created' | 'modified' | 'deleted' }>>([])
// 加载文件更改
async function refreshSidebarChanges() {
  if (!rpc.supportsMethod('workspace.file_changes.list')) return
  try {
    const res = await rpc.call<{ changes?: Array<Record<string, unknown>> }>('workspace.file_changes.list', {})
    sidebarChanges.value = (res.changes || []).map(c => {
      const p = String(c.path || '')
      const k = String(c.kind || c.change_type || 'modified')
      return {
        path: p,
        name: p.split(/[\\/]/).pop() || p,
        additions: Number(c.additions || c.lines_added || 0),
        deletions: Number(c.deletions || c.lines_deleted || 0),
        kind: (k === 'created' || k === 'deleted' ? k : 'modified') as 'created' | 'modified' | 'deleted',
      }
    })
  } catch { /* ignore */ }
}
watch(rightSidebarOpen, (open) => { if (open) refreshSidebarChanges() })

function onSidebarOpenArtifact(a: RsArtifact) {
  // 在应用内：打开内联编辑 / 预览覆盖层（markdown/text/csv 可编辑，其余走预览/系统打开）
  const artifact = sessionArtifacts.value.find(art =>
    (art.id && art.id === a.id) ||
    (art.download_url && art.download_url === a.download_url) ||
    (art.name || art.filename || '') === (a.name || '') || art.path === a.path,
  )
  if (artifact) {
    editingArtifact.value = artifact
  }
}

const editingArtifact = ref<RsArtifact | null>(null)
function onArtifactInlineSaved() {
  void reloadSidebarArtifacts()
}

// 编辑器所需的会话标识与同源信息（与预览器一致，走 window 同源）
const currentSessionKey = computed(() => String((route.params as Record<string, unknown>).sessionKey || ''))
const baseOrigin = window.location.origin
function rpcAuthToken(): string {
  const token = (rpc as unknown as { _token?: string })._token
  return token || ''
}

// 双轨打开：用系统程序打开（artifacts.open_in_system，缺实现时回退 Gateway 启动器）
const artifactActions = useArtifactActions({ rpc })
async function onSidebarOpenArtifactSystem(a: RsArtifact) {
  const artifact = sessionArtifacts.value.find(art =>
    (art.id && art.id === a.id) ||
    (art.download_url && art.download_url === a.download_url) ||
    (art.name || art.filename || '') === (a.name || '') || art.path === a.path,
  )
  if (!artifact) return
  const result = await artifactActions.openInSystem(artifact)
  if (!result.launched && result.error) {
    // 没有可用的系统启动器时，退化为应用内预览，避免死路
    if (result.method !== 'gateway-fallback') openArtifact(artifact)
  }
}

// 诊断：消息中引用了产物、但索引侧边栏为空，提示用户重试重载
const sidebarDiagnostics = computed<string[]>(() => {
  if (sessionArtifacts.value.length > 0) return []
  const referenced = new Set<string>()
  for (const msg of messages.value) {
    const refs = (msg as unknown as { artifact_refs?: Array<Record<string, unknown>> }).artifact_refs
    if (Array.isArray(refs)) {
      for (const r of refs) {
        const p = String(r.path || r.name || '')
        if (p) referenced.add(p)
      }
    }
  }
  return Array.from(referenced)
})

async function reloadSidebarArtifacts() {
  try { await loadSessionArtifacts() } catch { /* ignore */ }
}

// 订阅 artifact.updated：后端一旦发出即重拉索引，让内联编辑/系统写入后的产物实时出现。
// 防抖 + 去重：单次保存可能触发多次事件（多分片写入），避免并发重拉打满网关。
let artifactUpdatedTimer: ReturnType<typeof setTimeout> | null = null
const artifactUpdatedKeys = new Set<string>()
// 后端有更新版本、但侧边栏当前行仍指向旧内容时，暂存其 key 用于标 stale；
// 重拉索引后再清除，让状态点回归「就绪」。
const staleKeySet = new Set<string>()
const staleKeys = computed<string[]>(() => Array.from(staleKeySet))
function onArtifactUpdated(payload: { session_key?: string; sessionKey?: string; key?: string; id?: string | number }) {
  const evSession = payload?.session_key || payload?.sessionKey || ''
  if (evSession && evSession !== sessionKey.value) return
  const key = evSession || 'default'
  if (artifactUpdatedKeys.has(key)) return
  artifactUpdatedKeys.add(key)
  // 事件命中侧边栏已有产物 → 先标记 stale，等待重拉。
  const incomingId = payload?.id
  if (incomingId != null && sessionArtifacts.value.some((a) => String(a.id) === String(incomingId))) {
    staleKeySet.add(String(incomingId))
  }
  if (artifactUpdatedTimer) clearTimeout(artifactUpdatedTimer)
  artifactUpdatedTimer = setTimeout(() => {
    artifactUpdatedKeys.clear()
    staleKeySet.clear()
    void reloadSidebarArtifacts()
  }, 250)
}

// 流式期间经 session.event.artifact 出现的产物 key（仅在 isStreaming 时计入），
// 用于驱动侧边栏状态点的「更新中」态。turn 结束后 isStreaming 翻 false，自动清空。
const liveKeys = computed<string[]>(() =>
  isStreaming.value
    ? streamArtifacts.value.map((a) => String(a.id || a.download_url || `${a.path}|${a.name}`))
    : [],
)
const sandboxSetupStore = useSandboxSetupStore()
const {
  ensuring: sandboxSetupPending,
  outcome: sandboxSetupOutcome,
} = storeToRefs(sandboxSetupStore)
// Setup runs before this view's/ancestor children's mounted hooks. Holding the
// admission gate here prevents global onboarding/workspace metadata calls from
// entering the serialized Gateway queue ahead of session recovery.
let releaseOptionalRpcAdmission: (() => void) | null =
  claimSessionBootstrapAdmission()
let optionalRpcAdmissionGeneration = 0
const appStore = useAppStore()
const workbenchStore = useWorkbenchStore()
const artifactImageLightbox = useArtifactImageLightbox()
const platform = usePlatform()
const router = useRouter()
const { pushToast } = useToasts()
const { confirm } = useConfirm()
const projectWorkspaces = useProjectWorkspaces()
const activeProjectWorkspace = useActiveProjectWorkspace()
const draftProjectHydration = createDraftProjectHydrationGuard()
const {
  pendingWorkspaceId,
  boundWorkspaceId,
  activeWorkspace,
  status: activeWorkspaceStatus,
} = activeProjectWorkspace
// 项目校验控制器（activeProjectValidationController / cancelActiveProjectValidation）
// 已迁入 useChatProjectController，于装配层 chatSessionRuntime 之后接线。
const isCompactViewport = useMediaQuery('(max-width: 480px)')
const isDesktopViewport = useMediaQuery('(min-width: 769px)')
const landingAgentId = computed(() => agentIdFromSessionKey(sessionKey.value))
// True when the current draft opened with prefilled composer text (Sessions
// Hub task input); the landing suggestion chips stay out of the way then.
// Holds the prefill text when the Sessions Hub hand-off requested a one-step
// send ("Start task"). Flushed in onMounted once the draft subscription is live
// so the first turn streams into this view. Empty string = nothing pending.

/* ── DOM refs ──────────────────────────────────────────────────────── */

/* Floating-composer retract: a pure controller accumulates slow user travel
   while ignoring scrollTop changes caused by history, minimap, and layout. */
const composerRetraction = createComposerRetractionController()
let pendingComposerScrollIntent: ComposerScrollIntent = null
let composerScrollIntentTimer: number | null = null

// Settings → Appearance "Floating composer" toggle. Off: the composer docks in
// the normal layout and never retracts; on (default): it floats over the
// transcript and collapses to a single line while scrolling up.
const { enabled: composerFxEnabled } = useComposerFloatingPreference()

/* ── State ─────────────────────────────────────────────────────────── */


function clearPendingComposerScrollIntent() {
  pendingComposerScrollIntent = null
  if (composerScrollIntentTimer !== null) {
    window.clearTimeout(composerScrollIntentTimer)
    composerScrollIntentTimer = null
  }
}

function resetComposerRetraction() {
  clearPendingComposerScrollIntent()
  composerCollapsed.value = composerRetraction.reset()
}

function expandComposer() {
  clearPendingComposerScrollIntent()
  composerCollapsed.value = composerRetraction.expand(threadRef.value?.scrollTop ?? null)
}

function markThreadScrollIntent(intent: Exclude<ComposerScrollIntent, null>) {
  pendingComposerScrollIntent = intent
  if (composerScrollIntentTimer !== null) window.clearTimeout(composerScrollIntentTimer)
  // Wheel scrolling can land one frame after its input event. A short token
  // covers that browser scheduling gap; direction matching still rejects a
  // history-prepend correction that moves opposite to the gesture.
  composerScrollIntentTimer = window.setTimeout(() => {
    pendingComposerScrollIntent = null
    composerScrollIntentTimer = null
  }, 120)
}

function currentThreadScrollIntent(): ComposerScrollIntent {
  return pendingComposerScrollIntent
}

watch(composerFxEnabled, resetComposerRetraction, { flush: 'sync' })
watch(sessionKey, resetComposerRetraction, { flush: 'sync' })
const promptCacheKeepaliveAvailable = computed(() => (
  rpc.supportsMethod('sessions.promptCacheKeepalive.status')
  && rpc.supportsMethod('sessions.promptCacheKeepalive.set')
))
const workbenchEnabled = computed(() => appStore.features.artifactWorkbench === true)
const historyNavigationScrollLock = createHistoryNavigationScrollLock(autoScroll)

const forkInFlight = computed(() => (
  forkTransition.value !== null && forkTransition.value.phase !== 'error'
))
const forkTransitionLifetime = createForkTransitionLifetime()

// Session / UI
// Preview-before-download: Save renders the PNG to a blob and opens the modal
// instead of downloading blind. The view owns the object-URL lifecycle.
// Whether the browser can copy an image to the clipboard. Resolved once: the
// capability does not change within a session, and the modal hides Copy when false.
const copySupported = shareCopyImageSupported()

const chatElevatedMode = useChatElevatedMode({
  sessionKey,
})
// Persist the composer draft per session so a refresh / session switch / crash
// before the backend accepts a send cannot silently lose typed text (issue 248).
useChatDraftPersistence({ sessionKey, inputText })
const {
  elevatedMode,
  loadElevatedMode,
  setGlobalElevatedMode,
  normalizeElevatedMode,
} = chatElevatedMode

const {
  runMode: globalRunMode,
  allowedRunModes,
  hydrateRunModePreference,
  setGlobalRunMode,
  applyRunModePreferenceChanged,
} = useChatRunModePreference({
  rpc,
  hydrateCallOptions: optionalSessionRpcCallOptions,
  writeCallOptions: runModeWriteRpcCallOptions,
  runModePolicy: () => {
    const auth = rpc.auth as RpcAuthPayload | null
    return auth?.runModePolicy
  },
})
async function refreshRunModePreference() {
  try {
    await hydrateRunModePreference()
  } catch (cause) {
    console.warn(
      'Failed to hydrate global sandbox run mode:',
      cause instanceof Error ? cause.message : String(cause),
    )
  }
}
const requestedRunMode = computed<SandboxRunMode>(
  () => activeRunModeLock.value ?? globalRunMode.value,
)

const sandboxSetupRecovery = useSandboxSetupRecovery({
  rpc: {
    call: (method, params) =>
      rpc.call(method, params, sandboxSetupRpcCallOptions),
    waitForConnection: () => rpc.waitForConnection(10_000),
  },
  connectionState: computed(() => rpc.state),
  runMode: requestedRunMode,
  autoRefresh: false,
  onUnavailable: async (status) => {
    await platform.settings.reportSandboxUnavailable?.({
      state: status.state,
      ...(status.message ? { message: status.message } : {}),
    })
  },
})
const {
  status: sandboxSetupStatus,
} = sandboxSetupRecovery
const runMode = computed<SandboxRunMode>(() => effectiveComposerRunMode(
  globalRunMode.value,
  sandboxSetupStatus.value,
  activeRunModeLock.value,
  sandboxSetupRecovery.resolved.value,
))
const composerAllowedRunModes = computed<SandboxRunMode[]>(() => {
  if (!sandboxSetupRecovery.resolved.value) {
    return allowedRunModes.value.filter((mode) => mode !== 'safe')
  }
  const status = sandboxSetupStatus.value
  if (
    status !== null
    && status.state !== 'ready'
  ) {
    return allowedRunModes.value.filter((mode) => mode !== 'safe')
  }
  return allowedRunModes.value
})
const composerSafeSetupAvailable = computed(() => sandboxSetupRecovery.canSetup.value)

async function refreshPostBootstrapMetadata() {
  await refreshRunModePreference()
  if (!chatViewDisposed && rpc.state === 'connected') {
    await sandboxSetupRecovery.refresh()
  }
}

// Run status

// Epoch / seq
// Task id whose output the live stream renders; binds late events to the
// current turn so a prior task can't leak into it (issue 344).
let bindActiveStreamTask = (taskId: string) => { activeStreamTaskId.value = taskId }
let restoreLiveTurnSnapshot = (_snapshot: SessionMessagesSnapshotResponse) => {}

// Pending session intent
const freshTaskDraft = useFreshTaskDraft()
const promptCacheKeepaliveSessionReady = computed(() => pendingSessionIntent.value === null)

// Prompt Cache Keepalive 状态控制器（C 阶段抽出）。
const {
  refreshPromptCacheKeepaliveStatus,
  onPromptCacheKeepaliveSaved,
} = useChatPromptCacheController(viewState, {
  rpc,
  promptCacheKeepaliveAvailable,
  promptCacheKeepaliveSessionReady,
})

watch(sessionKey, () => {
  promptCacheKeepaliveStatus.value = null
})

// F1：切回会话时恢复流式 partial_text（若上一轮中断），提示用户生成内容已保留
const { load: loadPartialText } = usePartialTextRestore()
watch(sessionKey, () => {
  void loadPartialText(sessionKey.value).then((partial) => {
    if (partial) {
      pushToast('检测到未完成的生成，正在恢复…', { tone: 'info' })
    }
  })
})

// activeSnapshot(workspace) 已抽出到 ./chat/active-snapshot 并通过顶部 import 引入。
let applySessionRunState: (source: ChatRunStatusSource | null | undefined) => void = () => {}
let resetComposerInputHistory: () => void = () => {}

const chatTextRendering = useChatTextRendering()
const {
  renderMarkdown,
  sanitizeCopyText,
  stripDirectiveTags,
  stripGeneratedArtifactMarkers,
  stripTimePrefix,
} = chatTextRendering

// Resolution side-map for inline interrupt parts, owned here so it can be shared
// between the stream (which threads it into the turn-log fold) and the approvals
// composable (its sole writer). Constructed before the stream because the stream
// reads it at build time; the approvals composable, built later, drives it.

const chatStream = useChatStream({
  messages,
  lastHeaderRole,
  aborted,
  autoScroll,
  runStatus,
  applySessionRunState: source => applySessionRunState(source),
  renderMarkdown,
  stripDirectiveTags,
  stripGeneratedArtifactMarkers,
  scrollToBottom,
  interruptState,
  rpcPolicy: () => rpc.policy,
})
const {
  isStreaming,
  streamArtifacts,
  streamBubble,
  streamHasVisibleOutput,
  streamTimelineItems,
  streamActivityStale,
  streamPhaseLabel,
  streamPhaseElapsed,
  streamToolElapsedText,
  streamIdleTimeoutMs,
  thinkingVisible,
  thinkingText,
  startStreaming,
  resetStreamForRouterReplay,
  resetLiveTurnState: resetStreamLiveTurnState,
  resetStreamIdleTimer,
  setStreamActivity,
  isToolGroupOpen,
  toggleToolGroup,
  isToolItemOpen,
  toggleToolItem,
  cleanup: cleanupStream,
  assertLiveParity,
  useReducer: foldLiveTurnMode,
  foldedTurn,
  appendInterruptFrame,
  ensureInterruptBubble,
} = chatStream
// 实时轮工具耗时：把共享流格式化器绑定到 liveToolElapsedText（归一化整秒），
// 供活动时间线 / 折叠时间线的 tool-elapsed-text 使用。
const liveToolElapsed = (call: { toolId: string }): string =>
  liveToolElapsedText(call, streamToolElapsedText)
const chatAttachments = useChatAttachments()
const {
  pendingAttachments,
  attachmentWorkBusy,
  onFileInputChange,
  addAttachments,
  removeAttachment,
  retryAttachment,
  hasPendingAttachmentWork,
  prepareAttachmentsForSend,
} = chatAttachments
watch(
  [inputText, pendingAttachments],
  () => {
    composerRevision.value += 1
  },
  { deep: true, flush: 'sync' },
)

let sendCurrentInput: () => void = () => {}
let sendAutomaticInput: () => void = () => {}
// Late-bound: dispatchHiddenSend is created below (useChatSend) but the /meta
// slash handler (useChatSlashCommands, created earlier) needs it at call time.
let dispatchHiddenForMeta: (
  providerText: string,
  displayText: string,
  clientRequestId?: string,
  targetSessionKey?: string,
) => Promise<HiddenControlDispatchResult> = (
  _providerText,
  _displayText,
  clientRequestId = '',
  targetSessionKey = '',
) => (
  Promise.resolve({
    status: 'rejected',
    reason: 'invalid_request',
    clientRequestId,
    sessionKey: targetSessionKey || sessionKey.value,
  })
)
let dispatchPlanComposerPrompt: (prompt: string, composerText: string) => void = () => {}
let isCompactInFlightForCurrentSession: () => boolean = () => false
let isQueuedDeliveryBlocked: () => boolean = () => false
let isLiveDeliveryBlocked: () => boolean = () => true
let dispatchQueuedHiddenControl: (
  item: ChatPendingItem,
  ownerSessionKey: string,
) => Promise<ChatSendOutcome> = async () => 'not_sent'
let dispatchQueuedItem: (
  item: ChatPendingItem,
  ownerSessionKey?: string,
) => Promise<ChatSendOutcome> = async () => 'not_sent'
let handleHiddenControlDispatchResult: (result: HiddenControlDispatchResult) => void = () => {}
let discardHiddenControlOutbox: (sessionKey: string, clientRequestId: string) => boolean = () => false
let forgetHiddenControlOutbox: (sessionKey: string, clientRequestId: string) => void = () => {}
const chatPendingQueue = useChatPendingQueue({
  sessionKey,
  ownerContext: pendingQueueOwnerContext,
  inputText,
  pendingAttachments,
  pendingSessionIntent,
  isStreaming,
  isBlocked: () => (
    isCompactInFlightForCurrentSession()
    || isQueuedDeliveryBlocked()
    || isLiveDeliveryBlocked()
    || ['resolving', 'unavailable', 'removed', 'error'].includes(
      activeWorkspaceStatus.value,
    )
    || hasPendingAttachmentWork()
    || pendingQueueOwnerContext.value?.sessionKey === sessionKey.value
  ),
  autoResizeTextarea,
  sendCurrentInput: () => sendCurrentInput(),
  resetInputHistory: () => resetComposerInputHistory(),
  hasComposer: () => Boolean(composerRef.value),
  dispatchHiddenControl: (item, ownerSessionKey) =>
    dispatchQueuedHiddenControl(item, ownerSessionKey),
  onHiddenControlDispatchResult: (result) => {
    if (result.reason === 'discarded') {
      const discardPersisted = discardHiddenControlOutbox(
        result.sessionKey,
        result.clientRequestId,
      )
      if (!discardPersisted) {
        pushToast(t('chat.metaRuns.cancelNotSaved'), { tone: 'danger' })
        return false
      }
    }
    handleHiddenControlDispatchResult(result)
    return true
  },
  dispatchPendingItem: (item, ownerSessionKey) =>
    dispatchQueuedItem(item, ownerSessionKey),
})
const {
  pendingQueue,
  canQueueMore,
  busySendMode,
  maxPending,
  enqueuePendingInput,
  enqueuePendingPayload,
  enqueueRecoveredInput,
  enqueueHiddenControl,
  enqueuePendingSteerRetry,
  removePendingChip,
  beginPendingDelivery,
  settlePendingDelivery,
  clearPendingQueue,
  switchPendingQueue,
  adoptPendingQueue,
  popPendingTail,
  popAllPendingIntoComposer,
  schedulePendingDrainAfterTerminal,
  flushDeferredPendingDrain,
  cleanup: cleanupPendingQueue,
} = chatPendingQueue
watch(attachmentWorkBusy, (busy) => {
  if (!busy) flushDeferredPendingDrain()
})

// Composer 输入 / 发送控制器（C 阶段 send/input 簇）。sendCurrentInput 迟绑定缝
// 与晚声明 computed 经 getter-dep 延后读取，详见控制器文件头注释。
const {
  sendComposerText,
  applyLandingSuggestion,
  appendComposerText,
} = useChatComposerInputController(viewState, {
  getSendCurrentInput: () => sendCurrentInput,
  enqueuePendingPayload: chatPendingQueue.enqueuePendingPayload,
  schedulePendingDrainAfterTerminal,
  flushDeferredPendingDrain,
  pushToast,
  getIsCompactInFlightForCurrentSession: () => isCompactInFlightForCurrentSession(),
  getComposerSendBlockedMessage: () => composerSendBlockedMessage.value,
  getLandingSuggestionsDisabled: () => landingSuggestionsDisabled.value,
  pendingAttachments,
  isStreaming,
})

function restoreMetaLaunchDraft(launchText: string, targetSessionKey: string): void {
  const restored = String(launchText || '').trim()
  const target = String(targetSessionKey || '').trim()
  if (!restored || !target) return
  if (target !== sessionKey.value) {
    if (!persistDeferredMetaDraft({ sessionKey: target, launchText: restored })) {
      pushToast(t('chat.metaRuns.couldNotRunSkill', { skill: restored.split(/\s+/, 3)[1] || 'MetaSkill' }), {
        tone: 'danger',
      })
    }
    return
  }

  const currentDraft = inputText.value.trim()
  if (!currentDraft) {
    inputText.value = restored
    autoResizeTextarea()
    nextTick(() => composerRef.value?.focusTextarea())
    return
  }
  if (currentDraft === restored) return
  if (!enqueueRecoveredInput(restored)) {
    // Preserve the newer composer verbatim. A durable deferred copy is safer
    // than concatenating two independently sendable requests into one turn.
    persistDeferredMetaDraft({ sessionKey: target, launchText: restored })
  }
}

function restoreDeferredMetaDrafts(
  targetSessionKey: string,
  skipLaunchTexts: ReadonlySet<string> = new Set(),
): void {
  if (sessionKey.value !== targetSessionKey) return
  for (const launchText of takeDeferredMetaDrafts(targetSessionKey)) {
    if (skipLaunchTexts.has(launchText)) continue
    restoreMetaLaunchDraft(launchText, targetSessionKey)
  }
}

const chatCompaction = useChatCompaction({
  sessionKey,
  schedulePendingDrainAfterTerminal,
  popAllPendingIntoComposer,
})
const {
  compactStatus,
  getCompactionPlacement,
  setCompactInFlight,
  hideCompactStatus,
  showCompactStatus,
  showCompactionToast,
  cleanup: cleanupCompaction,
} = chatCompaction
isCompactInFlightForCurrentSession = chatCompaction.isCompactInFlightForCurrentSession

// 手动压缩（B2）：调用 context.compact（LLM 语义摘要优先、确定性回退）。
// 服务端推送 session.event.compaction started/completed 事件由事件处理器接管 UI 行；
// 网络失败无事件时手动释放 in-flight，避免按钮永久禁用。
async function requestManualCompact() {
  if (compactInFlight.value || !sessionKey.value) return
  if (chatCompaction.isCompactInFlightForCurrentSession()) return
  compactInFlight.value = true
  chatCompaction.setCompactInFlight(true, sessionKey.value)
  try {
    await rpc.call<{ compacted?: boolean; mode?: string }>('context.compact', {
      sessionKey: sessionKey.value,
    })
  } catch {
    chatCompaction.setCompactInFlight(false)
  } finally {
    compactInFlight.value = false
  }
}

// Standalone compaction has no assistant turn to own it. Anchor its lifecycle
// in the transcript at first observation and update that same row by id, so a
// later user turn cannot make the maintenance boundary drift down the page.
watch(compactStatus, (status) => {
  const compactionId = String(status.compactionId || '').trim()
  if (!status.visible || !compactionId) return
  const maintenance: ChatMaintenanceEvent = {
    kind: 'context_compaction',
    compactionId,
    source: status.source || 'manual',
    state: transcriptCompactionState(status.status),
    durability: status.durability || '',
    ...(status.detail ? { detail: status.detail } : {}),
  }
  const index = messages.value.findIndex(message => (
    message.role === 'maintenance'
    && message.maintenance?.kind === 'context_compaction'
    && message.maintenance.compactionId === compactionId
  ))
  if (index >= 0) {
    const previous = messages.value[index]!
    messages.value.splice(index, 1, { ...previous, maintenance })
    return
  }
  messages.value.push({
    role: 'maintenance',
    text: '',
    ts: Date.now(),
    clientId: `live-maintenance:context-compaction:${compactionId}`,
    maintenance,
  })
}, { flush: 'sync' })

const chatUsageWidget = useChatUsageWidget({
  rpc,
  readCallOptions: optionalSessionRpcCallOptions,
  sessionKey,
  tokenVizEnabled: () => appStore.features.tokenViz,
})
const {
  usageAccum,
  usageModel,
  resetSavingsPopupCooldown,
  saveWidgetState,
  restoreWidgetState,
  loadCurrentSessionUsage,
} = chatUsageWidget

const chatFeatureToggles = useChatFeatureToggles({
  rpc,
  readCallOptions: optionalSessionRpcCallOptions,
  setGlobalElevatedMode,
  loadCurrentSessionUsage,
})
const {
  routerSlots,
  routerModels,
  routerEnabled,
  modelRoutingMode,
  modelRoutingSettingsBusy,
  routerVisualEffectsEnabled,
  routerVisualMode,
  codingModeEnabled,
  codingModeSettingsBusy,
  routerTierConfigs,
  loadFeatureToggles,
  setModelRoutingMode,
  setCodingModeEnabled,
  bindFeatureRefresh,
} = chatFeatureToggles
isQueuedDeliveryBlocked = () => (
  modelRoutingSettingsBusy.value
  && hasSendableModelInputImageAttachment(pendingQueue.value[0]?.attachments || [])
)
watch(
  [modelRoutingMode, modelRoutingSettingsBusy],
  ([mode, busy], [previousMode, wasBusy]) => {
    const routingUnblocked = (
      ((previousMode === 'llm_ensemble' || previousMode === 'bloom_ensemble')
        && mode !== 'llm_ensemble' && mode !== 'bloom_ensemble')
      || (wasBusy && !busy)
    )
    if (!routingUnblocked || pendingQueue.value.length === 0) return
    schedulePendingDrainAfterTerminal()
    flushDeferredPendingDrain()
  },
)

const chatRouterDecisionRuntime = useChatRouterDecisionRuntime({
  messages,
  sessionKey,
  isStreaming,
  autoScroll,
  modelRoutingMode,
  streamBubble,
  streamHasVisibleOutput,
  startStreaming,
  resetStreamForRouterReplay,
  resetStreamIdleTimer,
  setStreamActivity,
  scrollToBottom,
})
const {
  pendingDecision,
  handleRouterControlReplay,
  queueRouterDecision,
  appendEnsembleProgress,
  markEnsembleHandoff,
  flushPendingRouterDecision,
  clearPendingRouterDecision,
} = chatRouterDecisionRuntime

// Gate the live answer's reveal to a [MIN,MAX] window so the model-router panel
// decides (and animates) first, then the answer follows. Self-cleans via the
// composable's onScopeDispose.
const { answerRevealOpen, revealNow } = useChatAnswerReveal({
  isStreaming,
  routerEnabled,
  routerVisualEffectsEnabled,
  routerDecided: () => pendingDecision.value,
})

const chatSessionRoute = useChatSessionRoute(sessionKey)
const {
  route,
  createSessionKey,
  draftAgentId,
  goToDraft,
  hasLegacyNewChatQuery,
  isDraftRoute,
  persistSession,
  readProjectFromUrl,
  readSessionFromUrl,
  resolveInitialSession,
} = chatSessionRoute

let switchToPlanSession: (key: string) => void | Promise<unknown> = () => {}
let planMutationAccepted: () => void = () => {}
const chatPlans = useChatPlans({
  rpc,
  sessionKey,
  currentEpoch,
  isStreaming,
  inputText,
  createSessionKey,
  agentId: () => agentIdFromSessionKey(sessionKey.value),
  switchToSession: key => switchToPlanSession(key),
  focusComposer: () => composerRef.value?.focusTextarea(),
  notifyError: message => pushToast(
    planActionErrorLabel(message),
    { tone: 'danger', duration: 8000 },
  ),
  onMutationAccepted: () => planMutationAccepted(),
  isDraft: () => isDraftRoute() && pendingSessionIntent.value === 'new_chat',
})
const {
  collaboration,
  initialCollaborationMode,
  currentPlan,
  currentPlanRevisionId,
  activePlanRun,
  modeBusy: planModeBusy,
  modeAppliesNextTurn: planModeAppliesNextTurn,
  pendingAction: planActionPending,
  replanTarget,
  replanActive,
} = chatPlans

const renderSourceMessages = computed(() => messages.value)
const chatRenderedMessages = useChatRenderedMessages({
  messages: renderSourceMessages,
  interruptState,
  sessionKey,
  routerSlots,
  routerModels,
  routerTierConfigs,
  routerVisualEffectsEnabled,
  routerVisualMode,
  modelRoutingMode,
  isStreaming,
  currentPlanRevisionId,
  renderMarkdown,
  stripGeneratedArtifactMarkers,
  stripTimePrefix,
  isSubagentCompletionMessage,
})
const { renderedMessages, routerDecisionCells } = chatRenderedMessages

/**
 * Pre-decision router-strip placeholder. It holds the strip's slot for the short
 * window before the first router_decision / ensemble_progress lands, so the live
 * ensemble strip appears from the very start of the turn rather than popping in.
 */
const routerStripReserve = computed<ChatRenderedMessage | null>(() => {
  if (!isStreaming.value || !routerEnabled.value || !routerVisualEffectsEnabled.value) return null
  const rendered = renderedMessages.value
  const liveTurnKey = [...rendered]
    .reverse()
    .find(message => message.displayRole === 'user')
    ?.turnKey
  for (let i = rendered.length - 1; i >= 0; i--) {
    const msg = rendered[i]
    if (msg.isRouterStrip && (!liveTurnKey || msg.turnKey === liveTurnKey)) return null
    if (msg.displayRole === 'user' && msg.turnKey !== liveTurnKey) break
  }
  if (modelRoutingMode.value === 'llm_ensemble' || modelRoutingMode.value === 'bloom_ensemble') {
    return {
      id: 'router-strip-reserve',
      role: 'router',
      displayRole: 'router',
      roleLabel: 'Router',
      text: '',
      timeStr: '',
      showHeader: false,
      isRouterStrip: true,
      routerState: 'pending',
      routerSource: modelRoutingMode.value,
      routerStatic: false,
      routerPanel: 'llm-ensemble',
      routerMode: modelRoutingMode.value,
      gridCells: [],
      winnerIdx: -1,
    }
  }

  const cells = routerDecisionCells({ tier: '', model: '' })
  if (cells.length <= 1) return null
  return {
    id: 'router-strip-reserve',
    role: 'router',
    displayRole: 'router',
    roleLabel: 'Router',
    text: '',
    timeStr: '',
    showHeader: false,
    isRouterStrip: true,
    routerState: 'pending',
    routerSource: 'none',
    routerStatic: false,
    routerPanel: routerVisualMode.value === 'legacy_grid' ? 'legacy-grid' : 'real-candidates',
    gridCells: cells,
    winnerIdx: -1,
  }
})

const aiGeneratedLabel = computed(() => t('chat.aiGeneratedLabel'))

const chatShareExport = useChatShareExport({
  threadRef,
  title: shareTitle,
  aiGeneratedLabel: () => aiGeneratedLabel.value,
})

const preserveHistoryLiveTail = computed(() =>
  isStreaming.value || ['queued', 'running', 'approval_pending'].includes(runStatus.value.status),
)

const chatHistory = useChatHistory({
  rpc,
  sessionKey,
  messages,
  threadRef,
  lastHeaderRole,
  lastHeaderDay,
  preserveLiveTail: preserveHistoryLiveTail,
  autoScroll,
  stripTimePrefix,
  scrollToBottom,
})
const {
  historySessionKey,
  historyState,
  loadHistory,
  loadEarlierHistory,
  retryHistory: retryHistoryRequest,
  scheduleHistorySync,
  cancelAnchorStabilization,
  cancelActiveHistory,
  cleanup: cleanupHistory,
} = chatHistory
planMutationAccepted = () => scheduleHistorySync()

// The durable artifact index fills gaps left by the bounded/compacted message
// history. History and the in-flight ArtifactEvent stream remain live fallback
// sources for mixed-version gateways and list-refresh races.
const chatSessionArtifacts = useSessionArtifacts({
  rpc,
  sessionKey,
  messages,
  streamArtifacts,
})
const {
  artifacts: sessionArtifacts,
  load: loadSessionArtifacts,
  loadAfterReconnect: loadSessionArtifactsAfterReconnect,
  reset: resetSessionArtifacts,
  cleanup: cleanupSessionArtifacts,
} = chatSessionArtifacts

const voiceInput = useVoiceInput()
const {
  voiceBusy,
  voiceRecording,
  toggleVoiceInput,
  cleanup: cleanupVoiceInput,
} = voiceInput

// Gate the composer mic button on real transcription readiness. onboarding.status
// resolves whether audio is enabled AND an ElevenLabs key is present server-side
// (including env-var keys the browser can't see), so audioConfigured is a true
// "voice will work" signal — this keeps the button from being clicked into a
// guaranteed failure. It's the same snapshot the empty-state chips read.
const voiceCapability = useRpcCall<{ audioConfigured?: boolean }>(
  'onboarding.status',
  undefined,
  { callOptions: optionalSessionRpcCallOptions },
)
const voiceReady = computed(() => voiceCapability.data.value?.audioConfigured === true)

const chatMessageActions = useChatMessageActions({
  messages,
  inputText,
  isStreaming,
  sanitizeCopyText,
  stripTimePrefix,
  autoResizeTextarea,
  sendCurrentInput: () => sendCurrentInput(),
  focusComposer: () => composerRef.value?.focusTextarea(),
  pendingForkBeforeMessageId,
  aiGeneratedLabel: () => aiGeneratedLabel.value,
  canDeliver: () => !composerSendBlockedMessage.value,
  notifyDeliveryBlocked: () => {
    if (liveSendBlockedReason.value) {
      pushToast(liveSendBlockedReason.value, { tone: 'info' })
    }
  },
  notifyMessagePending: () => pushToast(t('chat.toast.messageStillSaving'), { tone: 'info' }),
  notifyEditBlocked: () => pushToast(t('chat.pending.editWhileStreaming'), { tone: 'info' }),
})
const {
  copyMessage,
  regenerateMessage,
  editMessage,
} = chatMessageActions

let applyPendingUserInputSnapshot: typeof chatPlans.applyBootstrap = () => {}
const chatSessionSubscription = useChatSessionSubscription({
  rpc,
  sessionKey,
  lastStreamSeq,
  runStatus,
  isStreaming,
  hasActiveInterrupt: computed(() =>
    Array.from(interruptState.value.values()).some(state => !state.resolution)),
  activeStreamTaskId,
  activeTaskGroups,
  sessionRunStatus,
  startStreaming,
  loadHistory,
  resetStreamIdleTimer,
  resetStreamLiveTurnState,
  onLiveSnapshot: snapshot => restoreLiveTurnSnapshot(snapshot),
  onAuthoritativeIdle: () => {
    if (pendingQueueOwnerContext.value?.sessionKey !== sessionKey.value) {
      activeRunModeLock.value = null
    }
    const taskId = activeStreamTaskId.value
    if (
      taskId
      && taskId !== PENDING_STREAM_TASK_ID
      && taskId !== STOPPED_STREAM_TASK_ID
    ) {
      schedulePendingDrainAfterTerminal()
    }
  },
  onRunModeLock: lock => {
    if (lock.locked === false) return
    if (isRecognizedSandboxRunMode(lock.runMode)) {
      activeRunModeLock.value = normalizeSandboxRunMode(lock.runMode)
    } else if (activeRunModeLock.value === null) {
      activeRunModeLock.value = globalRunMode.value
    }
  },
  beginSessionMetadataResolution: key =>
    pendingSessionIntent.value === 'new_chat'
      ? -1
      : activeProjectWorkspace.beginSessionResolution(key),
  onSessionMetadata: (key, generation, metadata) => {
    if (generation < 0) return
    activeProjectWorkspace.applySessionSnapshot(key, generation, metadata)
  },
  onSessionMetadataError: (key, generation) => {
    if (generation < 0) return
    activeProjectWorkspace.failSessionResolution(key, generation)
  },
  onSnapshot: snapshot => {
    chatPlans.applyBootstrap(snapshot)
    applyPendingUserInputSnapshot(snapshot)
  },
})
const {
  subscribeSession,
  unsubscribeSession,
  cancelActiveSubscription,
} = chatSessionSubscription
applySessionRunState = chatSessionSubscription.applySessionRunState

const chatSessionBootstrap = useChatSessionBootstrap({
  sessionKey,
  loadHistory: async (context, retry) => (
    retry
      ? await retryHistoryRequest(context)
      : await loadHistory({}, context)
  ),
  subscribeSession,
  cancelHistory: cancelActiveHistory,
  cancelSubscription: cancelActiveSubscription,
  unsubscribeSession,
})
const {
  livePhase,
  startSessionBootstrap: startSessionBootstrapCoordinator,
  cancelSessionBootstrap: cancelSessionBootstrapCoordinator,
  retryHistory: retryHistoryCoordinator,
  retryLive: retryLiveCoordinator,
  handleConnectionState: handleSessionConnectionStateCoordinator,
  isSessionBootstrapCurrent,
} = chatSessionBootstrap

function holdOptionalRpcAdmission() {
  if (!releaseOptionalRpcAdmission) {
    releaseOptionalRpcAdmission = acquireSessionBootstrapAdmission()
  }
  return ++optionalRpcAdmissionGeneration
}

function releaseOptionalRpcAdmissionAfter(
  promises: readonly Promise<unknown>[],
  admissionGeneration: number,
) {
  void Promise.allSettled(promises).then(() => {
    if (admissionGeneration !== optionalRpcAdmissionGeneration) return
    releaseOptionalRpcAdmission?.()
    releaseOptionalRpcAdmission = null
  })
}

function trackSessionBootstrapAdmission<T extends {
  criticalRequestsQueued: Promise<void>
}>(run: T): T {
  const admissionGeneration = holdOptionalRpcAdmission()
  releaseOptionalRpcAdmissionAfter(
    [run.criticalRequestsQueued],
    admissionGeneration,
  )
  return run
}

let postBootstrapMetadataStarted = false
function schedulePostBootstrapMetadata(
  run: {
    generation: number
    criticalRequestsQueued: Promise<void>
  },
  key: string,
) {
  if (postBootstrapMetadataStarted) return
  void run.criticalRequestsQueued.then(() => {
    if (
      postBootstrapMetadataStarted
      || chatViewDisposed
      || sessionKey.value !== key
      || !isSessionBootstrapCurrent(run.generation, key)
    ) return
    postBootstrapMetadataStarted = true
    void refreshPostBootstrapMetadata()
    void loadFeatureToggles().then(() => {
      if (!chatViewDisposed) unsubs.push(bindFeatureRefresh(scheduleHistorySync))
    })
    loadSlashCommands()
  })
}

function startSessionBootstrap(options?: {
  includeHistory?: boolean
  force?: boolean
}) {
  const key = sessionKey.value
  const run = trackSessionBootstrapAdmission(
    startSessionBootstrapCoordinator(options),
  )
  schedulePostBootstrapMetadata(run, key)
  return run
}

function retryHistory() {
  return retryHistoryCoordinator()
}

function retryLive() {
  return retryLiveCoordinator()
}

function cancelSessionBootstrap() {
  optionalRpcAdmissionGeneration += 1
  cancelSessionBootstrapCoordinator()
}

function handleSessionConnectionState(
  state: string,
  includeHistory = true,
) {
  const run = handleSessionConnectionStateCoordinator(state, includeHistory)
  if (
    run
    && (historyState.value.initialLoadStatus === 'loading'
      || livePhase.value === 'connecting')
  ) {
    return trackSessionBootstrapAdmission(run)
  }
  return run
}

const isSessionHydrating = computed(() => livePhase.value === 'connecting')
const liveSendBlockedReason = computed<string | null>(() => {
  if (!sessionKey.value || livePhase.value === 'ready') return null
  return t(
    livePhase.value === 'degraded'
      ? 'chat.liveSendBlockedDegraded'
      : 'chat.liveSendBlockedConnecting',
  )
})
isLiveDeliveryBlocked = () => Boolean(liveSendBlockedReason.value)
watch(livePhase, (phase, previousPhase) => {
  if (
    phase !== 'ready'
    || previousPhase === 'ready'
    || pendingQueue.value.length === 0
  ) return
  schedulePendingDrainAfterTerminal()
  flushDeferredPendingDrain()
})
watch(activeWorkspaceStatus, (status, previousStatus) => {
  if (
    status !== 'ready'
    || previousStatus === 'ready'
    || pendingQueue.value.length === 0
  ) return
  schedulePendingDrainAfterTerminal()
  flushDeferredPendingDrain()
})

const sessionHasActiveWork = computed(() => (
  isStreaming.value
  || activeTaskGroups.value.size > 0
  || isCompactInFlightForCurrentSession()
  || ['queued', 'running', 'approval_pending'].includes(runStatus.value.status)
  || activePlanRun.value?.status === 'queued'
  || activePlanRun.value?.status === 'running'
  || pendingQueueOwnerContext.value?.sessionKey === sessionKey.value
))
const canStop = computed(() => !isSessionHydrating.value && sessionHasActiveWork.value)
const runModeLocked = computed(
  () => isSessionHydrating.value
    || sessionHasActiveWork.value
    || activeRunModeLock.value !== null,
)

watch(sessionHasActiveWork, active => {
  if (active && activeRunModeLock.value === null) {
    activeRunModeLock.value = globalRunMode.value
  } else if (!active && !isSessionHydrating.value) {
    activeRunModeLock.value = null
  }
}, { flush: 'sync' })

watch(sessionKey, () => {
  activeRunModeLock.value = null
})

const chatSessionRuntime = useChatSessionRuntime({
  sessionKey,
  messages,
  pendingSessionIntent,
  routerDecisionPending: pendingDecision,
  currentEpoch,
  lastStreamSeq,
  activeTaskGroups,
  aborted,
  lastHeaderRole,
  lastHeaderDay,
  usageAccum,
  usageModel,
  createSessionKey,
  persistSession,
  cancelSessionBootstrap: () => {
    // Retire draft-project work on the old socket before the next session's
    // coordinator can start, so its abort/reconnect cannot tear down B.
    draftProjectHydration.invalidate()
    cancelActiveProjectValidation()
    cancelSessionBootstrap()
  },
  startSessionBootstrap,
  loadCurrentSessionUsage,
  applySessionRunState,
  setCompactInFlight,
  hideCompactStatus,
  clearPendingQueue,
  switchPendingQueue,
  adoptPendingQueue,
  resetSavingsPopupCooldown,
  restoreWidgetState,
  resetStreamLiveTurnState,
  resetDraftComposer: () => {
    inputText.value = ''
    pendingAttachments.value = []
    resetComposerInputHistory()
    autoResizeTextarea()
  },
})
const {
  resetCurrentSessionAfterSlash,
  startDraftSession,
  switchToSession: switchRuntimeToSession,
  adoptResponseSession,
  rebindDraftSession,
} = chatSessionRuntime
const {
  cancelActiveProjectValidation,
  openProjectPicker,
  chooseProjectPath,
  closeProjectDraft,
  validateActiveProjectBeforeSend,
  consumeDraftPrefill,
  syncDraftProjectFromRoute,
  enterDraft,
} = useChatProjectController(viewState, {
  rpc,
  confirm,
  projectWorkspaces,
  activeProjectWorkspace,
  freshTaskDraft,
  draftProjectHydration,
  chatSessionRoute,
  chatSessionSubscription,
  chatSessionRuntime,
  isStreaming,
  isDesktopViewport,
  setProvisionalDraftUsed: (value: boolean) => { provisionalDraftUsed = value },
})
switchToPlanSession = switchToSession

// 拖拽 / 剪贴板 控制器：onChatDragEnter/Over/Leave/Drop 经模板 @drag 绑定，
// onDocumentPaste 经 onMounted 内 useDocumentEvent('paste', …) 注册。
const {
  onChatDragEnter,
  onChatDragOver,
  onChatDragLeave,
  onChatDrop,
  onDocumentPaste,
} = useChatDragClipboardController(viewState, {
  replanActive,
  addAttachments,
})

// 执行坞（plan-run dock）控制器：clearExecutionDockHideTimer 在 onUnmounted 清理时调用，
// syncExecutionDockRun 在 activePlanRun 变化 watch 中调用。
const {
  clearExecutionDockHideTimer,
  syncExecutionDockRun,
} = useChatExecutionDockController(viewState, {
  activePlanRun,
})

async function switchToSession(nextSessionKey: string) {
  if (nextSessionKey !== sessionKey.value) {
    activeProjectWorkspace.beginSessionResolution(nextSessionKey)
  }
  const outcome = await switchRuntimeToSession(nextSessionKey)
  if (outcome?.authoritative) {
    await handleAuthoritativeSessionSubscription(nextSessionKey)
  }
  return outcome
}

const metaSkillSetup = useMetaSkillSetup({
  rpc,
  currentSessionKey: sessionKey,
  dispatchHidden: (providerText: string, displayText: string, clientRequestId?: string) => (
    dispatchHiddenForMeta(providerText, displayText, clientRequestId)
  ),
  autoRestore: false,
  restoreDraft: restoreMetaLaunchDraft,
  discardDraft: async (draftSessionKey: string, clientRequestId: string) => {
    const result = await rpc.call<MetaDraftDiscardResponse>('meta.drafts.discard', {
      sessionKey: draftSessionKey,
      clientRequestId,
    })
    if (result?.accepted === true) {
      forgetHiddenControlOutbox(draftSessionKey, clientRequestId)
      return 'accepted'
    }
    if (result?.discarded !== true) return 'unconfirmed'
    // Only after the server confirms atomic discard may the setup flow restore
    // plain composer text. Remove the matching browser hidden-control copy too,
    // otherwise a later session restore could replay the old stable id beside
    // the newly restored composer request.
    forgetHiddenControlOutbox(draftSessionKey, clientRequestId)
    return 'discarded'
  },
  onDraftAlreadyAccepted: () => {
    pushToast(t('chat.metaRuns.cancelAlreadyAccepted'), { tone: 'info', duration: 7000 })
  },
  forgetHiddenControl: (draftSessionKey: string, clientRequestId: string) => {
    forgetHiddenControlOutbox(draftSessionKey, clientRequestId)
  },
})
const {
  setupState,
  requestSetup: requestMetaSetup,
  confirmSetup,
  beginProviderHandoff,
  cancelProviderHandoff,
  retrySetup,
  cancelSetup,
  restoreSetupJob: restoreMetaSetupJob,
  handleHiddenDispatchResult,
} = metaSkillSetup
handleHiddenControlDispatchResult = handleHiddenDispatchResult

// MetaSkill 提供商设置导航控制器（C 阶段抽出）。
const { openMetaSetupProviderSettings } = useChatMetaController(viewState, {
  setupState,
  beginProviderHandoff,
  cancelProviderHandoff,
  persistSession,
  route,
})

const chatSlashCommands = useChatSlashCommands({
  rpc,
  catalogCallOptions: optionalSessionRpcCallOptions,
  inputText,
  sessionKey,
  autoResizeTextarea,
  newSession: () => {
    freshTaskDraft.requestFreshTask('main')
    goToDraft({ agentId: 'main' })
  },
  resetCurrentSession: () => {
    resetCurrentSessionAfterSlash()
    resetSessionArtifacts()
    chatPlans.reset()
  },
  setCompactInFlight,
  showCompactStatus,
  showCompactionToast,
  notify: (message: string) => pushToast(message, { duration: 6000 }),
  dispatchHidden: (
    providerText: string,
    displayText: string,
    clientRequestId?: string,
    targetSessionKey?: string,
  ) => dispatchHiddenForMeta(
    providerText,
    displayText,
    clientRequestId,
    targetSessionKey,
  ),
  restoreDraft: restoreMetaLaunchDraft,
  requestMetaSetup,
  dispatchPlanPrompt: (prompt: string, composerText: string) => {
    dispatchPlanComposerPrompt(prompt, composerText)
  },
  activatePlanMode: () => chatPlans.setMode('plan'),
  planModeAvailable: () => planUiAvailable.value,
  codingModeEnabled,
  setCodingModeEnabled,
})
const {
  slashOpen,
  slashIdx,
  filteredSlashCmds,
  loadSlashCommands,
  handleSlashInput,
  closeSlashMenu,
  completeSlashCmd,
  activateSlashCmd,
  executeSlashCommand,
  restoreDurableMetaDrafts: restoreServerMetaDrafts,
} = chatSlashCommands

const chatComposerShortcuts = useChatComposerShortcuts({
  inputText,
  composing,
  messages,
  pendingQueue,
  canQueueMore,
  slashOpen,
  slashIdx,
  filteredSlashCmds,
  isStreaming,
  autoResizeTextarea,
  handleSlashInput,
  closeSlashMenu,
  completeSlashCmd,
  activateSlashCmd,
  popPendingTail,
  enqueuePendingInput,
  sendCurrentInput: () => sendCurrentInput(),
})
const {
  onTextareaBeforeInput,
  onTextareaInput,
  onTextareaKeydown,
} = chatComposerShortcuts
resetComposerInputHistory = chatComposerShortcuts.resetInputHistory

const chatSend = useChatSend({
  rpc,
  supportsMethod: method => rpc.supportsMethod(method),
  activeSteerCapability: computed(() => {
    const task = runStatus.value.task
    return task?.steer_capability || task?.steerCapability || null
  }),
  inputText,
  messages,
  sessionKey,
  pendingQueueOwnerContext,
  busySendMode,
  modelRoutingMode,
  modelRoutingSettingsBusy,
  elevatedMode,
  runMode,
  pendingAttachments,
  composerRevision,
  pendingSessionIntent,
  pendingWorkspaceId,
  sendBlockedReason: liveSendBlockedReason,
  validateActiveProjectBeforeSend,
  acceptPendingWorkspaceBinding: activeProjectWorkspace.acceptPendingBinding,
  initialCollaborationMode,
  pendingForkBeforeMessageId,
  materializeDraftSession: key => {
    if (!isDraftRoute()) return
    const workspaceId = pendingWorkspaceId.value
    if (workspaceId) {
      freshTaskDraft.bindMaterializedProjectTask(key, workspaceId)
    }
    persistSession(key, { source: 'chatView.draftAccepted' })
  },
  aborted,
  activeStreamTaskId,
  activeStreamSessionKey,
  autoScroll,
  stream: chatStream,
  canStop: () => canStop.value,
  normalizeElevatedMode,
  adoptResponseSession: async (key, ownerRequestId) => {
    const sourceKey = sessionKey.value
    const workspaceId = freshTaskDraft.materializedWorkspaceBySession.value[sourceKey]
      || boundWorkspaceId.value
    if (workspaceId && key !== sourceKey) {
      freshTaskDraft.bindMaterializedProjectTask(key, workspaceId)
      freshTaskDraft.forgetMaterializedProjectTask(sourceKey)
    }
    return adoptResponseSession(key, ownerRequestId)
  },
  scheduleHistorySync,
  schedulePendingDrainAfterTerminal,
  flushDeferredPendingDrain,
  bindActiveStreamTask: taskId => bindActiveStreamTask(taskId),
  isCompactInFlightForCurrentSession,
  hasPendingAttachmentWork,
  prepareAttachmentsForSend,
  enqueuePendingInput,
  enqueuePendingPayload,
  enqueueHiddenControl,
  enqueuePendingSteerRetry,
  restoreSteerIntoComposer: text => appendComposerText(text),
  popAllPendingIntoComposer,
  executeSlashCommand,
  closeSlashMenu,
  autoResizeTextarea,
  scrollToBottom,
})
const {
  onSend: dispatchCurrentInput,
  onStop,
  sendQueuedSteer,
  sendQueuedFollowup,
  dispatchComposerPrompt,
  dispatchHiddenSend,
  dispatchQueuedHiddenSend,
  discardHiddenControl,
  forgetHiddenControl,
  flushPendingMetaDiscards,
  restoreHiddenControls,
  sendHiddenMetaPreflightConfirmation,
} = chatSend
async function onSend(
  sendOptions?: Parameters<typeof dispatchCurrentInput>[0],
): Promise<void> {
  markProvisionalDraftUsed()
  if (pendingAutoSendSessionKey.value === sessionKey.value) {
    pendingAutoSend.value = ''
    pendingAutoSendSessionKey.value = ''
  }
  await dispatchCurrentInput(sendOptions)
}
sendCurrentInput = onSend
dispatchHiddenForMeta = dispatchHiddenSend
discardHiddenControlOutbox = discardHiddenControl
forgetHiddenControlOutbox = forgetHiddenControl

async function restoreDurableMetaControls(
  targetSessionKey: string,
  prefetchedServerDrafts?: DurableMetaDraft[],
  isCurrent: () => boolean = () => true,
): Promise<void> {
  // Setup owns a matching cancellation tombstone so it can clear its recovery
  // checkpoint without ever re-entering launch. Queue-only tombstones are then
  // retried here before any server draft is considered.
  const pendingDiscardIds = new Set(
    listPendingMetaDiscards(targetSessionKey).map(item => item.clientRequestId),
  )
  await restoreMetaSetupJob(targetSessionKey)
  if (!isCurrent()) return
  const setupDiscardRequestId = setupState.value?.retryMode === 'discard'
    ? setupState.value.resumeRequestId || ''
    : ''
  const flushedDiscardIds = await flushPendingMetaDiscards(
    targetSessionKey,
    setupDiscardRequestId ? [setupDiscardRequestId] : [],
  )
  if (!isCurrent()) return
  for (const requestId of flushedDiscardIds) {
    pendingDiscardIds.add(requestId)
  }
  const serverDrafts = (prefetchedServerDrafts
    ?? await listServerMetaDrafts(rpc, { sessionKey: targetSessionKey }))
    .filter(draft => !pendingDiscardIds.has(draft.clientRequestId))
  if (!isCurrent()) return
  restoreDeferredMetaDrafts(
    targetSessionKey,
    new Set(serverDrafts.map(draft => draft.launchText)),
  )
  const activeSetupRequestId = setupState.value?.sessionKey === targetSessionKey
    ? setupState.value.resumeRequestId || setupState.value.providerHandoff?.clientRequestId || ''
    : ''
  const matchingServerDrafts = serverDrafts.filter(
    draft => draft.sessionKey === targetSessionKey,
  )
  const setupHandledRequestIds = activeSetupRequestId
    ? matchingServerDrafts
        .filter(draft => draft.clientRequestId === activeSetupRequestId)
        .map(draft => draft.clientRequestId)
    : []
  const attemptedServerRequestIds = await restoreServerMetaDrafts(
    matchingServerDrafts.filter(
      draft => draft.clientRequestId !== activeSetupRequestId,
    ),
    isCurrent,
  )
  if (!isCurrent()) return
  await restoreHiddenControls(
    targetSessionKey,
    [...setupHandledRequestIds, ...attemptedServerRequestIds],
    isCurrent,
  )
}

function flushPendingAutoSend(targetSessionKey: string): boolean {
  if (
    !pendingAutoSend.value
    || pendingAutoSendSessionKey.value !== targetSessionKey
    || sessionKey.value !== targetSessionKey
  ) {
    return false
  }
  const text = pendingAutoSend.value
  pendingAutoSend.value = ''
  pendingAutoSendSessionKey.value = ''
  // The handoff is no longer automatic once the user edits its prefill while
  // waiting for an authoritative reconnect.
  if (inputText.value !== text) return false
  sendComposerText(text)
  return true
}

async function handleAuthoritativeSessionSubscription(
  targetSessionKey: string,
  prefetchedServerDrafts?: DurableMetaDraft[],
): Promise<void> {
  const attempt = ++durableRecoveryGeneration
  const isCurrent = () => (
    chatViewActive
    && attempt === durableRecoveryGeneration
    && sessionKey.value === targetSessionKey
  )
  if (!isCurrent()) return
  // Ordinary Sessions Hub handoffs must never wait behind optional Meta
  // recovery. Durable controls remain persisted for the next reconnect.
  if (flushPendingAutoSend(targetSessionKey)) return
  await Promise.all([
    metaRuns.hydrateRecovery(),
    restoreDurableMetaControls(targetSessionKey, prefetchedServerDrafts, isCurrent),
  ])
}

function isPristineDraftForRecovery(expectedSessionKey: string, agentId: string): boolean {
  return !provisionalDraftUsed
    && sessionKey.value === expectedSessionKey
    && isDraftRoute()
    && draftAgentId() === agentId
    && agentIdFromSessionKey(expectedSessionKey) === agentId
    && pendingSessionIntent.value === 'new_chat'
    && messages.value.length === 0
    && inputText.value.length === 0
    && pendingAttachments.value.length === 0
    && pendingQueue.value.length === 0
    && pendingAutoSend.value.length === 0
    && !isStreaming.value
    && setupState.value?.sessionKey !== expectedSessionKey
}

const metaDraftRecovery = createChatMetaDraftRecovery({
  currentSessionKey: () => sessionKey.value,
  listDrafts: query => queryServerMetaDrafts(rpc, query),
  isPristineDraft: isPristineDraftForRecovery,
  rebindDraftSession,
  onAuthoritativeSubscription: handleAuthoritativeSessionSubscription,
})

let provisionalDraftUsed = false
let durableRecoveryGeneration = 0

function markProvisionalDraftUsed(): void {
  if (provisionalDraftUsed) return
  provisionalDraftUsed = true
  metaDraftRecovery.invalidate()
}
const sameTurnSteerAvailable = computed(() => (
  isStreaming.value
  && chatSend.supportsSameTurnSteer()
))
const composerSameTurnSteerAvailable = computed(() => (
  sameTurnSteerAvailable.value
  && pendingAttachments.value.length === 0
  && !pendingSessionIntent.value
  && !pendingForkBeforeMessageId.value
))
watch(composerSameTurnSteerAvailable, (available) => {
  if (!available && busySendMode.value === 'steer') {
    busySendMode.value = 'queue'
  }
})

async function onComposerSend() {
  // All composer submission modes, including keyboard-driven plan revision,
  // share the same fail-closed delivery gate.
  if (composerSendBlockedMessage.value) return
  // Serialize an existing-session mode mutation before accepting another
  // composer turn, so the send cannot race the collaboration CAS update.
  if (planModeBusy.value) return
  const target = replanTarget.value
  if (!target) {
    onSend()
    return
  }
  const prompt = inputText.value.trim()
  if (!prompt) return
  const submittedRevision = composerRevision.value
  const accepted = await chatPlans.revise({ ...target, prompt })
  if (!accepted) return
  if (composerRevision.value === submittedRevision) {
    inputText.value = ''
    autoResizeTextarea()
  }
}

sendCurrentInput = onComposerSend
sendAutomaticInput = () => {
  void onSend({ cancelIfComposerChanged: true })
}
dispatchHiddenForMeta = dispatchHiddenSend
dispatchPlanComposerPrompt = (prompt, composerText) => {
  void dispatchComposerPrompt(prompt, composerText)
}
dispatchQueuedHiddenControl = dispatchQueuedHiddenSend
dispatchQueuedItem = sendQueuedFollowup

function takeVisiblePendingItem(index: number) {
  const item = pendingQueue.value[index]
  if (!item || item.hiddenControl || item.deliveryState) return null
  pendingQueue.value.splice(index, 1)
  return item
}

function editPendingMessage(index: number) {
  const item = takeVisiblePendingItem(index)
  if (!item) return
  inputText.value = [item.text, inputText.value].filter(text => text.trim()).join('\n')
  pendingAttachments.value = [...(item.attachments || []), ...pendingAttachments.value]
  pendingSessionIntent.value = item.intent || pendingSessionIntent.value
  autoResizeTextarea()
  nextTick(() => composerRef.value?.focusTextarea())
}

async function steerPendingMessage(index: number) {
  const candidate = pendingQueue.value[index]
  const item = beginPendingDelivery(index, candidate?.hiddenControl === true)
  if (!item) return

  let outcome: ChatSendOutcome = 'retryable_failure'
  try {
    outcome = item.hiddenControl
      ? await dispatchQueuedHiddenSend(item, item.ownerSessionKey || sessionKey.value)
      : await sendQueuedSteer(item)
  } finally {
    settlePendingDelivery(item, outcome)
  }
}

const chatApprovals = useChatApprovals({
  rpc,
  sessionKey,
  runStatus,
  stream: { isStreaming, appendInterruptFrame, ensureInterruptBubble },
  interruptState,
  onSnapshotCount: count => appStore.setApprovalCount(count),
})
const {
  approvalEntries,
  approvalBusyIds,
  pendingClarify,
  clarifySubmitted,
  clarifyBusy,
  clarifyError,
  resolveApproval,
  resolveInterrupt,
  extendInterrupt,
  submitClarify,
  dismissClarify,
  applyUserInputBootstrap,
} = chatApprovals
applyPendingUserInputSnapshot = applyUserInputBootstrap

const dockedPlanQuestionnaire = computed(() => (
  pendingClarify.value?.presentation === 'plan_questionnaire_v1'
    ? pendingClarify.value
    : null
))

function handlePlanQuestionnaireWheel(event: WheelEvent) {
  handoffPlanQuestionnaireWheel(event, threadRef.value)
}

const rpcEventHandlers = useChatRpcEventHandlers({
  sessionKey,
  currentEpoch,
  lastStreamSeq,
  activeTaskGroups,
  activeStreamTaskId,
  aborted,
  messages,
  pendingQueue,
  usageAccum,
  usageModel,
  stream: chatStream,
  normalizeRunStatus,
  sessionRunStatus,
  applySessionRunState,
  queueRouterDecision,
  appendEnsembleProgress,
  markEnsembleHandoff,
  flushPendingRouterDecision,
  clearPendingRouterDecision,
  handleRouterControlReplay,
  showCompactionToast,
  getCompactionPlacement: id => getCompactionPlacement(id) || undefined,
  showWarningToast: message => pushToast(message || t('chat.warning.default'), { tone: 'warn', duration: 5000 }),
  scheduleHistorySync,
  schedulePendingDrainAfterTerminal,
  popAllPendingIntoComposer,
  restoreSteerIntoComposer: text => appendComposerText(text),
  saveWidgetState,
  onSessionSubscribed: () => {
    if (isDraftRoute()) metaDraftRecovery.retry(draftAgentId())
    return handleAuthoritativeSessionSubscription(sessionKey.value)
  },
  handleSessionConnectionState: state =>
    handleSessionConnectionState(state, !isDraftRoute()),
  loadCurrentSessionUsage,
  refreshRunModePreference: refreshPostBootstrapMetadata,
})
bindActiveStreamTask = rpcEventHandlers.bindActiveStreamTask
restoreLiveTurnSnapshot = rpcEventHandlers.restoreLiveTurnSnapshot
const {
  streamThinkingText,
  streamThinkingElapsedText,
  attachTurnReasoning,
} = rpcEventHandlers

// live-turn shadow parity: in DEV/SHADOW, re-check the fold against the legacy
// live surface whenever a frame lands (the fold and legacy refs are tracked by
// assertLiveParity). Injects the thinking text owned by the event handlers.
// In production ON mode this is a no-op; DEV/SHADOW performs the parity check,
// while explicit OFF keeps the compatibility renderer without fold assertions.
watchEffect(() => assertLiveParity(streamThinkingText))

// Flag-selected live render source. In production the fold is authoritative by
// default; only marguerite.chat.foldLiveTurn=0 restores legacy. SHADOW and OFF
// return the IDENTICAL legacy refs, so with the flag off the render is byte-identical.
// The activity head (phase/elapsed) stays on the legacy activity refs.
const liveTimelineItems = computed(() =>
  foldLiveTurnMode.value === true ? foldedTurn.value.timelineItems : streamTimelineItems.value,
)
const liveTimelineSplit = computed(() => splitLiveAssistantTimeline(liveTimelineItems.value, {
  keepToolTurnTextInActivity: true,
}))
const liveAnswerPart = computed<Extract<ChatPart, { type: 'text' }> | null>(() => {
  const candidate = liveTimelineSplit.value.answerItem
  if (!candidate) return null
  return {
    type: 'text',
    key: `${candidate.key}:answer-candidate`,
    html: candidate.html,
    rawText: candidate.rawText || '',
  }
})
const liveActivityTimelineItems = computed<ChatStreamTimelineItem[]>(() =>
  liveTimelineSplit.value.activityItems,
)
const liveActivityStatusHistory = computed(() =>
  foldLiveTurnMode.value === false ? [] : foldedTurn.value.statusHistory,
)
const liveActivityProjection = computed(() =>
  projectAssistantActivityTimeline(liveActivityTimelineItems.value, {
    lifecycle: liveAnswerPart.value ? 'answering' : 'working',
    statusHistory: liveActivityStatusHistory.value,
  }),
)
const liveActivityPhaseLabel = computed(() => {
  if (streamActivityStale.value) return streamPhaseLabel.value
  const currentStatus = [...liveActivityProjection.value.statusSteps]
    .reverse()
    .find(step => step.isCurrent)
  if (
    currentStatus
    && currentStatus.category !== 'maintenance'
    && !currentStatus.label.code.startsWith('chat.activity.lifecycle.')
    && !liveActivityProjection.value.currentClusterKey
  ) {
    return String(t(currentStatus.label.code, currentStatus.label.params))
  }
  return String(t(
    liveAnswerPart.value
      ? 'chat.activity.lifecycle.answering'
      : 'chat.activity.lifecycle.working',
  ))
})
const liveToolStateScope = computed(() => JSON.stringify([sessionKey.value || '', 'stream']))
// Elapsed readouts in the live turn round to whole seconds ("4s"), matching
// streamPhaseElapsed and streamThinkingElapsedText. Shared util in
// utils/chat/liveToolElapsedText.ts; sub-second finished tools keep tenths so
// they never read as a nonsensical "0s".
const liveArtifacts = computed(() =>
  foldLiveTurnMode.value === true ? foldedTurn.value.artifacts : streamArtifacts.value,
)
const liveThinkingText = computed(() =>
  foldLiveTurnMode.value === true ? foldedTurn.value.thinkingText : streamThinkingText.value,
)
// Live reasoning rendered through the shared part component, so the live turn
// and settled turns use one wording and one disclosure affordance. The seconds
// derive from the ticking elapsed text, which is always `${seconds}s` live.
const liveReasoningPart = computed<Extract<ChatPart, { type: 'reasoning' }> | null>(() => {
  if (!liveThinkingText.value) return null
  const seconds = Number.parseInt(streamThinkingElapsedText.value, 10)
  return {
    type: 'reasoning',
    key: 'live-reasoning',
    text: liveThinkingText.value,
    seconds: Number.isFinite(seconds) ? seconds : 0,
  }
})
// No clamp and no raw status count: the header chip must agree with the
// visible body, which renders clusters plus only the semantic status steps.
// A text-only turn therefore counts 0 and the disclosure's stepCount > 0
// gate hides the chip instead of claiming "step 1" over an empty body.
const liveActivityStepCount = computed(() =>
  liveActivityProjection.value.activityClusters.length
    + liveActivityProjection.value.statusSteps.filter(isSemanticActivityStatusStep).length
    + (liveThinkingText.value ? 1 : 0),
)
const liveActivityFailureCount = computed(() =>
  liveActivityProjection.value.activityClusters.filter(cluster => cluster.isFailure).length,
)
// Inline interrupt parts for the live turn come from the fold whenever it is
// active (ON or SHADOW — frames are appended in both). Only the foldLiveTurn=0
// OFF rollback renders the legacy standalone ApprovalCard/ClarifyCard block, so
// the two never both show. Unlike the activity body (which has a legacy ref to
// fall back to in SHADOW), interrupts have no legacy live ref, so SHADOW must
// also render them from the fold.
const liveInterruptParts = computed(() =>
  foldLiveTurnMode.value === false
    ? []
    : foldedTurn.value.parts.filter(
        (part): part is Extract<typeof part, { type: 'interrupt' }> => part.type === 'interrupt',
      ),
)
const livePendingInterruptParts = computed(() =>
  liveInterruptParts.value.filter(part => !part.resolution),
)

const visiblePendingInterruptKeys = computed(() => {
  const keys = new Set(livePendingInterruptParts.value.map(part => part.key))
  for (const message of renderedMessages.value) {
    for (const part of message.parts ?? []) {
      if (part.type === 'interrupt' && !part.resolution) keys.add(part.key)
    }
  }
  return [...keys]
})

// focusPendingApprovalCard 已迁入 useChatComposerController；其 watch 接线保留于此。
watch(
  [
    () => appStore.approvalFocusRequest?.requestId ?? 0,
    sessionKey,
    () => visiblePendingInterruptKeys.value.join('\u0000'),
  ],
  () => { void focusPendingApprovalCard() },
  { flush: 'post', immediate: true },
)

// Feeds the persistent visually-hidden status region in the template. It only
// fills on the true→false streaming transition (a live turn actually settled),
// and empties as soon as the next turn starts so that setting the same
// "Completed" text again is a fresh mutation screen readers re-announce.
watch(isStreaming, (streaming, wasStreaming) => {
  if (streaming) turnSettledAnnouncement.value = ''
  else if (wasStreaming) {
    turnSettledAnnouncement.value = String(t('chat.activity.lifecycle.settled'))
    // Turn-end artifact resync — the fallback that makes AC-A1 hold even when
    // the daemon never emits `artifact.updated`. The live `session.event.artifact`
    // stream is gated on the turn's *current* task, so a file registered as the
    // turn winds down (or by a tool whose task was already cleared) never reaches
    // the sidebar. Re-listing once the turn settles closes that window without
    // polling: it costs one `artifacts.list` per turn, and the composable
    // de-duplicates by artifact identity, so an already-listed file is a no-op.
    void reloadSidebarArtifacts()
    staleKeySet.clear()
  }
})

// Soft content-silence watchdog: after the high negotiated threshold, surface
// a neutral long-running notice. Backend-deadline-owned Ensemble phases remain
// suppressed, while the hard idle timer continues to mean no events at all.
const stallWatchdog = useChatStallWatchdog({ isStreaming, streamIdleGraceMs: streamIdleTimeoutMs })
const { stallActive, stallSeconds } = stallWatchdog

const noop = () => {}

// Chat-view-scoped RPC events: torn down with the view (see onUnmounted).
// The handlers that must keep the sidebar/session list live across routes are
// replaced with no-ops here so the real ones only live on `globalRpcSubscriptions`.
const chatRpcSubscriptions = useChatRpcSubscriptions(rpc, {
  ...rpcEventHandlers.handlers,
  onSessionsChanged: noop,
  onRunHeartbeat: noop,
  onTaskQueued: noop,
  onTaskRunning: noop,
  onTaskGroupWaiting: noop,
  onTaskGroupSynthesizing: noop,
  onTaskGroupDone: noop,
  onTaskGroupFailed: noop,
  onEnsembleProgress: noop,
  onArtifactUpdated,
  // The wildcard handler is the one funnel that sees every gateway event with
  // its name; feed the active session's events to the watchdog before the
  // regular handler consumes them (same session filter as existing handlers).
  onAny: (rawEvent, rawPayload) => {
    const payloadObj = (rawPayload && typeof rawPayload === 'object' ? rawPayload : {}) as SessionEventPayload
    if (payloadIsCurrentSession(payloadObj, sessionKey.value)) {
      stallWatchdog.noteEvent(rawEvent, payloadObj)
    }
    rpcEventHandlers.handlers.onAny(rawEvent, rawPayload)
  },
})

// Global, route-surviving RPC events: sessions.changed / run-heartbeat / task.*
// / ensemble_progress are kept subscribed even after ChatView unmounts so the
// sidebar session list and run-status keep updating in real time (the user
// complaint was "leave the chat page and the task stops updating"). Its
// unsubscribe is held separately in `globalRpcUnsub` and is deliberately NOT
// added to `unsubs`, so onUnmounted never tears it down. Chat-view-only
// handlers are no-op'd here for an equivalent (single real listener) wiring.
const globalRpcSubscriptions = useChatRpcSubscriptions(rpc, {
  ...rpcEventHandlers.handlers,
  onTextDelta: noop,
  onToolUseStart: noop,
  onToolUseDelta: noop,
  onToolResult: noop,
  onArtifact: noop,
  onStateChange: noop,
  onCompaction: noop,
  onWarning: noop,
  onInputDisposition: noop,
  onCronResult: noop,
  onSubagentCompletion: noop,
  onEpochChanged: noop,
  onRouterDecision: noop,
  onRouterControlReplay: noop,
  onConnectionState: noop,
  onArtifactUpdated,
  onAny: noop,
})

// Session switches drop the previous session's stall tracking entirely.
watch(sessionKey, () => {
  stallWatchdog.reset()
  clearAssistantActivityExpansionState()
})

// MetaSkill run UI: preflight checkpoint + run-progress ribbon, driven by the
// four session.event.meta_* frames (delivered via the '*' wildcard, so this
// controller must not re-consume stream_seq).
const metaRuns = useMetaRuns({
  rpc,
  sessionKey,
  currentEpoch,
  lastStreamSeq,
  sendHiddenConfirmation: sendHiddenMetaPreflightConfirmation,
  sendHiddenReplay: (providerText: string, displayText: string) => (
    dispatchHiddenForMeta(providerText, displayText)
  ),
  scrollToStepCard,
  sendComposerText,
  lastUserMessageText,
  // The composer placeholder is a computed prop, so a true placeholder setter
  // is not exposed; surface the switch-skill hint via the toast path (keeping
  // focus) so the vanilla guidance is not silently dropped.
  setComposerPlaceholder: (hint: string) => pushToast(hint, { duration: 6000 }),
  focusComposer: () => composerRef.value?.focusTextarea(),
  pushToast,
})


// lastUserMessageText / scrollToStepCard 已迁入 useChatComposerController。

// History syncs replace the messages array; rows carry reasoning text but
// not the measured thinking duration — re-attach this session's records.
watch(messages, () => attachTurnReasoning())

// Unsubscribers
let unsubs: (() => void)[] = []
// Held separately from `unsubs` so it survives ChatView unmount (route-surviving
// session/task events). Deliberately never torn down by onUnmounted.
let globalRpcUnsub: (() => void) | null = null
let chatViewDisposed = false
let composerDockResizeObserver: ResizeObserver | null = null
let composerDockPinFrame: number | null = null
let lastComposerDockHeight = -1

/* ── Computed ──────────────────────────────────────────────────────── */

const isNewChatLanding = computed(() => {
  // Only the draft route (/chat/new — bare /chat redirects here) shows the
  // "new chat" landing. Without this gate, switching between existing
  // conversations briefly cleared `messages` and flashed the landing, because
  // the empty-thread moment of a session load looked identical to a new draft.
  return isDraftRoute() &&
    messages.value.length === 0 &&
    !isStreaming.value &&
    pendingQueue.value.length === 0 &&
    !compactStatus.value.visible
})

watch(isNewChatLanding, resetComposerRetraction, { flush: 'sync' })

const historyRecoveryState = computed(() => {
  return resolveChatHistoryRecoveryState({
    isDraftLanding: isNewChatLanding.value,
    initialHistoryStatus: historyState.value.initialLoadStatus,
    retrying: historyState.value.retrying,
    recoveryError: historyState.value.recoveryError,
  })
})

const visibleHistoryRecoveryState = computed(() => (
  visibleChatHistoryRecoveryState(historyRecoveryState.value)
))

const liveRecoveryState = computed(() => {
  if (livePhase.value === 'degraded') return 'live-degraded' as const
  if (
    livePhase.value === 'connecting'
    && historyRecoveryState.value === null
  ) {
    return 'live-connecting' as const
  }
  return null
})

const showConfirmedEmptySession = computed(() => shouldShowConfirmedEmptySession({
  isDraftLanding: isNewChatLanding.value,
  isStreaming: isStreaming.value,
  messageCount: messages.value.length,
  initialHistoryStatus: historyState.value.initialLoadStatus,
}))

const composerPlaceholder = computed(() => {
  if (dockedPlanQuestionnaire.value) return t('chat.clarify.answerPlanQuestionnaire')
  if (replanActive.value) return t('chat.plan.revisePromptPlaceholder')
  if (collaboration.value.mode === 'plan') return t('chat.planMode.placeholder')
  if (isNewChatLanding.value) return t('chat.placeholderLanding')
  return isCompactViewport.value ? t('chat.placeholderCompact') : t('chat.placeholder')
})

const hasSendContent = computed(() => {
  return inputText.value.trim().length > 0 || pendingAttachments.value.some(isSendableAttachment)
})
const composerHasSendContent = computed(() =>
  replanActive.value ? inputText.value.trim().length > 0 : hasSendContent.value,
)

// A mixed-version gateway may know plans.setMode but not the atomic first-send
// contract. Hide Plan rather than claim a read-only turn that would run Default.
const planUiAvailable = computed(() =>
  rpc.supportsMethod('plans.setMode')
  && rpc.supportsMethod('plans.capabilities'),
)
const planCardPendingAction = computed<PlanCardAction | null>(() => {
  const action = planActionPending.value
  if (action === 'revise') return 'replan'
  return action === 'implement-current' || action === 'implement-new' || action === 'replan'
    ? action
    : null
})
const planActionsDisabled = computed(() =>
  isStreaming.value
  || planModeBusy.value
  || Boolean(liveSendBlockedReason.value)
  || planActionPending.value !== null
  || activePlanRun.value?.status === 'queued'
  || activePlanRun.value?.status === 'running',
)
const composerStopsPlanRun = computed(() =>
  executionDockRun.value?.status === 'queued'
  || executionDockRun.value?.status === 'running',
)

// 执行坞编排（clearExecutionDockHideTimer / syncExecutionDockRun）已迁入
// useChatExecutionDockController，经装配层统一接线（activePlanRun 变化 watch 与
// onUnmounted 清理均指向解构出的同名绑定）。

watch(
  () => [
    activePlanRun.value?.runId,
    activePlanRun.value?.status,
    activePlanRun.value?.stateRevision,
  ],
  syncExecutionDockRun,
  { immediate: true },
)
const currentPlanInHistory = computed(() => {
  const revisionId = currentPlan.value?.revisionId
  if (!revisionId) return false
  return renderedMessages.value.some(message =>
    message.planRevisions?.some(plan => plan.revisionId === revisionId),
  )
})

const landingSuggestionsHidden = computed(() => landingPrefilled.value)
const landingSuggestionsDisabled = computed(() => shouldDisableLandingSuggestions({
  landingPrefilled: landingPrefilled.value,
  composerText: inputText.value,
  attachmentCount: pendingAttachments.value.length,
}))

const queuedImageSendBlockedMessage = computed(() => {
  if (modelRoutingSettingsBusy.value) {
    return t('chat.composer.routingUpdateImageBlocked')
  }
  return modelRoutingMode.value === 'llm_ensemble' || modelRoutingMode.value === 'bloom_ensemble'
    ? t('chat.composer.ensembleImageUnsupported')
    : ''
})

const modelImageSendBlockedMessage = computed(() => {
  return hasSendableModelInputImageAttachment(pendingAttachments.value)
    ? queuedImageSendBlockedMessage.value
    : ''
})

const activeProjectStatusMessage = computed(() => {
  switch (activeWorkspaceStatus.value) {
    case 'resolving':
      return t('workspaces.activeProjectResolving')
    case 'unavailable':
      return t('workspaces.activeProjectUnavailable')
    case 'removed':
      return t('workspaces.activeProjectRemoved')
    case 'unknown':
    case 'error':
      return t('workspaces.activeProjectBlocksSending')
    default:
      return ''
  }
})

const activeProjectComposerBlockMessage = computed(() => {
  switch (activeWorkspaceStatus.value) {
    case 'resolving':
    case 'unavailable':
    case 'removed':
      return activeProjectStatusMessage.value
    default:
      return ''
  }
})

const composerSendBlockedMessage = computed(() =>
  (forkTransition.value
    ? t(
        forkTransition.value.phase === 'error'
          ? 'chat.forkOpenFailed'
          : forkTransition.value.phase === 'creating'
            ? 'chat.forkCreating'
            : forkTransition.value.phase === 'returning'
              ? 'chat.forkReturning'
              : 'chat.forkOpening',
      )
    : '')
  || modelImageSendBlockedMessage.value
  || liveSendBlockedReason.value
  || activeProjectComposerBlockMessage.value,
)

const sendButtonTitle = computed(() => {
  if (replanActive.value) return t('chat.plan.reviseSend')
  if (composerSendBlockedMessage.value) return composerSendBlockedMessage.value
  if (isCompactInFlightForCurrentSession()) return t('chat.sendQueuesUntilCompaction')
  if (isStreaming.value) {
    return busySendMode.value === 'steer' && composerSameTurnSteerAvailable.value
      ? t('chat.sendSteers')
      : t('chat.sendQueues')
  }
  return t('chat.send')
})

// 运行模式 / 沙箱设置 / 计划动作编排已迁入 useChatRunModeController；
// 以下 14 个函数被模板或 ChatView 其它逻辑引用，沿用控制器接线（另 2 个
// 内部助手 reportRunModePersistenceError / persistComposerRunMode 不导出）。
const {
  implementCurrentPlan,
  implementPlanInNewTask,
  beginPlanRevision,
  cancelPlanRevision,
  cancelActivePlanRun,
  focusComposerAfterPlanRun,
  onComposerStop,
  setCollaborationMode,
  setComposerRunMode,
  cancelComposerSandboxSetup,
  confirmComposerSandboxSetup,
  runComposerSandboxSetupInBackground,
  setComposerModelRoutingMode,
  setComposerCodingModeEnabled,
} = useChatRunModeController(viewState, {
  chatPlans,
  sandboxSetupStore,
  sandboxSetupRecovery,
  onStop,
  pendingAttachments,
  liveSendBlockedReason,
  runModeLocked,
  setGlobalRunMode,
  setModelRoutingMode,
  setCodingModeEnabled,
  scheduleHistorySync,
  refreshRunModePreference,
})

const currentChatTitle = computed(() => {
  const firstUser = messages.value.find(msg => msg.role === 'user' && stripTimePrefix(msg.text || '').trim())
  if (firstUser) {
    return truncate(stripTimePrefix(firstUser.text).replace(/\s+/g, ' ').trim(), 28)
  }
  const suffix = sessionKey.value.split(':').pop() || ''
  if (!suffix || suffix === 'default') return t('chat.newChat')
  return t('chat.chatWithSuffix', { suffix })
})

function focusComposer() {
  composerRef.value?.focusTextarea?.()
}

const chatMarkdownExport = useChatMarkdownExport({
  messages: renderedMessages,
  currentTitle: currentChatTitle,
  aiGeneratedLabel,
})
const { exportMarkdown } = chatMarkdownExport

const shareableMessageCount = computed(() => renderedMessages.value.filter(isShareableChatMessage).length)
const selectedShareCount = computed(() => selectedShareMessageIds.value.size)

/* ── Helpers ───────────────────────────────────────────────────────── */

// readAuthToken 已抽出到 ./chat（views/chat/read-auth-token.ts），经顶部 import 消费。

// 运行模式 / 沙箱设置编排（reportRunModePersistenceError / persistComposerRunMode /
// setComposerRunMode / cancelComposerSandboxSetup / confirmComposerSandboxSetup /
// runComposerSandboxSetupInBackground / setComposerModelRoutingMode /
// setComposerCodingModeEnabled）已迁入 useChatRunModeController，经上方统一接线。

function onVoiceInput() {
  void toggleVoiceInput(appendComposerText)
}

// When voice isn't configured the mic button routes here instead of recording:
// tell the user what's missing and take them straight to the audio settings.
function onVoiceSetup() {
  pushToast(t('chat.toast.voiceSetupNeeded'), { tone: 'info' })
  router.push('/settings/capabilities').catch(() => {})
}

/* ── Artifacts ─────────────────────────────────────────────────────── */

const sessionWorkbenchArtifacts = computed(() =>
  sessionArtifacts.value.filter(artifactUsesWorkbenchPreview),
)

const headerDeliverableCount = computed(() => sessionArtifacts.value.length)

const {
  downloadAttachment,
  downloadArtifact,
  openDeliverables,
  openArtifact,
  closeDeliverables,
  startShareMode,
  endShareMode,
  toggleShareMessage,
  saveShareImage,
  onShareDownload,
  onShareCopy,
  onShareSetTheme,
  closeSharePreview,
} = useChatArtifactShareController(viewState, {
  chatShareExport,
  artifactImageLightbox,
  workbenchStore,
  platform,
  workbenchEnabled,
  chatHeaderActionsRef,
  shareBannerRef,
  sessionArtifacts,
  sessionWorkbenchArtifacts,
  shareableMessageCount,
  readAuthToken,
})


watch(sessionArtifacts, artifacts => {
  if (!sessionKey.value) return
  artifactImageLightbox.updateNavigation(artifacts, sessionKey.value)
  if (!workbenchEnabled.value) return
  for (const item of workbenchStore.items) {
    if (
      item.kind !== 'artifact-preview'
      || item.scope.type !== 'session'
      || item.scope.id !== sessionKey.value
    ) continue
    const artifact = artifactFromWorkbenchItem(item)
    if (!artifact) continue
    workbenchStore.updateItem(createArtifactPreviewWorkbenchItem({
      artifact,
      navigationArtifacts: artifacts,
      nativeHtml: item.hostKind === 'native-webcontents',
      sessionKey: sessionKey.value,
    }))
  }
})

/* ── Fork ──────────────────────────────────────────────────────────── */

// Fork orchestration lives in useChatForkController. The three watchers below
// (route.query.session hand-off, history status, live phase) still register in
// ChatView because it owns the lifecycle, but their bodies only call back into
// the controller — no fork state is mutated inline anymore.
// clearForkTransition / isForkTransitionActive / failForkTransition are
// intentionally not destructured: after the watcher bodies moved into the
// controller they have no remaining call site in this assembly layer.
const {
  retryForkTransition,
  returnToForkParent,
  forkConversation,
  resumeSandbox,
  applyForkRouteHandoff,
  syncForkFromHistoryStatus,
  syncForkFromLivePhase,
} = useChatForkController(viewState, {
  rpc,
  router,
  forkTransitionLifetime,
  isViewActive: () => chatViewActive,
  isViewDisposed: () => chatViewDisposed,
  readSessionFromUrl,
  renderedMessages,
  isStreaming,
  livePhase,
  historySessionKey,
  historyState,
  retryHistory,
  retryLive,
})

const {
  copyState: sessionCopyState,
  copyIconName: sessionCopyIcon,
  copyLiveText: sessionCopyLiveText,
  onCopyClick: onSessionCopyClick,
} = useCopyFeedback(async () => {
  if (!sessionKey.value) return false
  try {
    await copyTextWithFallback(sessionKey.value)
    return true
  } catch {
    pushToast(t('chat.toast.copyFailed'), { tone: 'danger' })
    return false
  }
})

// The export composable owns all filename composition and slugging (it is
// CJK-aware). Hand it the raw human title and nothing else — pre-mangling here
// (e.g. stripping non-ASCII) would erase Chinese titles before the slugger sees
// them, and pre-composing a filename only forced the composable to take it back
// apart.
function shareTitle(): string {
  return currentChatTitle.value
}

/* ── Streaming ─────────────────────────────────────────────────────── */

// scrollToBottom 已迁入 useChatComposerController。

function onThreadScroll() {
  const el = threadRef.value
  if (!el) return
  const gap = el.scrollHeight - el.scrollTop - el.clientHeight
  historyNavigationScrollLock.updateFromScroll(gap)
  if (isNewChatLanding.value || !composerFxEnabled.value) {
    resetComposerRetraction()
    return
  }
  const wasCollapsed = composerCollapsed.value
  composerCollapsed.value = composerRetraction.observe({
    scrollTop: el.scrollTop,
    bottomGap: gap,
    intent: currentThreadScrollIntent(),
    canCollapse: !slashOpen.value && (composerRef.value?.canCollapse() ?? true),
    navigationLocked: historyNavigationScrollLock.locked,
  })
  if (composerCollapsed.value !== wasCollapsed) clearPendingComposerScrollIntent()
}

function onThreadWheel(event: WheelEvent) {
  if (event.deltaY === 0) return
  markThreadScrollIntent(event.deltaY < 0 ? 'up' : 'down')
}

function onThreadPointerMove(event: PointerEvent) {
  if (event.buttons !== 0 || event.pointerType === 'touch') {
    markThreadScrollIntent('either')
  }
}

function onThreadScrollKeydown(event: KeyboardEvent) {
  if (event.target !== event.currentTarget) return
  const up = event.key === 'ArrowUp'
    || event.key === 'PageUp'
    || event.key === 'Home'
    || (event.key === ' ' && event.shiftKey)
  const down = event.key === 'ArrowDown'
    || event.key === 'PageDown'
    || event.key === 'End'
    || (event.key === ' ' && !event.shiftKey)
  if (up || down) markThreadScrollIntent(up ? 'up' : 'down')
}

function syncComposerRetractionFromThread() {
  const el = threadRef.value
  if (!el) return
  clearPendingComposerScrollIntent()
  const gap = el.scrollHeight - el.scrollTop - el.clientHeight
  historyNavigationScrollLock.updateFromScroll(gap)
  composerCollapsed.value = composerRetraction.observe({
    scrollTop: el.scrollTop,
    bottomGap: gap,
    intent: null,
    canCollapse: !slashOpen.value && (composerRef.value?.canCollapse() ?? true),
    navigationLocked: false,
  })
}

function onHistoryNavigate() {
  cancelAnchorStabilization()
  syncComposerRetractionFromThread()
  historyNavigationScrollLock.start()
}

function onHistoryNavigateEnd() {
  historyNavigationScrollLock.finish()
  // Smooth-scroll frames and the final arrival are navigation, not transcript
  // browsing gestures. Establish a baseline without toggling the composer.
  syncComposerRetractionFromThread()
  if (autoScroll.value) expandComposer()
}

// Show the jump-to-latest affordance whenever the reader has scrolled up off the
// live edge (autoScroll releases at gap >= 60) and there is content to return to.
// Re-pinning autoScroll lets the stream resume following the bottom.
const showJumpToLatest = computed(() => !autoScroll.value && messages.value.length > 0)
function jumpToLatest() {
  cancelAnchorStabilization()
  historyNavigationScrollLock.finish()
  expandComposer()
  autoScroll.value = true
  scrollToBottom()
}

/* ── Textarea ──────────────────────────────────────────────────────── */

// 拖拽 / 剪贴板 编排（onChatDragEnter / onChatDragOver / onChatDragLeave /
// onChatDrop / onDocumentPaste）已迁入 useChatDragClipboardController，经装配层统一接线
// （模板 @drag* 绑定与 onMounted 内 useDocumentEvent('paste', …) 注册均指向解构出的同名绑定）。

function autoResizeTextarea() {
  composerRef.value?.resizeTextarea()
}

/* ── Document keydown (ESC) ────────────────────────────────────────── */

// onDocumentKeydown 横跨 share 模式 / 待发送队列 / 停止逻辑，属高危跨切面全局处理，
// 按"不盲拆高危"原则保留在本视图。

function onDocumentKeydown(e: KeyboardEvent) {
  if (e.key !== 'Escape') return
  if (e.defaultPrevented) return
  if (hasOpenDialogLayer()) return

  // The share preview modal owns Escape while it is open: it closes only the
  // preview (share mode stays active) via its own handler, so bail here and let
  // it run rather than tearing down the whole share mode underneath it.
  if (sharePreview.value) return

  if (shareMode.value) {
    e.preventDefault()
    endShareMode()
    return
  }

  const target = e.target
  const editableTarget = target instanceof HTMLInputElement
    || target instanceof HTMLSelectElement
    || (target instanceof HTMLTextAreaElement && !composerRef.value?.isTextareaFocused())
    || (target instanceof HTMLElement && target.isContentEditable)
  if (editableTarget) return

  if (canStop.value) {
    e.preventDefault()
    onComposerStop()
    return
  }

  if (pendingQueue.value.length > 0 && !composerRef.value?.isTextareaFocused()) {
    e.preventDefault()
    popAllPendingIntoComposer()
  }
}

/* ── Lifecycle ─────────────────────────────────────────────────────── */

// One-shot composer prefill carried in history state (the Sessions Hub task
// input navigates here with it). Consumed on draft entry so reload or
// back/forward does not re-apply the text.
// 项目 / 草稿编排（cancelActiveProjectValidation / openProjectPicker / chooseProjectPath /
// closeProjectDraft / validateActiveProjectBeforeSend / consumeDraftPrefill /
// draftProjectHydrationIsCurrent / syncDraftProjectFromRoute / enterDraft）已迁入
// useChatProjectController，经装配层 chatSessionRuntime 之后统一接线。

let chatViewActive = false

onMounted(async () => {
  chatViewActive = true
  chatViewDisposed = false
  const initialRouteFullPath = route.fullPath
  // Initialize session key. Without an explicit ?session= the view opens as a
  // draft instead of restoring a previous session.
  const initialSession = resolveInitialSession()
  sessionKey.value = initialSession.sessionKey
  let initialDraftProjectGeneration: number | null = null
  let initialAutoSendSnapshot: {
    text: string
    revision: number
    attachments: Attachment[]
  } | null = null
  if (initialSession.draft) {
    pendingSessionIntent.value = 'new_chat'
    initialDraftProjectGeneration = draftProjectHydration.begin()
    // Apply the hand-off before any asynchronous project/live work. A later
    // completion must never overwrite text the operator typed while waiting.
    consumeDraftPrefill()
    if (pendingAutoSend.value) {
      initialAutoSendSnapshot = {
        text: pendingAutoSend.value,
        revision: composerRevision.value,
        attachments: [...pendingAttachments.value],
      }
    }
  } else {
    activeProjectWorkspace.beginSessionResolution(initialSession.sessionKey)
    persistSession(sessionKey.value, { updateRoute: false, source: 'chatView.initialSession' })
  }

  // Load elevated mode
  loadElevatedMode()

  unsubs.push(rpc.on(
    'sandbox.run_mode.preference.changed',
    payload => applyRunModePreferenceChanged(payload),
  ))

  // Register event handlers before sessions.messages.subscribe can replay
  // buffered events, then start the two critical phases before any optional
  // config, usage, slash-command, or project-list RPC can enter the Gateway's
  // serialized dispatch queue.
  unsubs.push(chatRpcSubscriptions.subscribe())
  // Route-surviving subscription: kept alive across unmount (see globalRpcUnsub).
  globalRpcUnsub = globalRpcSubscriptions.subscribe()
  unsubs.push(chatApprovals.subscribe())
  unsubs.push(metaRuns.subscribe())
  unsubs.push(chatPlans.subscribe())
  const sessionBootstrap = startSessionBootstrap({
    includeHistory: !initialSession.draft,
  })
  const initialDraftProjectSync = initialDraftProjectGeneration === null
    ? Promise.resolve(true)
    : sessionBootstrap.live.then(() =>
        syncDraftProjectFromRoute(initialDraftProjectGeneration!),
      )

  // Provisional Meta draft discovery is detached from the critical bootstrap.
  // It may rebind only an untouched draft and never delays ordinary chat.
  if (initialSession.draft) metaDraftRecovery.start(draftAgentId())
  const initialMetaSessionKey = sessionKey.value
  void sessionBootstrap.live.then((outcome) => {
    if (
      outcome.authoritative
      && chatViewActive
      && sessionKey.value === initialMetaSessionKey
    ) {
      if (initialSession.draft) metaDraftRecovery.retry(draftAgentId())
      return Promise.all([
        metaRuns.hydrateRecovery(),
        restoreDurableMetaControls(initialMetaSessionKey),
      ])
    }
  }).catch((error: unknown) => {
    console.warn(
      'Initial Meta recovery failed:',
      error instanceof Error ? error.message : error,
    )
  })
  // The entire dock can grow through attachments, pending work, and textarea
  // autoresize. Publish its real height locally so the thread always reserves
  // exactly enough clearance for the floating surface.
  const composerDock = composerRef.value?.composerElement()?.parentElement ?? null
  if (composerDock && typeof ResizeObserver !== 'undefined') {
    const publishComposerDockHeight = () => {
      const height = Math.ceil(composerDock.getBoundingClientRect().height)
      if (height === lastComposerDockHeight) return
      lastComposerDockHeight = height
      threadRef.value?.style.setProperty('--composer-dock-h', `${height}px`)
      if (autoScroll.value && composerDockPinFrame === null) {
        composerDockPinFrame = requestAnimationFrame(() => {
          composerDockPinFrame = null
          const thread = threadRef.value
          if (thread && autoScroll.value) thread.scrollTop = thread.scrollHeight
        })
      }
    }
    composerDockResizeObserver = new ResizeObserver(publishComposerDockHeight)
    composerDockResizeObserver.observe(composerDock)
    publishComposerDockHeight()
  }

  // Focus textarea on desktop
  if (isDesktopViewport.value) {
    composerRef.value?.focusTextarea()
  }

  if (initialDraftProjectGeneration !== null) {
    const synced = await initialDraftProjectSync
    if (synced && shouldCanonicalizeInitialDraftRoute({
      disposed: chatViewDisposed,
      initialFullPath: initialRouteFullPath,
      currentFullPath: route.fullPath,
      currentPathIsDraft: isDraftRoute(),
      hasLegacyNewChatQuery: hasLegacyNewChatQuery(),
    })) {
      goToDraft({ replace: true })
    }
  }

  // Sessions Hub "Start task" hand-off: send the prefilled draft in one step.
  // Wait for the subscription first so the first turn streams into this view
  // rather than being missed before sessions.messages.subscribe registers.
  if (pendingAutoSend.value && initialAutoSendSnapshot) {
    const text = initialAutoSendSnapshot.text
    const autoSendSessionKey = sessionKey.value
    const autoSendGeneration = sessionBootstrap.generation
    const subscription = await sessionBootstrap.live
    pendingAutoSend.value = ''
    const composerUnchanged = autoSendDraftIsUnchanged(
      text,
      inputText.value,
      initialAutoSendSnapshot.attachments,
      pendingAttachments.value,
      initialAutoSendSnapshot.revision,
      composerRevision.value,
    )
    if (
      !chatViewDisposed
      && sessionKey.value === autoSendSessionKey
      && isSessionBootstrapCurrent(autoSendGeneration, autoSendSessionKey)
      && subscription.authoritative
      && livePhase.value === 'ready'
      && composerUnchanged
    ) {
      sendAutomaticInput()
    } else {
      // Fail closed: the Sessions Hub hand-off remains an editable draft. The
      // inline live-recovery state owns retry and explains why sending paused.
      composerRef.value?.focusTextarea()
    }
  }
})

onUnmounted(() => {
  chatViewActive = false
  chatViewDisposed = true
  forkTransitionLifetime.dispose()
  forkTransition.value = null
  durableRecoveryGeneration += 1
  metaDraftRecovery.invalidate()
  draftProjectHydration.invalidate()
  cancelSessionBootstrap()
  releaseOptionalRpcAdmission?.()
  releaseOptionalRpcAdmission = null
  cancelActiveProjectValidation()
  clearExecutionDockHideTimer()
  unsubs.forEach(fn => fn())
  unsubs = []
  // NOTE: `globalRpcUnsub` is intentionally NOT invoked here. The route-surviving
  // session/task/ensemble_progress subscriptions must keep firing after ChatView
  // leaves the route so the sidebar session list and run-status keep updating in
  // real time. The handle is retained (read below) so it outlives the view.
  void globalRpcUnsub
  cleanupPendingQueue()
  cleanupHistory()
  cleanupSessionArtifacts()
  cleanupStream()
  cleanupCompaction()
  cleanupVoiceInput()
  chatApprovals.cleanup()
  metaRuns.cleanup()
  if (composerDockResizeObserver) {
    composerDockResizeObserver.disconnect()
    composerDockResizeObserver = null
  }
  if (composerDockPinFrame !== null) {
    cancelAnimationFrame(composerDockPinFrame)
    composerDockPinFrame = null
  }
  clearPendingComposerScrollIntent()
  threadRef.value?.style.removeProperty('--composer-dock-h')
  // Drop any live share-preview object URL so the blob can be reclaimed.
  if (sharePreview.value) {
    URL.revokeObjectURL(sharePreview.value.url)
    sharePreview.value = null
  }
})

useDocumentEvent('paste', onDocumentPaste)
useDocumentEvent('keydown', onDocumentKeydown)

// Watch for route changes
watch(() => route.query.session, async (newSession) => {
  durableRecoveryGeneration += 1
  metaDraftRecovery.invalidate()
  applyForkRouteHandoff(newSession)
  if (newSession && typeof newSession === 'string') {
    recordSessionNavigationDiag('route.query.session', {
      from: sessionKey.value,
      to: newSession,
      routeSession: newSession,
    })
    await switchToSession(newSession)
  }
})

// A route switch briefly retains the parent's terminal history status. Require
// the history composable to bind to the child key before treating `ready` as
// hand-off completion, otherwise the preview would disappear one tick early.
watch(
  () => [
    sessionKey.value,
    historySessionKey.value,
    historyState.value.initialLoadStatus,
  ] as const,
  ([activeKey, loadedKey, status]) => {
    syncForkFromHistoryStatus(activeKey, loadedKey, status)
  },
)

watch(
  () => [sessionKey.value, livePhase.value] as const,
  ([activeKey, phase]) => {
    syncForkFromLivePhase(activeKey, phase)
  },
)

// Entering the draft route resets to a clean draft for the requested agent.
watch(() => [route.path, route.query.agent, route.query.project], async () => {
  durableRecoveryGeneration += 1
  metaDraftRecovery.invalidate()
  const generation = draftProjectHydration.begin()
  if (!isDraftRoute()) return
  if (!await syncDraftProjectFromRoute(generation)) return
  enterDraft()
  metaDraftRecovery.start(draftAgentId())
})

watch(inputText, (value) => {
  if (value.length > 0) markProvisionalDraftUsed()
}, { flush: 'sync' })

watch(() => pendingAttachments.value.length, (count) => {
  if (count > 0) markProvisionalDraftUsed()
}, { flush: 'sync' })

watch(() => pendingQueue.value.length, (count) => {
  if (count > 0) markProvisionalDraftUsed()
}, { flush: 'sync' })

// Explicit new-task actions must reset even when navigation targets the exact
// draft URL already on screen (for example, clicking the same project pencil).
watch(freshTaskDraft.request, request => {
  if (!request) return
  draftProjectHydration.invalidate()
  landingPrefilled.value = false
  if (request.workspaceId && rpc.canChooseProject) {
    const workspace = projectWorkspaces.byId.value.get(request.workspaceId)
    if (workspace) {
      activeProjectWorkspace.beginProjectDraft(activeSnapshot(workspace))
    } else {
      activeProjectWorkspace.beginUnknownProjectDraft(request.workspaceId)
    }
  } else {
    activeProjectWorkspace.clearDraft()
  }
  startDraftSession(request.agentId)
  if (isDesktopViewport.value) composerRef.value?.focusTextarea()
})

watch(projectWorkspaces.workspaces, workspaces => {
  if (!rpc.canManageProjectWorkspaces) return
  const workspaceId = boundWorkspaceId.value
  if (!workspaceId) return
  const workspace = workspaces.find(item => item.id === workspaceId) || null
  activeProjectWorkspace.applyWorkspaceRefresh(
    workspace ? activeSnapshot(workspace) : null,
  )
})

watch(
  () => rpc.canChooseProject,
  allowed => {
    if (allowed) return
    projectPickerOpen.value = false
    if (!isDraftRoute() || !readProjectFromUrl()) return
    activeProjectWorkspace.clearDraft()
    freshTaskDraft.requestFreshTask(draftAgentId())
    goToDraft({
      agentId: draftAgentId(),
      projectId: null,
      replace: true,
    })
  },
)

// Legacy ?newChat=1 / ?new=1 links land on the draft route, then the params disappear.
watch(() => [route.query.newChat, route.query.new], () => {
  if (hasLegacyNewChatQuery()) goToDraft({ replace: true })
})

watch(sessionKey, () => {
  pendingForkBeforeMessageId.value = null
  // Retire any in-flight page walk and clear the old Session before starting
  // the new one, so a late response cannot leak deliverables across tabs/routes.
  resetSessionArtifacts()
  if (workbenchEnabled.value) workbenchStore.setSessionScope(sessionKey.value || null)
  if (shareMode.value) endShareMode()
  deliverablesOpen.value = false
  if (sessionKey.value && pendingSessionIntent.value !== 'new_chat') void loadSessionArtifacts()
})

// Hello refreshes method capabilities on reconnect. Retry the durable index
// for the current Session then; older gateways simply remain on history/live.
watch(() => rpc.state, (state, previous) => {
  if (
    state === 'connected'
    && previous !== 'connected'
    && sessionKey.value
    && pendingSessionIntent.value !== 'new_chat'
  ) {
    void loadSessionArtifactsAfterReconnect()
  }
})

watch(shareableMessageCount, (count) => {
  if (count === 0 && shareMode.value) endShareMode()
})

// Router-led turns hold the live answer/activity reveal back for [MIN,MAX] ms,
// then mount a block of content at once. Re-pin the thread on that reveal so it
// lands at the bottom instead of below the fold.
watch(answerRevealOpen, (open) => {
  if (open && autoScroll.value) scrollToBottom()
})

// An approval/clarify interrupt is a user-blocking control, not answer content.
// Reveal it immediately, re-pin the live edge, and keep it outside the
// collapsible activity surface so it cannot disappear while the backend waits.
watch(
  () => visiblePendingInterruptKeys.value,
  (keys, previousKeys = []) => {
    if (!keys.some(key => !previousKeys.includes(key))) return
    revealNow()
    autoScroll.value = true
    scrollToBottom()
  },
  { flush: 'post' },
)
</script>

<style scoped src="../styles/chat-view.css"></style>

<style scoped>
/* No shared sr-only utility exists in this repo (each component scopes its
   own), so the completion announcer's clip-out lives here: zero visual
   footprint, still exposed to assistive tech. */
.chat-turn-settled-announcer {
  position: absolute;
  width: 1px;
  height: 1px;
  padding: 0;
  margin: -1px;
  overflow: hidden;
  clip: rect(0, 0, 0, 0);
  white-space: nowrap;
  border: 0;
}

/* Epic A 内联编辑 / 预览覆盖层 */
.aie-overlay {
  position: fixed;
  inset: 0;
  z-index: 120;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 5vh 5vw;
}
.aie-overlay__backdrop {
  position: absolute;
  inset: 0;
  background: color-mix(in srgb, var(--bg) 60%, transparent);
  backdrop-filter: blur(2px);
}
.aie-overlay__panel {
  position: relative;
  width: min(960px, 92vw);
  height: min(80vh, 720px);
  display: flex;
  background: var(--bg-elevated);
  border: 1px solid var(--border-strong);
  border-radius: var(--radius-lg);
  box-shadow: var(--elev-3);
  overflow: hidden;
}
.aie-overlay__panel > :deep(.aie) { width: 100%; }

.aie-overlay-enter-active, .aie-overlay-leave-active {
  transition: opacity var(--dur-fast, 120ms) var(--ease-standard);
}
.aie-overlay-enter-from, .aie-overlay-leave-to { opacity: 0; }

@media (prefers-reduced-motion: reduce) {
  .aie-overlay-enter-active, .aie-overlay-leave-active { transition: none; }
}
</style>
