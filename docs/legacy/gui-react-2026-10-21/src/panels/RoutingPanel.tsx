import { useState, memo } from "react"
import { invoke } from "@tauri-apps/api/core"
import { btnGhostStyle, btnStyle, inputStyle, panelBox, panelSubtitle, panelTitle } from "../styles/panelStyles"

function RoutingPanelImpl() {
  const [json, setJson] = useState(
    JSON.stringify(
      { mode: "router", fusion_model: "", candidate_models: [], fixed_model: "", cross_provider_tiers: false },
      null,
      2
    )
  );
  const [msg, setMsg] = useState<{ ok: boolean; text: string } | null>(null);
  const [busy, setBusy] = useState(false);

  const applyMode = (mode: string) => {
    try {
      const obj = JSON.parse(json);
      obj.mode = mode;
      setJson(JSON.stringify(obj, null, 2));
    } catch {
      /* 输入过程中解析失败则忽略 */
    }
  };

  const handleSave = async () => {
    setBusy(true);
    setMsg(null);
    try {
      await invoke("routing_save", { json });
      setMsg({ ok: true, text: "模型路由已保存" });
    } catch (e) {
      setMsg({ ok: false, text: "保存失败: " + (e instanceof Error ? e.message : String(e)) });
    } finally {
      setBusy(false);
      setTimeout(() => setMsg(null), 2500);
    }
  };

  return (
    <div style={panelBox}>
      <div style={panelTitle}>模型路由</div>
      <div style={panelSubtitle}>切换模型路由模式（direct / router / ensemble / bloom），持久化到 config.yaml（红线修复：此前保存必失败）</div>
      <div style={{ display: "flex", gap: 6, marginBottom: 10, flexWrap: "wrap" }}>
        {["direct", "router", "ensemble", "bloom"].map((m) => (
          <button key={m} style={btnGhostStyle} onClick={() => applyMode(m)}>{m}</button>
        ))}
      </div>
      <textarea
        style={{ ...inputStyle, minHeight: 160, resize: "vertical", fontFamily: "ui-monospace, monospace", fontSize: 11, marginBottom: 10 }}
        value={json}
        onChange={(e) => setJson(e.target.value)}
        spellCheck={false}
      />
      <div style={{ display: "flex", alignItems: "center", gap: 10 }}>
        <button style={btnStyle} onClick={handleSave} disabled={busy}>保存路由</button>
        {msg && <span style={{ fontSize: 12, color: msg.ok ? "var(--status-success, #22c55e)" : "var(--status-error, #ef4444)" }}>{msg.text}</span>}
      </div>
    </div>
  );
}


// === B3: React.memo 包装 — 父级无关状态变更不再触发本组件重渲染 ===
export const RoutingPanel = memo(RoutingPanelImpl);
