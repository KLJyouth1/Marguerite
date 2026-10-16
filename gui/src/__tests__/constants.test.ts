import { describe, it, expect } from "vitest";
import { INITIAL_WORKSPACES, MODES, NAV_SECTIONS } from "../constants";

// 路线图 A2 / B3：导航分组契约（核心 ≤9 / 治理 / 实验室默认折叠），
// 玩具功能折叠进实验室、不物理删除。
describe("NAV_SECTIONS 分组契约", () => {
  it("按 core/governance/lab 三大分组组织", () => {
    expect(NAV_SECTIONS.map((s) => s.id)).toEqual(["core", "governance", "lab"]);
  });

  it("core 主屏条目数 ≤ 9（北极星：首次任务 90 秒完成率）", () => {
    const core = NAV_SECTIONS.find((s) => s.id === "core")!;
    expect(core.items.length).toBeLessThanOrEqual(9);
  });

  it("core 不承载跑偏玩具（氪金扑克/鲸鱼娘不在主链）", () => {
    const core = NAV_SECTIONS.find((s) => s.id === "core")!.items.map((i) => i.id);
    expect(core).not.toContain("poker");
    expect(core).not.toContain("whale");
  });

  it("lab 默认折叠且包含玩具（折叠=保留入口，不物理删除）", () => {
    const lab = NAV_SECTIONS.find((s) => s.id === "lab")!;
    expect(lab.defaultCollapsed).toBe(true);
    const ids = lab.items.map((i) => i.id);
    expect(ids).toContain("poker");
    expect(ids).toContain("pet");
  });

  it("每个分组内条目 id 唯一", () => {
    for (const s of NAV_SECTIONS) {
      const ids = s.items.map((i) => i.id);
      expect(new Set(ids).size).toBe(ids.length);
    }
  });

  it("三个导航分组至少覆盖核心工作流入口", () => {
    const all = NAV_SECTIONS.flatMap((s) => s.items.map((i) => i.id));
    for (const need of ["new_task", "sessions", "sandbox", "files", "kb", "workspace"]) {
      expect(all).toContain(need);
    }
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
