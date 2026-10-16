/**
 * 程序化 Canvas 纹理 —— 木桌 / 工程蓝图 / 混凝土 / 路面。
 *
 * 全部由 <canvas> 现场绘制，不下载外部贴图（任务书 §0 铁律）。
 * 仅在浏览器中被 SandboxScene 调用；node 单测不 import 本模块。
 *
 * 配色走“暖棕木桌 + 暖灰混凝土 + 米黄蓝图”：不使用蓝紫冷调。
 * （three.js 材质色值是数字常量，不经过 CSS 色守卫；这里同样用数字/Canvas 像素。）
 */
import * as THREE from 'three'
import { makeSaltedRng } from './rng'

function canvas2d(w: number, h: number): [HTMLCanvasElement, CanvasRenderingContext2D] {
  const canvas = document.createElement('canvas')
  canvas.width = w
  canvas.height = h
  const ctx = canvas.getContext('2d')!
  return [canvas, ctx]
}

/** 暖棕木纹：横向深浅条纹 + 少量节疤。 */
export function makeWoodTexture(seed = 7): THREE.CanvasTexture {
  const [canvas, ctx] = canvas2d(512, 512)
  const rng = makeSaltedRng(seed, 'wood')
  // 底色：暖棕
  const grad = ctx.createLinearGradient(0, 0, 0, 512)
  grad.addColorStop(0, '#8a5a33')
  grad.addColorStop(0.5, '#7a4e2b')
  grad.addColorStop(1, '#855630')
  ctx.fillStyle = grad
  ctx.fillRect(0, 0, 512, 512)
  // 木纹条纹
  for (let i = 0; i < 60; i++) {
    const y = rng() * 512
    const h = 1 + rng() * 4
    const shade = Math.floor(40 + rng() * 50)
    ctx.fillStyle = `rgba(${shade},${Math.floor(shade * 0.62)},${Math.floor(shade * 0.34)},0.25)`
    ctx.fillRect(0, y, 512, h)
  }
  // 节疤
  for (let i = 0; i < 5; i++) {
    const x = rng() * 512
    const y = rng() * 512
    ctx.beginPath()
    ctx.ellipse(x, y, 8 + rng() * 14, 3 + rng() * 5, rng() * Math.PI, 0, Math.PI * 2)
    ctx.fillStyle = 'rgba(50,28,14,0.35)'
    ctx.fill()
  }
  const tex = new THREE.CanvasTexture(canvas)
  tex.wrapS = tex.wrapT = THREE.RepeatWrapping
  tex.colorSpace = THREE.SRGBColorSpace
  return tex
}

/** 工程蓝图：米黄底 + 细网格 + 一条总平面轮廓 + 尺寸线。 */
export function makeBlueprintTexture(seed = 11): THREE.CanvasTexture {
  const [canvas, ctx] = canvas2d(512, 384)
  const rng = makeSaltedRng(seed, 'blueprint')
  // 米黄纸底（暖，不蓝）
  ctx.fillStyle = '#e8dcb8'
  ctx.fillRect(0, 0, 512, 384)
  // 细网格
  ctx.strokeStyle = 'rgba(120,96,50,0.18)'
  ctx.lineWidth = 1
  for (let x = 0; x <= 512; x += 32) {
    ctx.beginPath(); ctx.moveTo(x, 0); ctx.lineTo(x, 384); ctx.stroke()
  }
  for (let y = 0; y <= 384; y += 32) {
    ctx.beginPath(); ctx.moveTo(0, y); ctx.lineTo(512, y); ctx.stroke()
  }
  // 总平面轮廓（圆角矩形环线）
  ctx.strokeStyle = 'rgba(70,52,24,0.8)'
  ctx.lineWidth = 2
  ctx.strokeRect(60, 60, 392, 264)
  // 两个站块
  ctx.fillStyle = 'rgba(70,52,24,0.55)'
  ctx.fillRect(120, 168, 44, 48)
  ctx.fillRect(348, 168, 44, 48)
  // 尺寸线
  ctx.strokeStyle = 'rgba(70,52,24,0.6)'
  ctx.beginPath(); ctx.moveTo(60, 340); ctx.lineTo(452, 340); ctx.stroke()
  ctx.beginPath(); ctx.moveTo(60, 334); ctx.lineTo(60, 346); ctx.stroke()
  ctx.beginPath(); ctx.moveTo(452, 334); ctx.lineTo(452, 346); ctx.stroke()
  // 随机标注点（确定性散布）
  ctx.fillStyle = 'rgba(70,52,24,0.5)'
  for (let i = 0; i < 12; i++) {
    ctx.beginPath()
    ctx.arc(80 + rng() * 350, 80 + rng() * 220, 2, 0, Math.PI * 2)
    ctx.fill()
  }
  const tex = new THREE.CanvasTexture(canvas)
  tex.colorSpace = THREE.SRGBColorSpace
  return tex
}

/** 暖灰混凝土：细噪点。 */
export function makeConcreteTexture(seed = 13): THREE.CanvasTexture {
  const [canvas, ctx] = canvas2d(256, 256)
  const rng = makeSaltedRng(seed, 'concrete')
  ctx.fillStyle = '#9a938a'
  ctx.fillRect(0, 0, 256, 256)
  for (let i = 0; i < 2600; i++) {
    const g = Math.floor(120 + rng() * 40)
    ctx.fillStyle = `rgba(${g},${g - 4},${g - 10},0.4)`
    ctx.fillRect(rng() * 256, rng() * 256, 1.5, 1.5)
  }
  const tex = new THREE.CanvasTexture(canvas)
  tex.wrapS = tex.wrapT = THREE.RepeatWrapping
  tex.colorSpace = THREE.SRGBColorSpace
  return tex
}
