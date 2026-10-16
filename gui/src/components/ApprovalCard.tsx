import "./approvalCard.css"
import type { ApprovalRequest } from "../types"

const KIND_LABEL: Record<string, string> = {
  delete: "删除",
  overwrite: "覆盖",
  reset: "重置",
  terminate: "终止进程",
  batch: "批量操作",
  exec: "执行命令",
  network: "网络访问",
  create: "创建",
  modify: "修改",
  unknown: "敏感操作",
}

export function kindLabel(kind?: string): string {
  if (!kind) return "敏感操作";
  return KIND_LABEL[kind] ?? kind;
}

export interface ApprovalCardProps {
  approval: ApprovalRequest;
  busy?: boolean;
  error?: string | null;
  onAllowOnce: () => void;
  onAllowAlways?: () => void;
  onDeny: () => void;
}

export function ApprovalCard({
  approval,
  busy,
  error,
  onAllowOnce,
  onAllowAlways,
  onDeny,
}: ApprovalCardProps) {
  const intent = approval.intent;
  const danger =
    intent?.irreversible === true ||
    intent?.destructive === true ||
    approval.risk === "high";
  const showAlways =
    !danger && onAllowAlways != null;

  const intentSlots: Array<{ label: string; value?: string; danger?: boolean }> = [];
  if (intent) {
    if (intent.kind) {
      intentSlots.push({ label: "操作类型", value: kindLabel(intent.kind) });
    }
    if (intent.target) {
      intentSlots.push({ label: "目标对象", value: intent.target });
    }
    if (intent.scope && intent.scope !== intent.target) {
      intentSlots.push({ label: "影响范围", value: intent.scope });
    }
    if (intent.irreversible) {
      intentSlots.push({ label: "可逆性", value: "不可逆，操作后无法恢复", danger: true });
    }
    if (intent.explain) {
      intentSlots.push({ label: "说明", value: intent.explain });
    }
  }
  const sideEffects = intent?.side_effects ?? [];

  return (
    <div
      className="approve-card"
      data-testid="approval-card"
      data-approval-id={approval.id}
      role="group"
      aria-label={`需要确认：${approxKindLabel(approval)}`}
    >
      <header className="approve-card__head">
        <span
          className="approve-card__badge"
          data-testid="approval-badge"
          style={{
            background: danger ? "var(--status-error)" : "var(--status-warning)",
          }}
        >
          {danger ? "!" : "?"}
        </span>
        <div className="approve-card__heading">
          <span className="approve-card__eyebrow">需要人工确认 · {kindLabel(intent?.kind)}</span>
          <h3 className="approve-card__title" data-testid="approval-title">
            {approxKindLabel(approval)}
          </h3>
        </div>
      </header>

      <div className="approve-card__body">
        <dl className="approve-card__context">
          <div className="approve-card__row">
            <dt>工具</dt>
            <dd><code>{approval.tool}</code></dd>
          </div>
          {approval.argsPreview ? (
            <div className="approve-card__row">
              <dt>参数</dt>
              <dd><code>{approval.argsPreview}</code></dd>
            </div>
          ) : null}
          {approval.reason ? (
            <div className="approve-card__row">
              <dt>原因</dt>
              <dd>{approval.reason}</dd>
            </div>
          ) : null}
        </dl>

        {intentSlots.length > 0 ? (
          <section className="approve-card__intent" data-testid="approval-intent">
            <div className="approve-card__intent-label">结构化意图预览</div>
            <dl className="approve-card__intent-grid">
              {intentSlots.map((slot) => (
                <div className="approve-card__intent-row" key={slot.label}>
                  <dt>{slot.label}</dt>
                  <dd className={slot.danger ? "approve-card__intent-danger" : ""}>
                    {slot.value}
                  </dd>
                </div>
              ))}
            </dl>
            {sideEffects.length > 0 ? (
              <ul className="approve-card__effects">
                {sideEffects.map((fx, i) => (
                  <li key={i}>{fx}</li>
                ))}
              </ul>
            ) : null}
          </section>
        ) : null}

        {danger ? (
          <section className="approve-card__risk" data-testid="approval-risk">
            <strong>{intent?.irreversible ? "不可逆操作" : "高风险操作"}</strong>
            <p>该操作可能造成数据不可恢复，请确认目标与范围后谨慎放行。</p>
          </section>
        ) : null}
      </div>

      <footer className="approve-card__footer">
        {error ? (
          <p className="approve-card__error" role="alert">{error}</p>
        ) : null}
        <div className="approve-card__actions">
          <button
            type="button"
            className="approve-card__btn approve-card__btn--primary"
            disabled={busy}
            onClick={onAllowOnce}
            data-testid="approval-allow-once"
          >
            批准执行一次
          </button>
          {showAlways ? (
            <button
              type="button"
              className="approve-card__btn approve-card__btn--ghost"
              disabled={busy}
              onClick={onAllowAlways}
            >
              允许同类
            </button>
          ) : null}
          <button
            type="button"
            className="approve-card__btn approve-card__btn--deny"
            disabled={busy}
            onClick={onDeny}
            data-testid="approval-deny"
          >
            拒绝
          </button>
        </div>
      </footer>
    </div>
  );
}

/** intent 缺失时的兜底标题 */
function approxKindLabel(approval: ApprovalRequest): string {
  if (approval.intent?.kind === "unknown" || !approval.intent?.kind) {
    return `确认：${approval.tool}`;
  }
  return `确认：${kindLabel(approval.intent.kind)}`;
}
