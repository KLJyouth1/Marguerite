/**
 * 回合结束桌面通知（turn-ended）。
 *
 * # 为什么需要
 * 长任务（深度推理、批量执行、多轮工具链）动辄几分钟。用户会切去干别的，
 * 回来才发现早跑完了；或者一直盯着等。ChatGPT / Codex 桌面端都有这条。
 *
 * # 三条设计判据（都不是随手定的）
 *
 * 1. **只在窗口不聚焦时发**。用户正看着屏幕还弹通知，等于噪音；这也是
 *    ChatGPT / Codex 的通行做法。判据用 `document.hasFocus()` +
 *    `visibilityState`，两者任一表明"用户在看"就跳过。
 * 2. **中止（aborted）不发**。用户主动点了停止，说明他就在旁边，
 *    再弹一条"任务结束"是多余的。
 * 3. **可关，且记住**。偏好存 localStorage —— 不引 store 是因为这是纯
 *    终端行为偏好，与业务状态无关，放在 store 里会污染持久化契约。
 *
 * # 为什么走 Tauri command 而不是 Web Notification API
 * webui 是纯浏览器包，没有 `@tauri-apps/api`（静态 import 会让 vite build 失败）。
 * Web Notification API 在 WebView2 / WKWebView 上行为不一致，窗口最小化时
 * 可能静默失败，也进不了系统通知中心。所以通知由壳内 Rust 侧发，
 * 这里只负责"该不该发"的判断。
 *
 * # 契约
 * `getTauriInvoke()` 返回 `undefined`（浏览器形态）时本 composable 完全静默，
 * 不发起任何必然失败的调用，也不在 UI 上留下痕迹。
 */

import { onBeforeUnmount, ref, watch } from 'vue'
import { getTauriInvoke } from '@/utils/tauriBridge'

const STORAGE_KEY = 'marguerite.notify.turnEnded'

/** 偏好：默认开启（桌面端用户普遍希望长任务结束时被告知）。 */
export function loadTurnNotifyEnabled(): boolean {
  try {
    const raw = localStorage.getItem(STORAGE_KEY)
    // 缺省（从未设置）视为开启；只有显式存了 '0' 才算关闭。
    return raw === null ? true : raw !== '0'
  } catch {
    // localStorage 被禁用时按默认开启处理：宁可多一条通知，也不要静默失效。
    return true
  }
}

export function saveTurnNotifyEnabled(enabled: boolean): void {
  try {
    localStorage.setItem(STORAGE_KEY, enabled ? '1' : '0')
  } catch {
    /* 存不进去只影响下次启动的默认值，不影响本次会话 */
  }
}

/** 通知正文：截断过长回复，避免系统通知被撑成一屏。 */
export function buildTurnNotificationBody(
  text: string,
  reason?: string,
): string {
  const flat = String(text || '')
    .replace(/\s+/g, ' ')
    .trim()
  if (!flat) {
    // 空回复也要说清状态，不能只发一个空通知让用户猜
    return reason === 'aborted' ? '回合已中止' : '回合已结束（无文本输出）'
  }
  const LIMIT = 120
  return flat.length > LIMIT ? `${flat.slice(0, LIMIT)}…` : flat
}

/**
 * 判断此刻是否该发通知。
 *
 * 抽成纯函数是为了可测：`hasFocus` / `visibility` 在 happy-dom 里可控，
 * 不必去 mock 整个 document。
 */
export function shouldNotifyTurnEnded(input: {
  enabled: boolean
  isDesktop: boolean
  hasFocus: boolean
  visibility: string
  reason?: string
}): { ok: boolean; why: string } {
  if (!input.isDesktop) return { ok: false, why: 'not-desktop' }
  if (!input.enabled) return { ok: false, why: 'disabled' }
  if (input.reason === 'aborted') return { ok: false, why: 'aborted' }
  // 「用户在看」= 有焦点 **且** 页面可见。两个条件必须同时成立才算。
  //
  // 为什么是 AND 不是 OR（这是修过的一个真 bug）：最初写成
  // `hasFocus || visibility === 'visible'`，于是「窗口失焦但没最小化」
  // （切去回微信 / 看文档 —— 最常见的情形）因为 visibility 仍是 'visible'
  // 而**永远不通知**，功能等于只在最小化时才生效。
  //
  // 现在的语义：
  //   · 聚焦 + 可见        → 不发（他就在看）
  //   · 失焦 + 可见        → 发（人走了，窗口还开着）
  //   · 任意 + 隐藏/最小化 → 发
  if (input.hasFocus && input.visibility === 'visible') {
    return { ok: false, why: 'user-present' }
  }
  return { ok: true, why: 'notify' }
}

export interface TurnNotificationOptions {
  /** 回合结束时的正文（已生成的助手文本） */
  text: string
  /** `chat.done` 载荷里的 reason；'aborted' 表示用户主动中止 */
  reason?: string
}

/**
 * 发送一条"回合结束"桌面通知。
 *
 * 失败**不抛**：通知是锦上添花，发不出去不该影响主流程，也不该让用户
 * 看到一个和"任务已完成"矛盾的报错。仅返回 false 供调用方记日志。
 */
export async function notifyTurnEnded(
  opts: TurnNotificationOptions,
): Promise<boolean> {
  const invoke = getTauriInvoke()
  const decision = shouldNotifyTurnEnded({
    enabled: loadTurnNotifyEnabled(),
    isDesktop: Boolean(invoke),
    hasFocus: typeof document !== 'undefined' ? document.hasFocus() : false,
    visibility: typeof document !== 'undefined' ? document.visibilityState : 'visible',
    reason: opts.reason,
  })
  if (!decision.ok || !invoke) return false

  try {
    await invoke('notify_turn_ended', {
      title: 'Marguerite · 回合完成',
      body: buildTurnNotificationBody(opts.text, opts.reason),
    })
    return true
  } catch {
    // 系统级通知被禁用 / 权限被拒 / 插件缺失 —— 都不该冒泡成用户可见错误
    return false
  }
}

/**
 * 响应式开关（供设置页绑定）。
 *
 * 用 `watch` 而不是直接读写：设置页的 checkbox 与 localStorage 之间
 * 需要单向可信来源，避免"改了没生效"。
 */
export function useTurnNotifyPreference() {
  const enabled = ref(loadTurnNotifyEnabled())
  watch(enabled, (v) => saveTurnNotifyEnabled(v))
  return { enabled }
}

/** 组件卸载时可用于取消未完成的通知（当前实现即时发出，保留接口以备扩展）。 */
export function useTurnNotifyCleanup() {
  let disposed = false
  onBeforeUnmount(() => {
    disposed = true
  })
  return () => disposed
}
