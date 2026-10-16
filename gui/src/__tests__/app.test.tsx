import { describe, it, expect } from "vitest";
import { render, screen } from "@testing-library/react";
import App from "../App";

describe("App 整体（M0/M1 验收回归）", () => {
  it("首启渲染空状态引导（无假会话，A3）", () => {
    render(<App />);
    expect(screen.getByText("欢迎使用 Marguerite")).toBeInTheDocument();
  });

  it("渲染侧栏分组与主屏导航（A2）", async () => {
    render(<App />);
    expect(await screen.findByText("对话工作台")).toBeInTheDocument();
    expect(screen.getByText("治理")).toBeInTheDocument();
    expect(screen.getByText("实验室")).toBeInTheDocument();
  });

  it("首启不再出现其它项目复制的假会话（DataRecoveryPro 等已清零）", () => {
    render(<App />);
    expect(screen.queryByText(/DataRecoveryPro/i)).toBeNull();
    expect(screen.queryByText(/CCD Recovery/i)).toBeNull();
    expect(screen.queryByText(/DDOS/i)).toBeNull();
  });
});
