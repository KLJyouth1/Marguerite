// 读取失败的用户可见提示（W8.3 静默失败清理）。
//
// 为什么统一在这里：`catch { return [] }` 会把「本地存储损坏 / 网关离线」
// 伪装成「这里本来就没有数据」—— 用户看到空白列表却无从判断是该刷新、
// 该重连，还是数据真的为空。能恢复的读取失败因此用既有 toast 界面提示。
//
// 采样：同一 `source` 每页只提示一次。这些读取可能挂在轮询 / 会话切换上，
// 不采样会把提示栏刷满，反而盖住真正要读的内容。
import { t } from '@/i18n'
import { useToasts } from '@/composables/useToasts'

const notified = new Set<string>()

export function notifyReadFailure(source: string, messageKey: string): void {
  if (notified.has(source)) return
  notified.add(source)
  try {
    useToasts().pushToast(t(messageKey), { tone: 'warn' })
  } catch {
    // 提示通道自身失败绝不能反过来打断读取路径。
  }
}

/** 仅测试：清空采样表，让同一来源可以再次提示。 */
export function _resetReadFailureNoticeForTests(): void {
  notified.clear()
}
