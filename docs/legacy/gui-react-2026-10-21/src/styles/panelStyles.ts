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

// ───────────── 语义状态色（勿再硬编码 #ef4444 之类）─────────────
//
// 为什么必须有这一组（2026-10-21 逐面板审计）：
// `theme.css` 早就定义了 `--status-error/warning/success/info`，且**深色主题
// （data-theme="ink"）把它们重新调过**——例如 `--status-error` 浅色是 #EF4444、
// 深色是 #B23A2E。而 9 个面板此前直接写死 `#ef4444`：
//   · 深色主题下错用了浅色主题的红，对比度与观感都不对；
//   · 同一语义散落 9 份，改一次要改 9 处（必然漏）。
//
// 用法：把错误文本包一层 `errorText`，不要自己写 color。
// 无障碍：错误/警告一律同时带 `role="alert"`（屏幕阅读器才播报），
// 该约定由 `ErrorNotice` / `InlineNotice` 组件承担，见 ../components/Notice.tsx。

/** 错误文本（跟随主题；深色下自动切到更暗的红） */
export const errorText: React.CSSProperties = {
  color: "var(--status-error, #ef4444)",
  fontSize: 12,
};

/** 警告文本 */
export const warningText: React.CSSProperties = {
  color: "var(--status-warning, #f59e0b)",
  fontSize: 12,
};

/** 成功文本 */
export const successText: React.CSSProperties = {
  color: "var(--status-success, #10b981)",
  fontSize: 12,
};

/** 错误卡片容器（用于需要边框/背景的一级错误提示） */
export const errorCard: React.CSSProperties = {
  ...cardStyle,
  borderColor: "var(--status-error, #ef4444)",
  color: "var(--status-error, #ef4444)",
  fontSize: 12,
};

