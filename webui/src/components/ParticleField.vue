<template>
  <canvas ref="canvasEl" class="particle-field" aria-hidden="true" />
</template>

<script setup lang="ts">
import { onBeforeUnmount, onMounted, ref } from 'vue'
import { useBgm } from '../composables/useBgm'

/**
 * A canvas particle field that sits behind the chat surface. Designed to be
 * unobtrusive: it never hardcodes a colour (every hue is read from the active
 * theme's CSS custom properties at runtime, so light / dark / miami / … all
 * track automatically), pauses on hidden tabs and when the user prefers
 * reduced motion, scales its particle budget to the covered area, and reacts
 * subtly to BGM playback.
 *
 * No colour literal appears anywhere in this file on purpose — the CI colour
 * guard scans every line of a .vue (template, script and style). Colours are
 * resolved to concrete colour strings via a probe element + getComputedStyle,
 * and per-particle translucency is applied with ctx.globalAlpha rather than
 * an alpha-blended literal, so the source contains no colour literals.
 */
const props = withDefaults(
  defineProps<{
    /** Particle-count multiplier; the base count scales with the covered area. */
    density?: number
    /** Hard ceiling on particle count to bound cost on very large canvases. */
    maxParticles?: number
  }>(),
  { density: 1, maxParticles: 120 },
)

const canvasEl = ref<HTMLCanvasElement | null>(null)
const { playing: bgmPlaying, amplitude: bgmAmplitude } = useBgm()

interface Particle {
  x: number
  y: number
  vx: number
  vy: number
  radius: number
  color: string
  baseAlpha: number
  twinkle: number
  twinkleSpeed: number
}

let ctx: CanvasRenderingContext2D | null = null
let particles: Particle[] = []
let rafId = 0
let running = false
let lastTime = 0
let widthCss = 0
let heightCss = 0
let dpr = 1
let palette: string[] = []
let themeObserver: MutationObserver | null = null
let resizeObserver: ResizeObserver | null = null
let visibilityHandler: (() => void) | null = null
let reducedMotionMql: MediaQueryList | null = null
let reducedMotionHandler: ((e: MediaQueryListEvent) => void) | null = null

// Theme palette tokens, resolved to concrete colour strings at runtime. No
// colour is hardcoded here — the field inherits whatever the active theme
// declares for these semantic tokens.
const TOKEN_PALETTE = ['--accent', '--bloom-accent', '--text-dim', '--accent-secondary']

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

function refreshPalette(): void {
  palette = TOKEN_PALETTE.map(resolveTokenColor).filter((c) => c.length > 0)
  if (palette.length === 0) palette = ['currentColor']
}

function clamp(v: number, lo: number, hi: number): number {
  return Math.max(lo, Math.min(hi, v))
}

function computeCount(): number {
  const area = Math.max(1, widthCss * heightCss)
  const base = Math.round(area / 16000) * props.density
  return clamp(Math.round(base), 16, props.maxParticles)
}

function makeParticle(): Particle {
  const color = palette.length ? palette[Math.floor(Math.random() * palette.length)] : 'currentColor'
  const isAccent = palette.length > 0 && color === palette[0]
  return {
    x: Math.random() * widthCss,
    y: Math.random() * heightCss,
    vx: (Math.random() - 0.5) * 6,
    vy: (Math.random() - 0.5) * 6,
    radius: isAccent ? 1.1 + Math.random() * 1.4 : 0.6 + Math.random() * 1.2,
    color,
    baseAlpha: isAccent ? 0.28 : 0.14 + Math.random() * 0.12,
    twinkle: Math.random() * Math.PI * 2,
    twinkleSpeed: 0.4 + Math.random() * 0.8,
  }
}

function rebuildParticles(): void {
  const count = computeCount()
  particles = Array.from({ length: count }, makeParticle)
}

function resize(): void {
  const canvas = canvasEl.value
  if (!canvas) return
  const rect = canvas.getBoundingClientRect()
  widthCss = Math.max(1, rect.width)
  heightCss = Math.max(1, rect.height)
  dpr = Math.min(window.devicePixelRatio || 1, 2)
  canvas.width = Math.round(widthCss * dpr)
  canvas.height = Math.round(heightCss * dpr)
  if (ctx) ctx.setTransform(dpr, 0, 0, dpr, 0, 0)
  rebuildParticles()
}

