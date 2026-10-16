/**
 * LumpedThermalModel — 集总参数教学热模型（纯数值，node 可测）。
 *
 * 这是**教学模拟**，不是工程仿真：用两个热容量（箱内 + 冷凝器）和简化热流
 * 演示冰箱的热力学循环。界面必须显著标注「教学模拟 · 非实测」。
 *
 * 能量守恒教学要点：
 *   Q_cond（冷凝器排热） ≈ Q_evap（蒸发器吸热） + P_comp（压缩机耗电）
 * 稳态时箱内热量泄漏 = 制冷量，压缩机按温控器通断。
 *
 * 数值稳定性：
 * - 显式 Euler，dt 钳位 ≤ 1s；
 * - 温度钳位 [-30, 60] °C；
 * - 每步检查 NaN/Infinity，非法值回退上一步（绝不把 NaN 传给 UI）。
 */

export interface ThermalParams {
  /** 箱内热容量 J/K（空气+搁架+内壁的教学估值） */
  C_cab: number
  /** 冷凝器热容量 J/K（金属盘管小热容） */
  C_cond: number
  /** 墙体漏热系数 W/K（U*A， teaching 估值） */
  UA_wall: number
  /** 环境温度 °C */
  T_env: number
  /** 设定温度 °C */
  T_setpoint: number
  /** 温控器回差 °C（hysteresis） */
  hysteresis: number
  /** 压缩机运行时蒸发器制冷量 W */
  Q_evap_max: number
  /** 压缩机耗电功率 W（全部变成热，排到冷凝器） */
  W_comp: number
  /** 冷凝器风扇换热系数 W/K */
  UA_cond: number
}

export interface ThermalState {
  T_cab: number
  T_cond: number
  compressorOn: boolean
  /** 累计计数器（教学用） */
  cumulative: {
    Q_leak: number
    Q_evap: number
    Q_cond: number
    W_comp: number
    energyWh: number
  }
}

export const DEFAULT_THERMAL_PARAMS: ThermalParams = {
  C_cab: 4200,
  C_cond: 240,
  UA_wall: 0.9,
  T_env: 25,
  T_setpoint: 4,
  hysteresis: 1.5,
  Q_evap_max: 85,
  W_comp: 55,
  UA_cond: 12,
}

function clamp(v: number, lo: number, hi: number): number {
  if (!Number.isFinite(v)) return lo
  return v < lo ? lo : v > hi ? hi : v
}

function sane(v: number): boolean {
  return Number.isFinite(v)
}

export function createLumpedThermalModel(params: Partial<ThermalParams> = {}) {
  const p: ThermalParams = { ...DEFAULT_THERMAL_PARAMS, ...params }

  let state: ThermalState = {
    T_cab: p.T_env - 2,
    T_cond: p.T_env,
    compressorOn: false,
    cumulative: { Q_leak: 0, Q_evap: 0, Q_cond: 0, W_comp: 0, energyWh: 0 },
  }

  /** 温控器迟滞控制：高于上限开、低于下限关 */
  function thermostat(tCab: number, currentlyOn: boolean): boolean {
    const hi = p.T_setpoint + p.hysteresis
    const lo = p.T_setpoint - p.hysteresis
    if (!currentlyOn) return tCab >= hi
    return tCab > lo
  }

  /**
   * 推进一个时间步。dt 单位秒，内部钳位到 [0, 1] 防爆炸。
   * 返回新状态（不可变快照）。
   */
  function step(dt: number): ThermalState {
    const h = clamp(dt, 0, 1)
    if (h <= 0) return snapshot()

    const prev = state
    const compressorOn = thermostat(prev.T_cab, prev.compressorOn)

    // 热流（W）
    // 1. 环境经墙体漏进箱内
    const Q_leak = p.UA_wall * (p.T_env - prev.T_cab)
    // 2. 蒸发器从箱内吸热（压缩机开时才工作）
    const Q_evap = compressorOn ? p.Q_evap_max : 0
    // 3. 冷凝器排热 = 蒸发器吸热 + 压缩机耗电（能量守恒教学）
    const Q_cond = compressorOn ? Q_evap + p.W_comp : 0
    // 4. 冷凝器风扇向环境排热
    const Q_reject = p.UA_cond * (prev.T_cond - p.T_env)

    // ODE 显式 Euler
    let dT_cab = (Q_leak - Q_evap) / p.C_cab
    let dT_cond = (Q_cond - Q_reject) / p.C_cond

    let T_cab = prev.T_cab + dT_cab * h
    let T_cond = prev.T_cond + dT_cond * h

    // 数值护栏
    T_cab = clamp(T_cab, -30, 60)
    T_cond = clamp(T_cond, -20, 80)
    if (!sane(T_cab) || !sane(T_cond)) {
      // 退化步：保持上一步，绝不传播 NaN
      return snapshot()
    }

    // 累计（Wh：W * s / 3600）
    const dWh = (compressorOn ? p.W_comp : 0) * h / 3600

    state = {
      T_cab,
      T_cond,
      compressorOn,
      cumulative: {
        Q_leak: prev.cumulative.Q_leak + Q_leak * h,
        Q_evap: prev.cumulative.Q_evap + Q_evap * h,
        Q_cond: prev.cumulative.Q_cond + Q_cond * h,
        W_comp: prev.cumulative.W_comp + (compressorOn ? p.W_comp * h : 0),
        energyWh: prev.cumulative.energyWh + dWh,
      },
    }
    return snapshot()
  }

  /** 稳态校验（教学面板展示用）：|Q_cond - Q_evap - W_comp| / Q_cond < 0.05 */
  function energyBalanceError(): number {
    const c = state.cumulative
    if (c.Q_cond < 1) return 0
    return Math.abs(c.Q_cond - c.Q_evap - c.W_comp) / c.Q_cond
  }

  function snapshot(): ThermalState {
    return {
      T_cab: state.T_cab,
      T_cond: state.T_cond,
      compressorOn: state.compressorOn,
      cumulative: { ...state.cumulative },
    }
  }

  /** 重置到环境温度（演示模式切换时用） */
  function reset(): void {
    state = {
      T_cab: p.T_env - 2,
      T_cond: p.T_env,
      compressorOn: false,
      cumulative: { Q_leak: 0, Q_evap: 0, Q_cond: 0, W_comp: 0, energyWh: 0 },
    }
  }

  return { step, snapshot, energyBalanceError, reset, params: { ...p } }
}

export type LumpedThermalModel = ReturnType<typeof createLumpedThermalModel>
