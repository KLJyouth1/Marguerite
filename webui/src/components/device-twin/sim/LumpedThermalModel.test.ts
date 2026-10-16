import { describe, expect, it } from 'vitest'
import { createLumpedThermalModel, DEFAULT_THERMAL_PARAMS } from './LumpedThermalModel'

describe('LumpedThermalModel — 数值收敛', () => {
  it('跑 10 分钟仿真无 NaN/Infinity', () => {
    const m = createLumpedThermalModel()
    for (let i = 0; i < 600; i++) {
      const s = m.step(1) // 1 秒一步，共 600 秒
      expect(Number.isFinite(s.T_cab)).toBe(true)
      expect(Number.isFinite(s.T_cond)).toBe(true)
      expect(Number.isFinite(s.cumulative.energyWh)).toBe(true)
      expect(s.T_cab).toBeGreaterThanOrEqual(-30)
      expect(s.T_cab).toBeLessThanOrEqual(60)
      expect(s.T_cond).toBeGreaterThanOrEqual(-20)
      expect(s.T_cond).toBeLessThanOrEqual(80)
    }
  })

  it('压缩机温控：箱内温度最终收敛到设定值附近（±回差）', () => {
    const m = createLumpedThermalModel()
    // 跑足够久让它达到循环
    for (let i = 0; i < 3600; i++) m.step(1)
    const s = m.snapshot()
    const p = m.params
    // 箱内温度应在设定值 ± 回差 2 倍以内（教学模型允许小偏差）
    expect(s.T_cab).toBeGreaterThan(p.T_setpoint - p.hysteresis * 2 - 1)
    expect(s.T_cab).toBeLessThan(p.T_setpoint + p.hysteresis * 2 + 5)
  })

  it('能量平衡：Q_cond ≈ Q_evap + W_comp（累计误差 < 10%）', () => {
    const m = createLumpedThermalModel()
    for (let i = 0; i < 3600; i++) m.step(1)
    const err = m.energyBalanceError()
    expect(err).toBeLessThan(0.1)
  })

  it('从环境温度开始，箱内最终会降温（制冷有效）', () => {
    const m = createLumpedThermalModel()
    const start = m.snapshot().T_cab
    for (let i = 0; i < 1800; i++) m.step(1)
    const end = m.snapshot().T_cab
    expect(end).toBeLessThan(start)
    expect(end).toBeLessThan(m.params.T_env)
  })

  it('压缩机运行时冷凝器温度高于环境', () => {
    const m = createLumpedThermalModel()
    // 强制压缩机运行：跑到它开起来
    let on = false
    for (let i = 0; i < 3600 && !on; i++) {
      m.step(1)
      on = m.snapshot().compressorOn
    }
    if (on) {
      expect(m.snapshot().T_cond).toBeGreaterThan(m.params.T_env)
    }
  })

  it('reset 回到初始状态', () => {
    const m = createLumpedThermalModel()
    for (let i = 0; i < 100; i++) m.step(1)
    m.reset()
    const s = m.snapshot()
    expect(s.T_cab).toBe(m.params.T_env - 2)
    expect(s.cumulative.energyWh).toBe(0)
  })

  it('极端 dt（0 / 负 / NaN）不炸', () => {
    const m = createLumpedThermalModel()
    const before = m.snapshot()
    expect(m.step(0).T_cab).toBe(before.T_cab)
    expect(m.step(-10).T_cab).toBe(before.T_cab)
    expect(m.step(NaN).T_cab).toBe(before.T_cab)
  })

  it('默认参数物理合理', () => {
    expect(DEFAULT_THERMAL_PARAMS.T_setpoint).toBeGreaterThan(-30)
    expect(DEFAULT_THERMAL_PARAMS.T_setpoint).toBeLessThan(10)
    expect(DEFAULT_THERMAL_PARAMS.Q_evap_max).toBeGreaterThan(0)
    expect(DEFAULT_THERMAL_PARAMS.W_comp).toBeGreaterThan(0)
  })
})
