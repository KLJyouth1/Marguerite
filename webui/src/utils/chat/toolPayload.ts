/**
 * 工具参数 / 输出的展示投影（纯函数）。
 *
 * 为什么需要：编排卡里的工具行原本只有「名字 + 参数预览」，**输出根本没渲染**；
 * 而且预览是一整条没有换行的原始字符串，JSON、命令行输出、diff、URL 长得一模一样，
 * 用户无法判断一次调用到底做了什么、拿到了什么。
 *
 * 这里只做「判型 + 排版 + 脱敏 + 截断」，不碰 DOM，便于单测：
 *   - JSON  → 2 空格缩进（对象/数组才对，普通字符串不动）
 *   - diff  → 标记为 diff，由组件按行着色（+ 绿 / - 红 / @@ 蓝）
 *   - code  → 多行且能识别出代码特征（函数/import/分号/标签/装订线）
 *   - link  → 单行 http(s) URL，组件渲染成可点链接
 *   - text  → 其余一律按纯文本
 *
 * 脱敏沿用活动面板的 `redactActivityDetail`：密钥/令牌/URL 内嵌密码不得进 UI。
 */
import type { ChatToolCall } from '@/types/chat'
import { redactActivityDetail } from './activityToolDetails'

export type ToolPayloadKind = 'empty' | 'json' | 'diff' | 'code' | 'link' | 'text'

export interface ToolPayloadView {
  kind: ToolPayloadKind
  /** 可直接渲染的正文（已脱敏、已截断） */
  body: string
  /** 是否因长度上限被截断 */
  truncated: boolean
  /** 原始字节数（字符数） */
  bytes: number
  /** 原始行数 */
  lines: number
}

/** 单个内容块的展示上限：够看一次工具返回，又不至于把卡片撑爆。 */
export const TOOL_PAYLOAD_MAX_CHARS = 4000

function normalize(raw: unknown): string {
  if (raw == null) return ''
  if (typeof raw === 'string') return raw
  try {
    return JSON.stringify(raw)
  } catch {
    return String(raw)
  }
}

/** `{}` / `[]` / `""` / `null` 这类「结构化但没有信息」的载荷视为空。 */
function isEmptyish(text: string): boolean {
  const value = text.trim()
  return (
    !value
    || value === '""'
    || value === "''"
    || value === '{}'
    || value === '[]'
    || value === 'null'
  )
}

function prettifyJson(text: string): string | null {
  const value = text.trim()
  if (!value.startsWith('{') && !value.startsWith('[')) return null
  try {
    return JSON.stringify(JSON.parse(value), null, 2)
  } catch {
    return null
  }
}

function looksLikeDiff(text: string): boolean {
  if (text.startsWith('diff --git')) return true
  const lines = text.split('\n').filter(line => line.trim() !== '')
  if (lines.length < 2) return false
  const plus = lines.filter(line => line.startsWith('+') && !line.startsWith('+++')).length
  const minus = lines.filter(line => line.startsWith('-') && !line.startsWith('---')).length
  const hunks = lines.filter(line => line.startsWith('@@')).length
  return hunks > 0 || (plus > 0 && minus > 0)
}

function looksLikeCode(text: string): boolean {
  if (!text.includes('\n')) return false
  const cues = [
    /^\s*(?:def|class|function|const|let|var|import|from|export|package|use|fn|pub fn)\b/m,
    /=>/,
    /;\s*$/m,
    /^\s*[<][a-zA-Z!/]/m,
    /^\s*#!/,
  ]
  return cues.filter(pattern => pattern.test(text)).length >= 2
}

function looksLikeLink(text: string): boolean {
  if (/[\r\n]/.test(text)) return false
  return /^https?:\/\/\S+$/i.test(text.trim())
}

/** 把任意工具参数/输出投影成可渲染块。 */
export function viewToolPayload(
  raw: unknown,
  maxChars: number = TOOL_PAYLOAD_MAX_CHARS,
): ToolPayloadView {
  const normalized = normalize(raw)
  if (isEmptyish(normalized)) {
    return { kind: 'empty', body: '', truncated: false, bytes: 0, lines: 0 }
  }

  const redacted = redactActivityDetail(normalized)
  const lines = redacted.split(/\r?\n/).length
  const bytes = redacted.length

  const json = prettifyJson(redacted)
  let kind: ToolPayloadKind
  let body: string
  if (json !== null) {
    kind = 'json'
    body = json
  } else if (looksLikeDiff(redacted)) {
    kind = 'diff'
    body = redacted
  } else if (looksLikeLink(redacted)) {
    kind = 'link'
    body = redacted.trim()
  } else if (looksLikeCode(redacted)) {
    kind = 'code'
    body = redacted
  } else {
    kind = 'text'
    body = redacted
  }

  const truncated = body.length > maxChars
  return {
    kind,
    body: truncated ? `${body.slice(0, maxChars)}…` : body,
    truncated,
    bytes,
    lines,
  }
}

