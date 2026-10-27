/**
 * 静态资产 URL 解析（`public/` 目录随包分发的文件）。
 *
 * 两套运行环境路径不同，混用必坏：
 *  · DEV（vite）：`public/` 挂在站点根 → `/pets/x.png`
 *  · PROD（网关托管 dist）：静态根是 `<basePath>/static/`，**已指向 dist/**
 *    → `/control/static/pets/x.png`
 *
 * 历史坑：PROD 下拼成 `<basePath>/static/dist/...` 会被 SPA 回退成 index.html，
 * 浏览器把 HTML 当图片解析 → 静默空白（见 island3d/glbAssets.ts 的同款注释）。
 */
export function staticAssetUrl(path: string): string {
  const clean = path.replace(/^\/+/, '')
  if (import.meta.env.DEV) return `/${clean}`
  const base = document.getElementById('Marguerite-data')?.dataset.basePath || '/control'
  return `${base.replace(/\/$/, '')}/static/${clean}`
}
