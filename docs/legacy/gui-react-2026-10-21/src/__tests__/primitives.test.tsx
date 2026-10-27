import { describe, it, expect } from "vitest";
import { render, screen } from "@testing-library/react";
import { StatusDot, Badge, ProgressBar, ChevronIcon } from "../components/primitives";

describe("primitives 基础组件", () => {
  it("Badge 渲染 ok/否态标签", () => {
    const { rerender } = render(<Badge ok label="正常" />);
    expect(screen.getByText("正常")).toBeInTheDocument();
    rerender(<Badge ok={false} label="异常" />);
    expect(screen.getByText("异常")).toBeInTheDocument();
  });

  it("StatusDot 按状态绘制（running 含 pulse 动画层）且不含随机 key", () => {
    const { container } = render(<StatusDot status="running" />);
    expect(container.querySelector("span")).toBeTruthy();
    expect(container.innerHTML).not.toMatch(/Math\.random/);
  });

  it("ProgressBar 按 value 绘制填充宽度", () => {
    const { container } = render(<ProgressBar value={42} color="var(--accent-primary)" />);
    expect(container.querySelector("div[style*='42%']")).toBeTruthy();
  });

  it("ChevronIcon 随 open 状态做旋转变换", () => {
    const { container, rerender } = render(<ChevronIcon open />);
    expect(container.querySelector("span")).toBeTruthy();
    expect(container.innerHTML).toContain("rotate(90deg)");
    rerender(<ChevronIcon open={false} />);
    expect(container.innerHTML).toContain("rotate(0deg)");
  });
});
