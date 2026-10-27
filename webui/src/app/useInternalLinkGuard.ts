// 应用内链接守卫。
//
// 背景（真机反馈）：左侧导航与部分页面里仍存在「裸 `<a href>`」或写死
// `/control/...` 的内部链接。在桌面 WebView 中这类链接会触发整页导航（丢失
// SPA 状态、看起来像跳到了浏览器的新页面），在浏览器里则是一次完整刷新。
// 路由链接本身由 `<router-link>` 保证 SPA，但任何遗留的裸锚点都不该破坏外壳。
//
// 规则（保守，宁可放过不可误伤）：
//  - 只拦截主键/无修饰键的左键点击；
//  - 跳过 target/download/rel=external/mailto/tel/javascript 等显式外链语义；
//  - 只接管同源、且解析后能被当前 router 匹配（非 not-found）的路径；
//  - 外部站点、REST 资产（/api/**）、网关直链一律不碰。

import type { Router } from 'vue-router'

const EXTERNAL_SCHEMES = /^(?:mailto|tel|sms|javascript|data|blob):/i

function isPlainLeftClick(event: MouseEvent): boolean {
  return event.button === 0 && !event.metaKey && !event.ctrlKey && !event.shiftKey && !event.altKey
}

function anchorFromEvent(event: Event): HTMLAnchorElement | null {
  const target = event.target
  if (!(target instanceof Element)) return null
  const anchor = target.closest('a[href]')
  return anchor instanceof HTMLAnchorElement ? anchor : null
}

export function installInternalLinkGuard(router: Router): () => void {
  if (typeof document === 'undefined') return () => {}
  const basePath = (() => {
    const el = document.getElementById('Marguerite-data')
    const raw = el?.dataset.basePath || '/control'
    return raw.endsWith('/') ? raw : `${raw}/`
  })()
  const origin = window.location.origin

  function onClick(event: MouseEvent) {
    if (event.defaultPrevented || !isPlainLeftClick(event)) return
    const anchor = anchorFromEvent(event)
    if (!anchor) return
    const rawHref = anchor.getAttribute('href') || ''
    if (!rawHref || EXTERNAL_SCHEMES.test(rawHref)) return
    if (anchor.hasAttribute('download')) return
    if (anchor.getAttribute('rel')?.split(/\s+/).includes('external')) return
    const target = anchor.getAttribute('target')
    if (target && target !== '_self') return

    let url: URL
    try {
      url = new URL(rawHref, window.location.href)
    } catch {
      return
    }
    if (url.origin !== origin) return
    if (!url.pathname.startsWith(basePath)) return
    // REST / 网关直链不归 Vue Router 管，交给浏览器默认行为。
    if (/^\/(?:api|ws)(?:\/|$)/.test(url.pathname)) return

    const resolved = router.resolve({
      path: url.pathname.slice(basePath.length - 1),
      query: Object.fromEntries(url.searchParams.entries()),
      hash: url.hash,
    })
    if (!resolved.matched.length || resolved.name === 'not-found') return

    event.preventDefault()
    void router.push(resolved.fullPath)
  }

  document.addEventListener('click', onClick, true)
  return () => document.removeEventListener('click', onClick, true)
}
