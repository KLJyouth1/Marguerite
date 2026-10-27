/**
 * 玛格丽特 · 星夜 · 环境音（合成，无音频文件）。
 *
 * 为什么用 Web Audio 合成而不是随包放 mp3：
 * - 仓库对版权/水印素材有硬要求（见 DEF-14），合成音源零版权风险、零体积；
 * - 星夜的“风声 + 远处铃音”本身是低频噪声与正弦泛音，合成比采样更干净；
 * - 可精确受 `prefers-reduced-motion` / 用户音量控制，且不干扰语音输入。
 *
 * 复用 `audioEngine` 的单一 AudioContext（浏览器限制每个页面一个可用上下文），
 * 但环境音的节点直连 destination，不经过分析器，避免污染音乐可视化。
 *
 * 默认关闭，偏好写入 `localStorage['marguerite.ambience']`；开启需用户手势
 * （浏览器自动播放策略），因此开启后若上下文仍 suspended，会挂一次性手势监听。
 */

import { reactive } from 'vue'
import { audioEngine } from './audioEngine'

export const AMBIENCE_STORAGE_KEY = 'marguerite.ambience'
export const AMBIENCE_DEFAULT_VOLUME = 0.35

export interface AmbiencePrefs {
  enabled: boolean
  volume: number
}

/** 五声音阶（大调五声）半音偏移：听感柔和，适合随机铃声。 */
export const PENTATONIC_STEPS = [0, 2, 4, 7, 9, 12, 14, 16] as const

export function clampVolume(value: number): number {
  if (!Number.isFinite(value)) return AMBIENCE_DEFAULT_VOLUME
  return Math.min(1, Math.max(0, Math.round(value * 100) / 100))
}

export function readAmbiencePrefs(): AmbiencePrefs {
  try {
    const raw = localStorage.getItem(AMBIENCE_STORAGE_KEY)
    if (!raw) return { enabled: false, volume: AMBIENCE_DEFAULT_VOLUME }
    const parsed = JSON.parse(raw) as Partial<AmbiencePrefs> | null
    return {
      enabled: parsed?.enabled === true,
      volume: clampVolume(typeof parsed?.volume === 'number' ? parsed.volume : AMBIENCE_DEFAULT_VOLUME),
    }
  } catch {
    return { enabled: false, volume: AMBIENCE_DEFAULT_VOLUME }
  }
}

export function writeAmbiencePrefs(prefs: AmbiencePrefs): void {
  try {
    localStorage.setItem(
      AMBIENCE_STORAGE_KEY,
      JSON.stringify({ enabled: prefs.enabled, volume: clampVolume(prefs.volume) }),
    )
  } catch {
    /* 配额/隐私模式：保持当前会话生效 */
  }
}

/** 五声音阶频率：root 为基音（Hz）。 */
export function chimeFrequency(rootHz: number, step: number): number {
  return rootHz * 2 ** (step / 12)
}

/** 随机铃音间隔（毫秒）：11–25s，避免打扰。 */
export function nextChimeDelay(rand: () => number): number {
  return 11000 + Math.floor(rand() * 14000)
}

/* ────────────────────────── 运行时 ────────────────────────── */

export const ambienceState = reactive({
  enabled: false,
  volume: AMBIENCE_DEFAULT_VOLUME,
  running: false,
  supported: true,
})

interface Nodes {
  master: GainNode
  wind: AudioBufferSourceNode
  windGain: GainNode
  lfo: OscillatorNode
  lfoGain: GainNode
  padA: OscillatorNode
  padB: OscillatorNode
  padGain: GainNode
}

let ctxRef: AudioContext | null = null
let nodes: Nodes | null = null
let chimeTimer: number | null = null
let gestureArmed = false

function random(): number {
  return Math.random()
}

