import { memo } from "react"
import { invoke } from "@tauri-apps/api/core"
import { useTauriInvoke } from "../hooks/useTauriInvoke"
import { btnGhostStyle, cardStyle, panelBox, panelSubtitle, panelTitle } from "../styles/panelStyles"

function SessionsPanelImpl() {
  const { data, error, loading, run } = useTauriInvoke<
    { session_id: string; summary: string; message_count: number; updated_at: string }[]
  >("session_list");

  const handleDelete = async (id: string) => {
    if (!confirm("确认删除会话 " + id.slice(0, 8) + "?")) return;
    try {
      await invoke("session_delete", { id });
      run();
    } catch (e) {
      alert("删除失败: " + (e instanceof Error ? e.message : String(e)));
    }
  };

  return (
    <div style={panelBox}>
      <div style={panelTitle}>会话管理</div>
      <div style={panelSubtitle}>对齐 CLI: marguerite session list/delete — 历史会话管理</div>
      <button style={btnGhostStyle} onClick={() => run()}>刷新</button>
      {error && <div style={{ color: "#ef4444", marginTop: 12, fontSize: 12 }}>{error}</div>}
      {loading && <div style={{ fontSize: 12, color: "var(--text-secondary)" }}>加载中…</div>}
      {data?.map((s) => (
        <div key={s.session_id} style={{ ...cardStyle, display: "flex", alignItems: "center", gap: 10 }}>
          <div style={{ flex: 1 }}>
            <div style={{ fontWeight: 600, fontSize: 12, fontFamily: "monospace" }}>{s.session_id.slice(0, 12)}…</div>
            <div style={{ fontSize: 11, color: "var(--text-secondary)" }}>{s.summary || "(无摘要)"}</div>
            <div style={{ fontSize: 10, color: "var(--text-tertiary, #555)" }}>{s.message_count} 条消息 · {s.updated_at}</div>
          </div>
          <button style={{ ...btnGhostStyle, borderColor: "#ef4444", color: "#ef4444" }} onClick={() => handleDelete(s.session_id)}>
            删除
          </button>
        </div>
      ))}
    </div>
  );
}


// === B3: React.memo 包装 — 父级无关状态变更不再触发本组件重渲染 ===
export const SessionsPanel = memo(SessionsPanelImpl);
