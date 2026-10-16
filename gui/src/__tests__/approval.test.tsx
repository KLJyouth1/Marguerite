import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent, renderHook, act } from "@testing-library/react";
import { ApprovalCard } from "../components/ApprovalCard";
import {
  normalizeApproval,
  sampleApprovalPayload,
  useApprovalBridge,
} from "../hooks/useApprovalBridge";
import type { ApprovalRequest } from "../types";

const base: ApprovalRequest = {
  id: "ap_1",
  tool: "delete_file",
  argsPreview: '{"path":"D:/tmp/x"}',
  risk: "high",
  reason: "高影响操作：批量删除需用户确认",
  session: "s1",
  intent: {
    kind: "delete",
    target: "D:/tmp/x",
    scope: "x 目录下 23 个文件",
    irreversible: true,
    destructive: true,
    high_risk: true,
    side_effects: ["永久删除无法恢复", "影响归档索引"],
    explain: "删除归档目录以释放空间",
  },
};

describe("normalizeApproval 载荷归一化", () => {
  it("缺 approval_id 返回 null", () => {
    expect(normalizeApproval({ tool: "edit_file" })).toBeNull();
  });

  it("从 envelope payload 归一化为 ApprovalRequest", () => {
    const req = normalizeApproval(sampleApprovalPayload());
    expect(req).not.toBeNull();
    expect(req!.id).toBe("ap_sample");
    expect(req!.tool).toBe("delete_file");
    expect(req!.risk).toBe("high");
    expect(req!.intent?.kind).toBe("delete");
    expect(req!.session).toBe("gui-session");
  });

  it("非对象输入返回 null", () => {
    expect(normalizeApproval("x")).toBeNull();
    expect(normalizeApproval(null)).toBeNull();
  });
});

describe("ApprovalCard 动作确认卡", () => {
  it("渲染结构化意图预览（类型/目标/范围/副作用）", () => {
    render(
      <ApprovalCard
        approval={base}
        onAllowOnce={() => {}}
        onDeny={() => {}}
      />
    );
    expect(screen.getByTestId("approval-card")).toBeInTheDocument();
    expect(screen.getByText("删除")).toBeInTheDocument(); // 操作类型
    // 目标对象 + 参数预览各出现一次该路径文本
    expect(screen.getAllByText("D:/tmp/x", { exact: false }).length).toBeGreaterThanOrEqual(2);
    expect(screen.getByText("永久删除无法恢复")).toBeInTheDocument();
    expect(screen.getByText("结构化意图预览")).toBeInTheDocument();
  });

  it("高风险不可逆操作显示风险块且不提供『允许同类』", () => {
    const allowAlways = vi.fn();
    render(
      <ApprovalCard
        approval={base}
        onAllowOnce={() => {}}
        onAllowAlways={allowAlways}
        onDeny={() => {}}
      />
    );
    expect(screen.getByTestId("approval-risk")).toBeInTheDocument();
    expect(screen.getByText("不可逆操作")).toBeInTheDocument();
    expect(screen.queryByText("允许同类")).toBeNull();
  });

  it("非破坏性操作提供『允许同类』按钮", () => {
    const allowAlways = vi.fn();
    render(
      <ApprovalCard
        approval={{
          ...base,
          risk: "medium",
          intent: {
            kind: "modify",
            target: "src/App.tsx",
            destructive: false,
            irreversible: false,
            side_effects: [],
          },
        }}
        onAllowOnce={() => {}}
        onAllowAlways={allowAlways}
        onDeny={() => {}}
      />
    );
    const btn = screen.getByText("允许同类");
    fireEvent.click(btn);
    expect(allowAlways).toHaveBeenCalledTimes(1);
  });

  it("点击『批准执行一次』与『拒绝』分别触发回调", () => {
    const onAllowOnce = vi.fn();
    const onDeny = vi.fn();
    render(
      <ApprovalCard
        approval={base}
        onAllowOnce={onAllowOnce}
        onDeny={onDeny}
      />
    );
    fireEvent.click(screen.getByTestId("approval-allow-once"));
    expect(onAllowOnce).toHaveBeenCalledTimes(1);
    fireEvent.click(screen.getByTestId("approval-deny"));
    expect(onDeny).toHaveBeenCalledTimes(1);
  });

  it("busy 时禁用按钮", () => {
    render(
      <ApprovalCard
        approval={base}
        busy
        onAllowOnce={() => {}}
        onDeny={() => {}}
      />
    );
    expect((screen.getByTestId("approval-allow-once") as HTMLButtonElement).disabled).toBe(true);
    expect((screen.getByTestId("approval-deny") as HTMLButtonElement).disabled).toBe(true);
  });

  it("intent 缺失时按 tool 兜底标题", () => {
    const req: ApprovalRequest = { id: "ap_2", tool: "edit_file", risk: "low", session: "s1" };
    render(<ApprovalCard approval={req} onAllowOnce={() => {}} onDeny={() => {}} />);
    expect(screen.getByTestId("approval-title").textContent).toContain("edit_file");
  });
});

describe("useApprovalBridge 事件桥", () => {
  it("dev fallback：dispatchApproval 注入挂起审批，resolve 后清除", async () => {
    const { result } = renderHook(() => useApprovalBridge());
    act(() => {
      result.current.dispatchApproval(sampleApprovalPayload());
    });
    expect(result.current.pending?.id).toBe("ap_sample");

    await act(async () => {
      await result.current.resolve("ap_sample", "approved");
    });
    expect(result.current.pending).toBeNull();
    expect(result.current.error).toBeNull();
  });

  it("resolve 不匹配的 approval_id 时保留当前审批", async () => {
    const { result } = renderHook(() => useApprovalBridge());
    act(() => {
      result.current.dispatchApproval(sampleApprovalPayload());
    });
    await act(async () => {
      await result.current.resolve("ap_other", "denied");
    });
    expect(result.current.pending?.id).toBe("ap_sample");
  });
});
