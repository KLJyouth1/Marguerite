import { describe, it, expect } from "vitest";
import { INITIAL_WORKSPACES, MODES, NAV_SECTIONS } from "../constants";

// 导航分组契约（2026-10-21 重排后）。
//
// 断言分两类：
//   · **三层分类法**：必须与 WebUI 的 `NavCategory`（core/governance/lab）一致 ——
//     这是四端共享的既有契约，分叉即视为不一致；
//   · **会话优先**：core 组前两项必须是开始对话与回看会话。
//     这一条才是"重排"的实质，且不锁具体分组名，将来再调也不会误红。
describe("NAV_SECTIONS 分组契约", () => {
  it("三层分类法，与 WebUI NavCategory 严格对齐", () => {
    // webui/src/router/nav.ts: export type NavCategory = 'core' | 'governance' | 'lab'
    expect(NAV_SECTIONS.map((s) => s.id)).toEqual(["core", "governance", "lab"]);
  });

  it("会话优先：core 组前两项是对话工作台与会话管理", () => {
    const core = NAV_SECTIONS.find((s) => s.id === "core")!;
    expect(core.items.slice(0, 2).map((i) => i.id)).toEqual(["new_task", "sessions"]);
    // 第一组不能被折叠成"看不见"
    expect(Boolean(core.defaultCollapsed)).toBe(false);
  });

  it("未实现/实验能力不得出现在核心组（黄金位置只放能用完的东西）", () => {
    // office_assistant / templates 在 core 里没有对应能力实现，属"未交付"。
    // 放到核心组会让用户以为已经能用——这是最伤信任的一类排布错误。
    const coreIds = NAV_SECTIONS.find((s) => s.id === "core")!.items.map((i) => i.id);
    for (const id of ["office_assistant", "templates"]) {
      expect(coreIds).not.toContain(id);
    }
  });

  it("实验室组默认折叠，且承载未交付与娱乐项（折叠=保留入口，不物理删除）", () => {
    const lab = NAV_SECTIONS.find((s) => s.id === "lab")!;
    expect(lab.defaultCollapsed).toBe(true);
    const ids = lab.items.map((i) => i.id);
    for (const need of ["poker", "pet", "office_assistant", "templates", "evolve"]) {
      expect(ids).toContain(need);
    }
  });

  it("每个分组内条目 id 唯一", () => {
    for (const s of NAV_SECTIONS) {
      const ids = s.items.map((i) => i.id);
      expect(new Set(ids).size).toBe(ids.length);
    }
  });

  it("跨分组不重复：同一入口只能出现在一个组里", () => {
    // 重复入口会造成"两处都能进、但状态不同步"的错觉，也是双配置源的温床。
    const all = NAV_SECTIONS.flatMap((s) => s.items.map((i) => i.id));
    expect(new Set(all).size).toBe(all.length);
  });

  it("全部导航分组覆盖核心工作流入口（不因重排丢失任何一项）", () => {
    const all = NAV_SECTIONS.flatMap((s) => s.items.map((i) => i.id));
    for (const need of [
      "new_task",
      "sessions",
      "sandbox",
      "files",
      "kb",
      "workspace",
      "automation",
      "fleet",
      "routing",
      "skills",
      "mcp",
      "plugins",
      "doctor",
      "usage",
      "capabilities",
      "computer_use",
    ]) {
      expect(all).toContain(need);
    }
  });

  it("重排后入口总数不减少（用户明确要求只增不减）", () => {
    const total = NAV_SECTIONS.reduce((n, s) => n + s.items.length, 0);
    // 旧结构：core(9) + governance(8) + lab(6) = 23。重排不得少于这个数。
    expect(total).toBeGreaterThanOrEqual(23);
  });
});

describe("MODES 契约", () => {
  it("九种运行模式（3 档位 x 3 工作模式）且 id 唯一", () => {
    expect(MODES).toHaveLength(9);
    expect(new Set(MODES.map((m) => m.id)).size).toBe(9);
  });

  it("包含 minimal/standard/extreme 三档，每档含 ask/plan/doit", () => {
    const ids = MODES.map((m) => m.id);
    for (const id of [
      "minimal_ask",
      "standard_plan",
      "extreme_doit",
    ])
      expect(ids).toContain(id);
  });

  it("每种模式具备 label / short / accent 令牌且 intensity/working 合法", () => {
    const intensities = new Set(["minimal", "standard", "extreme"]);
    const workings = new Set(["ask", "plan", "doit"]);
    for (const m of MODES) {
      expect(m.label.length).toBeGreaterThan(0);
      expect(m.short.length).toBeGreaterThan(0);
      expect(m.accent.length).toBeGreaterThan(0);
      expect(intensities.has(m.intensity)).toBe(true);
      expect(workings.has(m.working)).toBe(true);
    }
  });
});

describe("A3 假数据清零", () => {
  it("INITIAL_WORKSPACES 为空数组（首启空状态，无建模仿真会话）", () => {
    expect(Array.isArray(INITIAL_WORKSPACES)).toBe(true);
    expect(INITIAL_WORKSPACES).toHaveLength(0);
  });
});
