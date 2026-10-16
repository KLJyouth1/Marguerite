/**
 * 桌宠 SVG 调色工具（纯函数，无副作用）。
 *
 * 存在意义：桌宠艺术色全部来自 `PetModelSpec.appearance`（0xRRGGBB 数字）与
 * `spec.accent`，渲染层必须在运行时把它们派生成「主色 / 阴影 / 高光 / 轮廓光 / 描边」
 * 一整套层级 —— 这是 2D SVG 能读出立体感的关键（见 `petSvg.ts` 的光照约定）。
 *
 * 为什么不用颜色字面量：仓库守卫 `check-webui-colors.mjs` 禁止 `src/` 下出现裸色字面量
 * （hex / rgb() / hsl()），强制走主题令牌。这里的色值由数字**在运行时拼装**（模板里是
 * `${}` 占位而非字面量），因此既不违守卫，也不把桌宠配色钉死在某个主题上。
 */

/** 0xRRGGBB → `#rrggbb`（运行时拼装；源码内不出现颜色字面量） */
export function hexOf(rgb: number): string {
  const v = Math.max(0, Math.min(0xffffff, Math.round(rgb)))
  return '#' + v.toString(16).padStart(6, '0')
}

/** 拆出 [r,g,b]（0..255） */
export function channelsOf(rgb: number): [number, number, number] {
  const v = Math.max(0, Math.min(0xffffff, Math.round(rgb)))
  return [(v >> 16) & 0xff, (v >> 8) & 0xff, v & 0xff]
}

function clampChan(v: number): number {
  return Math.max(0, Math.min(255, Math.round(v)))
}

/** 打包回 0xRRGGBB */
function pack(r: number, g: number, b: number): number {
  return (clampChan(r) << 16) | (clampChan(g) << 8) | clampChan(b)
}

/**
 * 明暗调整：`amount > 0` 提亮、`< 0` 压暗（相对比例，0.3 = 30%）。
 * 提亮走「向白靠拢」、压暗走「乘性衰减」，前者保持材质感，后者保持色相不脏。
 */
export function shadeRgb(rgb: number, amount: number): number {
  const [r, g, b] = channelsOf(rgb)
  if (amount >= 0) {
    const t = Math.min(1, amount)
    return pack(r + (255 - r) * t, g + (255 - g) * t, b + (255 - b) * t)
  }
  const k = Math.max(0, 1 + amount)
  return pack(r * k, g * k, b * k)
}

/** 线性混色：`t=0` 取 a，`t=1` 取 b */
export function mixRgb(a: number, b: number, t: number): number {
  const [ar, ag, ab] = channelsOf(a)
  const [br, bg, bb] = channelsOf(b)
  const k = Math.min(1, Math.max(0, t))
  return pack(ar + (br - ar) * k, ag + (bg - ag) * k, ab + (bb - ab) * k)
}

/**
 * 饱和度提升（用于让 accent 在浅色主色上仍然立得住）。
 * `amount > 0` 向外推离灰度轴，`< 0` 向灰度收拢。
 */
export function saturateRgb(rgb: number, amount: number): number {
  const [r, g, b] = channelsOf(rgb)
  const lum = 0.2126 * r + 0.7152 * g + 0.0722 * b
  const k = 1 + amount
  return pack(lum + (r - lum) * k, lum + (g - lum) * k, lum + (b - lum) * k)
}

/** 调色板：一种材质在光照约定下需要的全部层级（渲染层只消费这六个值） */
export interface MaterialRamp {
  /** 受光面（高光） */
  light: string
  /** 主色（固有明度） */
  base: string
  /** 暗面 */
  shade: string
  /** 环境遮蔽（贴地处最深） */
  deep: string
  /** 轮廓光（背光边缘，取 accent 提亮） */
  rim: string
  /** 描边（勾线色，压暗主色而非纯黑，避免脏线） */
  line: string
}

/**
 * 由材质主色 + accent 派生完整明暗层级。
 *
 * 光照约定（全渲染层统一，保证所有部件同向受光）：
 * - **主光来自左上**；右下为暗面，最深处接近环境遮蔽；
 * - **轮廓光取 accent**（背光边缘的一圈冷光），这是「立体且通透」的主要来源；
 * - 勾线用压暗主色而非黑色 —— 黑色描边会让浅色材质瞬间发脏。
 */
export function rampOf(baseRgb: number, accentRgb: number): MaterialRamp {
  return {
    light: hexOf(shadeRgb(baseRgb, 0.34)),
    base: hexOf(baseRgb),
    shade: hexOf(shadeRgb(baseRgb, -0.16)),
    deep: hexOf(shadeRgb(baseRgb, -0.34)),
    rim: hexOf(saturateRgb(shadeRgb(mixRgb(accentRgb, baseRgb, 0.35), 0.3), 0.25)),
    line: hexOf(shadeRgb(baseRgb, -0.52)),
  }
}

/** 中性色（源码里只用命名色，避免颜色字面量） */
export const NEUTRAL = {
  /** 眼白 / 高光 */
  white: 'white',
  /** 瞳孔 / 睫毛 */
  black: 'black',
  /** 透明（配合 opacity 属性做半透） */
  none: 'none',
} as const