import { computed, ref, watch, type Ref } from 'vue'

import type { ChatMessage, ChatToolCall } from '@/types/chat'
import type { ArtifactPayload } from '@/types/rpc'
import type { RsArtifact } from '@/components/chat/RightSidebar.vue'

/**
 * 右侧侧栏面板状态（从 ChatView 分治迁出）。
 *
 * 承载聊天页右侧边栏的全部面板态：文件变更抽屉、
 * 侧栏开关持久化、任务轴（实时工具调用 + 历史时间线折叠）、文件更改列表、
 * 产物索引（stale/live 追踪 + artifact.updated 防抖重拉）与系统打开回退。
 * 视图侧只负责把 chatStream / 产物索引 / 打开动作等依赖注入进来。
 *
 * 底部用量计量条**不在这里**：它已改成 ChatView 按会话消息里的真实 usage 求和
 * （`utils/chat/sessionUsage.ts`），不再轮询 usage.status / cache.status。
 */

/** 任务轴行（实时工具调用与历史时间线的统一投影） */
export interface SidebarTask {
  title: string
  detail?: string
  status: 'running' | 'done' | 'error' | 'pending'
  tools?: string[]
  duration?: string
  /** 行类型：右侧栏据此换图标、并把空 title 本地化成「思考 / 执行总结」 */
  kind?: 'tool' | 'thinking' | 'summary' | 'step'
  /** 入参（命令行 / 文件路径 / JSON） */
  input?: string
  /** 出参（命令输出 / 文件内容 / 工具结果） */
  output?: string
  /** 稳定 key（避免流式追加时整列重排闪烁） */
  id?: string
}

/** 单条运行记录的正文长度上限（超出截断并标 `…`） */
const RUN_TEXT_LIMIT = 4000

function clipRunText(value: unknown): string {
  const text = typeof value === 'string' ? value : value == null ? '' : JSON.stringify(value, null, 2)
  return text.length > RUN_TEXT_LIMIT ? `${text.slice(0, RUN_TEXT_LIMIT)}\n…` : text
}

export interface SidebarTaskBuildInput {
  /** 当前轮的真实思考正文（可为空） */
  thinking: string
  streamToolCalls: ChatToolCall[]
  messages: ChatMessage[]
  streaming: boolean
}

/**
 * 运行流投影：思考 → 实时工具 → 历史步骤/工具 → 本轮总结。
 *
 * 抽成纯函数是为了能单测「顺序、kind、入参出参、总结抑制」这些契约，
 * 而不必去 mount 整个 ChatView。
 */
export function buildSidebarTasks(input: SidebarTaskBuildInput): SidebarTask[] {
  const tasks: SidebarTask[] = []
  const { thinking, streamToolCalls, messages, streaming } = input

  // 1) 实时思考：后台仍在跑时状态为 running，结束后保留全文供回看
  if (thinking.trim()) {
    tasks.push({
      id: 'thinking:live',
      kind: 'thinking',
      title: '',
      status: streaming ? 'running' : 'done',
      output: clipRunText(thinking),
    })
  }

  // 2) 实时工具调用（in-flight）：这是"命令行正在跑"的直接证据
  for (const tc of streamToolCalls) {
    tasks.push({
      id: `tool:${tc.toolId}`,
      kind: 'tool',
      title: tc.displayName || tc.name,
      status: tc.isRunning ? 'running' : (tc.isError ? 'error' : 'done'),
      tools: [tc.name],
      input: clipRunText(tc.inputRaw || tc.inputPreview || ''),
      output: clipRunText(tc.result || tc.resultPreview || ''),
    })
  }

  // 3) 已落库的历史：思考 / 时间线 / 工具调用
  for (const msg of messages) {
    if (msg.role !== 'assistant') continue
    if (msg.reasoning?.text) {
      tasks.push({
        id: `thinking:${msg.turnId || msg.ts}`,
        kind: 'thinking',
        title: '',
        status: 'done',
        output: clipRunText(String(msg.reasoning.text)),
      })
    }
    if (msg.timeline) {
      for (const seg of msg.timeline) {
        const s = seg as Record<string, unknown>
        tasks.push({
          kind: 'step',
          title: String(s.label || s.kind || s.text || '步骤'),
          detail: s.detail ? String(s.detail) : undefined,
          status: String(s.status || 'done') as SidebarTask['status'],
          duration: s.duration ? String(s.duration) : undefined,
        })
      }
    }
    if (msg.tool_calls) {
      for (const tc of msg.tool_calls) {
        const toolName = String(tc.name || tc.tool_name || tc.tool || 'tool')
        tasks.push({
          kind: 'tool',
          title: toolName,
          status: tc.error ? 'error' : (tc.result ? 'done' : 'running'),
          tools: [toolName],
          input: clipRunText(tc.input ?? ''),
          output: clipRunText(tc.error ? tc.error : (tc.result ?? '')),
        })
      }
    }
  }

  // 4) 本轮总结：最后一条 assistant 正文（流式期间不作为"总结"出现）
  if (!streaming) {
    const last = [...messages].reverse().find(m => m.role === 'assistant' && String(m.text || '').trim())
    if (last) {
      tasks.push({
        id: `summary:${last.turnId || last.ts}`,
        kind: 'summary',
        title: '',
        status: 'done',
        output: clipRunText(String(last.text)),
      })
    }
  }

  return tasks
}

