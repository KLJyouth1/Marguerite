/**
 * 共享柔光工具（桌宠 / 岛屿共用）—— 用「径向渐变贴图 + 叠加混合的 Sprite」模拟柔光，
 * 刻意**不引入** EffectComposer / bloom 后处理管线：那会带来包体与兼容风险，而这两处
 * 都是常驻桌面 / 常驻页面的场景，必须零管线风险。
 *
 * 放在独立模块而非某个场景目录下，是因为岛屿与桌宠都要用同一张贴图与同一套材质契约；
 * 各自复制一份会让「光晕该多亮、是否写深度」这类约定漂移。
 */

import * as THREE from 'three'

/**
 * 径向柔光贴图（CanvasTexture）：日月圆盘、灯火光晕、桌宠光环光晕共用一张。
 * 外圈恒为全透明，靠 `AdditiveBlending` 叠加出「发光」而非「贴片」的观感。
 */
export function createRadialGlowTexture(
  inner = 'rgba(255,255,255,1)',
  mid = 'rgba(255,255,255,0.35)',
): THREE.CanvasTexture {
  const size = 128
  const canvas = document.createElement('canvas')
  canvas.width = size
  canvas.height = size
  const ctx = canvas.getContext('2d')
  if (ctx) {
    const gradient = ctx.createRadialGradient(size / 2, size / 2, 0, size / 2, size / 2, size / 2)
    gradient.addColorStop(0, inner)
    gradient.addColorStop(0.45, mid)
    gradient.addColorStop(1, 'rgba(255,255,255,0)')
    ctx.fillStyle = gradient
    ctx.fillRect(0, 0, size, size)
  }
  const texture = new THREE.CanvasTexture(canvas)
  texture.colorSpace = THREE.SRGBColorSpace
  return texture
}

/**
 * 构建柔光精灵（**纯构造**，不触碰 WebGL 上下文 → 可在无 GPU 的单测里断言材质契约）。
 *
 * - `AdditiveBlending`：叠加混合才有发光观感（普通混合会把光晕变成一块灰贴片）
 * - `depthWrite = false`：光晕是后置光效，不写深度，否则会遮挡同位置的实体件
 * - `radius` 是**半径**，Sprite 的 scale 是边长 → 乘 2 得到直径
 */
export function createGlowSprite(
  map: THREE.Texture,
  color: number,
  radius: number,
  opacity: number,
): THREE.Sprite {
  const sprite = new THREE.Sprite(new THREE.SpriteMaterial({
    map,
    color,
    transparent: true,
    opacity,
    blending: THREE.AdditiveBlending,
    depthWrite: false,
  }))
  sprite.scale.set(radius * 2, radius * 2, 1)
  return sprite
}
