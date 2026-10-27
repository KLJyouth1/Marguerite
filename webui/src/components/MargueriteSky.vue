<template>
  <!--
    玛格丽特 · 星夜 的天空画布。
    - 只在 `marguerite-night` 主题激活时挂载（App.vue 控制），其它主题零成本；
    - 元素：确定性星空（大小/色温/闪烁）+ 雏菊花瓣飘落 + 间歇流星 + 指针视差；
    - 降级：`prefers-reduced-motion` 时只画一帧静态星空，不跑 rAF；
    - 隐私/功耗：页面隐藏时暂停，尺寸变化时按 DPR≤1.5 重建，星空/极光为离屏预渲染。
  -->
  <canvas ref="canvasEl" class="marguerite-sky" aria-hidden="true" />
</template>

<script setup lang="ts">
import { onBeforeUnmount, onMounted, ref } from 'vue'
import {
  createPetals,
  createStars,
  nextShootingStarDelay,
  parallaxOffset,
  petalX,
  seededRandom,
  starAlpha,
  stepPetal,
  type Petal,
  type Star,
} from '@/utils/skyMath'

const canvasEl = ref<HTMLCanvasElement | null>(null)

const SEED = 0x5f3a91 // 固定种子：同一片天空

/**
 * 调色板全部来自主题语义令牌，源码里不出现任何颜色字面量：
 * 与 `ParticleField.vue` 同一套约定（探针元素 + getComputedStyle）。
 * 透明度一律用 `ctx.globalAlpha`，因此不需要拼 `rgba(...)` 字符串。
 */
const TOKEN_TONES = ['--text', '--accent', '--accent-secondary'] as const
const TOKEN_PETAL = '--text'
const TOKEN_PETAL_CORE = '--warn'
const TOKEN_NEBULA = ['--accent-secondary', '--queued', '--accent'] as const
const TOKEN_METEOR_HEAD = '--text'
const TOKEN_METEOR_TAIL = '--accent'

let starTones: string[] = []
let petalColor = ''
let petalCoreColor = ''
let meteorHead = ''
let meteorTail = ''

function resolveTokenColor(token: string): string {
  const probe = document.createElement('span')
  probe.style.color = `var(${token})`
  probe.style.position = 'fixed'
  probe.style.left = '-9999px'
  probe.style.top = '-9999px'
  probe.style.pointerEvents = 'none'
  probe.style.opacity = '0'
  document.body.appendChild(probe)
  const resolved = getComputedStyle(probe).color
  document.body.removeChild(probe)
  return resolved || ''
}

function resolvePalette(): void {
  starTones = TOKEN_TONES.map(resolveTokenColor)
  petalColor = resolveTokenColor(TOKEN_PETAL)
  petalCoreColor = resolveTokenColor(TOKEN_PETAL_CORE)
  meteorHead = resolveTokenColor(TOKEN_METEOR_HEAD)
  meteorTail = resolveTokenColor(TOKEN_METEOR_TAIL)
}

let raf = 0
let ctx: CanvasRenderingContext2D | null = null
let stars: Star[] = []
let petals: Petal[] = []
let nebula: HTMLCanvasElement | null = null
let width = 0
let height = 0
let dpr = 1
let lastMs = 0
let startedAt = 0
let nextStarAt = 0
let shooting: { x: number; y: number; vx: number; vy: number; life: number; max: number } | null = null
let pointer = { x: 0, y: 0 }
let reduced = false
let resizeObserver: ResizeObserver | null = null
let rand = seededRandom(SEED)
const randShooting = seededRandom(SEED ^ 0x9e3779b9)

function onPointerMove(e: PointerEvent): void {
  const w = window.innerWidth || 1
  const h = window.innerHeight || 1
  pointer = { x: (e.clientX / w) * 2 - 1, y: (e.clientY / h) * 2 - 1 }
}

function onVisibility(): void {
  if (document.hidden) {
    cancelAnimationFrame(raf)
    raf = 0
  } else if (!reduced && !raf && ctx) {
    lastMs = performance.now()
    raf = requestAnimationFrame(frame)
  }
}

