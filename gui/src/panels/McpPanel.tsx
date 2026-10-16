import { memo } from "react"
import { useTauriInvoke } from "../hooks/useTauriInvoke"
import { btnGhostStyle, cardStyle, panelBox, panelSubtitle, panelTitle } from "../styles/panelStyles"

function McpPanelImpl() {
  const { data, error, loading, run } = useTauriInvoke<
    { name: string; command: string; args: string[] }[]
  >("mcp_list");

  return (
    <div style={panelBox}>
      <div style={panelTitle}>MCP 服务器</div>
      <div style={panelSubtitle}>对齐 CLI: marguerite mcp list — 已配置的 MCP 工具服务器</div>
      <button style={btnGhostStyle} onClick={() => run()}>刷新</button>
      {error && <div style={{ color: "#ef4444", marginTop: 12, fontSize: 12 }}>{error}</div>}
      {loading && <div style={{ fontSize: 12, color: "var(--text-secondary)" }}>加载中…</div>}
      {data?.length === 0 && <div style={{ marginTop: 16, fontSize: 12, color: "var(--text-secondary)" }}>未配置 MCP 服务器</div>}
      {data?.map((s) => (
        <div key={s.name} style={cardStyle}>
          <div style={{ fontWeight: 600, fontSize: 13 }}>{s.name}</div>
          <div style={{ fontSize: 11, color: "var(--text-secondary)", marginTop: 4, fontFamily: "monospace" }}>
            {s.command} {s.args.join(" ")}
          </div>
        </div>
      ))}
    </div>
  );
}


// === B3: React.memo 包装 — 父级无关状态变更不再触发本组件重渲染 ===
export const McpPanel = memo(McpPanelImpl);
