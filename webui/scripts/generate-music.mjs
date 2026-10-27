#!/usr/bin/env node
/**
 * generate-music.mjs — render the built-in, CC0 music catalog for the Control
 * UI. Every track is synthesised from scratch here (no samples, no licensed
 * material): oscillators + envelopes + a feedback-delay "reverb", written to
 * WAV and encoded to MP3 with ffmpeg.
 *
 * Deterministic: a fixed PRNG seed means re-running this script produces
 * byte-identical files. Idempotent: outputs are overwritten, temp WAVs cleaned
 * up. Run from webui/:  node scripts/generate-music.mjs
 */
import { spawnSync } from 'node:child_process'
import { existsSync, mkdirSync, readFileSync, rmSync, statSync, writeFileSync } from 'node:fs'
import { createHash } from 'node:crypto'
import { dirname, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

const SR = 44100
const HERE = dirname(fileURLToPath(import.meta.url))
const OUT_DIR = resolve(HERE, '../public/music')
const TMP_DIR = resolve(HERE, '.music-build')
const FFMPEG = process.env.FFMPEG_PATH || 'C:\\ProgramData\\chocolatey\\bin\\ffmpeg.exe'
const BITRATE = '112k'

// ---------------------------------------------------------------- PRNG -----
function mulberry32(seed) {
  let a = seed >>> 0
  return () => {
    a |= 0; a = (a + 0x6d2b79f5) | 0
    let t = Math.imul(a ^ (a >>> 15), 1 | a)
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296
  }
}

// ------------------------------------------------------------- low-level ---
const mtof = m => 440 * Math.pow(2, (m - 69) / 12)

function makeBuffer(seconds) {
  const n = Math.ceil(seconds * SR)
  return { l: new Float32Array(n), r: new Float32Array(n), n }
}

function add(buf, i, sl, sr) {
  if (i >= 0 && i < buf.n) {
    buf.l[i] += sl
    buf.r[i] += sr
  }
}

// Drum machine ---------------------------------------------------------------
function addKick(buf, t0, gain = 0.9) {
  const i0 = Math.floor(t0 * SR)
  const dur = 0.26
  let phase = 0
  for (let i = 0; i < dur * SR; i++) {
    const t = i / SR
    const f = 50 + 110 * Math.exp(-t * 28)
    phase += (2 * Math.PI * f) / SR
    const s = Math.sin(phase) * Math.exp(-t * 10) * gain
    add(buf, i0 + i, s, s)
  }
}

function addHat(buf, t0, dur = 0.05, gain = 0.25, rnd) {
  const i0 = Math.floor(t0 * SR)
  const len = Math.floor(dur * SR)
  let prev = 0
  for (let i = 0; i < len; i++) {
    const n = rnd() * 2 - 1
    // crude high-pass: difference of successive noise samples
    const hp = n - prev
    prev = n
    const s = hp * Math.exp(-i / (len * 0.35)) * gain
    add(buf, i0 + i, s, s)
  }
}

function addSnare(buf, t0, gain = 0.4, rnd) {
  const i0 = Math.floor(t0 * SR)
  const dur = 0.18
  let phase = 0
  for (let i = 0; i < dur * SR; i++) {
    const t = i / SR
    const noise = (rnd() * 2 - 1) * Math.exp(-t * 16) * gain * 0.7
    phase += (2 * Math.PI * 190) / SR
    const tone = Math.sin(phase) * Math.exp(-t * 14) * gain * 0.5
    add(buf, i0 + i, noise + tone, noise + tone)
  }
}

// Instruments ----------------------------------------------------------------
// Bass: saw-ish (additive odd harmonics) through a one-pole lowpass.
function addBassNote(buf, t0, dur, midi, gain = 0.5) {
  const i0 = Math.floor(t0 * SR)
  const len = Math.floor(dur * SR)
  const f = mtof(midi)
  let lp = 0
  for (let i = 0; i < len; i++) {
    const t = i / SR
    let s = 0
    for (let h = 1; h <= 6; h += 2) s += Math.sin(2 * Math.PI * f * h * t) / h
    lp += (0.25 - lp) * 0.15 // one-pole smoothing → softens the saw
    const env = Math.min(1, t / 0.02) * Math.exp(-t / (dur * 0.55))
    const out = s * env * gain * lp * 0.35
    add(buf, i0 + i, out, out)
  }
}

// Pluck: triangle-ish additive partials with exponential decay.
function addPluck(buf, t0, dur, midi, gain = 0.35, detuneCents = 0) {
  const i0 = Math.floor(t0 * SR)
  const len = Math.floor(dur * SR)
  const f = mtof(midi) * Math.pow(2, detuneCents / 1200)
  for (let i = 0; i < len; i++) {
    const t = i / SR
    let s = 0
    for (let h = 1; h <= 10; h += 2) s += Math.sin(2 * Math.PI * f * h * t) / (h * h)
    const out = s * Math.exp(-t / (dur * 0.3)) * gain
    add(buf, i0 + i, out, out * 0.94)
  }
}

// Pad: detuned sines with slow attack and release.
function addPadChord(buf, t0, dur, midis, gain = 0.12) {
  const i0 = Math.floor(t0 * SR)
  const len = Math.floor(dur * SR)
  for (const midi of midis) {
    for (const det of [-4, 3]) {
      const f = mtof(midi) * Math.pow(2, det / 1200)
      for (let i = 0; i < len; i++) {
        const t = i / SR
        const attack = Math.min(1, t / 2.5)
        const release = Math.min(1, (dur - t) / 3.5)
        const env = attack * release
        const s = Math.sin(2 * Math.PI * f * t) * env * gain
        add(buf, i0 + i, s, s * 0.96)
      }
    }
  }
}

// Vinyl-ish hiss bed for lo-fi texture.
function addNoiseBed(buf, rnd, gain = 0.012) {
  for (let i = 0; i < buf.n; i++) {
    const s = (rnd() * 2 - 1) * gain
    buf.l[i] += s
    buf.r[i] += s
  }
}

// Master bus: feedback delay + soft clip + peak normalize.
function master(buf, delaySeconds = 0.26) {
  const dlen = Math.floor(delaySeconds * SR)
  const delayL = new Float32Array(dlen)
  const delayR = new Float32Array(dlen)
  let head = 0
  let peak = 0
  for (let i = 0; i < buf.n; i++) {
    const dl = delayL[head]
    const dr = delayR[head]
    delayL[head] = buf.l[i] + dl * 0.34
    delayR[head] = buf.r[i] + dr * 0.34
    head = (head + 1) % dlen
    let l = buf.l[i] + dl * 0.2
    let r = buf.r[i] + dr * 0.2
    l = Math.tanh(l * 1.1)
    r = Math.tanh(r * 1.1)
    buf.l[i] = l
    buf.r[i] = r
    const p = Math.max(Math.abs(l), Math.abs(r))
    if (p > peak) peak = p
  }
  const scale = peak > 0 ? 0.89 / peak : 1
  for (let i = 0; i < buf.n; i++) {
    buf.l[i] *= scale
    buf.r[i] *= scale
  }
}

// ------------------------------------------------------------------- WAV ---
function writeWav(path, buf) {
  const dataSize = buf.n * 2 * 2
  const out = Buffer.alloc(44 + dataSize)
  out.write('RIFF', 0)
  out.writeUInt32LE(36 + dataSize, 4)
  out.write('WAVE', 8)
  out.write('fmt ', 12)
  out.writeUInt32LE(16, 16)
  out.writeUInt16LE(1, 20) // PCM
  out.writeUInt16LE(2, 22) // stereo
  out.writeUInt32LE(SR, 24)
  out.writeUInt32LE(SR * 4, 28)
  out.writeUInt16LE(4, 32)
  out.writeUInt16LE(16, 34)
  out.write('data', 36)
  out.writeUInt32LE(dataSize, 40)
  let o = 44
  for (let i = 0; i < buf.n; i++) {
    out.writeInt16LE(Math.max(-1, Math.min(1, buf.l[i])) * 32767, o)
    out.writeInt16LE(Math.max(-1, Math.min(1, buf.r[i])) * 32767, o + 2)
    o += 4
  }
  writeFileSync(path, out)
}

function encodeMp3(wavPath, mp3Path) {
  const res = spawnSync(
    FFMPEG,
    ['-y', '-i', wavPath, '-codec:a', 'libmp3lame', '-b:a', BITRATE, '-ar', '44100', '-ac', '2', mp3Path],
    { stdio: 'pipe' },
  )
  if (res.status !== 0) {
    throw new Error(`ffmpeg failed for ${mp3Path}: ${res.stderr.toString().slice(-500)}`)
  }
}

// --------------------------------------------------------------- TRACKS ----
// 1) dusk-pad: slow ambient pad, no drums.
function trackDuskPad() {
  const dur = 75
  const buf = makeBuffer(dur)
  const rnd = mulberry32(101)
  // Cmaj9 → Am9 → Fmaj9 → G6, 18.75s each.
  const chords = [
    [48, 52, 55, 59],
    [45, 48, 52, 55],
    [41, 45, 48, 52],
    [43, 47, 50, 55],
  ]
  chords.forEach((chord, c) => {
    addPadChord(buf, c * 18.75, 18.75, chord, 0.11)
  })
  // Sparse bell plucks, one per bar, wide spacing.
  const bells = [60, 64, 67, 71, 69, 72, 76, 79]
  bells.forEach((m, i) => addPluck(buf, 6 + i * 8.4, 3.5, m, 0.06))
  addNoiseBed(buf, rnd, 0.006)
  master(buf, 0.34)
  return buf
}

// 2) neon-arp: synthwave arpeggio at 100 BPM.
function trackNeonArp() {
  const dur = 80
  const buf = makeBuffer(dur)
  const rnd = mulberry32(202)
  const bpm = 100
  const beat = 60 / bpm
  // Am – F – C – G, one chord per 4 bars (16 beats).
  const chords = [
    [45, 48, 52, 57],
    [41, 45, 48, 53],
    [36, 48, 52, 55],
    [43, 47, 50, 55],
  ]
  const totalBeats = Math.floor(dur / beat)
  for (let b = 0; b < totalBeats; b++) {
    const t = b * beat
    addKick(buf, t, 0.75) // four-on-the-floor
    if (b % 2 === 1) addHat(buf, t + beat / 2, 0.04, 0.18, rnd)
    const chord = chords[Math.floor(b / 16) % chords.length]
    // 16th-note arpeggio up two octaves
    for (let s = 0; s < 4; s++) {
      const note = chord[s % chord.length] + 12 + 12 * Math.floor(s / chord.length)
      addPluck(buf, t + (s * beat) / 4, 0.22, note, 0.14)
    }
    // Bass on beats 1 and 3
    if (b % 2 === 0) addBassNote(buf, t, beat * 0.9, chord[0] - 12, 0.5)
  }
  master(buf, 0.24)
  return buf
}

// 3) lofi-drift: lazy jazz chords, soft beat, vinyl noise.
function trackLofiDrift() {
  const dur = 80
  const buf = makeBuffer(dur)
  const rnd = mulberry32(303)
  const bpm = 76
  const beat = 60 / bpm
  // Dm7 – G7 – Cmaj7 – A7
  const chords = [
    [50, 53, 57, 60],
    [43, 47, 50, 53],
    [48, 52, 55, 59],
    [45, 49, 52, 55],
  ]
  const totalBeats = Math.floor(dur / beat)
  for (let b = 0; b < totalBeats; b++) {
    const t = b * beat
    if (b % 4 === 0) addKick(buf, t, 0.5)
    if (b % 4 === 2) addSnare(buf, t, 0.28, rnd)
    addHat(buf, t + beat * 0.5, 0.04, 0.1, rnd)
    const chord = chords[Math.floor(b / 8) % chords.length]
    // Soft chord stabs on beats 1 and 3 of each 8-bar half-loop
    if (b % 4 === 0) {
      for (const m of chord) addPluck(buf, t, 1.6, m, 0.12)
    }
  }
  addNoiseBed(buf, rnd, 0.01)
  master(buf, 0.3)
  return buf
}

// 4) focus-pulse: steady four-on-floor with a sparse pluck motif.
function trackFocusPulse() {
  const dur = 85
  const buf = makeBuffer(dur)
  const rnd = mulberry32(404)
  const bpm = 112
  const beat = 60 / bpm
  const totalBeats = Math.floor(dur / beat)
  // Minimal two-chord vamping: Em – C
  const roots = [40, 36]
  const motif = [64, 67, 71, 67, 64, 62, 60, 62]
  for (let b = 0; b < totalBeats; b++) {
    const t = b * beat
    addKick(buf, t, 0.8)
    if (b % 2 === 1) addHat(buf, t + beat / 2, 0.03, 0.14, rnd)
    const root = roots[Math.floor(b / 16) % roots.length]
    addBassNote(buf, t, beat * 0.8, root, 0.55)
    if (b % 4 === 2) addPluck(buf, t, 0.4, motif[b % motif.length], 0.16)
  }
  master(buf, 0.22)
  return buf
}

// ------------------------------------------------------------------ main ----
const TRACKS = [
  { file: 'dusk-pad.mp3', title: 'Dusk Pad', genre: 'ambient', render: trackDuskPad },
  { file: 'neon-arp.mp3', title: 'Neon Arpeggio', genre: 'synthwave', render: trackNeonArp },
  { file: 'lofi-drift.mp3', title: 'Lo-fi Drift', genre: 'lo-fi chill', render: trackLofiDrift },
  { file: 'focus-pulse.mp3', title: 'Focus Pulse', genre: 'focus electronic', render: trackFocusPulse },
]

mkdirSync(OUT_DIR, { recursive: true })
mkdirSync(TMP_DIR, { recursive: true })

const manifest = []
for (const track of TRACKS) {
  console.log(`rendering ${track.file} …`)
  const buf = track.render()
  const wav = resolve(TMP_DIR, track.file.replace(/\.mp3$/, '.wav'))
  const mp3 = resolve(OUT_DIR, track.file)
  writeWav(wav, buf)
  encodeMp3(wav, mp3)
  const bytes = statSync(mp3).size
  const hash = createHash('sha256').update(readFileSync(mp3)).digest('hex')
  manifest.push({
    file: track.file,
    title: track.title,
    genre: track.genre,
    seconds: +(buf.n / SR).toFixed(1),
    bytes,
    sha256: hash,
  })
}

rmSync(TMP_DIR, { recursive: true, force: true })
console.log('\n' + JSON.stringify(manifest, null, 2))
