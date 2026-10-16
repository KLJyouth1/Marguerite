/**
 * @vitest-environment happy-dom
 *
 * 柔光工具单测：只断言**材质契约**（构造 Sprite/SpriteMaterial 不需要 WebGL 上下文），
 * 因此无 GPU 的 CI 也能真正跑起来，而不是「跳过即通过」。
 */
import * as THREE from 'three'
import { describe, expect, it } from 'vitest'
import { createGlowSprite, createRadialGlowTexture } from './glowTexture'

describe('柔光工具（桌宠 / 岛屿共用）', () => {
  it('柔光精灵材质契约：叠加混合 + 不写深度 + 贴图/颜色就位', () => {
    const map = createRadialGlowTexture()
    const sprite = createGlowSprite(map, 0x60a5fa, 1.5, 0.4)
    const mat = sprite.material as THREE.SpriteMaterial

    expect(mat.map).toBe(map)
    // 普通混合会把光晕渲染成一块灰贴片；叠加混合才有「发光」观感
    expect(mat.blending).toBe(THREE.AdditiveBlending)
    // 光晕是后置光效：写深度会遮挡同位置的实体件
    expect(mat.depthWrite).toBe(false)
    expect(mat.transparent).toBe(true)
    expect(mat.opacity).toBe(0.4)
    expect(mat.color.getHex()).toBe(0x60a5fa)
    // 入参是**半径**，Sprite 的 scale 是边长 → 应等于直径
    expect(sprite.scale.x).toBeCloseTo(3, 5)
    expect(sprite.scale.y).toBeCloseTo(3, 5)
  })

  it('柔光贴图：128²、sRGB，缺 2D 上下文时也不抛错（只退化为一整块透明）', () => {
    const tex = createRadialGlowTexture('rgba(255,255,255,1)', 'rgba(255,255,255,0.32)')
    expect(tex.colorSpace).toBe(THREE.SRGBColorSpace)
    const image = tex.image as HTMLCanvasElement
    expect(image.width).toBe(128)
    expect(image.height).toBe(128)
    // happy-dom 无 2D 上下文 → 走 `if (ctx)` 守卫，不抛错即可（不冒充贴图内容正确）
    expect(() => tex.dispose()).not.toThrow()
  })
})
