import { useState, memo } from "react"
import { invoke } from "@tauri-apps/api/core"
import { useTauriInvoke } from "../hooks/useTauriInvoke"
import { btnStyle, cardStyle, inputStyle, panelBox, panelSubtitle, panelTitle } from "../styles/panelStyles"

function SandboxPanelImpl() {
  const { data: info } = useTauriInvoke<{ os: string; platform_support: string; workspace: string }>("sandbox_info");
  const [cmd, setCmd] = useState("");
  const [result, setResult] = useState<{ stdout: string; stderr: string; exit_code: number; decision: string } | null>(null);
  const [running, setRunning] = useState(false);

  const handleExec = async () => {
    if (!cmd.trim()) return;
    setRunning(true);
    try {
      const argv = cmd.trim().split(/\s+/);
      const res = await invoke<{ stdout: string; stderr: string; exit_code: number; decision: string }>("exec_command", { argv });
      setResult(res);
    } catch (e) {
      setResult({ stdout: "", stderr: String(e), exit_code: -1, decision: "error" });
    } finally {
      setRunning(false);
    }
  };

  return (
    <div style={panelBox}>
      <div style={panelTitle}>沙箱执行</div>
      <div style={panelSubtitle}>对齐 CLI: marguerite exec -- &lt;cmd&gt; — 白名单沙箱命令执行</div>
      {info && (
        <div style={{ ...cardStyle, fontSize: 11 }}>
          平台: <b>{info.os}</b> · 沙箱: <b>{info.platform_support}</b> · 工作区: <code>{info.workspace}</code>
        </div>
      )}
      <div style={{ display: "flex", gap: 8, marginBottom: 12 }}>
        <input
          style={inputStyle}
          placeholder='输入命令，如: echo hello / cargo --version'
          value={cmd}
          onChange={(e) => setCmd(e.target.value)}
          onKeyDown={(e) => e.key === "Enter" && handleExec()}
        />
        <button style={btnStyle} onClick={handleExec} disabled={running}>
          {running ? "执行中…" : "执行"}
        </button>
      </div>
      {result && (
        <div style={cardStyle}>
          <div style={{ fontSize: 11, marginBottom: 6 }}>
            决策: <b>{result.decision}</b> · 退出码: <b>{result.exit_code}</b>
          </div>
          {result.stdout && (
            <pre style={{ margin: 0, fontSize: 11, whiteSpace: "pre-wrap", wordBreak: "break-all", color: "var(--status-success, #22c55e)" }}>
              {result.stdout}
            </pre>
          )}
          {result.stderr && (
            <pre style={{ margin: "4px 0 0", fontSize: 11, whiteSpace: "pre-wrap", wordBreak: "break-all", color: "var(--status-error, #ef4444)" }}>
              {result.stderr}
            </pre>
          )}
        </div>
      )}
    </div>
  );
}


// === B3: React.memo 包装 — 父级无关状态变更不再触发本组件重渲染 ===
export const SandboxPanel = memo(SandboxPanelImpl);