function drawFrame(now: number): void {
  if (!ctx) return
  const dt = lastTime ? Math.min((now - lastTime) / 1000, 0.05) : 0.016
  lastTime = now

  // Subtle breathing: the synth pad's live output level (0..1, RMS-smoothed)
  // nudges drift speed and translucency so the field rises and falls with the
  // music instead of snapping between two hardcoded states.
  const amp = bgmPlaying.value ? bgmAmplitude.value : 0
  const boost = 1 + amp * 0.7
  ctx.clearRect(0, 0, widthCss, heightCss)

  for (const p of particles) {
    p.x += p.vx * dt * boost
    p.y += p.vy * dt * boost
    if (p.x < -4) p.x = widthCss + 4
    else if (p.x > widthCss + 4) p.x = -4
    if (p.y < -4) p.y = heightCss + 4
    else if (p.y > heightCss + 4) p.y = -4

    p.twinkle += p.twinkleSpeed * dt
    const twinkle = (Math.sin(p.twinkle) + 1) / 2
    const alpha = clamp(p.baseAlpha * (0.6 + 0.4 * twinkle) * (1 + amp * 0.6), 0, 1)

    ctx.globalAlpha = alpha
    ctx.fillStyle = p.color
    ctx.beginPath()
    ctx.arc(p.x, p.y, p.radius, 0, Math.PI * 2)
    ctx.fill()
  }
  ctx.globalAlpha = 1
  rafId = requestAnimationFrame(drawFrame)
}

function renderStatic(): void {
  // Reduced-motion: a single calm frame, no animation loop.
  if (!ctx) return
  ctx.clearRect(0, 0, widthCss, heightCss)
  for (const p of particles) {
    ctx.globalAlpha = p.baseAlpha
    ctx.fillStyle = p.color
    ctx.beginPath()
    ctx.arc(p.x, p.y, p.radius, 0, Math.PI * 2)
    ctx.fill()
  }
  ctx.globalAlpha = 1
}

function start(): void {
  if (running || reducedMotionMql?.matches) return
  running = true
  lastTime = 0
  rafId = requestAnimationFrame(drawFrame)
}

function stop(): void {
  running = false
  if (rafId) cancelAnimationFrame(rafId)
  rafId = 0
}

onMounted(() => {
  const canvas = canvasEl.value
  if (!canvas) return
  ctx = canvas.getContext('2d')
  refreshPalette()
  resize()

  reducedMotionMql = window.matchMedia('(prefers-reduced-motion: reduce)')
  if (reducedMotionMql.matches) {
    renderStatic()
  } else {
    start()
  }

  // Re-resolve the palette when the active theme swaps on <html>.
  themeObserver = new MutationObserver(() => {
    refreshPalette()
    rebuildParticles()
    if (reducedMotionMql?.matches) renderStatic()
  })
  themeObserver.observe(document.documentElement, {
    attributes: true,
    attributeFilter: ['data-theme'],
  })

  resizeObserver = new ResizeObserver(() => {
    resize()
    if (reducedMotionMql?.matches) renderStatic()
  })
  resizeObserver.observe(canvas)

  visibilityHandler = () => {
    if (document.hidden) stop()
    else if (!reducedMotionMql?.matches) start()
  }
  document.addEventListener('visibilitychange', visibilityHandler)

  reducedMotionHandler = (e) => {
    if (e.matches) {
      stop()
      renderStatic()
    } else {
      rebuildParticles()
      start()
    }
  }
  reducedMotionMql.addEventListener('change', reducedMotionHandler)
})

onBeforeUnmount(() => {
  stop()
  themeObserver?.disconnect()
  resizeObserver?.disconnect()
  if (visibilityHandler) document.removeEventListener('visibilitychange', visibilityHandler)
  if (reducedMotionMql && reducedMotionHandler) {
    reducedMotionMql.removeEventListener('change', reducedMotionHandler)
  }
})
</script>

<style scoped>
.particle-field {
  position: absolute;
  inset: 0;
  z-index: 0;
  pointer-events: none;
  display: block;
  width: 100%;
  height: 100%;
}
</style>
