import React, { memo } from "react"
import { ProgressBar, StatusDot } from "./primitives"
import { AgentStatus, SubAgent } from "../types"

const AgentCardImpl: React.FC<{ agent: SubAgent }> = ({ agent }) => {
  const statusLabel: Record<AgentStatus, string> = {
    idle: "待命中",
    running: "运行中",
    paused: "已暂停",
    error: "异常",
    done: "已完成",
  };
  return (
    <div
      style={{
        padding: 12,
        borderRadius: 12,
        background: "var(--surface-card)",
        border: `1px solid ${agent.status === "running"
          ? "color-mix(in srgb, " + agent.color + " 40%, var(--surface-border))"
          : "var(--surface-border)"}`,
        boxShadow: agent.status === "running" ? "var(--shadow-sm)" : "none",
        transition: "border-color 160ms ease",
      }}
    >
      <div style={{ display: "flex", alignItems: "center", gap: 10 }}>
        <div
          style={{
            width: 28,
            height: 28,
            borderRadius: 8,
            background: `color-mix(in srgb, ${agent.color} 15%, #fff)`,
            color: agent.color,
            fontWeight: 800,
            fontSize: 11,
            display: "grid",
            placeItems: "center",
            border: `1px solid color-mix(in srgb, ${agent.color} 30%, #fff)`,
          }}
        >
          {agent.name.slice(0, 1)}
        </div>
        <div style={{ flex: 1, minWidth: 0 }}>
          <div style={{ display: "flex", alignItems: "center", gap: 6 }}>
            <span style={{ fontSize: 12, fontWeight: 700 }}>{agent.name}</span>
            <StatusDot status={agent.status} size={6} />
          </div>
          <div style={{ fontSize: 10.5, color: "var(--text-tertiary)", marginTop: 1 }}>
            {agent.role}
          </div>
        </div>
        <div
          style={{
            fontSize: 10.5,
            fontWeight: 700,
            padding: "2px 8px",
            borderRadius: 999,
            color: agent.color,
            background: `color-mix(in srgb, ${agent.color} 12%, #fff)`,
          }}
        >
          {statusLabel[agent.status]}
        </div>
      </div>

      {agent.status === "running" && (
        <div style={{ marginTop: 10 }}>
          <ProgressBar value={agent.progressPct} color={agent.color} />
          {agent.lastStep && (
            <div
              style={{
                marginTop: 6,
                fontSize: 11,
                color: "var(--text-secondary)",
                whiteSpace: "nowrap",
                overflow: "hidden",
                textOverflow: "ellipsis",
              }}
              title={agent.lastStep}
            >
              {agent.lastStep}
            </div>
          )}
        </div>
      )}

      <div style={{ marginTop: 10, display: "flex", flexWrap: "wrap", gap: 4 }}>
        {agent.tools.map((t) => (
          <span
            key={t}
            style={{
              fontSize: 10,
              padding: "2px 7px",
              borderRadius: 5,
              background: "var(--surface-hover)",
              color: "var(--text-secondary)",
              border: "1px solid var(--surface-border)",
              fontFamily: "ui-monospace, SFMono-Regular, Menlo, monospace",
            }}
          >
            {t}
          </span>
        ))}
      </div>
    </div>
  );
};


// === B3: React.memo 包装 — 父级无关状态变更不再触发本组件重渲染 ===
export const AgentCard = memo(AgentCardImpl);