/** 文件更改行（workspace.file_changes.list 归一化） */
export interface SidebarFileChange {
  path: string
  name?: string
  additions: number
  deletions: number
  kind?: 'created' | 'modified' | 'deleted'
}

/** 系统打开结果（artifacts.open_in_system：未启动时携带错误与方法回退标记） */
export interface ArtifactSystemOpenResult {
  launched: boolean
  error?: string
  method?: string
}

type SidebarRpc = {
  call: <T = unknown>(
    method: string,
    params?: Record<string, unknown>,
  ) => Promise<T>
  supportsMethod: (method: string) => boolean
}

export interface ChatSidebarPanelsDeps {
  rpc: SidebarRpc
  sessionKey: Ref<string>
  messages: Ref<ChatMessage[]>
  /** 实时编排工具调用流（in-flight 工具调用驱动任务轴 running 态） */
  streamToolCalls: Ref<ChatToolCall[]>
  /** 实时思考正文（session.event.thinking 累积；空串表示本轮没思考） */
  thinking?: Ref<string>
  /** 产物索引（会话级持久索引，stale 判定的对照面） */
  sessionArtifacts: Ref<ArtifactPayload[]>
  loadSessionArtifacts: () => Promise<unknown>
  isStreaming: Ref<boolean>
  streamArtifacts: Ref<ArtifactPayload[]>
  /** 应用内打开（内联编辑 / 预览覆盖层） */
  openArtifact: (artifact: RsArtifact) => void
  /** 系统程序打开（artifacts.open_in_system；未启动成功时由本面板决定回退） */
  openArtifactInSystem: (artifact: RsArtifact) => Promise<ArtifactSystemOpenResult>
}

/** 侧栏产物匹配：id > download_url > 名称 > 路径（与既有行为逐字段一致） */
function findSessionArtifact(artifacts: ArtifactPayload[], a: RsArtifact): RsArtifact | undefined {
  return artifacts.find(art =>
    (art.id && art.id === a.id)
    || (art.download_url && art.download_url === a.download_url)
    || ((art.name || art.filename || '') === (a.name || ''))
    || art.path === a.path,
  )
}

