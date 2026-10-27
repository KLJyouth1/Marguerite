// 统一提示组件：错误 / 警告 / 成功 / 加载。
//
// 为什么要有它（2026-10-21 逐面板审计）：
//   1. **无障碍**：此前 9 个面板的错误提示都是裸 `<div>`，没有 `role="alert"`，
//      屏幕阅读器不会播报——用户只看到"点了没反应"。
//   2. **主题**：错误色硬编码 `#ef4444`，深色主题（ink）下 `--status-error` 是
//      `#B23A2E`，写死等于用错主题色。
//   3. **一致性**：同一语义在 9 个文件里有 6 种写法（有的 marginTop 12、有的 8、
//      有的塞进 cardStyle）。收敛到一处，改一次全站生效。

import { memo, type ReactNode } from "react"
import { cardStyle, errorText, successText, warningText } from "../styles/panelStyles"

export type NoticeTone = "error" | "warning" | "success"

const TONE_STYLE = {
  error: { ...errorText, color: "var(--status-error, #ef4444)" },
  warning: { ...warningText, color: "var(--status-warning, #f59e0b)" },
  success: { ...successText, color: "var(--status-success, #10b981)" },
} as const

/**
 * 内联提示。默认渲染为 `role="alert"`（错误/警告必须让读屏用户知道），
 * 成功提示用 `role="status"`——它不是紧急事件，不该打断当前朗读。
 */
function InlineNoticeImpl({
  tone,
  children,
  onRetry,
  retryLabel = "重试",
}: {
  tone: NoticeTone
  children: ReactNode
  /** 可恢复错误给一个重试出口；不给出口的错误提示等于甩手 */
  onRetry?: () => void
  retryLabel?: string
}) {
  return (
    <div
      role={tone === "success" ? "status" : "alert"}
      style={{ ...TONE_STYLE[tone], marginTop: 8, display: "flex", alignItems: "center", gap: 8 }}
    >
      <span style={{ flex: 1, minWidth: 0 }}>{children}</span>
      {onRetry && (
        <button
          onClick={onRetry}
          style={{
            flex: "0 0 auto",
            padding: "2px 8px",
            fontSize: 11,
            borderRadius: 4,
            border: "1px solid currentColor",
            background: "transparent",
            color: "inherit",
            cursor: "pointer",
          }}
        >
          {retryLabel}
        </button>
      )}
    </div>
  )
}

export const InlineNotice = memo(InlineNoticeImpl)

/** 卡片式提示（用于需要更醒目的一级错误块） */
function NoticeCardImpl({
  tone,
  children,
  onRetry,
}: {
  tone: NoticeTone
  children: ReactNode
  onRetry?: () => void
}) {
  return (
    <div
      role={tone === "success" ? "status" : "alert"}
      style={{
        ...cardStyle,
        marginTop: 8,
        borderColor: TONE_STYLE[tone].color,
        color: TONE_STYLE[tone].color,
        fontSize: 12,
        display: "flex",
        alignItems: "center",
        gap: 8,
      }}
    >
      <span style={{ flex: 1, minWidth: 0 }}>{children}</span>
      {onRetry && (
        <button
          onClick={onRetry}
          style={{
            flex: "0 0 auto",
            padding: "2px 8px",
            fontSize: 11,
            borderRadius: 4,
            border: "1px solid currentColor",
            background: "transparent",
            color: "inherit",
            cursor: "pointer",
          }}
        >
          重试
        </button>
      )}
    </div>
  )
}

export const NoticeCard = memo(NoticeCardImpl)
