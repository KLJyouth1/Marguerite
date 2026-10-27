// 侧栏「顺序 / 置顶」的本地副本读取。
//
// 抽成独立模块（原先内联在 App.vue）是为了可测试：这里的 catch 曾经静默
// 吞掉损坏的 localStorage，用户保存的侧栏顺序会无声地变回默认值，看不出
// 是「读失败」还是「本来就没排过」。现在失败会走既有 toast 提示一次。
import { notifyReadFailure } from './readFailureNotice'

export function readStoredSessionKeys(storageKey: string): string[] {
  try {
    const parsed = JSON.parse(localStorage.getItem(storageKey) || '[]')
    if (!Array.isArray(parsed)) {
      notifyReadFailure(`sidebar-session-keys:${storageKey}`, 'shared.storage.sessionOrderReadFailed')
      return []
    }
    return [...new Set(parsed.filter((key): key is string => typeof key === 'string' && key.length > 0))]
      .slice(0, 1000)
  } catch {
    notifyReadFailure(`sidebar-session-keys:${storageKey}`, 'shared.storage.sessionOrderReadFailed')
    return []
  }
}
