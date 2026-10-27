// 自动化面板契约测试（2026-10-21 审计补齐）。
//
// 锁三件事：
//   1. 校验判据与后端 `parse_trigger` **同规则**（否则前端放过去、后端拒绝，
//      用户看到的是"点了没反应"）；
//   2. 触发方式的**可读化**正确（"每 1 小时" 而不是 "3600 秒"）；
//   3. 面板在空态/错误态下不伪造数据、不渲染假行。

import { describe, expect, it, vi, beforeEach, afterEach } from "vitest"
import { render, screen, waitFor } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { AutomationPanel, describeTrigger, validateDraft } from "../panels/AutomationPanel"

const invokeMock = vi.fn()
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...a: unknown[]) => invokeMock(...a) }))

beforeEach(() => {
  invokeMock.mockReset()
})
afterEach(() => {
  vi.restoreAllMocks()
})

describe("validateDraft —— 与后端 parse_trigger 同规则", () => {
  it("every 拒绝小于 300 秒（防误填把库打爆）", () => {
    expect(validateDraft("every", "299")).toEqual({ ok: false, reason: "间隔不得小于 300 秒" })
    expect(validateDraft("every", "300")).toEqual({ ok: true, value: 300 })
  })

  it("after 拒绝 0 与负数", () => {
    expect(validateDraft("after", "0").ok).toBe(false)
    expect(validateDraft("after", "-5").ok).toBe(false)
    expect(validateDraft("after", "60")).toEqual({ ok: true, value: 60 })
  })

  it("at 拒绝过去时刻", () => {
    const past = Date.now() - 60_000
    expect(validateDraft("at", String(past)).ok).toBe(false)
    const future = Date.now() + 3_600_000
    expect(validateDraft("at", String(future)).ok).toBe(true)
  })

  it("非数字与小数被拒（不静默取整）", () => {
    expect(validateDraft("every", "abc").ok).toBe(false)
    expect(validateDraft("every", "3600.7").ok).toBe(false)
    // at/after 允许任何有限数；但空串不是数字
    expect(validateDraft("after", "").ok).toBe(false)
  })
})

describe("describeTrigger —— 说人话，不把换算成本丢给用户", () => {
  it("every 按小时/分钟归一", () => {
    expect(describeTrigger({ kind: "every", value: 3600 })).toBe("每 1 小时")
    expect(describeTrigger({ kind: "every", value: 7200 })).toBe("每 2 小时")
    expect(describeTrigger({ kind: "every", value: 1800 })).toBe("每 30 分钟")
    // 不能整除的退化为秒，但绝不留空
    expect(describeTrigger({ kind: "every", value: 301 })).toBe("每 301 秒")
  })

  it("after 直接给秒", () => {
    expect(describeTrigger({ kind: "after", value: 90 })).toBe("90 秒后")
  })

  it("at 给出绝对时间与相对差（含已过的情况）", () => {
    const now = 1_700_000_000_000
    const future = describeTrigger({ kind: "at", value: now + 3_600_000 }, now)
    expect(future).toContain("还有 1 小时")
    const past = describeTrigger({ kind: "at", value: now - 3_600_000 }, now)
    expect(past).toContain("已过")
  })
})

describe("AutomationPanel —— 状态穷举", () => {
  it("空态：显示引导，不渲染任何假任务行", async () => {
    invokeMock.mockResolvedValue({ items: [] })
    render(<AutomationPanel />)
    await waitFor(() => expect(screen.getByText(/还没有定时任务/)).toBeTruthy())
    expect(screen.queryAllByRole("listitem")).toHaveLength(0)
  })

  it("错误态：显示错误，不伪装成空态", async () => {
    invokeMock.mockRejectedValue(new Error("db locked"))
    render(<AutomationPanel />)
    await waitFor(() => expect(screen.getByRole("alert").textContent).toContain("db locked"))
    // 关键：错误时**不能**同时显示"还没有定时任务"——那是在骗用户
    expect(screen.queryByText(/还没有定时任务/)).toBeNull()
  })

  it("有数据：按 next_fire_ms 升序渲染，并显示可读触发方式", async () => {
    invokeMock.mockResolvedValue({
      items: [
        {
          id: "b", session: "desktop", message: "后触发的",
          trigger: { kind: "every", value: 7200 },
          created_at_ms: 1, next_fire_ms: 2000, fired: false,
        },
        {
          id: "a", session: "desktop", message: "先触发的",
          trigger: { kind: "every", value: 3600 },
          created_at_ms: 1, next_fire_ms: 1000, fired: false,
        },
      ],
    })
    render(<AutomationPanel />)
    await waitFor(() => expect(screen.getAllByRole("listitem")).toHaveLength(2))
    const items = screen.getAllByRole("listitem")
    // 最近的排前面：用户关心"下一步会发生什么"
    expect(items[0].textContent).toContain("先触发的")
    expect(items[0].textContent).toContain("每 1 小时")
  })

  it("创建：内容为空时不调用后端", async () => {
    invokeMock.mockResolvedValue({ items: [] })
    render(<AutomationPanel />)
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("cron_list"))

    const createBtn = screen.getByRole("button", { name: "创建" })
    await userEvent.click(createBtn)
    expect(invokeMock).not.toHaveBeenCalledWith("cron_create", expect.anything())
  })

  it("删除：后端返回 ok=false 时不谎报成功", async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "cron_list") {
        return Promise.resolve({
          items: [{
            id: "x", session: "desktop", message: "任务",
            trigger: { kind: "every", value: 3600 },
            created_at_ms: 1, next_fire_ms: 9e15, fired: false,
          }],
        })
      }
      if (cmd === "cron_remove") return Promise.resolve({ ok: false, id: "x" })
      return Promise.resolve({})
    })
    render(<AutomationPanel />)
    await waitFor(() => expect(screen.getAllByRole("listitem")).toHaveLength(1))

    await userEvent.click(screen.getByRole("button", { name: /删除 任务/ }))
    await waitFor(() =>
      expect(screen.getByRole("alert").textContent).toContain("删除未生效"),
    )
  })
})
