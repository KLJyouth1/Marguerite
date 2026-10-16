import { computed, ref } from 'vue'
import { getNavigationItems, getWorkNavigationSection, getGroupedNavigation } from '@/router/nav'

// v3 侧边栏精简：次要分组（Observe/Settings/Extras）默认折叠，只保留 Work/Operate
// 展开。用户手动展开/折叠后按组记忆（localStorage），刷新后保持其选择。
const SIDEBAR_COLLAPSE_KEY = 'marguerite.sidebar.collapsedGroups.v1'
// 实验室（Extras）以及 Observe/Settings 治理子组默认折叠；主 rail 只保留核心工作流。
// 导出该常量供 App.sidebar-contract.test.ts 固化「实验室默认折叠」契约。
export const DEFAULT_COLLAPSED: readonly string[] = ['Observe', 'Settings', 'Extras']

function loadCollapsedGroups(): Set<string> {
  try {
    const raw = localStorage.getItem(SIDEBAR_COLLAPSE_KEY)
    if (raw) {
      const parsed = JSON.parse(raw) as unknown
      if (Array.isArray(parsed)) return new Set(parsed as string[])
    }
  } catch {
    // 首次运行 / 存储不可用 → 用默认值
  }
  return new Set(DEFAULT_COLLAPSED)
}

function persistCollapsedGroups(groups: Set<string>) {
  try {
    localStorage.setItem(SIDEBAR_COLLAPSE_KEY, JSON.stringify([...groups]))
  } catch {
    // 忽略持久化失败：会话内仍生效
  }
}

export function useNavigation() {
  const bottomRoutes = computed(() => getNavigationItems('bottom'))
  // Flat primary rows, single-sourced from route metadata so the desktop rail,
  // mobile drawer, and command palette stay in the same order.
  const workNav = computed(() => getWorkNavigationSection())
  // Collapsible secondary sections (Task #6): deep-link capabilities bucketed
  // into Work / Operate / Observe / Settings. Collapse state is per-group and
  // persists across re-renders within the session.
  const groupedNav = computed(() => getGroupedNavigation())
  const collapsedGroups = ref<Set<string>>(loadCollapsedGroups())

  function toggleGroup(id: string) {
    const next = new Set(collapsedGroups.value)
    if (next.has(id)) next.delete(id)
    else next.add(id)
    collapsedGroups.value = next
    persistCollapsedGroups(next)
  }

  return {
    bottomRoutes,
    workNav,
    groupedNav,
    collapsedGroups,
    toggleGroup,
  }
}
