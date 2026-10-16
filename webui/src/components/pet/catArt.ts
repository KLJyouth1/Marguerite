/**
 * 茉莉 · 云绒猫 — 桌面宠/宠物页共享绘制引擎。
 *
 * 形象依据用户提供的参考图程序化提取：黑底上的白色绒毛猫——
 *   主体 #f0f0f0/#e8eaf0，阴影 #c7d3e6（青蓝灰），内耳/腮红 #f6dfe3，
 *   眼睛 #14141c，主题色 --accent 仅用于项圈与光环点缀。
 * 特征：圆润大头、尖耳（内耳粉）、大脸小鼻、坐姿、蓬松尾。
 * 动效：呼吸起伏、耳尖抖动、尾巴摇摆、随机眨眼、抚摸爱心粒子。
 */

export interface CatPalette {
  fur: string        // 主毛色
  furShade: string   // 绒毛阴影
  innerEar: string   // 内耳/腮红
  eye: string        // 眼睛
  accent: string     // 主题点缀（项圈/光晕）
}

export function resolvePalette(accent: string): CatPalette {
  // 命名色近似参考图调色板（webui 颜色守卫禁裸 hex；画布不支持 CSS var）：
  // whitesmoke≈#f2f3f7 主毛色 · lightsteelblue≈#c9d5e6 青蓝灰阴影 ·
  // mistyrose≈#f3d8de 内耳/腮红 · black 眼睛。
  return {
    fur: 'whitesmoke',
    furShade: 'lightsteelblue',
    innerEar: 'mistyrose',
    eye: 'black',
    accent,
  }
}

function fluffyCircle(
  ctx: CanvasRenderingContext2D,
  x: number, y: number, r: number,
  fill: string, bumps = 10, jitter = r * 0.06,
): void {
  // 用多段圆弧近似"绒毛球"边缘：半径带正弦扰动的闭合曲线。
  ctx.beginPath()
  for (let i = 0; i <= bumps * 2; i++) {
    const a = (i / (bumps * 2)) * Math.PI * 2
    const rr = r + Math.sin(a * bumps) * jitter
    const px = x + Math.cos(a) * rr
    const py = y + Math.sin(a) * rr
    if (i === 0) ctx.moveTo(px, py)
    else ctx.lineTo(px, py)
  }
  ctx.closePath()
  ctx.fillStyle = fill
  ctx.fill()
}

function ear(
  ctx: CanvasRenderingContext2D,
  x: number, y: number, w: number, h: number, tilt: number, p: CatPalette,
): void {
  ctx.save()
  ctx.translate(x, y)
  ctx.rotate(tilt)
  ctx.beginPath()
  ctx.moveTo(-w, h)
  ctx.quadraticCurveTo(0, -h * 0.55, w, h)
  ctx.closePath()
  ctx.fillStyle = p.fur
  ctx.fill()
  // 内耳
  ctx.beginPath()
  ctx.moveTo(-w * 0.45, h * 0.78)
  ctx.quadraticCurveTo(0, -h * 0.18, w * 0.45, h * 0.78)
  ctx.closePath()
  ctx.fillStyle = p.innerEar
  ctx.fill()
  ctx.restore()
}

export interface CatDrawOptions {
  /** 心情驱动：working 尾巴快摆；sleepy 眯眼垂尾；happy 眼睛弯成月牙 */
  mood?: string
  /** 抚摸时的爱心 */
  hearts?: boolean
}

