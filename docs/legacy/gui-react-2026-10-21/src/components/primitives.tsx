import React, { memo } from "react"
import { AgentStatus } from "../types"

const PillBadgeImpl: React.FC<{ value: string; color?: string; dot?: boolean; tone?: "solid" | "soft" }> = ({
  value,
  color = "var(--status-success)",
  dot,
  tone = "soft",
}) => {
  const style: React.CSSProperties =
    tone === "solid"
      ? { background: color, color: "#fff" }
      : {
          background: `color-mix(in srgb, ${color} 12%, #fff)`,
          color,
          border: `1px solid color-mix(in srgb, ${color} 25%, #fff)`,
        };
  return (
    <span
      style={{
        display: "inline-flex",
        alignItems: "center",
        gap: 6,
        padding: "2px 10px",
        borderRadius: 999,
        fontSize: 11,
        fontWeight: 600,
        lineHeight: "18px",
        ...style,
      }}
    >
      {dot && (
        <span
          style={{
            width: 6,
            height: 6,
            borderRadius: 999,
            background: color,
            boxShadow: `0 0 0 2px color-mix(in srgb, ${color} 30%, transparent)`,
          }}
        />
      )}
      {value}
    </span>
  );
};

const StatusDotImpl: React.FC<{ status: AgentStatus; size?: number }> = ({ status, size = 8 }) => {
  const colorMap: Record<AgentStatus, string> = {
    idle: "var(--text-tertiary)",
    running: "var(--status-success)",
    paused: "var(--status-warning)",
    error: "var(--status-error)",
    done: "var(--status-info)",
  };
  const color = colorMap[status];
  const pulse = status === "running";
  return (
    <span
      style={{
        position: "relative",
        width: size,
        height: size,
        borderRadius: 999,
        background: color,
        display: "inline-block",
      }}
    >
      {pulse && (
        <span
          style={{
            position: "absolute",
            inset: -3,
            borderRadius: 999,
            background: color,
            opacity: 0.35,
            animation: "mg-pulse 1.6s ease-out infinite",
          }}
        />
      )}
    </span>
  );
};

const ProgressBarImpl: React.FC<{ value: number; color: string }> = ({ value, color }) => (
  <div
    style={{
      height: 4,
      background: "var(--surface-border)",
      borderRadius: 999,
      overflow: "hidden",
    }}
  >
    <div
      style={{
        height: "100%",
        width: `${Math.max(0, Math.min(100, value))}%`,
        background: color,
        borderRadius: 999,
        transition: "width 300ms ease",
      }}
    />
  </div>
);

const ChevronIconImpl: React.FC<{ open: boolean; size?: number }> = ({ open, size = 12 }) => (
  <span
    style={{
      display: "inline-block",
      transition: "transform 150ms ease",
      transform: open ? "rotate(90deg)" : "rotate(0deg)",
      fontSize: size,
      lineHeight: 1,
    }}
  >
    ›
  </span>
);

const SessionDotImpl: React.FC<{ kind?: "success" | "warning" | "error" | null; size?: number }> = ({
  kind,
  size = 6,
}) => {
  if (!kind) return null;
  const colorMap = {
    success: "var(--status-success)",
    warning: "var(--status-warning)",
    error: "var(--status-error)",
  };
  return (
    <span
      style={{
        width: size,
        height: size,
        borderRadius: 999,
        background: colorMap[kind],
        flexShrink: 0,
      }}
    />
  );
};

function BadgeImpl({ ok, label }: { ok: boolean; label?: string }) {
  return (
    <span
      style={{
        fontSize: 10,
        fontWeight: 700,
        padding: "2px 8px",
        borderRadius: 999,
        // 半透明底 + 前景色都跟主题：此前写死 rgba(34,197,94,.15)/#22c55e，
        // 深色主题（ink）下 --status-success 是 #3E7C59，写死会偏亮刺眼。
        // color-mix 让底色由同一个语义色派生，改 token 时两处一起变。
        background: ok
          ? "color-mix(in srgb, var(--status-success, #22c55e) 15%, transparent)"
          : "color-mix(in srgb, var(--status-error, #ef4444) 15%, transparent)",
        color: ok ? "var(--status-success, #22c55e)" : "var(--status-error, #ef4444)",
      }}
    >
      {label ?? (ok ? "正常" : "异常")}
    </span>
  );
}


// === B3: React.memo 包装 — 父级无关状态变更不再触发本组件重渲染 ===
export const PillBadge = memo(PillBadgeImpl);
export const StatusDot = memo(StatusDotImpl);
export const ProgressBar = memo(ProgressBarImpl);
export const ChevronIcon = memo(ChevronIconImpl);
export const SessionDot = memo(SessionDotImpl);
export const Badge = memo(BadgeImpl);
