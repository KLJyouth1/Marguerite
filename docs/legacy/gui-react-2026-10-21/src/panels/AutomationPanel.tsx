// 自动化 / 定时任务面板。
//
// 为什么是"补"而不是"新造"（2026-10-21 审计）：桌面端此处原是
// `PlaceholderPanel`（"该模块正在开发中，敬请期待"），而 core 早就有
// `crate::schedule::ScheduleStore`（SQLite 持久化 Reminder CRUD），
// WebUI 也有完整 CronView。**能力一直存在，只是桌面端没接**——
// 这正是「Zero Demo」要清掉的那类占位。
//
// 数据源：`cron_list` / `cron_create` / `cron_remove` / `cron_run`（Tauri command），
// 它们与网关 `cron.*` 走**同一个** `ScheduleStore`：
// 桌面端建的任务，WebUI/CLI 也能看到，不存在两套账。
//
// 状态覆盖（缺一个就是不合格的界面）：加载中 / 空态 / 错误态 / 提交中 / 校验失败。

import { memo, useCallback, useEffect, useMemo, useState } from "react"
import { invoke } from "@tauri-apps/api/core"
import { btnGhostStyle, cardStyle, panelBox, panelSubtitle, panelTitle } from "../styles/panelStyles"

/** 触发方式（与 ReminderTrigger 的 wire 形态一致） */
type TriggerKind = "after" | "at" | "every"

interface CronTrigger {
  kind: TriggerKind
  /** after/every 为秒；at 为毫秒时间戳 */
  value: number
}

interface CronItem {
  id: string
  session: string
  message: string
  trigger: CronTrigger
  created_at_ms: number
  next_fire_ms: number
  fired: boolean
}

const KIND_LABEL: Record<TriggerKind, string> = {
  after: "延迟",
  at: "定时",
  every: "间隔",
}

/** 把触发方式说成人话。
 *
 * 为什么不直接显示原始秒数：`every: 3600` 要用户自己换算成"每小时"，
 * 这是把认知成本推给用户。这里给出可读文本，原始值放 title。 */
export function describeTrigger(t: CronTrigger, nowMs = Date.now()): string {
  if (t.kind === "after") {
    return `${t.value} 秒后`
  }
  if (t.kind === "every") {
    if (t.value % 3600 === 0) return `每 ${t.value / 3600} 小时`
    if (t.value % 60 === 0) return `每 ${t.value / 60} 分钟`
    return `每 ${t.value} 秒`
  }
  const dt = new Date(t.value)
  const diff = t.value - nowMs
  const abs = Math.abs(diff)
  const rel = abs < 60_000
    ? `${Math.max(1, Math.round(abs / 1000))} 秒`
    : abs < 3_600_000
      ? `${Math.round(abs / 60_000)} 分钟`
      : `${Math.round(abs / 3_600_000)} 小时`
  return `${diff >= 0 ? "" : "已过 "}${dt.toLocaleString()}（${diff >= 0 ? "还有 " : ""}${rel}${diff >= 0 ? "" : "前"}）`
}

/** 禁用态与提示：与后端 `parse_trigger` 的校验保持一致。
 *
 * 前端做一遍不是为了"信任前端"，而是让用户在点提交前就看到问题——
 * 后端仍然会独立校验（那条才是权威）。
 */
export function validateDraft(
  kind: TriggerKind,
  raw: string,
): { ok: true; value: number } | { ok: false; reason: string } {
  const n = Number(raw)
  if (!Number.isFinite(n)) return { ok: false, reason: "请输入数字" }
  if (kind === "at") {
    if (n <= Date.now()) return { ok: false, reason: "定时时刻必须是将来的时间" }
    return { ok: true, value: Math.trunc(n) }
  }
  if (!Number.isInteger(n)) return { ok: false, reason: "请填整数" }
  if (kind === "after") {
    if (n <= 0) return { ok: false, reason: "延迟秒数必须大于 0" }
    return { ok: true, value: n }
  }
  if (n < 300) return { ok: false, reason: "间隔不得小于 300 秒" }
  return { ok: true, value: n }
}

