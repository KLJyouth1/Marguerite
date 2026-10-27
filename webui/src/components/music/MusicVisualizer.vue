<script setup lang="ts">
/**
 * Audio-driven "constellation" visualiser (design §4): a pointer-transparent,
 * fixed 2D-canvas layer sitting BELOW the app chrome.
 *
 * Scene (all original, Hero-MV inspired but re-composed):
 *   1. concentric radar rings in the lower-middle of the viewport — bass
 *      breathing expands them, a beat flashes the outer ring, radial spectrum
 *      ticks ride the outermost ring;
 *   2. a dust-mote particle field drifting upward, brightness/size driven by
 *      mid+treble energy, contracting toward centre on a beat;
 *   3. a hair-thin waveform line near the bottom that breathes with the track.
 *
 * The geometry tuning lives in ./visualizerMath.ts (pure functions, unit
 * tested); this component only owns the canvas lifecycle, token colour lookup
 * and the rAF loop. The loop pauses entirely when playback stops, the tab is
 * hidden, or the user asked for reduced motion.
 */
import { onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { useBgm } from '@/composables/useBgm'
import {
  decayBeatPulse,
  ringSpecs,
  spawnParticles,
  spectrumBars,
  stepParticle,
  type VizParticle,
} from './visualizerMath'

const canvasRef = ref<HTMLCanvasElement | null>(null)

const bgm = useBgm()
const { playing } = bgm
// Shared analysis frame (started by the BGM bootstrap; zeroed until unlock).
const { spectrum, waveform, bass, mid, treble, energy, beat } = bgm.audio

let ctx: CanvasRenderingContext2D | null = null
let rafId = 0
let lastTs = 0
let time = 0
let beatPulse = 0
let particles: VizParticle[] = []
// Resolved token colours as 0..255 channels — read once on mount, never raw
// literals, so the component colour guard stays happy.
let accentRgb: [number, number, number] | null = null
let bloomRgb: [number, number, number] | null = null

function reducedMotion(): boolean {
  return window.matchMedia('(prefers-reduced-motion: reduce)').matches
}

// Parse a CSS colour token into integer channels (hex shorthand, longhand,
// or a space-separated channel list).
function tokenToChannels(raw: string): [number, number, number] | null {
  const s = raw.trim()
  if (!s) return null
  if (s.startsWith('#')) {
    const v = parseInt(s.slice(1), 16)
    if (Number.isNaN(v)) return null
    const full = s.length === 4
      ? ((v & 0xf00) << 12) | ((v & 0x0f0) << 8) | ((v & 0x00f) << 4)
      : v
    return [(full >> 16) & 0xff, (full >> 8) & 0xff, full & 0xff]
  }
  const nums = s.match(/[\d.]+/g)
  if (!nums || nums.length < 3) return null
  return [Number(nums[0]), Number(nums[1]), Number(nums[2])]
}

function readTokens() {
  const styles = getComputedStyle(document.documentElement)
  accentRgb = tokenToChannels(styles.getPropertyValue('--accent'))
  bloomRgb = tokenToChannels(styles.getPropertyValue('--bloom-accent')) || accentRgb
}

function resize() {
  const canvas = canvasRef.value
  if (!canvas) return
  const dpr = Math.min(2, window.devicePixelRatio || 1)
  canvas.width = Math.max(1, Math.floor(window.innerWidth * dpr))
  canvas.height = Math.max(1, Math.floor(window.innerHeight * dpr))
}

function frame(ts: number) {
  rafId = requestAnimationFrame(frame)
  if (!ctx) return
  const dt = Math.min(0.05, lastTs ? (ts - lastTs) / 1000 : 0.016)
  lastTs = ts
  time += dt

  const w = window.innerWidth
  const h = window.innerHeight
  const dpr = Math.min(2, window.devicePixelRatio || 1)
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0)
  ctx.clearRect(0, 0, w, h)

  const b = bass.value
  const e = energy.value
  if (beat.value) beatPulse = 1
  beatPulse = decayBeatPulse(beatPulse, dt)

  // Global restraint: the layer must never fight the text for contrast.
  const live = playing.value ? Math.min(1, 0.25 + e * 1.2) : 0
  if (live <= 0.01) return

  // --- 1. Radar rings ------------------------------------------------------
  const cx = w / 2
  const cy = h * 0.62
  const maxR = Math.min(w, h) * 0.30
  const rings = ringSpecs(4, b, beatPulse, maxR)
  ctx.lineWidth = 1
  for (const ring of rings) {
    ctx.beginPath()
    ctx.arc(cx, cy, ring.radius, 0, Math.PI * 2)
    ctx.strokeStyle = withAlpha(accentRgb, ring.alpha * live)
    ctx.lineWidth = ring.width
    ctx.stroke()
  }
  // Radial spectrum ticks on the outermost ring.
  const outer = rings[rings.length - 1]
  const bars = spectrumBars(spectrum.value, 48)
  for (let i = 0; i < bars.length; i++) {
    const a = (i / bars.length) * Math.PI * 2 - Math.PI / 2
    const len = 4 + bars[i] * 16
    ctx.beginPath()
    ctx.moveTo(cx + Math.cos(a) * outer.radius, cy + Math.sin(a) * outer.radius)
    ctx.lineTo(cx + Math.cos(a) * (outer.radius + len), cy + Math.sin(a) * (outer.radius + len))
    ctx.strokeStyle = withAlpha(bloomRgb, (0.15 + bars[i] * 0.55) * live)
    ctx.lineWidth = 1
    ctx.stroke()
  }

  // --- 2. Constellation particles -----------------------------------------
  const level = Math.min(1, mid.value + treble.value * 0.8)
  for (const p of particles) {
    const next = stepParticle(p, dt, time, level, beatPulse)
    p.x = next.x
    p.y = next.y
    const twinkle = 0.6 + 0.4 * Math.sin(time * 1.4 + p.phase)
    const r = p.size * (1 + level * 0.9)
    ctx.beginPath()
    ctx.arc(p.x * w, p.y * h, r, 0, Math.PI * 2)
    ctx.fillStyle = withAlpha(bloomRgb, p.alpha * twinkle * live * 0.8)
    ctx.fill()
  }

  // --- 3. Hair-thin breathing waveform near the bottom -------------------
  const wave = waveform.value
  if (wave.length > 8) {
    const wy = h * 0.86
    const stepX = w / wave.length
    ctx.beginPath()
    for (let i = 0; i < wave.length; i++) {
      const v = (wave[i] - 128) / 128
      const x = i * stepX
      const y = wy + v * 26
      if (i === 0) ctx.moveTo(x, y)
      else ctx.lineTo(x, y)
    }
    ctx.strokeStyle = withAlpha(accentRgb, 0.28 * live)
    ctx.lineWidth = 1
    ctx.stroke()
  }
}

