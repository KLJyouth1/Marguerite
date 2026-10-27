import { describe, it, expect } from "vitest";
import { Profiler, useState } from "react";
import { render, fireEvent, act } from "@testing-library/react";
import { Sidebar } from "../components/Sidebar";
import { TopBar } from "../components/TopBar";
import { ChatCenter } from "../components/ChatCenter";
import { RightRail } from "../components/RightRail";
import { EmptyStateGuide } from "../components/EmptyStateGuide";
import { AgentCard } from "../components/AgentCard";
import { Bubble } from "../components/Bubble";
import { ToolPanels } from "../panels";
import { INITIAL_AGENTS } from "../constants";

type RenderSample = {
  id: string;
  phase: "mount" | "update" | "nested-update";
  actualDuration: number;
  baseDuration: number;
};

async function profileChurn<T>(Component: React.ElementType, props: T, churns: number) {
  const samples: RenderSample[] = [];
  const Harness = function Harness() {
    const [tick, setTick] = useState(0);
    void tick;
    return (
      <div>
        <button onClick={() => setTick((t) => t + 1)}>churn</button>
        <Profiler
          id="probe"
          onRender={(id, phase, actualDuration, baseDuration) =>
            samples.push({ id, phase, actualDuration, baseDuration })
          }
        >
          <Component {...(props as object)} />
        </Profiler>
      </div>
    );
  };
  const view = render(<Harness />);
  for (let i = 0; i < churns; i++) {
    await act(async () => {
      fireEvent.click(view.getByText("churn"));
    });
  }
  view.unmount();
  return samples;
}

describe("B3: React.memo 统一包装 + 渲染频率对比（M1 验收）", () => {
  const memoWrapped = [
    Sidebar, TopBar, ChatCenter, RightRail, EmptyStateGuide, AgentCard, Bubble, ToolPanels,
  ];
  const memoType = Symbol.for("react.memo");

  it("核心产品组件均已挂载 React.memo（接线断言）", () => {
    for (const C of memoWrapped) {
      expect(C).toBeDefined();
      expect(C.$$typeof).toBe(memoType);
    }
  });

  it("同 props 下父级无关重渲染被 memo 短路（真实渲染频率对比）", async () => {
    const agents = INITIAL_AGENTS;
    // memo 包装：React.memo 暴露内部原函数于 .type
    const MemoComponent = RightRail as unknown as React.FC<{ agents: typeof agents }>;
    const InnerFn = (RightRail as { type?: unknown }).type as React.FC<{ agents: typeof agents }>;
    expect(InnerFn).toBeTypeOf("function");

    const memoized = await profileChurn(MemoComponent, { agents }, 6);
    const plain = await profileChurn(InnerFn, { agents }, 6);

    const sum = (s: RenderSample[], k: "actualDuration" | "baseDuration") =>
      s.reduce((a, b) => a + b[k], 0);
    const memoUpdateActual = sum(
      memoized.filter((s) => s.phase === "update"),
      "actualDuration"
    );
    const plainUpdateActual = sum(
      plain.filter((s) => s.phase === "update"),
      "actualDuration"
    );
    const memoUpdateBase = sum(
      memoized.filter((s) => s.phase === "update"),
      "baseDuration"
    );
    const plainUpdateBase = sum(
      plain.filter((s) => s.phase === "update"),
      "baseDuration"
    );

    // 可量化证据（真数）写入报告：
    // - actualDuration：本次提交中子树实际承担的渲染工作
    // - baseDuration：同一子树若不做 memo 短路的假设成本（React Profiler 语义）
    console.log(
      `[B3-render] memo 化: updateActual=${memoUpdateActual.toFixed(4)}ms updateBase(无memo假设)=${memoUpdateBase.toFixed(4)}ms; ` +
        `未包装: updateActual=${plainUpdateActual.toFixed(4)}ms updateBase=${plainUpdateBase.toFixed(4)}ms`
    );
    // 依据：memo 化子树实际渲染工作显著小于未包装基线（同 props 父级 churn）
    expect(memoUpdateActual).toBeLessThan(plainUpdateActual);
    // 依据：memo 短路的组件，其实际成本远低于"不短路"的假设成本
    expect(memoUpdateActual).toBeLessThan(memoUpdateBase);
  });
});
