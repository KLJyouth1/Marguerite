import React, { useEffect, useState, memo } from "react"
import { PillBadge } from "./primitives"
import { MODES } from "../constants"
import { ModeKind } from "../types"

const TopBarImpl: React.FC<{
  mode: ModeKind;
  onModeChange: (m: ModeKind) => void;
  engineStatus: { kind: "ok" | "warn" | "err"; label: string };
  sidecar: "idle" | "starting" | "running" | "stopped" | "crashed";
  onStartSidecar: () => void;
}> = ({ mode, onModeChange, engineStatus, sidecar, onStartSidecar }) => {
  // 中国水墨主题切换：persist localStorage，作用于 documentElement.dataset.theme
  const [theme, setTheme] = useState<"light" | "ink">(() =>
    (localStorage.getItem("mg-theme") as "light" | "ink") || "light"
  );
  useEffect(() => {
    document.documentElement.dataset.theme = theme;
    localStorage.setItem("mg-theme", theme);
  }, [theme]);
  return (
    <header
      style={{
        height: 56,
        background: "var(--surface-toolbar)",
        borderBottom: "1px solid var(--surface-border)",
        display: "flex",
        alignItems: "center",
        gap: 14,
        padding: "0 18px",
      }}
    >
      {/* Mode switcher — 3 intensities x 3 working modes (3x3 matrix) */}
      {(["minimal", "standard", "extreme"] as const).map((inten) => {
        const gLabel = inten === "minimal" ? "极简" : inten === "standard" ? "标准" : "极致";
        const group = MODES.filter((m) => m.intensity === inten);
        return (
          <div key={inten} style={{ display: "inline-flex", alignItems: "center", gap: 2 }}>
            <span
              style={{
                fontSize: 11,
                color: "var(--text-tertiary)",
                margin: "0 4px",
                fontWeight: 500,
              }}
            >
              {gLabel}
            </span>
            <div
              style={{
                display: "inline-flex",
                padding: 4,
                background: "var(--surface-hover)",
                borderRadius: 10,
                gap: 2,
              }}
            >
              {group.map((m) => {
                const active = m.id === mode;
                return (
                  <button
                    key={m.id}
                    onClick={() => onModeChange(m.id)}
                    title={m.tagline}
                    style={{
                      padding: "6px 10px",
                      border: "none",
                      borderRadius: 7,
                      fontSize: 12,
                      fontWeight: active ? 700 : 500,
                      background: active ? "var(--surface-card)" : "transparent",
                      color: active ? "var(--text-primary)" : "var(--text-secondary)",
                      boxShadow: active ? "var(--shadow-sm)" : "none",
                      transition: "all 140ms ease",
                    }}
                  >
                    {m.short}
                  </button>
                );
              })}
            </div>
          </div>
        );
      })}

      <div style={{ width: 1, height: 24, background: "var(--surface-divider)" }} />

      {/* Engine / Sidecar status */}
      <div style={{ display: "inline-flex", alignItems: "center", gap: 8 }}>
        <PillBadge
          value={engineStatus.label}
          color={
            engineStatus.kind === "ok"
              ? "var(--status-success)"
              : engineStatus.kind === "warn"
              ? "var(--status-warning)"
              : "var(--status-error)"
          }
          tone="soft"
          dot
        />
        {sidecar === "running" ? (
          <PillBadge value="OpenSquilla: 运行中" color="var(--status-info)" tone="soft" dot />
        ) : sidecar === "starting" ? (
          <PillBadge value="OpenSquilla: 启动中" color="var(--status-warning)" tone="soft" dot />
        ) : sidecar === "crashed" ? (
          <PillBadge value="OpenSquilla: 异常" color="var(--status-error)" tone="soft" dot />
        ) : (
          <button
            onClick={onStartSidecar}
            style={{
              padding: "4px 10px",
              borderRadius: 8,
              fontSize: 11,
              fontWeight: 700,
              border: "1px dashed color-mix(in srgb, var(--accent-secondary) 45%, #fff)",
              background: "var(--accent-secondary-50)",
              color: "var(--accent-secondary-700)",
            }}
          >
            启动双引擎桥 ▸
          </button>
        )}
      </div>

      <div style={{ flex: 1 }} />

      {/* Search placeholder */}
      <div
        style={{
          width: 360,
          padding: "7px 12px",
          background: "var(--surface-hover)",
          border: "1px solid var(--surface-border)",
          borderRadius: 9,
          fontSize: 12,
          color: "var(--text-tertiary)",
          display: "flex",
          alignItems: "center",
          gap: 8,
        }}
      >
        <span>🔎</span>
        <span>搜索会话 / 任务 / 能力 / 经验…</span>
        <span
          style={{
            marginLeft: "auto",
            padding: "1px 6px",
            borderRadius: 4,
            fontSize: 10,
            border: "1px solid var(--surface-border)",
            background: "var(--surface-card)",
            color: "var(--text-secondary)",
          }}
        >
          ⌘ K
        </span>
      </div>

      <button
        onClick={() => setTheme(theme === "ink" ? "light" : "ink")}
        style={{
          padding: "6px 12px",
          borderRadius: 10,
          border: "1px solid var(--surface-border)",
          background: "var(--surface-card)",
          fontSize: 12,
          fontWeight: 600,
          color: theme === "ink" ? "var(--accent-primary)" : "var(--text-secondary)",
        }}
        title="切换中国水墨主题（剑侠江湖 · 泼墨风）"
      >
        {theme === "ink" ? "浅色" : "水墨"}
      </button>

      <button
        style={{
          width: 34,
          height: 34,
          borderRadius: 10,
          border: "1px solid var(--surface-border)",
          background: "var(--surface-card)",
          display: "grid",
          placeItems: "center",
          fontSize: 14,
        }}
        title="设置"
        aria-label="设置"
      >
        ⚙️
      </button>
    </header>
  );
};


// === B3: React.memo 包装 — 父级无关状态变更不再触发本组件重渲染 ===
export const TopBar = memo(TopBarImpl);