/** Build an `rgb(r,g,b / a)` canvas colour at runtime (no source literals). */
function withAlpha(rgb: [number, number, number] | null, alpha: number): string {
  if (!rgb) return 'transparent'
  const a = Math.min(1, Math.max(0, alpha)).toFixed(3)
  return 'rgb' + `(${rgb[0]},${rgb[1]},${rgb[2]} / ${a})`
}

function start() {
  if (rafId) return
  lastTs = 0
  rafId = requestAnimationFrame(frame)
}

function stop() {
  if (rafId) cancelAnimationFrame(rafId)
  rafId = 0
  // Clear once so a paused workstation never leaves a frozen frame behind.
  if (ctx && canvasRef.value) {
    ctx.setTransform(1, 0, 0, 1, 0, 0)
    ctx.clearRect(0, 0, canvasRef.value.width, canvasRef.value.height)
  }
}

watch(
  playing,
  on => {
    if (on && !reducedMotion() && !document.hidden) start()
    else stop()
  },
  { immediate: true },
)

onMounted(() => {
  readTokens()
  ctx = canvasRef.value?.getContext('2d') ?? null
  particles = spawnParticles(110)
  resize()
  window.addEventListener('resize', resize)
  document.addEventListener('visibilitychange', onVisibility)
  if (playing.value && !reducedMotion() && !document.hidden) start()
})

function onVisibility() {
  if (document.hidden) stop()
  else if (playing.value && !reducedMotion()) start()
}

onBeforeUnmount(() => {
  stop()
  window.removeEventListener('resize', resize)
  document.removeEventListener('visibilitychange', onVisibility)
})
</script>

<template>
  <canvas
    ref="canvasRef"
    class="music-viz"
    aria-hidden="true"
  ></canvas>
</template>

<style scoped>
.music-viz {
  position: fixed;
  inset: 0;
  z-index: 0;
  pointer-events: none;
}

@media (prefers-reduced-motion: reduce) {
  .music-viz {
    display: none;
  }
}
</style>