function AutomationPanelImpl() {
  const [items, setItems] = useState<CronItem[]>([])
  const [loading, setLoading] = useState(true)
  // 两类错误必须分开（实测教训）：
  //   · loadError  = 读列表失败 → 不能把列表渲染成"空态"，否则用户以为任务被删光了；
  //   · actionError = 删除/运行失败 → 不能因为随后 `load()` 重置 loadError 而消失。
  // 旧实现共用一个 error，且 load() 开头无条件 setError(null)，
  // 导致"删除未生效"的提示刚设置就被清掉（提示闪一下或根本不出现）。
  const [loadError, setLoadError] = useState<string | null>(null)
  const [actionError, setActionError] = useState<string | null>(null)
  const [busyId, setBusyId] = useState<string | null>(null)

  // 新建表单
  const [message, setMessage] = useState("")
  const [kind, setKind] = useState<TriggerKind>("every")
  const [raw, setRaw] = useState("3600")
  const [submitting, setSubmitting] = useState(false)
  const [formError, setFormError] = useState<string | null>(null)
  const [notice, setNotice] = useState<string | null>(null)

  const load = useCallback(async () => {
    setLoading(true)
    setLoadError(null)
    try {
      const res = await invoke<{ items: CronItem[] }>("cron_list")
      setItems(res.items ?? [])
    } catch (e) {
      // 读取失败 ≠ 没有任务。必须显示错误，不能渲染成空态。
      setLoadError(e instanceof Error ? e.message : String(e))
    } finally {
      setLoading(false)
    }
  }, [])

  useEffect(() => {
    void load()
  }, [load])

  const draft = useMemo(() => validateDraft(kind, raw), [kind, raw])

  const submit = async () => {
    setFormError(null)
    setNotice(null)
    if (!message.trim()) {
      setFormError("提醒内容不能为空")
      return
    }
    if (!draft.ok) {
      setFormError(draft.reason)
      return
    }
    setSubmitting(true)
    try {
      await invoke("cron_create", {
        session: "desktop",
        message: message.trim(),
        kind,
        value: draft.value,
      })
      setMessage("")
      setNotice("已创建")
      await load()
    } catch (e) {
      setFormError(e instanceof Error ? e.message : String(e))
    } finally {
      setSubmitting(false)
    }
  }

  const remove = async (id: string) => {
    setBusyId(id)
    setActionError(null)
    try {
      const res = await invoke<{ ok: boolean }>("cron_remove", { id })
      if (!res.ok) {
        // 后端说没删掉就别说成功——这正是"静默失败"的反面。
        // 用 actionError 而不是 loadError：随后的 load() 会清 loadError，
        // 若共用就会把这条提示吞掉（实测过）。
        setActionError("删除未生效（任务可能已被别处移除）")
      }
      await load()
    } catch (e) {
      setActionError(e instanceof Error ? e.message : String(e))
    } finally {
      setBusyId(null)
    }
  }

  const runNow = async (id: string) => {
    setBusyId(id)
    setActionError(null)
    try {
      const res = await invoke<{ ok: boolean; message: string; recurring: boolean }>("cron_run", { id })
      setNotice(`已触发：${res.message}${res.recurring ? "（周期任务，保留计划）" : "（一次性任务，已结束）"}`)
      await load()
    } catch (e) {
      setActionError(e instanceof Error ? e.message : String(e))
    } finally {
      setBusyId(null)
    }
  }

  const sorted = useMemo(
    () => [...items].sort((a, b) => a.next_fire_ms - b.next_fire_ms),
    [items],
  )

  return (
    <div style={panelBox}>
      <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center" }}>
        <div>
          <div style={panelTitle}>自动化</div>
          <div style={panelSubtitle}>
            定时任务 — 与 WebUI 的 cron、CLI 的 `marguerite cron` 共用同一份计划台账
          </div>
        </div>
        <button style={btnGhostStyle} onClick={() => void load()} disabled={loading}>
          {loading ? "读取中…" : "刷新"}
        </button>
      </div>

      {loadError && (
        <div
          role="alert"
          style={{ ...cardStyle, color: "var(--danger, #ef4444)", fontSize: 12, marginTop: 8 }}
        >
          读取任务失败：{loadError}
        </div>
      )}
      {actionError && (
        <div
          role="alert"
          style={{ ...cardStyle, color: "var(--danger, #ef4444)", fontSize: 12, marginTop: 8 }}
        >
          {actionError}
        </div>
      )}
      {notice && <div style={{ ...cardStyle, fontSize: 12, marginTop: 8 }} role="status">{notice}</div>}

      {/* ── 新建 ── */}
      <div style={{ ...cardStyle, marginTop: 12 }}>
        <div style={{ fontWeight: 600, fontSize: 12, marginBottom: 8 }}>新建任务</div>
        <label style={{ display: "block", fontSize: 11, color: "var(--text-secondary)" }}>
          提醒内容
          <input
            aria-label="提醒内容"
            value={message}
            onChange={(e) => setMessage(e.target.value)}
            placeholder="例如：检查今日部署结果"
            style={{
              display: "block", width: "100%", marginTop: 4, padding: "6px 8px",
              fontSize: 12, borderRadius: 6, border: "1px solid var(--surface-border)",
              background: "var(--surface-raised, transparent)", color: "inherit",
            }}
          />
        </label>

        <div style={{ display: "flex", gap: 8, marginTop: 8, flexWrap: "wrap", alignItems: "flex-end" }}>
          <label style={{ fontSize: 11, color: "var(--text-secondary)" }}>
            触发方式
            <select
              aria-label="触发方式"
              value={kind}
              onChange={(e) => {
                const k = e.target.value as TriggerKind
                setKind(k)
                // 切换类型时给一个合法的默认值：否则用户会被"上一次的秒数"
                // 悄悄校验失败，却看不出为什么。
                setRaw(k === "at" ? String(Date.now() + 3_600_000) : k === "after" ? "600" : "3600")
              }}
              style={{
                display: "block", marginTop: 4, padding: "6px 8px", fontSize: 12,
                borderRadius: 6, border: "1px solid var(--surface-border)",
                background: "var(--surface-raised, transparent)", color: "inherit",
              }}
            >
              <option value="every">间隔重复</option>
              <option value="after">延迟一次</option>
              <option value="at">指定时刻</option>
            </select>
          </label>

          <label style={{ fontSize: 11, color: "var(--text-secondary)", flex: "1 1 200px" }}>
            {kind === "at" ? "时刻（毫秒时间戳）" : kind === "after" ? "延迟（秒）" : "间隔（秒，≥300）"}
            <input
              aria-label="触发参数"
              value={raw}
              onChange={(e) => setRaw(e.target.value)}
              inputMode="numeric"
              style={{
                display: "block", width: "100%", marginTop: 4, padding: "6px 8px",
                fontSize: 12, borderRadius: 6,
                border: `1px solid ${draft.ok ? "var(--surface-border)" : "var(--danger, #ef4444)"}`,
                background: "var(--surface-raised, transparent)", color: "inherit",
              }}
            />
          </label>

          <button
            style={btnGhostStyle}
            onClick={() => void submit()}
            disabled={submitting || !draft.ok}
            aria-disabled={submitting || !draft.ok}
          >
            {submitting ? "创建中…" : "创建"}
          </button>
        </div>

        {!draft.ok && (
          <div style={{ fontSize: 11, color: "var(--danger, #ef4444)", marginTop: 6 }}>{draft.reason}</div>
        )}
        {formError && (
          <div role="alert" style={{ fontSize: 11, color: "var(--danger, #ef4444)", marginTop: 6 }}>
            {formError}
          </div>
        )}
      </div>

      {/* ── 列表 ── */}
      <div style={{ ...cardStyle, marginTop: 12 }}>
        <div style={{ fontWeight: 600, fontSize: 12, marginBottom: 8 }}>
          任务列表{items.length > 0 ? `（${items.length}）` : ""}
        </div>

        {loading && items.length === 0 ? (
          // 骨架：固定高度，避免加载完成时列表跳动
          <div aria-busy="true" style={{ display: "grid", gap: 6 }}>
            {[0, 1, 2].map((i) => (
              <div
                key={i}
                style={{ height: 34, borderRadius: 6, background: "var(--surface-border)", opacity: 0.4 }}
              />
            ))}
          </div>
        ) : loadError ? (
          // 读取失败时**不能**显示"还没有定时任务"：那是把错误伪装成空态，
          // 用户会以为任务被删光了。这里给重试出口。
          <div style={{ fontSize: 12, color: "var(--text-secondary)" }}>
            任务列表读取失败，无法判断当前是否有任务。
            <button style={{ ...btnGhostStyle, marginLeft: 8 }} onClick={() => void load()}>
              重试
            </button>
          </div>
        ) : items.length === 0 ? (
          <div style={{ fontSize: 12, color: "var(--text-secondary)" }}>
            还没有定时任务。上面创建一个，或用 CLI `marguerite cron add`。
          </div>
        ) : (
          <ul style={{ listStyle: "none", margin: 0, padding: 0, display: "grid", gap: 6 }}>
            {sorted.map((it) => (
              <li
                key={it.id}
                style={{
                  display: "grid",
                  gridTemplateColumns: "1fr auto",
                  gap: 8,
                  alignItems: "center",
                  padding: "6px 8px",
                  borderRadius: 6,
                  border: "1px solid var(--surface-border)",
                }}
              >
                <div style={{ minWidth: 0 }}>
                  <div
                    style={{
                      fontSize: 13,
                      overflow: "hidden",
                      textOverflow: "ellipsis",
                      whiteSpace: "nowrap",
                    }}
                    title={it.message}
                  >
                    {it.message}
                  </div>
                  <div style={{ fontSize: 11, color: "var(--text-tertiary)" }}>
                    <span title={`原始：${it.trigger.kind}=${it.trigger.value}`}>
                      {KIND_LABEL[it.trigger.kind]} · {describeTrigger(it.trigger)}
                    </span>
                    {it.fired && <span> · 已触发</span>}
                    <span> · {it.session}</span>
                  </div>
                </div>
                <div style={{ display: "flex", gap: 6 }}>
                  <button
                    style={btnGhostStyle}
                    onClick={() => void runNow(it.id)}
                    disabled={busyId === it.id}
                    aria-label={`立即运行 ${it.message}`}
                  >
                    运行
                  </button>
                  <button
                    style={btnGhostStyle}
                    onClick={() => void remove(it.id)}
                    disabled={busyId === it.id}
                    aria-label={`删除 ${it.message}`}
                  >
                    删除
                  </button>
                </div>
              </li>
            ))}
          </ul>
        )}
      </div>
    </div>
  )
}

// === React.memo 包装 — 与本目录其余面板保持一致的渲染纪律 ===
export const AutomationPanel = memo(AutomationPanelImpl)
