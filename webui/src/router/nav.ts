import { getPlatform } from '@/platform'
import type { PlatformId } from '@/platform'
import type { RouteRecordRaw } from 'vue-router'
import type { IconName } from '@/utils/icons'
import i18n from '@/i18n'
import { desktopRoutes } from './desktopRoutes'
import { sharedRoutes } from './sharedRoutes'
import { webRoutes } from './webRoutes'

type NavigationSlot = 'primary' | 'bottom'

export interface NavigationItem {
  path: string
  title: string
  icon: IconName
}

// Semantic navigation groups (Task #6). Core task-flow destinations stay in the
// flat primary rail; everything else is bucketed into one of these collapsible
// sections so deep-link capabilities (engine health, editor, terminal, settings
// subsystems…) become discoverable without bloating the rail.
export type NavGroupId = 'Work' | 'Operate' | 'Observe' | 'Settings' | 'Extras'

// Unified three-tier grouping, shared with the GUI sidebar (核心 / 治理 / 实验室):
// - core      : Work + Operate   → 核心（对话/会话/沙箱/文件/自动化/操作类执行）
// - governance: Observe + Settings → 治理（诊断/能力/路由/统计/设置）
// - lab       : Extras           → 实验室（德州扑克/鲸鱼娘/视觉取证等玩具，默认折叠）
export type NavCategory = 'core' | 'governance' | 'lab'

export const NAV_CATEGORY: Record<NavGroupId, NavCategory> = {
  Work: 'core',
  Operate: 'core',
  Observe: 'governance',
  Settings: 'governance',
  Extras: 'lab',
}

export const CATEGORY_ORDER: NavCategory[] = ['core', 'governance', 'lab']

// i18n keys for the three-tier category names; navCategoryTitle() falls back to
// the flat id when a locale key is missing.
export const CATEGORY_TITLES: Record<NavCategory, string> = {
  core: 'nav.categoryCore',
  governance: 'nav.categoryGovernance',
  lab: 'nav.categoryLab',
}

export interface NavGroup {
  id: NavGroupId
  category: NavCategory
  title: string
  items: NavigationItem[]
}

// Stable display order of the collapsible groups.
const GROUP_ORDER: NavGroupId[] = ['Work', 'Operate', 'Observe', 'Settings', 'Extras']

// Groups whose members are reachable as primary nav items get surfaced as a
// dedicated section even when empty of secondary links (Work always shows the
// primary rail items; the rest only render when they contain deep links).
const GROUP_TITLES: Record<NavGroupId, string> = {
  Work: 'nav.groupWork',
  Operate: 'nav.groupOperate',
  Observe: 'nav.groupObserve',
  Settings: 'nav.groupSettings',
  Extras: 'nav.groupExtras',
}

function navGroupTitle(id: NavGroupId): string {
  const key = GROUP_TITLES[id]
  const translated = i18n.global.t(key)
  return translated !== key ? translated : id
}

const navRoutes = [
  ...sharedRoutes,
  ...webRoutes,
  ...desktopRoutes,
]

function routePlatforms(platforms: unknown): PlatformId[] {
  if (!Array.isArray(platforms)) return ['web', 'desktop']
  return platforms.filter((item): item is PlatformId => item === 'web' || item === 'desktop')
}

// Localize a nav row title from its route name token (e.g. `nav.sessions`),
// falling back to the English meta.title literal when no key exists. Called
// inside the useNavigation() computeds, so reading the reactive i18n locale here
// makes the rail/drawer/palette re-render on a language switch.
function navTitle(route: RouteRecordRaw): string {
  const explicitKey = route.meta?.navLabelKey
  if (explicitKey) {
    const translated = i18n.global.t(explicitKey)
    if (translated !== explicitKey) return translated
  }
  const name = typeof route.name === 'string' ? route.name : ''
  if (name) {
    const key = `nav.${name}`
    const translated = i18n.global.t(key)
    if (translated !== key) return translated
  }
  return String(route.meta?.title || route.name || route.path)
}

export function getNavigationItems(slot: NavigationSlot): NavigationItem[] {
  const platform = getPlatform()
  return navRoutes
    .filter((route) => route.meta?.nav === slot)
    .filter((route) => routePlatforms(route.meta?.platforms).includes(platform.id))
    .sort((a, b) => Number(a.meta?.navOrder || 0) - Number(b.meta?.navOrder || 0))
    .map((route) => ({
      path: route.path,
      title: navTitle(route),
      icon: (route.meta?.icon || 'home') as IconName,
    }))
}

