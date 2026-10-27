import { describe, expect, it } from 'vitest'
import { getNavigationItems, getWorkNavigationSection, getGroupedNavigation } from './nav'
import { sharedRoutes } from './sharedRoutes'

// Guards the flat route taxonomy shared by the sidebar rail, mobile drawer,
// and command palette. Long-lived Agent administration remains a direct route,
// but is intentionally outside this primary navigation source.

describe('getWorkNavigationSection', () => {
  it('returns the flat sidebar order after the dedicated chat action', () => {
    // v11 导航锐减：主 rail 保留核心工作流；Gap4 起 /task（交付流水线工作台）晋升为
    // 一等 Work 目的地（navOrder 35，紧邻 /skills），evolve/plugins 归 Work 折叠组，
    // 扑克/游戏/桌宠/壁纸等娱乐功能全部隔离到 Extras 扩展组。
    // v3 岛屿工作空间：/island（navOrder 37）作为 /task 的可视化孪生视图进入主 rail ——
    // 两者回答同一个问题（「任务现在怎么在跑」），只是呈现方式不同（流水线 vs 3D 岛屿）。
    expect(getWorkNavigationSection().map((item) => item.path)).toEqual([
      '/task',
      '/island',
      // 「我的文件」（产物中心，navOrder 38）：跨会话聚合产物，与 /task 互补。
      '/artifacts',
      '/skills',
      '/cron',
      '/usage',
    ])
  })

  it('excludes Chat, the task ledger, and advanced Agent administration', () => {
    const paths = getWorkNavigationSection().map((item) => item.path)
    expect(paths).not.toContain('/chat')
    // Sessions stays routed for deep links and the Not Found fallback, but
    // "New task" leads the sidebar and the recents list covers session access.
    expect(paths).not.toContain('/sessions')
    expect(paths).not.toContain('/agents')
  })

  it('uses the compound label while keeping /skills as the destination', () => {
    const item = getWorkNavigationSection().find(candidate => candidate.path === '/skills')

    expect(item).toMatchObject({
      path: '/skills',
      title: 'Skills & Channels',
      icon: 'skills',
    })
  })

  it('uses Usage as the rail entry while preserving the Status deep link', () => {
    const item = getWorkNavigationSection().find(candidate => candidate.path === '/usage')
    const route = sharedRoutes.find(candidate => candidate.path === '/overview')

    expect(item?.title).toBe('View usage')
    expect(route?.meta?.titleKey).toBe('nav.status')
  })
})

describe('navigation taxonomy invariants', () => {
  it('keeps Agents out of every primary navigation consumer', () => {
    // v10 导航精简：电脑操控/舰队是 agent 能力与模式执行逻辑、关系图谱与宠物
    // 移入设置，均不再出现在主导航。Agents 保留深链但不带 nav:'primary' 元数据。
    const primaryPaths = getNavigationItems('primary').map((item) => item.path)
    expect(primaryPaths).not.toContain('/agents')
    // The dedicated Settings footer owns /settings; it must not surface as a
    // primary rail row.
    expect(primaryPaths).not.toContain('/settings')
  })

  it('keeps the /agents deep link while omitting primary-nav metadata', () => {
    const agentsRoute = sharedRoutes.find((route) => route.path === '/agents')
    expect(agentsRoute).toBeDefined()
    expect(agentsRoute?.name).toBe('agents')
    expect(agentsRoute?.component).toBeDefined()
    expect(agentsRoute?.meta?.nav).toBeUndefined()
  })

  it('surfaces grouped non-Work primary items without duplicating the flat rail', () => {
    // Task #6: engine/editor/terminal etc. carry nav:'primary' but a non-Work
    // group, so they belong in the collapsible groups, not the flat rail.
    const primaryPaths = getNavigationItems('primary').map((i) => i.path)
    const grouped = getGroupedNavigation()
    const groupedPaths = new Set(grouped.flatMap((g) => g.items.map((i) => i.path)))
    for (const p of ['/engine', '/editor', '/terminal', '/browser-cockpit']) {
      expect(groupedPaths.has(p)).toBe(true)
    }
    // The flat rail (Work-group primary) must not also appear in the groups.
    for (const p of ['/skills', '/cron', '/usage']) {
      expect(groupedPaths.has(p)).toBe(false)
    }
    expect(primaryPaths).toContain('/chat')
  })

  it('keeps retired and hub-hosted routes out of the flat navigation', () => {
    const paths = getWorkNavigationSection().map((item) => item.path)
    for (const path of ['/approvals', '/agents', '/channels', '/overview', '/logs']) {
      expect(paths).not.toContain(path)
    }
  })
})

