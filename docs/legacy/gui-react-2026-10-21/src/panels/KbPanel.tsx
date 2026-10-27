import { useState, memo } from "react"
import { invoke } from "@tauri-apps/api/core"
import { btnStyle, cardStyle, inputStyle, panelBox, panelSubtitle, panelTitle } from "../styles/panelStyles"

function KbPanelImpl() {
  const [query, setQuery] = useState("");
  const [results, setResults] = useState<{ doc_id: string; score: number; text_preview: string }[]>([]);
  const [addTitle, setAddTitle] = useState("");
  const [addText, setAddText] = useState("");
  const [msg, setMsg] = useState("");

  const handleSearch = async () => {
    if (!query.trim()) return;
    try {
      const res = await invoke<{ doc_id: string; score: number; text_preview: string }[]>("kb_search", {
        query: query.trim(),
        topK: 5,
      });
      setResults(res);
    } catch (e) {
      setMsg("检索失败: " + (e instanceof Error ? e.message : String(e)));
    }
  };

  const handleAdd = async () => {
    if (!addTitle.trim() || !addText.trim()) return;
    try {
      await invoke("kb_add", { title: addTitle.trim(), text: addText.trim() });
      setAddTitle("");
      setAddText("");
      setMsg("文档已添加");
      setTimeout(() => setMsg(""), 2000);
    } catch (e) {
      setMsg("添加失败: " + (e instanceof Error ? e.message : String(e)));
    }
  };

  return (
    <div style={panelBox}>
      <div style={panelTitle}>知识库</div>
      <div style={panelSubtitle}>BM25 + 向量嵌入 + RRF 混合检索（HybridRetriever）</div>
      <div style={{ ...cardStyle, display: "flex", gap: 8 }}>
        <input style={inputStyle} placeholder="搜索知识库…" value={query} onChange={(e) => setQuery(e.target.value)} onKeyDown={(e) => e.key === "Enter" && handleSearch()} />
        <button style={btnStyle} onClick={handleSearch}>检索</button>
      </div>
      {results.map((r, i) => (
        <div key={i} style={cardStyle}>
          <div style={{ display: "flex", justifyContent: "space-between" }}>
            <span style={{ fontWeight: 600, fontSize: 12 }}>{r.doc_id}</span>
            <span style={{ fontSize: 10, color: "var(--text-secondary)" }}>score: {r.score.toFixed(4)}</span>
          </div>
          <div style={{ fontSize: 11, color: "var(--text-secondary)", marginTop: 4 }}>{r.text_preview}</div>
        </div>
      ))}
      <div style={{ ...cardStyle, marginTop: 16 }}>
        <div style={{ fontWeight: 600, fontSize: 12, marginBottom: 8 }}>添加文档</div>
        <input style={{ ...inputStyle, marginBottom: 8 }} placeholder="标题" value={addTitle} onChange={(e) => setAddTitle(e.target.value)} />
        <textarea
          style={{ ...inputStyle, minHeight: 80, resize: "vertical", marginBottom: 8, fontFamily: "inherit" }}
          placeholder="内容…"
          value={addText}
          onChange={(e) => setAddText(e.target.value)}
        />
        <button style={btnStyle} onClick={handleAdd}>添加</button>
        {msg && <span style={{ marginLeft: 10, fontSize: 11, color: "var(--status-success, #22c55e)" }}>{msg}</span>}
      </div>
    </div>
  );
}


// === B3: React.memo 包装 — 父级无关状态变更不再触发本组件重渲染 ===
export const KbPanel = memo(KbPanelImpl);
