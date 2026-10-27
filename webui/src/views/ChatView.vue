<template>
  <div
    class="chat"
    :style="chatRootStyle"
    :class="{
      'chat--new-landing': isNewChatLanding,
      'chat--meta-setup': Boolean(setupState),
      'chat--drag-over': threadDragOver,
      'chat--plan-questionnaire-open': Boolean(dockedPlanQuestionnaire),
      'chat--composer-floating': composerFxEnabled && !isNewChatLanding,
      'chat--composer-collapsed': composerCollapsed && composerFxEnabled && !isNewChatLanding,
      'chat--panel-resizing': rightPanelResizing,
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
      <!-- P3 全域打通：当前模型指示（读 config.get，订阅 models.changed）。
           放在标题之后、动作之前——用户扫一眼就知道这条消息会发给谁。 -->
      <ChatModelIndicator v-if="!isNewChatLanding" class="chat-header__model" />
      <!-- 上下文压力：网关 usage.status 的 contextStatus 越过阈值才出现（见组件注释） -->
      <ChatContextPressureChip
        v-if="!isNewChatLanding && contextWarning"
        :warning="contextWarning"
        @compact="requestManualCompact"
      />
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
        <ModeHintBar />
        <ChatForkTransitionOverlay
          v-if="forkTransition"
          :fork="forkTransition"
          @retry="retryForkTransition"
          @return-to-forkParent="returnToForkParent"
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
          <LandingHero
            :project-name="activeWorkspace?.name || ''"
            :project-path="activeWorkspace?.path || ''"
            :branch="workspaceGit?.branch ?? null"
            :detached="workspaceGit?.detached === true"
          >
            <template #model>
              <ChatModelPicker variant="compact" panel-placement="below" />
            </template>
          </LandingHero>
          <div class="chat-landing-brand" :aria-label="t('chat.newChatBrand')">
            <EmptyStateChips
              :key="landingAgentId"
              :agent-id="landingAgentId"
              :agent-name="landingAgentName"
              :suppressed="landingSuggestionsHidden"
              :disabled="landingSuggestionsDisabled"
              :show-intro="false"
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
          :can-replan="failedTurnReplanTarget !== null"
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
          @open-auth-settings="openAuthSettings"
          @retry-auth-failure="retryAuthFailure"
          @replan-failed-turn="replanFromFailedTurn"
          @view-turn-logs="openTurnLogs"
          @plan-implement-current="implementCurrentPlan"
          @plan-implement-new="implementPlanInNewTask"
          @plan-replan="beginPlanRevision"
          @plan-reuse-last="reuseLastPlan"
          @focus-subagent="openSubagentInSidebar"
          @show-full-message="showFullMessage"
        >
          <template #router-strip="{ message: msg }">
            <RouterFxStrip v-if="shouldRenderRouterStrip(msg)" :message="msg" />
          </template>
        </ChatMessageList>

        <!-- 编排执行时间线：蜂群 / 团队 / 子智能体的成员与工具过程实时可见。
             【2026-10-23 位置修复】原先渲染在输入框上方（PendingQueue 之后），
             用户口径是「卡片一直贴在输入框上，执行完也不进对话」。现在放进消息
             流末尾（`chat-message-surface` 内），随对话一起滚动 —— 执行过程属于
             这一轮对话的一部分，不是输入区的附属控件。 -->
        <div v-if="orchestrationRuns.length" class="chat-orch-runs">
          <OrchestrationRunCard
            v-for="run in orchestrationRunsChronological"
            :key="run.runId"
            :run="run"
            :now="orchestrationNow"
          />
        </div>
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
          :editing="planCardEditing"
          @implement-current="implementCurrentPlan"
          @implement-new="implementPlanInNewTask"
          @replan="beginPlanRevision"
          @reuse-last="reuseLastPlan"
          @step-add="onPlanStepAdd"
          @step-remove="onPlanStepRemove"
          @step-update="onPlanStepUpdate"
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
              <HeartbeatWaitIndicator
                :visible="waitIndicatorVisible"
                :phase-label="streamPhaseLabel"
                :seconds="streamTurnElapsedSeconds"
                :suspended="stallSuspendReason !== null"
                :suspend-label="waitSuspendLabel"
              />
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
                live
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
        <TurnSuggestions
          v-if="showTurnSuggestions"
          :can-revert="turnChangeStats.canRevert"
          @view="fileChangesOpen = true"
          @run-tests="prefillComposerFromTurnSuggestion(t('chat.turnSuggestions.runPrompt'))"
          @revert="turnChangeSummaryRef?.requestRevert()"
        />
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
    <!-- Durable execution progress belongs to the work surface, not to the
         transcript. Keeping it immediately above the composer also lets a
         future goal driver reuse this dock across multiple turns. -->
    <Transition name="plan-run-dock">
      <div v-if="executionDockRun" class="plan-run-dock">
        <PlanRunRibbon
          :run="executionDockRun"
          :cancel-busy="planActionPending === 'cancel-run'"
          :disabled="planModeBusy || planActionPending !== null"
          :step-kind="chatPlans.activeStepKind.value"
          :timing="chatPlans.runTiming.value"
          @cancel="cancelActivePlanRun"
          @focus-return="focusComposerAfterPlanRun"
        />
      </div>
    </Transition>
    <!-- M2-C4：护栏升级「需要确认」浮层。存在 pending escalation 时置顶呈现，
         用户选择 继续/中止 后由 escalation.ack 决策并弹出队列下一项。 -->
    <Transition name="escalation-dock">
      <div v-if="chatEscalations.pending.value" class="plan-run-dock escalation-dock">
        <EscalationCard
          :event="chatEscalations.pending.value"
          :busy="chatEscalations.busy.value"
          @ack="decision => void chatEscalations.resolve(chatEscalations.pending.value!, decision)"
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
      @send-now="sendPendingNow"
      @move="movePendingMessage"
    />

    <ChatOrchestrationStatus :items="orchestrationItems" />

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

    <!-- 本回合文件改动：把「改了几个文件、多少行」和「全部保留 / 全部撤销」
         收在输入框正上方，省掉一次打开抽屉再逐条裁决。 -->
    <TurnChangeSummary
      ref="turnChangeSummaryRef"
      v-if="lastTurnChangeSince !== null"
      :key="String(lastTurnChangeSince)"
      :session-key="sessionKey"
      :since="lastTurnChangeSince"
      @view="fileChangesOpen = true"
      @reverted="focusComposer"
      @stats="turnChangeStats = $event"
    />

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
      :safe-setup-state="runModeSetupState"
      :run-mode-locked="runModeLocked"
      :run-mode-lock-message="t('chat.composer.runModeLocked')"
      :model-routing-mode="modelRoutingMode"
      :model-routing-settings-busy="modelRoutingSettingsBusy"
      :effort="composerEffort"
      :effort-wire-hint="effortWireHint"
      :orchestration="orchestration"
      :coding-mode-enabled="codingModeEnabled"
      :coding-mode-settings-busy="codingModeSettingsBusy"
      :voice-busy="voiceBusy"
      :voice-recording="voiceRecording"
      :voice-ready="voiceReady"
      :project-workspace="activeWorkspace"
      :project-workspace-status="activeWorkspaceStatus"
      :workspace-git="workspaceGit"
      :project-status-message="activeProjectStatusMessage"
      :can-close-project="isDraftRoute() && pendingWorkspaceId !== null"
      :can-choose-project="rpc.canChooseProject"
      :can-attach-project-roots="rpc.canManageProjectWorkspaces"
      :project-roots="projectRootChips"
      :project-root-options="projectRootOptions"
      :primary-project-id="activeWorkspace?.id || ''"
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
      :show-presets="true"
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
      @set-effort="setComposerEffort"
      @update-orchestration="applyOrchestrationPatch"
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
      @apply-project-roots="projectRoots.applySelection($event, activeWorkspace?.id)"
      @remove-project-root="projectRoots.removeExtraId"
      @open-commands="openComposerCommands"
      @open-artifacts="focusSidebarTab('artifacts')"
      @open-file-tree="focusSidebarTab('tree')"
      @open-skills="void router.push('/skills')"
      @open-plugins="void router.push('/plugins')"
      @open-prompt-cache-keepalive="promptCacheKeepaliveOpen = true"
      @refresh-prompt-cache-keepalive="void refreshPromptCacheKeepaliveStatus()"
    />
    <!-- Usage telemetry: small-print footnote rendered BELOW the composer's
         "内容由 AI 生成" disclaimer, so runtime stats never compete with input.
         数据来自网关每轮真实下发的 `session.event.usage`（token 与缓存命中都在
         这一帧里）；没有真实用量时整条不渲染，不再拿全为 0 的全局计数冒充本会话。 -->
    <div
      v-if="sessionUsage.total > 0 || lastTtftMs !== null || runModeDegradedReason"
      class="chat-usage-strip chat-usage-strip--foot"
      data-testid="chat-usage-strip"
    >
      <span class="chat-usage-strip__item">{{ t('chat.usage.ttft', { ms: lastTtftMs === null ? '—' : lastTtftMs + ' ms' }) }}</span>
      <span class="chat-usage-strip__item">{{ t('chat.usage.sessionTokens', { n: sessionUsage.total }) }}</span>
      <span class="chat-usage-strip__item">{{ t('chat.usage.rounds', { n: sessionUsage.turns }) }}</span>
      <span v-if="sessionUsage.cacheHitPct !== null" class="chat-usage-strip__item">{{ t('chat.usage.cacheHitRate', { pct: sessionUsage.cacheHitPct }) }}</span>
      <!-- 真实能力降级：安全模式未就绪时如实说明，不在页脚假装已隔离。 -->
      <span
        v-if="runModeDegradedReason"
        class="chat-usage-strip__item chat-usage-strip__item--warn"
        data-testid="chat-run-mode-degraded"
      >{{ runModeDegradedReason }}</span>
    </div>
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

    <FullMessageModal
      :open="fullMessageModal.open"
      :content="fullMessageModal.content"
      @close="fullMessageModal.open = false"
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
      :focus-tab="sidebarFocus.tab"
      :focus-seq="sidebarFocus.seq"
      :focus-subagent-key="sidebarSubagentFocus"
      :tasks="sidebarTasks"
      :subagents="sidebarSubagents"
      :artifacts="sessionArtifacts"
      :changes="sidebarChanges"
      :root-path="activeWorkspace?.path"
      :session-key="sessionKey"
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
          :aria-label="t('chat.artifactEditorAria')"
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
import { useAgentOptions } from '@/composables/useAgentOptions'
import { useChatFontScale } from '@/composables/useChatFontScale'
import { useAppStore } from '@/stores/app'
import { useSandboxSetupStore } from '@/stores/sandboxSetup'
import { useWorkbenchStore } from '@/workbench/store'
import { usePlatform } from '@/platform'
import ApprovalCard from '@/components/chat/ApprovalCard.vue'
import ActivityDisclosure from '@/components/chat/ActivityDisclosure.vue'
import HeartbeatWaitIndicator from '@/components/chat/HeartbeatWaitIndicator.vue'
import AssistantActivityTimeline from '@/components/chat/AssistantActivityTimeline.vue'
import ChatArtifactList from '@/components/chat/ChatArtifactList.vue'
import ChatForkTransitionOverlay from '@/components/chat/ChatForkTransitionOverlay.vue'
import ChatHeaderActions from '@/components/chat/ChatHeaderActions.vue'
import ChatModelIndicator from '@/components/chat/ChatModelIndicator.vue'
import ChatContextPressureChip from '@/components/chat/ChatContextPressureChip.vue'
import ModeHintBar from '@/components/ModeHintBar.vue'
import PromptCacheKeepaliveDialog from '@/components/chat/PromptCacheKeepaliveDialog.vue'
import DeliverablesDrawer from '@/components/chat/DeliverablesDrawer.vue'
import FileChangesDrawer from '@/components/chat/FileChangesDrawer.vue'
import TurnChangeSummary from '@/components/chat/TurnChangeSummary.vue'
import TurnSuggestions from '@/components/chat/TurnSuggestions.vue'
import RightSidebar from '@/components/chat/RightSidebar.vue'
import ArtifactInlineEditor from '@/components/chat/ArtifactInlineEditor.vue'
import type { RsArtifact } from '@/components/chat/RightSidebar.vue'
import ChatComposer from '@/components/chat/ChatComposer.vue'
import ChatOrchestrationStatus from '@/components/chat/ChatOrchestrationStatus.vue'
import OrchestrationRunCard from '@/components/chat/OrchestrationRunCard.vue'
import { useOrchestrationActivity } from '@/composables/chat/useOrchestrationActivity'
import { useOrchestrationRuns } from '@/composables/chat/useOrchestrationRuns'
import { activeUserMode } from '@/composables/useActiveMode'
import { useHostExitEvidence } from '@/composables/useHostExitEvidence'
import {
  EXPERT_TEAMS,
  loadStoredOrchestration,
  persistOrchestration,
  shouldSendDispatchBrief,
  type OrchestrationState,
} from '@/composables/chat/orchestration'
import ProjectWorkspacePickerDialog from '@/components/ProjectWorkspacePickerDialog.vue'
import ChatMessageList from '@/components/chat/ChatMessageList.vue'
import ChatSessionRecoveryStatus from '@/components/chat/ChatSessionRecoveryStatus.vue'
import ChatStallNotice from '@/components/chat/ChatStallNotice.vue'
import ClarifyCard from '@/components/chat/ClarifyCard.vue'
import ConversationMinimap from '@/components/chat/ConversationMinimap.vue'
import EmptyStateChips from '@/components/chat/EmptyStateChips.vue'
import LandingHero from '@/components/chat/LandingHero.vue'
import ChatModelPicker from '@/components/chat/ChatModelPicker.vue'
import InterruptPart from '@/components/chat/parts/InterruptPart.vue'
import ReasoningPart from '@/components/chat/parts/ReasoningPart.vue'
import TextPart from '@/components/chat/parts/TextPart.vue'
import MetaPreflightCard from '@/components/chat/MetaPreflightCard.vue'
import MetaRibbon from '@/components/chat/MetaRibbon.vue'
import MetaSkillSetupCard from '@/components/chat/MetaSkillSetupCard.vue'
import PendingQueue from '@/components/chat/PendingQueue.vue'
import PlanCard from '@/components/chat/PlanCard.vue'
import PlanRunRibbon from '@/components/chat/PlanRunRibbon.vue'
import EscalationCard from '@/components/chat/EscalationCard.vue'
import RouterFxStrip from '@/components/chat/RouterFxStrip.vue'
import SharePreviewModal from '@/components/chat/SharePreviewModal.vue'
import SandboxSetupDialog from '@/components/sandbox/SandboxSetupDialog.vue'
import ToolResultModal from '@/components/chat/ToolResultModal.vue'
import FullMessageModal from '@/components/chat/FullMessageModal.vue'
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
  resolveComposerRunMode,
  type ComposerRunModeDegradation,
} from '@/composables/chat/composerRunMode'
import { useSandboxSetupRecovery } from '@/composables/chat/useSandboxSetupRecovery'
import { useChatStallWatchdog } from '@/composables/chat/useChatStallWatchdog'
import { useArtifactImageLightbox } from '@/composables/chat/useArtifactImageLightbox'
import { useMetaRuns } from '@/composables/chat/useMetaRuns'
import { useMetaSkillSetup } from '@/composables/chat/useMetaSkillSetup'
import { useChatPlans } from '@/composables/chat/useChatPlans'
import { useChatEscalations } from '@/composables/chat/useChatEscalations'
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
import { useChatSidebarPanels } from '@/composables/chat/useChatSidebarPanels'
import { usePanelResize } from '@/composables/usePanelResize'
import { useVoiceInput } from '@/composables/chat/useVoiceInput'
import { useDocumentEvent } from '@/composables/useDocumentEvent'
import { hasOpenDialogLayer } from '@/composables/useDialogA11y'
import { useToasts } from '@/composables/useToasts'
import { notifyTurnEnded } from '@/composables/useTurnNotification'
import { usePartialTextRestore } from '@/composables/chat/usePartialTextRestore'
import { useConfirm } from '@/composables/useConfirm'
import { useProjectWorkspaces } from '@/composables/useProjectWorkspaces'
import { useWorkspaceStatus } from '@/composables/sessions/useWorkspaceStatus'
import {
  createDraftProjectHydrationGuard,
  useActiveProjectWorkspace,
} from '@/composables/useActiveProjectWorkspace'
import { useFreshTaskDraft } from '@/composables/useFreshTaskDraft'
import {
  SESSION_LIST_VIEW,
  itemKey,
  normalizeSessionItem,
} from '@/composables/useSessions'
import {
  projectRootChips as projectRootChipsFrom,
  useChatProjectRoots,
} from '@/composables/chat/useChatProjectRoots'
import type {
  Attachment,
  ChatMaintenanceEvent,
  ChatMessage,
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
  ChatDispatchBrief,
  MetaDraftDiscardResponse,
  SessionEventPayload,
  SessionMessagesSnapshotResponse,
  SessionsListResponse,
} from '@/types/rpc'
import type { ModelRoutingMode } from '@/types/modelRouting'
import {
  effortFromThinking,
  isEffortLevel,
  thinkingFromEffort,
  type EffortLevel,
} from '@/types/effort'
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
import { isLastTurnSuccessful, resolveLastTurnStartTs } from '@/utils/chat/lastTurnStart'
import { subagentPanelEntries } from '@/utils/chat/subagent'
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

