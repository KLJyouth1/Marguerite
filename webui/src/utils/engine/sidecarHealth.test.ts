import { describe, expect, it } from 'vitest'
import { sidecarDegradedText } from './sidecarHealth'

// 假 t 直接回显「键 + 参数」，断言只关心"选了哪条键、参数对不对"，
// 不把 6 个 locale 的具体措辞写进单测（措辞会改，语义不该改）。
const t = (key: string, params?: Record<string, unknown>) =>
  params && 'modules' in params ? `${key}(${String(params.modules)})` : key

describe('sidecarDegradedText', () => {
  it('未降级 / 无载荷时返回空串（界面整块不渲染）', () => {
    expect(sidecarDegradedText(null, t)).toBe('')
    expect(sidecarDegradedText({ degraded: false, missing: [] }, t)).toBe('')
  })

  it('缺 numpy 时使用「本地 ML 路由已停用」这条具体影响，而不是笼统的依赖缺失', () => {
    const text = sidecarDegradedText(
      { degraded: true, missing: ['numpy'], reason: 'local_ml_router_disabled' },
      t,
    )
    expect(text).toBe('engineHealth.sidecarMissing(numpy) · engineHealth.sidecarNumpyImpact')
  })

  it('其它缺依赖走通用影响说明', () => {
    const text = sidecarDegradedText({ degraded: true, missing: ['httpx'], reason: 'dependencies_missing' }, t)
    expect(text).toBe('engineHealth.sidecarMissing(httpx) · engineHealth.sidecarGenericImpact')
  })

  it('多个缺失模块按序拼接', () => {
    const text = sidecarDegradedText({ degraded: true, missing: ['numpy', 'anyio'] }, t)
    expect(text).toContain('numpy, anyio')
  })

  it('degraded=true 但 missing 为空时只说影响，不显示空的「缺少依赖：」', () => {
    const text = sidecarDegradedText({ degraded: true, missing: [] }, t)
    expect(text).toBe('engineHealth.sidecarGenericImpact')
    expect(text).not.toContain('sidecarMissing')
  })
})