/** 极光与底色一次性画进离屏画布：每帧只 drawImage，避免逐帧创建渐变。 */
function buildNebula(): void {
  if (!width || !height) return
  const off = document.createElement('canvas')
  off.width = Math.max(1, Math.floor(width * 0.5))
  off.height = Math.max(1, Math.floor(height * 0.5))
  const g = off.getContext('2d')
  if (!g) return
  const w = off.width
  const h = off.height
  const wash = (x: number, y: number, r: number, color: string, alpha: number) => {
    const grad = g.createRadialGradient(x, y, 0, x, y, r)
    grad.addColorStop(0, color)
    // `transparent` 是 CSS 关键字（不是颜色字面量），渐变末端自然淡出。
    grad.addColorStop(1, 'transparent')
    g.globalAlpha = alpha
    g.fillStyle = grad
    g.fillRect(0, 0, w, h)
    g.globalAlpha = 1
  }
  g.clearRect(0, 0, w, h)
  const nebulaTones = TOKEN_NEBULA.map(resolveTokenColor)
  wash(w * 0.24, h * 0.18, Math.max(w, h) * 0.42, nebulaTones[0], 0.10)
  wash(w * 0.78, h * 0.26, Math.max(w, h) * 0.38, nebulaTones[1], 0.09)
  wash(w * 0.52, h * 0.94, Math.max(w, h) * 0.46, nebulaTones[2], 0.07)
  nebula = off
}

function resize(): void {
  const el = canvasEl.value
  if (!el) return
  width = Math.max(1, window.innerWidth)
  height = Math.max(1, window.innerHeight)
  dpr = Math.min(1.5, window.devicePixelRatio || 1)
  el.width = Math.floor(width * dpr)
  el.height = Math.floor(height * dpr)
  el.style.width = `${width}px`
  el.style.height = `${height}px`
  ctx = el.getContext('2d')
  if (!ctx) return
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0)
  resolvePalette()
  const area = width * height
  const density = area > 1_600_000 ? 1.25 : area > 900_000 ? 1 : 0.78
  rand = seededRandom(SEED)
  stars = createStars(Math.round(120 * density), rand)
  petals = createPetals(Math.round(14 * density), rand)
  buildNebula()
  nextStarAt = performance.now() + nextShootingStarDelay(randShooting)
  drawStatic()
}

function drawStars(t: number): void {
  if (!ctx) return
  for (const s of stars) {
    const alpha = starAlpha(t, s)
    const off = parallaxOffset(pointer.x, pointer.y, 0.35 + s.r * 0.25)
    const x = s.x * width + off.x
    const y = s.y * height * 0.82 + off.y
    ctx.fillStyle = starTones[s.tone] ?? starTones[0]
    ctx.globalAlpha = alpha * 0.9
    ctx.beginPath()
    ctx.arc(x, y, s.r, 0, Math.PI * 2)
    ctx.fill()
    // 大一点的星带一圈极淡光晕，像真的在闪
    if (s.r > 1.35) {
      ctx.globalAlpha = alpha * 0.12
      ctx.beginPath()
      ctx.arc(x, y, s.r * 3.2, 0, Math.PI * 2)
      ctx.fill()
    }
  }
  ctx.globalAlpha = 1
}

function drawPetals(t: number): void {
  if (!ctx) return
  for (const p of petals) {
    const off = parallaxOffset(pointer.x, pointer.y, 0.8)
    const x = petalX(p) * width + off.x
    const y = p.y * height + off.y * 0.5
    ctx.save()
    ctx.translate(x, y)
    ctx.rotate(p.rotation)
    ctx.globalAlpha = p.alpha
    // 一片近白花瓣 + 金色花心（雏菊语汇）；颜色随主题令牌走
    ctx.fillStyle = petalColor
    ctx.beginPath()
    ctx.ellipse(0, 0, p.size, p.size * 0.52, 0, 0, Math.PI * 2)
    ctx.fill()
    ctx.fillStyle = petalCoreColor
    ctx.beginPath()
    ctx.arc(0, 0, Math.max(0.8, p.size * 0.26), 0, Math.PI * 2)
    ctx.fill()
    ctx.restore()
  }
  ctx.globalAlpha = 1
  void t
}

