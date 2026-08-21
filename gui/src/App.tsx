import React, { useEffect, useMemo, useRef, useState } from "react";
import "./theme.css";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

type AgentStatus = "idle" | "running" | "paused" | "error" | "done";

interface SubAgent {
  id: string;
  name: string;
  role: string;
  color: string; // token css
  status: AgentStatus;
  progressPct: number;
  tools: string[];
  lastStep?: string;
}

type ModeKind = "work" | "research" | "automation" | "safe" | "bloom";

interface ModeOption {
  id: ModeKind;
  label: string;
  icon: string;
  tagline: string;
  accent: string;
}

const MODES: ModeOption[] = [
  { id: "work", label: "工作模式", icon: "⚡", tagline: "对话 + 写代码", accent: "var(--accent-primary)" },
  { id: "research", label: "研究模式", icon: "🔍", tagline: "深挖 + 长链路", accent: "#6366F1" },
  { id: "automation", label: "自动化", icon: "🤖", tagline: "Agent 自治 · 可回滚", accent: "var(--accent-secondary)" },
  { id: "safe", label: "护栏模式", icon: "🛡️", tagline: "高风险 · 每步确认", accent: "var(--status-success)" },
  { id: "bloom", label: "Bloom 模式", icon: "❀", tagline: "MAX 四阶段推理 · YOLO 全自动 · 快照回滚", accent: "var(--accent-secondary)" },
];

interface ChatMessage {
  id: string;
  role: "user" | "assistant" | "tool" | "system";
  content: string;
  agent?: string;
  thinking?: boolean;
}

const INITIAL_AGENTS: SubAgent[] = [
  {
    id: "orchestrator",
    name: "编排器",
    role: "Orchestrator · 主代理",
    color: "var(--accent-primary)",
    status: "idle",
    progressPct: 0,
    tools: ["plan_task", "run_turn", "recall"],
  },
  {
    id: "coder",
    name: "代码代理",
    role: "Coder Agent · 工程实现",
    color: "#6366F1",
    status: "idle",
    progressPct: 0,
    tools: ["read_file", "edit", "grep"],
  },
  {
    id: "sandbox",
    name: "终端沙箱",
    role: "Sandbox · Shell / 进程",
    color: "var(--status-success)",
    status: "idle",
    progressPct: 0,
    tools: ["pty_exec", "fs_ops", "run_cmd"],
  },
  {
    id: "browser",
    name: "浏览器代理",
    role: "Browser Agent · 网页交互",
    color: "var(--accent-secondary)",
    status: "idle",
    progressPct: 0,
    tools: ["navigate", "click", "snapshot"],
  },
  {
    id: "researcher",
    name: "研究代理",
    role: "Researcher · 知识检索",
    color: "#A855F7",
    status: "idle",
    progressPct: 0,
    tools: ["web_search", "fetch", "kb_query"],
  },
  {
    id: "guard",
    name: "护栏校验",
    role: "Guard · 安全 / 语法 / 修复",
    color: "var(--status-warning)",
    status: "idle",
    progressPct: 0,
    tools: ["guard_check", "repair_code"],
  },
];

const NAV_ITEMS = [
  { id: "new_task", label: "对话工作台", icon: "✦", badge: null as number | null },
  { id: "doctor", label: "系统诊断", icon: "🩺", badge: null },
  { id: "skills", label: "技能管理", icon: "⚡", badge: null },
  { id: "mcp", label: "MCP 服务器", icon: "🔌", badge: null },
  { id: "sessions", label: "会话管理", icon: "💬", badge: null },
  { id: "sandbox", label: "沙箱执行", icon: "▶", badge: null },
  { id: "computer_use", label: "电脑操控", icon: "🖥", badge: null },
  { id: "kb", label: "知识库", icon: "📚", badge: null },
  { id: "fleet", label: "模型舰队", icon: "🚀", badge: null },
  { id: "evolve", label: "自进化", icon: "🧬", badge: null },
  { id: "plugins", label: "插件市场", icon: "⌬", badge: null },
  { id: "capabilities", label: "能力总览", icon: "▦", badge: null },
  { id: "automation", label: "自动化", icon: "◐", badge: null },
  { id: "office_assistant", label: "办公助理", icon: "☺", badge: null },
  { id: "templates", label: "模板库", icon: "▦", badge: null },
  { id: "workspace", label: "工作区", icon: "▤", badge: null },
  { id: "routing", label: "模型路由", icon: "⇄", badge: null },
  { id: "modlens", label: "视觉取证", icon: "◉", badge: null },
  { id: "poker", label: "德州扑克", icon: "◆", badge: null },
  { id: "graph", label: "关系图谱", icon: "⋈", badge: null },
  { id: "usage", label: "消费统计", icon: "▥", badge: null },
  { id: "pet", label: "鲸鱼娘", icon: "◍", badge: null },
  { id: "files", label: "文件", icon: "▧", badge: null },
];

type NavId = (typeof NAV_ITEMS)[number]["id"];

type WorkTab = "work" | "code" | "design";

interface HistorySession {
  id: string;
  title: string;
  timestamp: string;
  isActive?: boolean;
  hasUnread?: boolean;
  statusDot?: "success" | "warning" | "error" | null;
}

interface Workspace {
  id: string;
  name: string;
  icon?: string;
  sessions: HistorySession[];
  collapsed?: boolean;
  showMore?: boolean;
  totalCount?: number;
}

const INITIAL_WORKSPACES: Workspace[] = [
  {
    id: "marguerite",
    name: "Marguerite",
    icon: "▢",
    sessions: [
      { id: "s1", title: "CLI工作台架构设计与开发", timestamp: "刚刚", isActive: false, hasUnread: false },
      { id: "s2", title: "分析 ConnectionResetError 日志", timestamp: "2小时前", isActive: false, hasUnread: false },
    ],
    collapsed: false,
  },
  {
    id: "default",
    name: "默认",
    icon: "▢",
    sessions: [
      { id: "s3", title: "后端开发规范文档", timestamp: "昨天", isActive: false, hasUnread: false },
    ],
    collapsed: false,
  },
  {
    id: "datarecoverypro",
    name: "DataRecoveryPro",
    icon: "▢",
    sessions: [
      { id: "s4", title: "测试Pro版V1.4和Pro+版2.0", timestamp: "3天前", isActive: false, hasUnread: true },
    ],
    collapsed: false,
  },
  {
    id: "ddos",
    name: "DDOS",
    icon: "▢",
    sessions: [
      { id: "s5", title: "项目与UI/UX全面升级", timestamp: "5天前", isActive: false, hasUnread: false },
    ],
    collapsed: false,
  },
  {
    id: "gui_agent",
    name: "GUI_Agent",
    icon: "▢",
    sessions: [
      { id: "s6", title: "CCD Recovery Pro Desktop Tool Dev 设计与实现", timestamp: "刚刚", isActive: true, hasUnread: false, statusDot: "success" },
      { id: "s7", title: "后端状态指标与模块自检报告", timestamp: "12天前", isActive: false, hasUnread: false },
      { id: "s8", title: "项目全面自检与优化报告", timestamp: "12天前", isActive: false, hasUnread: false },
      { id: "s9", title: "核对WorkBuddy项目文件", timestamp: "13天前", isActive: false, hasUnread: false, statusDot: "warning" },
      { id: "s10", title: "智能体平台功能增强与架构设计", timestamp: "14天前", isActive: false, hasUnread: false },
      { id: "s11", title: "Max模式功能与痛点改进", timestamp: "15天前", isActive: false, hasUnread: false },
      { id: "s12", title: "Tauri Dev Command Error 排查", timestamp: "18天前", isActive: false, hasUnread: false, statusDot: "error" },
    ],
    collapsed: false,
    showMore: false,
    totalCount: 48,
  },
];

const useTauri = () => {
  // Detect Tauri environment; graceful fallback for browser dev server.
  const [available, setAvailable] = useState<boolean | null>(null);
  useEffect(() => {
    try {
      const w = window as unknown as { __TAURI_INTERNALS__?: unknown };
      setAvailable(!!w.__TAURI_INTERNALS__);
    } catch {
      setAvailable(false);
    }
  }, []);
  return available === true;
};

const PillBadge: React.FC<{ value: string; color?: string; dot?: boolean; tone?: "solid" | "soft" }> = ({
  value,
  color = "var(--status-success)",
  dot,
  tone = "soft",
}) => {
  const style: React.CSSProperties =
    tone === "solid"
      ? { background: color, color: "#fff" }
      : {
          background: `color-mix(in srgb, ${color} 12%, #fff)`,
          color,
          border: `1px solid color-mix(in srgb, ${color} 25%, #fff)`,
        };
  return (
    <span
      style={{
        display: "inline-flex",
        alignItems: "center",
        gap: 6,
        padding: "2px 10px",
        borderRadius: 999,
        fontSize: 11,
        fontWeight: 600,
        lineHeight: "18px",
        ...style,
      }}
    >
      {dot && (
        <span
          style={{
            width: 6,
            height: 6,
            borderRadius: 999,
            background: color,
            boxShadow: `0 0 0 2px color-mix(in srgb, ${color} 30%, transparent)`,
          }}
        />
      )}
      {value}
    </span>
  );
};

