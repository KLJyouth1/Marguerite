import { memo } from "react"
import { useTauriInvoke } from "../hooks/useTauriInvoke"
import { btnGhostStyle, cardStyle, panelBox, panelSubtitle, panelTitle } from "../styles/panelStyles"

function PluginsPanelImpl() {
  const { data, error, loading, run } = useTauriInvoke<
    { name: string; kind: string; description: string; version: string }[]
  >("plugins_list");

  return (
    <div style={panelBox}>
      <div style={panelTitle}>插件市场</div>
      <div style={panelSubtitle}>PluginSlot 统一注册槽 — 工具/技能/MCP/命令</div>
      <button style={btnGhostStyle} onClick={() => run()}>刷新</button>
      {error && <div style={{ color: "#ef4444", marginTop: 12, fontSize: 12 }}>{error}</div>}
      {loading && <div style={{ fontSize: 12, color: "var(--text-secondary)" }}>加载中…</div>}
      <div style={{ marginTop: 12, display: "grid", gridTemplateColumns: "repeat(auto-fill, minmax(280px, 1fr))", gap: 8 }}>
        {data?.map((p) => (
          <div key={p.name} style={cardStyle}>
            <div style={{ display: "flex", justifyContent: "space-between" }}>
              <span style={{ fontWeight: 600, fontSize: 12, fontFamily: "monospace" }}>{p.name}</span>
              <span style={{ fontSize: 10, color: "var(--text-secondary)" }}>v{p.version}</span>
            </div>
            <div style={{ fontSize: 10, color: "var(--accent-primary)", marginTop: 2 }}>{p.kind}</div>
            <div style={{ fontSize: 11, color: "var(--text-secondary)", marginTop: 4 }}>{p.description}</div>
          </div>
        ))}
      </div>
    </div>
  );
}


// === B3: React.memo 包装 — 父级无关状态变更不再触发本组件重渲染 ===
export const PluginsPanel = memo(PluginsPanelImpl);
