import type { RouteRecordRaw } from 'vue-router'
import { detectPlatformId } from '@/platform/capabilities'
import { readLastRoute } from './lastRoute'

const ChatView = () => import('@/views/ChatView.vue')
const TaskWorkspaceView = () => import('@/views/TaskWorkspaceView.vue')
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
const IslandView = () => import('@/views/IslandView.vue')
const IslandGameView = () => import('@/views/IslandGameView.vue')
const PokerView = () => import('@/views/PokerView.vue')
const GamesView = () => import('@/views/GamesView.vue')
const EngineHealthView = () => import('@/views/EngineHealthView.vue')
const BrainView = () => import('@/views/BrainView.vue')
const WorkspaceEditorView = () => import('@/views/WorkspaceEditorView.vue')
const BrowserCockpitView = () => import('@/views/BrowserCockpitView.vue')
const SshCollabView = () => import('@/views/SshCollabView.vue')
const CapabilitiesHubView = () => import('@/views/CapabilitiesHubView.vue')
const ControlCenterView = () => import('@/views/ControlCenterView.vue')
const KnowledgePagesView = () => import('@/views/KnowledgePagesView.vue')
const TerminalView = () => import('@/views/TerminalView.vue')
const WallpaperView = () => import('@/views/WallpaperView.vue')
const ExpertCenterView = () => import('@/views/ExpertCenterView.vue')
const LocalModelsView = () => import('@/views/LocalModelsView.vue')
const OpsView = () => import('@/views/OpsView.vue')
const DeviceTwinStage = () => import('@/components/device-twin/DeviceTwinStage.vue')

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
  // Task workspace: the delivery-pipeline surface (steps + deliverables +
  // quality gate). It is a first-class Work destination (primary rail) so it is
  // discoverable exactly like /skills, /cron and /usage — a URL-only route was
  // effectively unreachable (no rail row, no push('/task') anywhere). It never
  // becomes the landing page: defaultRootRedirect() still returns /chat.
  { path: '/task',      name: 'task',      component: TaskWorkspaceView, meta: { title: 'Task', titleKey: 'task.title', group: 'Work', icon: 'listChecks', nav: 'primary', navOrder: 35, navLabelKey: 'task.title', platforms: ['web', 'desktop'] } },
  // Task ledger: still routed (deep links, the Not Found fallback, /approvals)
  // but off the nav — "New task" owns the top of the sidebar and the recents
  // list below it covers day-to-day session access.
  { path: '/sessions',  name: 'sessions',  component: SessionsView,  meta: { title: 'Sessions', titleKey: 'sessions.title', group: 'Work', icon: 'sessions', platforms: ['web', 'desktop'], keepAlive: true } },
  // Status and Usage share the Overview destination. Runtime logs remain a
  // kept-alive diagnostic deep link rather than a peer navigation tab.
  { path: '/overview',  name: 'overview',  component: OverviewHubView, meta: { title: 'Status', titleKey: 'nav.status', icon: 'home', platforms: ['web', 'desktop'], keepAlive: true, viewKey: 'overview-hub' } },
  { path: '/usage',     name: 'usage',     component: OverviewHubView, meta: { title: 'Usage', group: 'Work', icon: 'usage', nav: 'primary', navOrder: 60, navLabelKey: 'nav.viewUsage', platforms: ['web', 'desktop'], keepAlive: true, viewKey: 'overview-hub' } },
  { path: '/logs',      name: 'logs',      component: LogsView, meta: { title: 'Logs', group: 'Observe', icon: 'logs', platforms: ['web', 'desktop'], keepAlive: true } },
  // Approvals retired as a front-end destination: the pending queue resolves
  // inline in the chat transcript (ApprovalCard) and via the topbar interrupt
  // pill. The old deep link redirects to Sessions so bookmarks and the pill
  // degrade gracefully
  // (openBlockedApprovalSession() routes straight to the blocked chat first).
  { path: '/approvals', redirect: '/sessions' },
  // Agent administration remains available as an advanced deep link, but is
  // intentionally absent from primary navigation and cold-start restoration.
  { path: '/agents',    name: 'agents',    component: AgentsView,    meta: { title: 'Agents', group: 'Settings', icon: 'agents', platforms: ['web', 'desktop'], keepAlive: true } },
  // Skills and Channels form one primary destination. /skills remains the
  // rail target while both canonical routes share the same kept-alive hub.
  { path: '/skills',    name: 'skills',    component: SkillsChannelsHubView, meta: { title: 'Skills', group: 'Work', icon: 'skills', nav: 'primary', navOrder: 40, navLabelKey: 'nav.skillsChannels', platforms: ['web', 'desktop'], keepAlive: true, viewKey: 'skills-channels-hub' } },
  { path: '/channels',  name: 'channels',  component: SkillsChannelsHubView, meta: { title: 'Channels', icon: 'channels', platforms: ['web', 'desktop'], keepAlive: true, viewKey: 'skills-channels-hub' } },
  { path: '/cron',      name: 'cron',      component: CronView,      meta: { title: 'Cron', group: 'Work', icon: 'cron', nav: 'primary', navOrder: 50, platforms: ['web', 'desktop'], keepAlive: true } },
  // Editorial surface (read, not operated): the first route to opt into an
  // Axis-B expressive skin. Not in the primary nav — reached by URL / links.
  { path: '/changelog', name: 'changelog', component: ChangelogView, meta: { title: 'Changelog', group: 'Extras', icon: 'logs', platforms: ['web', 'desktop'], skin: 'out-of-register' } },
  // Readiness/doctor moved inline into Overview; the old deep link stays valid.
  { path: '/health',    redirect: '/overview' },
  // ── 电脑操控（三层架构：屏幕感知/键鼠控制/GUI定位）──
  // 这是 agent 的能力与模式执行逻辑（computer_use 工具链），归入 Operate 执行工具分组。
  { path: '/computer-use', name: 'computer-use', component: ComputerUseView, meta: { title: 'Computer Use', group: 'Operate', icon: 'cursor', platforms: ['web', 'desktop'] } },
  // ── 模型舰队（多角色路由）──
  // 舰队是 agent 的编排/模型路由执行形态（multi_strategy），归入 Operate 执行工具分组。
  { path: '/fleet', name: 'fleet', component: FleetView, meta: { title: 'Fleet', group: 'Operate', icon: 'agents', platforms: ['web', 'desktop'] } },
  // ── 自进化 ──
  // 两级闭环：元智能体（监控→诊断→规划→沙箱裁决→版本，提案须人工批准）+ prompt 自进化历史。
  // 归入 Observe 观测诊断分组（与引擎健康/共享大脑同组：都在回答「这套系统现在怎么样」）。
  { path: '/evolve', name: 'evolve', component: EvolveView, meta: { title: 'Evolve', group: 'Observe', icon: 'sparkles', nav: 'primary', navOrder: 82, platforms: ['web', 'desktop'], keepAlive: true } },
  // ── 插件市场 ──
  { path: '/plugins', name: 'plugins', component: PluginsView, meta: { title: 'Plugins', group: 'Work', icon: 'plugin', platforms: ['web', 'desktop'] } },
  // ── 关系图谱（KB 2.0 知识图谱可视化）── 归入 Settings 系统/能力分组。
  { path: '/graph', name: 'graph', component: GraphView, meta: { title: 'Graph', group: 'Settings', icon: 'gitBranch', platforms: ['web', 'desktop'] } },
  // ── 德州扑克（游戏大厅 · 赌 Token）── 娱乐功能，归 Extras 扩展组。
  { path: '/poker', name: 'poker', component: PokerView, meta: { title: 'Poker', group: 'Extras', icon: 'coins', platforms: ['web', 'desktop'] } },
  // ── 内置小游戏（games.list / games.play RPC；能力健康度横幅同源）── 归 Extras。
  { path: '/games', name: 'games', component: GamesView, meta: { title: 'Games', group: 'Extras', icon: 'play', platforms: ['web', 'desktop'] } },
  // ── 共享大脑 / Viky 数字生命体（PSI 意识引擎：八维情感 + Φ 存在感 + 相位节律 + 因果推理 + 知识资产）──
  // 归入 Observe 观测诊断分组——只读呈现真实引擎状态（brain.status）。
  { path: '/brain', name: 'brain', component: BrainView, meta: { title: 'Brain', group: 'Observe', icon: 'sparkles', nav: 'primary', navOrder: 83, platforms: ['web', 'desktop'] } },
  // ── 引擎健康 / 模型档案 / 运行时模式（runtime.status + models.profiles.* + mode.get/switch）── 归入 Observe 观测诊断分组。
  { path: '/engine', name: 'engine', component: EngineHealthView, meta: { title: 'Engine Health', group: 'Observe', icon: 'gauge', nav: 'primary', navOrder: 84, platforms: ['web', 'desktop'] } },
  // ── Ops 运维面板（ops.* 只读 RPC：主机/状态/文件/日志/进程）── 归入 Observe 观测诊断分组。
  { path: '/ops', name: 'ops', component: OpsView, meta: { title: 'Ops', group: 'Observe', icon: 'terminal', nav: 'primary', navOrder: 87, platforms: ['web', 'desktop'] } },
  // ── 智能体与专家中心（预设智能体/专家库/蜂群规格）── 归入 Settings 系统/能力分组。
  { path: '/expert-center', name: 'expert-center', component: ExpertCenterView, meta: { title: 'Expert Center', group: 'Settings', icon: 'listChecks', nav: 'primary', navOrder: 85, platforms: ['web', 'desktop'] } },
  // ── 内置工作区编辑器（文件树 + 多标签文本编辑）── 归入 Operate 执行工具分组。
  { path: '/editor', name: 'editor', component: WorkspaceEditorView, meta: { title: 'Editor', group: 'Operate', icon: 'fileCode', nav: 'primary', navOrder: 86, platforms: ['web', 'desktop'] } },
  // ── 浏览器驾驶舱（CDP 操控 / JS 控制台 / 截图点击 / 无障碍树）── 归入 Operate 执行工具分组。
  { path: '/browser-cockpit', name: 'browser-cockpit', component: BrowserCockpitView, meta: { title: 'Browser Cockpit', group: 'Operate', icon: 'appWindow', nav: 'primary', navOrder: 88, platforms: ['web', 'desktop'] } },
  // ── 企业协同 / SSH + 能力中心 / 设置总览 ──
  { path: '/terminal', name: 'terminal', component: TerminalView, meta: { title: 'Terminal', group: 'Operate', icon: 'command', nav: 'primary', navOrder: 89, platforms: ['web', 'desktop'] } },
  { path: '/ssh-collab', name: 'ssh-collab', component: SshCollabView, meta: { title: 'SSH Collab', group: 'Operate', icon: 'collab', nav: 'primary', navOrder: 90, platforms: ['web', 'desktop'] } },
  { path: '/capabilities-hub', name: 'capabilities-hub', component: CapabilitiesHubView, meta: { title: 'Capabilities Hub', group: 'Settings', icon: 'capabilities', nav: 'primary', navOrder: 92, platforms: ['web', 'desktop'] } },
  // ── 智能中枢（模式切换 / BloomSpec / 三层缓存 / 能力矩阵 统一仪表）──
  { path: '/control-center', name: 'control-center', component: ControlCenterView, meta: { title: 'Control Center', group: 'Settings', icon: 'mode', nav: 'primary', navOrder: 93, platforms: ['web', 'desktop'], keepAlive: true } },
  // ── 本地模型中心（聚合下载源 + 系统推荐 + 一键部署/启停 + 本地/云路由标记）──
  { path: '/local-models', name: 'local-models', component: LocalModelsView, meta: { title: 'Local Models', group: 'Operate', icon: 'mode', nav: 'primary', navOrder: 94, platforms: ['web', 'desktop'], keepAlive: true } },
  { path: '/knowledge', name: 'knowledge', component: KnowledgePagesView, meta: { title: 'Knowledge', group: 'Settings', icon: 'knowledge', nav: 'primary', navOrder: 88, platforms: ['web', 'desktop'] } },
  // ── 岛屿工作空间（智能体运行的 3D 可视化）──
  // 建筑语义：主堡=会话运行态、工坊=工具调用、哨所=子代理、仓库=产物、灯塔=路由策略、栈桥=计划进度。
  // 与 /task 同属 Work 分组（都是「任务怎么在跑」的视图），紧随任务工作区之后。
  { path: '/island', name: 'island', component: IslandView, meta: { title: 'Island', titleKey: 'island.title', group: 'Work', icon: 'blocks', nav: 'primary', navOrder: 37, navLabelKey: 'island.title', platforms: ['web', 'desktop'], keepAlive: true } },
  // ── 岛屿开放世界 MVP（空间兼游戏：WASD 漫游 3D 岛 + 真实引擎 RPC 绑定 + 委托/编队/称号/天气）──
  // 娱乐/实验性质，归 Extras 扩展组；与既有 /island（运行事件→建筑可视化）互不干扰。
  { path: '/island-game', name: 'island-game', component: IslandGameView, meta: { title: 'Island Game', titleKey: 'islandGame.title', group: 'Extras', icon: 'play', navLabelKey: 'islandGame.navLabel', platforms: ['web', 'desktop'] } },
  // ── 鲸鱼娘桌宠 ── 娱乐功能，归 Extras 扩展组。
  { path: '/pet', name: 'pet', component: PetView, meta: { title: 'Pet', group: 'Extras', icon: 'pet', platforms: ['web', 'desktop'] } },
  // ── 皮肤中心 · 壁纸（Web UI 插件：本地壁纸库 + 动态/静态壁纸 + 沉浸式主题）── 归 Extras。
  { path: '/wallpaper', name: 'wallpaper', component: WallpaperView, meta: { title: 'Wallpaper', group: 'Extras', icon: 'image', platforms: ['web', 'desktop'] } },
  // ── 设备 3D 数字孪生（冰箱参考实现；教学模拟，未连真实设备）──
  { path: '/device-twin', name: 'device-twin', component: DeviceTwinStage, meta: { title: 'Device Twin', titleKey: 'deviceTwin.title', group: 'Extras', icon: 'blocks', platforms: ['web', 'desktop'] } },
]