const fullMessageModal = ref<{ open: boolean; content: string }>({ open: false, content: '' })

function showFullMessage(message: ChatRenderedMessage) {
  const content = message.text || ''
  if (!content.trim()) return
  fullMessageModal.value = { open: true, content }
}
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

// 编辑器所需的会话标识与同源信息（与预览器一致，走 window 同源）
const currentSessionKey = computed(() => String((route.params as Record<string, unknown>).sessionKey || ''))
const baseOrigin = window.location.origin
function rpcAuthToken(): string {
  const token = (rpc as unknown as { _token?: string })._token
  return token || ''
}


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
// 对话页字号：只作用于本视图根节点（`.chat`），页内 rem 字号随之按比例级联。
const { fontPx: chatFontPx } = useChatFontScale()
const router = useRouter()
const { pushToast } = useToasts()
const { confirm } = useConfirm()
const projectWorkspaces = useProjectWorkspaces()
const activeProjectWorkspace = useActiveProjectWorkspace()
// 附加项目（P2-9 多根）：会话级状态放在单一组合式实例里，ChatComposer 通过
// props/事件读写同一份数据，不允许两边各存一份。
const projectRoots = useChatProjectRoots()
// 会话身份一变（切换会话 / 进入草稿 / 草稿落库换了 key）就先清空附加项目，
// 再等新会话元数据里的 projectPaths 回填 —— 这是唯一的跨会话泄漏入口。
watch(sessionKey, () => { projectRoots.resetForSession() })
const projectRootOptions = computed(() => projectWorkspaces.workspaces.value)
const projectRootChips = computed(() => projectRootChipsFrom(
  projectRoots.extraIds.value,
  projectRootOptions.value,
))
// 会话列表是 `projectPaths`（多根）唯一稳定的下发点；会话订阅快照只有单值
// workspace。元数据没带多根时按 key 回填一次，避免 chips 在旧网关上永远为空。
let projectRootsHydrationSequence = 0
async function hydrateProjectRootsFromSessionList(key: string) {
  if (!key) return
  const sequence = ++projectRootsHydrationSequence
  try {
    const response = await rpc.call<SessionsListResponse>(
      'sessions.list',
      { limit: 200, view: SESSION_LIST_VIEW },
    )
    if (sequence !== projectRootsHydrationSequence || key !== sessionKey.value) return
    const raw = (response?.sessions || response?.keys || [])
      .find(entry => itemKey(entry) === key)
    const item = normalizeSessionItem(raw)
    projectRoots.applyProjectPaths(
      item?.projectPaths,
      projectWorkspaces.workspaces.value,
    )
  } catch {
    // 列表读取失败保持「无附加项目」：下一轮元数据或会话切换会再试，绝不阻断对话。
  }
}
const draftProjectHydration = createDraftProjectHydrationGuard()
const {
  pendingWorkspaceId,
  boundWorkspaceId,
  activeWorkspace,
  status: activeWorkspaceStatus,
} = activeProjectWorkspace
// 输入框上下文条的分支胶囊：`workspace.status` 已返回权威解析 + 只读 Git 摘要，
// 这里只做一次拉取并随会话/绑定目录变化刷新，不另造第二个数据源。
const workspaceStatus = useWorkspaceStatus()
const workspaceGit = computed(() => workspaceStatus.status.value?.git ?? null)
watch(
  () => [activeWorkspace.value?.path || '', sessionKey.value],
  () => { void workspaceStatus.load(activeWorkspace.value?.path) },
  { immediate: true },
)
// 项目校验控制器（activeProjectValidationController / cancelActiveProjectValidation）
// 已迁入 useChatProjectController，于装配层 chatSessionRuntime 之后接线。
const isCompactViewport = useMediaQuery('(max-width: 480px)')
const isDesktopViewport = useMediaQuery('(min-width: 769px)')
const landingAgentId = computed(() => agentIdFromSessionKey(sessionKey.value))
/**
 * 落地页身份行：首条消息之前还没有 `router_decision` 事件，会话头的
 * `AgentPersonaBadge` 是空的——这里补上「正在和谁对话」。
 * 只对主智能体渲染（`chat.mainAgent` 是既有文案键，此前一直没人用）；
 * 其他专家的身份由会话头徽章负责，避免同一事实两处不同措辞。
 * 取不到名字（agents.list 未回来 / 只有 `Main Agent` 占位名）时整行不渲染。
 */
