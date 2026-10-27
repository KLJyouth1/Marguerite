import { memo } from "react"
import { Badge } from "../components/primitives"
import { useTauriInvoke } from "../hooks/useTauriInvoke"
import { btnGhostStyle, cardStyle, panelBox, panelSubtitle, panelTitle } from "../styles/panelStyles"

function EvolvePanelImpl() {
  const { data, error, loading, run } = useTauriInvoke<
    { round: number; accepted: boolean; score: number; timestamp: string; summary: string }[]
  >("evolve_history");

  return (
    <div style={panelBox}>
      <div style={panelTitle}>自进化历史</div>
      <div style={panelSubtitle}>EvolutionRunner — prompt 自进化与回滚记录</div>
      <button style={btnGhostStyle} onClick={() => run()}>刷新</button>
      {error && <div style={{ color: "var(--status-error, #ef4444)", marginTop: 12, fontSize: 12 }}>{error}</div>}
      {loading && <div style={{ fontSize: 12, color: "var(--text-secondary)" }}>加载中…</div>}
      {data?.length === 0 && <div style={{ marginTop: 16, fontSize: 12, color: "var(--text-secondary)" }}>暂无进化记录</div>}
      {data?.map((r) => (
        <div key={r.round} style={{ ...cardStyle, display: "flex", gap: 10, alignItems: "flex-start" }}>
          <Badge ok={r.accepted} label={r.accepted ? "采纳" : "拒绝"} />
          <div style={{ flex: 1 }}>
            <div style={{ fontWeight: 600, fontSize: 12 }}>第 {r.round} 轮 · score: {r.score.toFixed(3)}</div>
            <div style={{ fontSize: 11, color: "var(--text-secondary)" }}>{r.summary}</div>
            <div style={{ fontSize: 10, color: "var(--text-tertiary, #555)" }}>{r.timestamp}</div>
          </div>
        </div>
      ))}
    </div>
  );
}


// === B3: React.memo 包装 — 父级无关状态变更不再触发本组件重渲染 ===
export const EvolvePanel = memo(EvolvePanelImpl);
