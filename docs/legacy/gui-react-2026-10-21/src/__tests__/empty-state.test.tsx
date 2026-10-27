import { describe, it, expect, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent, { UserEvent } from "@testing-library/user-event";
import { EMPTY_START_SUGGESTIONS, EmptyStateGuide } from "../components/EmptyStateGuide";

describe("EmptyStateGuide 空状态引导卡（A3）", () => {
  it("渲染欢迎语与新手引导文案", () => {
    render(<EmptyStateGuide onSend={() => {}} onNewTask={() => {}} />);
    expect(screen.getByText("欢迎使用 Marguerite")).toBeInTheDocument();
  });

  it("渲染 3 条起始任务建议（北极星：90 秒内发起首任务）", () => {
    render(<EmptyStateGuide onSend={() => {}} onNewTask={() => {}} />);
    expect(EMPTY_START_SUGGESTIONS).toHaveLength(3);
    for (const s of EMPTY_START_SUGGESTIONS) {
      expect(screen.getByText(s)).toBeInTheDocument();
    }
  });

  it("点击建议触发 onSend 上屏", async () => {
    const user: UserEvent = userEvent.setup();
    const onSend = vi.fn();
    render(<EmptyStateGuide onSend={onSend} onNewTask={() => {}} />);
    await user.click(screen.getByText(EMPTY_START_SUGGESTIONS[0]));
    expect(onSend).toHaveBeenCalledWith(EMPTY_START_SUGGESTIONS[0]);
  });

  it("点击「新建任务」触发 onNewTask", async () => {
    const user: UserEvent = userEvent.setup();
    const onNewTask = vi.fn();
    render(<EmptyStateGuide onSend={() => {}} onNewTask={onNewTask} />);
    await user.click(screen.getByText("＋ 新建任务"));
    expect(onNewTask).toHaveBeenCalledTimes(1);
  });
});
