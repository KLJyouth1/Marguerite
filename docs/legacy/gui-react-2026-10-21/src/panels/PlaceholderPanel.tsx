import { memo } from "react"
import { cardStyle, panelBox, panelSubtitle, panelTitle } from "../styles/panelStyles"

/**
 * 未实现能力的**诚实**占位。
 *
 * 旧版这里是"该模块正在开发中，敬请期待"——这句话有两个问题：
 *   1. 它暗示"排期在做"，但很多情况下**功能根本不在当前范围**，属于误导；
 *   2. 它不告诉用户现在该怎么达成目的，用户只能空手离开。
 *
 * 现在区分两种状态：
 *   · `unsupported`：能力本身不存在 → 给出**真实可用的替代路径**；
 *   · `planned`：确实已排期 → 保留原语义，但说明得具体些。
 *
 * 无论哪种，都不伪造按钮、不放假数据——这是 Zero Demo 的底线。
 */
export type PlaceholderStatus = "unsupported" | "planned"

function PlaceholderPanelImpl({
  title,
  subtitle,
  status = "planned",
  hint,
}: {
  title: string
  subtitle: string
  status?: PlaceholderStatus
  /** 替代路径或排期说明。unsupported 时必填——不给出口的"不支持"等于甩手。 */
  hint?: string
}) {
  const badge =
    status === "unsupported"
      ? { text: "当前版本未提供", color: "var(--text-secondary)" }
      : { text: "计划中", color: "var(--accent-primary)" }

  return (
    <div style={panelBox}>
      <div style={panelTitle}>{title}</div>
      <div style={panelSubtitle}>{subtitle}</div>
      <div style={{ ...cardStyle, textAlign: "center", padding: 40 }}>
        <div style={{ fontSize: 13, color: badge.color, fontWeight: 600 }}>{badge.text}</div>
        {hint && (
          <div
            style={{
              marginTop: 10,
              fontSize: 12,
              lineHeight: 1.7,
              color: "var(--text-secondary)",
              maxWidth: 520,
              marginLeft: "auto",
              marginRight: "auto",
              textAlign: "left",
            }}
          >
            {hint}
          </div>
        )}
      </div>
    </div>
  )
}

export const PlaceholderPanel = memo(PlaceholderPanelImpl)
