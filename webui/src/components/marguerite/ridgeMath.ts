/**
 * 「玛格丽特 · 星夜」3D 山脊场景的地形与天光数学（纯函数，可单测）。
 *
 * 场景取材自真实地形与真实天光（都只取"语汇"，不复制任何照片）：
 *   · 玛格丽特王后补给站（Capanna Regina Margherita，4554 m，Punta Gnifetti 峰顶）：
 *     长 20.5 m / 宽 8 m / 高 7 m 的长条木构、整体铜皮包覆，长边有木露台，屋顶常年积雪；
 *   · Punta Gnifetti 是金字塔形峰，缓坡存雪、陡壁裸岩，冰川致密冰呈蓝色；
 *   · 日出前天顶转「萨伏依蓝」、地平线由红转橙，云海受光面珊瑚粉、背光面蓝灰；
 *   · 南侧（Po 平原）地平线有城市光穹，银河低空部分被削弱。
 *
 * 坐标系：+x 向右、+y 向上、+z 朝相机（相机在 +z 看向 -z）。单位≈米。
 */

/** 云海平面高度（地形低于它就被云遮住，形成"云海翻涌"）。 */
export const CLOUD_SEA_Y = 0

/** 高斯包（山体轮廓的基础形状）。 */
export function gauss(d: number, sigma: number): number {
  return Math.exp(-(d * d) / (2 * sigma * sigma))
}

/** 金字塔形主峰的水平剖面：中间高、两侧按幂次收窄。 */
export function pyramidProfile(x: number, halfWidth = 42, power = 1.55): number {
  const t = Math.max(0, 1 - Math.abs(x) / halfWidth)
  return Math.pow(t, power)
}

/**
 * 山脊高度场。
 *
 * 三段结构（从相机往远处）：
 *   1. 前景肩脊 z≈+8：登山队结组行走的那道雪脊，是画面下缘的剪影；
 *   2. 营地鞍部 z≈-10：补给站所在的小平台；
 *   3. 主峰 z≈-44：Punta Gnifetti 的金字塔尖。
 * 中间用两个"谷"把高度压到云海平面以下，云才会在山坳里翻涌。
 */
export function ridgeHeight(x: number, z: number): number {
  const shoulder = 6.2 * gauss(z - 8, 9) * (0.82 + 0.18 * Math.sin(x * 0.085))
  const knoll = 10.4 * gauss(z + 10, 9.5) * (0.93 + 0.07 * Math.cos(x * 0.05))
  const summit = 28 * gauss(z + 44, 17) * pyramidProfile(x)
  // 右侧次峰（画面右半的日出方向要有起伏，太阳才不会悬在空处）
  const rightRidge = 7.5 * gauss(z + 30, 14) * gauss(x - 42, 20)
  // 两个山坳：把中间压到云海以下（云会从鞍部缺口"漫"进来，这是高山云海的真实形态）
  const gully = -14 * gauss(z + 1, 5.2) - 9.5 * gauss(z + 27, 7)
  // 起伏细节（确定性，不用随机数，保证每次打开山形一致）
  const detail =
    1.15 * Math.sin(x * 0.075 + z * 0.05) +
    0.62 * Math.sin(x * 0.19 - z * 0.11) +
    0.34 * Math.sin(x * 0.44 + z * 0.27) +
    0.3 * Math.sin(z * 0.31)
  return shoulder + knoll + summit + rightRidge + gully + detail
}

/** 有限差分的法线 y 分量：>0.55 视为缓坡（积雪），否则裸岩。 */
export function slopeUpness(x: number, z: number, eps = 0.6): number {
  const dx = (ridgeHeight(x + eps, z) - ridgeHeight(x - eps, z)) / (2 * eps)
  const dz = (ridgeHeight(x, z + eps) - ridgeHeight(x, z - eps)) / (2 * eps)
  return 1 / Math.sqrt(1 + dx * dx + dz * dz)
}

/** 补给站落位（营地鞍部偏左三分线，给主峰与日出方向让出中心）。 */
export const HUT_X = -21
export const HUT_Z = -16
export const HUT_LENGTH = 20.5
export const HUT_WIDTH = 8
export const HUT_HEIGHT = 7

/** 补给站底座高度：贴在鞍部地形上，略微下沉 0.4 免得悬空。 */
export function hutBaseY(): number {
  return ridgeHeight(HUT_X, HUT_Z) - 0.4
}

/**
 * 登山队结组路径：在补给站右侧的雪坡上从左向右横切（t∈0..1），全程贴地。
 * 放在 z≈-8 而不是更近的肩脊：再往前就掉进输入框后面，人被面板切掉一半。
 */
export function ropePathPoint(t: number): { x: number; y: number; z: number } {
  const clamped = Math.min(1, Math.max(0, t))
  const x = -8 + clamped * 40
  const z = -8 + Math.sin(clamped * Math.PI) * 1.5
  return { x, y: ridgeHeight(x, z), z }
}

/** 太阳高度角（弧度）。120 s 一个来回：跃出山脊 → 停在高处 → 缓缓落回。 */
export function sunElevation(t: number): number {
  const period = 240
  const phase = ((t % period) + period) % period
  const k = 0.5 - 0.5 * Math.cos((phase / period) * Math.PI * 2)
  return (-6 + 16 * k) * (Math.PI / 180)
}

/** 晨曦强度 0..1：太阳刚跃出山脊时最暖，升到高处后回落。 */
export function dawnIntensity(t: number): number {
  const elevDeg = (sunElevation(t) * 180) / Math.PI
  const rise = Math.min(1, Math.max(0, (elevDeg + 6) / 8))
  const fade = Math.min(1, Math.max(0, (10 - elevDeg) / 7))
  return Math.min(1, rise * 0.55 + rise * fade * 0.45)
}

/** 太阳方向（单位向量的方向分量，相机空间外由渲染层乘距离）。 */
export function sunDirection(t: number): { x: number; y: number; z: number } {
  const elev = sunElevation(t)
  const az = (16 * Math.PI) / 180 // 偏右，正对画面右侧的山口
  return {
    x: Math.sin(az) * Math.cos(elev),
    y: Math.sin(elev),
    z: -Math.cos(az) * Math.cos(elev),
  }
}

/** 登山队员在队伍中的相对位置（0=队首，1=队尾），间距按真实结组 10–15 m 压缩到画面尺度。 */
export function teamMemberOffset(index: number, spacing = 3.1): number {
  return index * spacing
}
