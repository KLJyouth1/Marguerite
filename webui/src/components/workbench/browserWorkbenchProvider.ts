import { defineAsyncComponent } from 'vue'
import type { WorkbenchPanelDefinition } from '@/workbench/types'
import { browserUrlFromWorkbenchItem } from '@/workbench/browserItems'

/**
 * 侧边浏览器面板定义。
 *
 * 2026-10-24 改版：原先这里跑的是**原生 WebContents 表面**（Tauri 子 webview），
 * 但当前桌面壳根本没有实现 `createWorkbenchSurface` 一族命令 ——
 * 也就是说这个面板在真机上是「打不开」的（会提示升级桌面端）。
 *
 * 现在改为 **CDP 镜像面板**：真实 Chromium（`browser.*` RPC）+ 可点击转发 +
 * 控制权租约（用户接管 / 交还）。它不依赖壳的命令，安装版开箱可用；
 * 代价是画面是截图镜像而非原生控件（滚动/键盘输入不在本版范围）。
 *
 * 面板按需异步加载：`highlight`/CDP 相关逻辑不进工作台首屏包。
 */
const BrowserCdpPanel = defineAsyncComponent(() => import('./BrowserCdpPanel.vue'))

export interface BrowserWorkbenchProviderOptions {
  t(key: string, params?: Record<string, unknown>): string
}

export function createBrowserWorkbenchDefinition(
  options: BrowserWorkbenchProviderOptions,
): WorkbenchPanelDefinition {
  return {
    kind: 'browser',
    component: BrowserCdpPanel,
    supports: item => item.kind === 'browser' && Boolean(browserUrlFromWorkbenchItem(item)),
    getHeader: (item, state) => {
      const url = String(state.runtimeState.currentUrl || browserUrlFromWorkbenchItem(item))
      let host = url
      try { host = new URL(url).hostname || url } catch { /* 保留原文 */ }
      return {
        icon: 'languages',
        title: String(state.runtimeState.pageTitle || host || options.t('workbench.browser.preview')),
        subtitle: String(state.runtimeState.currentUrl || url),
      }
    },
    getProps: (item, state) => ({
      scopeId: item.scope.type === 'session' ? item.scope.id : 'app',
      initialUrl: browserUrlFromWorkbenchItem(item),
      active: state.active,
    }),
  }
}