function buildNoiseBuffer(ctx: AudioContext): AudioBuffer {
  const seconds = 2
  const buffer = ctx.createBuffer(1, ctx.sampleRate * seconds, ctx.sampleRate)
  const data = buffer.getChannelData(0)
  let last = 0
  for (let i = 0; i < data.length; i += 1) {
    const white = Math.random() * 2 - 1
    // 一阶低通后的白噪声更像“山风”，不会像电视雪花那样刺耳
    last = last * 0.97 + white * 0.03
    data[i] = last * 3.2
  }
  return buffer
}

function playBell(ctx: AudioContext, master: GainNode, freq: number, gain: number): void {
  const now = ctx.currentTime
  const osc = ctx.createOscillator()
  const env = ctx.createGain()
  osc.type = 'triangle'
  osc.frequency.setValueAtTime(freq, now)
  env.gain.setValueAtTime(0.0001, now)
  env.gain.exponentialRampToValueAtTime(Math.max(0.0002, gain), now + 0.02)
  env.gain.exponentialRampToValueAtTime(0.0001, now + 1.6)
  osc.connect(env)
  env.connect(master)
  osc.start(now)
  osc.stop(now + 1.7)

  // 泛音：让铃声有“远处风铃”的空气感
  const overtone = ctx.createOscillator()
  const og = ctx.createGain()
  overtone.type = 'sine'
  overtone.frequency.setValueAtTime(freq * 2.01, now)
  og.gain.setValueAtTime(0.0001, now)
  og.gain.exponentialRampToValueAtTime(Math.max(0.0002, gain * 0.35), now + 0.03)
  og.gain.exponentialRampToValueAtTime(0.0001, now + 1.1)
  overtone.connect(og)
  og.connect(master)
  overtone.start(now)
  overtone.stop(now + 1.2)
}

function scheduleChime(): void {
  if (chimeTimer !== null) window.clearTimeout(chimeTimer)
  chimeTimer = window.setTimeout(() => {
    if (!ctxRef || !nodes) return
    const step = PENTATONIC_STEPS[Math.floor(random() * PENTATONIC_STEPS.length)]
    const octave = random() > 0.6 ? 2 : 1
    playBell(ctxRef, nodes.master, chimeFrequency(659.25, step) * octave, 0.055)
    scheduleChime()
  }, nextChimeDelay(random))
}

function startScene(ctx: AudioContext): void {
  if (nodes) return
  const master = ctx.createGain()
  master.gain.setValueAtTime(0.0001, ctx.currentTime)
  master.gain.exponentialRampToValueAtTime(Math.max(0.0002, ambienceState.volume * 0.5), ctx.currentTime + 1.2)
  master.connect(ctx.destination)

  const wind = ctx.createBufferSource()
  wind.buffer = buildNoiseBuffer(ctx)
  wind.loop = true
  const band = ctx.createBiquadFilter()
  band.type = 'bandpass'
  band.frequency.value = 460
  band.Q.value = 0.7
  const windGain = ctx.createGain()
  windGain.gain.value = 0.05
  wind.connect(band)
  band.connect(windGain)
  windGain.connect(master)
  wind.start()

  // 极慢的阵风起伏（0.06Hz），让风声不机械
  const lfo = ctx.createOscillator()
  lfo.frequency.value = 0.06
  const lfoGain = ctx.createGain()
  lfoGain.gain.value = 0.018
  lfo.connect(lfoGain)
  lfoGain.connect(windGain.gain)
  lfo.start()

  const padA = ctx.createOscillator()
  padA.type = 'sine'
  padA.frequency.value = 98
  const padB = ctx.createOscillator()
  padB.type = 'sine'
  padB.frequency.value = 147
  padB.detune.value = 5
  const padGain = ctx.createGain()
  padGain.gain.value = 0.016
  const lowpass = ctx.createBiquadFilter()
  lowpass.type = 'lowpass'
  lowpass.frequency.value = 720
  padA.connect(padGain)
  padB.connect(padGain)
  padGain.connect(lowpass)
  lowpass.connect(master)
  padA.start()
  padB.start()

  nodes = { master, wind, windGain, lfo, lfoGain, padA, padB, padGain }
  scheduleChime()
  ambienceState.running = true
}

