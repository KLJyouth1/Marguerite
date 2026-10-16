import React, { useState, memo } from "react"
import { ChevronIcon, SessionDot } from "./primitives"
import { NAV_SECTIONS } from "../constants"
import { NavId, WorkTab, Workspace } from "../types"

const SidebarImpl: React.FC<{
  active: NavId;
  onNav: (id: NavId) => void;
  capTotal: number | null;
  onNewTask: () => void;
  workspaces: Workspace[];
  onToggleWorkspace: (id: string) => void;
  onToggleShowMore: (id: string) => void;
  onSelectSession: (wsId: string, sessionId: string) => void;
  activeSessionId?: string | null;
  workTab: WorkTab;
  onWorkTabChange: (tab: WorkTab) => void;
}> = ({
  active,
  onNav,
  onNewTask,
  workspaces,
  onToggleWorkspace,
  onToggleShowMore,
  onSelectSession,
  activeSessionId,
  workTab,
  onWorkTabChange,
}) => {
  const WORK_TABS: { id: WorkTab; label: string; icon: string }[] = [
    { id: "work", label: "Work", icon: "" },
    { id: "code", label: "Code", icon: "⟨/⟩" },
    { id: "design", label: "Design", icon: "" },
  ];

  const totalTasks = workspaces.reduce((sum, ws) => sum + (ws.totalCount ?? ws.sessions.length), 0);

  // A2 统一分组：核心默认展开（主屏），治理 / 实验室默认折叠，可点击切换。
  const [collapsedSections, setCollapsedSections] = useState<Record<string, boolean>>(() =>
    Object.fromEntries(NAV_SECTIONS.map((s) => [s.id, Boolean(s.defaultCollapsed)]))
  );
  const toggleSection = (id: string) =>
    setCollapsedSections((prev) => ({ ...prev, [id]: !prev[id] }));

  return (
    <aside
      style={{
        width: 268,
        background: "var(--surface-sidebar)",
        borderRight: "1px solid var(--surface-border)",
        display: "flex",
        flexDirection: "column",
        height: "100%",
      }}
    >
      {/* Top header + tabs */}
      <div
        style={{
          padding: "10px 12px 8px",
          borderBottom: "1px solid var(--surface-border)",
        }}
      >
        <div style={{ display: "flex", alignItems: "center", gap: 10, marginBottom: 10 }}>
          <div
            style={{
              width: 30,
              height: 30,
              borderRadius: 8,
              background:
                "linear-gradient(135deg, var(--accent-primary) 0%, var(--accent-secondary) 100%)",
              display: "grid",
              placeItems: "center",
              color: "#fff",
              fontWeight: 800,
              fontSize: 13,
              boxShadow: "0 4px 12px -6px rgba(12,102,228,0.4)",
            }}
          >
            M
          </div>
          <div style={{ flex: 1, fontSize: 13, fontWeight: 700 }}>Marguerite</div>
          <button
            style={{
              width: 28,
              height: 28,
              borderRadius: 6,
              border: "none",
              background: "transparent",
              color: "var(--text-tertiary)",
              fontSize: 14,
              display: "grid",
              placeItems: "center",
            }}
            title="搜索"
            aria-label="搜索"
          >
            🔍
          </button>
          <div
            style={{
              display: "inline-flex",
              fontSize: 12,
              color: "var(--text-tertiary)",
              border: "1px solid var(--surface-border)",
              borderRadius: 6,
              padding: "2px 6px",
              gap: 6,
              alignItems: "center",
            }}
          >
            <span>编辑</span>
            <span style={{ color: "var(--surface-border)" }}>|</span>
            <span>帮助</span>
          </div>
        </div>

        {/* Work / Code / Design tabs */}
        <div
          style={{
            display: "inline-flex",
            padding: 2,
            background: "var(--surface-hover)",
            borderRadius: 8,
            gap: 1,
            width: "100%",
          }}
        >
          {WORK_TABS.map((t) => {
            const isActive = workTab === t.id;
            return (
              <button
                key={t.id}
                onClick={() => onWorkTabChange(t.id)}
                style={{
                  flex: 1,
                  padding: "5px 8px",
                  border: "none",
                  borderRadius: 6,
                  fontSize: 11,
                  fontWeight: isActive ? 700 : 500,
                  background: isActive ? "var(--surface-card)" : "transparent",
                  color: isActive ? "var(--text-primary)" : "var(--text-tertiary)",
                  boxShadow: isActive ? "var(--shadow-sm)" : "none",
                  display: "inline-flex",
                  alignItems: "center",
                  justifyContent: "center",
                  gap: 4,
                  transition: "all 120ms ease",
                }}
              >
                {t.icon && <span style={{ fontSize: 10 }}>{t.icon}</span>}
                <span>{t.label}</span>
              </button>
            );
          })}
        </div>
      </div>

      {/* New Task CTA */}
      <div style={{ padding: "10px 12px 8px" }}>
        <button
          onClick={onNewTask}
          style={{
            width: "100%",
            padding: "8px 12px",
            border: "none",
            borderRadius: 8,
            color: "var(--accent-secondary-on)",
            background:
              "linear-gradient(180deg, var(--accent-secondary-300) 0%, var(--accent-secondary) 100%)",
            boxShadow:
              "0 3px 8px -4px var(--accent-secondary-700), inset 0 1px 0 rgba(255,255,255,0.18)",
            fontWeight: 700,
            fontSize: 12,
            display: "inline-flex",
            alignItems: "center",
            justifyContent: "center",
            gap: 6,
          }}
        >
          <span style={{ fontSize: 14, lineHeight: 1 }} aria-hidden="true">＋</span>
          新建任务
        </button>
      </div>

      {/* Nav sections (A2): 核心 / 治理 / 实验室，实验室默认折叠 */}
      <nav
        style={{
          padding: "2px 8px 8px",
          display: "flex",
          flexDirection: "column",
          gap: 1,
          borderBottom: "1px solid var(--surface-border)",
        }}
      >
        {NAV_SECTIONS.map((section) => {
          const isCollapsed = Boolean(collapsedSections[section.id]);
          return (
            <div key={section.id} data-nav-group={section.id}>
              <button
                onClick={() => toggleSection(section.id)}
                aria-expanded={!isCollapsed}
                style={{
                  width: "100%",
                  display: "flex",
                  alignItems: "center",
                  gap: 6,
                  padding: "6px 10px",
                  border: "none",
                  background: "transparent",
                  borderRadius: 6,
                  color: "var(--text-tertiary)",
                  fontSize: 11,
                  fontWeight: 700,
                  letterSpacing: 0.3,
                  textAlign: "left",
                }}
              >
                <span
                  aria-hidden="true"
                  style={{
                    display: "inline-block",
                    fontSize: 10,
                    transform: isCollapsed ? "rotate(-90deg)" : "none",
                    transition: "transform 120ms ease",
                  }}
                >
                  ▼
                </span>
                <span style={{ flex: 1 }}>{section.label}</span>
                <span
                  style={{
                    fontSize: 10,
                    color: "var(--text-tertiary)",
                    opacity: 0.7,
                  }}
                >
                  {section.items.length}
                </span>
              </button>
              {!isCollapsed &&
                section.items.map((item) => {
                  const isActive = active === item.id;
                  const badge = item.badge;
                  return (
                    <button
                      key={item.id}
                      onClick={() => onNav(item.id)}
                      style={{
                        display: "flex",
                        alignItems: "center",
                        gap: 10,
                        padding: "6px 10px 6px 26px",
                        border: "none",
                        borderRadius: 6,
                        background: isActive ? "var(--accent-primary-50)" : "transparent",
                        color: isActive ? "var(--accent-primary-700)" : "var(--text-secondary)",
                        fontWeight: isActive ? 600 : 500,
                        fontSize: 12,
                        textAlign: "left",
                        transition: "background 120ms ease, color 120ms ease",
                      }}
                    >
                      <span aria-hidden="true" style={{ fontSize: 13, width: 16, textAlign: "center" }}>{item.icon}</span>
                      <span style={{ flex: 1 }}>{item.label}</span>
                      {badge != null && (
                        <span
                          style={{
                            fontSize: 10,
                            fontWeight: 700,
                            padding: "1px 6px",
                            borderRadius: 999,
                            background: isActive
                              ? "color-mix(in srgb, var(--accent-primary) 18%, #fff)"
                              : "var(--surface-hover)",
                            color: isActive ? "var(--accent-primary-700)" : "var(--text-secondary)",
                          }}
                        >
                          {badge}
                        </span>
                      )}
                    </button>
                  );
                })}
            </div>
          );
        })}
      </nav>

      {/* Task List header */}
      <div
        style={{
          padding: "10px 14px 6px",
          display: "flex",
          alignItems: "center",
          gap: 6,
        }}
      >
        <div
          style={{
            fontSize: 11,
            fontWeight: 700,
            color: "var(--text-tertiary)",
            letterSpacing: 0.3,
          }}
        >
          任务列表
        </div>
        <div style={{ flex: 1 }} />
        <button
          style={{
            border: "none",
            background: "transparent",
            color: "var(--text-tertiary)",
            fontSize: 12,
            width: 20,
            height: 20,
            display: "grid",
            placeItems: "center",
            borderRadius: 4,
          }}
          title="按时间排序"
          aria-label="按时间排序"
        >
          ↻
        </button>
        <button
          style={{
            border: "none",
            background: "transparent",
            color: "var(--text-tertiary)",
            fontSize: 12,
            width: 20,
            height: 20,
            display: "grid",
            placeItems: "center",
            borderRadius: 4,
          }}
          title="筛选"
          aria-label="筛选"
        >
          ☰
        </button>
      </div>

      {/* Workspaces + Session list */}
      <div
        style={{
          flex: 1,
          overflowY: "auto",
          padding: "0 8px 10px",
          display: "flex",
          flexDirection: "column",
          gap: 2,
        }}
      >
        {workspaces.map((ws) => {
          const visibleSessions =
            ws.showMore || !ws.totalCount
              ? ws.sessions
              : ws.sessions.slice(0, 5);
          const hiddenCount = (ws.totalCount ?? ws.sessions.length) - visibleSessions.length;
          return (
            <div key={ws.id}>
              {/* Workspace header */}
              <button
                onClick={() => onToggleWorkspace(ws.id)}
                style={{
                  width: "100%",
                  display: "flex",
                  alignItems: "center",
                  gap: 6,
                  padding: "6px 8px 5px",
                  border: "none",
                  background: "transparent",
                  borderRadius: 6,
                  textAlign: "left",
                }}
              >
                <ChevronIcon open={!ws.collapsed} size={11} />
                <span
                  style={{
                    fontSize: 11,
                    color: "var(--text-tertiary)",
                    width: 14,
                    textAlign: "center",
                  }}
                >
                  {ws.icon}
                </span>
                <span
                  style={{
                    fontSize: 12,
                    fontWeight: 600,
                    color: "var(--text-secondary)",
                    flex: 1,
                    whiteSpace: "nowrap",
                    overflow: "hidden",
                    textOverflow: "ellipsis",
                  }}
                >
                  {ws.name}
                </span>
              </button>

              {/* Sessions */}
              {!ws.collapsed && (
                <div style={{ display: "flex", flexDirection: "column", gap: 1, marginLeft: 18 }}>
                  {visibleSessions.map((sess) => {
                    const isSel = activeSessionId === sess.id;
                    return (
                      <button
                        key={sess.id}
                        onClick={() => onSelectSession(ws.id, sess.id)}
                        style={{
                          display: "flex",
                          alignItems: "center",
                          gap: 8,
                          padding: "6px 10px 6px 8px",
                          border: "none",
                          borderRadius: 6,
                          background: isSel
                            ? "linear-gradient(90deg, color-mix(in srgb, var(--accent-primary) 10%, transparent) 0%, color-mix(in srgb, var(--accent-primary) 4%, transparent) 100%)"
                            : sess.isActive
                            ? "var(--surface-hover)"
                            : "transparent",
                          color: isSel ? "var(--text-primary)" : "var(--text-secondary)",
                          fontSize: 12,
                          textAlign: "left",
                          fontWeight: isSel || sess.isActive ? 500 : 400,
                          transition: "background 100ms ease",
                          borderLeft: isSel
                            ? `2px solid var(--accent-primary)`
                            : "2px solid transparent",
                        }}
                      >
                        {sess.hasUnread ? (
                          <span
                            style={{
                              width: 6,
                              height: 6,
                              borderRadius: 999,
                              background: "var(--accent-secondary)",
                              flexShrink: 0,
                              marginLeft: 1,
                            }}
                          />
                        ) : sess.statusDot ? (
                          <div style={{ width: 8, display: "flex", justifyContent: "center" }}>
                            <SessionDot kind={sess.statusDot} />
                          </div>
                        ) : (
                          <span style={{ width: 8, flexShrink: 0 }} />
                        )}
                        <span
                          style={{
                            flex: 1,
                            whiteSpace: "nowrap",
                            overflow: "hidden",
                            textOverflow: "ellipsis",
                            lineHeight: 1.35,
                          }}
                          title={sess.title}
                        >
                          {sess.title}
                        </span>
                        {(sess.timestamp === "刚刚" || sess.isActive) && (
                          <span
                            style={{
                              fontSize: 10,
                              color: sess.isActive ? "var(--status-success)" : "var(--text-tertiary)",
                              flexShrink: 0,
                              fontWeight: 600,
                            }}
                          >
                            {sess.timestamp}
                          </span>
                        )}
                      </button>
                    );
                  })}

                  {/* Show more */}
                  {hiddenCount > 0 && !ws.showMore && (
                    <button
                      onClick={() => onToggleShowMore(ws.id)}
                      style={{
                        display: "flex",
                        alignItems: "center",
                        gap: 6,
                        padding: "5px 10px 5px 8px",
                        border: "none",
                        background: "transparent",
                        borderRadius: 6,
                        color: "var(--text-tertiary)",
                        fontSize: 11,
                        textAlign: "left",
                        fontWeight: 500,
                        marginLeft: 8,
                      }}
                    >
                      <span style={{ fontSize: 9 }}>››</span>
                      查看更多 ({hiddenCount})
                    </button>
                  )}
                  {ws.showMore && hiddenCount <= 0 && ws.totalCount && ws.totalCount > ws.sessions.length && (
                    <button
                      onClick={() => onToggleShowMore(ws.id)}
                      style={{
                        display: "flex",
                        alignItems: "center",
                        gap: 6,
                        padding: "5px 10px 5px 8px",
                        border: "none",
                        background: "transparent",
                        borderRadius: 6,
                        color: "var(--text-tertiary)",
                        fontSize: 11,
                        textAlign: "left",
                        fontWeight: 500,
                        marginLeft: 8,
                      }}
                    >
                      <span style={{ fontSize: 9 }}>‹‹</span>
                      收起
                    </button>
                  )}
                </div>
              )}
            </div>
          );
        })}
        {workspaces.length === 0 && (
          <div
            style={{
              padding: "22px 14px",
              textAlign: "center",
              color: "var(--text-tertiary)",
              fontSize: 12,
              lineHeight: 1.8,
            }}
          >
            <div style={{ fontSize: 20, marginBottom: 4, opacity: 0.8 }}>▢</div>
            暂无会话
            <br />
            点击上方「新建任务」开始你的第一个任务
          </div>
        )}
      </div>

      {/* Footer: User profile */}
      <div
        style={{
          padding: "10px 12px",
          borderTop: "1px solid var(--surface-border)",
          background: "var(--surface-card)",
          display: "flex",
          alignItems: "center",
          gap: 10,
        }}
      >
        <div
          style={{
            width: 28,
            height: 28,
            borderRadius: 999,
            background:
              "radial-gradient(circle at 30% 30%, #22C55E 0%, #10B981 50%, #059669 100%)",
            display: "grid",
            placeItems: "center",
            color: "#fff",
            fontWeight: 700,
            fontSize: 12,
            boxShadow: "0 2px 8px -2px rgba(16,185,129,0.5)",
          }}
        >
          F
        </div>
        <div style={{ flex: 1, minWidth: 0 }}>
          <div
            style={{
              fontSize: 12,
              fontWeight: 600,
              color: "var(--text-primary)",
              whiteSpace: "nowrap",
              overflow: "hidden",
              textOverflow: "ellipsis",
            }}
          >
            Felice
          </div>
          <div style={{ fontSize: 10, color: "var(--text-tertiary)", marginTop: 1 }}>
            {totalTasks} 个任务
          </div>
        </div>
        <button
          style={{
            width: 24,
            height: 24,
            border: "none",
            background: "transparent",
            borderRadius: 6,
            color: "var(--text-tertiary)",
            fontSize: 12,
            display: "grid",
            placeItems: "center",
          }}
          title="通知"
          aria-label="通知"
        >
          🔔
        </button>
        <button
          style={{
            width: 24,
            height: 24,
            border: "none",
            background: "transparent",
            borderRadius: 6,
            color: "var(--text-tertiary)",
            fontSize: 12,
            display: "grid",
            placeItems: "center",
          }}
          title="设置"
          aria-label="设置"
        >
          ⊞
        </button>
      </div>
    </aside>
  );
};


// === B3: React.memo 包装 — 父级无关状态变更不再触发本组件重渲染 ===
export const Sidebar = memo(SidebarImpl);