/** 工具行头部的一行参数摘要：压平空白、去引号、限长。 */
export function summarizeToolArgs(raw: unknown, max = 140): string {
  const normalized = normalize(raw)
  if (isEmptyish(normalized)) return ''
  const json = prettifyJson(normalized)
  const source = json !== null ? JSON.stringify(JSON.parse(json)) : normalized
  const flat = redactActivityDetail(source).replace(/\s+/g, ' ').trim()
  if (flat.length <= max) return flat
  return `${flat.slice(0, Math.max(1, max - 1)).trimEnd()}…`
}

/** diff 行的色调（供组件直接映射到语义 token，避免在模板里写正则）。 */
export function diffLineTone(line: string): 'add' | 'del' | 'hunk' | 'meta' | 'context' {
  if (line.startsWith('@@')) return 'hunk'
  if (line.startsWith('+++') || line.startsWith('---') || line.startsWith('diff ')) return 'meta'
  if (line.startsWith('+')) return 'add'
  if (line.startsWith('-')) return 'del'
  return 'context'
}

// ── 会话文件改动（sessions.fileChanges 消费侧归一）──────────────────────────

export type SessionFileChangeKind = 'create' | 'modify' | 'delete' | ''

export interface SessionFileChange {
  /** 工作区相对路径（或后端返回的原始路径） */
  path: string
  /** 空串 = 后端没有给出可靠类型（例如降级到 artifacts.list 时） */
  kind: SessionFileChangeKind
  /** 触发工具名；未知为空串 */
  tool: string
  /** 后端给出的新增/删除行数；缺失为 null，由 diff 正文兜底统计 */
  added: number | null
  removed: number | null
  /** 统一 diff 正文；降级模式为空 */
  diff: string
  /** 后端明确标记内容被截断 */
  truncated: boolean
  /** 台账时间（unix 秒）；旧后端或降级模式缺省为 null */
  ts: number | null
}

function asRecord(value: unknown): Record<string, unknown> | null {
  return value && typeof value === 'object' && !Array.isArray(value)
    ? value as Record<string, unknown>
    : null
}

function finiteNonNegative(value: unknown): number | null {
  const n = Number(value)
  return Number.isFinite(n) && n >= 0 ? Math.floor(n) : null
}

function normalizedKind(value: unknown): SessionFileChangeKind {
  const kind = String(value || '').trim().toLowerCase()
  return kind === 'create' || kind === 'modify' || kind === 'delete' ? kind : ''
}

/**
 * `sessions.fileChanges` → 行模型。
 * 契约：`{ ok, changes: [{ path, kind, tool, added, removed, diff, truncated? }] }`。
 * 缺字段就保持缺失（`null` / 空串），不猜「应该是新增还是修改」。
 */
export function normalizeSessionFileChanges(raw: unknown): SessionFileChange[] {
  const list = Array.isArray(raw)
    ? raw
    : (asRecord(raw)?.changes ?? asRecord(raw)?.items)
  if (!Array.isArray(list)) return []
  const out: SessionFileChange[] = []
  for (const item of list) {
    const row = asRecord(item)
    if (!row) continue
    const path = String(row.path ?? row.file ?? row.name ?? '').trim()
    if (!path) continue
    out.push({
      path,
      kind: normalizedKind(row.kind ?? row.type),
      tool: String(row.tool ?? row.toolName ?? '').trim(),
      added: finiteNonNegative(row.added ?? row.additions),
      removed: finiteNonNegative(row.removed ?? row.deletions),
      diff: typeof row.diff === 'string' ? row.diff : '',
      truncated: row.truncated === true,
      ts: finiteNonNegative(row.ts ?? row.timestamp),
    })
  }
  return out
}

/** 从统一 diff 正文统计 `+` / `-` 行数（忽略 `+++` / `---` 文件头）。 */
export function countDiffLines(diff: string): { added: number, removed: number } {
  let added = 0
  let removed = 0
  for (const line of String(diff || '').split(/\r?\n/)) {
    if (line.startsWith('+++') || line.startsWith('---')) continue
    if (line.startsWith('+')) added += 1
    else if (line.startsWith('-')) removed += 1
  }
  return { added, removed }
}

