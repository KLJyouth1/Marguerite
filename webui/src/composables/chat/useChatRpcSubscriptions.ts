import type {
  ArtifactPayload,
  CompactionPayload,
  CronResultPayload,
  EnsembleProgressPayload,
  InputDispositionPayload,
  MemoryRecallPayload,
  RouterDecisionPayload,
  SessionEventPayload,
  StepLoopPayload,
  SubagentCompletionPayload,
  TextDeltaPayload,
  ToolDeltaPayload,
  ToolResultPayload,
  ToolUsePayload,
  WarningPayload,
} from '@/types/rpc'
import type { RpcEventHandler } from '@/lib/rpc'

type RpcSubscriptionClient = {
  on(event: string, handler: RpcEventHandler): () => void
}

export type ChatRpcSubscriptionHandlers = {
  onTextDelta: (payload: TextDeltaPayload) => void
  onToolUseStart: (payload: ToolUsePayload) => void
  onToolUseDelta: (payload: ToolDeltaPayload) => void
  onToolResult: (payload: ToolResultPayload) => void
  onArtifact: (payload: ArtifactPayload) => void
  onStateChange: (payload: SessionEventPayload) => void
  onRunHeartbeat: (payload: SessionEventPayload) => void
  onCompaction: (payload: CompactionPayload, meta: unknown) => void
  onWarning: (payload: WarningPayload) => void
  onInputDisposition: (payload: InputDispositionPayload) => void
  onCronResult: (payload: CronResultPayload) => void
  onSubagentCompletion: (payload: SubagentCompletionPayload) => void
  onEpochChanged: (payload: SessionEventPayload) => void
  onSessionsChanged: (payload: SessionEventPayload) => void
  onTaskQueued: (payload: SessionEventPayload) => void
  onTaskRunning: (payload: SessionEventPayload) => void
  onTaskGroupWaiting: (payload: SessionEventPayload) => void
  onTaskGroupSynthesizing: (payload: SessionEventPayload) => void
  onTaskGroupDone: (payload: SessionEventPayload) => void
  onTaskGroupFailed: (payload: SessionEventPayload) => void
  onRouterDecision: (payload: RouterDecisionPayload) => void
  onEnsembleProgress: (payload: EnsembleProgressPayload) => void
  onRouterControlReplay: (payload: SessionEventPayload) => void
  /** Fired when an artifact's stored bytes change out-of-band (SPEC §5
   * EVENT artifact.updated). Drives a sidebar index resync so inline edits and
   * externally-written files surface without a manual refresh. */
  onArtifactUpdated: (payload: SessionEventPayload) => void
  // T04：此前被前端丢弃的 3 类 Rust 事件（补订订阅 + 渲染）。
  onMemoryRecall: (payload: MemoryRecallPayload) => void
  onStepStart: (payload: StepLoopPayload) => void
  onStepFinish: (payload: StepLoopPayload) => void
  /**
   * 通配处理器。第三个参数是 WS 帧的 `meta`（`envelope_to_frame` 组装，携带
   * `session` / `ts` / `trace_id`）——**必须透传**：`session.event.usage` 这类帧的
   * payload 里没有会话标识，会话归属只存在于 meta 里。丢掉 meta 就只能靠
   * 「本页是否在流式中」猜归属，多标签页并行时会把别的会话的用量算进自己。
   */
  onAny: (rawEvent: string, rawPayload: unknown, meta?: Record<string, unknown>) => void
  onConnectionState: (state: string) => void
}

export function useChatRpcSubscriptions(
  rpc: RpcSubscriptionClient,
  handlers: ChatRpcSubscriptionHandlers,
) {
  let unsubs: Array<() => void> = []

  function subscribe(): () => void {
    unsubscribe()
    unsubs = [
      rpc.on('session.event.text_delta', handlers.onTextDelta),
      rpc.on('session.event.tool_use_start', handlers.onToolUseStart),
      rpc.on('session.event.tool_use_delta', handlers.onToolUseDelta),
      rpc.on('session.event.tool_result', handlers.onToolResult),
      rpc.on('session.event.artifact', handlers.onArtifact),
      rpc.on('session.event.state_change', handlers.onStateChange),
      rpc.on('session.event.run_heartbeat', handlers.onRunHeartbeat),
      rpc.on('session.event.compaction', handlers.onCompaction),
      rpc.on('session.event.warning', handlers.onWarning),
      rpc.on('session.event.input_disposition', handlers.onInputDisposition),
      rpc.on('session.event.cron_result', handlers.onCronResult),
      rpc.on('session.event.subagent_completion', handlers.onSubagentCompletion),
      rpc.on('session.epoch_changed', handlers.onEpochChanged),
      rpc.on('sessions.changed', handlers.onSessionsChanged),
      rpc.on('task.queued', handlers.onTaskQueued),
      rpc.on('task.running', handlers.onTaskRunning),
      rpc.on('session.event.task_group.waiting', handlers.onTaskGroupWaiting),
      rpc.on('session.event.task_group.synthesizing', handlers.onTaskGroupSynthesizing),
      rpc.on('session.event.task_group.done', handlers.onTaskGroupDone),
      rpc.on('session.event.task_group.failed', handlers.onTaskGroupFailed),
      rpc.on('session.event.router_decision', handlers.onRouterDecision),
      rpc.on('session.event.ensemble_progress', handlers.onEnsembleProgress),
      rpc.on('session.event.router_control_replay', handlers.onRouterControlReplay),
      // Two channels for one event on purpose. SPEC §5 names it `artifact.updated`,
      // but every other session-scoped event on this bus is namespaced
      // `session.event.*`, so a daemon that follows the local convention would
      // emit `session.event.artifact_updated` instead. Subscribing to both costs
      // one extra listener and means a naming mismatch degrades to "the sidebar
      // refreshes on turn end" rather than "artifacts never appear". The handler
      // is idempotent (it debounces and re-lists), so a daemon emitting both
      // names causes one resync, not two.
      rpc.on('artifact.updated', handlers.onArtifactUpdated),
      rpc.on('session.event.artifact_updated', handlers.onArtifactUpdated),
      // T04：补订 3 类此前无订阅的事件（Rust 侧已稳定发布）。
      rpc.on('session.event.memory_recall', handlers.onMemoryRecall),
      rpc.on('session.event.step_start', handlers.onStepStart),
      rpc.on('session.event.step_finish', handlers.onStepFinish),
      rpc.on('*', handlers.onAny),
      rpc.on('_state', handlers.onConnectionState),
    ]
    return unsubscribe
  }

  function unsubscribe() {
    unsubs.forEach(fn => fn())
    unsubs = []
  }

  return {
    subscribe,
    unsubscribe,
  }
}
