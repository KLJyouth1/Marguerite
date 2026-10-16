import { NAV_SECTIONS } from "./constants"

export type AgentStatus = "idle" | "running" | "paused" | "error" | "done";

export interface SubAgent {
  id: string;
  name: string;
  role: string;
  color: string; // token css
  status: AgentStatus;
  progressPct: number;
  tools: string[];
  lastStep?: string;
}

export type Intensity = "minimal" | "standard" | "extreme";
export type Working = "ask" | "plan" | "doit";

// The only legal expert modes: 3 intensities x 3 working modes = 9.
// Replaces bloom and other legacy modes; their capabilities are remapped
// into these three intensities.
export type ModeKind =
  | "minimal_ask" | "minimal_plan" | "minimal_doit"
  | "standard_ask" | "standard_plan" | "standard_doit"
  | "extreme_ask" | "extreme_plan" | "extreme_doit";

export interface ModeOption {
  id: ModeKind;
  intensity: Intensity;
  working: Working;
  label: string;
  short: string;
  icon: string;
  tagline: string;
  accent: string;
}

export interface ChatMessage {
  id: string;
  role: "user" | "assistant" | "tool" | "system";
  content: string;
  agent?: string;
  thinking?: boolean;
}

// ───────────── M2-C1：Intent Preview（动作确认卡）─────────────
// 与 core guard::tool_intent::ToolIntention 的 JSON 形状对齐。

export interface ChatIntentPreview {
  /** 操作类型：delete / overwrite / reset / terminate / batch / exec / network / unknown */
  kind: string;
  /** 目标对象描述 */
  target?: string;
  /** 影响范围 */
  scope?: string;
  /** 是否不可逆 */
  irreversible?: boolean;
  /** 是否破坏性 */
  destructive?: boolean;
  /** 是否高风险（core 已提级 High） */
  high_risk?: boolean;
  /** 预估副作用列表 */
  side_effects?: string[];
  /** 人话解释 */
  explain?: string;
}

/** core Envelope.payload（type=approval）在 GUI 端的最小可用形状 */
export interface ApprovalEnvelopePayload {
  approval_id?: string;
  tool?: string;
  args_preview?: { args?: string; risk?: string };
  intent?: ChatIntentPreview;
  reason?: string;
  session_key?: string;
}

/** 前端确认卡的数据模型（由 ApprovalEnvelopePayload 归一化而来） */
export interface ApprovalRequest {
  id: string;
  tool: string;
  argsPreview?: string;
  risk: string;
  intent?: ChatIntentPreview;
  reason?: string;
  session: string;
}

export type NavId = (typeof NAV_SECTIONS)[number]["items"][number]["id"];

export type WorkTab = "work" | "code" | "design";

export interface HistorySession {
  id: string;
  title: string;
  timestamp: string;
  isActive?: boolean;
  hasUnread?: boolean;
  statusDot?: "success" | "warning" | "error" | null;
}

export interface Workspace {
  id: string;
  name: string;
  icon?: string;
  sessions: HistorySession[];
  collapsed?: boolean;
  showMore?: boolean;
  totalCount?: number;
}

// A3 假数据清零：不再内置其它项目的演示会话（DataRecoveryPro / DDOS / CCD Recovery Pro 等）。
// GUI 首启为空工作区 + 空状态引导卡；会话由真实 RPC / 用户「新建任务」填充。
