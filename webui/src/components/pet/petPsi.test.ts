import { describe, it, expect } from 'vitest'
import { unpackPsiBrain, type PsiSnapshot } from './petPsi'

// 回归守护：DesktopPet 曾把 `psi.value = res`（整包 {ok, brain}）后读顶层 `psi.phi` /
// `psi.emotions`，而真实字段全在 `res.brain` 下（且 phi_existence 是字符串）。此测试锁定
// 正确解包，防止该 bug 回退——八维情感必须能从 `brain.emotion` 取到，Φ 来自 `phi_existence`。

describe('unpackPsiBrain (PSI 字段解包 / DesktopPet 回归)', () => {
  it('从 brain.status 的 { ok, brain } 包裹解包出 emotion（不丢字段）', () => {
    const res = {
      ok: true,
      brain: {
        emotion: { valence: 0.9, arousal: 0.8, curiosity: 0.7, confidence: 0.6, connection: 0.5, growth: 0.4, certainty: 0.7, existence: 0.6 },
        phi_existence: '0.750',
        dominant_mood: 'curious',
        circadian: 'Day',
      },
    }
    const b = unpackPsiBrain(res)
    expect(b).not.toBeNull()
    // 关键：八维情感从 brain.emotion 取，不再回落 0.5
    expect(b!.emotion).toBeDefined()
    expect(b!.emotion!.valence).toBeCloseTo(0.9)
    expect(b!.emotion!.existence).toBeCloseTo(0.6)
    expect(b!.phi_existence).toBe('0.750')
    expect(b!.dominant_mood).toBe('curious')
    expect(b!.circadian).toBe('Day')
  })

  it('phi_existence 是字符串字段（旧 bug：误读顶层 psi.phi → 永远 undefined）', () => {
    const b = unpackPsiBrain({ ok: true, brain: { phi_existence: '0.123' } })
    // 旧代码读 res.phi（顶层），结果恒 undefined；正确解包下 psi.phi 不存在、phi_existence 有值
    expect((b as Record<string, unknown> | null)?.phi).toBeUndefined()
    expect(b?.phi_existence).toBe('0.123')
  })

  it('psi_broadcast 直接投递 snapshot（无 brain 包装）时原样返回', () => {
    const snap: PsiSnapshot = { emotion: { valence: 0.5, arousal: 0.5 }, circadian: 'Night' }
    expect(unpackPsiBrain(snap)).toEqual(snap)
  })

  it('引擎未就绪（null / 无 brain / 无已知字段）时返回 null，调用方静默跳过', () => {
    expect(unpackPsiBrain(null)).toBeNull()
    expect(unpackPsiBrain(undefined)).toBeNull()
    expect(unpackPsiBrain('garbage')).toBeNull()
    expect(unpackPsiBrain({ ok: true })).toBeNull() // 无 brain 且自身不是 snapshot
    expect(unpackPsiBrain(42)).toBeNull()
  })

  it('{ ok, brain: <非空> } 即便 brain 无 emotion 也返回该快照（由调用方决定是否驱动外观）', () => {
    const b = unpackPsiBrain({ ok: true, brain: { circadian: 'Day' } })
    expect(b).not.toBeNull()
    expect(b!.circadian).toBe('Day')
    expect(b!.emotion).toBeUndefined()
  })
})
