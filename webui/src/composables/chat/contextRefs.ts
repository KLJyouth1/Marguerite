import { ref } from 'vue'

/**
 * 本轮「@」上下文引用。
 *
 * 为什么用模块级单例而不是 props 逐层传递：引用由 `ChatComposer`（输入框内）
 * 写入、由 `useChatSend`（发送链路）读取，两者不在同一条 props 链上；
 * 用共享 ref 可以避免把状态穿过 ChatView 的十几个字段。
 *
 * 生命周期：选中即加入 → 发送时快照进 `chat.send` → 发送成功后清空。
 * 发送失败保留，便于用户重试（重试走同一份 `params`）。
 */
export interface ContextRefItem {
  /** file / folder / rule / session —— 与后端 `context.references` 的分组 id 一致。 */
  kind: string
  /** 文件为相对路径，规则/会话为其 id。 */
  id: string
  /** 输入框里展示的短名。 */
  label: string
  /** 绝对路径（仅文件/目录）。 */
  path?: string
}

const items = ref<ContextRefItem[]>([])

export function useContextRefs() {
  return items
}

export function addContextRef(item: ContextRefItem) {
  const dup = items.value.some((x) => x.kind === item.kind && x.id === item.id)
  if (!dup) items.value = [...items.value, item]
}

export function removeContextRef(kind: string, id: string) {
  items.value = items.value.filter((x) => !(x.kind === kind && x.id === id))
}

export function clearContextRefs() {
  items.value = []
}

/** 发送时快照（避免异步过程中用户继续增删导致负载与 UI 不一致）。 */
export function snapshotContextRefs(): ContextRefItem[] {
  return items.value.map((x) => ({ ...x }))
}