const { agents: landingAgents } = useAgentOptions()
const landingAgentName = computed(() => {
  if (landingAgentId.value !== 'main') return ''
  const name = (landingAgents.value.find(a => a.id === 'main')?.name || '').trim()
  return name && name !== 'Main Agent' ? name : ''
})
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
/**
 * 展示模式 + 降级原因。
 *
 * 修复前：setup 未 ready 时这里把 safe 静默改写成 full，同时把 safe 从
 * `allowedRunModes` 剔除 → 原生 disabled 按钮不派发 click（「点了没反应」）。
 * 现在保留用户的 safe 选择，只把「隔离运行时未就绪」作为 degraded 原因展示。
 */
const runModeResolution = computed(() => resolveComposerRunMode(
  globalRunMode.value,
  sandboxSetupStatus.value,
  activeRunModeLock.value,
  sandboxSetupRecovery.resolved.value,
))
const runMode = computed<SandboxRunMode>(() => runModeResolution.value.mode)
const runModeSetupState = computed(() => sandboxSetupStatus.value?.state ?? null)
// safe 仍受网关策略的硬约束（auth.runModePolicy.allowedRunModes），
// 但不再因「setup 未就绪」被剔除。
const composerAllowedRunModes = computed<SandboxRunMode[]>(() => allowedRunModes.value)
const composerSafeSetupAvailable = computed(() => sandboxSetupRecovery.canSetup.value)

const runModeDegradation = computed<ComposerRunModeDegradation | null>(
  () => runModeResolution.value.degraded,
)
/**
 * 页脚降级说明：只在「用户确实选了 safe、但后端确认无法隔离」时出现。
 * 文案直接引用后端 `sandbox.setup.status.message`，不伪造能力。
 */
const runModeDegradedReason = computed(() => {
  const reason = runModeDegradation.value
  if (reason === null) return ''
  if (reason === 'setup_unresolved') {
    return t('settings.sandbox.safeMode.degradedUnresolved')
  }
  const backendMessage = (sandboxSetupStatus.value?.message || '').trim()
  return backendMessage
    ? t('settings.sandbox.safeMode.degraded', { reason: backendMessage })
    : t('settings.sandbox.safeMode.degradedFallback')
})

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
      pushToast(t('chat.recoveringGeneration'), { tone: 'info' })
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
  streamTurnElapsedSeconds,
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
// ── 首 token（客户端实测：发送 → 首个可见输出；含网关排队/编排开销）──
const ttftStartMs = ref(0)
const lastTtftMs = ref<number | null>(null)
watch(streamHasVisibleOutput, (visible) => {
  if (visible && ttftStartMs.value > 0) {
    lastTtftMs.value = Math.round(performance.now() - ttftStartMs.value)
    ttftStartMs.value = 0
  }
})
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
  contextWarning,
  resetSavingsPopupCooldown,
  saveWidgetState,
  restoreWidgetState,
  loadCurrentSessionUsage,
} = chatUsageWidget

/**
 * 底部计量条：token 来自 `session.event.usage` 帧的实时累加（每次真实 LLM 调用
 * 一条），轮次按本会话已完成的回答数计。
 *
 * 为什么不再用 `usage.status`：Rust 台账按 `usage_events.session` 过滤，而绝大多数
 * 事件写入时 session 是空串（实测 10434 行中 6982 行 session=''），任何会话查出来
 * 都是 0；`cache.status` 的 prefix 计数在默认路径上也全为 0。两个 0 拼在一起就是
 * 用户看到的「假仪表」。缓存命中率只在真的观测到 cached>0 时才显示。
 */
