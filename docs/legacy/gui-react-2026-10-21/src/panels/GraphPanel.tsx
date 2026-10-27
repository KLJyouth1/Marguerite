import { useEffect, useState, memo } from "react"
import { invoke } from "@tauri-apps/api/core"
import { btnGhostStyle, btnStyle, cardStyle, inputStyle, panelBox, panelSubtitle, panelTitle } from "../styles/panelStyles"


interface KbGraphNodeView {
  id: string;
  degree: number;
}

interface KbGraphEdgeView {
  from: string;
  to: string;
  kind: string;
  weight: number;
}

interface KbGraphView {
  nodes: KbGraphNodeView[];
  edges: KbGraphEdgeView[];
  entity_count: number;
  edge_count: number;
}

const GRAPH_EDGE_COLOR: Record<string, string> = {
  co_occurs: "#94A3B8",
  is_a: "#2563EB",
  uses: "#059669",
  implements: "#D97706",
  provides: "#0D9488",
};

export const GRAPH_KIND_LABEL: Record<string, string> = {
  co_occurs: "共现",
  is_a: "是",
  uses: "使用",
  implements: "实现",
  provides: "提供",
};


function GraphPanelImpl() {
  const [graph, setGraph] = useState<KbGraphView | null>(null);
  const [selected, setSelected] = useState<string | null>(null);
  const [related, setRelated] = useState<string[]>([]);
  const [seedTitle, setSeedTitle] = useState("");
  const [seedText, setSeedText] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const load = async () => {
    setBusy(true);
    setError(null);
    try {
      const g = await invoke<KbGraphView>("kb_graph_view");
      setGraph(g);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  useEffect(() => {
    void load();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const pick = async (id: string) => {
    setSelected(id);
    setRelated([]);
    try {
      const r = await invoke<string[]>("kb_graph_related", { entity: id });
      setRelated(r);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  };

  const addDoc = async () => {
    if (!seedTitle.trim() || !seedText.trim()) return;
    setBusy(true);
    setError(null);
    try {
      await invoke("kb_add", { title: seedTitle.trim(), text: seedText.trim() });
      setSeedTitle("");
      setSeedText("");
      await load();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  // 环形布局：节点按度数降序排在圆环上
  const nodes = graph?.nodes ?? [];
  const pos = new Map<string, { x: number; y: number; r: number }>();
  const sorted = [...nodes].sort((a, b) => b.degree - a.degree);
  const cx = 400;
  const cy = 230;
  const radius = Math.max(120, 175);
  sorted.forEach((n, i) => {
    const angle = (i / Math.max(1, sorted.length)) * Math.PI * 2 - Math.PI / 2;
    pos.set(n.id, {
      x: cx + Math.cos(angle) * radius,
      y: cy + Math.sin(angle) * radius,
      r: Math.min(16, 5 + n.degree * 2.2),
    });
  });

  const neighborSet = new Set(related);

  return (
    <div style={panelBox}>
      <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center" }}>
        <div>
          <div style={panelTitle}>关系图谱</div>
          <div style={panelSubtitle}>
            KB 2.0 质量层 · kb.graph — 实体共现/定型关系抽取（离线启发式，零 NLP 依赖）
            {graph ? ` · ${graph.entity_count} 实体 / ${graph.edge_count} 关系` : ""}
          </div>
        </div>
        <button style={btnGhostStyle} onClick={() => load()} disabled={busy}>刷新</button>
      </div>

      {error && <div style={{ ...cardStyle, color: "var(--status-error, #ef4444)", fontSize: 12, marginTop: 8 }}>{error}</div>}

      {/* 种子入库（同时进 KB + 图谱） */}
      <div style={{ ...cardStyle, marginTop: 12, display: "flex", flexDirection: "column", gap: 6 }}>
        <input style={inputStyle} placeholder="标题（doc_id）" value={seedTitle} onChange={(e) => setSeedTitle(e.target.value)} />
        <textarea
          style={{ ...inputStyle, minHeight: 64, resize: "vertical", fontSize: 12 }}
          placeholder="正文（实体用大写开头或引号，如：Rust and Kubernetes are used for cloud native systems. Rust uses ownership.）"
          value={seedText}
          onChange={(e) => setSeedText(e.target.value)}
        />
        <div>
          <button style={btnStyle} onClick={addDoc} disabled={busy || !seedTitle.trim() || !seedText.trim()}>
            添加知识条目
          </button>
        </div>
      </div>

      {/* 图谱 SVG */}
      {nodes.length > 0 ? (
        <div style={{ ...cardStyle, marginTop: 12, overflow: "auto" }}>
          <svg viewBox="0 0 800 460" style={{ width: "100%", height: 420, display: "block" }}>
            {(graph?.edges ?? []).map((e, i) => {
              const a = pos.get(e.from);
              const b = pos.get(e.to);
              if (!a || !b) return null;
              const active = selected && (e.from === selected || e.to === selected);
              return (
                <line
                  key={i}
                  x1={a.x} y1={a.y} x2={b.x} y2={b.y}
                  stroke={GRAPH_EDGE_COLOR[e.kind] ?? "#CBD5E1"}
                  strokeWidth={Math.min(3, 0.6 + e.weight * 0.7)}
                  opacity={selected ? (active ? 0.95 : 0.12) : 0.55}
                />
              );
            })}
            {sorted.map((n) => {
              const p = pos.get(n.id)!;
              const isSel = selected === n.id;
              const isNeighbor = selected !== null && neighborSet.has(n.id);
              const fill = isSel ? "var(--accent-primary)" : isNeighbor ? "#0D9488" : "#F1F5F9";
              const stroke = isSel ? "var(--accent-primary)" : "#64748B";
              return (
                <g key={n.id} style={{ cursor: "pointer" }} onClick={() => pick(n.id)}>
                  <circle cx={p.x} cy={p.y} r={p.r} fill={fill} stroke={stroke} strokeWidth={isSel ? 2.5 : 1.2} />
                  <text
                    x={p.x}
                    y={p.y + p.r + 13}
                    textAnchor="middle"
                    fontSize={11}
                    fill={isSel ? "var(--accent-primary)" : "var(--text-secondary)"}
                    fontWeight={isSel ? 700 : 400}
                  >
                    {n.id.length > 16 ? n.id.slice(0, 15) + "…" : n.id}
                  </text>
                </g>
              );
            })}
          </svg>
        </div>
      ) : (
        <div style={{ ...cardStyle, marginTop: 12, textAlign: "center", color: "var(--text-secondary)", fontSize: 12, padding: 32 }}>
          图谱为空 — 先在上方添加知识条目，或到「知识库」页添加文档后刷新
        </div>
      )}

      {/* 选中实体详情 */}
      {selected && (
        <div style={{ ...cardStyle, marginTop: 12 }}>
          <div style={{ fontWeight: 700, fontSize: 13 }}>{selected}</div>
          <div style={{ fontSize: 11, color: "var(--text-secondary)", marginTop: 4 }}>
            {related.length > 0 ? `直接关联（${related.length}）：` : "无直接关联实体"}
          </div>
          <div style={{ display: "flex", gap: 6, flexWrap: "wrap", marginTop: 6 }}>
            {related.map((r) => (
              <span key={r} onClick={() => pick(r)} style={{ fontSize: 11, padding: "3px 8px", borderRadius: 999, background: "var(--surface-hover)", border: "1px solid var(--surface-border)", cursor: "pointer" }}>
                {r}
              </span>
            ))}
          </div>
          <div style={{ fontSize: 10, color: "var(--text-tertiary)", marginTop: 8 }}>
            图例：共现 灰 · 是 蓝 · 使用 绿 · 实现 橙 · 提供 青
          </div>
        </div>
      )}
    </div>
  );
}

// ───────────── Daily Usage（Token 消费统计，原生内置）─────────────


// === B3: React.memo 包装 — 父级无关状态变更不再触发本组件重渲染 ===
export const GraphPanel = memo(GraphPanelImpl);
