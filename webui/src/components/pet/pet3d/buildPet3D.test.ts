import { describe, expect, it } from 'vitest'
import * as THREE from 'three'
import { DEFAULT_PET_SPEC, ELF_SPECS, PET_TOON_RAMP_WIDTH } from '../petSpec'
import { buildPet3D, disposePet3D, toonGradientMap } from './buildPet3D'

const SPECS = [DEFAULT_PET_SPEC, ...Object.values(ELF_SPECS)]

function namesOf(root: THREE.Object3D): string[] {
  const out: string[] = []
  root.traverse((o) => { if (o.name) out.push(o.name) })
  return out
}

function meshCount(root: THREE.Object3D): number {
  let n = 0
  root.traverse((o) => { if ((o as THREE.Mesh).isMesh) n += 1 })
  return n
}

describe('buildPet3D — 参数化建模', () => {
  it('五款皮肤都能建模，且部件齐全（兽形/人形各自必需件）', () => {
    for (const spec of SPECS) {
      const parts = buildPet3D(spec)
      const names = namesOf(parts.root)

      expect(parts.root.children.length, `${spec.id} 根节点为空`).toBeGreaterThan(0)
      expect(parts.height, `${spec.id} 高度非法`).toBeGreaterThan(0.8)
      expect(Number.isFinite(parts.headY)).toBe(true)
      expect(meshCount(parts.root), `${spec.id} 网格过少`).toBeGreaterThan(20)

      for (const required of ['body', 'head', 'earL', 'earR', 'eyeL', 'eyeR', 'pupilL', 'pupilR', 'browL', 'browR']) {
        expect(names, `${spec.id} 缺少部件 ${required}`).toContain(required)
      }
      expect(names.some((n) => n.startsWith('mouth-')), `${spec.id} 缺少嘴部件`).toBe(true)
      expect(parts.blush.length).toBe(2)

      if (spec.kind === 'beast') {
        expect(parts.tail, '兽形必须有尾巴').not.toBeNull()
        expect(parts.dress).toBeNull()
        expect(parts.armL).toBeNull()
      } else {
        expect(parts.hairBack, '人形必须有后发').not.toBeNull()
        expect(parts.dress, '人形必须有裙').not.toBeNull()
        expect(parts.armL && parts.armR, '人形必须双臂').toBeTruthy()
        expect(parts.wingL && parts.wingR, '人形必须有蝶翼（4 片）').toBeTruthy()
        expect(parts.ornament, '人形必须有花冠').not.toBeNull()
        expect(parts.halo, '人形必须有光环').not.toBeNull()
        // 4 片翼：左右各 2 片
        const wingMeshes = names.filter((n) => n.startsWith('wingUpper') || n.startsWith('wingLower'))
        expect(wingMeshes.length).toBe(4)
        // 裙摆/裙身存在
        expect(names).toContain('dress-skirt')
      }

      disposePet3D(parts)
    }
  })

  it('所有几何的包围盒都是有限值（无 NaN / 无无穷大）', () => {
    for (const spec of SPECS) {
      const parts = buildPet3D(spec)
      const box = new THREE.Box3().setFromObject(parts.root)
      expect(Number.isFinite(box.min.x) && Number.isFinite(box.max.y), `${spec.id} 包围盒非法`).toBe(true)
      expect(box.max.y - box.min.y, `${spec.id} 包围盒高度异常`).toBeGreaterThan(0.5)
      expect(box.max.y - box.min.y, `${spec.id} 包围盒过高`).toBeLessThan(6)
      // 底部不穿地：脚/爪不应低于 -0.05
      expect(box.min.y).toBeGreaterThan(-0.05)
      disposePet3D(parts)
    }
  })

  it('背面是真实几何：后发/裙背有实体（非镜像贴图）', () => {
    const elf = buildPet3D(ELF_SPECS.blue)
    const backBox = new THREE.Box3()
    elf.hairBack?.traverse((o) => {
      const m = o as THREE.Mesh
      if (m.isMesh) backBox.expandByObject(m)
    })
    expect(backBox.isEmpty()).toBe(false)
    // 后发体积必须伸到模型背部（z < 0）
    expect(backBox.min.z).toBeLessThan(-0.05)
    const dress = elf.dress as THREE.Object3D
    const dressBox = new THREE.Box3().setFromObject(dress)
    expect(dressBox.min.z).toBeLessThan(-0.2)
    disposePet3D(elf)
  })

  it('卡通渐变贴图是 RGBA 六档（RGBFormat 陷阱的回归锚点）', () => {
    const tex = toonGradientMap()
    expect(tex.image.width).toBe(PET_TOON_RAMP_WIDTH)
    expect(tex.image.height).toBe(1)
    expect(tex.format).toBe(THREE.RGBAFormat)
    expect((tex.image.data as Uint8Array).length).toBe(PET_TOON_RAMP_WIDTH * 4)
  })

  it('重复 build/dispose 不会共享可变状态（每只宠物独立材质）', () => {
    const a = buildPet3D(DEFAULT_PET_SPEC)
    const b = buildPet3D(DEFAULT_PET_SPEC)
    const matA = (a.head.children.find((c) => c.name === 'headBall') as THREE.Mesh).material
    const matB = (b.head.children.find((c) => c.name === 'headBall') as THREE.Mesh).material
    expect(matA).not.toBe(matB)
    disposePet3D(a)
    disposePet3D(b)
  })
})
