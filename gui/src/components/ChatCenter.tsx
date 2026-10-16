import React, { useEffect, useMemo, useRef, useState, memo } from "react"
import { Bubble } from "./Bubble"
import { EmptyStateGuide } from "./EmptyStateGuide"
import { PillBadge } from "./primitives"
import { MODES } from "../constants"
import { ChatMessage, ModeKind, SubAgent } from "../types"

const ChatCenterImpl: React.FC<{
  messages: ChatMessage[];
  mode: ModeKind;
  running: boolean;
  onSend: (text: string) => void;
  onNewTask: () => void;
  agents: SubAgent[];
}> = ({ messages, mode, running, onSend, onNewTask, agents }) => {
  const [input, setInput] = useState("");
  const scrollRef = useRef<HTMLDivElement | null>(null);
  const modeAccent = MODES.find((m) => m.id === mode)!.accent;

  useEffect(() => {
    scrollRef.current?.scrollTo({ top: scrollRef.current.scrollHeight, behavior: "smooth" });
  }, [messages]);

  const send = () => {
    const text = input.trim();
    if (!text) return;
    setInput("");
    onSend(text);
  };

  const runningTools = useMemo(() => {
    return agents
      .filter((a) => a.status === "running")
      .flatMap((a) => a.tools.slice(0, 2))
      .slice(0, 4);
  }, [agents]);

  return (
    <section style={{ flex: 1, display: "flex", flexDirection: "column", minWidth: 0 }}>
      {/* Center Header: Session */}
      <div
        style={{
          padding: "14px 24px 12px",
          borderBottom: "1px solid var(--surface-border)",
          background: "var(--surface-card)",
          display: "flex",
          alignItems: "center",
          gap: 12,
        }}
      >
        <div
          style={{
            padding: "6px 10px",
            borderRadius: 10,
            fontSize: 11,
            fontWeight: 700,
            background: `color-mix(in srgb, ${modeAccent} 15%, #fff)`,
            color: modeAccent,
            border: `1px solid color-mix(in srgb, ${modeAccent} 25%, #fff)`,
            display: "inline-flex",
            alignItems: "center",
            gap: 6,
          }}
        >
          <span>{MODES.find((m) => m.id === mode)!.icon}</span>
          {MODES.find((m) => m.id === mode)!.label}
        </div>
        <div style={{ fontSize: 13, fontWeight: 700 }}>Session · 任务工作台</div>
        <div style={{ flex: 1 }} />
        <PillBadge value={running ? "正在处理…" : "就绪"} color={running ? "var(--status-success)" : "var(--text-tertiary)"} tone="soft" dot />
      </div>

      {/* Conversation */}
      <div
        ref={scrollRef}
        style={{
          flex: 1,
          overflowY: "auto",
          padding: "20px 32px 12px",
          display: "flex",
          flexDirection: "column",
          gap: 2,
        }}
      >
        {messages.length === 0 ? (
          <EmptyStateGuide onSend={(t) => onSend(t)} onNewTask={onNewTask} />
        ) : (
          messages.map((m) => (
            <Bubble key={m.id} msg={m} />
          ))
        )}
      </div>

      {/* Running tools live feed */}
      {runningTools.length > 0 && (
        <div
          style={{
            margin: "0 32px 6px",
            padding: "8px 10px",
            background: "var(--surface-card)",
            border: "1px solid var(--surface-border)",
            borderRadius: 10,
            display: "flex",
            alignItems: "center",
            gap: 8,
            flexWrap: "wrap",
          }}
        >
          <span style={{ fontSize: 11, fontWeight: 700, color: "var(--text-secondary)" }}>
            工具流
          </span>
          {runningTools.map((t, i) => (
            <span
              key={t + i}
              style={{
                fontSize: 10.5,
                fontFamily: "ui-monospace, SFMono-Regular, Menlo, monospace",
                padding: "2px 8px",
                borderRadius: 6,
                background: "var(--accent-primary-50)",
                color: "var(--accent-primary-700)",
                border: "1px solid color-mix(in srgb, var(--accent-primary) 20%, #fff)",
              }}
            >
              ▸ {t}
            </span>
          ))}
        </div>
      )}

      {/* Composer */}
      <div
        style={{
          padding: "10px 24px 18px",
          background: "linear-gradient(180deg, transparent 0%, var(--surface-bg) 40%)",
        }}
      >
        <div
          style={{
            padding: 6,
            background: "var(--surface-card)",
            borderRadius: 14,
            border: "1px solid var(--surface-border)",
            boxShadow: "var(--shadow-md)",
          }}
        >
          <textarea
            value={input}
            onChange={(e) => setInput(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter" && !e.shiftKey) {
                e.preventDefault();
                send();
              }
            }}
            rows={2}
            placeholder="描述你的任务 —— 支持多代理编排、终端沙箱、浏览器操控、知识检索… (Enter 发送 / Shift+Enter 换行)"
            style={{
              width: "100%",
              resize: "none",
              border: "none",
              outline: "none",
              padding: "8px 10px",
              fontFamily: "inherit",
              fontSize: 13,
              lineHeight: 1.6,
              color: "var(--text-primary)",
              background: "transparent",
            }}
          />
          <div
            style={{
              display: "flex",
              alignItems: "center",
              gap: 8,
              padding: "2px 6px 2px 10px",
            }}
          >
            <div style={{ display: "inline-flex", gap: 4 }}>
              {[
                { icon: "📎", label: "插入附件" },
                { icon: "🖼️", label: "插入图片" },
                { icon: "📋", label: "粘贴剪贴板" },
                { icon: "🎯", label: "目标插件" },
                { icon: "🧪", label: "实验性工具" },
              ].map(item => (
                <button
                  key={item.icon}
                  aria-label={item.label}
                  title={item.label}
                  style={{
                    border: "1px solid var(--surface-border)",
                    background: "var(--surface-hover)",
                    width: 28,
                    height: 28,
                    borderRadius: 8,
                    fontSize: 13,
                  }}
                >
                  {item.icon}
                </button>
              ))}
            </div>
            <div style={{ flex: 1 }} />
            <span style={{ fontSize: 10.5, color: "var(--text-tertiary)" }}>
              双引擎桥 · Rust + Python
            </span>
            <button
              onClick={send}
              disabled={!input.trim() || running}
              style={{
                border: "none",
                padding: "8px 16px",
                borderRadius: 10,
                fontSize: 12,
                fontWeight: 700,
                color: "var(--accent-primary-on)",
                background:
                  "linear-gradient(180deg, var(--accent-primary-300) 0%, var(--accent-primary) 100%)",
                boxShadow: "0 6px 14px -8px var(--accent-primary-700)",
                opacity: input.trim() && !running ? 1 : 0.55,
                cursor: input.trim() && !running ? "pointer" : "not-allowed",
                display: "inline-flex",
                alignItems: "center",
                gap: 6,
              }}
            >
              发送
              <span>↵</span>
            </button>
          </div>
        </div>
      </div>
    </section>
  );
};

// ───────────── 工具面板：对齐 TUI/CLI 全部子命令 ─────────────


// === B3: React.memo 包装 — 父级无关状态变更不再触发本组件重渲染 ===
export const ChatCenter = memo(ChatCenterImpl);
