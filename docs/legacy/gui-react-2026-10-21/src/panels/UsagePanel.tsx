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

interface TierBudgetView {
  tokens_cap: number;
  usd_cap: number;
  tokens_used: number;
  usd_used: number;
  remaining_ratio: number;
  state: string;
}

interface TierView {
  locked_tier: string | null;
  thinking_effort: string;
  effective_tier: string;
  tier_reasons: string[];
  budget: TierBudgetView;
  tiers: Record<'flagship' | 'balanced' | 'turbo', string[]>;
}

const TIER_LABEL: Record<string, string> = { flagship: '旗舰', balanced: '均衡', turbo: '极速' };
const BUDGET_LABEL: Record<string, string> = {
  healthy: '健康',
  degraded: '吃紧（自动切便宜档）',
  emergency: '紧急（非高价值请求降 Turbo）',
  depleted: '已耗尽（拒绝新请求）',
};


export const fmtTokens = (n: number) => {
  if (n >= 1_000_000) return (n / 1_000_000).toFixed(2) + "M";
  if (n >= 1_000) return (n / 1_000).toFixed(1) + "K";
  return String(n);
};

function UsagePanelImpl() {
  const [summary, setSummary] = useState<UsageSummaryView | null>(null);
  const [days, setDays] = useState<UsageDayView[]>([]);
  const [tier, setTier] = useState<TierView | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [tierBusy, setTierBusy] = useState(false);

  const load = async () => {
    setBusy(true);
    setError(null);
    try {
      const [s, d, t] = await Promise.all([
        invoke<UsageSummaryView>("usage_summary"),
        invoke<{ days: UsageDayView[] }>("usage_daily", { days: 28 }),
        invoke<TierView>("model_tier_get"),
      ]);
      setSummary(s);
      setDays(d.days ?? []);
      setTier(t);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  const lockTier = async (t: 'auto' | 'flagship' | 'balanced' | 'turbo') => {
    setTierBusy(true);
    setError(null);
    try {
      await invoke("model_tier_set", { tier: t });
      setTier(await invoke<TierView>("model_tier_get"));
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setTierBusy(false);
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
      {error && <div style={{ ...cardStyle, color: "var(--status-error, #ef4444)", fontSize: 12, marginTop: 8 }}>{error}</div>}

      {/* 三档模型分层 + 会话预算（与 CLI/TUI/WebUI 同源 SessionState） */}
      <div style={{ ...cardStyle, marginTop: 12 }} data-testid="tier-ledger-card">
        <div style={{ display: "flex", justifyContent: "space-between", alignItems: "baseline", gap: 8 }}>
          <div style={{ fontWeight: 600, fontSize: 12 }}>模型档位与成本台账</div>
          <div style={{ fontSize: 11, color: "var(--text-tertiary)" }}>
            {tier ? `${tier.locked_tier ? `已锁定 ${tier.locked_tier}` : "自动选择"} · 思考 ${tier.thinking_effort}` : "—"}
          </div>
        </div>
        {tier ? (
          <>
            <div style={{ display: "flex", alignItems: "baseline", gap: 8, marginTop: 8 }}>
              <span style={{ fontSize: 11, color: "var(--text-secondary)" }}>生效档位</span>
              <strong style={{ fontSize: 18, color: "var(--accent-primary)" }}>
                {TIER_LABEL[tier.effective_tier] ?? tier.effective_tier}
              </strong>
              <span style={{ fontSize: 11, color: "var(--text-tertiary)" }}>
                {BUDGET_LABEL[tier.budget.state] ?? tier.budget.state}
              </span>
            </div>

            <div
              role="progressbar"
              aria-label="会话预算剩余"
              aria-valuemin={0}
              aria-valuemax={100}
              aria-valuenow={Math.round(tier.budget.remaining_ratio * 100)}
              style={{ height: 6, borderRadius: 3, background: "var(--surface-border)", overflow: "hidden", margin: "8px 0" }}
            >
              <div
                style={{
                  height: "100%",
                  width: `${Math.max(0, Math.min(100, tier.budget.remaining_ratio * 100))}%`,
                  background: tier.budget.state === "healthy" ? "var(--status-success, #22c55e)"
                    : tier.budget.state === "degraded" ? "var(--status-warning, #f59e0b)" : "var(--status-error, #ef4444)",
                }}
              />
            </div>

            <div style={{ display: "flex", flexWrap: "wrap", gap: "4px 16px", fontSize: 11, color: "var(--text-secondary)" }}>
              <span>Token {fmtTokens(tier.budget.tokens_used)} / {fmtTokens(tier.budget.tokens_cap)}</span>
              <span>成本 ${tier.budget.usd_used.toFixed(2)} / ${tier.budget.usd_cap.toFixed(2)}</span>
              <span>剩余 {Math.round(tier.budget.remaining_ratio * 100)}%</span>
            </div>

            {tier.tier_reasons?.length > 0 && (
              <ul style={{ margin: "8px 0 0", paddingLeft: 18, fontSize: 11, color: "var(--text-tertiary)" }}>
                {tier.tier_reasons.map((r, i) => <li key={i}>{r}</li>)}
              </ul>
            )}

            <div style={{ display: "flex", flexWrap: "wrap", gap: 6, marginTop: 10 }} role="group" aria-label="锁定模型档位">
              {([
                { id: "auto" as const, label: "自动" },
                { id: "flagship" as const, label: "旗舰" },
                { id: "balanced" as const, label: "均衡" },
                { id: "turbo" as const, label: "极速" },
              ]).map((t) => {
                const active = t.id === "auto" ? !tier.locked_tier : tier.locked_tier === t.id;
                return (
                  <button
                    key={t.id}
                    style={{ ...btnGhostStyle, borderColor: active ? "var(--accent-primary)" : undefined, color: active ? "var(--accent-primary)" : undefined }}
                    aria-pressed={active}
                    disabled={tierBusy}
                    onClick={() => void lockTier(t.id)}
                  >{t.label}</button>
                );
              })}
            </div>

            <div style={{ display: "grid", gap: 2, marginTop: 10, fontSize: 11, color: "var(--text-tertiary)" }}>
              {(["flagship", "balanced", "turbo"] as const).map((k) => (
                <div key={k} style={{ display: "grid", gridTemplateColumns: "42px 1fr", gap: 8 }}>
                  <span>{TIER_LABEL[k]}</span>
                  <code style={{ overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
                    {(tier.tiers?.[k] ?? []).join(" → ") || "（未配置）"}
                  </code>
                </div>
              ))}
            </div>
          </>
        ) : (
          <div style={{ fontSize: 11, color: "var(--text-tertiary)", marginTop: 8 }}>读取档位中…</div>
        )}
      </div>

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
