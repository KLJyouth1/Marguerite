import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, waitFor } from "@testing-library/react";
import axe from "axe-core";
import { ApprovalCard } from "../components/ApprovalCard";
import { UsagePanel } from "../panels/UsagePanel";
import type { ApprovalRequest } from "../types";

// Tauri IPC mock（UsagePanel 依赖）
vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(async (cmd: string) => {
    if (cmd === "usage_summary") {
      return {
        total_tokens: 123_456,
        total_calls: 321,
        total_cost_usd: 12.34,
        currency: "USD",
        models: [
          { model: "deepseek-v3", calls: 200, tokens: 100_000, cost_usd: 10.0 },
          { model: "deepseek-r1", calls: 121, tokens: 23_456, cost_usd: 2.34 },
        ],
      };
    }
    if (cmd === "usage_daily") {
      return {
        days: [
          { day: "2026-09-01", total_tokens: 10_000, calls: 20, cost_usd: 0.1 },
          { day: "2026-09-02", total_tokens: 20_000, calls: 30, cost_usd: 0.2 },
        ],
      };
    }
    throw new Error(`unexpected invoke: ${cmd}`);
  }),
}));

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

function safeApproval(): ApprovalRequest {
  return {
    id: "ap_2",
    tool: "edit_file",
    argsPreview: '{"path":"D:/tmp/a.txt"}',
    risk: "low",
    reason: "低风险单文件修改",
    session: "s1",
    intent: { kind: "modify", target: "D:/tmp/a.txt", scope: "单文件" },
  };
}

type AxeResult = {
  violations: Array<{ id: string; impact?: string; help: string; nodes: unknown[] }>;
  incomplete: Array<{ id: string; impact?: string; help: string; nodes: unknown[] }>;
};

function wcagViolations(r: AxeResult) {
  return r.violations.filter(v => {
    // 仅统计 serious/critical（WCAG A/AA 主体）
    return v.impact === "serious" || v.impact === "critical";
  });
}

beforeEach(() => {
  document.body.innerHTML = "";
});

describe("M3-E3 GUI axe 合规扫描 (WCAG A/AA)", () => {
  it("ApprovalCard（高风险不可逆）无 serious/critical WCAG 违规", async () => {
    const el = document.createElement("div");
    document.body.appendChild(el);
    render(
      <ApprovalCard
        approval={base}
        onAllowOnce={() => {}}
        onDeny={() => {}}
      />,
      { container: el },
    );
    const result = (await axe.run(el, {
      runOnly: { type: "tag", values: ["wcag2a", "wcag2aa", "wcag21a", "wcag21aa"] },
    })) as AxeResult;
    const serious = wcagViolations(result);
    if (result.violations.length) {
      // 失败诊断用
      // eslint-disable-next-line no-console
      console.warn(
        "ApprovalCard(high) violations:",
        result.violations.map(v => `${v.id}(${v.impact}):${v.nodes.length}`),
      );
    }
    expect(serious, "ApprovalCard(high) 存在 serious/critical 违规").toEqual([]);
    el.remove();
  });

  it("ApprovalCard（低风险含『允许同类』）无 serious/critical WCAG 违规", async () => {
    const el = document.createElement("div");
    document.body.appendChild(el);
    render(
      <ApprovalCard
        approval={safeApproval()}
        onAllowOnce={() => {}}
        onAllowAlways={() => {}}
        onDeny={() => {}}
      />,
      { container: el },
    );
    const result = (await axe.run(el, {
      runOnly: { type: "tag", values: ["wcag2a", "wcag2aa", "wcag21a", "wcag21aa"] },
    })) as AxeResult;
    const serious = wcagViolations(result);
    if (result.violations.length) {
      // eslint-disable-next-line no-console
      console.warn(
        "ApprovalCard(safe) violations:",
        result.violations.map(v => `${v.id}(${v.impact}):${v.nodes.length}`),
      );
    }
    expect(serious, "ApprovalCard(safe) 存在 serious/critical 违规").toEqual([]);
    el.remove();
  });

  it("UsagePanel（含刷新按钮/热力图/月度开销）无 serious/critical WCAG 违规", async () => {
    const el = document.createElement("div");
    document.body.appendChild(el);
    render(<UsagePanel />, { container: el });
    // 等待异步加载完成
    await waitFor(() => {
      expect(el.textContent).toContain("消费统计");
    });
    await new Promise(resolve => setTimeout(resolve, 50));
    const result = (await axe.run(el, {
      runOnly: { type: "tag", values: ["wcag2a", "wcag2aa", "wcag21a", "wcag21aa"] },
    })) as AxeResult;
    const serious = wcagViolations(result);
    if (result.violations.length) {
      // 失败诊断用
      // eslint-disable-next-line no-console
      console.warn(
        "UsagePanel violations:",
        result.violations.map(v => `${v.id}(${v.impact}):${v.nodes.length}`),
      );
    }
    expect(serious, "UsagePanel 存在 serious/critical 违规").toEqual([]);
    el.remove();
  });
});
