import { useState, memo } from "react"
import { invoke } from "@tauri-apps/api/core"
import { useTauriInvoke } from "../hooks/useTauriInvoke"
import { InlineNotice } from "../components/Notice"
import { btnGhostStyle, cardStyle, panelBox, panelSubtitle, panelTitle } from "../styles/panelStyles"

interface SessionRow {
  session_id: string
  summary: string
  message_count: number
  updated_at: string
}

function SessionsPanelImpl() {
  const { data, error, loading, run } = useTauriInvoke<SessionRow[]>("session_list");
  // 删除确认改为**行内二次确认**，不再用原生 `confirm()`：
  //   · confirm 在 Tauri（WebView2 / WKWebView）里样式随系统、不跟主题；
  //   · 阻塞式弹窗打断读屏，且只能靠指针完成；
  //   · 行内确认不需要焦点陷阱，也不会有"弹窗盖住内容"的层级问题。
  // 与 GitHub 一类产品的"点击 → 变成确认"模式一致。
  const [confirmingId, setConfirmingId] = useState<string | null>(null);
  const [deleteError, setDeleteError] = useState<string | null>(null);
  const [deletingId, setDeletingId] = useState<string | null>(null);

  const doDelete = async (id: string) => {
    setDeletingId(id);
    setDeleteError(null);
    try {
      await invoke("session_delete", { id });
      setConfirmingId(null);
      run();
    } catch (e) {
      setDeleteError("删除失败: " + (e instanceof Error ? e.message : String(e)));
    } finally {
      setDeletingId(null);
    }
  };

  return (
    <div style={panelBox}>
      <div style={panelTitle}>会话管理</div>
      <div style={panelSubtitle}>对齐 CLI: marguerite session list/delete — 历史会话管理</div>
      <button style={btnGhostStyle} onClick={() => run()}>刷新</button>
      {error && <InlineNotice tone="error" onRetry={() => run()}>读取会话失败：{error}</InlineNotice>}
      {deleteError && <InlineNotice tone="error">{deleteError}</InlineNotice>}
      {loading && <div style={{ fontSize: 12, color: "var(--text-secondary)", marginTop: 8 }}>加载中…</div>}
      {!loading && data?.length === 0 && (
        <div style={{ marginTop: 12, fontSize: 12, color: "var(--text-secondary)" }}>
          还没有历史会话。开始一次对话后会自动出现在这里。
        </div>
      )}
      {data?.map((s) => {
        const confirming = confirmingId === s.session_id;
        const busy = deletingId === s.session_id;
        return (
          <div key={s.session_id} style={{ ...cardStyle, display: "flex", alignItems: "center", gap: 10 }}>
            <div style={{ flex: 1, minWidth: 0 }}>
              <div style={{ fontWeight: 600, fontSize: 12, fontFamily: "monospace" }}>{s.session_id.slice(0, 12)}…</div>
              <div style={{ fontSize: 11, color: "var(--text-secondary)", overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
                {s.summary || "(无摘要)"}
              </div>
              <div style={{ fontSize: 10, color: "var(--text-tertiary, #555)" }}>{s.message_count} 条消息 · {s.updated_at}</div>
            </div>
            {confirming ? (
              <div style={{ display: "flex", gap: 6, alignItems: "center" }}>
                <span style={{ fontSize: 11, color: "var(--status-error, #ef4444)" }}>确认删除？</span>
                <button
                  style={{ ...btnGhostStyle, borderColor: "var(--status-error, #ef4444)", color: "var(--status-error, #ef4444)" }}
                  onClick={() => void doDelete(s.session_id)}
                  disabled={busy}
                  aria-label={`确认删除会话 ${s.session_id.slice(0, 8)}`}
                >
                  {busy ? "删除中…" : "确认"}
                </button>
                <button style={btnGhostStyle} onClick={() => setConfirmingId(null)} disabled={busy}>
                  取消
                </button>
              </div>
            ) : (
              <button
                style={{ ...btnGhostStyle, borderColor: "var(--status-error, #ef4444)", color: "var(--status-error, #ef4444)" }}
                onClick={() => { setConfirmingId(s.session_id); setDeleteError(null); }}
                aria-label={`删除会话 ${s.session_id.slice(0, 8)}`}
              >
                删除
              </button>
            )}
          </div>
        );
      })}
    </div>
  );
}


// === B3: React.memo 包装 — 父级无关状态变更不再触发本组件重渲染 ===
export const SessionsPanel = memo(SessionsPanelImpl);
