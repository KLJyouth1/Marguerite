import { useState, memo } from "react"
import { invoke } from "@tauri-apps/api/core"
import { btnStyle, cardStyle, inputStyle, panelBox, panelSubtitle, panelTitle } from "../styles/panelStyles"

function WorkspacePanelImpl() {
  const [name, setName] = useState("");
  const [path, setPath] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const handleCreate = async () => {
    if (!name.trim()) return;
    setBusy(true);
    setError(null);
    setPath(null);
    try {
      const p = await invoke<string>("create_workspace_folder", { name: name.trim() });
      setPath(p);
      setName("");
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div style={panelBox}>
      <div style={panelTitle}>工作区</div>
      <div style={panelSubtitle}>在 ~/.marguerite/workspaces/ 下创建工作区目录（红线修复：此前无法直接创建文件夹）</div>
      <div style={{ ...cardStyle, display: "flex", gap: 8 }}>
        <input
          style={inputStyle}
          placeholder="工作区名称（不可含 / 或 ..）"
          value={name}
          onChange={(e) => setName(e.target.value)}
          onKeyDown={(e) => e.key === "Enter" && handleCreate()}
        />
        <button style={btnStyle} onClick={handleCreate} disabled={busy || !name.trim()}>
          {busy ? "创建中…" : "创建"}
        </button>
      </div>
      {error && <div style={{ ...cardStyle, color: "var(--status-error, #ef4444)", fontSize: 12, marginTop: 8 }}>{error}</div>}
      {path && (
        <div style={{ ...cardStyle, marginTop: 12 }}>
          <div style={{ fontSize: 11, color: "var(--text-secondary)" }}>已创建目录</div>
          <div style={{ fontSize: 12, fontFamily: "ui-monospace, monospace", marginTop: 4, wordBreak: "break-all" }}>{path}</div>
        </div>
      )}
    </div>
  );
}


// === B3: React.memo 包装 — 父级无关状态变更不再触发本组件重渲染 ===
export const WorkspacePanel = memo(WorkspacePanelImpl);
