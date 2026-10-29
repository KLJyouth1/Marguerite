export interface AgentOption {
  id: string
  name: string
  model?: string
}

export interface AgentsListResponse {
  agents?: Array<{
    id?: string
    agentId?: string
    name?: string
    model?: string
  }>
}

export interface RawSessionThread {
  id?: string
  kind?: string
}

export interface RawSessionChannelContext {
  name?: string
  id?: string
  accountId?: string
  threadId?: string
}

export interface RawSessionTask {
  status?: string
  task_id?: string
  taskId?: string
  turn_id?: string
  turnId?: string
  started_at?: number | string
  startedAt?: number | string
  finished_at?: number | string
  finishedAt?: number | string
  turn_outcome?: Record<string, unknown>
  turnOutcome?: Record<string, unknown>
  steer_capability?: import('./chat').ChatSteerCapability
  steerCapability?: import('./chat').ChatSteerCapability
}

export interface RawSessionCron {
  id?: string
  jobId?: string
  job_id?: string
  name?: string
}

export interface RawSessionItem {
  key?: string
  session?: string
  sessionKey?: string
  sessionId?: string
  agentId?: string
  agent_id?: string
  effectiveAgentId?: string
  sessionKind?: string
  surface?: string
  conversationKind?: string
  thread?: RawSessionThread | null
  channelContext?: RawSessionChannelContext | null
  title?: string
  subtitle?: string
  groupLabel?: string
  workspace?: string
  workspaceId?: string
  workspace_id?: string
  workspaceLabel?: string
  workspaceDisplayPath?: string
  /** 会话所属项目根目录（sessions.db project_path 下发）；用于把会话归类到项目下 */
  projectPath?: string
  project_path?: string
  /** 会话绑定的全部项目目录（P2-9 多根；首项与 projectPath 一致） */
  projectPaths?: string[]
  updatedAt?: number | string
  updated_at?: number | string
  lastActivityAt?: number | string
  last_activity_at?: number | string
  messageCount?: number
  message_count?: number
  entry_count?: number
  status?: string
  runStatus?: string
  run_status?: string
  active_task?: RawSessionTask | null
  activeTask?: RawSessionTask | null
  last_task?: RawSessionTask | null
  lastTask?: RawSessionTask | null
  terminal_status?: string
  terminalStatus?: string
  display_name?: string
  displayName?: string
  subject?: string
  derived_title?: string
  derivedTitle?: string
  source_kind?: string
  sourceKind?: string
  channel_kind?: string
  channelKind?: string
  channel_id?: string
  channelId?: string
  chat_type?: string
  chatType?: string
  group_id?: string
  groupId?: string
  last_channel?: string
  lastChannel?: string
  last_to?: string
  lastTo?: string
  last_account_id?: string
  lastAccountId?: string
  last_thread_id?: string
  lastThreadId?: string
  delivery_context?: Record<string, unknown>
  deliveryContext?: Record<string, unknown>
  origin?: Record<string, unknown>
  interactive?: boolean
  model?: string
  channel?: Record<string, unknown>
  parent?: Record<string, unknown>
  forked_from_parent?: boolean
  forkedFromParent?: boolean
  parent_session_key?: string
  parentSessionKey?: string
  cron?: RawSessionCron
  /** 会话级思考强度（'off'|'normal'|'deep'|'max'）；null/缺省 = 继承全局模型配置 */
  reasoningEffort?: string | null
  reasoning_effort?: string | null
  /** 会话级上下文长度；null/缺省 = 继承全局模型配置 */
  contextWindow?: number | null
  context_window?: number | null
}

export type RawSessionListEntry = RawSessionItem | string

export interface SessionsListResponse {
  sessions?: RawSessionListEntry[]
  keys?: RawSessionListEntry[]
}

/** sessions.get 返回：session 为完整会话行（含 reasoning/contextWindow 双 key） */
export interface SessionsGetResponse {
  ok?: boolean
  session?: RawSessionItem | null
}

/** sessions.patch 返回：三态语义 patch 后的持久化结果 */
export interface SessionsPatchResponse {
  ok?: boolean
  session?: string
  title?: string | null
  reasoningEffort?: string | null
  contextWindow?: number | null
}

export interface ProjectWorkspaceItem {
  id: string
  name: string
  path: string
  taskCount: number
  pinned: boolean
  available: boolean
  availabilityReason?: string
}

