import { memo } from "react"
import { Badge } from "../components/primitives"
import { useTauriInvoke } from "../hooks/useTauriInvoke"
import { btnGhostStyle, cardStyle, panelBox, panelSubtitle, panelTitle } from "../styles/panelStyles"

function DoctorPanelImpl() {
  const { data, error, loading, run } = useTauriInvoke<{
    os: string;
    arch: string;
    config_dir: string;
    model_name: string;
    model_provider: string;
    api_key_configured: boolean;
    mcp_servers: number;
    skills_dir: string;
    checks: { name: string; ok: boolean; detail: string; hint: string }[];
    total: number;
    passed: number;
    failed: number;
    status: string;
  }>("doctor_check");

  return (
    <div style={panelBox}>
      <div style={panelTitle}>系统诊断</div>
      <div style={panelSubtitle}>对齐 CLI: marguerite doctor — 环境自检与状态报告</div>
      <button style={btnGhostStyle} onClick={() => run()}>
        {loading ? "检查中…" : "重新检查"}
      </button>
      {error && <div style={{ color: "#ef4444", marginTop: 12, fontSize: 12 }}>{error}</div>}
      {data && (
        <div style={{ marginTop: 16 }}>
          <div style={{ ...cardStyle, display: "flex", gap: 16, flexWrap: "wrap" }}>
            <span>OS: <b>{data.os}/{data.arch}</b></span>
            <span>模型: <b>{data.model_name}</b></span>
            <span>API Key: <Badge ok={data.api_key_configured} /></span>
            <span>MCP: <b>{data.mcp_servers}</b></span>
            <span>状态: <Badge ok={data.status === "ok"} label={data.status} /></span>
            <span>通过: <b>{data.passed}/{data.total}</b></span>
          </div>
          {data.checks.map((c) => (
            <div key={c.name} style={{ ...cardStyle, display: "flex", alignItems: "center", gap: 10 }}>
              <Badge ok={c.ok} />
              <div style={{ flex: 1 }}>
                <div style={{ fontWeight: 600, fontSize: 13 }}>{c.name}</div>
                <div style={{ fontSize: 11, color: "var(--text-secondary)" }}>{c.detail}</div>
                {c.hint && <div style={{ fontSize: 11, color: "#f59e0b" }}>提示: {c.hint}</div>}
              </div>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}


// === B3: React.memo 包装 — 父级无关状态变更不再触发本组件重渲染 ===
export const DoctorPanel = memo(DoctorPanelImpl);
