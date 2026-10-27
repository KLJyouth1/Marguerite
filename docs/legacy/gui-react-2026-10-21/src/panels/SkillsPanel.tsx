import { useState, memo } from "react"
import { invoke } from "@tauri-apps/api/core"
import { useTauriInvoke } from "../hooks/useTauriInvoke"
import { InlineNotice } from "../components/Notice"
import { btnGhostStyle, btnStyle, cardStyle, inputStyle, panelBox, panelSubtitle, panelTitle } from "../styles/panelStyles"

function SkillsPanelImpl() {
  const { data, error, loading, run } = useTauriInvoke<
    { name: string; description: string; version: string; author: string }[]
  >("skills_list");
  const [installUrl, setInstallUrl] = useState("");
  const [installing, setInstalling] = useState(false);
  // 原生 alert → 面板内提示（不阻塞、跟主题、读屏可播报）
  const [installError, setInstallError] = useState<string | null>(null);

  const handleInstall = async () => {
    if (!installUrl.trim()) {
      setInstallError("请填写技能 YAML 的 URL");
      return;
    }
    setInstallError(null);
    setInstalling(true);
    try {
      await invoke("skills_install", { url: installUrl.trim() });
      setInstallUrl("");
      run();
    } catch (e) {
      setInstallError("安装失败: " + (e instanceof Error ? e.message : String(e)));
    } finally {
      setInstalling(false);
    }
  };

  return (
    <div style={panelBox}>
      <div style={panelTitle}>技能管理</div>
      <div style={panelSubtitle}>对齐 CLI: marguerite skills list/install — 已安装技能与远程安装</div>
      <div style={{ display: "flex", gap: 8, marginBottom: 16 }}>
        <input
          style={inputStyle}
          placeholder="输入技能 YAML URL 进行安装…"
          value={installUrl}
          onChange={(e) => setInstallUrl(e.target.value)}
        />
        <button style={btnStyle} onClick={handleInstall} disabled={installing}>
          {installing ? "安装中…" : "安装"}
        </button>
        <button style={btnGhostStyle} onClick={() => run()}>刷新</button>
      </div>
      {error && <InlineNotice tone="error" onRetry={() => run()}>读取技能失败：{error}</InlineNotice>}
      {installError && <InlineNotice tone="error">{installError}</InlineNotice>}
      {loading && <div style={{ fontSize: 12, color: "var(--text-secondary)" }}>加载中…</div>}
      {data?.map((s) => (
        <div key={s.name} style={cardStyle}>
          <div style={{ display: "flex", justifyContent: "space-between" }}>
            <span style={{ fontWeight: 600, fontSize: 13 }}>{s.name}</span>
            <span style={{ fontSize: 10, color: "var(--text-secondary)" }}>v{s.version}</span>
          </div>
          <div style={{ fontSize: 11, color: "var(--text-secondary)", marginTop: 4 }}>{s.description}</div>
          {s.author && <div style={{ fontSize: 10, color: "var(--text-tertiary, #555)" }}>作者: {s.author}</div>}
        </div>
      ))}
    </div>
  );
}


// === B3: React.memo 包装 — 父级无关状态变更不再触发本组件重渲染 ===
export const SkillsPanel = memo(SkillsPanelImpl);
