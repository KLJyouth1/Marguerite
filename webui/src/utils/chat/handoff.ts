/**
 * `/handoff` 工作区交接单组装（Lancer 范式落地 A1）。
 *
 * # 为什么是"落盘文件"而不是会话消息
 * 多模型/多会话协作的交接如果走对话，接手方必须靠人复述上下文或搬运文件；
 * 把交接物写成工作区内的一份 markdown，接手方（任何模型、任何会话、
 * team-board 上的角色 Agent）直接用文件工具读取即可承接——会话只做调度，
 * 产物与交接上下文全部落盘共享。人保决策权：交接单里的"交接说明"由人输入，
 * AI 不自动代写审核结论。
 *
 * # 契约
 * `composeHandoffMarkdown(sessionKey, note, artifacts, generatedAtIso)` 是
 * 纯函数：不碰网络、不碰时间（时间由调用方传入）、不碰 locale（交接单是
 * 给模型与接手会话读的工作区文件，固定双语标题 + 结构化表格）。
 */

export interface HandoffArtifactRow {
  path?: string
  kind?: string
  mime?: string
}

/** 把自由输入压成安全的 markdown 段落：去围栏、压控制符、限长。 */
export function sanitizeHandoffNote(raw: string, maxChars = 2000): string {
  const flat = String(raw ?? '')
    // 剥掉围栏与标题注入：交接说明是"人写给接手方"的正文，不允许伪装成结构
    .replace(/```/g, "'''")
    .replace(/^\s*#{1,6}\s+/gm, '＞ ')
    .replace(/[\u0000-\u0008\u000b\u000c\u000e-\u001f]/g, '')
    .trim()
  return flat.length > maxChars ? flat.slice(0, maxChars) + '…' : flat
}

export function composeHandoffMarkdown(
  sessionKey: string,
  note: string,
  artifacts: HandoffArtifactRow[],
  generatedAtIso: string,
): string {
  const lines: string[] = []
  lines.push('# 任务交接单 (Task Handoff)')
  lines.push('')
  lines.push(`- session: \`${sessionKey}\``)
  lines.push(`- generated_at: ${generatedAtIso}`)
  lines.push('- purpose: 工作区交接。接手的模型/会话/角色 Agent 直接读取本工作区继续任务，无需在对话中重复传输上下文。')
  lines.push('- decision_policy: 最终决策权在人。本文件不含 AI 自动审核结论；接手方产出需经用户审查。')
  lines.push('')
  lines.push('## 交接说明 (Handoff Note)')
  lines.push('')
  const safeNote = sanitizeHandoffNote(note)
  lines.push(safeNote || '（无补充说明 / no extra note）')
  lines.push('')
  lines.push('## 产物清单 (Artifacts in this session)')
  lines.push('')
  const rows = (artifacts ?? []).filter(a => a && typeof a.path === 'string' && a.path.trim())
  if (rows.length === 0) {
    lines.push('_（本会话暂无登记产物 / no registered artifacts）_')
  } else {
    lines.push('| # | path | kind | mime |')
    lines.push('|---|---|---|---|')
    rows.forEach((a, i) => {
      const path = String(a.path).replace(/\|/g, '\\|')
      const kind = String(a.kind ?? '').replace(/\|/g, '\\|')
      const mime = String(a.mime ?? '').replace(/\|/g, '\\|')
      lines.push(`| ${i + 1} | ${path} | ${kind} | ${mime} |`)
    })
  }
  lines.push('')
  lines.push('## 承接指引 (How to continue)')
  lines.push('')
  lines.push('1. 读取本文件所在 `handoff/` 目录与本会话产物（见上表）。')
  lines.push('2. 以「交接说明」为目标继续执行；不要重做已完成步骤。')
  lines.push('3. 你的产出继续落盘到工作区并登记产物台账，供下一次交接。')
  lines.push('')
  return lines.join('\n')
}
