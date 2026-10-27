import React, { memo } from "react"
import { AgentCard } from "./AgentCard"
import { PillBadge } from "./primitives"
import { SubAgent } from "../types"

const RightRailImpl: React.FC<{ agents: SubAgent[] }> = ({ agents }) => {
  const running = agents.filter((a) => a.status === "running").length;
  return (
    <aside
      style={{
        width: 304,
        background: "var(--surface-bg)",
        borderLeft: "1px solid var(--surface-border)",
        display: "flex",
        flexDirection: "column",
        height: "100%",
      }}
    >
      <div
        style={{
          padding: "16px 16px 12px",
          borderBottom: "1px solid var(--surface-border)",
          background: "var(--surface-card)",
        }}
      >
        <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
          <div style={{ fontSize: 13, fontWeight: 700 }}>子代理 · 并行执行</div>
          <PillBadge
            value={running > 0 ? `${running} 运行中` : "待命"}
            color={running > 0 ? "var(--status-success)" : "var(--text-tertiary)"}
            tone="soft"
            dot
          />
        </div>
        <div style={{ fontSize: 11, color: "var(--text-tertiary)", marginTop: 4 }}>
          Orchestrator 统一调度 · 实时进度与工具调用
        </div>
      </div>

      <div style={{ padding: 12, display: "flex", flexDirection: "column", gap: 8, overflowY: "auto" }}>
        {agents.map((a) => (
          <AgentCard key={a.id} agent={a} />
        ))}
      </div>
    </aside>
  );
};


// === B3: React.memo 包装 — 父级无关状态变更不再触发本组件重渲染 ===
export const RightRail = memo(RightRailImpl);
