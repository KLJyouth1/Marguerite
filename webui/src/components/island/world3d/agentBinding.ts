/**
 * 游戏 ↔ Agent 双向绑定（纯函数，可 vitest）。
 *
 * 方向一（游戏 → Agent）：玩家拾取道具 = 岛上发生的「事件」，翻译成一句
 *   自然语言委托提示词，经 chat.send 派给工作台（outbox 排队保证断线不丢）。
 *   提示词模板在这里，发送时机/连接降级由 useWorld3dBridge 负责。
 * 方向二（Agent → 游戏）：islandRuntime 归约出的真实运行时事件翻译成
 *   「场景脉冲指令」（雾开、灯塔点亮、极光增强……），渲染层只消费指令，
 *   不关心事件从哪个 RPC 方法来——单测即可钉死映射。
 *
 * 诚实性红线：scenePulseFor 只为**真实事件**产生脉冲；与岛屿无关的事件
 * 返回 null（引用不变）。npcReplyFor 的台词长度只反映档位预算（tierProfile），
 * 绝不伪装成「模型真的在岛上跑了一个 NPC」——它是提示词工程的确定性输出。
 */

import type { PropKind, PropInteraction } from './propsSpec'
import type { World3dTier, TierProfile } from './tierProfile'
import { tierProfile } from './tierProfile'

// ───────────────────────── 游戏 → Agent ─────────────────────────

/** 拾取上下文（道具 + 档位神力域），构成一条 chat.send 委托 */
export interface PickupContext {
  kind: PropKind
  interaction: PropInteraction
  /** 道具命名（工具/产物摘要，写进提示词让 Agent 知道「是哪个」） */
  name: string
  tier: World3dTier
}

/**
 * 拾取道具 → 委托提示词（中文，桌面工作台主语境）。
 * 模板是确定性的：同一拾取永远生成同一句话 → 可断言、可回归。
 */
export function agentPromptFor(ctx: PickupContext): string {
  const subject = ctx.name?.trim() || ctx.kind
  switch (ctx.interaction) {
    case 'analyze':
      return `岛上拾取了代码结晶「${subject}」，请分析这份产出并总结要点。`
    case 'explain':
      return `岛上拾取了文档卷轴「${subject}」，请讲解这份文档的核心内容。`
    case 'insight':
      return `岛上拾取了数据果实「${subject}」，请给出这批数据的洞察结论。`
    case 'quest':
      return `岛上点亮了任务灯塔「${subject}」，请认领并推进这项委托。`
    case 'brainstorm':
      return `岛上绽放了灵感花「${subject}」，请围绕它做一轮头脑风暴。`
    default:
      return `岛上发生了交互「${subject}」，请回应。`
  }
}

/**
 * 拾取特效强度 0..1（渲染层据此决定粒子爆发规模）。
 * 由档位粒子预算归一化：预算越高爆发越华丽——神力表现随档位缩放。
 */
export function pickupGlowStrength(tier: World3dTier): number {
  const budget = tierProfile(tier).particleBudget
  // 720（旗舰）→ 1.0；180（极速）→ 0.25，线性映射并夹取
  return Math.min(1, Math.max(0.15, budget / 720))
}

// ───────────────────────── Agent → 游戏 ─────────────────────────

/** 场景脉冲指令：渲染层消费（雾/天象/光强/道具生成……） */
export interface ScenePulse {
  /** 脉冲类别 */
  type: 'aurora' | 'beacon' | 'fog' | 'bloom' | 'spawn' | 'storm'
  /** 强度 0..1（渲染层映射到光照/粒子） */
  intensity: number
  /** 极短人类可读标签（HUD 可直接显示） */
  label: string
}

const EVENT_PULSES: Record<string, Omit<ScenePulse, 'intensity'>> = {
  'session.event.tool_use_start': { type: 'spawn', label: 'tool' },
  'session.event.tool_result': { type: 'bloom', label: 'tool-done' },
  'session.event.router_decision': { type: 'beacon', label: 'routing' },
  'session.event.ensemble_progress': { type: 'aurora', label: 'ensemble' },
  'session.event.plan_run': { type: 'fog', label: 'plan' },
  'session.event.plan_revision': { type: 'fog', label: 'plan-revise' },
  'session.event.progress': { type: 'spawn', label: 'progress' },
  'session.event.subagent_completion': { type: 'bloom', label: 'subagent' },
  'session.event.artifact': { type: 'bloom', label: 'artifact' },
  'session.event.memory_recall': { type: 'aurora', label: 'memory' },
  'session.event.state_change': { type: 'bloom', label: 'state' },
  'task.queued': { type: 'spawn', label: 'queued' },
  'task.running': { type: 'beacon', label: 'running' },
}

/**
 * 运行时事件 → 场景脉冲。未映射/未知事件返回 null（不产生虚构特效）。
 * intensity 全部由档位预算推导：神力越强，事件在岛上的可见反馈越强。
 */
export function scenePulseFor(event: string, tier: World3dTier, profileOverride?: TierProfile): ScenePulse | null {
  const mapped = EVENT_PULSES[event]
  if (!mapped) return null
  const profile = profileOverride ?? tierProfile(tier)
  const strength = Math.min(1, Math.max(0.15, profile.particleBudget / 720))
  return { ...mapped, intensity: Number(strength.toFixed(3)) }
}

// ───────────────────────── AI NPC 对话 ─────────────────────────

/** NPC 台词条目（由档位深度决定条数） */
export interface NpcDialogueLine {
  speaker: string
  text: string
}

/**
 * AI NPC 台词生成（确定性：由 (topic, tier) 完全决定）。
 * npcDialogueDepth 决定条数：1=单句快人，3=多轮深谈——「AI NPC 对话复杂度」
 * 随档位缩放的契约实现。台词是模板拼接而非模型输出，UI 须标注「神力拟声」。
 */
export function npcReplyFor(topic: string, tier: World3dTier): NpcDialogueLine[] {
  const profile = tierProfile(tier)
  const subject = topic?.trim() || '这座岛'
  const lines: NpcDialogueLine[] = [
    { speaker: 'npc', text: `${subject}……我感应到了。` },
    { speaker: 'npc', text: `以${tier}档神力，我可以把${subject}看得更透。` },
    { speaker: 'npc', text: `要不要我把${subject}的来龙去脉都讲给你？` },
  ]
  return lines.slice(0, profile.npcDialogueDepth)
}