export function useChatSidebarPanels(deps: ChatSidebarPanelsDeps) {
  const { rpc, sessionKey, messages, streamToolCalls, sessionArtifacts } = deps

  // ── 文件更改抽屉 ──
  const fileChangesOpen = ref(false)
  const fileChangesVisible = computed(() => rpc.supportsMethod('workspace.file_changes.list'))

  // ── 侧栏开关持久化（跨路由保持，issue #A）──
  const RIGHT_SIDEBAR_OPEN_KEY = 'marguerite.chat.rightSidebarOpen'
  const rightSidebarOpen = ref(localStorage.getItem(RIGHT_SIDEBAR_OPEN_KEY) === '1')
  watch(rightSidebarOpen, (open) => {
    try { localStorage.setItem(RIGHT_SIDEBAR_OPEN_KEY, open ? '1' : '0') } catch { /* ignore */ }
  })

  // ── 任务轴：实时工具调用流 + 思考正文 + 历史时间线 + 本轮总结 ──
  // 侧栏这一列是用户"看见智能体在干什么"的唯一常驻入口，所以每行都带
  // 可展开的入参/出参：命令与输出、文件路径与内容摘录、工具结果、思考正文。
  //
  // 思考正文的"锁存"：`thinkingContent` 在 turn 结束时会被清空（用于收起计时器），
  // 但用户需要在下班之后仍能回看"它刚才想了什么"。这里保留最后一次非空内容，
  // 切会话时清掉，避免串味。
  const lastThinking = ref('')
  watch(
    () => deps.thinking?.value || '',
    (text) => {
      if (text.trim()) lastThinking.value = text
    },
  )
  // 新一轮开始即清空：否则模型这一轮没思考时，面板会拿上一轮的思考冒充本轮。
  watch(
    () => deps.isStreaming.value,
    (running, wasRunning) => {
      if (running && !wasRunning) lastThinking.value = ''
    },
  )
  watch(() => deps.sessionKey.value, () => { lastThinking.value = '' })

  const sidebarTasks = computed((): SidebarTask[] => buildSidebarTasks({
    thinking: String(deps.thinking?.value || '').trim() || lastThinking.value,
    streamToolCalls: streamToolCalls.value,
    messages: messages.value,
    streaming: deps.isStreaming.value === true,
  }))

  // ── 文件更改列表（侧栏打开时拉取）──
  const sidebarChanges = ref<SidebarFileChange[]>([])
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
          kind: (k === 'created' || k === 'deleted' ? k : 'modified') as SidebarFileChange['kind'],
        }
      })
    } catch { /* ignore */ }
  }
  watch(rightSidebarOpen, (open) => { if (open) refreshSidebarChanges() })

  // ── 产物编辑/系统打开 ──
  const editingArtifact = ref<RsArtifact | null>(null)
  function onSidebarOpenArtifact(a: RsArtifact) {
    // 在应用内：打开内联编辑 / 预览覆盖层（markdown/text/csv 可编辑，其余走预览/系统打开）
    const artifact = findSessionArtifact(sessionArtifacts.value, a)
    if (artifact) {
      editingArtifact.value = artifact
    }
  }
  function onArtifactInlineSaved() {
    void reloadSidebarArtifacts()
  }
  async function onSidebarOpenArtifactSystem(a: RsArtifact) {
    const artifact = findSessionArtifact(sessionArtifacts.value, a)
    if (!artifact) return
    const result = await deps.openArtifactInSystem(artifact)
    if (!result.launched && result.error) {
      // 没有可用的系统启动器时，退化为应用内预览，避免死路
      if (result.method !== 'gateway-fallback') deps.openArtifact(artifact)
    }
  }

  // ── 诊断：消息中引用了产物、但索引侧边栏为空，提示用户重试重载 ──
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
    try { await deps.loadSessionArtifacts() } catch { /* ignore */ }
  }

  // ── artifact.updated 订阅：后端一旦发出即重拉索引 ──
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

  // ── 流式期间的「更新中」态：live 产物 key（仅在 isStreaming 时计入）──
  // turn 结束后 isStreaming 翻 false，自动清空。
  const liveKeys = computed<string[]>(() =>
    deps.isStreaming.value
      ? deps.streamArtifacts.value.map((a) => String(a.id || a.download_url || `${a.path}|${a.name}`))
      : [],
  )

  /** Turn 收尾重同步：兜底重拉索引 + 清 stale（无 artifact.updated 事件的网关也能对齐）。 */
  function resyncArtifactsAfterTurn() {
    void reloadSidebarArtifacts()
    staleKeySet.clear()
  }

  return {
    fileChangesOpen,
    fileChangesVisible,
    rightSidebarOpen,
    sidebarTasks,
    sidebarChanges,
    refreshSidebarChanges,
    onSidebarOpenArtifact,
    onSidebarOpenArtifactSystem,
    editingArtifact,
    onArtifactInlineSaved,
    sidebarDiagnostics,
    reloadSidebarArtifacts,
    onArtifactUpdated,
    staleKeys,
    liveKeys,
    resyncArtifactsAfterTurn,
  }
}
