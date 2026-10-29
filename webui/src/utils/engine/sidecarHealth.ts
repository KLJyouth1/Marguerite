// 「侧车能力降级」文案的唯一产出点。
//
// 真机审计 2026-10-29 P1-2：安装版侧车缺 `numpy` → 本地 ML 路由（V4 Phase 3）被禁用、
// 回退 default-tier。日志里刷了 22 条 `No module named 'numpy'`，而界面上**一片正常**，
// 用户于是把「丢了一个能力」理解成「这版本就这样」。
//
// 所以这段文案的职责不是"报个错"，而是回答两个问题：
// ① **丢了什么**（缺哪些模块）；② **影响是什么**（哪个能力没了、什么还可用）。
// 只说"依赖缺失"等于没说 —— 用户无法判断要不要处理。
//
// 后端只下发机器键（`reason: local_ml_router_disabled`），翻译在前端完成（i18n 纪律：
// 后端拼进界面的中文字符串在 6 个 locale 里搜不到）。

export interface SidecarHealth {
  python?: string
  pyengine_src?: string
  missing?: string[]
  degraded?: boolean
  reason?: string
}

export type Translate = (key: string, params?: Record<string, unknown>) => string

/**
 * 生成降级说明；未降级时返回空串（调用方据此决定是否渲染整块）。
 *
 * 空 `missing` 但 `degraded=true` 属后端自相矛盾的载荷：此时不编模块名，
 * 只给通用影响说明 —— 界面宁可少说，也不要显示「缺少依赖：」后面空着。
 */
export function sidecarDegradedText(sc: SidecarHealth | null | undefined, t: Translate): string {
  if (!sc?.degraded) return ''
  const modules = (sc.missing ?? []).filter(Boolean).join(', ')
  const impact = sc.reason === 'local_ml_router_disabled'
    ? t('engineHealth.sidecarNumpyImpact')
    : t('engineHealth.sidecarGenericImpact')
  if (!modules) return impact
  return `${t('engineHealth.sidecarMissing', { modules })} · ${impact}`
}
