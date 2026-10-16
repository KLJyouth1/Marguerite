import React from "react"

export const panelBox: React.CSSProperties = {
  flex: 1,
  overflowY: "auto",
  padding: "20px 24px",
  background: "var(--surface-bg, #0f1117)",
  color: "var(--text-primary, #e4e6eb)",
};

export const panelTitle: React.CSSProperties = {
  fontSize: 18,
  fontWeight: 700,
  marginBottom: 4,
  // 水墨主题（data-theme=ink）下自动切楷体（--font-display 仅在 ink 主题定义）
  fontFamily: "var(--font-display, inherit)",
  letterSpacing: "var(--title-letter-spacing, 0px)",
};

export const panelSubtitle: React.CSSProperties = {
  fontSize: 12,
  color: "var(--text-secondary, #8b8fa3)",
  marginBottom: 16,
};

export const cardStyle: React.CSSProperties = {
  background: "var(--surface-card, #181b24)",
  border: "1px solid var(--surface-border, #252836)",
  borderRadius: 8,
  padding: "12px 14px",
  marginBottom: 8,
};

export const btnStyle: React.CSSProperties = {
  padding: "6px 14px",
  borderRadius: 6,
  border: "1px solid var(--accent-primary, #6366f1)",
  background: "var(--accent-primary, #6366f1)",
  color: "#fff",
  fontSize: 12,
  fontWeight: 600,
  cursor: "pointer",
};

export const btnGhostStyle: React.CSSProperties = {
  ...btnStyle,
  background: "transparent",
  color: "var(--accent-primary, #6366f1)",
};

export const inputStyle: React.CSSProperties = {
  flex: 1,
  padding: "7px 10px",
  borderRadius: 6,
  border: "1px solid var(--surface-border, #252836)",
  background: "var(--surface-input, #12141c)",
  color: "var(--text-primary, #e4e6eb)",
  fontSize: 12,
  outline: "none",
};