const sessionUsage = computed(() => {
  const input = Number(usageAccum.value.input || 0)
  const output = Number(usageAccum.value.output || 0)
  const cached = Number(usageAccum.value.cacheRead || 0)
  const turns = (messages.value as ChatMessage[]).filter(
    (m) => m.role === 'assistant' && String(m.text || '').trim().length > 0,
  ).length
  return {
    input,
    output,
    cached,
    turns,
    total: input + output,
    // 与用量页同口径：cache_read / (prompt_tokens + cache_read)，避免两处显示不同命中率。
    cacheHitPct: cached > 0 && input + cached > 0
      ? Math.round(Math.min(1, cached / (input + cached)) * 100)
      : null,
  }
})

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
// M2-C4：护栏 escalation「需要确认」事件流（同一会话内一条 pending 卡 + 队列）。
const chatEscalations = useChatEscalations({
  rpc,
  sessionKey,
  notifyError: message => pushToast(message, { tone: 'danger', duration: 8000 }),
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
  stepEditAdd: planStepEditAdd,
  stepEditRemove: planStepEditRemove,
  stepEditUpdate: planStepEditUpdate,
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
    if (!activeProjectWorkspace.applySessionSnapshot(key, generation, metadata)) return
    // 多根回填：只认当前会话的元数据，过期快照不得覆盖新会话的附加项目。
    if (key === sessionKey.value) {
      projectRoots.applyProjectPaths(metadata.projectPaths, projectWorkspaces.workspaces.value)
      if (!metadata.projectPaths) void hydrateProjectRootsFromSessionList(key)
    }
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

/** 编排（蜂群 / 专家团 / 子智能体）是否仍在跑：决定「停止」按钮是否可用。 */
const orchestrationRunningCount = ref(0)

const sessionHasActiveWork = computed(() => (
  isStreaming.value
  || activeTaskGroups.value.size > 0
  || isCompactInFlightForCurrentSession()
  || ['queued', 'running', 'approval_pending'].includes(runStatus.value.status)
  || activePlanRun.value?.status === 'queued'
  || activePlanRun.value?.status === 'running'
  // 编排跑在独立后台任务里，不能因为「对话流没在推送」就隐藏停止按钮 ——
  // 否则蜂群/专家团执行期间用户没有任何中止入口（真机反馈）。
  || orchestrationRunningCount.value > 0
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

/* ── 输入框「+」菜单的执行端 ─────────────────────────────────────────
 * 菜单只负责发意图，具体动作在这里接真实能力：
 *   命令 → 把输入框置为 `/` 并打开斜杠命令清单（与手输 `/` 完全同一条路径）；
 *   我的产物 / 工作区文件 → 打开右侧栏对应标签；
 *   技能 / 插件 → 进入对应管理页，可真实启停。
 */
const sidebarFocus = ref<{ tab: 'tasks' | 'subagents' | 'artifacts' | 'changes' | 'tree'; seq: number }>({
  tab: 'tasks',
  seq: 0,
})
const sidebarSubagentFocus = ref('')

function focusSidebarTab(tab: 'tasks' | 'subagents' | 'artifacts' | 'changes' | 'tree') {
  sidebarFocus.value = { tab, seq: sidebarFocus.value.seq + 1 }
  rightSidebarOpen.value = true
}

/** 消息流里的子智能体卡 → 右栏第 5 轴；空 key 只打开列表，不做过滤。 */
function openSubagentInSidebar(payload: { key: string }) {
  sidebarSubagentFocus.value = payload?.key || ''
  focusSidebarTab('subagents')
}

function openComposerCommands() {
  const current = inputText.value
  if (!current.startsWith('/')) {
    inputText.value = current.trim() ? `/${current}` : '/'
  }
  handleSlashInput()
  expandComposer()
  void nextTick(() => composerRef.value?.focusTextarea())
}

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

// 推理等级（reasoning effort）：输入框拖动条的唯一状态源。
// 旧实现只有二值 `thinking`（localStorage），表达不了后端的四档；
// 现在档位既写本地（新会话默认值），也写服务端 `session.thinking.*`（跨设备/重装可恢复）。
const COMPOSER_EFFORT_KEY = 'chat.composer.effort'
/** 旧键：仅用于一次性迁移（开=深度、关=关闭），迁移后不再写入。 */
const LEGACY_COMPOSER_THINKING_KEY = 'chat.composer.thinking'

function readInitialEffort(): EffortLevel {
  try {
    const stored = localStorage.getItem(COMPOSER_EFFORT_KEY)
    if (stored && isEffortLevel(stored)) return stored
    if (localStorage.getItem(LEGACY_COMPOSER_THINKING_KEY) === 'true') {
      return effortFromThinking(true)
    }
  } catch {
    // localStorage 不可用（隐私模式/配额满）→ 用默认档位，不阻断输入。
  }
  return 'off'
}

const composerEffort = ref<EffortLevel>(readInitialEffort())

/**
 * 上一轮请求实际发出的思考参数（`model.effort.last`）。
 *
 * 用户反馈「推理档位是玩具」——根因是部分模型不在后端支持名单时被**静默跳过**。
 * 这里把真实结果拿回来展示：注入了哪个字段、值是什么，或明确说「该模型不支持」。
 */
const effortWire = ref<Record<string, unknown> | null>(null)

async function refreshEffortWire() {
  if (!rpc.supportsMethod('model.effort.last')) return
  try {
    effortWire.value = (await rpc.call('model.effort.last')) as Record<string, unknown>
  } catch {
    effortWire.value = null
  }
}

const effortWireHint = computed(() => {
  const raw = effortWire.value
  if (!raw || raw.available !== true) return ''
  // 同一回合里还有预热/摘要等辅助请求，它们会覆盖「最近一次」快照。
  // 因此优先取最近一条 `turnScoped` 记录（= 主回复那次请求）来判定档位是否真的发出。
  const recent = Array.isArray(raw.recent) ? (raw.recent as Record<string, unknown>[]) : []
  const scoped = [...recent].reverse().find((e) => e && e.turnScoped === true)
  const wire = (scoped || raw) as Record<string, unknown>
  const model = String(wire.model || '')
  if (wire.supported === false) {
    return t('chat.effort.wireUnsupported', { model })
  }
  if (wire.injected === true) {
    return t('chat.effort.wireInjected', {
      model,
      field: String(wire.field || ''),
      value: String(wire.value || ''),
    })
  }
  return t('chat.effort.wireOff', { model })
})

function persistEffortLocally(level: EffortLevel) {
  try {
    localStorage.setItem(COMPOSER_EFFORT_KEY, level)
    localStorage.removeItem(LEGACY_COMPOSER_THINKING_KEY)
  } catch {
    // 静默降级：仅本次会话有效。
  }
}

/**
 * 把档位写回服务端（best-effort）。
 *
 * 为什么不在发送时顺带持久化：`chat.send` 带 `effort` 时确实会落盘，但用户
 * **只调滑块不发送**时那条路径不会触发；而滑块是「先设好再写」的控件，
 * 必须自己把状态存住，否则切会话回来档位就丢了。
 */
function pushEffortToServer(level: EffortLevel) {
  const key = sessionKey.value
  if (!key || !rpc.supportsMethod('session.thinking.set')) return
  void rpc
    .call('session.thinking.set', { sessionKey: key, effort: level })
    .catch(() => {
      // 服务端不可用不阻断 UI：本地已存，下一轮 chat.send 仍会带上 effort。
    })
}

function setComposerEffort(level: EffortLevel) {
  composerEffort.value = level
  persistEffortLocally(level)
  pushEffortToServer(level)
}

/** 会话切换/恢复时以服务端档位为准（服务端没有记录则保留本地默认值）。 */
async function syncEffortFromServer() {
  const key = sessionKey.value
  if (!key || !rpc.supportsMethod('session.thinking.get')) return
  try {
    const res = await rpc.call<{ effort?: string | null }>('session.thinking.get', {
      sessionKey: key,
    })
    const serverEffort = res?.effort
    // `null/undefined` = 该会话从未设过档位：**不能**当成 off 覆盖用户偏好。
    if (!serverEffort || !isEffortLevel(serverEffort)) return
    if (serverEffort === composerEffort.value) return
    composerEffort.value = serverEffort
    persistEffortLocally(serverEffort)
  } catch {
    // 读取失败保持现状（离线/侧车未起都能正常对话）。
  }
}

// `immediate`：页面重开/刷新后要立刻采用服务端档位（否则滑块停在本地默认值，
// 与「上一轮实际生效的档位」不一致，用户会以为档位丢了）。
watch(sessionKey, () => {
  void syncEffortFromServer()
}, { immediate: true })

/** 二值 `thinking` 语义保留：非 off 即视为开（模式/编排仍读它）。 */
const composerThinking = computed(() => thinkingFromEffort(composerEffort.value))

// 编排简报（模式/智能体/蜂群/团队/能力）：每轮随 chat.send 上行，后端据此要求
// 主智能体先出「调度计划」再执行。用 ref + watch 回填（orchestration 在下方才定义，
// 这里不能直接依赖它的值；传 ref 给 useChatSend 即可）。
const dispatchBrief = ref<ChatDispatchBrief | null>(null)

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
  enableThinking: composerThinking,
  reasoningEffort: composerEffort,
  elevatedMode,
  runMode,
  pendingAttachments,
  composerRevision,
  pendingSessionIntent,
  pendingWorkspaceId,
  boundWorkspaceId,
  pendingWorkspaceIds: projectRoots.extraIds,
  dispatchBrief,
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
  ttftStartMs.value = performance.now()
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
    // P2·E：编排形态（子智能体/团队/专家/蜂群）走 orchestrate.run 统一入口；
    // single 保持原 chat.send 路径，零额外开销。
    const orch = orchestration.value
    if (orch.form !== 'single' && rpc.supportsMethod('orchestrate.run')) {
      const message = inputText.value.trim()
      if (!message) return
      // 网关没连上 / 宿主不在位时先别弹确认框：确认完也发不出去，
      // 用户只会觉得"按了确认没用"（真机反馈）。
      if (!rpc.isConnected || hostProcessDown.value) {
        pushToast(t('chat.orchestration.offline'), { tone: 'warn', duration: 6000 })
        return
      }
      // 高自主形态先弹确认框；取消 = 原样保留输入，一个字节都不发。
      if (!(await confirmHighAutonomySend(orch))) return
      await dispatchOrchestratedSend(
        HIGH_AUTONOMY_FORMS.has(orch.form) ? { ...orch, confirmed: true } : orch,
        message,
      )
      return
    }
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

/**
 * 直接发送：把该条提到可见队首并立即尝试投递。
 * 空闲时队列会当场发出；正在执行时它成为本轮结束后的第一条（不打断当前任务）。
 */
function sendPendingNow(index: number) {
  const item = takeVisiblePendingItem(index)
  if (!item) return
  const head = pendingQueue.value.findIndex(
    candidate => !candidate.hiddenControl && !candidate.deliveryState,
  )
  pendingQueue.value.splice(head < 0 ? 0 : head, 0, item)
  schedulePendingDrainAfterTerminal()
  flushDeferredPendingDrain()
}

/** 队列内上下调序：只交换普通条目，机器控制项与投递中的条目保持原位。 */
function movePendingMessage(index: number, delta: -1 | 1) {
  const item = pendingQueue.value[index]
  if (!item || item.hiddenControl || item.deliveryState) return
  let target = index + delta
  while (target >= 0 && target < pendingQueue.value.length) {
    const candidate = pendingQueue.value[target]
    if (!candidate.hiddenControl && !candidate.deliveryState) break
    target += delta
  }
  if (target < 0 || target >= pendingQueue.value.length) return
  pendingQueue.value.splice(index, 1)
  pendingQueue.value.splice(target, 0, item)
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
  hasActiveInterrupt: computed(() =>
    Array.from(interruptState.value.values()).some(state => !state.resolution)),
  reappendInterruptFrames: chatApprovals.appendPendingInterrupts,
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
    sidebarPanels.resyncArtifactsAfterTurn()
    // 回合结束才有「本轮实际发出的思考参数」：刷新它，让档位的真实效果可核对。
    void refreshEffortWire()
  }
})

// Soft content-silence watchdog: after the high negotiated threshold, surface
// a neutral long-running notice. Backend-deadline-owned Ensemble phases remain
// suppressed, while the hard idle timer continues to mean no events at all.
const stallWatchdog = useChatStallWatchdog({ isStreaming, streamIdleGraceMs: streamIdleTimeoutMs })
const { stallActive, stallSeconds, suspendReason: stallSuspendReason } = stallWatchdog

// F-09：等待指示器的派生（纯展示组件 HeartbeatWaitIndicator）。
// 秒数与「正在工作」共用同一个回合时钟（N1），因此不会自相矛盾、也不重置
// 软看门狗的内容静默时钟。挂起态与看门狗的 suspendReason 严格一致。
const waitSuspendLabel = computed(() => {
  switch (stallSuspendReason.value) {
    case 'tool-running': return String(t('chat.waitIndicator.toolRunning'))
    case 'approval-pending': return String(t('chat.waitIndicator.approvalPending'))
    case 'ensemble-running': return String(t('chat.waitIndicator.ensemble'))
    default: return ''
  }
})
const waitIndicatorVisible = computed(() =>
  isStreaming.value && (stallSuspendReason.value !== null || !streamHasVisibleOutput.value),
)

const noop = () => {}

// Late-bound: onArtifactUpdated 由 useChatSidebarPanels 创建（装配在产物索引与
// 打开动作就绪之后），两处 RPC 订阅接线先持占位、装配后立即回填（本文件既有惯例）。
let onArtifactUpdated: (payload: {
  session_key?: string
  sessionKey?: string
  key?: string
  id?: string | number
}) => void = () => {}

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
  // Thunk, not the value: `onArtifactUpdated` is late-bound after
  // useChatSidebarPanels is assembled (line ~3458). Passing the value here would
  // freeze the placeholder no-op and silently drop every `artifact.updated`
  // frame, so a registered file would only ever appear after a full reload.
  onArtifactUpdated: (payload) => onArtifactUpdated(payload),
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
  // T04：这三类事件只驱动 live 活动时间线，归 chat-view 订阅独占（避免双处理）。
  onMemoryRecall: noop,
  onStepStart: noop,
  onStepFinish: noop,
  // Late-bound thunk (same reason as the chat-view subscription above).
  onArtifactUpdated: (payload) => onArtifactUpdated(payload),
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

// ── P2·E 编排状态：ChatComposer 四按钮收集 → ChatView 提交 orchestrate.run ──
// 初值取本地持久化的上次选择（形态/蜂群/子智能体/团队/能力），
// 避免每次进入对话页都要重选一遍。
const orchestration = ref(loadStoredOrchestration())

function applyOrchestrationPatch(patch: Partial<OrchestrationState>) {
  orchestration.value = { ...orchestration.value, ...patch }
}

/**
 * 门禁档位以**全局模式 store**为准。
 *
 * 真机缺陷（2026-10-23，浏览器实测）：顶栏/模式预设把运行模式切到「标准·执行」，
 * 顶栏徽标与状态条都变了，但输入框编排面板里的档位仍是旧的「标准·方案」，
 * 于是蜂群先弹确认框、确认后被后端 `ORCHESTRATE_DENIED` 硬拒 —— 用户被带进死胡同。
 *
 * 根因是同一个概念有两个可写副本（模式 store ↔ 编排面板的 `modeKey`）。这里把
 * 发送门禁与上抛参数都改成读 store（顶栏、ModeMatrix、编排面板都写它），
 * 编排面板的副本只作为 store 尚未 hydrate 时的兜底。
 */
const gearKey = computed(() => activeUserMode.value?.key || orchestration.value.modeKey)

// 反向同步：全局档位一变，立刻回填编排面板，面板高亮不再与顶栏打架。
watch(
  activeUserMode,
  (um) => {
    const key = um?.key
    if (key && key !== orchestration.value.modeKey) applyOrchestrationPatch({ modeKey: key })
  },
  { immediate: true },
)

watch(
  orchestration,
  (state) => persistOrchestration(state),
  { deep: true },
)

// 调度简报回填：把用户当前选择（档位 + 子智能体 + 团队 + 蜂群 + 能力）同步到一个
// 纯数据 ref，供每轮 chat.send 上行。缺省字段不写（后端按缺席处理，不猜）。
//
// 只在**真的选了分工**时才上行：`single` + 全空 = 原 `chat.send` 路径（与
// `createDefaultOrchestration` 的「零额外开销」承诺一致）。此前无论有没有选择都发简报，
// 于是每轮（含「你好」）都被注入「先出调度计划」+ 分工纪律，既白烧 token，
// 又让模型误以为未被勾选的能力不可用。
watch(
  [gearKey, orchestration, sessionKey],
  () => {
    const orch = orchestration.value
    if (!shouldSendDispatchBrief(orch)) {
      dispatchBrief.value = null
      return
    }
    const brief: ChatDispatchBrief = {
      mode: gearKey.value || 'standard_doit',
      agent: agentIdFromSessionKey(sessionKey.value) || 'main',
      form: orch.form,
      swarmOn: orch.swarmOn,
      swarmSize: orch.swarmSize,
    }
    if (orch.agents.length) brief.agents = [...orch.agents]
    if (orch.teams.length) brief.teams = [...orch.teams]
    if (orch.capabilities.length) brief.capabilities = [...orch.capabilities]
    dispatchBrief.value = brief
  },
  { immediate: true, deep: true },
)

const orchActivity = useOrchestrationActivity(rpc)
const orchestrationItems = computed(() => orchActivity.items.value)

// 编排运行时间线（成员 / 工具 / 终态）：后端按 phase 发 `session.event.progress`，
// 这里聚合成对话内可折叠的执行泳道。会话键用 ref 包一层，切会话时自动按新键过滤。
const orchestrationRunsModel = useOrchestrationRuns(computed(() => sessionKey.value))
const orchestrationRuns = computed(() => orchestrationRunsModel.runs.value)
// 卡片位于消息流末尾，必须按时间正序排列（旧→新）。存储层为了"最近一次优先"
// 用了新→旧，直接渲染会让同一会话里多次编排的先后顺序颠倒。
const orchestrationRunsChronological = computed(() => [...orchestrationRuns.value].reverse())
const orchestrationNow = computed(() => orchestrationRunsModel.now.value)
// 「停止」按钮要能把编排也一起停掉：把运行中的条数同步给 `sessionHasActiveWork`。
watch(
  () => orchestrationRunsModel.activeCount.value,
  (count) => { orchestrationRunningCount.value = count },
  { immediate: true },
)
/** 编排终态对账定时器（仅在有 running 的编排时真正发请求） */
let orchestrationReconcileTimer: ReturnType<typeof setInterval> | null = null

// 宿主（Python 侧车）是否**此刻**不在位。`hasEvidence` 是历史记录，
// `sidecarPresent` 才是实时事实：两者同时成立才算真的挂了。
//
// 【2026-10-23 修复】还必须加上 `source === 'memory'`：落盘记录是跨重启的
// 历史（"上一次运行退出过"），拿它当"此刻挂了"会**误拦发送** —— 启动瞬间侧车
// 还没注册，用户一发消息就弹「离线」，而实际上几秒后一切正常。
// 只认本次运行观测到的退出，历史记录最多让诊断卡片延后显示。
const hostExitEvidence = useHostExitEvidence()
const hostProcessDown = computed(() =>
  hostExitEvidence.hasEvidence.value
  && !hostExitEvidence.sidecarPresent.value
  && hostExitEvidence.source.value === 'memory',
)

/**
 * 高自主编排形态：与后端 `PermissionMatrix`（`core/src/gateway/orchestrate.rs::gate_form`）
 * 的 Confirm 档对齐 —— team / expert / swarm 在标准档位下需要显式授权。
 *
 * 为什么必须在**前端发送前**问一次：后端只会回 `ORCHESTRATE_CONFIRM_REQUIRED`，
 * 旧实现把它翻译成一条 6 秒 toast「再次发送即确认执行」，用户的真实体感是
 * 「蜂群打开后要我再次确认，但我根本没看见确认弹窗」，同一条消息还会被草稿
 * 状态吃掉，重发时又忘。现在改成发送前弹出真正的确认框，明确列出形态、
 * 并行规模与权限含义，取消则一个字节都不发。
 */
const HIGH_AUTONOMY_FORMS = new Set<OrchestrationState['form']>(['team', 'expert', 'swarm'])

/**
 * 高自主形态只在执行档放行。
 *
 * 与后端 `gate_form` 对齐：`PermissionMatrix` 对 team / expert / swarm 在 Ask / Plan
 * 档一律拒绝（连 `confirmed` 也救不回来），只有 `*_doit` 档才允许。前端提前判定，
 * 是为了不把用户带进「确认完才被拒」的死胡同。
 */
function gearAllowsHighAutonomy(modeKey: string): boolean {
  return modeKey.endsWith('_doit')
}

const FORM_LABEL_KEY: Record<OrchestrationState['form'], string> = {
  single: 'chat.orchestration.formSingle',
  subagent: 'chat.orchestration.formSubagent',
  team: 'chat.orchestration.formTeam',
  expert: 'chat.orchestration.formExpert',
  swarm: 'chat.orchestration.formSwarm',
}

const HIGH_AUTONOMY_FORM_LABEL: Record<'team' | 'expert' | 'swarm', string> = {
  team: FORM_LABEL_KEY.team,
  expert: FORM_LABEL_KEY.expert,
  swarm: FORM_LABEL_KEY.swarm,
}

async function confirmHighAutonomySend(orch: Readonly<OrchestrationState>): Promise<boolean> {
  if (!HIGH_AUTONOMY_FORMS.has(orch.form)) return true
  // 先过档位门，再弹确认框：Ask / Plan 档对高自主形态是**硬拒**（后端 gate_form），
  // 此前先弹「确认启动蜂群？」、用户点确认后才被拒，等于把用户带进死胡同
  // （真机：切了顶栏档位但输入框面板仍是方案档，连拒两次且没说怎么改）。
  if (!gearAllowsHighAutonomy(gearKey.value)) {
    pushToast(
      t('chat.orchestration.gearBlocked', {
        gear: t(`mode.userMode.${gearKey.value}`),
        form: t(FORM_LABEL_KEY[orch.form]),
      }),
      { tone: 'warn', duration: 10_000 },
    )
    return false
  }
  if (orch.confirmed) return true
  const form = orch.form as 'team' | 'expert' | 'swarm'
  const bodyKey = form === 'swarm'
    ? 'chat.orchestration.confirmBodySwarm'
    : form === 'expert'
      ? 'chat.orchestration.confirmBodyExpert'
      : 'chat.orchestration.confirmBodyTeam'
  return confirm({
    title: t('chat.orchestration.confirmTitle', { form: t(HIGH_AUTONOMY_FORM_LABEL[form]) }),
    body: t(bodyKey, {
      n: orch.swarmSize,
      agents: orch.agents.length,
      team: orch.teams.length
        ? orch.teams
            .map(id => EXPERT_TEAMS.find(team => team.id === id)?.label || id)
            .join(' + ')
        : t('chat.orchestration.teamNone'),
    }),
    primaryLabel: t('chat.orchestration.confirmPrimary'),
    primaryClass: 'btn--primary',
  })
}

/**
 * 编排发送：非 single 形态走 `orchestrate.run` 统一入口（权限门在后端）。
 * ORCHESTRATE_CONFIRM_REQUIRED → 本会话内置 confirmed 门并提示再次发送；
 * ORCHESTRATE_DENIED → 直接 toast 拒绝原因；成功 → 清空输入。
 * 注意：编排与 pendingQueue / chat.send 的排队语义无关，成功后无需触碰队列。
 */
async function dispatchOrchestratedSend(
  orch: Readonly<OrchestrationState>,
  message: string,
): Promise<void> {
  markProvisionalDraftUsed()
  ttftStartMs.value = performance.now()
  try {
    // 编排提交必须有上限：此前不传 timeoutMs 时请求会无限等待。后端 `team.run`
    // 曾在台账过大时同步重写 SQLite 数十秒（根因已修），前端若没有超时，用户
    // 看到的仍是「确认后什么都没发生」，输入框也不会恢复。
    const res = await rpc.call<{ ok?: boolean; taskId?: string; error?: string }>(
      'orchestrate.run',
      {
        form: orch.form,
        // 档位取全局模式 store（`gearKey`），不取编排面板的本地副本：两份副本
        // 一旦漂移，就会出现「顶栏显示执行档、后端按方案档拒绝」的死胡同（真机）。
        mode: gearKey.value,
        message,
        sessionKey: sessionKey.value,
        capabilities: orch.capabilities,
        agents: orch.agents,
        // 团队/专家团可多选：后端按名册合并、按角色去重后执行。
        teams: orch.teams,
        swarm_on: orch.swarmOn,
        // 并行规模必须真传：后端此前忽略它，导致确认框写「6 路并行」实际只跑 1 个成员。
        swarmSize: orch.swarmSize,
        confirmed: orch.confirmed,
      },
      { timeoutMs: 120_000, queue: false, timeoutAction: 'reject' },
    )
    // 后端用 `ok:false` 表达「请求已到达但提交失败」（团队编排失败 / 台账登记失败）。
    // 旧实现无条件清空输入且没有任何提示，等于把失败也当成成功。
    if (res && res.ok === false) {
      pushToast(
        t('chat.orchestration.submitFailed', { error: res.error || '—' }),
        { tone: 'warn', duration: 10_000 },
      )
      return
    }
    orchestration.value = { ...orchestration.value, confirmed: false }
    inputText.value = ''
    autoResizeTextarea()
    // 编排结果的正文不会回写到对话流（蜂群写入 long_running 台账），必须显式告诉
    // 用户「已提交 + taskId」，否则界面看起来与没发送完全一样。
    pushToast(
      t('chat.orchestration.submitted', {
        form: t(FORM_LABEL_KEY[orch.form]),
        task: res?.taskId || '—',
      }),
      { duration: 10_000 },
    )
  } catch (err) {
    const code = (err as { code?: string })?.code
    if (code === 'ORCHESTRATE_CONFIRM_REQUIRED') {
      orchestration.value = { ...orchestration.value, confirmed: true }
      pushToast(t('chat.orchestration.confirmRequired'), { duration: 6000 })
      return
    }
    if (code === 'ORCHESTRATE_DENIED') {
      pushToast(t('chat.orchestration.denied'), { duration: 6000 })
      return
    }
    pushToast(err instanceof Error ? err.message : String(err), { duration: 6000 })
  }
}

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

/**
 * 本回合文件改动汇总的时间窗左界：最近一条助手消息之前的用户消息时间戳。
 * 流式进行中、新会话首屏、分享/分叉过渡期间都不显示 —— 那几种状态下
 * 「本回合」尚未定型，给出汇总只会让用户误以为它已经结束。
 */
const lastTurnChangeSince = computed<string | number | null>(() =>
  resolveLastTurnStartTs(renderedMessages.value, {
    blocked: Boolean(
      isStreaming.value || isNewChatLanding.value || shareMode.value || forkTransition.value,
    ),
  }),
)

/**
 * 汇总条上报的「本回合确实有改动」信号 —— 下一步建议只在有改动可操作时出现，
 * 并与汇总条共用同一次台账查询（不再单独拉一遍）。
 */
const turnChangeStats = ref<{ files: number, canRevert: boolean }>({ files: 0, canRevert: false })
watch(lastTurnChangeSince, () => {
  turnChangeStats.value = { files: 0, canRevert: false }
})

const showTurnSuggestions = computed(() => (
  lastTurnChangeSince.value !== null &&
  turnChangeStats.value.files > 0 &&
  isLastTurnSuccessful(renderedMessages.value)
))

// 建议行是在台账返回后才出现的（晚于流式收尾的那次贴底）——
// 仍跟着底部时补一次滚动，否则 chip 会刚好落在视口下沿之外。
watch(showTurnSuggestions, async (visible) => {
  if (!visible || !autoScroll.value) return
  await nextTick()
  scrollToBottom()
})

const turnChangeSummaryRef = ref<{ requestRevert: () => void } | null>(null)


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

// 历史恢复失败态的有界自愈：初次读失败（首启瞬时不可用）不该只能等用户手点
// 「重新加载历史」。重连路径由 bootstrap 的 handleConnectionState 兜底，这里
// 补齐「连接从未掉线、读请求自身失败」这一窗口；预算耗尽仍是诚实的手动重试。
const HISTORY_AUTO_RETRY_DELAYS_MS = [4_000, 15_000] as const
let historyAutoRetryTimer: ReturnType<typeof setTimeout> | null = null
let historyAutoRetryAttempt = 0

function cancelHistoryAutoRetry() {
  if (!historyAutoRetryTimer) return
  clearTimeout(historyAutoRetryTimer)
  historyAutoRetryTimer = null
}

watch(visibleHistoryRecoveryState, state => {
  if (state === 'history-error') {
    if (
      historyAutoRetryTimer
      || historyAutoRetryAttempt >= HISTORY_AUTO_RETRY_DELAYS_MS.length
    ) return
    const delay = HISTORY_AUTO_RETRY_DELAYS_MS[historyAutoRetryAttempt]
    historyAutoRetryAttempt += 1
    historyAutoRetryTimer = setTimeout(() => {
      historyAutoRetryTimer = null
      if (visibleHistoryRecoveryState.value !== 'history-error') return
      void retryHistory()
    }, delay)
    return
  }
  cancelHistoryAutoRetry()
  // 恢复成功即复位预算：下一次真实故障重新享有自愈机会。
  if (state === null) historyAutoRetryAttempt = 0
})

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
  // 裸 /chat（刷新后 lastRoute 只恢复路径、丢弃 ?session=）会重建一个临时草稿：
  // 它不请求历史，状态永远停在 pending。这里把它当作权威空会话，避免空态被跳过。
  isNewSession: pendingSessionIntent.value === 'new_chat',
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

// M2-C2锛氭墽琛屽墠鍦ㄨ鍒掑崱涓婂鍒犳敼瀛愪换鍔★紙浠呭綋鍓嶈鍒掍笖鏃犺繍琛屼腑浠诲姟鏃跺惎鐢級
const planCardEditing = computed(() => !planActionsDisabled.value)
function onPlanStepAdd(step: { stepTitle: string; details?: string; deps?: string[] }): void {
  void planStepEditAdd(step)
}
function onPlanStepRemove(stepId: string): void {
  void planStepEditRemove(stepId)
}
function onPlanStepUpdate(step: { stepId: string; stepTitle: string; details?: string; deps?: string[] }): void {
  void planStepEditUpdate(step)
}

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
  reuseLastPlan,
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
  // 空会话此前把会话键尾段当标题渲染（真机实测标题是「聊天 8w1h8wxe」）——
  // 那串是内部 id 片段，对用户没有任何意义。没有消息时就说「新对话」，
  // 有消息时上面那条分支会立刻换成首句内容。
  return t('chat.newChat')
})