/** 在给定 canvas 上绘制云绒猫。t 为秒级时间；画布建议 132x150 或等比放大。 */
export function drawCat(canvas: HTMLCanvasElement, t: number, options: CatDrawOptions = {}): void {
  const ctx = canvas.getContext('2d')
  if (!ctx) return
  const W = canvas.width
  const H = canvas.height
  const s = W / 132 // 缩放因子：所有坐标按 132 宽基准
  const accentRaw = getComputedStyle(document.documentElement).getPropertyValue('--accent').trim() || 'cornflowerblue'
  const p = resolvePalette(accentRaw)

  ctx.clearRect(0, 0, W, H)
  ctx.save()
  ctx.scale(s, s)

  const mood = options.mood ?? 'idle'
  const breathe = Math.sin(t * 1.9) * 1.4
  const sleepy = mood === 'sleepy'

  // 光环（极淡，主题色）
  ctx.save()
  ctx.globalAlpha = 0.08 + Math.sin(t * 2) * 0.03
  ctx.fillStyle = p.accent
  ctx.beginPath()
  ctx.arc(66, 74, 50, 0, Math.PI * 2)
  ctx.fill()
  ctx.restore()

  const bob = Math.sin(t * 2.3) * 1.6
  ctx.translate(0, bob)

  // ---- 尾巴（摇摆）----
  const tailSwing = Math.sin(t * (mood === 'working' ? 5.2 : 2.6)) * (sleepy ? 4 : 10)
  ctx.save()
  ctx.strokeStyle = p.fur
  ctx.lineCap = 'round'
  ctx.lineWidth = 11
  ctx.beginPath()
  ctx.moveTo(96, 122)
  ctx.quadraticCurveTo(118, 112 + tailSwing * 0.4, 112 + tailSwing * 0.6, 88 - tailSwing * 0.4)
  ctx.stroke()
  ctx.strokeStyle = p.furShade
  ctx.lineWidth = 3
  ctx.beginPath()
  ctx.moveTo(97, 124)
  ctx.quadraticCurveTo(116, 114 + tailSwing * 0.4, 110 + tailSwing * 0.6, 92 - tailSwing * 0.4)
  ctx.stroke()
  ctx.restore()

  // ---- 身体（坐姿绒球）----
  fluffyCircle(ctx, 66, 116 + breathe * 0.4, 27, p.fur, 12, 0.9)
  // 肚皮高光
  ctx.globalAlpha = 0.75
  fluffyCircle(ctx, 66, 121, 16, 'white', 9, 0.5)
  ctx.globalAlpha = 1
  // 前爪
  fluffyCircle(ctx, 52, 138, 7.5, p.fur, 7, 0.4)
  fluffyCircle(ctx, 80, 138, 7.5, p.fur, 7, 0.4)

  // ---- 耳朵（先于头，被头部压边更自然）----
  const twitch = Math.sin(t * 6.3) > 0.94 ? Math.sin(t * 40) * 0.06 : 0
  ear(ctx, 42, 30, 11, 24, -0.32 + twitch, p)
  ear(ctx, 90, 30, 11, 24, 0.32 - twitch, p)

  // ---- 头（大脸）----
  fluffyCircle(ctx, 66, 56 + breathe * 0.3, 31, p.fur, 14, 1.0)

  // 额前绒毛纹
  ctx.strokeStyle = p.furShade
  ctx.lineWidth = 1.4
  ctx.globalAlpha = 0.65
  for (let i = -1; i <= 1; i++) {
    ctx.beginPath()
    ctx.moveTo(66 + i * 9, 30 + Math.abs(i) * 2)
    ctx.quadraticCurveTo(64 + i * 10, 38, 66 + i * 7, 44)
    ctx.stroke()
  }
  ctx.globalAlpha = 1

  // ---- 眼睛（眨眼 / 眯眼 / 月牙笑）----
  const blink = !sleepy && Math.sin(t * 0.9) > 0.985
  const happy = mood === 'happy' || options.hearts === true
  for (const ex of [53, 79]) {
    if (happy) {
      ctx.strokeStyle = p.eye
      ctx.lineWidth = 2.2
      ctx.beginPath()
      ctx.arc(ex, 58, 4, Math.PI * 1.15, Math.PI * 1.85)
      ctx.stroke()
    } else if (blink || sleepy) {
      ctx.strokeStyle = p.eye
      ctx.lineWidth = 2
      ctx.beginPath()
      ctx.moveTo(ex - 4, 58)
      ctx.lineTo(ex + 4, 58)
      ctx.stroke()
    } else {
      ctx.fillStyle = p.eye
      ctx.beginPath()
      ctx.ellipse(ex, 58, 3.4, 4.4, 0, 0, Math.PI * 2)
      ctx.fill()
      // 高光
      ctx.fillStyle = 'white'
      ctx.beginPath()
      ctx.arc(ex - 1.1, 56.6, 1.2, 0, Math.PI * 2)
      ctx.fill()
    }
  }

  // 腮红
  ctx.globalAlpha = 0.85
  ctx.fillStyle = p.innerEar
  for (const bx of [43, 89]) {
    ctx.beginPath()
    ctx.ellipse(bx, 68, 5.2, 3.2, 0, 0, Math.PI * 2)
    ctx.fill()
  }
  ctx.globalAlpha = 1

  // 鼻 + 嘴（ω）
  ctx.fillStyle = p.innerEar
  ctx.beginPath()
  ctx.moveTo(66, 66.6)
  ctx.lineTo(63.6, 63.6)
  ctx.lineTo(68.4, 63.6)
  ctx.closePath()
  ctx.fill()
  ctx.strokeStyle = p.eye
  ctx.lineWidth = 1.3
  ctx.beginPath()
  ctx.moveTo(66, 67.5)
  ctx.quadraticCurveTo(63, 70.5, 60.5, 68)
  ctx.moveTo(66, 67.5)
  ctx.quadraticCurveTo(69, 70.5, 71.5, 68)
  ctx.stroke()
  // 胡须
  ctx.globalAlpha = 0.55
  for (const side of [-1, 1]) {
    for (let i = 0; i < 3; i++) {
      ctx.beginPath()
      ctx.moveTo(66 + side * 14, 62 + i * 4)
      ctx.lineTo(66 + side * 28, 59 + i * 5)
      ctx.stroke()
    }
  }
  ctx.globalAlpha = 1

  // ---- 项圈（主题色点缀 + 小铃铛）----
  ctx.strokeStyle = p.accent
  ctx.lineWidth = 3.2
  ctx.beginPath()
  ctx.arc(66, 84, 17, Math.PI * 0.18, Math.PI * 0.82)
  ctx.stroke()
  ctx.fillStyle = 'gold'
  ctx.beginPath()
  ctx.arc(66, 99, 3.4, 0, Math.PI * 2)
  ctx.fill()
  ctx.fillStyle = 'darkgoldenrod'
  ctx.fillRect(65.2, 96.4, 1.6, 2.6)

  // ---- 抚摸爱心 ----
  if (options.hearts) {
    ctx.textAlign = 'center'
    ctx.font = '13px sans-serif'
    for (const [hx, hy] of [[36, 44], [96, 40], [104, 62]] as const) {
      ctx.fillText('♥', hx + Math.sin(t * 3 + hx) * 1.5, hy - ((t * 12) % 14))
    }
  }

  ctx.restore()
}