export interface ProjectWorkspacesResponse {
  workspaces?: ProjectWorkspaceItem[]
}

export interface SandboxPathEntry {
  name: string
  path: string
  kind: 'directory' | 'file'
  selectable: boolean
  hidden?: boolean
}

export interface SandboxPathListResponse {
  currentPath: string
  path: string
  parentPath: string | null
  systemPickerAvailable: boolean
  entries: SandboxPathEntry[]
}

export interface ProjectWorkspaceHistoryDeleteResponse {
  workspaceId?: string
  deletedTaskCount?: number
  deletedSessionKeys?: string[]
}

/** One title/subject match from `sessions.search`. */
export interface SessionSearchHit {
  key: string
  title: string
  effectiveAgentId?: string | null
  surface?: string | null
  updatedAt?: number | null
}

/** One transcript (full-text) match from `sessions.search`. The snippet wraps
 *  matched terms in `>>>`/`<<<` delimiters (highlighted by the renderer). */
export interface MessageSearchHit {
  key: string
  title: string
  role?: string | null
  snippet: string
  createdAt?: number | null
  effectiveAgentId?: string | null
}

export interface SessionsSearchResponse {
  sessions?: SessionSearchHit[]
  messages?: MessageSearchHit[]
  query?: string
  ts?: number
}

export interface ArtifactPayload {
  id?: string
  key?: string
  kind?: string
  sha256?: string
  session_id?: string
  session_key?: string
  sessionKey?: string
  epoch?: number
  stream_seq?: number
  name?: string
  mime?: string
  size?: number | string
  source?: string
  created_at?: string
  createdAt?: string
  store?: string
  download_url?: string
  thumbnail_url?: string
  /**
   * Filesystem path inside the artifact store. Declared explicitly rather than
   * left to the index signature below: every surface that classifies or labels
   * an artifact reads it, and an `unknown` there forces a cast at each call
   * site — which is exactly where a wrong-type bug slips in.
   */
  path?: string
  /** Legacy alias some producers emit instead of `name`. */
  filename?: string
  /** Legacy alias some producers emit instead of `mime`. */
  content_type?: string
  [key: string]: unknown
}

export interface ArtifactsListResponse {
  artifacts?: ArtifactPayload[]
  has_more?: boolean
  hasMore?: boolean
  oldest_cursor?: string | null
  oldestCursor?: string | null
  newest_cursor?: string | null
  newestCursor?: string | null
  total_count?: number
  totalCount?: number
  page_size?: number
  pageSize?: number
}

export interface ArtifactsGetResponse {
  artifact?: ArtifactPayload | null
}

export interface StreamEventEnvelope {
  key?: string
  session_key?: string
  sessionKey?: string
  epoch?: number
  stream_seq?: number
  [key: string]: unknown
}

/** Turn boundary carried by the gateway's `sessions.changed` envelope. */
export type SessionChangeOccasion = 'turn_start' | 'turn_end'

/**
 * Canonical run-status literals used by `Envelope::sessions_changed`
 * (`running` / `idle` / `error`). Kept open (`| (string & {})`) so the
 * task-level vocabularies (queued/…, approval_pending, …) still assign cleanly.
 */
export type SessionChangeRunStatus = 'running' | 'idle' | 'error'

export interface SessionEventPayload extends StreamEventEnvelope {
  task_id?: string
  taskId?: string
  turn_id?: string
  turnId?: string
  started_at?: number
  emitted_at?: number
  reason?: string
  status?: string
  run_status?: SessionChangeRunStatus | (string & {})
  runStatus?: SessionChangeRunStatus | (string & {})
  // Gateway `Envelope::sessions_changed` carries the turn boundary explicitly:
  // `turn_start` when a run begins, `turn_end` when it settles. Optional because
  // legacy/other `sessions.changed` producers omit it.
  occasion?: SessionChangeOccasion
  terminal_message?: string
  terminal_reason?: string
  message?: string
  code?: string
  group_id?: string
  to_state?: string
  toState?: string
  active_task?: RawSessionTask | null
  last_task?: RawSessionTask | null
  [key: string]: unknown
}

export interface WarningPayload extends SessionEventPayload {
  message?: string
  code?: string
}

export interface CronResultMessagePayload {
  role?: string
  text?: string
  timestamp?: string | number | null
  messageId?: string
  message_id?: string
  provenanceKind?: string
  provenanceSourceTool?: string
  provenanceSourceSessionKey?: string
}

