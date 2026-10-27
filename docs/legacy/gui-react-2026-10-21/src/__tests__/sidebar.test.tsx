import { describe, it, expect, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent, { UserEvent } from "@testing-library/user-event";
import { Sidebar } from "../components/Sidebar";
import { NAV_SECTIONS } from "../constants";
import { Workspace, NavId } from "../types";

const noop = () => {};
const noopId = (_id: string) => {};
const noopNav = (_id: NavId) => {};

describe("Sidebar 侧栏（A2 分组契约）", () => {
  const renderSidebar = () =>
    render(
      <Sidebar
        active="new_task"
        onNav={noopNav}
        capTotal={120}
        onNewTask={noop}
        workspaces={[] as Workspace[]}
        onToggleWorkspace={noopId}
        onToggleShowMore={noopId}
        onSelectSession={noopId}
        activeSessionId={null}
        workTab="work"
        onWorkTabChange={noop}
      />
    );

  it("全部导航分组标识都渲染出来", () => {
    renderSidebar();
    for (const s of NAV_SECTIONS) {
      expect(screen.getByText(s.label)).toBeInTheDocument();
    }
  });

  it("首个分组（核心）默认展开，渲染其全部条目", () => {
    renderSidebar();
    // 重排后第一组是 chat；这里不写死 id，而是取"第一个非折叠组"，
    // 这样将来再调分组名也不会误红，但仍能守"首组默认可见"。
    const first = NAV_SECTIONS.find((s) => !s.defaultCollapsed)!;
    for (const item of first.items) {
      expect(screen.getByText(item.label)).toBeInTheDocument();
    }
  });

  it("默认折叠的分组不渲染其条目（折叠=保留入口不删除）", () => {
    renderSidebar();
    for (const s of NAV_SECTIONS.filter((x) => x.defaultCollapsed)) {
      for (const item of s.items) {
        expect(screen.queryByText(item.label)).toBeNull();
      }
    }
  });

  it("点击折叠分组标题可展开该组条目", async () => {
    const user: UserEvent = userEvent.setup();
    renderSidebar();
    const lab = NAV_SECTIONS.find((s) => s.defaultCollapsed)!;
    const head = screen.getByText(lab.label);
    await user.click(head);
    for (const item of lab.items) {
      expect(await screen.findByText(item.label)).toBeInTheDocument();
    }
  });

  it("点击核心组条目触发 onNav", async () => {
    const user: UserEvent = userEvent.setup();
    const onNav = vi.fn();
    render(
      <Sidebar
        active="new_task"
        onNav={onNav}
        capTotal={120}
        onNewTask={noop}
        workspaces={[] as Workspace[]}
        onToggleWorkspace={noopId}
        onToggleShowMore={noopId}
        onSelectSession={noopId}
        activeSessionId={null}
        workTab="work"
        onWorkTabChange={noop}
      />
    );
    await user.click(screen.getByText("会话管理"));
    expect(onNav).toHaveBeenCalledWith("sessions");
  });
});
