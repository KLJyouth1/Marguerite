import { useEffect, useState, memo } from "react"
import { invoke } from "@tauri-apps/api/core"
import { btnStyle, cardStyle, panelBox, panelSubtitle, panelTitle } from "../styles/panelStyles"

interface PetStatusView {
  mood: string;
  energy: number;
  affection: number;
}

const PET_MOOD_LABEL: Record<string, string> = {
  idle: "空闲",
  happy: "开心",
  sleepy: "困倦",
  working: "工作中",
};


function WhalePetImpl({ mood, running }: { mood: string; running: boolean }) {
  const effMood = running ? "working" : mood;
  const body = "var(--accent-primary-200)";
  const fin = "var(--accent-primary-300)";
  const ink = "var(--text-primary)";
  return (
    <svg viewBox="0 0 200 120" style={{ width: "100%", maxWidth: 340, display: "block", margin: "0 auto" }}>
      <g>
        {/* 喷水（Happy/Working） */}
        {(effMood === "happy" || effMood === "working") && (
          <g stroke={fin} strokeWidth={2.5} fill="none" strokeLinecap="round">
            <path d="M100 8 Q104 -2 100 -10" />
            <path d="M106 10 Q112 2 108 -6" />
          </g>
        )}
        {/* 尾鳍 */}
        <path d="M160 55 Q185 38 194 28 Q184 60 194 92 Q185 82 160 65 Z" fill={fin} />
        {/* 身体 */}
        <ellipse cx="105" cy="60" rx="62" ry="36" fill={body} />
        {/* 腹白 */}
        <ellipse cx="112" cy="76" rx="44" ry="18" fill="color-mix(in srgb, var(--surface-card) 85%, transparent)" />
        {/* 胸鳍 */}
        <ellipse cx="120" cy="72" rx="14" ry="8" fill={fin} transform="rotate(-18 120 72)" />
        {/* 眼睛 */}
        <circle cx="72" cy="50" r={effMood === "sleepy" ? 2.5 : 5} fill={ink} />
        {effMood === "working" && <circle cx="72" cy="50" r="8.5" fill="none" stroke={ink} strokeWidth="1" opacity="0.4" />}
        {/* 嘴 */}
        {effMood === "happy" ? (
          <path d="M56 66 Q66 76 78 66" stroke={ink} strokeWidth="2" fill="none" strokeLinecap="round" />
        ) : effMood === "sleepy" ? (
          <line x1="60" y1="66" x2="72" y2="66" stroke={ink} strokeWidth="2" strokeLinecap="round" />
        ) : (
          <path d="M58 64 Q68 70 76 64" stroke={ink} strokeWidth="2" fill="none" strokeLinecap="round" />
        )}
        {/* 腮红（Happy） */}
        {effMood === "happy" && <ellipse cx="52" cy="62" rx="7" ry="4" fill="var(--accent-secondary-200)" opacity="0.8" />}
        {/* 困倦 Zzz */}
        {effMood === "sleepy" && (
          <text x="86" y="30" fontSize="13" fill={ink} opacity="0.7" fontFamily="sans-serif">Z z z</text>
        )}
        {/* 呼吸浮动 */}
        <animateTransform attributeName="transform" type="translate" values="0 0; 0 -4; 0 0" dur={effMood === "working" ? "1.2s" : "3.2s"} repeatCount="indefinite" />
      </g>
    </svg>
  );
}

function PetPanelImpl({ running }: { running: boolean }) {
  const [pet, setPet] = useState<PetStatusView | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const load = async () => {
    try {
      const p = await invoke<PetStatusView>("pet_status");
      setPet(p);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  };

  useEffect(() => {
    void load();
    const timer = window.setInterval(() => void load(), 15000);
    return () => window.clearInterval(timer);
  }, []);

  const act = async (action: "pet" | "feed") => {
    setBusy(true);
    setError(null);
    try {
      const p = await invoke<PetStatusView>("pet_action", { action });
      setPet(p);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div style={panelBox}>
      <div style={panelTitle}>鲸鱼娘</div>
      <div style={panelSubtitle}>DSH Pet 原生化 — 随 AI 运行状态切换动作，摸头/投喂小鱼干提升好感度（状态持久化）</div>
      {error && <div style={{ ...cardStyle, color: "#ef4444", fontSize: 12, marginTop: 8 }}>{error}</div>}
      <div style={{ ...cardStyle, marginTop: 12, textAlign: "center" }}>
        <WhalePet mood={pet?.mood ?? "idle"} running={running} />
        <div style={{ fontSize: 13, fontWeight: 700, marginTop: 4 }}>
          {running ? "工作中" : PET_MOOD_LABEL[pet?.mood ?? "idle"] ?? pet?.mood}
        </div>
        <div style={{ fontSize: 11, color: "var(--text-secondary)", marginTop: 2 }}>
          能量 {pet?.energy ?? 100}/100 · 好感度 {pet?.affection ?? 0}
        </div>
        {/* 能量条 */}
        <div style={{ height: 8, borderRadius: 999, background: "var(--surface-hover)", marginTop: 10, overflow: "hidden", maxWidth: 300, marginLeft: "auto", marginRight: "auto" }}>
          <div style={{
            height: "100%",
            width: `${pet?.energy ?? 100}%`,
            background: (pet?.energy ?? 100) < 20 ? "var(--status-warning)" : "var(--accent-primary)",
            transition: "width 300ms ease",
          }} />
        </div>
        <div style={{ display: "flex", gap: 8, justifyContent: "center", marginTop: 14 }}>
          <button style={btnStyle} onClick={() => act("pet")} disabled={busy}>摸头</button>
          <button style={{ ...btnStyle, background: "var(--accent-secondary)" }} onClick={() => act("feed")} disabled={busy}>投喂小鱼干</button>
        </div>
        <div style={{ fontSize: 10, color: "var(--text-tertiary)", marginTop: 10 }}>
          能量每 10 秒自然衰减 1；低于 20 进入困倦。互动回能并提升好感度。
        </div>
      </div>
    </div>
  );
}

// ───────────── Better Sidebar：文件树 / 编辑器 / 浏览器 ─────────────


// === B3: React.memo 包装 — 父级无关状态变更不再触发本组件重渲染 ===
export const WhalePet = memo(WhalePetImpl);
export const PetPanel = memo(PetPanelImpl);
