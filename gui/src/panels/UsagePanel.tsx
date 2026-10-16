import { useEffect, useState, memo } from "react"
import { invoke } from "@tauri-apps/api/core"
import { btnGhostStyle, cardStyle, panelBox, panelSubtitle, panelTitle } from "../styles/panelStyles"


interface UsageModelRow {
  model: string;
  calls: number;
  prompt_tokens: number;
  completion_tokens: number;
  total_tokens: number;
}

interface UsageSummaryView {
  total_tokens: number;
  total_input_tokens: number;
  total_output_tokens: number;
  total_sessions: number;
  runs: number;
  models: UsageModelRow[];
  since: number | null;
}

interface UsageDayView {
  day: number;
  total_tokens: number;
  calls: number;
}


export const fmtTokens = (n: number) => {
  if (n >= 1_000_000) return (n / 1_000_000).toFixed(2) + "M";
  if (n >= 1_000) return (n / 1_000).toFixed(1) + "K";
  return String(n);
};

function UsagePanelImpl() {
  const [summary, setSummary] = useState<UsageSummaryView | null>(null);
  const [days, setDays] = useState<UsageDayView[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const load = async () => {
    setBusy(true);
    setError(null);
    try {
      const [s, d] = await Promise.all([
        invoke<UsageSummaryView>("usage_summary"),
        invoke<{ days: UsageDayView[] }>("usage_daily", { days: 28 }),
      ]);
      setSummary(s);
      setDays(d.days ?? []);
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

  const maxDay = days.reduce((m, d) => Math.max(m, d.total_tokens), 1);
  // 月度开销：按 UTC 日桶聚合到月
  const monthly = new Map<string, { tokens: number; calls: number }>();
  for (const d of days) {
    const dt = new Date(d.day);
    const key = `${dt.getUTCFullYear()}-${String(dt.getUTCMonth() + 1).padStart(2, "0")}`;
    const cur = monthly.get(key) ?? { tokens: 0, calls: 0 };
    cur.tokens += d.total_tokens;
    cur.calls += d.calls;
    monthly.set(key, cur);
  }
  const months = [...monthly.entries()].sort((a, b) => b[0].localeCompare(a[0]));

  return (
    <div style={panelBox}>
      <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center" }}>
        <div>
          <div style={panelTitle}>消费统计</div>
          <div style={panelSubtitle}>Daily Usage — Token 消耗 / 热力图 / 月度开销（台账落盘 ~/.marguerite/usage.db）</div>
        </div>
        <button style={btnGhostStyle} onClick={() => load()} disabled={busy}>刷新</button>
      </div>
      {error && <div style={{ ...cardStyle, color: "#ef4444", fontSize: 12, marginTop: 8 }}>{error}</div>}

      {/* 概览卡片 */}
      <div style={{ display: "grid", gridTemplateColumns: "repeat(auto-fill, minmax(150px, 1fr))", gap: 8, marginTop: 12 }}>
        {[
          { label: "总 Token", value: summary ? fmtTokens(summary.total_tokens) : "—" },
          { label: "输入 Token", value: summary ? fmtTokens(summary.total_input_tokens) : "—" },
          { label: "输出 Token", value: summary ? fmtTokens(summary.total_output_tokens) : "—" },
          { label: "调用次数", value: summary ? String(summary.runs) : "—" },
          { label: "会话数", value: summary ? String(summary.total_sessions) : "—" },
          { label: "账户余额", value: "BYOK" },
        ].map((c) => (
          <div key={c.label} style={cardStyle}>
            <div style={{ fontSize: 11, color: "var(--text-secondary)" }}>{c.label}</div>
            <div style={{ fontSize: 20, fontWeight: 700, marginTop: 4, color: "var(--accent-primary)" }}>{c.value}</div>
            {c.label === "账户余额" && (
              <div style={{ fontSize: 10, color: "var(--text-tertiary)", marginTop: 2 }}>自带密钥，无平台余额概念</div>
            )}
          </div>
        ))}
      </div>

      {/* 近 28 天热力图 */}
      <div style={{ ...cardStyle, marginTop: 12 }}>
        <div style={{ fontWeight: 600, fontSize: 12, marginBottom: 8 }}>近 28 天消耗热力图（按日，UTC）</div>
        {days.length === 0 ? (
          <div style={{ fontSize: 12, color: "var(--text-secondary)" }}>暂无用量记录 — 发起对话后自动入账</div>
        ) : (
          <div style={{ display: "flex", gap: 4, flexWrap: "wrap" }}>
            {days.map((d) => {
              const ratio = d.total_tokens / maxDay;
              const dt = new Date(d.day);
              return (
                <div
                  key={d.day}
                  title={`${dt.toLocaleDateString()} · ${fmtTokens(d.total_tokens)} tokens · ${d.calls} 次`}
                  style={{
                    width: 22, height: 22, borderRadius: 4,
                    background: `color-mix(in srgb, var(--accent-primary) ${Math.round(8 + ratio * 72)}%, transparent)`,
                    cursor: "default",
                  }}
                />
              );
            })}
          </div>
        )}
      </div>

      {/* 月度开销 */}
      <div style={{ ...cardStyle, marginTop: 12 }}>
        <div style={{ fontWeight: 600, fontSize: 12, marginBottom: 8 }}>月度开销（Token 计 · 价格表未配置故不折算金额）</div>
        {months.length === 0 ? (
          <div style={{ fontSize: 12, color: "var(--text-secondary)" }}>暂无数据</div>
        ) : (
          months.map(([k, v]) => (
            <div key={k} style={{ display: "flex", justifyContent: "space-between", fontSize: 12, padding: "4px 0", borderBottom: "1px solid var(--surface-border)" }}>
              <span style={{ fontWeight: 600 }}>{k}</span>
              <span style={{ color: "var(--text-secondary)" }}>{fmtTokens(v.tokens)} tokens · {v.calls} 次</span>
            </div>
          ))
        )}
      </div>

      {/* 分模型明细 */}
      {summary && summary.models.length > 0 && (
        <div style={{ ...cardStyle, marginTop: 12 }}>
          <div style={{ fontWeight: 600, fontSize: 12, marginBottom: 8 }}>分模型明细</div>
          {summary.models.map((m) => (
            <div key={m.model} style={{ display: "flex", justifyContent: "space-between", fontSize: 12, padding: "4px 0" }}>
              <span style={{ fontFamily: "monospace" }}>{m.model}</span>
              <span style={{ color: "var(--text-secondary)" }}>{m.calls} 次 · 入 {fmtTokens(m.prompt_tokens)} / 出 {fmtTokens(m.completion_tokens)}</span>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}

// ───────────── 鲸鱼娘桌宠（原生状态机）─────────────


// === B3: React.memo 包装 — 父级无关状态变更不再触发本组件重渲染 ===
export const UsagePanel = memo(UsagePanelImpl);
