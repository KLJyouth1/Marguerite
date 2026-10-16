// M2-C4：护栏 Escalation（升级确认）前端契约
//
// 对应 core `guard::escalation::EscalationEvent`，经事件总线发布到
// `{session}.escalation`（WebUI 订阅 `session.event.escalation`）。
// 四类触发（trigger）：high_impact_side_effect / intent_ambiguity /
// timeout / budget_exceeded。前端渲染「需要确认」，决策通过
// `escalation.ack { id, decision: 'continue'|'abort' }` 回传。

export type EscalationTrigger =
  | 'high_impact_side_effect'
  | 'intent_ambiguity'
  | 'timeout'
  | 'budget_exceeded'

export type EscalationDecision = 'continue' | 'abort'

export interface EscalationEvent {
  id: string
  session: string
  trigger: EscalationTrigger
  severity: number
  reason: string
  context: Record<string, unknown>
  suggested_action: string
  created_at: string
}

/** 原始 wire payload 中 `escalation` 字段的宽松形状 */
export interface EscalationWireEnvelope {
  kind?: string
  escalation?: unknown
}

export interface EscalationAckResult {
  ok?: boolean
  error?: string
  id?: string
  decision?: string
  escalation?: EscalationEvent
}