// The flat, always-visible destinations shared by the desktop rail, mobile
// drawer, and command palette. Chat is excluded because the dedicated New-chat
// action owns that destination.
export function getWorkNavigationSection(): NavigationItem[] {
  const groupOf = new Map(
    navRoutes
      .filter((route) => route.meta?.nav === 'primary')
      .map((route) => [route.path, route.meta?.group ?? 'Operate']),
  )
  return getNavigationItems('primary').filter(
    (item) => groupOf.get(item.path) === 'Work' && item.path !== '/chat',
  )
}

// Collapsible secondary navigation (Task #6). Buckets every route that declares
// a `group` into Work / Operate / Observe / Settings sections, ordered by
// `navOrder` inside each group. The flat primary rail (getWorkNavigationSection)
// owns the Work-group primary items; this function surfaces the *remaining*
// grouped destinations — deep-link capabilities (engine health, editor,
// terminal, settings subsystems…) that were previously unreachable from the
// sidebar. The dedicated Settings footer entry and Chat are excluded.
export function getGroupedNavigation(): NavGroup[] {
  const platform = getPlatform()
  const buckets: Record<NavGroupId, NavigationItem[]> = {
    Work: [],
    Operate: [],
    Observe: [],
    Settings: [],
    Extras: [],
  }
  const skip = new Set(['/chat', '/settings', '/settings/:section'])
  // navRoutes merges sharedRoutes + webRoutes + desktopRoutes, so a path may
  // appear more than once (e.g. /logs in both). Prefer the occurrence that
  // carries an explicit `group` so the canonical grouping wins over a bare
  // duplicate (which would otherwise default to Operate).
  const canonical = new Map<string, (typeof navRoutes)[number]>()
  for (const route of navRoutes) {
    const meta = route.meta
    if (!meta) continue
    const path = route.path
    if (skip.has(path)) continue
    const existing = canonical.get(path)
    const hasGroup = meta.group !== undefined
    if (!existing || (hasGroup && existing.meta?.group === undefined)) {
      canonical.set(path, route)
    }
  }
  for (const route of canonical.values()) {
    const meta = route.meta!
    const path = route.path
    // A4 修正：**未声明 `meta.group` 的路由不进任何分组**，而不是默默落进
    // Operate。旧实现 `meta.group ?? 'Operate'` 把两类路由塞进了「运营」：
    //   ① 契约上不该出现在侧栏的（/pet-overlay —— 桌面悬浮窗，注释明确写明
    //      「不设置 meta.group：R5 契约保证不出现于侧栏」）；真机实测它却以
    //      「Marguerite Pet」出现在运营组里，违背契约；
    //   ② 该由别处承载的 hub 路由（/overview、/channels —— 分别由 /usage、
    //      /skills 的 hub 页承载）。
    // 现在只有显式分组的路由才可见，语义分组因此可被逐条审计。
    if (meta.group === undefined) continue
    const group = meta.group as NavGroupId
    if (!(group in buckets)) continue
    // Work-group primary items are owned by the flat rail; everything else
    // (including non-Work primary items such as /engine, /editor, /terminal)
    // lands in its group section.
    if (group === 'Work' && meta.nav === 'primary') continue
    if (!routePlatforms(meta.platforms).includes(platform.id)) continue
    buckets[group].push({
      path,
      title: navTitle(route),
      icon: (meta.icon || 'home') as IconName,
    })
  }
  // Sort each bucket by the route's navOrder so its display matches the rail.
  const orderOf = new Map(
    navRoutes.map((route) => [route.path, Number(route.meta?.navOrder || 0)]),
  )
  for (const id of GROUP_ORDER) {
    buckets[id].sort((a, b) => (orderOf.get(a.path) ?? 0) - (orderOf.get(b.path) ?? 0))
  }
  return GROUP_ORDER.filter((id) => buckets[id].length > 0).map((id) => ({
    id,
    category: NAV_CATEGORY[id],
    title: navGroupTitle(id),
    items: buckets[id],
  }))
}
