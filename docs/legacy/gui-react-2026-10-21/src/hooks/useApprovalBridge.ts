import { useEffect, useState } from "react"
import { invoke } from "@tauri-apps/api/core"
import { listen, type UnlistenFn } from "@tauri-apps/api/event"
import { useTauri } from "./useTauri"
import type {
  ApprovalEnvelopePayload,
  ApprovalRequest,
  ChatIntentPreview,
} from "../types"

/**
 * M2-C1 GUI 端动作确认卡的事件桥：
 *
 * - Tauri 宿主：订阅 core 经 sidecar 转发的事件 `mg-event-approval-requested` /
 *   `mg-event-approval-resolved`，裁决时 invoke `approve_approval` / `deny_approval`
 *   （命令契约见 core guard::approval_registry 注释：任意宿主凭 approval_id 定位并裁决）。
 * - 浏览器 dev fallback：暴露 `dispatchApproval()` 供测试与预览注入，裁决仅清除本地状态。
 *
 * GUI 本身无需复制门控：core ApprovalGate 持有唯一挂起审批（fail-closed），
 * 前端只负责把 approval_id 回传裁决。
 */

export function normalizeApproval(payload: unknown): ApprovalRequest | null {
  if (!payload || typeof payload !== "object") return null;
  const p = payload as ApprovalEnvelopePayload;
  const id = p.approval_id;
  if (!id) return null;
  const tool = p.tool ?? "unknown";
  const session = p.session_key ?? "";
  return {
    id,
    tool,
    argsPreview: p.args_preview?.args,
    risk: p.args_preview?.risk ?? (p.intent?.high_risk ? "high" : "medium"),
    intent: p.intent ?? undefined,
    reason: p.reason ?? "",
    session,
  };
}

export interface ApprovalBridge {
  pending: ApprovalRequest | null;
  busy: boolean;
  error: string | null;
  resolve: (approvalId: string, outcome: "approved" | "denied") => Promise<void>;
  /** dev fallback：注入一条挂起审批（浏览器预览 / 测试用） */
  dispatchApproval: (payload: ApprovalEnvelopePayload) => void;
}

const EVENT_REQUESTED = "mg-event-approval-requested";
const EVENT_RESOLVED = "mg-event-approval-resolved";

export function useApprovalBridge(): ApprovalBridge {
  const tauri = useTauri();
  const [pending, setPending] = useState<ApprovalRequest | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!tauri) return;
    const guards: Promise<UnlistenFn>[] = [];
    const onRequested = (ev: { payload: unknown }) => {
      const req = normalizeApproval(ev.payload);
      setError(null);
      setPending(req);
    };
    const onResolved = (ev: { payload: unknown }) => {
      const req = normalizeApproval(ev.payload);
      if (req) {
        setPending((cur) => (cur && cur.id === req.id ? null : cur));
      }
    };
    try {
      guards.push(listen(EVENT_REQUESTED, onRequested));
      guards.push(listen(EVENT_RESOLVED, onResolved));
    } catch {
      // ignore listen failures in dev fallback
    }
    return () => {
      Promise.all(guards)
        .then((fns) => fns.forEach((u) => u()))
        .catch(() => {});
    };
  }, [tauri]);

  const resolve = async (approvalId: string, outcome: "approved" | "denied") => {
    setBusy(true);
    setError(null);
    try {
      if (tauri) {
        await invoke(outcome === "approved" ? "approve_approval" : "deny_approval", {
          approvalId,
        });
      }
      setPending((cur) => (cur && cur.id === approvalId ? null : cur));
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  const dispatchApproval = (payload: ApprovalEnvelopePayload) => {
    setPending(normalizeApproval(payload));
  };

  return { pending, busy, error, resolve, dispatchApproval };
}

/** 供 dev fallback / 预览页在桥外部直接注入一条挂起审批的便捷函数 */
export function sampleApprovalPayload(overrides: Partial<ApprovalEnvelopePayload> = {}): ApprovalEnvelopePayload {
  const intent: ChatIntentPreview = {
    kind: "delete",
    target: "D:/Downloads/archive/old_reports",
    scope: "old_reports 目录下约 23 个 .pdf",
    irreversible: true,
    destructive: true,
    high_risk: true,
    side_effects: ["回收站外不可恢复（永久删除）", "影响归档索引"],
  };
  return {
    approval_id: "ap_sample",
    tool: "delete_file",
    args_preview: { args: '{"path":"D:/Downloads/archive/old_reports"}', risk: "high" },
    intent,
    reason: "高影响操作：批量删除需用户确认",
    session_key: "gui-session",
    ...overrides,
  };
}