const StatusDot: React.FC<{ status: AgentStatus; size?: number }> = ({ status, size = 8 }) => {
  const colorMap: Record<AgentStatus, string> = {
    idle: "var(--text-tertiary)",
    running: "var(--status-success)",
    paused: "var(--status-warning)",
    error: "var(--status-error)",
    done: "var(--status-info)",
  };
  const color = colorMap[status];
  const pulse = status === "running";
  return (
    <span
      style={{
        position: "relative",
        width: size,
        height: size,
        borderRadius: 999,
        background: color,
        display: "inline-block",
      }}
    >
      {pulse && (
        <span
          key={status + Math.random()}
          style={{
            position: "absolute",
            inset: -3,
            borderRadius: 999,
            background: color,
            opacity: 0.35,
            animation: "mg-pulse 1.6s ease-out infinite",
          }}
        />
      )}
    </span>
  );
};

const ProgressBar: React.FC<{ value: number; color: string }> = ({ value, color }) => (
  <div
    style={{
      height: 4,
      background: "var(--surface-border)",
      borderRadius: 999,
      overflow: "hidden",
    }}
  >
    <div
      style={{
        height: "100%",
        width: `${Math.max(0, Math.min(100, value))}%`,
        background: color,
        borderRadius: 999,
        transition: "width 300ms ease",
      }}
    />
  </div>
);

const ChevronIcon: React.FC<{ open: boolean; size?: number }> = ({ open, size = 12 }) => (
  <span
    style={{
      display: "inline-block",
      transition: "transform 150ms ease",
      transform: open ? "rotate(90deg)" : "rotate(0deg)",
      fontSize: size,
      lineHeight: 1,
    }}
  >
    ›
  </span>
);

const SessionDot: React.FC<{ kind?: "success" | "warning" | "error" | null; size?: number }> = ({
  kind,
  size = 6,
}) => {
  if (!kind) return null;
  const colorMap = {
    success: "var(--status-success)",
    warning: "var(--status-warning)",
    error: "var(--status-error)",
  };
  return (
    <span
      style={{
        width: size,
        height: size,
        borderRadius: 999,
        background: colorMap[kind],
        flexShrink: 0,
      }}
    />
  );
};

const Sidebar: React.FC<{
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
  capTotal,
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
          <span style={{ fontSize: 14, lineHeight: 1 }}>＋</span>
          新建任务
        </button>
      </div>

      {/* Nav items */}
      <nav
        style={{
          padding: "2px 8px 8px",
          display: "flex",
          flexDirection: "column",
          gap: 1,
          borderBottom: "1px solid var(--surface-border)",
        }}
      >
        {NAV_ITEMS.map((item) => {
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
                padding: "7px 10px",
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
              <span style={{ fontSize: 13, width: 16, textAlign: "center" }}>{item.icon}</span>
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
        >
          ⊞
        </button>
      </div>
    </aside>
  );
};

const TopBar: React.FC<{
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
      {/* Mode switcher */}
      <div
        style={{
          display: "inline-flex",
          padding: 4,
          background: "var(--surface-hover)",
          borderRadius: 10,
          gap: 2,
        }}
      >
        {MODES.map((m) => {
          const active = m.id === mode;
          return (
            <button
              key={m.id}
              onClick={() => onModeChange(m.id)}
              title={m.tagline}
              style={{
                padding: "6px 12px",
                border: "none",
                borderRadius: 7,
                fontSize: 12,
                fontWeight: active ? 700 : 500,
                background: active ? "var(--surface-card)" : "transparent",
                color: active ? "var(--text-primary)" : "var(--text-secondary)",
                boxShadow: active ? "var(--shadow-sm)" : "none",
                display: "inline-flex",
                alignItems: "center",
                gap: 6,
                transition: "all 140ms ease",
              }}
            >
              <span>{m.icon}</span>
              <span>{m.label}</span>
            </button>
          );
        })}
      </div>

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
      >
        ⚙️
      </button>
    </header>
  );
};

const AgentCard: React.FC<{ agent: SubAgent }> = ({ agent }) => {
  const statusLabel: Record<AgentStatus, string> = {
    idle: "待命中",
    running: "运行中",
    paused: "已暂停",
    error: "异常",
    done: "已完成",
  };
  return (
    <div
      style={{
        padding: 12,
        borderRadius: 12,
        background: "var(--surface-card)",
        border: `1px solid ${agent.status === "running"
          ? "color-mix(in srgb, " + agent.color + " 40%, var(--surface-border))"
          : "var(--surface-border)"}`,
        boxShadow: agent.status === "running" ? "var(--shadow-sm)" : "none",
        transition: "border-color 160ms ease",
      }}
    >
      <div style={{ display: "flex", alignItems: "center", gap: 10 }}>
        <div
          style={{
            width: 28,
            height: 28,
            borderRadius: 8,
            background: `color-mix(in srgb, ${agent.color} 15%, #fff)`,
            color: agent.color,
            fontWeight: 800,
            fontSize: 11,
            display: "grid",
            placeItems: "center",
            border: `1px solid color-mix(in srgb, ${agent.color} 30%, #fff)`,
          }}
        >
          {agent.name.slice(0, 1)}
        </div>
        <div style={{ flex: 1, minWidth: 0 }}>
          <div style={{ display: "flex", alignItems: "center", gap: 6 }}>
            <span style={{ fontSize: 12, fontWeight: 700 }}>{agent.name}</span>
            <StatusDot status={agent.status} size={6} />
          </div>
          <div style={{ fontSize: 10.5, color: "var(--text-tertiary)", marginTop: 1 }}>
            {agent.role}
          </div>
        </div>
        <div
          style={{
            fontSize: 10.5,
            fontWeight: 700,
            padding: "2px 8px",
            borderRadius: 999,
            color: agent.color,
            background: `color-mix(in srgb, ${agent.color} 12%, #fff)`,
          }}
        >
          {statusLabel[agent.status]}
        </div>
      </div>

      {agent.status === "running" && (
        <div style={{ marginTop: 10 }}>
          <ProgressBar value={agent.progressPct} color={agent.color} />
          {agent.lastStep && (
            <div
              style={{
                marginTop: 6,
                fontSize: 11,
                color: "var(--text-secondary)",
                whiteSpace: "nowrap",
                overflow: "hidden",
                textOverflow: "ellipsis",
              }}
              title={agent.lastStep}
            >
              {agent.lastStep}
            </div>
          )}
        </div>
      )}

      <div style={{ marginTop: 10, display: "flex", flexWrap: "wrap", gap: 4 }}>
        {agent.tools.map((t) => (
          <span
            key={t}
            style={{
              fontSize: 10,
              padding: "2px 7px",
              borderRadius: 5,
              background: "var(--surface-hover)",
              color: "var(--text-secondary)",
              border: "1px solid var(--surface-border)",
              fontFamily: "ui-monospace, SFMono-Regular, Menlo, monospace",
            }}
          >
            {t}
          </span>
        ))}
      </div>
    </div>
  );
};

const RightRail: React.FC<{ agents: SubAgent[] }> = ({ agents }) => {
  const running = agents.filter((a) => a.status === "running").length;
  return (
    <aside
      style={{
        width: 304,
        background: "var(--surface-bg)",
        borderLeft: "1px solid var(--surface-border)",
        display: "flex",
        flexDirection: "column",
        height: "100%",
      }}
    >
      <div
        style={{
          padding: "16px 16px 12px",
          borderBottom: "1px solid var(--surface-border)",
          background: "var(--surface-card)",
        }}
      >
        <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
          <div style={{ fontSize: 13, fontWeight: 700 }}>子代理 · 并行执行</div>
          <PillBadge
            value={running > 0 ? `${running} 运行中` : "待命"}
            color={running > 0 ? "var(--status-success)" : "var(--text-tertiary)"}
            tone="soft"
            dot
          />
        </div>
        <div style={{ fontSize: 11, color: "var(--text-tertiary)", marginTop: 4 }}>
          Orchestrator 统一调度 · 实时进度与工具调用
        </div>
      </div>

      <div style={{ padding: 12, display: "flex", flexDirection: "column", gap: 8, overflowY: "auto" }}>
        {agents.map((a) => (
          <AgentCard key={a.id} agent={a} />
        ))}
      </div>
    </aside>
  );
};

const Bubble: React.FC<{ msg: ChatMessage }> = ({ msg }) => {
  const isUser = msg.role === "user";
  const isSystem = msg.role === "system";
  if (isSystem) {
    return (
      <div
        style={{
          display: "flex",
          justifyContent: "center",
          padding: "4px 0",
        }}
      >
        <PillBadge value={msg.content} color="var(--text-tertiary)" tone="soft" />
      </div>
    );
  }
  return (
    <div
      style={{
        display: "flex",
        justifyContent: isUser ? "flex-end" : "flex-start",
        padding: "4px 0",
      }}
    >
      <div
        style={{
          maxWidth: "74%",
          padding: "10px 12px",
          borderRadius: 12,
          background: isUser
            ? "linear-gradient(180deg, var(--accent-primary-300) 0%, var(--accent-primary) 100%)"
            : "var(--surface-card)",
          color: isUser ? "#fff" : "var(--text-primary)",
          border: isUser ? "none" : "1px solid var(--surface-border)",
          boxShadow: isUser
            ? "0 6px 14px -10px var(--accent-primary-700)"
            : "var(--shadow-sm)",
          fontSize: 13,
          lineHeight: 1.55,
          whiteSpace: "pre-wrap",
          wordBreak: "break-word",
        }}
      >
        {msg.agent && (
          <div
            style={{
              fontSize: 10.5,
              fontWeight: 700,
              marginBottom: 4,
              color: isUser ? "rgba(255,255,255,0.85)" : "var(--accent-primary-700)",
            }}
          >
            @{msg.agent}
          </div>
        )}
        {msg.content}
        {msg.thinking && (
          <div
            style={{
              marginTop: 8,
              padding: "6px 8px",
              fontSize: 11,
              borderRadius: 8,
              background: isUser ? "rgba(255,255,255,0.18)" : "var(--surface-hover)",
              color: isUser ? "rgba(255,255,255,0.9)" : "var(--text-secondary)",
              borderLeft: `3px solid ${isUser ? "#fff" : "var(--accent-secondary)"}`,
            }}
          >
            <span style={{ display: "inline-block", animation: "mg-thinking 1.2s infinite" }}>
              ● ● ●
            </span>
            {"  "}思考中…
          </div>
        )}
      </div>
    </div>
  );
};

const ChatCenter: React.FC<{
  messages: ChatMessage[];
  mode: ModeKind;
  running: boolean;
  onSend: (text: string) => void;
  agents: SubAgent[];
}> = ({ messages, mode, running, onSend, agents }) => {
  const [input, setInput] = useState("");
  const scrollRef = useRef<HTMLDivElement | null>(null);
  const modeAccent = MODES.find((m) => m.id === mode)!.accent;

  useEffect(() => {
    scrollRef.current?.scrollTo({ top: scrollRef.current.scrollHeight, behavior: "smooth" });
  }, [messages]);

  const send = () => {
    const text = input.trim();
    if (!text) return;
    setInput("");
    onSend(text);
  };

  const runningTools = useMemo(() => {
    return agents
      .filter((a) => a.status === "running")
      .flatMap((a) => a.tools.slice(0, 2))
      .slice(0, 4);
  }, [agents]);

  return (
    <section style={{ flex: 1, display: "flex", flexDirection: "column", minWidth: 0 }}>
      {/* Center Header: Session */}
      <div
        style={{
          padding: "14px 24px 12px",
          borderBottom: "1px solid var(--surface-border)",
          background: "var(--surface-card)",
          display: "flex",
          alignItems: "center",
          gap: 12,
        }}
      >
        <div
          style={{
            padding: "6px 10px",
            borderRadius: 10,
            fontSize: 11,
            fontWeight: 700,
            background: `color-mix(in srgb, ${modeAccent} 15%, #fff)`,
            color: modeAccent,
            border: `1px solid color-mix(in srgb, ${modeAccent} 25%, #fff)`,
            display: "inline-flex",
            alignItems: "center",
            gap: 6,
          }}
        >
          <span>{MODES.find((m) => m.id === mode)!.icon}</span>
          {MODES.find((m) => m.id === mode)!.label}
        </div>
        <div style={{ fontSize: 13, fontWeight: 700 }}>Session · 任务工作台</div>
        <div style={{ flex: 1 }} />
        <PillBadge value={running ? "正在处理…" : "就绪"} color={running ? "var(--status-success)" : "var(--text-tertiary)"} tone="soft" dot />
      </div>

      {/* Conversation */}
      <div
        ref={scrollRef}
        style={{
          flex: 1,
          overflowY: "auto",
          padding: "20px 32px 12px",
          display: "flex",
          flexDirection: "column",
          gap: 2,
        }}
      >
        {messages.map((m) => (
          <Bubble key={m.id} msg={m} />
        ))}
      </div>

      {/* Running tools live feed */}
      {runningTools.length > 0 && (
        <div
          style={{
            margin: "0 32px 6px",
            padding: "8px 10px",
            background: "var(--surface-card)",
            border: "1px solid var(--surface-border)",
            borderRadius: 10,
            display: "flex",
            alignItems: "center",
            gap: 8,
            flexWrap: "wrap",
          }}
        >
          <span style={{ fontSize: 11, fontWeight: 700, color: "var(--text-secondary)" }}>
            工具流
          </span>
          {runningTools.map((t, i) => (
            <span
              key={t + i}
              style={{
                fontSize: 10.5,
                fontFamily: "ui-monospace, SFMono-Regular, Menlo, monospace",
                padding: "2px 8px",
                borderRadius: 6,
                background: "var(--accent-primary-50)",
                color: "var(--accent-primary-700)",
                border: "1px solid color-mix(in srgb, var(--accent-primary) 20%, #fff)",
              }}
            >
              ▸ {t}
            </span>
          ))}
        </div>
      )}

      {/* Composer */}
      <div
        style={{
          padding: "10px 24px 18px",
          background: "linear-gradient(180deg, transparent 0%, var(--surface-bg) 40%)",
        }}
      >
        <div
          style={{
            padding: 6,
            background: "var(--surface-card)",
            borderRadius: 14,
            border: "1px solid var(--surface-border)",
            boxShadow: "var(--shadow-md)",
          }}
        >
          <textarea
            value={input}
            onChange={(e) => setInput(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter" && !e.shiftKey) {
                e.preventDefault();
                send();
              }
            }}
            rows={2}
            placeholder="描述你的任务 —— 支持多代理编排、终端沙箱、浏览器操控、知识检索… (Enter 发送 / Shift+Enter 换行)"
            style={{
              width: "100%",
              resize: "none",
              border: "none",
              outline: "none",
              padding: "8px 10px",
              fontFamily: "inherit",
              fontSize: 13,
              lineHeight: 1.6,
              color: "var(--text-primary)",
              background: "transparent",
            }}
          />
          <div
            style={{
              display: "flex",
              alignItems: "center",
              gap: 8,
              padding: "2px 6px 2px 10px",
            }}
          >
            <div style={{ display: "inline-flex", gap: 4 }}>
              {["📎", "🖼️", "📋", "🎯", "🧪"].map((ic, i) => (
                <button
                  key={i}
                  style={{
                    border: "1px solid var(--surface-border)",
                    background: "var(--surface-hover)",
                    width: 28,
                    height: 28,
                    borderRadius: 8,
                    fontSize: 13,
                  }}
                >
                  {ic}
                </button>
              ))}
            </div>
            <div style={{ flex: 1 }} />
            <span style={{ fontSize: 10.5, color: "var(--text-tertiary)" }}>
              双引擎桥 · Rust + Python
            </span>
            <button
              onClick={send}
              disabled={!input.trim() || running}
              style={{
                border: "none",
                padding: "8px 16px",
                borderRadius: 10,
                fontSize: 12,
                fontWeight: 700,
                color: "var(--accent-primary-on)",
                background:
                  "linear-gradient(180deg, var(--accent-primary-300) 0%, var(--accent-primary) 100%)",
                boxShadow: "0 6px 14px -8px var(--accent-primary-700)",
                opacity: input.trim() && !running ? 1 : 0.55,
                cursor: input.trim() && !running ? "pointer" : "not-allowed",
                display: "inline-flex",
                alignItems: "center",
                gap: 6,
              }}
            >
              发送
              <span>↵</span>
            </button>
          </div>
        </div>
      </div>
    </section>
  );
};