function spawnShooting(): void {
  shooting = {
    x: width * (0.15 + randShooting() * 0.6),
    y: height * (0.05 + randShooting() * 0.25),
    vx: 380 + randShooting() * 220,
    vy: 140 + randShooting() * 90,
    life: 0,
    max: 0.9 + randShooting() * 0.5,
  }
}

function drawShooting(dt: number): void {
  if (!ctx) return
  if (!shooting) return
  shooting.life += dt
  if (shooting.life > shooting.max) {
    shooting = null
    return
  }
  const s = shooting
  const tail = 120 + 60 * Math.min(1, s.life / s.max)
  s.x += s.vx * dt
  s.y += s.vy * dt
  const grad = ctx.createLinearGradient(s.x, s.y, s.x - tail, s.y - tail * 0.37)
  const fade = 1 - s.life / s.max
  grad.addColorStop(0, meteorHead)
  grad.addColorStop(0.35, meteorTail)
  grad.addColorStop(1, 'transparent')
  ctx.globalAlpha = 0.85 * fade
  ctx.strokeStyle = grad
  ctx.lineWidth = 1.6
  ctx.beginPath()
  ctx.moveTo(s.x, s.y)
  ctx.lineTo(s.x - tail, s.y - tail * 0.37)
  ctx.stroke()
  ctx.globalAlpha = 1
}

function drawStatic(): void {
  if (!ctx) return
  ctx.clearRect(0, 0, width, height)
  if (nebula) ctx.drawImage(nebula, 0, 0, width, height)
  const t = 1.2
  drawStars(t)
  drawPetals(t)
}

function frame(now: number): void {
  if (!ctx) return
  const dt = Math.min(0.05, (now - lastMs) / 1000 || 0)
  lastMs = now
  const t = (now - startedAt) / 1000
  ctx.clearRect(0, 0, width, height)
  if (nebula) ctx.drawImage(nebula, 0, 0, width, height)
  drawStars(t)
  petals = petals.map((p) => stepPetal(p, dt, rand))
  drawPetals(t)
  if (now >= nextStarAt) {
    spawnShooting()
    nextStarAt = now + nextShootingStarDelay(randShooting)
  }
  drawShooting(dt)
  raf = requestAnimationFrame(frame)
}

onMounted(() => {
  const media = window.matchMedia?.('(prefers-reduced-motion: reduce)')
  reduced = !!media?.matches
  media?.addEventListener?.('change', (e) => {
    reduced = e.matches
    if (reduced) {
      cancelAnimationFrame(raf)
      raf = 0
      drawStatic()
      document.documentElement.dataset.sky = 'static'
    } else {
      lastMs = performance.now()
      raf = requestAnimationFrame(frame)
      document.documentElement.dataset.sky = 'running'
    }
  })
  resize()
  window.addEventListener('resize', resize, { passive: true })
  window.addEventListener('pointermove', onPointerMove, { passive: true })
  document.addEventListener('visibilitychange', onVisibility)
  if (typeof ResizeObserver !== 'undefined' && canvasEl.value) {
    resizeObserver = new ResizeObserver(() => resize())
    resizeObserver.observe(document.documentElement)
  }
  if (reduced) {
    document.documentElement.dataset.sky = 'static'
    return
  }
  startedAt = performance.now()
  lastMs = startedAt
  document.documentElement.dataset.sky = 'running'
  raf = requestAnimationFrame(frame)
})

onBeforeUnmount(() => {
  cancelAnimationFrame(raf)
  raf = 0
  window.removeEventListener('resize', resize)
  window.removeEventListener('pointermove', onPointerMove)
  document.removeEventListener('visibilitychange', onVisibility)
  resizeObserver?.disconnect()
  resizeObserver = null
  delete document.documentElement.dataset.sky
  stars = []
  petals = []
  nebula = null
  ctx = null
})
</script>

<style scoped>
.marguerite-sky {
  position: fixed;
  inset: 0;
  z-index: 0;
  pointer-events: none;
  contain: strict;
}
</style>
