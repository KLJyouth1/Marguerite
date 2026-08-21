import type { RouteRecordRaw } from 'vue-router'
import { detectPlatformId } from '@/platform/capabilities'
import { readLastRoute } from './lastRoute'

const ChatView = () => import('@/views/ChatView.vue')
const CronView = () => import('@/views/CronView.vue')
const AgentsView = () => import('@/views/AgentsView.vue')
const SessionsView = () => import('@/views/SessionsView.vue')
const ChangelogView = () => import('@/views/ChangelogView.vue')
const OverviewHubView = () => import('@/views/OverviewHubView.vue')
const LogsView = () => import('@/views/LogsView.vue')
const SkillsChannelsHubView = () => import('@/views/SkillsChannelsHubView.vue')
const ComputerUseView = () => import('@/views/ComputerUseView.vue')
const FleetView = () => import('@/views/FleetView.vue')
const EvolveView = () => import('@/views/EvolveView.vue')
const PluginsView = () => import('@/views/PluginsView.vue')
const GraphView = () => import('@/views/GraphView.vue')
const PetView = () => import('@/views/PetView.vue')
const PokerView = () => import('@/views/PokerView.vue')

export function defaultRootRedirect(): string {
  if (detectPlatformId() === 'desktop') return '/chat'
  const saved = readLastRoute()
  if (saved) return saved
  // Chat on every form factor: Sessions is no longer a navigation destination,
  // so landing there would open on a page the sidebar cannot get back to.
  return '/chat'
}

export const sharedRoutes: RouteRecordRaw[] = [
  {
    path: '/',
    redirect: () => {
      // Desktop app cold starts should feel like opening an assistant: land on
      // Chat, not the session ledger. Browser builds still restore the last
      // stable view when available, with the existing responsive fallback.
      return defaultRootRedirect()
    },
  },
  { path: '/chat',      name: 'chat',      component: ChatView,      meta: { title: 'Chat', group: 'Work', icon: 'chat', nav: 'primary', navOrder: 10, platforms: ['web', 'desktop'], keepAlive: true, viewKey: 'chat' } },
  // Draft state: a clean composer with no session key until the first send.
  { path: '/chat/new',  name: 'chat-new',  component: ChatView,      meta: { title: 'Chat', group: 'Work', icon: 'chat', platforms: ['web', 'desktop'], keepAlive: true, viewKey: 'chat' } },
  // Task ledger: still routed (deep links, the Not Found fallback, /approvals)
  // but off the nav — "New task" owns the top of the sidebar and the recents
  // list below it covers day-to-day session access.
  { path: '/sessions',  name: 'sessions',  component: SessionsView,  meta: { title: 'Sessions', titleKey: 'sessions.title', group: 'Work', icon: 'sessions', platforms: ['web', 'desktop'], keepAlive: true } },
  // Status and Usage share the Overview destination. Runtime logs remain a
  // kept-alive diagnostic deep link rather than a peer navigation tab.
  { path: '/overview',  name: 'overview',  component: OverviewHubView, meta: { title: 'Status', titleKey: 'nav.status', icon: 'home', platforms: ['web', 'desktop'], keepAlive: true, viewKey: 'overview-hub' } },
  { path: '/usage',     name: 'usage',     component: OverviewHubView, meta: { title: 'Usage', group: 'Work', icon: 'usage', nav: 'primary', navOrder: 60, navLabelKey: 'nav.viewUsage', platforms: ['web', 'desktop'], keepAlive: true, viewKey: 'overview-hub' } },
  { path: '/logs',      name: 'logs',      component: LogsView, meta: { title: 'Logs', icon: 'logs', platforms: ['web', 'desktop'], keepAlive: true } },
  // Approvals retired as a front-end destination: the pending queue resolves
  // inline in the chat transcript (ApprovalCard) and via the topbar interrupt
  // pill. The old deep link redirects to Sessions so bookmarks and the pill
  // degrade gracefully
  // (openBlockedApprovalSession() routes straight to the blocked chat first).
  { path: '/approvals', redirect: '/sessions' },
  // Agent administration remains available as an advanced deep link, but is
  // intentionally absent from primary navigation and cold-start restoration.
  { path: '/agents',    name: 'agents',    component: AgentsView,    meta: { title: 'Agents', icon: 'agents', platforms: ['web', 'desktop'], keepAlive: true } },
  // Skills and Channels form one primary destination. /skills remains the
  // rail target while both canonical routes share the same kept-alive hub.
  { path: '/skills',    name: 'skills',    component: SkillsChannelsHubView, meta: { title: 'Skills', group: 'Work', icon: 'skills', nav: 'primary', navOrder: 40, navLabelKey: 'nav.skillsChannels', platforms: ['web', 'desktop'], keepAlive: true, viewKey: 'skills-channels-hub' } },
  { path: '/channels',  name: 'channels',  component: SkillsChannelsHubView, meta: { title: 'Channels', icon: 'channels', platforms: ['web', 'desktop'], keepAlive: true, viewKey: 'skills-channels-hub' } },
  { path: '/cron',      name: 'cron',      component: CronView,      meta: { title: 'Cron', group: 'Work', icon: 'cron', nav: 'primary', navOrder: 50, platforms: ['web', 'desktop'], keepAlive: true } },
  // Editorial surface (read, not operated): the first route to opt into an
  // Axis-B expressive skin. Not in the primary nav — reached by URL / links.
  { path: '/changelog', name: 'changelog', component: ChangelogView, meta: { title: 'Changelog', platforms: ['web', 'desktop'], skin: 'out-of-register' } },
  // Readiness/doctor moved inline into Overview; the old deep link stays valid.
  { path: '/health',    redirect: '/overview' },
  // ── 电脑操控（三层架构：屏幕感知/键鼠控制/GUI定位）──
  { path: '/computer-use', name: 'computer-use', component: ComputerUseView, meta: { title: 'Computer Use', group: 'Work', icon: 'cursor', nav: 'primary', navOrder: 55, platforms: ['web', 'desktop'] } },
  // ── 模型舰队（多角色路由）──
  { path: '/fleet', name: 'fleet', component: FleetView, meta: { title: 'Fleet', group: 'Work', icon: 'agents', nav: 'primary', navOrder: 45, platforms: ['web', 'desktop'] } },
  // ── 自进化 ──
  { path: '/evolve', name: 'evolve', component: EvolveView, meta: { title: 'Evolve', group: 'Work', icon: 'sparkles', nav: 'primary', navOrder: 65, platforms: ['web', 'desktop'] } },
  // ── 插件市场 ──
  { path: '/plugins', name: 'plugins', component: PluginsView, meta: { title: 'Plugins', group: 'Work', icon: 'blocks', nav: 'primary', navOrder: 70, platforms: ['web', 'desktop'] } },
  // ── 关系图谱（KB 2.0 知识图谱可视化）──
  { path: '/graph', name: 'graph', component: GraphView, meta: { title: 'Graph', group: 'Work', icon: 'gitBranch', nav: 'primary', navOrder: 75, platforms: ['web', 'desktop'] } },
  // ── 德州扑克（游戏大厅 · 赌 Token）──
  { path: '/poker', name: 'poker', component: PokerView, meta: { title: 'Poker', group: 'Work', icon: 'coins', nav: 'primary', navOrder: 80, platforms: ['web', 'desktop'] } },
  // ── 鲸鱼娘桌宠 ──
  { path: '/pet', name: 'pet', component: PetView, meta: { title: 'Pet', group: 'Work', icon: 'sparkles', nav: 'primary', navOrder: 85, platforms: ['web', 'desktop'] } },
]
