import "./theme.css"
import { useCallback, useEffect, useState } from "react"
import { invoke } from "@tauri-apps/api/core"
import { listen, type UnlistenFn } from "@tauri-apps/api/event"
import { ChatCenter } from "./components/ChatCenter"
import { RightRail } from "./components/RightRail"
import { Sidebar } from "./components/Sidebar"
import { TopBar } from "./components/TopBar"
import { INITIAL_AGENTS, INITIAL_WORKSPACES } from "./constants"
import { useTauri } from "./hooks/useTauri"
import { useApprovalBridge } from "./hooks/useApprovalBridge"
import { ApprovalCard } from "./components/ApprovalCard"
import { ToolPanels } from "./panels"
import { ChatMessage, HistorySession, ModeKind, NavId, SubAgent, WorkTab, Workspace } from "./types"

export default function App() {
  const tauri = useTauri();
  const [nav, setNav] = useState<NavId>("new_task");
  const [mode, setMode] = useState<ModeKind>("standard_ask");
  const [workTab, setWorkTab] = useState<WorkTab>("work");
  const [running, setRunning] = useState(false);
  const [workspaces, setWorkspaces] = useState<Workspace[]>(INITIAL_WORKSPACES);
  // A3 假数据清零：首启无预置演示会话/消息，进入「空状态引导」。
  const [activeSessionId, setActiveSessionId] = useState<string | null>(null);
  const [messages, setMessages] = useState<ChatMessage[]>([]);
  const [agents, setAgents] = useState<SubAgent[]>(INITIAL_AGENTS);
  const [capTotal, setCapTotal] = useState<number | null>(null);
  const [sidecar, setSidecar] = useState<
    "idle" | "starting" | "running" | "stopped" | "crashed"
  >("idle");

  const toggleWorkspace = useCallback((id: string) => {
    setWorkspaces((prev) =>
      prev.map((ws) => (ws.id === id ? { ...ws, collapsed: !ws.collapsed } : ws))
    );
  }, []);

  // M2-C1：动作确认卡（Intent Preview）事件桥 —— 订阅 core 挂起审批事件
  const approvalBridge = useApprovalBridge();

  const toggleShowMore = useCallback((id: string) => {
    setWorkspaces((prev) =>
      prev.map((ws) => (ws.id === id ? { ...ws, showMore: !ws.showMore } : ws))
    );
  }, []);

  const selectSession = useCallback((_wsId: string, sessionId: string) => {
    setActiveSessionId(sessionId);
    setNav("new_task");
  }, []);

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
            console.debug("[tauri-event]", ev, evt.payload);
          })
        );
      } catch {
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

  const fakeTurn = useCallback((text: string) => {
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
  }, []);

  const realTurn = useCallback(async (text: string) => {
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
  }, [tauri, fakeTurn]);

  const handleSidecarStart = useCallback(async () => {
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
  }, [tauri]);

  const handleNewTask = useCallback(() => {
    const newId = "s_new_" + Date.now();
    const newSession: HistorySession = {
      id: newId,
      title: "新任务 · " + new Date().toLocaleTimeString(),
      timestamp: "刚刚",
      isActive: true,
    };
    // A3 空态设计：首启无工作区时，首次新建任务创建一个默认工作区。
    setWorkspaces((prev) => {
      const existing =
        prev.length === 0
          ? [{ id: "marguerite", name: "Marguerite", icon: "▢", sessions: [], collapsed: false }]
          : prev;
      return existing.map((ws) =>
        ws.id === "marguerite"
          ? { ...ws, sessions: [newSession, ...ws.sessions] }
          : ws
      );
    });
    setActiveSessionId(newId);
  }, []);

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
            <div style={{ position: "relative", flex: 1, minWidth: 0 }}>
              <ChatCenter
                messages={messages}
                mode={mode}
                running={running}
                onSend={realTurn}
                onNewTask={handleNewTask}
                agents={agents}
              />
              {approvalBridge.pending ? (
                <div
                  style={{
                    position: "absolute",
                    inset: 0,
                    zIndex: 40,
                    display: "flex",
                    justifyContent: "center",
                    alignItems: "flex-start",
                    paddingTop: 24,
                    backgroundColor: "color-mix(in srgb, var(--surface-bg) 64%, transparent)",
                  }}
                  data-testid="approval-overlay"
                >
                  <ApprovalCard
                    approval={approvalBridge.pending}
                    busy={approvalBridge.busy}
                    error={approvalBridge.error}
                    onAllowOnce={() =>
                      approvalBridge.resolve(approvalBridge.pending!.id, "approved")
                    }
                    onDeny={() =>
                      approvalBridge.resolve(approvalBridge.pending!.id, "denied")
                    }
                  />
                </div>
              ) : null}
            </div>
          ) : (
            <ToolPanels nav={nav} running={running} />
          )}
          <RightRail agents={agents} />
        </div>
      </div>
    </div>
  );
}