// ───────────── 工具面板：对齐 TUI/CLI 全部子命令 ─────────────

const panelBox: React.CSSProperties = {
  flex: 1,
  overflowY: "auto",
  padding: "20px 24px",
  background: "var(--surface-bg, #0f1117)",
  color: "var(--text-primary, #e4e6eb)",
};

const panelTitle: React.CSSProperties = {
  fontSize: 18,
  fontWeight: 700,
  marginBottom: 4,
  // 水墨主题（data-theme=ink）下自动切楷体（--font-display 仅在 ink 主题定义）
  fontFamily: "var(--font-display, inherit)",
  letterSpacing: "var(--title-letter-spacing, 0px)",
};

const panelSubtitle: React.CSSProperties = {
  fontSize: 12,
  color: "var(--text-secondary, #8b8fa3)",
  marginBottom: 16,
};

const cardStyle: React.CSSProperties = {
  background: "var(--surface-card, #181b24)",
  border: "1px solid var(--surface-border, #252836)",
  borderRadius: 8,
  padding: "12px 14px",
  marginBottom: 8,
};

const btnStyle: React.CSSProperties = {
  padding: "6px 14px",
  borderRadius: 6,
  border: "1px solid var(--accent-primary, #6366f1)",
  background: "var(--accent-primary, #6366f1)",
  color: "#fff",
  fontSize: 12,
  fontWeight: 600,
  cursor: "pointer",
};

const btnGhostStyle: React.CSSProperties = {
  ...btnStyle,
  background: "transparent",
  color: "var(--accent-primary, #6366f1)",
};

const inputStyle: React.CSSProperties = {
  flex: 1,
  padding: "7px 10px",
  borderRadius: 6,
  border: "1px solid var(--surface-border, #252836)",
  background: "var(--surface-input, #12141c)",
  color: "var(--text-primary, #e4e6eb)",
  fontSize: 12,
  outline: "none",
};

function Badge({ ok, label }: { ok: boolean; label?: string }) {
  return (
    <span
      style={{
        fontSize: 10,
        fontWeight: 700,
        padding: "2px 8px",
        borderRadius: 999,
        background: ok ? "rgba(34,197,94,0.15)" : "rgba(239,68,68,0.15)",
        color: ok ? "#22c55e" : "#ef4444",
      }}
    >
      {label ?? (ok ? "正常" : "异常")}
    </span>
  );
}

function useTauriInvoke<T>(cmd: string, args?: Record<string, unknown>) {
  const [data, setData] = useState<T | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const tauri = useTauri();

  const run = useCallback(
    async (extraArgs?: Record<string, unknown>) => {
      if (!tauri) {
        setError("非 Tauri 环境（浏览器预览）");
        return;
      }
      setLoading(true);
      setError(null);
      try {
        const result = await invoke<T>(cmd, { ...args, ...extraArgs });
        setData(result);
      } catch (e) {
        setError(e instanceof Error ? e.message : String(e));
      } finally {
        setLoading(false);
      }
    },
    [tauri, cmd, JSON.stringify(args)]
  );

  useEffect(() => {
    run();
  }, [run]);

  return { data, error, loading, run, setData };
}