function stopScene(): void {
  if (chimeTimer !== null) {
    window.clearTimeout(chimeTimer)
    chimeTimer = null
  }
  const ctx = ctxRef
  const current = nodes
  nodes = null
  ambienceState.running = false
  if (!ctx || !current) return
  try {
    const now = ctx.currentTime
    current.master.gain.cancelScheduledValues(now)
    current.master.gain.setValueAtTime(Math.max(0.0001, current.master.gain.value), now)
    current.master.gain.exponentialRampToValueAtTime(0.0001, now + 0.4)
  } catch {
    /* 上下文已关闭：直接断开即可 */
  }
  window.setTimeout(() => {
    try { current.wind.stop() } catch { /* 已停止 */ }
    try { current.lfo.stop() } catch { /* 已停止 */ }
    try { current.padA.stop() } catch { /* 已停止 */ }
    try { current.padB.stop() } catch { /* 已停止 */ }
    try { current.master.disconnect() } catch { /* 已断开 */ }
  }, 600)
}

function armGestureOnce(): void {
  if (gestureArmed || typeof window === 'undefined') return
  gestureArmed = true
  const handler = () => {
    window.removeEventListener('pointerdown', handler, true)
    window.removeEventListener('keydown', handler, true)
    gestureArmed = false
    if (ambienceState.enabled) void ensureRunning()
  }
  window.addEventListener('pointerdown', handler, true)
  window.addEventListener('keydown', handler, true)
}

async function ensureRunning(): Promise<boolean> {
  await audioEngine.unlock()
  const ctx = audioEngine.context()
  if (!ctx) {
    ambienceState.supported = false
    ambienceState.running = false
    return false
  }
  ctxRef = ctx
  startScene(ctx)
  return true
}

export const ambience = {
  /** 读持久化偏好并按需启动（首屏调用；没有用户手势时挂一次性监听）。 */
  init(): void {
    const prefs = readAmbiencePrefs()
    ambienceState.enabled = prefs.enabled
    ambienceState.volume = prefs.volume
    if (!prefs.enabled) return
    // 上下文未解锁：等用户第一次交互再启动（浏览器自动播放策略）
    if (!audioEngine.supported()) {
      void ensureRunning()
    } else {
      armGestureOnce()
    }
  },

  async setEnabled(on: boolean): Promise<boolean> {
    ambienceState.enabled = on
    writeAmbiencePrefs({ enabled: on, volume: ambienceState.volume })
    if (!on) {
      stopScene()
      return true
    }
    const ok = await ensureRunning()
    if (!ok) {
      // 环境不支持：如实回滚开关，不做“看起来开着其实没声”
      ambienceState.enabled = false
      writeAmbiencePrefs({ enabled: false, volume: ambienceState.volume })
    }
    return ok
  },

  toggle(): Promise<boolean> {
    return this.setEnabled(!ambienceState.enabled)
  },

  setVolume(value: number): void {
    ambienceState.volume = clampVolume(value)
    writeAmbiencePrefs({ enabled: ambienceState.enabled, volume: ambienceState.volume })
    if (nodes && ctxRef) {
      try {
        nodes.master.gain.setTargetAtTime(
          Math.max(0.0002, ambienceState.volume * 0.5),
          ctxRef.currentTime,
          0.08,
        )
      } catch {
        /* 上下文已关闭 */
      }
    }
  },

  /** 交互音（抚摸/投喂/成功）：仅在环境音开启时响，音量更低。 */
  chime(kind: 'pet' | 'feed' | 'ok' = 'ok'): void {
    if (!ambienceState.enabled || !nodes || !ctxRef) return
    const base = kind === 'feed' ? 523.25 : kind === 'pet' ? 783.99 : 659.25
    playBell(ctxRef, nodes.master, base, 0.05)
  },

  dispose(): void {
    stopScene()
    ctxRef = null
  },
}