export type CronResultPayload = StreamEventEnvelope & {
  message?: CronResultMessagePayload
}

export interface SubagentCompletionPayload extends SessionEventPayload {
  type?: 'subagent_completion'
  parent_session_key?: string
  child_session_key?: string
  status?: string
  terminal_reason?: string
  message_id?: string
  messageId?: string
  result?: { text?: string; [key: string]: unknown }
}

export interface ApprovalStatusPayload {
  found?: boolean
  id?: string
  namespace?: string
  pending?: boolean
  resolved?: boolean
  approved?: boolean
  resolution?: string
  resolutionInProgress?: boolean
  consumed?: boolean
  deadline?: number
}

export interface TextDeltaPayload extends SessionEventPayload {
  text?: string
  /** Gateway-owned semantic role for this text span. */
  presentation?: 'intermediate' | 'answer'
}

export interface ToolUsePayload extends SessionEventPayload {
  id?: string
  toolId?: string
  tool_use_id?: string
  toolUseId?: string
  tool_id?: string
  name?: string
  tool_name?: string
  input?: unknown
  input_delta?: string
  inputDelta?: string
  json_fragment?: string
  jsonFragment?: string
  fragment?: string
  // Server wall-clock tool start time (epoch ms). Present on tool_use_start so a
  // running tool's elapsed timer survives page switches / stream replay instead of
  // restarting from a fresh local clock on remount (issue #329). 0/absent => use
  // the local clock.
  started_at?: number
  /** 引擎真实审批裁决（approved / pending / denied）。仅高自主本机工具会带。 */
  approval?: string
  /** 该工具的自主档位（readonly / mutating / high_autonomy）。 */
  autonomy?: string
}

export interface ToolDeltaPayload extends ToolUsePayload {
  delta?: string
  input_delta?: string
}

export interface ToolResultPayload extends ToolUsePayload {
  result?: unknown
  content?: unknown
  output?: unknown
  error?: unknown
  is_error?: boolean
  isError?: boolean
  execution_status?: { status?: string }
  executionStatus?: { status?: string }
}

export interface SessionMessagesSubscribeParams {
  key: string
  since_stream_seq?: number
  [key: string]: unknown
}

export interface SessionLiveSnapshotEvent {
  event: string
  payload: SessionEventPayload
}

export interface SessionMessagesSnapshotResponse {
  key: string
  task_id?: string | null
  current_stream_seq?: number
  events?: SessionLiveSnapshotEvent[]
}

export interface SessionProjectWorkspaceSnapshot {
  id: string
  name: string
  path: string
  available: boolean
  removed: boolean
  availabilityReason?: string
}

export interface SessionMessagesSubscribeResponse extends SessionEventPayload {
  subscribed?: boolean
  hydration_complete?: boolean
  hydrationComplete?: boolean
  deferred_fields?: string[]
  deferredFields?: string[]
  projectWorkspaceDeferred?: boolean
  project_workspace_deferred?: boolean
  replay_complete?: boolean
  current_stream_seq?: number
  active_task_group_ids?: string[]
  activeTaskGroupIds?: string[]
  run_mode_lock?: {
    locked?: boolean
    runMode?: 'safe' | 'full'
    source?: string
  }
  runModeLock?: {
    locked?: boolean
    runMode?: 'safe' | 'full'
    source?: string
  }
  workspaceId?: string
  /** 会话级全部项目目录（多根，首项为主根；后端有的版本只发 snake_case）。 */
  projectPaths?: string[]
  project_paths?: string[]
  projectWorkspace?: SessionProjectWorkspaceSnapshot | null
  collaboration?: import('./plans').CollaborationSnapshot
  currentPlan?: import('./plans').PlanRevisionSnapshot | null
  current_plan?: unknown
  activePlanRun?: import('./plans').PlanRunSnapshot | null
  active_plan_run?: unknown
  pendingUserInputs?: unknown[]
  pending_user_inputs?: unknown[]
}

export interface ChatSendAttachmentPayload {
  type: string
  mime: string
  name: string
  data?: string
  file_uuid?: string
}

/**
 * 输入框编排简报（用户当前选择）。随每轮 chat.send 上行，后端把它渲染成系统提示
 * 「本轮调度简报」，要求主智能体先产出分工/调度计划再执行。
 * 缺省字段不出现（不写「未选择」占位）。
 */
