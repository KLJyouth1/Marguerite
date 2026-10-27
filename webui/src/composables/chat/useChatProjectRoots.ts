// 会话级「附加项目」状态（P2-9 多根的用户可见入口）。
//
// 为什么要单独抽一个组合式函数：附加项目是**会话级**状态，ChatView（发送装配、
// 会话切换）与 ChatComposer（chips、多选弹窗）必须读写同一份数据。两个组件各
// 自维护一份 ref 时，切换会话后 composer 会继续显示上一个会话的附加项目 —— 这是
// 多根能力最容易被用户看见的串会话缺陷。
//
// 因此这里采用**模块级单例**（与 `useProjectWorkspaces` 一致）：任何组件调用
// `useChatProjectRoots()` 都拿到同一组 ref；生命周期由 ChatView 在会话边界上
// 调用 `resetForSession` 统一重置。
//
// 状态只存 workspace **id**：`chat.send` 的 `workspaceIds` 参数按 id 解析路径，
// 路径字符串既不是稳定标识，也会在 Windows 大小写/分隔符上出现假不等。
import { ref } from 'vue'
import type { ProjectWorkspaceItem } from '@/types/rpc'

/** composer 上展示的一枚附加项目 chip。 */
export interface ChatProjectRoot {
  id: string
  name: string
  path: string
  available: boolean
}

const extraIds = ref<string[]>([])

/** 路径归一化（仅用于比较）：统一分隔符、去掉尾部分隔符；Windows 忽略大小写。 */
export function normalizeProjectRootPath(path: string): string {
  const trimmed = String(path || '').trim().replace(/\\/g, '/').replace(/\/+$/, '')
  if (!trimmed) return ''
  return /^[A-Za-z]:/.test(trimmed) || trimmed.startsWith('//')
    ? trimmed.toLowerCase()
    : trimmed
}

/** 去空、去重、保序。 */
export function uniqueProjectRootIds(ids: readonly string[]): string[] {
  const seen = new Set<string>()
  const result: string[] = []
  for (const raw of ids) {
    const id = String(raw || '').trim()
    if (!id || seen.has(id)) continue
    seen.add(id)
    result.push(id)
  }
  return result
}

/**
 * 会话 `projectPaths` → 附加项目 id 列表。
 *
 * 约定：`projectPaths[0]` 是主根（也是 `workspace` 单值字段），**不进入附加集合**；
 * 其余路径按工作区清单的 path 反查 id（归一化比较）。反查不到的路径直接跳过 ——
 * 后端只认 id，凭路径无法绑定，塞进去只会在发送时被静默丢弃，反而误导用户。
 */
export function projectRootIdsFromPaths(
  paths: readonly string[] | null | undefined,
  workspaces: readonly ProjectWorkspaceItem[],
): string[] {
  const normalizedPaths = (paths || [])
    .map(path => String(path || '').trim())
    .filter(Boolean)
  if (normalizedPaths.length <= 1) return []
  const byPath = new Map<string, string>()
  for (const workspace of workspaces) {
    const key = normalizeProjectRootPath(workspace.path)
    if (key && !byPath.has(key)) byPath.set(key, workspace.id)
  }
  const ids: string[] = []
  for (const path of normalizedPaths.slice(1)) {
    const id = byPath.get(normalizeProjectRootPath(path))
    if (id) ids.push(id)
  }
  return uniqueProjectRootIds(ids)
}

/** 附加项目 id → composer chip（按工作区清单补全名称/路径/可用性）。 */
export function projectRootChips(
  ids: readonly string[],
  workspaces: readonly ProjectWorkspaceItem[],
): ChatProjectRoot[] {
  const byId = new Map(workspaces.map(workspace => [workspace.id, workspace]))
  const chips: ChatProjectRoot[] = []
  for (const id of uniqueProjectRootIds(ids)) {
    const workspace = byId.get(id)
    if (!workspace) continue
    chips.push({
      id: workspace.id,
      name: workspace.name,
      path: workspace.path,
      available: workspace.available,
    })
  }
  return chips
}

export function useChatProjectRoots() {
  /**
   * 会话边界重置：切换会话、进入草稿、草稿落库后都会调用。
   * 先清空再等会话元数据回填，保证不会把上一个会话的附加项目带进新会话。
   */
  function resetForSession() {
    extraIds.value = []
  }

  /** 会话元数据（`sessions.list` / 订阅快照里的 projectPaths）回填附加项目。 */
  function applyProjectPaths(
    paths: readonly string[] | null | undefined,
    workspaces: readonly ProjectWorkspaceItem[],
  ) {
    extraIds.value = projectRootIdsFromPaths(paths, workspaces)
  }

  /** 用户在多选弹窗里确认的附加项目集合（主根由对话上下文决定，这里不接受）。 */
  function applySelection(ids: readonly string[], primaryId?: string | null) {
    const primary = String(primaryId || '').trim()
    extraIds.value = uniqueProjectRootIds(ids).filter(id => id !== primary)
  }

  function removeExtraId(id: string) {
    extraIds.value = extraIds.value.filter(candidate => candidate !== id)
  }

  return {
    extraIds,
    resetForSession,
    applyProjectPaths,
    applySelection,
    removeExtraId,
  }
}
