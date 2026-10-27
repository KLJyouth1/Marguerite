import { describe, it, expect, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent, { UserEvent } from "@testing-library/user-event";
import { ChatCenter } from "../components/ChatCenter";
import { ChatMessage, SubAgent } from "../types";

const agents: SubAgent[] = [];
const messages: ChatMessage[] = [
  { id: "m1", role: "user", content: "你好" },
  { id: "m2", role: "assistant", content: "你好，我是 Marguerite" },
];

describe("ChatCenter 会话中心", () => {
  const renderChat = (msgs: ChatMessage[]) =>
    render(
      <ChatCenter
        messages={msgs}
        mode="standard_ask"
        running={false}
        onSend={() => {}}
        onNewTask={() => {}}
        agents={agents}
      />
    );

  it("无消息时展示空状态引导卡", () => {
    renderChat([]);
    expect(screen.getByText("欢迎使用 Marguerite")).toBeInTheDocument();
  });

  it("有消息时逐条渲染气泡", () => {
    renderChat(messages);
    expect(screen.getByText("你好")).toBeInTheDocument();
    expect(screen.getByText("你好，我是 Marguerite")).toBeInTheDocument();
  });

  it("输入并发送触发 onSend 且清空输入", async () => {
    const user: UserEvent = userEvent.setup();
    const onSend = vi.fn();
    render(
      <ChatCenter
        messages={[]}
        mode="standard_ask"
        running={false}
        onSend={onSend}
        onNewTask={() => {}}
        agents={agents}
      />
    );
    const input = screen.getByPlaceholderText(/描述|任务|输入/i) as HTMLInputElement;
    await user.type(input, "帮我整理文件");
    await user.click(screen.getByRole("button", { name: /发送|➤|send/i }));
    expect(onSend).toHaveBeenCalledWith("帮我整理文件");
    expect(input.value).toBe("");
  });
});