export interface ChatDispatchBrief {
  /** 3×3 档位键，如 `standard_doit` */
  mode?: string
  /** 主智能体 id */
  agent?: string
  /** single / subagent / team / expert / swarm */
  form?: string
  /** 子智能体角色 id 列表 */
  agents?: string[]
  /** 专家团 / 团队 id（多选；旧后端只认单值时可取首个） */
  teams?: string[]
  /** @deprecated 单团队字段，保留一版供旧侧车读取 */
  team?: string
  swarmOn?: boolean
  swarmSize?: number
  /** 能力勾选 id */
  capabilities?: string[]
}

export interface ChatSendParams {
  message: string
  sessionKey: string
  /** 本轮的编排简报（模式/智能体/蜂群/团队/能力），后端注入为「调度计划」纪律。 */
  dispatch?: ChatDispatchBrief
  /** Stable idempotency key for one logical send attempt. */
  clientRequestId?: string
  /** Stable client identity for reconciling the optimistic user row. */
  clientMessageId?: string
  _source?: { elevated?: string; runMode?: 'safe' | 'full' }
  intent?: string
  workspaceId?: string
  /** 会话级多根（P2-9）：主根在首位，其余为附加项目；单根时不发该字段。 */
  workspaceIds?: string[]
  collaborationMode?: import('./plans').CollaborationMode
  forkBeforeMessageId?: string
  displayText?: string
  attachments?: ChatSendAttachmentPayload[]
  /**
   * 本轮「@」引用（文件/目录/规则/历史会话）。
   * 后端 `gateway/context_refs.rs` 把材料拼在**用户消息尾部**（仅本轮生效），
   * 落库的用户消息仍是原文，避免把整份文件内容写进会话台账。
   */
  contextRefs?: Array<{ kind: string; id: string; label?: string; path?: string }>
  /**
   * 深度思考开关（DeepSeek thinking/reasoning 模式）。
   * 为 true 时请求编排层开启模型推理链，并在多轮对话中回传 reasoning_content
   * （DeepSeek 要求回传否则 API 400，见 core/src/agent/runtime.rs）。
   * 缺省时由当前模式/编排深度（thinking_depth）决定。
   * 保留 boolean 兼容旧调用方；新代码优先用 reasoningEffort 档位。
   */
  thinking?: boolean
  /**
   * 推理等级 —— **这是服务端真正读取的字段名**（`gateway/chat.rs` 读 `params["effort"]`）。
   *
   * 历史坑：本类型曾声明一个 `reasoningEffort` 字段，但发送侧从未填过它、
   * 服务端也从不读它 —— 一个「声明了却永远为空」的死字段，让 UI 以为
   * 四档能力已接通，实际只发得出 `thinking: true`。故此处改回真实 wire 名，
   * 类型与后端参数名一一对应，避免下次再有人照着一个假字段写代码。
   */
  effort?: 'off' | 'normal' | 'deep' | 'max' | (string & {})
  [key: string]: unknown
}

export interface ChatSendResponse {
  sessionKey?: string
  message_id?: string
  user_message_id?: string
  client_message_id?: string
  clientMessageId?: string
  task_id?: string
  taskId?: string
  replayed?: boolean
  task_status?: string
  taskStatus?: string
  terminal_reason?: string
  terminalReason?: string
  terminal_message?: string
  terminalMessage?: string
  reason?: string
}

/** Server-owned recovery record for one unaccepted manual MetaSkill launch. */
export interface MetaLaunchDraftPayload {
  sessionKey: string
  clientRequestId: string
  name: string
  launchText: string
  createdAt: number
  expiresAt: number
  sessionExists: boolean
}

export interface MetaDraftsListResponse {
  ok: boolean
  durable: boolean
  drafts: MetaLaunchDraftPayload[]
}

export interface MetaDraftDiscardResponse {
  ok: boolean
  discarded: boolean
  accepted?: boolean
}

export interface SessionSteerV2Params {
  key: string
  message: string
  expected_turn_id?: string
  client_request_id?: string
  client_message_id?: string
  surface_id?: string
  _source?: { elevated?: string; runMode?: 'safe' | 'full' }
}

