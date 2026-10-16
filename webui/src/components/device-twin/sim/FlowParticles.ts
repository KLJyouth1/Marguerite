/**
 * FlowParticles — 通用「设备工作流粒子系统」（纯路径数据 + 粒子推进数学，node 可测）。
 *
 * 原冰箱的三路流体泛化为通用语义通道：
 * - cold   冷流 = 箱内空气循环（青蓝，走 --accent 冷调）
 * - hot    热流 = 向环境排热（暖橙）
 * - working 工质 = 密闭制冷剂（铜色）
 *
 * 渲染层用 CatmullRomCurve3 + TubeGeometry / Points 沿 path 撒粒子。
 * 本文件只管「粒子沿路径走多远、什么速度、什么语义色」，不碰 three。
 *
 * 视觉铁律：禁止粗蓝发光管；细粒子 + 细 Tube，三色语义不得混淆。
 */

export type FlowChannel = 'cold' | 'hot' | 'working'

/** 路径点（世界坐标，渲染层喂给 CatmullRomCurve3） */
export type FlowPath = [number, number, number][]

export interface ParticleChannelSpec {
  channel: FlowChannel
  /** 路径闭合（工质回路）还是开放（箱内出风→回风） */
  closed: boolean
  /** 粒子数（按质量档自动减量） */
  count: number
  /** 归一化流速 0..1（cycle 态才 > 0） */
  speed: number
  /** 路径控制点（至少 2 个） */
  path: FlowPath
}

interface ParticleState {
  /** 沿路径 0..1 的归一化位置 */
  t: number
  /** 通道内速度系数（粒子间错落） */
  phase: number
}

/** 确定性伪随机（同 seed 同结果，测试可重复） */
function seededUnit(seed: number): number {
  const x = Math.sin(seed * 12.9898) * 43758.5453
  return x - Math.floor(x)
}

export function createFlowParticles(spec: ParticleChannelSpec) {
  const particles: ParticleState[] = []
  for (let i = 0; i < spec.count; i++) {
    particles.push({
      t: seededUnit(i + 1),
      phase: 0.7 + seededUnit(i + 100) * 0.6,
    })
  }

  /**
   * 推进粒子。dt 秒；flowActive = 是否处于工作循环态。
   * 返回新的 t 值数组（0..1），渲染层据此在曲线上取点。
   */
  function tick(dt: number, flowActive: boolean): number[] {
    if (!Number.isFinite(dt) || dt <= 0 || !flowActive) {
      return particles.map((p) => p.t)
    }
    const out: number[] = new Array(particles.length)
    for (let i = 0; i < particles.length; i++) {
      const p = particles[i]
      p.t += dt * spec.speed * p.phase * 0.08
      if (spec.closed) {
        // 闭合回路：环形循环
        p.t = p.t % 1
      } else {
        // 开放路径：到 1 后回 0（出风→消失→重新从风口出发）
        if (p.t > 1) p.t = 0
      }
      out[i] = p.t
    }
    return out
  }

  /** 通道是否有粒子在动（cycle 态 && speed>0） */
  function isActive(flowActive: boolean): boolean {
    return flowActive && spec.speed > 0
  }

  return {
    tick,
    isActive,
    channel: spec.channel,
    closed: spec.closed,
    path: spec.path,
    count: particles.length,
  }
}

export type FlowParticles = ReturnType<typeof createFlowParticles>