function focusComposer() {
  composerRef.value?.focusTextarea?.()
}

/**
 * 「下一步建议」只把提示词放进输入框，不替用户发送。
 * 已有草稿时追加而不是覆盖 —— 用户正在写的字不能被一个 chip 抹掉。
 */
function prefillComposerFromTurnSuggestion(prompt: string) {
  const text = prompt.trim()
  if (!text) return
  const current = inputText.value.trimEnd()
  if (!current.trim()) inputText.value = text
  else if (!current.includes(text)) inputText.value = `${current}\n${text}`
  focusComposer()
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

// P0-1c：鉴权失败（AUTH_FAILED）错误卡的两个操作。
// 「前往设置」直达模型服务设置页修复 Key；「重试」把上一条用户消息回填输入框并重新发送。
function openAuthSettings() {
  router.push('/settings/provider').catch(() => {})
}

function retryAuthFailure() {
  const lastUser = [...messages.value].reverse().find((m) => m.role === 'user')
  const text = (lastUser?.text ?? '').trim()
  if (!text) return
  inputText.value = text
  void onComposerSend()
}

/**
 * B2：失败轮次卡片的「改计划」出口。目标是当前方案卡（会话里存在且可修订时），
 * 复用方案卡自身的 replan 通道（`beginPlanRevision`）—— 不新造第二套修订逻辑。
 * 没有方案卡时按钮不出现（`canReplan` 为 false），因此这里拿到的一定是有效目标。
 */
const failedTurnReplanTarget = computed<PlanCardActionTarget | null>(() => {
  const plan = currentPlan.value
  if (!plan || !plan.planId) return null
  return { planId: plan.planId, revisionId: currentPlanRevisionId.value || plan.revisionId }
})

function replanFromFailedTurn() {
  const target = failedTurnReplanTarget.value
  if (target) beginPlanRevision(target)
}

// B2「看日志」：失败轮次的技术细节落在日志页，直接跳过去比死路强。
function openTurnLogs() {
  router.push('/logs').catch(() => {})
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

// ── 右侧侧栏面板（计量条 / 文件更改 / 任务轴 / 产物追踪，分治迁出）──
// 依赖产物索引与打开动作，装配在两者就绪之后；模板引用名与迁移前一致。
const sidebarPanels = useChatSidebarPanels({
  rpc,
  sessionKey,
  messages,
  streamToolCalls: chatStream.streamToolCalls,
  // 右侧「运行」面板要能看见"思考及内容"，不能只有一行 "Thinking…" 计时
  thinking: chatStream.thinkingContent,
  sessionArtifacts,
  loadSessionArtifacts,
  isStreaming,
  streamArtifacts,
  openArtifact,
  openArtifactInSystem: (a) => artifactActions.openInSystem(a),
})
const {
  fileChangesOpen,
  fileChangesVisible,
  rightSidebarOpen,
  sidebarTasks,
  sidebarChanges,
  onSidebarOpenArtifact,
  onSidebarOpenArtifactSystem,
  editingArtifact,
  onArtifactInlineSaved,
  sidebarDiagnostics,
  reloadSidebarArtifacts,
  staleKeys,
  liveKeys,
} = sidebarPanels
const sidebarSubagents = computed(() => subagentPanelEntries(renderedMessages.value))
// 右侧面板打开时，会话列向左让位（而不是被浮层盖住）：宽度与 RightSidebar
// 共享同一份 usePanelResize 状态；拖动分割线时会话列同步收缩。
// 让位宽度 = 面板宽度 + 工作台占位（面板右缘贴在工作台左侧）。
const { width: rightPanelWidth, resizing: rightPanelResizing } = usePanelResize()
const chatRootStyle = computed(() => ({
  fontSize: chatFontPx.value,
  '--right-panel-w': rightSidebarOpen.value ? `${rightPanelWidth.value}px` : '0px',
}) as Record<string, string>)
// 晚绑回填：RPC 订阅接线先持占位，此处立即接管真实处理器。
onArtifactUpdated = sidebarPanels.onArtifactUpdated
// 双轨打开（系统程序优先，缺实现时由侧栏面板回退 Gateway 启动器/应用内预览）
const artifactActions = useArtifactActions({ rpc })


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
  void refreshEffortWire()
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

  // 回合结束桌面通知（turn-ended）。判据全在 composable 里：
  // 仅桌面壳、窗口不聚焦、用户未关闭、且非 aborted。发不出去也不打扰主流程。
  // 用 `text_snapshot`（`chat.done` 的权威快照）而不是 UI 里最后一条消息：
  // 后者在流式收尾/history 替换的时序里可能还没落位。
  unsubs.push(rpc.on('chat.done', (payload) => {
    const p = (payload || {}) as {
      reason?: string
      text_snapshot?: string
      textSnapshot?: string
      text?: string
      session_key?: string
      sessionKey?: string
      key?: string
    }
    // Side-chat turns share the WS bus. They must not fire the main turn-ended
    // notification or refresh the main session's usage/pressure. Legacy frames
    // without a session key still count as the current session.
    const doneSession = String(p.session_key ?? p.sessionKey ?? p.key ?? '')
    if (doneSession && doneSession !== sessionKey.value) return
    const body = p.text_snapshot ?? p.textSnapshot ?? p.text ?? ''
    void notifyTurnEnded({ text: body, reason: p.reason })
    // 每回合结束后刷新用量/上下文压力：此前只在 config.get 时拉一次，
    // 长会话里 chip 会一直停在旧压力值（等于不告警）。
    void loadCurrentSessionUsage()
  }))

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
  unsubs.push(chatEscalations.subscribe())
  // P2·E 编排活动日志：常驻订阅 agent.started/stopped，供状态条实时展示。
  unsubs.push(orchActivity.subscribe())
  // 编排执行时间线：后端 `ProgressReporter` 把 member / tool 过程发到本会话
  // （`session.event.progress`，payload.kind = orchestration），这里接住并渲染。
  unsubs.push(rpc.on('session.event.progress', (payload: unknown, meta?: unknown) => {
    orchestrationRunsModel.handleFrame({
      payload: (payload ?? undefined) as Record<string, unknown> | undefined,
      meta: (meta ?? undefined) as { session?: string } | undefined,
    })
  }))
  // 编排终态对账：进度帧可能永久丢失（真机 2026-10-24 —— 卡片停在「运行中 31s」，
  // 独立 WS 客户端却收到了 run_completed）。原因可能是服务端写超时拆链、客户端 seq 断档
  // 重连，而网关不做进度补发。这里在「仍有 running 的编排」时每 20s 拉一次
  // `orchestration.status`（每个 run 的最后一条编排帧），只接受**终态帧**（幂等）：
  // 一旦确认已完成/失败，卡片就收敛，不再依赖「终态帧刚好在线」。
  // 没有 running 时该定时器零开销（一次数组遍历即返回）。
  const reconcileOrchestration = async (): Promise<void> => {
    const running = orchestrationRuns.value.filter(run => run.status === 'running')
    if (!running.length) return
    try {
      const res = await rpc.call<{
        runs?: Array<{ phase?: string; payload?: unknown; runId?: unknown }>
        coveredSince?: unknown
        historyOldest?: unknown
      }>(
        'orchestration.status',
        { sessionKey: sessionKey.value },
        // 对账是尽力而为：断线时不进 outbox（否则每 20s 堆一条永不结束的请求），
        // 也不能让慢网关把 UI 拖住。
        { timeoutMs: 8_000, queue: false },
      )
      const rows = res?.runs ?? []
      const seen = new Set<string>()
      for (const row of rows) {
        const phase = typeof row.phase === 'string' ? row.phase : ''
        if (typeof row.runId === 'string') seen.add(row.runId)
        if (phase !== 'run_completed' && phase !== 'run_failed') continue
        orchestrationRunsModel.handleFrame({
          payload: (row.payload ?? undefined) as Record<string, unknown> | undefined,
          meta: { session: sessionKey.value },
        })
      }
      // 覆盖窗口已包含该 run 的起点、服务端却完全没有它的任何帧 → 这条记录已经不存在了
      // （网关重启 / 后台任务被 kill / 进程崩溃）。如实收口为失败，不让卡片无限转圈。
      // `historyOldest` 是缓冲里最旧一条的时间：比它还早的 run 可能只是被 2000 条上限挤掉，
      // 不能据此判死（否则高频会话里仍在跑的长任务是会被误判）。
      const sinceMs = typeof res?.coveredSince === 'string' ? Date.parse(res.coveredSince) : Number.NaN
      const oldestMs = typeof res?.historyOldest === 'string' ? Date.parse(res.historyOldest) : Number.NaN
      const floorMs = Math.max(
        Number.isFinite(sinceMs) ? sinceMs : Number.NEGATIVE_INFINITY,
        Number.isFinite(oldestMs) ? oldestMs : Number.NEGATIVE_INFINITY,
      )
      if (Number.isFinite(floorMs)) {
        for (const run of running) {
          if (seen.has(run.runId) || run.startedAtMs < floorMs) continue
          orchestrationRunsModel.markRunLost(run.runId, t('chat.orchestration.runStateLost'))
        }
      }
    } catch {
      // 对账失败不打断对话：下一轮再试；旧网关不支持该方法时也会落到这里。
    }
  }
  orchestrationReconcileTimer = setInterval(() => { void reconcileOrchestration() }, 20_000)
  unsubs.push(() => {
    if (orchestrationReconcileTimer !== null) {
      clearInterval(orchestrationReconcileTimer)
      orchestrationReconcileTimer = null
    }
  })
  // 重连成功立刻对一次：断线窗口内的终态帧不会补发，只能靠这一次查询收敛。
  watch(() => rpc.isConnected, (connected) => {
    if (connected) void reconcileOrchestration()
  })
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
  cancelHistoryAutoRetry()
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
/* 右侧面板打开 → 会话列向左平滑收缩（面板是 fixed 浮层，用 margin 让它真正让位，
   而不是被盖住）。宽度变量由 usePanelResize 提供；拖动分割线时关掉过渡，
   否则会话列会「追」着指针慢半拍。
   用户开了「减少动态效果」→ 直接瞬移，不做收缩动画。 */
.chat {
  margin-right: calc(var(--right-panel-w, 0px) + var(--workbench-inset-right, 0px));
  transition: margin-right var(--dur-base, 200ms) var(--ease-standard);
}
.chat--panel-resizing { transition: none; }
@media (prefers-reduced-motion: reduce) {
  .chat { transition: none; }
}

/* 编排执行时间线在消息流末尾：与消息之间留出与普通消息一致的呼吸，
   并限制宽度跟随消息列（避免在宽屏上被拉成一条横贯全屏的卡片）。 */
.chat-orch-runs {
  display: flex;
  flex-direction: column;
  gap: var(--sp-2);
  margin: var(--sp-3) 0 var(--sp-5);
}

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

/* Usage telemetry footnote — renders BELOW the composer's AI-generated
   disclaimer. Deliberately small print: muted, centered, never competing
   with the input surface. */
.chat-usage-strip {
  display: flex;
  flex-wrap: wrap;
  gap: 4px 14px;
  padding: 4px 16px 6px;
  font-size: var(--fs-xs);
  line-height: 1.5;
  color: var(--text-dim);
}
.chat-usage-strip--foot { justify-content: center; opacity: 0.85; }
.chat-usage-strip__item { white-space: nowrap; }
/* 降级说明允许折行：它通常比 token 计数长，nowrap 会把原因截成一截。 */
.chat-usage-strip__item--warn {
  color: var(--warn);
  white-space: normal;
}
</style>