export interface SessionSteerV2Response {
  accepted?: boolean
  replayed?: boolean
  session_key?: string
  turn_id?: string
  user_message_id?: string
  client_request_id?: string
  client_message_id?: string
  disposition?: import('./chat').ChatSteerDisposition
  revision?: number
  promoted_turn_id?: string
  promoted_from_turn_id?: string
  applied_iteration?: number
  model_call_id?: string
  fallback_safe?: boolean
  failure_code?: string
  retryable?: boolean
  recovery?: string
  reason?: string
}

export interface InputDispositionPayload extends SessionEventPayload {
  target_turn_id?: string
  client_request_id?: string
  client_message_id?: string
  user_message_id?: string
  disposition?: import('./chat').ChatSteerDisposition
  promoted_from_turn_id?: string
  promoted_turn_id?: string
  applied_iteration?: number
  model_call_id?: string
  failure_code?: string
  retryable?: boolean
  recovery?: string
  fallback_safe?: boolean
  revision?: number
}

export interface ChatHistoryAttachmentPayload {
  type?: unknown
  mime?: unknown
  mime_type?: unknown
  media_type?: unknown
  name?: unknown
  filename?: unknown
  size?: unknown
  data?: unknown
  dataUrl?: unknown
  data_url?: unknown
  sha256_ref?: unknown
  download_url?: unknown
  kind?: unknown
  [key: string]: unknown
}

export interface ChatHistoryMessage {
  role?: string
  text?: string
  timestamp?: string | number | null
  ts?: string | number | null
  id?: string
  message_id?: string
  attachments?: ChatHistoryAttachmentPayload[]
  artifacts?: ArtifactPayload[]
  router_decision?: RouterDecisionPayload | null
  routerDecision?: RouterDecisionPayload | null
  tool_calls?: unknown[]
  timeline?: unknown[]
  provenance_kind?: string
  provenance_source_session_key?: string
  provenance_source_tool?: string
  turn_context?: Record<string, unknown>
  reasoning_content?: string
  usage?: unknown
  turn_usage?: unknown
  model?: string
  input?: number
  input_tokens?: number
  output?: number
  output_tokens?: number
}

export interface ChatCompactionSummary {
  id?: string | number | null
  compaction_id?: string | null
  compaction_index?: number | null
  trigger_reason?: string | null
  summary_text?: string
  summary_format?: string
  coverage_status?: string
  removed_count?: number | null
  kept_count?: number | null
  covered_through_id?: number | null
  created_at?: string | number | null
}

export interface ChatHistoryResponse {
  messages?: ChatHistoryMessage[]
  has_more?: boolean
  hasMore?: boolean
  oldest_cursor?: string | number | null
  oldestCursor?: string | number | null
  newest_cursor?: string | number | null
  newestCursor?: string | number | null
  history_scope?: string
  historyScope?: string
  canonical_available?: boolean
  canonicalAvailable?: boolean
  canonical_complete?: boolean
  canonicalComplete?: boolean
  limit?: number
  returned?: number
  compaction_summaries?: ChatCompactionSummary[]
  compactionSummaries?: ChatCompactionSummary[]
  turn_outcomes?: ChatHistoryTurnOutcome[]
}

export interface ChatHistoryTurnOutcome {
  turn_id?: string
  task_id?: string
  status?: string
  started_at?: string | number
  finished_at?: string | number
  outcome?: Record<string, unknown>
}

/* ── Task workspace contract ───────────────────────────────────────────
 * Types shared by the delivery-pipeline router payload (`decision.subtasks`)
 * and the task workbench panel. The backend emits these on
 * `session.event.router_decision` (and, later, `task.contract.get`). Every
 * field is optional on the wire so an older gateway degrades cleanly and the
 * UI renders no steps rather than fabricating them.
 */
export type DeliverableKind =
  | 'Research'
  | 'HtmlPage'
  | 'Presentation'
  | 'Document'
  | 'Spreadsheet'
  | 'Other'

export type TaskStepStatus = 'pending' | 'in_progress' | 'done' | 'failed'

/** One planned deliverable/step of a compound task (mirrors `DeliverableSpec`). */
export interface TaskStep {
  id: string
  role: string
  title: string
  kind: DeliverableKind
  /** Declared output path, or null for a pure research step. */
  targetPath: string | null
  /** Ids of steps this one depends on (empty for the first step). */
  dependsOn: string[]
  status: TaskStepStatus
  elapsedMs?: number
  tools?: string[]
  /** Path of the artifact produced by this step once it lands on disk. */
  artifactPath?: string | null
}

