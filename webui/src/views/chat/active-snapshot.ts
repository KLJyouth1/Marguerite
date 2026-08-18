// 纯函数：把项目工作区条目投影为快照结构。
// 从 views/ChatView.vue 的 `activeSnapshot` 抽出——无响应式依赖，仅由入参派生。
import type { ProjectWorkspaceItem } from '@/composables/useProjectWorkspaces'
import type { ActiveProjectWorkspaceSnapshot } from '@/composables/useActiveProjectWorkspace'

export function activeSnapshot(
  workspace: ProjectWorkspaceItem,
): ActiveProjectWorkspaceSnapshot {
  return {
    id: workspace.id,
    name: workspace.name,
    path: workspace.path,
    available: workspace.available,
    removed: false,
    ...(workspace.availabilityReason
      ? { availabilityReason: workspace.availabilityReason }
      : {}),
  }
}