function DoctorPanel() {
  const { data, error, loading, run } = useTauriInvoke<{
    os: string;
    arch: string;
    config_dir: string;
    model_name: string;
    model_provider: string;
    api_key_configured: boolean;
    mcp_servers: number;
    skills_dir: string;
    checks: { name: string; ok: boolean; detail: string; hint: string }[];
    total: number;
    passed: number;
    failed: number;
    status: string;
  }>("doctor_check");

  return (
    <div style={panelBox}>
      <div style={panelTitle}>系统诊断</div>
      <div style={panelSubtitle}>对齐 CLI: marguerite doctor — 环境自检与状态报告</div>
      <button style={btnGhostStyle} onClick={() => run()}>
        {loading ? "检查中…" : "重新检查"}
      </button>
      {error && <div style={{ color: "#ef4444", marginTop: 12, fontSize: 12 }}>{error}</div>}
      {data && (
        <div style={{ marginTop: 16 }}>
          <div style={{ ...cardStyle, display: "flex", gap: 16, flexWrap: "wrap" }}>
            <span>OS: <b>{data.os}/{data.arch}</b></span>
            <span>模型: <b>{data.model_name}</b></span>
            <span>API Key: <Badge ok={data.api_key_configured} /></span>
            <span>MCP: <b>{data.mcp_servers}</b></span>
            <span>状态: <Badge ok={data.status === "ok"} label={data.status} /></span>
            <span>通过: <b>{data.passed}/{data.total}</b></span>
          </div>
          {data.checks.map((c) => (
            <div key={c.name} style={{ ...cardStyle, display: "flex", alignItems: "center", gap: 10 }}>
              <Badge ok={c.ok} />
              <div style={{ flex: 1 }}>
                <div style={{ fontWeight: 600, fontSize: 13 }}>{c.name}</div>
                <div style={{ fontSize: 11, color: "var(--text-secondary)" }}>{c.detail}</div>
                {c.hint && <div style={{ fontSize: 11, color: "#f59e0b" }}>提示: {c.hint}</div>}
              </div>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}

function SkillsPanel() {
  const { data, error, loading, run } = useTauriInvoke<
    { name: string; description: string; version: string; author: string }[]
  >("skills_list");
  const [installUrl, setInstallUrl] = useState("");
  const [installing, setInstalling] = useState(false);

  const handleInstall = async () => {
    if (!installUrl.trim()) return;
    setInstalling(true);
    try {
      await invoke("skills_install", { url: installUrl.trim() });
      setInstallUrl("");
      run();
    } catch (e) {
      alert("安装失败: " + (e instanceof Error ? e.message : String(e)));
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
      {error && <div style={{ color: "#ef4444", fontSize: 12 }}>{error}</div>}
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

function McpPanel() {
  const { data, error, loading, run } = useTauriInvoke<
    { name: string; command: string; args: string[] }[]
  >("mcp_list");

  return (
    <div style={panelBox}>
      <div style={panelTitle}>MCP 服务器</div>
      <div style={panelSubtitle}>对齐 CLI: marguerite mcp list — 已配置的 MCP 工具服务器</div>
      <button style={btnGhostStyle} onClick={() => run()}>刷新</button>
      {error && <div style={{ color: "#ef4444", marginTop: 12, fontSize: 12 }}>{error}</div>}
      {loading && <div style={{ fontSize: 12, color: "var(--text-secondary)" }}>加载中…</div>}
      {data?.length === 0 && <div style={{ marginTop: 16, fontSize: 12, color: "var(--text-secondary)" }}>未配置 MCP 服务器</div>}
      {data?.map((s) => (
        <div key={s.name} style={cardStyle}>
          <div style={{ fontWeight: 600, fontSize: 13 }}>{s.name}</div>
          <div style={{ fontSize: 11, color: "var(--text-secondary)", marginTop: 4, fontFamily: "monospace" }}>
            {s.command} {s.args.join(" ")}
          </div>
        </div>
      ))}
    </div>
  );
}

function SessionsPanel() {
  const { data, error, loading, run } = useTauriInvoke<
    { session_id: string; summary: string; message_count: number; updated_at: string }[]
  >("session_list");

  const handleDelete = async (id: string) => {
    if (!confirm("确认删除会话 " + id.slice(0, 8) + "?")) return;
    try {
      await invoke("session_delete", { id });
      run();
    } catch (e) {
      alert("删除失败: " + (e instanceof Error ? e.message : String(e)));
    }
  };

  return (
    <div style={panelBox}>
      <div style={panelTitle}>会话管理</div>
      <div style={panelSubtitle}>对齐 CLI: marguerite session list/delete — 历史会话管理</div>
      <button style={btnGhostStyle} onClick={() => run()}>刷新</button>
      {error && <div style={{ color: "#ef4444", marginTop: 12, fontSize: 12 }}>{error}</div>}
      {loading && <div style={{ fontSize: 12, color: "var(--text-secondary)" }}>加载中…</div>}
      {data?.map((s) => (
        <div key={s.session_id} style={{ ...cardStyle, display: "flex", alignItems: "center", gap: 10 }}>
          <div style={{ flex: 1 }}>
            <div style={{ fontWeight: 600, fontSize: 12, fontFamily: "monospace" }}>{s.session_id.slice(0, 12)}…</div>
            <div style={{ fontSize: 11, color: "var(--text-secondary)" }}>{s.summary || "(无摘要)"}</div>
            <div style={{ fontSize: 10, color: "var(--text-tertiary, #555)" }}>{s.message_count} 条消息 · {s.updated_at}</div>
          </div>
          <button style={{ ...btnGhostStyle, borderColor: "#ef4444", color: "#ef4444" }} onClick={() => handleDelete(s.session_id)}>
            删除
          </button>
        </div>
      ))}
    </div>
  );
}

function SandboxPanel() {
  const { data: info } = useTauriInvoke<{ os: string; platform_support: string; workspace: string }>("sandbox_info");
  const [cmd, setCmd] = useState("");
  const [result, setResult] = useState<{ stdout: string; stderr: string; exit_code: number; decision: string } | null>(null);
  const [running, setRunning] = useState(false);

  const handleExec = async () => {
    if (!cmd.trim()) return;
    setRunning(true);
    try {
      const argv = cmd.trim().split(/\s+/);
      const res = await invoke<{ stdout: string; stderr: string; exit_code: number; decision: string }>("exec_command", { argv });
      setResult(res);
    } catch (e) {
      setResult({ stdout: "", stderr: String(e), exit_code: -1, decision: "error" });
    } finally {
      setRunning(false);
    }
  };

  return (
    <div style={panelBox}>
      <div style={panelTitle}>沙箱执行</div>
      <div style={panelSubtitle}>对齐 CLI: marguerite exec -- &lt;cmd&gt; — 白名单沙箱命令执行</div>
      {info && (
        <div style={{ ...cardStyle, fontSize: 11 }}>
          平台: <b>{info.os}</b> · 沙箱: <b>{info.platform_support}</b> · 工作区: <code>{info.workspace}</code>
        </div>
      )}
      <div style={{ display: "flex", gap: 8, marginBottom: 12 }}>
        <input
          style={inputStyle}
          placeholder='输入命令，如: echo hello / cargo --version'
          value={cmd}
          onChange={(e) => setCmd(e.target.value)}
          onKeyDown={(e) => e.key === "Enter" && handleExec()}
        />
        <button style={btnStyle} onClick={handleExec} disabled={running}>
          {running ? "执行中…" : "执行"}
        </button>
      </div>
      {result && (
        <div style={cardStyle}>
          <div style={{ fontSize: 11, marginBottom: 6 }}>
            决策: <b>{result.decision}</b> · 退出码: <b>{result.exit_code}</b>
          </div>
          {result.stdout && (
            <pre style={{ margin: 0, fontSize: 11, whiteSpace: "pre-wrap", wordBreak: "break-all", color: "#22c55e" }}>
              {result.stdout}
            </pre>
          )}
          {result.stderr && (
            <pre style={{ margin: "4px 0 0", fontSize: 11, whiteSpace: "pre-wrap", wordBreak: "break-all", color: "#ef4444" }}>
              {result.stderr}
            </pre>
          )}
        </div>
      )}
    </div>
  );
}

function ComputerUsePanel() {
  const [action, setAction] = useState("screenshot");
  const [x, setX] = useState("");
  const [y, setY] = useState("");
  const [text, setText] = useState("");
  const [key, setKey] = useState("");
  const [output, setOutput] = useState("");
  const [busy, setBusy] = useState(false);

  const actions = [
    "screenshot", "click", "double_click", "right_click", "middle_click",
    "type", "key", "key_combo", "scroll", "move",
    "clipboard_set", "clipboard_get", "window_focus",
    "cursor_pos", "key_state", "idle_time", "perceive",
  ];

  const handleRun = async () => {
    setBusy(true);
    setOutput("执行中…");
    try {
      const args: Record<string, unknown> = { action };
      if (x) args.x = parseInt(x);
      if (y) args.y = parseInt(y);
      if (text) args.text = text;
      if (key) args.key = key;
      const res = await invoke<string>("computer_use_action", args);
      setOutput(res);
    } catch (e) {
      setOutput("错误: " + (e instanceof Error ? e.message : String(e)));
    } finally {
      setBusy(false);
    }
  };

  const needsCoords = ["click", "double_click", "right_click", "middle_click", "move", "scroll"].includes(action);
  const needsText = action === "type" || action === "clipboard_set";
  const needsKey = ["key", "key_combo", "window_focus", "key_state"].includes(action);

  return (
    <div style={panelBox}>
      <div style={panelTitle}>电脑操控</div>
      <div style={panelSubtitle}>对齐 CLI/TUI computer_use 工具 — 屏幕感知 + 键鼠控制 + GUI 定位（三层架构）</div>
      <div style={{ ...cardStyle, display: "flex", flexDirection: "column", gap: 8 }}>
        <div style={{ display: "flex", gap: 8, flexWrap: "wrap" }}>
          {actions.map((a) => (
            <button
              key={a}
              onClick={() => setAction(a)}
              style={{
                padding: "4px 10px",
                borderRadius: 5,
                border: "1px solid " + (action === a ? "var(--accent-primary)" : "var(--surface-border)"),
                background: action === a ? "var(--accent-primary)" : "transparent",
                color: action === a ? "#fff" : "var(--text-secondary)",
                fontSize: 11,
                cursor: "pointer",
              }}
            >
              {a}
            </button>
          ))}
        </div>
        <div style={{ display: "flex", gap: 8 }}>
          {needsCoords && (
            <>
              <input style={{ ...inputStyle, maxWidth: 100 }} placeholder="X" value={x} onChange={(e) => setX(e.target.value)} />
              <input style={{ ...inputStyle, maxWidth: 100 }} placeholder="Y" value={y} onChange={(e) => setY(e.target.value)} />
            </>
          )}
          {needsText && <input style={inputStyle} placeholder="文本内容" value={text} onChange={(e) => setText(e.target.value)} />}
          {needsKey && <input style={inputStyle} placeholder="按键/窗口标题" value={key} onChange={(e) => setKey(e.target.value)} />}
          <button style={btnStyle} onClick={handleRun} disabled={busy}>{busy ? "…" : "执行"}</button>
        </div>
      </div>
      <pre style={{ ...cardStyle, fontSize: 11, whiteSpace: "pre-wrap", wordBreak: "break-all", maxHeight: 400, overflowY: "auto" }}>
        {output || "选择操作并点击「执行」"}
      </pre>
    </div>
  );
}

function KbPanel() {
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
        {msg && <span style={{ marginLeft: 10, fontSize: 11, color: "#22c55e" }}>{msg}</span>}
      </div>
    </div>
  );
}

function FleetPanel() {
  const { data, error, run } = useTauriInvoke<
    { role_id: string; provider: string; model: string; reasoning: string }[]
  >("fleet_list");
  const [task, setTask] = useState("");
  const [recommendation, setRecommendation] = useState<{ role_id: string; provider: string; model: string; reasoning: string } | null>(null);

  const handleRoute = async () => {
    if (!task.trim()) return;
    try {
      const res = await invoke<{ role_id: string; provider: string; model: string; reasoning: string }>("fleet_route", {
        task: task.trim(),
      });
      setRecommendation(res);
    } catch (e) {
      alert("路由失败: " + (e instanceof Error ? e.message : String(e)));
    }
  };

  return (
    <div style={panelBox}>
      <div style={panelTitle}>模型舰队</div>
      <div style={panelSubtitle}>Fleet 多角色路由 — architect/builder/reviewer 并行编排</div>
      <button style={btnGhostStyle} onClick={() => run()}>刷新</button>
      {error && <div style={{ color: "#ef4444", marginTop: 12, fontSize: 12 }}>{error}</div>}
      <div style={{ marginTop: 12, display: "flex", gap: 8, flexWrap: "wrap" }}>
        {data?.map((r) => (
          <div key={r.role_id} style={{ ...cardStyle, minWidth: 200, flex: 1 }}>
            <div style={{ fontWeight: 700, fontSize: 13, color: "var(--accent-primary)" }}>{r.role_id}</div>
            <div style={{ fontSize: 11, marginTop: 4 }}>模型: <b>{r.model}</b></div>
            <div style={{ fontSize: 10, color: "var(--text-secondary)" }}>{r.provider}</div>
            <div style={{ fontSize: 10, color: "var(--text-tertiary, #555)" }}>推理: {r.reasoning}</div>
          </div>
        ))}
      </div>
      <div style={{ ...cardStyle, marginTop: 16 }}>
        <div style={{ fontWeight: 600, fontSize: 12, marginBottom: 8 }}>任务路由推荐</div>
        <div style={{ display: "flex", gap: 8 }}>
          <input style={inputStyle} placeholder="描述任务，推荐角色…" value={task} onChange={(e) => setTask(e.target.value)} onKeyDown={(e) => e.key === "Enter" && handleRoute()} />
          <button style={btnStyle} onClick={handleRoute}>推荐</button>
        </div>
        {recommendation && (
          <div style={{ marginTop: 10, fontSize: 12 }}>
            推荐角色: <b style={{ color: "var(--accent-primary)" }}>{recommendation.role_id}</b> · 模型: {recommendation.model}
          </div>
        )}
      </div>
    </div>
  );
}

function EvolvePanel() {
  const { data, error, loading, run } = useTauriInvoke<
    { round: number; accepted: boolean; score: number; timestamp: string; summary: string }[]
  >("evolve_history");

  return (
    <div style={panelBox}>
      <div style={panelTitle}>自进化历史</div>
      <div style={panelSubtitle}>EvolutionRunner — prompt 自进化与回滚记录</div>
      <button style={btnGhostStyle} onClick={() => run()}>刷新</button>
      {error && <div style={{ color: "#ef4444", marginTop: 12, fontSize: 12 }}>{error}</div>}
      {loading && <div style={{ fontSize: 12, color: "var(--text-secondary)" }}>加载中…</div>}
      {data?.length === 0 && <div style={{ marginTop: 16, fontSize: 12, color: "var(--text-secondary)" }}>暂无进化记录</div>}
      {data?.map((r) => (
        <div key={r.round} style={{ ...cardStyle, display: "flex", gap: 10, alignItems: "flex-start" }}>
          <Badge ok={r.accepted} label={r.accepted ? "采纳" : "拒绝"} />
          <div style={{ flex: 1 }}>
            <div style={{ fontWeight: 600, fontSize: 12 }}>第 {r.round} 轮 · score: {r.score.toFixed(3)}</div>
            <div style={{ fontSize: 11, color: "var(--text-secondary)" }}>{r.summary}</div>
            <div style={{ fontSize: 10, color: "var(--text-tertiary, #555)" }}>{r.timestamp}</div>
          </div>
        </div>
      ))}
    </div>
  );
}

function PluginsPanel() {
  const { data, error, loading, run } = useTauriInvoke<
    { name: string; kind: string; description: string; version: string }[]
  >("plugins_list");

  return (
    <div style={panelBox}>
      <div style={panelTitle}>插件市场</div>
      <div style={panelSubtitle}>PluginSlot 统一注册槽 — 工具/技能/MCP/命令</div>
      <button style={btnGhostStyle} onClick={() => run()}>刷新</button>
      {error && <div style={{ color: "#ef4444", marginTop: 12, fontSize: 12 }}>{error}</div>}
      {loading && <div style={{ fontSize: 12, color: "var(--text-secondary)" }}>加载中…</div>}
      <div style={{ marginTop: 12, display: "grid", gridTemplateColumns: "repeat(auto-fill, minmax(280px, 1fr))", gap: 8 }}>
        {data?.map((p) => (
          <div key={p.name} style={cardStyle}>
            <div style={{ display: "flex", justifyContent: "space-between" }}>
              <span style={{ fontWeight: 600, fontSize: 12, fontFamily: "monospace" }}>{p.name}</span>
              <span style={{ fontSize: 10, color: "var(--text-secondary)" }}>v{p.version}</span>
            </div>
            <div style={{ fontSize: 10, color: "var(--accent-primary)", marginTop: 2 }}>{p.kind}</div>
            <div style={{ fontSize: 11, color: "var(--text-secondary)", marginTop: 4 }}>{p.description}</div>
          </div>
        ))}
      </div>
    </div>
  );
}

function CapabilitiesPanel() {
  const { data, error } = useTauriInvoke<
    { group: string; items: { id: string; name: string; tier: string; maturity: string; description: string }[] }[]
  >("capabilities_groups");

  return (
    <div style={panelBox}>
      <div style={panelTitle}>能力总览</div>
      <div style={panelSubtitle}>原生 harness 能力注册表（SSOT）— 按组展示真实落地状态</div>
      {error && <div style={{ color: "#ef4444", fontSize: 12 }}>{error}</div>}
      {data?.map((g) => (
        <div key={g.group} style={{ marginBottom: 16 }}>
          <div style={{ fontWeight: 700, fontSize: 13, marginBottom: 6, color: "var(--accent-primary)" }}>{g.group}</div>
          {g.items.map((c) => (
            <div key={c.id} style={{ ...cardStyle, display: "flex", alignItems: "center", gap: 8, padding: "8px 12px" }}>
              <Badge ok={c.maturity === "Real"} label={c.maturity} />
              <div style={{ flex: 1 }}>
                <span style={{ fontWeight: 600, fontSize: 12 }}>{c.name}</span>
                <span style={{ fontSize: 10, color: "var(--text-secondary)", marginLeft: 8 }}>{c.tier}</span>
              </div>
            </div>
          ))}
        </div>
      ))}
    </div>
  );
}

function PlaceholderPanel({ title, subtitle }: { title: string; subtitle: string }) {
  return (
    <div style={panelBox}>
      <div style={panelTitle}>{title}</div>
      <div style={panelSubtitle}>{subtitle}</div>
      <div style={{ ...cardStyle, textAlign: "center", padding: 40, color: "var(--text-secondary)", fontSize: 13 }}>
        该模块正在开发中，敬请期待
      </div>
    </div>
  );
}

function WorkspacePanel() {
  const [name, setName] = useState("");
  const [path, setPath] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const handleCreate = async () => {
    if (!name.trim()) return;
    setBusy(true);
    setError(null);
    setPath(null);
    try {
      const p = await invoke<string>("create_workspace_folder", { name: name.trim() });
      setPath(p);
      setName("");
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div style={panelBox}>
      <div style={panelTitle}>工作区</div>
      <div style={panelSubtitle}>在 ~/.marguerite/workspaces/ 下创建工作区目录（红线修复：此前无法直接创建文件夹）</div>
      <div style={{ ...cardStyle, display: "flex", gap: 8 }}>
        <input
          style={inputStyle}
          placeholder="工作区名称（不可含 / 或 ..）"
          value={name}
          onChange={(e) => setName(e.target.value)}
          onKeyDown={(e) => e.key === "Enter" && handleCreate()}
        />
        <button style={btnStyle} onClick={handleCreate} disabled={busy || !name.trim()}>
          {busy ? "创建中…" : "创建"}
        </button>
      </div>
      {error && <div style={{ ...cardStyle, color: "#ef4444", fontSize: 12, marginTop: 8 }}>{error}</div>}
      {path && (
        <div style={{ ...cardStyle, marginTop: 12 }}>
          <div style={{ fontSize: 11, color: "var(--text-secondary)" }}>已创建目录</div>
          <div style={{ fontSize: 12, fontFamily: "ui-monospace, monospace", marginTop: 4, wordBreak: "break-all" }}>{path}</div>
        </div>
      )}
    </div>
  );
}

function RoutingPanel() {
  const [json, setJson] = useState(
    JSON.stringify(
      { mode: "router", fusion_model: "", candidate_models: [], fixed_model: "", cross_provider_tiers: false },
      null,
      2
    )
  );
  const [msg, setMsg] = useState<{ ok: boolean; text: string } | null>(null);
  const [busy, setBusy] = useState(false);

  const applyMode = (mode: string) => {
    try {
      const obj = JSON.parse(json);
      obj.mode = mode;
      setJson(JSON.stringify(obj, null, 2));
    } catch {
      /* 输入过程中解析失败则忽略 */
    }
  };

  const handleSave = async () => {
    setBusy(true);
    setMsg(null);
    try {
      await invoke("routing_save", { json });
      setMsg({ ok: true, text: "模型路由已保存" });
    } catch (e) {
      setMsg({ ok: false, text: "保存失败: " + (e instanceof Error ? e.message : String(e)) });
    } finally {
      setBusy(false);
      setTimeout(() => setMsg(null), 2500);
    }
  };

  return (
    <div style={panelBox}>
      <div style={panelTitle}>模型路由</div>
      <div style={panelSubtitle}>切换模型路由模式（direct / router / ensemble / bloom），持久化到 config.yaml（红线修复：此前保存必失败）</div>
      <div style={{ display: "flex", gap: 6, marginBottom: 10, flexWrap: "wrap" }}>
        {["direct", "router", "ensemble", "bloom"].map((m) => (
          <button key={m} style={btnGhostStyle} onClick={() => applyMode(m)}>{m}</button>
        ))}
      </div>
      <textarea
        style={{ ...inputStyle, minHeight: 160, resize: "vertical", fontFamily: "ui-monospace, monospace", fontSize: 11, marginBottom: 10 }}
        value={json}
        onChange={(e) => setJson(e.target.value)}
        spellCheck={false}
      />
      <div style={{ display: "flex", alignItems: "center", gap: 10 }}>
        <button style={btnStyle} onClick={handleSave} disabled={busy}>保存路由</button>
        {msg && <span style={{ fontSize: 12, color: msg.ok ? "#22c55e" : "#ef4444" }}>{msg.text}</span>}
      </div>
    </div>
  );
}

function ModlensPanel() {
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

// ───────────── 德州扑克（Game Hall / 赌 Token）─────────────

interface PokerCardView {
  rank: string;
  suit: string;
}

interface PokerPlayerView {
  seat: number;
  name: string;
  tokens: number;
  committed: number;
  street_committed: number;
  folded: boolean;
  all_in: boolean;
  hole: [PokerCardView | null, PokerCardView | null] | null;
  is_human: boolean;
}

interface PokerPotView {
  amount: number;
  eligible: number[];
}

interface PokerRoomView {
  id: string;
  name: string;
  players: PokerPlayerView[];
  board: PokerCardView[];
  street: string;
  dealer_seat: number;
  current_seat: number;
  small_blind: number;
  big_blind: number;
  street_high: number;
  min_raise: number;
  pots: PokerPotView[];
  started: boolean;
  hand_over: boolean;
  winners: [number, number][];
  last_action: string;
}

const POKER_RANK_LABEL: Record<string, string> = {
  Two: "2", Three: "3", Four: "4", Five: "5", Six: "6", Seven: "7",
  Eight: "8", Nine: "9", Ten: "T", Jack: "J", Queen: "Q", King: "K", Ace: "A",
};

const POKER_SUIT_LABEL: Record<string, string> = {
  Spades: "s", Hearts: "h", Diamonds: "d", Clubs: "c",
};

const POKER_RED_SUIT = new Set(["Hearts", "Diamonds"]);

const POKER_STREET_LABEL: Record<string, string> = {
  Preflop: "翻前", Flop: "翻牌", Turn: "转牌", River: "河牌", Showdown: "摊牌",
};

function PokerCardFace({ card }: { card: PokerCardView | null }) {
  if (!card) {
    return (
      <div style={{
        width: 46, height: 62, borderRadius: 6, border: "1px dashed var(--surface-border)",
        display: "inline-flex", alignItems: "center", justifyContent: "center",
        color: "var(--text-tertiary)", fontSize: 10,
      }}>待发</div>
    );
  }
  const red = POKER_RED_SUIT.has(card.suit);
  return (
    <div style={{
      width: 46, height: 62, borderRadius: 6, background: "#FFFFFF", border: "1px solid var(--surface-border)",
      display: "inline-flex", flexDirection: "column", alignItems: "center", justifyContent: "center",
      color: red ? "#DC2626" : "#1F2937", fontWeight: 700, boxShadow: "var(--shadow-sm)",
      fontFamily: "ui-monospace, monospace",
    }}>
      <span style={{ fontSize: 15 }}>{POKER_RANK_LABEL[card.rank] ?? card.rank}</span>
      <span style={{ fontSize: 11 }}>{POKER_SUIT_LABEL[card.suit] ?? card.suit}</span>
    </div>
  );
}

function PokerPanel() {
  const [room, setRoom] = useState<PokerRoomView | null>(null);
  const [roomName, setRoomName] = useState("茉莉牌局");
  const [sb, setSb] = useState("5");
  const [bb, setBb] = useState("10");
  const [raiseTo, setRaiseTo] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const refresh = async (rid: string) => {
    try {
      const r = await invoke<PokerRoomView>("poker_state", { roomId: rid });
      setRoom(r);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  };

  const createAndStart = async () => {
    setBusy(true);
    setError(null);
    try {
      const rid = await invoke<string>("poker_create_room", {
        name: roomName.trim() || "茉莉牌局",
        smallBlind: parseInt(sb, 10) || 5,
        bigBlind: parseInt(bb, 10) || 10,
      });
      await invoke("poker_join_room", { roomId: rid, playerName: "你", tokens: 1000, isHuman: true });
      await invoke("poker_join_room", { roomId: rid, playerName: "莫斯", tokens: 1000, isHuman: false });
      await invoke("poker_join_room", { roomId: rid, playerName: "星黎", tokens: 1000, isHuman: false });
      await invoke("poker_start_room", { roomId: rid });
      await invoke("poker_auto", { roomId: rid });
      await refresh(rid);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  const act = async (action: string, amount?: number) => {
    if (!room) return;
    setBusy(true);
    setError(null);
    try {
      await invoke("poker_action", {
        roomId: room.id,
        seat: room.players.find((p) => p.is_human)?.seat ?? 0,
        action,
        amount: amount ?? null,
      });
      await invoke("poker_auto", { roomId: room.id });
      await refresh(room.id);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
      await refresh(room.id);
    } finally {
      setBusy(false);
    }
  };

  const nextHand = async () => {
    if (!room) return;
    setBusy(true);
    setError(null);
    try {
      await invoke("poker_start_room", { roomId: room.id });
      await invoke("poker_auto", { roomId: room.id });
      await refresh(room.id);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  const human = room?.players.find((p) => p.is_human);
  const humanSeat = human?.seat ?? -1;
  const isMyTurn = !!room && !room.hand_over && room.current_seat === humanSeat;
  const toCall = room && human ? room.street_high - human.street_committed : 0;
  const potTotal = room ? room.pots.reduce((s, p) => s + p.amount, 0) : 0;

  if (!room) {
    return (
      <div style={panelBox}>
        <div style={panelTitle}>德州扑克</div>
        <div style={panelSubtitle}>游戏大厅 · 赌 Token — 多人同桌对战（你 + AI 座位），Token 为局内筹码（本地房间模型，跨机联机需 relay 服务）</div>
        <div style={{ ...cardStyle, display: "flex", flexDirection: "column", gap: 8, maxWidth: 420 }}>
          <input style={inputStyle} placeholder="房间名" value={roomName} onChange={(e) => setRoomName(e.target.value)} />
          <div style={{ display: "flex", gap: 8 }}>
            <input style={inputStyle} placeholder="小盲" value={sb} onChange={(e) => setSb(e.target.value)} />
            <input style={inputStyle} placeholder="大盲" value={bb} onChange={(e) => setBb(e.target.value)} />
          </div>
          <button style={btnStyle} onClick={createAndStart} disabled={busy}>
            {busy ? "创建中…" : "创建房间并开局（1 人 + 2 AI）"}
          </button>
          {error && <div style={{ color: "#ef4444", fontSize: 12 }}>{error}</div>}
        </div>
      </div>
    );
  }

  return (
    <div style={panelBox}>
      <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center" }}>
        <div>
          <div style={panelTitle}>{room.name}</div>
          <div style={panelSubtitle}>
            盲注 {room.small_blind}/{room.big_blind} · 阶段 {POKER_STREET_LABEL[room.street] ?? room.street} · 底池 {potTotal}
          </div>
        </div>
        <button style={btnGhostStyle} onClick={() => refresh(room.id)} disabled={busy}>刷新</button>
      </div>

      {error && <div style={{ ...cardStyle, color: "#ef4444", fontSize: 12, marginTop: 8 }}>{error}</div>}

      {/* 公共牌 */}
      <div style={{ ...cardStyle, marginTop: 12 }}>
        <div style={{ fontSize: 11, color: "var(--text-secondary)", marginBottom: 8 }}>公共牌</div>
        <div style={{ display: "flex", gap: 6 }}>
          {room.board.map((c, i) => <PokerCardFace key={i} card={c} />)}
          {Array.from({ length: Math.max(0, 5 - room.board.length) }).map((_, i) => <PokerCardFace key={"e" + i} card={null} />)}
        </div>
      </div>

      {/* 玩家座位 */}
      <div style={{ marginTop: 12, display: "grid", gridTemplateColumns: "repeat(auto-fill, minmax(200px, 1fr))", gap: 8 }}>
        {room.players.map((p) => {
          const showHole = room.hand_over || p.is_human;
          const isTurn = !room.hand_over && room.current_seat === p.seat;
          return (
            <div key={p.seat} style={{
              ...cardStyle,
              border: isTurn ? "2px solid var(--accent-primary)" : undefined,
              opacity: p.folded ? 0.55 : 1,
            }}>
              <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center" }}>
                <span style={{ fontWeight: 700, fontSize: 13 }}>{p.name}{p.seat === room.dealer_seat ? " · 庄" : ""}{p.is_human ? " · 你" : ""}</span>
                <span style={{ fontSize: 11, color: "var(--text-secondary)" }}>Token {p.tokens}</span>
              </div>
              <div style={{ display: "flex", gap: 4, marginTop: 6 }}>
                {(p.hole && showHole ? p.hole : [null, null]).map((c, i) => <PokerCardFace key={i} card={c} />)}
              </div>
              <div style={{ display: "flex", gap: 4, marginTop: 6, flexWrap: "wrap" }}>
                {p.folded && <Badge ok={false} label="弃牌" />}
                {p.all_in && <Badge ok={true} label="全下" />}
                {isTurn && <Badge ok={true} label="行动中" />}
                {p.committed > 0 && <span style={{ fontSize: 10, color: "var(--text-secondary)" }}>已投入 {p.committed}</span>}
              </div>
            </div>
          );
        })}
      </div>

      {/* 最近动作 + 结果 */}
      <div style={{ ...cardStyle, marginTop: 12, fontSize: 12 }}>
        {room.last_action && <div style={{ color: "var(--text-secondary)" }}>最近动作：{room.last_action}</div>}
        {room.hand_over && room.winners.map(([seat, amount]) => (
          <div key={seat} style={{ marginTop: 4, fontWeight: 600, color: "var(--accent-primary)" }}>
            {room.players[seat]?.name ?? "座位" + seat} 赢得 {amount} Token
          </div>
        ))}
      </div>

      {/* 操作区 */}
      <div style={{ ...cardStyle, marginTop: 12 }}>
        {room.hand_over ? (
          <button style={btnStyle} onClick={nextHand} disabled={busy}>下一局</button>
        ) : isMyTurn ? (
          <div style={{ display: "flex", gap: 8, alignItems: "center", flexWrap: "wrap" }}>
            <button style={btnStyle} onClick={() => act("fold")} disabled={busy}>弃牌</button>
            <button style={btnStyle} onClick={() => act("check")} disabled={busy || toCall > 0}>过牌</button>
            <button style={btnStyle} onClick={() => act("call")} disabled={busy || toCall <= 0}>跟注 {toCall}</button>
            <input
              style={{ ...inputStyle, width: 110 }}
              placeholder={`加注到 ≥${room.street_high + room.min_raise}`}
              value={raiseTo}
              onChange={(e) => setRaiseTo(e.target.value)}
            />
            <button style={btnStyle} onClick={() => act("raise", parseInt(raiseTo, 10))} disabled={busy || !raiseTo}>加注</button>
            <button style={{ ...btnStyle, background: "var(--status-warning)" }} onClick={() => act("all_in")} disabled={busy}>全下</button>
          </div>
        ) : (
          <div style={{ fontSize: 12, color: "var(--text-secondary)" }}>
            等待 {room.players[room.current_seat]?.name ?? "…"} 行动…
          </div>
        )}
      </div>
    </div>
  );
}

// ───────────── 关系图谱（KB 2.0 · kb.graph）─────────────

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

const GRAPH_KIND_LABEL: Record<string, string> = {
  co_occurs: "共现",
  is_a: "是",
  uses: "使用",
  implements: "实现",
  provides: "提供",
};

function GraphPanel() {
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

      {error && <div style={{ ...cardStyle, color: "#ef4444", fontSize: 12, marginTop: 8 }}>{error}</div>}

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

const fmtTokens = (n: number) => {
  if (n >= 1_000_000) return (n / 1_000_000).toFixed(2) + "M";
  if (n >= 1_000) return (n / 1_000).toFixed(1) + "K";
  return String(n);
};

function UsagePanel() {
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

function WhalePet({ mood, running }: { mood: string; running: boolean }) {
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

function PetPanel({ running }: { running: boolean }) {
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

interface FsEntryView {
  name: string;
  path: string;
  is_dir: boolean;
  size: number;
}

function FilePanel() {
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

function ToolPanels({ nav, running }: { nav: string; running: boolean }) {
  switch (nav) {
    case "doctor": return <DoctorPanel />;
    case "skills": return <SkillsPanel />;
    case "mcp": return <McpPanel />;
    case "sessions": return <SessionsPanel />;
    case "sandbox": return <SandboxPanel />;
    case "computer_use": return <ComputerUsePanel />;
    case "kb": return <KbPanel />;
    case "fleet": return <FleetPanel />;
    case "evolve": return <EvolvePanel />;
    case "plugins": return <PluginsPanel />;
    case "capabilities": return <CapabilitiesPanel />;
    case "workspace": return <WorkspacePanel />;
    case "routing": return <RoutingPanel />;
    case "modlens": return <ModlensPanel />;
    case "poker": return <PokerPanel />;
    case "graph": return <GraphPanel />;
    case "usage": return <UsagePanel />;
    case "pet": return <PetPanel running={running} />;
    case "files": return <FilePanel />;
    case "automation": return <PlaceholderPanel title="自动化" subtitle="自动化工作流编排" />;
    case "office_assistant": return <PlaceholderPanel title="办公助理" subtitle="办公场景智能助理" />;
    case "templates": return <PlaceholderPanel title="模板库" subtitle="任务模板库" />;
    default: return null;
  }
}

export default function App() {
  const tauri = useTauri();
  const [nav, setNav] = useState<NavId>("new_task");
  const [mode, setMode] = useState<ModeKind>("work");
  const [workTab, setWorkTab] = useState<WorkTab>("work");
  const [running, setRunning] = useState(false);
  const [workspaces, setWorkspaces] = useState<Workspace[]>(INITIAL_WORKSPACES);
  const [activeSessionId, setActiveSessionId] = useState<string | null>("s6");
  const [messages, setMessages] = useState<ChatMessage[]>([
    {
      id: "m0",
      role: "system",
      content: "会话开始 · 模式：工作",
    },
    {
      id: "m1",
      role: "assistant",
      agent: "Orchestrator",
      content:
        "你好，欢迎使用 Marguerite Fusion。\n我已接入手写引擎 RealEngine + 自进化经验台账 + OpenSquilla 双引擎桥。你可以直接描述任务，或从左侧「新建任务」启动高级计划模式。",
    },
  ]);
  const [agents, setAgents] = useState<SubAgent[]>(INITIAL_AGENTS);
  const [capTotal, setCapTotal] = useState<number | null>(null);
  const [sidecar, setSidecar] = useState<
    "idle" | "starting" | "running" | "stopped" | "crashed"
  >("idle");

  const toggleWorkspace = (id: string) => {
    setWorkspaces((prev) =>
      prev.map((ws) => (ws.id === id ? { ...ws, collapsed: !ws.collapsed } : ws))
    );
  };

  const toggleShowMore = (id: string) => {
    setWorkspaces((prev) =>
      prev.map((ws) => (ws.id === id ? { ...ws, showMore: !ws.showMore } : ws))
    );
  };

  const selectSession = (wsId: string, sessionId: string) => {
    setActiveSessionId(sessionId);
    setNav("new_task");
  };

  // EventBus forward (from rust core to right rail)
  useEffect(() => {
    const unlistenPromises: Promise<UnlistenFn>[] = [];
    if (!tauri) return;
    const eventNames = [
      "mg-event-agent-started",
      "mg-event-agent-progress",
      "mg-event-agent-stopped",
      "mg-event-tool-call",
      "mg-event-llm-chunk",
    ];
    for (const ev of eventNames) {
      try {
        unlistenPromises.push(
          listen(ev, (evt) => {
            // eslint-disable-next-line no-console
            console.debug("[tauri-event]", ev, evt.payload);
          })
        );
      } catch (e) {
        // ignore listen failures in dev fallback
      }
    }
    return () => {
      Promise.all(unlistenPromises)
        .then((fns) => fns.forEach((u) => u()))
        .catch(() => {});
    };
  }, [tauri]);

  // Capabilities total (for nav badge + sidebar KPI)
  useEffect(() => {
    if (!tauri) {
      // dev fallback: use a mocked number so layout looks alive
      setCapTotal(120);
      return;
    }
    (async () => {
      try {
        const s = (await invoke("capabilities_stats")) as { total: number };
        setCapTotal(s.total);
      } catch (e) {
        console.warn("capabilities_stats failed:", e);
      }
    })();
  }, [tauri]);

  const fakeTurn = (text: string) => {
    const uid = "m" + Math.random().toString(36).slice(2, 7);
    setMessages((ms) => [...ms, { id: uid, role: "user", content: text }]);
    setRunning(true);

    // Simulate agent progression.
    setAgents((prev) =>
      prev.map((a) =>
        a.id === "orchestrator"
          ? { ...a, status: "running", progressPct: 15, lastStep: "plan_task → 生成执行计划" }
          : a
      )
    );
    setMessages((ms) => [
      ...ms,
      {
        id: "a_" + Math.random().toString(36).slice(2, 6),
        role: "assistant",
        agent: "Orchestrator",
        content: "收到。我将拆解为：研究/实现/验证 三阶段，并调度子代理并行执行。",
        thinking: true,
      },
    ]);

    // Step 2: 研究代理 + 代码代理
    setTimeout(() => {
      setAgents((prev) =>
        prev.map((a) => {
          if (a.id === "researcher")
            return { ...a, status: "running", progressPct: 40, lastStep: "web_search → 结果聚合" };
          if (a.id === "orchestrator") return { ...a, progressPct: 40 };
          return a;
        })
      );
    }, 700);

    setTimeout(() => {
      setAgents((prev) =>
        prev.map((a) => {
          if (a.id === "coder")
            return { ...a, status: "running", progressPct: 30, lastStep: "grep / edit_file" };
          if (a.id === "researcher") return { ...a, status: "done", progressPct: 100 };
          return a;
        })
      );
      setMessages((ms) => [
        ...ms.filter((m) => !m.thinking),
        {
          id: "a2_" + Math.random().toString(36).slice(2, 6),
          role: "tool",
          agent: "Researcher",
          content:
            "▸ web_search: 3 个来源\n▸ fetch: 2 篇文档\n▸ 相关性排序完成，引用 6 条。",
        },
      ]);
    }, 1500);

    // Step 3: 护栏校验
    setTimeout(() => {
      setAgents((prev) =>
        prev.map((a) => {
          if (a.id === "guard")
            return { ...a, status: "running", progressPct: 55, lastStep: "guard_check → 修复回路" };
          if (a.id === "coder") return { ...a, progressPct: 70 };
          return a;
        })
      );
    }, 2200);

    setTimeout(() => {
      setAgents((prev) =>
        prev.map((a) => {
          if (a.id === "guard") return { ...a, status: "done", progressPct: 100 };
          if (a.id === "sandbox")
            return { ...a, status: "running", progressPct: 35, lastStep: "pty_exec: cargo check" };
          if (a.id === "coder") return { ...a, progressPct: 88 };
          return a;
        })
      );
    }, 3000);

    // Step 4: 收尾：完成 / 经验沉淀
    setTimeout(() => {
      setAgents((prev) =>
        prev.map((a) => ({
          ...a,
          status: a.id === "orchestrator" ? "done" : "idle",
          progressPct: a.id === "orchestrator" ? 100 : 0,
          lastStep: undefined,
        }))
      );
      setMessages((ms) => [
        ...ms.filter((m) => !m.thinking),
        {
          id: "a_final_" + Math.random().toString(36).slice(2, 6),
          role: "assistant",
          agent: "Orchestrator",
          content:
            "任务完成：\n1. 资料检索并归纳 6 条要点\n2. 代码变更 2 个文件，通过语法护栏\n3. 在沙箱运行 3 条验证命令，全部通过\n4. 已沉淀 1 条经验，存入「经验」页。",
        },
      ]);
      setRunning(false);
    }, 4200);
  };

  const realTurn = async (text: string) => {
    if (!tauri) {
      fakeTurn(text);
      return;
    }
    setRunning(true);
    setMessages((ms) => [
      ...ms,
      { id: "u_" + Date.now(), role: "user", content: text },
      {
        id: "a_thinking_" + Date.now(),
        role: "assistant",
        agent: "Orchestrator",
        content: "正在规划步骤、启动子代理…",
        thinking: true,
      },
    ]);
    try {
      const res = (await invoke("run_turn", { prompt: text, sessionId: "gui-session" })) as {
        success: boolean;
        message?: string;
      };
      setMessages((ms) => [
        ...ms.filter((m) => !m.thinking),
        {
          id: "a_res_" + Date.now(),
          role: "assistant",
          agent: "Orchestrator",
          content:
            (res.message ?? "引擎执行完成。") +
            (res.success ? "" : "\n\n⚠️ 本次执行返回非成功状态，建议查看事件流或日志。"),
        },
      ]);
    } catch (e) {
      setMessages((ms) => [
        ...ms.filter((m) => !m.thinking),
        {
          id: "a_err_" + Date.now(),
          role: "assistant",
          agent: "系统",
          content: `引擎调用失败：${e instanceof Error ? e.message : String(e)}`,
        },
      ]);
    } finally {
      setRunning(false);
    }
  };

  const handleSidecarStart = async () => {
    if (!tauri) {
      setSidecar("starting");
      setTimeout(() => setSidecar("running"), 900);
      return;
    }
    setSidecar("starting");
    try {
      await invoke("sidecar_start");
      const poll = window.setInterval(async () => {
        try {
          const s = (await invoke("sidecar_status")) as { status: string };
          if (s.status === "running") {
            setSidecar("running");
            window.clearInterval(poll);
          } else if (s.status === "crashed" || s.status === "stopped") {
            setSidecar(s.status as "crashed" | "stopped");
            window.clearInterval(poll);
          }
        } catch {
          // ignore transient errors
        }
      }, 800);
    } catch (e) {
      setSidecar("crashed");
      console.warn(e);
    }
  };

  const handleNewTask = () => {
    const newId = "s_new_" + Date.now();
    setWorkspaces((prev) =>
      prev.map((ws) =>
        ws.id === "marguerite"
          ? {
              ...ws,
              sessions: [
                { id: newId, title: "新任务 · " + new Date().toLocaleTimeString(), timestamp: "刚刚", isActive: true },
                ...ws.sessions,
              ],
            }
          : ws
      )
    );
    setActiveSessionId(newId);
  };

  return (
    <div style={{ display: "flex", height: "100vh", width: "100vw", overflow: "hidden" }}>
      <style>{`
        @keyframes mg-pulse {
          0%   { transform: scale(0.7); opacity: 0.7; }
          80%  { transform: scale(2.4); opacity: 0;   }
          100% { transform: scale(2.4); opacity: 0;   }
        }
        @keyframes mg-thinking {
          0%, 80%, 100% { opacity: 0.2; }
          40%           { opacity: 1;   }
        }
        *::-webkit-scrollbar { width: 8px; height: 8px; }
        *::-webkit-scrollbar-thumb {
          background: color-mix(in srgb, var(--accent-primary) 16%, transparent);
          border-radius: 10px;
        }
        *::-webkit-scrollbar-thumb:hover {
          background: color-mix(in srgb, var(--accent-primary) 30%, transparent);
        }
        *::-webkit-scrollbar-track { background: transparent; }
      `}</style>
      <Sidebar
        active={nav}
        onNav={setNav}
        capTotal={capTotal}
        onNewTask={handleNewTask}
        workspaces={workspaces}
        onToggleWorkspace={toggleWorkspace}
        onToggleShowMore={toggleShowMore}
        onSelectSession={selectSession}
        activeSessionId={activeSessionId}
        workTab={workTab}
        onWorkTabChange={setWorkTab}
      />
      <div style={{ flex: 1, display: "flex", flexDirection: "column", minWidth: 0 }}>
        <TopBar
          mode={mode}
          onModeChange={setMode}
          engineStatus={{ kind: "ok", label: "引擎 · 就绪" }}
          sidecar={sidecar}
          onStartSidecar={handleSidecarStart}
        />
        <div style={{ flex: 1, display: "flex", minHeight: 0 }}>
          {nav === "new_task" ? (
            <ChatCenter
              messages={messages}
              mode={mode}
              running={running}
              onSend={realTurn}
              agents={agents}
            />
          ) : (
            <ToolPanels nav={nav} running={running} />
          )}
          <RightRail agents={agents} />
        </div>
      </div>
    </div>
  );
}
