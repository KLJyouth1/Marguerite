import { describe, it, expect } from "vitest";
import { render, screen } from "@testing-library/react";
import { Bubble } from "../components/Bubble";
import { ChatMessage } from "../types";

const userMsg: ChatMessage = { id: "m1", role: "user", content: "帮我分析这个项目" };
const asstMsg: ChatMessage = { id: "m2", role: "assistant", content: "好的，正在分析", agent: "编排器" };
const sysMsg: ChatMessage = { id: "m3", role: "system", content: "会话已创建" };

describe("Bubble 会话气泡", () => {
  it("渲染用户消息", () => {
    render(<Bubble msg={userMsg} />);
    expect(screen.getByText("帮我分析这个项目")).toBeInTheDocument();
  });

  it("渲染助手消息与来源代理名", () => {
    render(<Bubble msg={asstMsg} />);
    expect(screen.getByText("好的，正在分析")).toBeInTheDocument();
  });

  it("system 消息以居中胶囊呈现", () => {
    render(<Bubble msg={sysMsg} />);
    expect(screen.getByText("会话已创建")).toBeInTheDocument();
  });
});
