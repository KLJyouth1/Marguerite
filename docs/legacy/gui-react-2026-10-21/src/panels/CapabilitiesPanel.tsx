import { memo } from "react"
import { Badge } from "../components/primitives"
import { useTauriInvoke } from "../hooks/useTauriInvoke"
import { cardStyle, panelBox, panelSubtitle, panelTitle } from "../styles/panelStyles"

function CapabilitiesPanelImpl() {
  const { data, error } = useTauriInvoke<
    { group: string; items: { id: string; name: string; tier: string; maturity: string; description: string }[] }[]
  >("capabilities_groups");

  return (
    <div style={panelBox}>
      <div style={panelTitle}>能力总览</div>
      <div style={panelSubtitle}>原生 harness 能力注册表（SSOT）— 按组展示真实落地状态</div>
      {error && <div style={{ color: "var(--status-error, #ef4444)", fontSize: 12 }}>{error}</div>}
      {data?.map((g) => (
        <div key={g.group} style={{ marginBottom: 16 }}>
          <div style={{ fontWeight: 700, fontSize: 13, marginBottom: 6, color: "var(--accent-primary)" }}>{g.group}</div>
          {g.items.map((c) => (
            <div key={c.id} style={{ ...cardStyle, display: "flex", alignItems: "center", gap: 8, padding: "8px 12px" }}>
              <Badge ok={c.maturity === "Real"} label={c.maturity} />
              <div style={{ flex: 1 }}>
                <span style={{ fontWeight: 600, fontSize: 12 }}>{c.name}</span>
                <span style={{ fontSize: 10, color: "var(--text-secondary)", marginLeft: 8 }}>{c.tier}</span>
              </div>
            </div>
          ))}
        </div>
      ))}
    </div>
  );
}


// === B3: React.memo 包装 — 父级无关状态变更不再触发本组件重渲染 ===
export const CapabilitiesPanel = memo(CapabilitiesPanelImpl);
