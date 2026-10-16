import { useEffect, useState, memo } from "react"
import { invoke } from "@tauri-apps/api/core"
import { btnGhostStyle, btnStyle, cardStyle, inputStyle, panelBox, panelSubtitle, panelTitle } from "../styles/panelStyles"

interface FsEntryView {
  name: string;
  path: string;
  is_dir: boolean;
  size: number;
}


function FilePanelImpl() {
  const [path, setPath] = useState("");
  const [pathInput, setPathInput] = useState("");
  const [entries, setEntries] = useState<FsEntryView[]>([]);
  const [openPath, setOpenPath] = useState<string | null>(null);
  const [content, setContent] = useState("");
  const [dirty, setDirty] = useState(false);
  const [url, setUrl] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const list = async (p: string) => {
    setBusy(true);
    setError(null);
    try {
      const es = await invoke<FsEntryView[]>("fs_list_dir", { path: p });
      setEntries(es);
      setPath(p);
      setPathInput(p);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  useEffect(() => {
    void list("");
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const open = async (p: string, isDir: boolean) => {
    if (isDir) {
      await list(p);
      return;
    }
    setBusy(true);
    setError(null);
    try {
      const c = await invoke<string>("fs_read_file", { path: p });
      setOpenPath(p);
      setContent(c);
      setDirty(false);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  const save = async () => {
    if (!openPath) return;
    setBusy(true);
    setError(null);
    try {
      await invoke("fs_write_file", { path: openPath, content });
      setDirty(false);
      await list(path);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  const goUp = () => {
    const p = path.trim();
    if (!p) return;
    const parent = p.replace(/[\\/]+$/, "").split(/[\\/]/).slice(0, -1).join("\\") || "";
    void list(parent);
  };

  const browse = async () => {
    setBusy(true);
    setError(null);
    try {
      await invoke("open_url", { url: url.trim() });
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  const quickOpen = async (u: string) => {
    setUrl(u);
    setBusy(true);
    setError(null);
    try {
      await invoke("open_url", { url: u });
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div style={panelBox}>
      <div style={panelTitle}>文件</div>
      <div style={panelSubtitle}>Better Sidebar 原生化 — 文件树 + 文本编辑器 + 浏览器直达（默认为工作区目录）</div>
      {error && <div style={{ ...cardStyle, color: "#ef4444", fontSize: 12, marginTop: 8 }}>{error}</div>}

      <div style={{ display: "flex", gap: 8, marginTop: 12 }}>
        <input
          style={inputStyle}
          placeholder="目录路径（留空 = 工作区根）"
          value={pathInput}
          onChange={(e) => setPathInput(e.target.value)}
          onKeyDown={(e) => e.key === "Enter" && list(pathInput)}
        />
        <button style={btnStyle} onClick={() => list(pathInput)} disabled={busy}>进入</button>
        <button style={btnGhostStyle} onClick={goUp} disabled={busy || !path}>上级</button>
      </div>

      <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr", gap: 12, marginTop: 12, alignItems: "start" }}>
        {/* 文件树 */}
        <div style={cardStyle}>
          <div style={{ fontWeight: 600, fontSize: 12, marginBottom: 8, color: "var(--text-secondary)" }}>
            目录：{path || "（工作区根）"}
          </div>
          {entries.length === 0 && !busy && (
            <div style={{ fontSize: 12, color: "var(--text-secondary)" }}>（空目录）</div>
          )}
          <div style={{ maxHeight: 420, overflowY: "auto" }}>
            {entries.map((e) => (
              <div
                key={e.path}
                onClick={() => open(e.path, e.is_dir)}
                style={{
                  display: "flex", alignItems: "center", gap: 8, padding: "5px 8px", borderRadius: 6,
                  cursor: "pointer", fontSize: 12,
                  background: openPath === e.path ? "var(--surface-hover)" : "transparent",
                }}
                title={e.path}
              >
                <span style={{ color: e.is_dir ? "var(--accent-primary)" : "var(--text-tertiary)", width: 16, textAlign: "center" }}>
                  {e.is_dir ? "▸" : "·"}
                </span>
                <span style={{ flex: 1, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>{e.name}</span>
                {!e.is_dir && <span style={{ fontSize: 10, color: "var(--text-tertiary)" }}>{e.size} B</span>}
              </div>
            ))}
          </div>
        </div>

        {/* 编辑器 */}
        <div style={cardStyle}>
          <div style={{ fontWeight: 600, fontSize: 12, marginBottom: 8, color: "var(--text-secondary)" }}>
            {openPath ? "编辑：" + openPath : "（点击左侧文件打开编辑器）"}
          </div>
          {openPath ? (
            <>
              <textarea
                style={{ ...inputStyle, width: "100%", minHeight: 280, resize: "vertical", fontFamily: "ui-monospace, monospace", fontSize: 12 }}
                value={content}
                onChange={(e) => { setContent(e.target.value); setDirty(true); }}
                spellCheck={false}
              />
              <div style={{ display: "flex", gap: 8, marginTop: 8 }}>
                <button style={btnStyle} onClick={save} disabled={busy || !dirty}>保存</button>
                {dirty && <span style={{ fontSize: 11, color: "var(--status-warning)" }}>未保存</span>}
              </div>
            </>
          ) : (
            <div style={{ fontSize: 12, color: "var(--text-secondary)", padding: "24px 0", textAlign: "center" }}>
              选择文件后在此直接编辑保存
            </div>
          )}
        </div>
      </div>

      {/* 浏览器直达 */}
      <div style={{ ...cardStyle, marginTop: 12 }}>
        <div style={{ fontWeight: 600, fontSize: 12, marginBottom: 8 }}>浏览器直达（在系统默认浏览器打开）</div>
        <div style={{ display: "flex", gap: 8 }}>
          <input
            style={inputStyle}
            placeholder="https://…"
            value={url}
            onChange={(e) => setUrl(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && browse()}
          />
          <button style={btnStyle} onClick={browse} disabled={busy || !url.trim()}>打开</button>
        </div>
        <div style={{ display: "flex", gap: 6, flexWrap: "wrap", marginTop: 8 }}>
          {["https://chat.deepseek.com", "https://github.com", "https://news.ycombinator.com"].map((u) => (
            <button key={u} style={btnGhostStyle} onClick={() => quickOpen(u)}>
              {u.replace("https://", "")}
            </button>
          ))}
        </div>
      </div>
    </div>
  );
}

// ───────────── Better Sidebar：文件树 / 编辑器 / 浏览器 ─────────────


// === B3: React.memo 包装 — 父级无关状态变更不再触发本组件重渲染 ===
export const FilePanel = memo(FilePanelImpl);
