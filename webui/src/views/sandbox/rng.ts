/**
 * 确定性伪随机数发生器（mulberry32）—— 固定种子可精确回放。
 *
 * 沙盘里的“随机”装饰（窗灯、树木、雨丝落点、站名牌细节）都从这里取，
 * 而不是 Math.random()。这样时间轴拖回同一时刻，整座沙盘的随机分布完全一致
 * （任务书 §1.3：固定种子，State From Time）。
 *
 * 纯函数、无 DOM 依赖，可在 node 单测中跑。
 */

/** 沙盘固定种子（任意常量；改它会重排装饰分布，但运行图运动不变）。 */
export const SANDBOX_SEED = 20260928

/** mulberry32 —— 紧凑、无外部依赖、可跨平台复现的 PRNG。 */
export function makeRng(seed: number): () => number {
  let a = seed >>> 0
  return function next(): number {
    a |= 0
    a = (a + 0x6d2b79f5) | 0
    let t = Math.imul(a ^ (a >>> 15), 1 | a)
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296
  }
}

/** 用种子 + 盐值派生一个独立子 RNG（不同装饰系统互不串扰）。 */
export function makeSaltedRng(seed: number, salt: string): () => number {
  let h = seed >>> 0
  for (let i = 0; i < salt.length; i++) {
    h = (Math.imul(h ^ salt.charCodeAt(i), 2654435761)) >>> 0
  }
  return makeRng(h)
}

/** 在 [min, max) 取一个确定性浮点。 */
export function range(rng: () => number, min: number, max: number): number {
  return min + (max - min) * rng()
}

/** 在 [min, max] 取一个确定性整数。 */
export function intRange(rng: () => number, min: number, max: number): number {
  return Math.floor(range(rng, min, max + 1))
}

/** 以固定顺序产出 n 个确定性装饰落点（闭环节点上的稳定散布）。 */
export function seededScatter(seed: number, salt: string, count: number): number[] {
  const rng = makeSaltedRng(seed, salt)
  const out: number[] = []
  for (let i = 0; i < count; i++) out.push(rng())
  return out
}