describe('getGroupedNavigation (Task #6 collapsible groups)', () => {
  it('buckets deep-link capabilities into the four semantic groups', () => {
    const groups = getGroupedNavigation()
    const byId = Object.fromEntries(groups.map((g) => [g.id, g.items.map((i) => i.path)]))
    // Observe must surface runtime diagnostics that were previously hidden.
    expect(byId.Observe).toEqual(expect.arrayContaining(['/logs', '/engine', '/team-board']))
    // Operate surfaces execution tools.
    expect(byId.Operate).toEqual(expect.arrayContaining(['/terminal', '/editor', '/browser-cockpit', '/computer-use']))
    // Settings surfaces system/agent subsystems.
    // 2026-10-26：插件改为独立管理页后重新进入侧栏（与 MCP / 工具 / 技能并列）；
    // /agents 与 /graph 仍由设置页 CTA 承载，不在侧栏重复。
    expect(byId.Settings).toEqual(expect.arrayContaining(['/expert-center', '/capabilities-hub', '/plugins']))
    for (const settingsHosted of ['/agents', '/graph']) {
      expect(byId.Settings).not.toContain(settingsHosted)
      expect(groups.flatMap((g) => g.items.map((i) => i.path))).not.toContain(settingsHosted)
    }
    // Operate also surfaces the standalone tools manager.
    expect(byId.Operate).toContain('/tools')
    // Extras isolates entertainment features away from the working rail.
    expect(byId.Extras).toEqual(expect.arrayContaining(['/poker', '/games', '/pet', '/wallpaper']))
    // No duplicate paths across groups (dedup by canonical route).
    const all = groups.flatMap((g) => g.items.map((i) => i.path))
    expect(new Set(all).size).toBe(all.length)
    // Primary rail items stay out of the groups.
    for (const primary of ['/skills', '/cron', '/usage']) {
      for (const paths of Object.values(byId)) {
        expect(paths).not.toContain(primary)
      }
    }
  })

  it('registers the team board as an Observe diagnostic destination', () => {
    const route = sharedRoutes.find((candidate) => candidate.path === '/team-board')
    expect(route?.name).toBe('team-board')
    expect(route?.meta?.group).toBe('Observe')
    expect(route?.meta?.nav).toBe('primary')
    expect(route?.meta?.navLabelKey).toBe('team.board.title')
    // 轮询视图不缓存：离开路由必须卸载以清理 5s 定时器与事件订阅。
    expect(route?.meta?.keepAlive).toBeUndefined()
    const observe = getGroupedNavigation().find((group) => group.id === 'Observe')
    expect(observe?.items.map((item) => item.path)).toContain('/team-board')
  })

  it('orders items inside a group by navOrder', () => {
    const operate = getGroupedNavigation().find((g) => g.id === 'Operate')
    expect(operate).toBeDefined()
    const idx = (p: string) => operate!.items.findIndex((i) => i.path === p)
    // /editor (86) precedes /browser-cockpit (88) precedes /terminal (89) precedes /ssh-collab (90)
    expect(idx('/editor')).toBeLessThan(idx('/browser-cockpit'))
    expect(idx('/browser-cockpit')).toBeLessThan(idx('/terminal'))
    expect(idx('/terminal')).toBeLessThan(idx('/ssh-collab'))
  })

  it('never emits an empty group', () => {
    for (const g of getGroupedNavigation()) {
      expect(g.items.length).toBeGreaterThan(0)
    }
  })
})
