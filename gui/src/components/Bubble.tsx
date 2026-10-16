import React, { memo } from "react"
import { PillBadge } from "./primitives"
import { ChatMessage } from "../types"

const BubbleImpl: React.FC<{ msg: ChatMessage }> = ({ msg }) => {
  const isUser = msg.role === "user";
  const isSystem = msg.role === "system";
  if (isSystem) {
    return (
      <div
        style={{
          display: "flex",
          justifyContent: "center",
          padding: "4px 0",
        }}
      >
        <PillBadge value={msg.content} color="var(--text-tertiary)" tone="soft" />
      </div>
    );
  }
  return (
    <div
      style={{
        display: "flex",
        justifyContent: isUser ? "flex-end" : "flex-start",
        padding: "4px 0",
      }}
    >
      <div
        style={{
          maxWidth: "74%",
          padding: "10px 12px",
          borderRadius: 12,
          background: isUser
            ? "linear-gradient(180deg, var(--accent-primary-300) 0%, var(--accent-primary) 100%)"
            : "var(--surface-card)",
          color: isUser ? "#fff" : "var(--text-primary)",
          border: isUser ? "none" : "1px solid var(--surface-border)",
          boxShadow: isUser
            ? "0 6px 14px -10px var(--accent-primary-700)"
            : "var(--shadow-sm)",
          fontSize: 13,
          lineHeight: 1.55,
          whiteSpace: "pre-wrap",
          wordBreak: "break-word",
        }}
      >
        {msg.agent && (
          <div
            style={{
              fontSize: 10.5,
              fontWeight: 700,
              marginBottom: 4,
              color: isUser ? "rgba(255,255,255,0.85)" : "var(--accent-primary-700)",
            }}
          >
            @{msg.agent}
          </div>
        )}
        {msg.content}
        {msg.thinking && (
          <div
            style={{
              marginTop: 8,
              padding: "6px 8px",
              fontSize: 11,
              borderRadius: 8,
              background: isUser ? "rgba(255,255,255,0.18)" : "var(--surface-hover)",
              color: isUser ? "rgba(255,255,255,0.9)" : "var(--text-secondary)",
              borderLeft: `3px solid ${isUser ? "#fff" : "var(--accent-secondary)"}`,
            }}
          >
            <span style={{ display: "inline-block", animation: "mg-thinking 1.2s infinite" }}>
              ● ● ●
            </span>
            {"  "}思考中…
          </div>
        )}
      </div>
    </div>
  );
};

// A3 空状态引导组件：GUI 首启（无任何预置会话/消息）时展示「首任务引导卡」。


// === B3: React.memo 包装 — 父级无关状态变更不再触发本组件重渲染 ===
export const Bubble = memo(BubbleImpl);