// ── 电脑操控监管事件（computer_use payload 归一）────────────────────────────

export type ComputerUseApproval = 'approved' | 'pending' | 'denied'

export interface ComputerUseEvent {
  id: string
  /** 动作（click / type / key …）；缺失为空串 */
  action: string
  /** 操作目标（窗口标题 / 控件 / 坐标）；缺失为空串 */
  target: string
  /** true / false；缺失为 null */
  ok: boolean | null
  /** 审批状态；缺失为 null */
  approval: ComputerUseApproval | null
  note: string
}

function parseJsonRecord(raw: unknown): Record<string, unknown> | null {
  if (raw == null || raw === '') return null
  const direct = asRecord(raw)
  if (direct) return direct
  if (typeof raw !== 'string') return null
  try {
    return asRecord(JSON.parse(raw))
  } catch {
    return null
  }
}

function approvalOf(value: unknown): ComputerUseApproval | null {
  const normalized = String(value || '').trim().toLowerCase()
  return normalized === 'approved' || normalized === 'pending' || normalized === 'denied'
    ? normalized
    : null
}

function eventFromRecord(record: Record<string, unknown> | null): ComputerUseEvent | null {
  if (!record) return null
  const nested = asRecord(record.payload) || asRecord(record.data) || asRecord(record.event)
  const source = nested && String(nested.kind || '') === 'computer_use' ? nested : record
  const kind = String(source.kind || '').trim()
  if (kind && kind !== 'computer_use') return null
  const action = typeof source.action === 'string' ? source.action.trim() : ''
  const target = typeof source.target === 'string'
    ? source.target.trim()
    : typeof source.selector === 'string'
      ? source.selector.trim()
      : ''
  const ok = typeof source.ok === 'boolean' ? source.ok : null
  const approval = approvalOf(source.approval)
  const note = typeof source.note === 'string' ? source.note.trim() : ''
  if (!action && !target && ok === null && !approval && !note) return null
  return { id: '', action, target, ok, approval, note }
}

function baseToolName(name: string): string {
  const normalized = String(name || '')
    .trim()
    .replace(/([a-z0-9])([A-Z])/g, '$1_$2')
    .toLowerCase()
    .replace(/-/g, '_')
  const parts = normalized.split(/__|[.:/]/).filter(Boolean)
  return parts[parts.length - 1] || ''
}

/**
 * 从一回合的工具调用里聚合 computer_use 监管事件。
 *
 * 只读取调用里真实存在的字段：`{ kind:'computer_use', action, target, ok,
 * approval, note? }` 可以出现在参数或结果 JSON 中；`ok` 缺失时允许回落到这次
 * 调用的真实执行状态（success/error），其余字段缺失就留空，不编造。
 */
export function computerUseEventsFromToolCalls(calls: ChatToolCall[] | undefined): ComputerUseEvent[] {
  const out: ComputerUseEvent[] = []
  const seen = new Set<string>()
  for (const call of calls || []) {
    if (baseToolName(call.name) !== 'computer_use') continue
    const key = call.toolId || `${call.name}:${out.length}`
    if (seen.has(key)) continue
    seen.add(key)
    const input = parseJsonRecord(call.inputRaw)
    const result = parseJsonRecord(call.result)
    let event = eventFromRecord(result) || eventFromRecord(input)
    if (!event) {
      // 没有显式 kind 时，按 tool 名确认是 computer_use，再取其真实字段。
      const source = result && (result.action || result.target || result.ok !== undefined)
        ? result
        : input && (input.action || input.target)
          ? input
          : null
      event = source
        ? eventFromRecord({ ...source, kind: 'computer_use' })
        : null
    }
    // 引擎在 ToolCall/ToolResult 事件上给出的真实裁决优先于 payload 里的同名字段：
    // 这是后端 PermissionMatrix 的判定结果，不是前端猜测。
    const fromCall = approvalOf(call.approval)
    if (!event) {
      const ok = call.status === 'success' ? true : call.status === 'error' ? false : null
      if (ok === null && !call.isRunning && !fromCall) continue
      event = { id: '', action: '', target: '', ok, approval: fromCall, note: '' }
    } else if (fromCall) {
      // call 上的裁决优先：它是引擎 PermissionMatrix 的真实结果，
      // payload 里的同名字段可能是工具自己回显的旧值。
      event = { ...event, approval: fromCall }
    }
    out.push({ ...event, id: key })
  }
  return out
}
