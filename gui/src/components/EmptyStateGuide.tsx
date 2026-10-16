import React, { memo } from "react"

export const EMPTY_START_SUGGESTIONS = [
  "帮我整理一下这个目录里的文件",
  "分析一下当前引擎的健康状态",
  "学习这个项目的架构并总结给我",
];

const EmptyStateGuideImpl: React.FC<{ onSend: (text: string) => void; onNewTask: () => void }> = ({
  onSend,
  onNewTask,
}) => {
  return (
    <div
      style={{
        margin: "auto",
        maxWidth: 560,
        width: "100%",
        padding: "36px 24px",
        background: "var(--surface-card)",
        border: "1px solid var(--surface-border)",
        borderRadius: 16,
        boxShadow: "0 10px 40px -20px rgba(0,0,0,0.25)",
        display: "flex",
        flexDirection: "column",
        alignItems: "center",
        gap: 14,
        textAlign: "center",
      }}
    >
      <div
        style={{
          fontSize: 30,
          width: 56,
          height: 56,
          display: "grid",
          placeItems: "center",
          borderRadius: 16,
          background: "color-mix(in srgb, var(--accent-primary) 12%, #fff)",
          color: "var(--accent-primary)",
        }}
      >
        ✦
      </div>
      <div style={{ fontSize: 17, fontWeight: 800, color: "var(--text-primary)" }}>
        欢迎使用 Marguerite
      </div>
      <div style={{ fontSize: 13, color: "var(--text-secondary)", lineHeight: 1.7, maxWidth: 420 }}>
        直接描述你的第一个任务，例如「帮我分析这个项目并写一份架构说明」。
        <br />
        或点击左侧「新建任务」开启高级计划模式。
      </div>
      <div
        style={{
          display: "flex",
          gap: 8,
          flexWrap: "wrap",
          justifyContent: "center",
          marginTop: 6,
        }}
      >
        {EMPTY_START_SUGGESTIONS.map((s) => (
          <button
            key={s}
            onClick={() => onSend(s)}
            style={{
              padding: "8px 14px",
              border: "1px solid var(--surface-border)",
              background: "var(--surface-hover)",
              borderRadius: 999,
              color: "var(--text-secondary)",
              fontSize: 12,
              cursor: "pointer",
            }}
          >
            {s}
          </button>
        ))}
      </div>
      <button
        onClick={onNewTask}
        style={{
          marginTop: 4,
          padding: "10px 22px",
          border: "none",
          borderRadius: 10,
          background: "var(--accent-primary)",
          color: "#fff",
          fontSize: 13,
          fontWeight: 700,
          cursor: "pointer",
        }}
      >
        ＋ 新建任务
      </button>
    </div>
  );
};


// === B3: React.memo 包装 — 父级无关状态变更不再触发本组件重渲染 ===
export const EmptyStateGuide = memo(EmptyStateGuideImpl);