export type TaskGateVerdict = 'pass' | 'partial' | 'fail'

/** Terminal quality-gate verdict for a task (search validity + artifact completeness). */
export interface TaskGate {
  searchValid: boolean
  artifactsComplete: boolean
  missing: string[]
  verdict: TaskGateVerdict
}

/**
 * The `decision` object carried by `session.event.router_decision`. Every field
 * is optional and the index signature keeps unknown backend fields readable, so
 * adding `subtasks`/`pipeline` stays backward compatible with older payloads.
 */
export interface RouterDecision {
  intent?: string
  complexity?: string
  needsLlm?: boolean
  needs_llm?: boolean
  /** True when the router chose the compound delivery pipeline. */
  pipeline?: boolean
  rationale?: string
  /** Ordered deliverable steps; absent/empty for single-intent turns. */
  subtasks?: TaskStep[]
  [key: string]: unknown
}

export interface RouterDecisionPersona {
  agent_id?: string
  agentId?: string
  name?: string
  description?: string
  is_builtin?: boolean
  isBuiltin?: boolean
}

export interface RouterDecisionPayload extends SessionEventPayload {
  tier?: string
  model?: string
  routed_model?: string
  source?: string
  routing_applied?: boolean
  /** Bloom 融合路由的 AI 自动编排形态（Solo Agent / MAX Dual-Leader / Expert Team / Cluster）。 */
  strategy?: string
  decision?: RouterDecision
  /**
   * P0-2: the expert persona active for this session (`agent:{agentId}:...`).
   * Omitted (never null) for non-expert sessions and for disabled experts.
   */
  persona?: RouterDecisionPersona | null
}

/* ── T04：补订此前被前端丢弃的 3 类 Rust 事件 ─────────────────────────
 * `memory_recall`（core/src/memory/cross_project.rs 仅命中时发布）、
 * `step_start` / `step_finish`（agent 工具回路 `{round[,reason]}`，交付流水线形态
 * `{task_step_id,index,total,title,role,kind,target_path,status[,elapsed_ms]}`）。
 * 两处 step 发布点字段不同，故载荷用可选字段兼容两种形态。
 */
export interface MemoryRecallPayload extends SessionEventPayload {
  query?: string
  hits?: number
  topScore?: number
  top_score?: number
  ids?: string[]
  reinforced?: number
}

export interface StepLoopPayload extends SessionEventPayload {
  /** agent 工具回路轮次（orchestration 单轮路径）。 */
  round?: number
  /** 该轮结束原因（runner 自有稳定 token）。 */
  reason?: string
  /** 交付流水线形态字段。 */
  task_step_id?: string
  taskStepId?: string
  index?: number
  total?: number
  title?: string
  role?: string
  kind?: string
  target_path?: string
  targetPath?: string
  status?: string
  elapsed_ms?: number
  elapsedMs?: number
}

/* ── LLM ensemble progress ─────────────────────────────────────────────
 * Mid-turn `session.event.ensemble_progress` frames announce each ensemble
 * proposer/aggregator starting and finishing, so the router strip can reveal
 * members incrementally before the terminal `done` event lands.
 */
export interface EnsembleProgressPayload extends SessionEventPayload {
  event_type?: 'proposer_start' | 'proposer_finish' | 'aggregator_start' | 'aggregator_finish'
  proposer_index?: number
  proposer_label?: string
  proposer_model?: string
  proposer_provider?: string
  sample_index?: number
  elapsed_ms?: number
  input_tokens?: number
  output_tokens?: number
  cost_usd?: number
  error?: string
}

/* ── 文件更改台账（workspace.file_changes.list）─────────────────────
 * Agent 通过内置文件工具（write_file/edit_file/apply_patch/delete_file）
 * 对工作区文件的每次真实改动，供「文件更改卡片」展示/折叠/diff。
 */
export interface FileChangePayload {
  path: string
  kind: 'create' | 'modify' | 'delete'
  before: string
  after: string
  tool: string
  ts: number
}

export interface FileChangeListPayload {
  items: FileChangePayload[]
}

