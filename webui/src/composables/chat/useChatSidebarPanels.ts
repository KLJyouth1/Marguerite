import { computed, onMounted, onUnmounted, ref, watch, type Ref } from 'vue'

import type { ChatMessage, ChatToolCall } from '@/types/chat'
import type { ArtifactPayload } from '@/types/rpc'
import type { RsArtifact } from '@/components/chat/RightSidebar.vue'

/**
 * 右侧侧栏面板状态（从 ChatView 分治迁出）。
 *
 * 承载聊天页右侧边栏的全部面板态：用量/缓存计量条轮询、文件变更抽屉、
 * 侧栏开关持久化、任务轴（实时工具调用 + 历史时间线折叠）、文件更改列表、
 * 产物索引（stale/live 追踪 + artifact.updated 防抖重拉）与系统打开回退。
 * 视图侧只负责把 chatStream / 产物索引 / 打开动作等依赖注入进来。
 */

/** 用量计量条（usage.status 快照，camelCase/snake_case 双容） */
interface UsageStrip { total_tokens?: number; totalTokens?: number; runs?: number; sessions?: number }
/** 缓存计量条（cache.status 快照） */
interface CacheStrip { ok?: boolean; server_cache_hit_rate?: number; saved_prompt_tokens?: number; prefix_hits?: number; client_hit_rate?: number }

/** 任务轴行（实时工具调用与历史时间线的统一投影） */
export interface SidebarTask {
  title: string
  detail?: string
  status: 'running' | 'done' | 'error' | 'pending'
  tools?: string[]
  duration?: string
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

  // ── 计量条：本会话 token / 缓存命中（usage.status + cache.status，每 12s 刷新）──
  const usageStrip = ref<UsageStrip | null>(null)
  const cacheStrip = ref<CacheStrip | null>(null)
  let usageTimer: ReturnType<typeof setInterval> | null = null
  async function refreshUsageStrip() {
    // ⚠️ 没有会话键时**不能**问 `usage.status`：网关把空 `sessionKey` 当成一次合法
    // 过滤，于是返回 `session = ''`（无归属事件）那一桶 —— 真机实测是 6930 轮 /
    // 680677 token，前端却按「本会话」渲染给用户（全新会话不可能有 6930 轮）。
    if (!sessionKey.value) {
      usageStrip.value = null
    } else {
      try {
        const u = await rpc.call<UsageStrip>('usage.status', { sessionKey: sessionKey.value })
        usageStrip.value = u ?? null
      } catch { /* 静默 */ }
    }
    try {
      const c = await rpc.call<CacheStrip>('cache.status', {})
      cacheStrip.value = c ?? null
    } catch { /* 静默 */ }
  }
  onMounted(() => {
    void refreshUsageStrip()
    usageTimer = setInterval(() => void refreshUsageStrip(), 12000)
  })
  onUnmounted(() => {
    if (usageTimer) clearInterval(usageTimer)
  })

  // ── 文件更改抽屉 ──
  const fileChangesOpen = ref(false)
  const fileChangesVisible = computed(() => rpc.supportsMethod('workspace.file_changes.list'))

  // ── 侧栏开关持久化（跨路由保持，issue #A）──
  const RIGHT_SIDEBAR_OPEN_KEY = 'marguerite.chat.rightSidebarOpen'
  const rightSidebarOpen = ref(localStorage.getItem(RIGHT_SIDEBAR_OPEN_KEY) === '1')
  watch(rightSidebarOpen, (open) => {
    try { localStorage.setItem(RIGHT_SIDEBAR_OPEN_KEY, open ? '1' : '0') } catch { /* ignore */ }
  })

  // ── 任务轴：实时工具调用流 + 历史消息时间线/工具调用的统一投影 ──
  const sidebarTasks = computed((): SidebarTask[] => {
    const tasks: SidebarTask[] = []
    // Feed the live orchestrator step stream (in-flight tool calls) so the task
    // axis reflects real-time execution, not just committed history.
    for (const tc of streamToolCalls.value) {
      tasks.push({
        title: `调用工具：${tc.displayName || tc.name}`,
        status: (tc.isRunning ? 'running' : (tc.isError ? 'error' : 'done')) as SidebarTask['status'],
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
            status: String(s.status || 'done') as SidebarTask['status'],
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
            status: (tc.error ? 'error' : (tc.result ? 'done' : 'running')) as SidebarTask['status'],
            tools: [toolName],
          })
        }
      }
    }
    return tasks
  })

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
    usageStrip,
    cacheStrip,
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
