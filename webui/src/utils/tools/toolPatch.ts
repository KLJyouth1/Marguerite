/**
 * 工具面 patch 层的纯函数（与后端 `tools.patch.get/set` 契约对应）。
 *
 * 抽出来是为了锁住一个真实不变量：**切换一个工具时绝不能丢掉其它已停用的工具**。
 * 补丁是「整体覆盖」语义（`tools.patch.set` 接收完整 disabled 数组），一旦本地集合
 * 算错，用户上一次禁用的工具会被悄悄放回来——而这种错误在 UI 上完全看不出来。
 */

/** 目录项（`tools.catalog` 的 items；`enabled` 缺省视为启用，兼容旧后端）。 */
export interface ToolCatalogItem {
  name: string
  enabled?: boolean
}

/**
 * 计算切换 `name` 之后应提交的 disabled 数组。
 *
 * @param current 当前所有已停用的工具名（来自后端或本地同步后的状态）
 * @param name    本次切换的工具名
 * @param enabled 切换**之后**该工具是否启用
 */
export function computeNextDisabled(
  current: readonly string[],
  name: string,
  enabled: boolean,
): string[] {
  const next = new Set(current)
  if (enabled) next.delete(name)
  else next.add(name)
  return [...next].sort()
}

/** 用后端返回的 disabled 集合重算目录项的 `enabled`（不修改入参）。 */
export function applyDisabled<T extends ToolCatalogItem>(
  items: readonly T[],
  disabled: readonly string[],
): T[] {
  const off = new Set(disabled)
  return items.map(item => ({ ...item, enabled: !off.has(item.name) }))
}

/** 目录里当前被停用的工具名（用于切换时构造整体覆盖的 disabled 数组）。 */
export function disabledNames(items: readonly ToolCatalogItem[]): string[] {
  return items.filter(item => item.enabled === false).map(item => item.name)
}
