import { describe, it, expect } from "vitest";
import { render, screen } from "@testing-library/react";
import { AgentCard } from "../components/AgentCard";
import { RightRail } from "../components/RightRail";
import { SubAgent } from "../types";

const idleAgent: SubAgent = {
  id: "orchestrator",
  name: "编排器",
  role: "Orchestrator · 主代理",
  color: "var(--accent-primary)",
  status: "idle",
  progressPct: 0,
  tools: ["plan_task", "run_turn", "recall"],
};

const runningAgent: SubAgent = {
  id: "coder",
  name: "代码代理",
  role: "Coder Agent · 工程实现",
  color: "#6366F1",
  status: "running",
  progressPct: 68,
  tools: ["read_file", "edit"],
  lastStep: "正在重构 App.tsx",
};

describe("AgentCard 子代理卡片", () => {
  it("idle 态渲染名称/角色/待命标签与工具", () => {
    render(<AgentCard agent={idleAgent} />);
    expect(screen.getByText("编排器")).toBeInTheDocument();
    expect(screen.getByText("Orchestrator · 主代理")).toBeInTheDocument();
    expect(screen.getByText("待命中")).toBeInTheDocument();
    expect(screen.getByText("plan_task")).toBeInTheDocument();
  });

  it("running 态渲染运行中标签/进度/最近步骤", () => {
    const { container } = render(<AgentCard agent={runningAgent} />);
    expect(screen.getByText("运行中")).toBeInTheDocument();
    expect(screen.getByText("正在重构 App.tsx")).toBeInTheDocument();
    expect(container.querySelector('[style*="width: 68%"]')).toBeTruthy();
  });
});

describe("RightRail 子代理列表", () => {
  it("无运行代理时显示待命徽标", () => {
    render(<RightRail agents={[idleAgent]} />);
    expect(screen.getByText("子代理 · 并行执行")).toBeInTheDocument();
    expect(screen.getByText("待命")).toBeInTheDocument();
  });

  it("存在运行代理时显示运行计数", () => {
    render(<RightRail agents={[idleAgent, runningAgent]} />);
    expect(screen.getByText("1 运行中")).toBeInTheDocument();
  });
});
