import { useState, memo } from "react"
import { invoke } from "@tauri-apps/api/core"
import { useTauriInvoke } from "../hooks/useTauriInvoke"
import { btnGhostStyle, btnStyle, cardStyle, inputStyle, panelBox, panelSubtitle, panelTitle } from "../styles/panelStyles"

function FleetPanelImpl() {
  const { data, error, run } = useTauriInvoke<
    { role_id: string; provider: string; model: string; reasoning: string }[]
  >("fleet_list");
  const [task, setTask] = useState("");
  const [recommendation, setRecommendation] = useState<{ role_id: string; provider: string; model: string; reasoning: string } | null>(null);

  const handleRoute = async () => {
    if (!task.trim()) return;
    try {
      const res = await invoke<{ role_id: string; provider: string; model: string; reasoning: string }>("fleet_route", {
        task: task.trim(),
      });
      setRecommendation(res);
    } catch (e) {
      alert("路由失败: " + (e instanceof Error ? e.message : String(e)));
    }
  };

  return (
    <div style={panelBox}>
      <div style={panelTitle}>模型舰队</div>
      <div style={panelSubtitle}>Fleet 多角色路由 — architect/builder/reviewer 并行编排</div>
      <button style={btnGhostStyle} onClick={() => run()}>刷新</button>
      {error && <div style={{ color: "#ef4444", marginTop: 12, fontSize: 12 }}>{error}</div>}
      <div style={{ marginTop: 12, display: "flex", gap: 8, flexWrap: "wrap" }}>
        {data?.map((r) => (
          <div key={r.role_id} style={{ ...cardStyle, minWidth: 200, flex: 1 }}>
            <div style={{ fontWeight: 700, fontSize: 13, color: "var(--accent-primary)" }}>{r.role_id}</div>
            <div style={{ fontSize: 11, marginTop: 4 }}>模型: <b>{r.model}</b></div>
            <div style={{ fontSize: 10, color: "var(--text-secondary)" }}>{r.provider}</div>
            <div style={{ fontSize: 10, color: "var(--text-tertiary, #555)" }}>推理: {r.reasoning}</div>
          </div>
        ))}
      </div>
      <div style={{ ...cardStyle, marginTop: 16 }}>
        <div style={{ fontWeight: 600, fontSize: 12, marginBottom: 8 }}>任务路由推荐</div>
        <div style={{ display: "flex", gap: 8 }}>
          <input style={inputStyle} placeholder="描述任务，推荐角色…" value={task} onChange={(e) => setTask(e.target.value)} onKeyDown={(e) => e.key === "Enter" && handleRoute()} />
          <button style={btnStyle} onClick={handleRoute}>推荐</button>
        </div>
        {recommendation && (
          <div style={{ marginTop: 10, fontSize: 12 }}>
            推荐角色: <b style={{ color: "var(--accent-primary)" }}>{recommendation.role_id}</b> · 模型: {recommendation.model}
          </div>
        )}
      </div>
    </div>
  );
}


// === B3: React.memo 包装 — 父级无关状态变更不再触发本组件重渲染 ===
export const FleetPanel = memo(FleetPanelImpl);
