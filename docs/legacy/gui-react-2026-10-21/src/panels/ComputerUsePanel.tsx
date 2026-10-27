import { useState, memo } from "react"
import { invoke } from "@tauri-apps/api/core"
import { btnStyle, cardStyle, inputStyle, panelBox, panelSubtitle, panelTitle } from "../styles/panelStyles"

function ComputerUsePanelImpl() {
  const [action, setAction] = useState("screenshot");
  const [x, setX] = useState("");
  const [y, setY] = useState("");
  const [text, setText] = useState("");
  const [key, setKey] = useState("");
  const [output, setOutput] = useState("");
  const [busy, setBusy] = useState(false);

  const actions = [
    "screenshot", "click", "double_click", "right_click", "middle_click",
    "type", "key", "key_combo", "scroll", "move",
    "clipboard_set", "clipboard_get", "window_focus",
    "cursor_pos", "key_state", "idle_time", "perceive",
  ];

  const handleRun = async () => {
    setBusy(true);
    setOutput("执行中…");
    try {
      const args: Record<string, unknown> = { action };
      if (x) args.x = parseInt(x);
      if (y) args.y = parseInt(y);
      if (text) args.text = text;
      if (key) args.key = key;
      const res = await invoke<string>("computer_use_action", args);
      setOutput(res);
    } catch (e) {
      setOutput("错误: " + (e instanceof Error ? e.message : String(e)));
    } finally {
      setBusy(false);
    }
  };

  const needsCoords = ["click", "double_click", "right_click", "middle_click", "move", "scroll"].includes(action);
  const needsText = action === "type" || action === "clipboard_set";
  const needsKey = ["key", "key_combo", "window_focus", "key_state"].includes(action);

  return (
    <div style={panelBox}>
      <div style={panelTitle}>电脑操控</div>
      <div style={panelSubtitle}>对齐 CLI/TUI computer_use 工具 — 屏幕感知 + 键鼠控制 + GUI 定位（三层架构）</div>
      <div style={{ ...cardStyle, display: "flex", flexDirection: "column", gap: 8 }}>
        <div style={{ display: "flex", gap: 8, flexWrap: "wrap" }}>
          {actions.map((a) => (
            <button
              key={a}
              onClick={() => setAction(a)}
              style={{
                padding: "4px 10px",
                borderRadius: 5,
                border: "1px solid " + (action === a ? "var(--accent-primary)" : "var(--surface-border)"),
                background: action === a ? "var(--accent-primary)" : "transparent",
                color: action === a ? "#fff" : "var(--text-secondary)",
                fontSize: 11,
                cursor: "pointer",
              }}
            >
              {a}
            </button>
          ))}
        </div>
        <div style={{ display: "flex", gap: 8 }}>
          {needsCoords && (
            <>
              <input style={{ ...inputStyle, maxWidth: 100 }} placeholder="X" value={x} onChange={(e) => setX(e.target.value)} />
              <input style={{ ...inputStyle, maxWidth: 100 }} placeholder="Y" value={y} onChange={(e) => setY(e.target.value)} />
            </>
          )}
          {needsText && <input style={inputStyle} placeholder="文本内容" value={text} onChange={(e) => setText(e.target.value)} />}
          {needsKey && <input style={inputStyle} placeholder="按键/窗口标题" value={key} onChange={(e) => setKey(e.target.value)} />}
          <button style={btnStyle} onClick={handleRun} disabled={busy}>{busy ? "…" : "执行"}</button>
        </div>
      </div>
      <pre style={{ ...cardStyle, fontSize: 11, whiteSpace: "pre-wrap", wordBreak: "break-all", maxHeight: 400, overflowY: "auto" }}>
        {output || "选择操作并点击「执行」"}
      </pre>
    </div>
  );
}


// === B3: React.memo 包装 — 父级无关状态变更不再触发本组件重渲染 ===
export const ComputerUsePanel = memo(ComputerUsePanelImpl);
