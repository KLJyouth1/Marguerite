import React, { useRef, useState, memo } from "react"
import { invoke } from "@tauri-apps/api/core"
import { btnStyle, cardStyle, inputStyle, panelBox, panelSubtitle, panelTitle } from "../styles/panelStyles"

function ModlensPanelImpl() {
  const [hint, setHint] = useState("");
  const [result, setResult] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const fileRef = useRef<HTMLInputElement | null>(null);

  const handleFile = async (e: React.ChangeEvent<HTMLInputElement>) => {
    const file = e.target.files?.[0];
    if (!file) return;
    setBusy(true);
    setError(null);
    setResult(null);
    try {
      const buf = await file.arrayBuffer();
      const bytes = new Uint8Array(buf);
      let bin = "";
      for (let i = 0; i < bytes.length; i++) bin += String.fromCharCode(bytes[i]);
      const b64 = btoa(bin);
      const doc = await invoke<string>("modlens_analyze", { image_base64: b64, hint: hint.trim() });
      setResult(doc);
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div style={panelBox}>
      <div style={panelTitle}>视觉取证</div>
      <div style={panelSubtitle}>ModLens — 图片 → 结构化文档（文字块 / 阅读顺序 / Ⅱ语义 / 版面），无视觉模型时降级为元数据兜底</div>
      <div style={{ ...cardStyle, display: "flex", flexDirection: "column", gap: 8 }}>
        <input style={inputStyle} placeholder="分析提示（可选），如：这是一张发票" value={hint} onChange={(e) => setHint(e.target.value)} />
        <div style={{ display: "flex", alignItems: "center", gap: 10 }}>
          <button style={btnStyle} onClick={() => fileRef.current?.click()} disabled={busy}>
            {busy ? "分析中…" : "选择图片"}
          </button>
          <input ref={fileRef} type="file" accept="image/*" style={{ display: "none" }} onChange={handleFile} />
          <span style={{ fontSize: 11, color: "var(--text-secondary)" }}>支持 PNG / JPG，本地解析不上传</span>
        </div>
      </div>
      {error && <div style={{ ...cardStyle, color: "#ef4444", fontSize: 12, marginTop: 8 }}>{error}</div>}
      {result && (
        <pre style={{ ...cardStyle, fontSize: 11, whiteSpace: "pre-wrap", wordBreak: "break-all", maxHeight: 420, overflowY: "auto", fontFamily: "ui-monospace, monospace" }}>
          {result}
        </pre>
      )}
    </div>
  );
}


// === B3: React.memo 包装 — 父级无关状态变更不再触发本组件重渲染 ===
export const ModlensPanel = memo(ModlensPanelImpl);
