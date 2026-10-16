import { describe, it, expect } from "vitest";
import { render, screen } from "@testing-library/react";
import { DoctorPanel } from "../panels/DoctorPanel";
import { SkillsPanel } from "../panels/SkillsPanel";
import { KbPanel } from "../panels/KbPanel";
import { McpPanel } from "../panels/McpPanel";

// 冒烟：非 Tauri 环境（浏览器 dev/CI）下面板须可安全渲染出标题与空态，不崩溃。
describe("工具面板冒烟渲染", () => {
  it("DoctorPanel 呈现\"系统诊断\"标题", () => {
    render(<DoctorPanel />);
    expect(screen.getByText("系统诊断")).toBeInTheDocument();
  });

  it("SkillsPanel 呈现\"技能管理\"标题", () => {
    render(<SkillsPanel />);
    expect(screen.getByText("技能管理")).toBeInTheDocument();
  });

  it("KbPanel 呈现\"知识库\"标题", () => {
    render(<KbPanel />);
    expect(screen.getByText("知识库")).toBeInTheDocument();
  });

  it("McpPanel 呈现\"MCP 服务器\"标题", () => {
    render(<McpPanel />);
    expect(screen.getByText("MCP 服务器")).toBeInTheDocument();
  });
});
