import { createRouter, createWebHistory } from 'vue-router'
import type { RouteRecordRaw, RouteLocationNormalized } from 'vue-router'
import { getPlatform } from '@/platform'
import i18n, { isI18nReady } from '@/i18n'
import { desktopRoutes } from './desktopRoutes'
import { sharedRoutes } from './sharedRoutes'
import { webRoutes } from './webRoutes'
import { captureContentScroll, contentScrollBehavior } from './scrollMemory'
import { saveLastRoute } from './lastRoute'
import { legacyChannelHashRedirect } from './legacyRedirects'
import {
  clearPrimedSessionBootstrapAdmission,
  primeSessionBootstrapAdmission,
} from '@/composables/chat/sessionBootstrapAdmission'

const basePath = (() => {
  const el = document.getElementById('Marguerite-data')
  const raw = el?.dataset.basePath || '/control'
  return raw.endsWith('/') ? raw : raw + '/'
})()

const platform = getPlatform()
const NotFoundView = () => import('@/views/NotFoundView.vue')

export const routes: RouteRecordRaw[] = [
  ...sharedRoutes,
  ...(platform.capabilities.hasWebConfig ? webRoutes : []),
  // 桌面专属路由（目前只有 /pet-overlay 悬浮窗）按 **isDesktop** 注册，而不是
  // `hasDesktopOnboarding`：后者是「桥是否实现了 onboarding 三个命令」的能力位，
  // 与悬浮窗毫无关系。真机缺陷（2026-10-23）：桥是部分实现时 hasDesktopOnboarding
  // 为 false ⇒ 悬浮窗窗口加载 /control/pet-overlay 落到 NotFound，
  // 360×420 的透明窗口里显示「未找到」+ 整条侧栏。
  ...(platform.capabilities.isDesktop ? desktopRoutes : []),
  // 直觉 URL 的 redirect（真机实测三个都 404：04/05/06 截图）。
  // 用户对「模型 / 能力 / 缓存」的第一直觉路径不是产品命名，缺了这层等于告诉
  // 用户页面不存在。放在 catch-all 之前，否则会被 NotFound 吞掉。
  { path: '/models', redirect: '/local-models', meta: { platforms: ['web', 'desktop'] } },
  { path: '/capabilities', redirect: '/capabilities-hub', meta: { platforms: ['web', 'desktop'] } },
  { path: '/cache', redirect: '/control-center', meta: { platforms: ['web', 'desktop'] } },
  { path: '/:pathMatch(.*)*', name: 'not-found', component: NotFoundView, meta: { title: 'Not Found', platforms: ['web', 'desktop'] } },
]

export const router = createRouter({
  history: createWebHistory(basePath),
  routes,
  scrollBehavior: contentScrollBehavior,
})

function isChatRoutePath(path: string): boolean {
  return path === '/chat' || path === '/chat/new'
}

// ChatView is lazy-loaded, while App/Sidebar mounted hooks can run as soon as
// the root shell exists. Prime a singleton admission hold before resolving the
// lazy route so optional shell RPCs cannot enter the Gateway's serialized
// dispatcher ahead of session subscribe/snapshot/history. Query-only chat
// navigation reuses the mounted view and owns its hold through the coordinator.
router.beforeEach((to, from) => {
  const enteringChat = isChatRoutePath(to.path) && !isChatRoutePath(from.path)
  if (enteringChat) {
    primeSessionBootstrapAdmission()
  } else if (!isChatRoutePath(to.path)) {
    clearPrimedSessionBootstrapAdmission()
  }
})

// Capture the leaving route's content scroll offset so back/forward can restore it.
router.beforeEach((_to, from) => {
  captureContentScroll(from)
})

// Stale channel-setup bookmarks (#channel-… hashes) land on the workspace.
router.beforeEach((to) => legacyChannelHashRedirect(to) ?? true)

// Localize the document title from the route name token (e.g. `nav.sessions`),
// falling back to the English meta.title. `applyRouteTitle` is also re-run when
// the locale changes (App.vue watches the store) since afterEach does not
// re-fire without a navigation.
export function routeTitle(route: RouteLocationNormalized): string {
  const explicitKey = route.meta?.titleKey
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
  return (route.meta?.title as string) || 'Marguerite'
}

/**
 * 写入页面标题。i18n 未就绪时返回 false 并且**不碰** document.title：
 * 首屏导航常常早于语言包加载完成，此时 routeTitle() 只有英文 meta.title 可回退，
 * 写了就会在语言包就绪后再跳成中文。留白由 App.vue 在 whenI18nReady() 后补写。
 * `ready` 显式传入以便单测覆盖两条分支。
 */
export function applyRouteTitle(
  route: RouteLocationNormalized,
  ready: boolean = isI18nReady(),
): boolean {
  if (!ready) return false
  document.title = `${routeTitle(route)} — Marguerite`
  return true
}

router.afterEach((to, _from, failure) => {
  if (failure || !isChatRoutePath(to.path)) {
    clearPrimedSessionBootstrapAdmission()
  }
  applyRouteTitle(to)
  // Remember the current view (path only) so the next launch reopens here.
  saveLastRoute(to.path)
})

router.onError(() => {
  clearPrimedSessionBootstrapAdmission()
})