export interface CompactionPayload extends SessionEventPayload {
  status?:
    | 'started'
    | 'observed'
    | 'completed'
    | 'skipped'
    | 'stale'
    | 'failed'
    | 'error'
    | 'cancelled'
    | 'timed_out'
    | 'emergency_ephemeral'
    | (string & {})
  compacted?: boolean
  detail?: string
  source?: string
  phase?: string
  compaction_id?: string
  compactionId?: string
  sequence?: number
  heartbeat?: boolean
  heartbeat_at?: number
  elapsed_ms?: number
  stage?: string
  refused?: boolean
  safe_to_send?: boolean
  safeToSend?: boolean
  applied?: boolean
  durability?: 'durable' | 'request_scoped' | (string & {})
  user_visible?: boolean
  userVisible?: boolean
}

/* ── MetaSkill run events ──────────────────────────────────────────────
 * Four `session.event.meta_*` frames drive the run-progress ribbon and the
 * preflight checkpoint card. They are delivered through the `*` wildcard
 * handler (handleRpcAny) rather than an explicit rpc.on, so the composable
 * casts the raw payload to these shapes. snake_case keys mirror the gateway. */

export interface MetaPreflightFieldSpec {
  name?: string
  label?: string
  title?: string
  type?: string
  kind?: string
  multiline?: boolean
  required?: boolean
  default?: unknown
  description?: string
  help?: string
  hint?: string
  options?: unknown[]
  choices?: unknown[]
  [key: string]: unknown
}

export interface MetaPreflightRequestTemplate {
  language?: string
  outcome?: string
  deliverable?: string
  fields?: MetaPreflightFieldSpec[]
  [key: string]: unknown
}

export interface MetaPreflightPayload extends SessionEventPayload {
  run_id?: string
  meta_skill_name?: string
  language?: string
  interpreted_request?: string
  missing_fields?: string[]
  assumptions?: string[]
  request_template?: MetaPreflightRequestTemplate
  can_skip?: boolean
  requires_confirmation?: boolean
}

export interface MetaRunStepSpec {
  id?: string
  label?: string
  kind?: string
  depends_on?: string[]
}

export interface MetaRunAnnouncedPayload extends SessionEventPayload {
  run_id?: string
  meta_skill_name?: string
  language?: string
  user_language?: string
  meta_language?: string
  steps?: MetaRunStepSpec[]
  total?: number
}

export interface MetaStepRescueAction {
  id?: string
  label?: string
  [key: string]: unknown
}

export interface MetaStepRescue {
  actions?: MetaStepRescueAction[]
  [key: string]: unknown
}

export interface MetaStepStatePayload extends SessionEventPayload {
  run_id?: string
  step_id?: string
  state?: string
  status_text?: string | null
  error?: string
  substitute_for?: string | null
  rescue?: MetaStepRescue
}

export interface MetaRunCompletedPayload extends SessionEventPayload {
  run_id?: string
  outcome?: string
  completed_steps?: string[]
  failed_steps?: string[]
  recovered_steps?: string[]
  skipped_steps?: string[]
}

export interface RpcEventMap {
  'session.event.text_delta': TextDeltaPayload
  'session.event.tool_use_start': ToolUsePayload
  'session.event.tool_use_delta': ToolDeltaPayload
  'session.event.tool_result': ToolResultPayload
  'session.event.artifact': ArtifactPayload
  'session.event.router_decision': RouterDecisionPayload
  'session.event.ensemble_progress': EnsembleProgressPayload
  'session.event.router_control_replay': SessionEventPayload
  'session.event.state_change': SessionEventPayload
  'session.event.run_heartbeat': SessionEventPayload
  'session.event.compaction': CompactionPayload
  'session.event.warning': SessionEventPayload
  'session.event.input_disposition': InputDispositionPayload
  'session.epoch_changed': SessionEventPayload
  'sessions.changed': SessionEventPayload
  'task.queued': SessionEventPayload
  'task.running': SessionEventPayload
  'session.event.task_group.waiting': SessionEventPayload
  'session.event.task_group.synthesizing': SessionEventPayload
  'session.event.task_group.done': SessionEventPayload
  'session.event.task_group.failed': SessionEventPayload
  'session.event.meta_preflight': MetaPreflightPayload
  'session.event.meta_run_announced': MetaRunAnnouncedPayload
  'session.event.meta_step_state': MetaStepStatePayload
  'session.event.meta_run_completed': MetaRunCompletedPayload
  // T04：补订此前被前端丢弃的 3 类 Rust 事件。
  'session.event.memory_recall': MemoryRecallPayload
  'session.event.step_start': StepLoopPayload
  'session.event.step_finish': StepLoopPayload
}
