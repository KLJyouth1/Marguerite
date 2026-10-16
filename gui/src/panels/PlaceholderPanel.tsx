import { memo } from "react"
import { cardStyle, panelBox, panelSubtitle, panelTitle } from "../styles/panelStyles"

function PlaceholderPanelImpl({ title, subtitle }: { title: string; subtitle: string }) {
  return (
    <div style={panelBox}>
      <div style={panelTitle}>{title}</div>
      <div style={panelSubtitle}>{subtitle}</div>
      <div style={{ ...cardStyle, textAlign: "center", padding: 40, color: "var(--text-secondary)", fontSize: 13 }}>
        该模块正在开发中，敬请期待
      </div>
    </div>
  );
}


// === B3: React.memo 包装 — 父级无关状态变更不再触发本组件重渲染 ===
export const PlaceholderPanel = memo(PlaceholderPanelImpl);
