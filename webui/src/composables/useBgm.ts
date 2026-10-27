import { computed, ref } from 'vue'
import { t } from '@/i18n'
import { useAmbientSynth, BGM_SYNTH_TRACK_ID, BGM_SYNTH_I18N_KEY } from './useAmbientSynth'
import { audioEngine } from './audioEngine'
import { startAudioReactivity, useAudioReactivity } from './useAudioReactivity'
import { clamp, rotateTrackId, shuffledIndices } from './audioMath'

/**
 * One entry from `public/music/playlist.json`. `src` is either a filename
 * relative to the music directory (bundled into the build) or an absolute
 * HTTPS URL streamed at runtime.
 */
export interface BgmTrack {
  id: string
  title: string
  src: string
  artist?: string
  duration?: number
  genre?: string
}

/**
 * Synthetic id for an ad-hoc "Choose local file…" pick. Session-only: the
 * object URL cannot be restored after a reload, so on init this id falls back
 * to the playlist default with playback off.
 */
export const BGM_LOCAL_TRACK_ID = '__local__'

// Built-in, code-generated ambient pad: always present as the first track so a
// fresh install ships with playable music without bundling any audio file.
export { BGM_SYNTH_TRACK_ID, BGM_SYNTH_I18N_KEY } from './useAmbientSynth'

export type RepeatMode = 'off' | 'all' | 'one'
const REPEAT_MODES: readonly RepeatMode[] = ['off', 'all', 'one'] as const

const STORAGE_KEY = 'Marguerite-bgm'
const DEFAULT_VOLUME = 0.6
const ABSOLUTE_URL_SCHEME = /^[a-z][a-z\d+.-]*:/i
const HTTPS_STREAM = /^https:\/\//i
const URL_ESCAPE = /%[0-9a-f]{2}/i

// Module-level singleton state (mirrors useAgentOptions/useConfirm): one
// <audio> element and one state tree app-wide, however many components mount.
const tracks = ref<BgmTrack[]>([])
const playing = ref(false)
const currentTrackId = ref('')
const volume = ref(DEFAULT_VOLUME)
const playlistError = ref(false)
// Display name of the session-only local file; '' until one has been picked.
const localTrackTitle = ref('')
// Transport state: position within the current element (synth pad has none).
const currentTime = ref(0)
const duration = ref(0)
// Playback mode.
const repeat = ref<RepeatMode>('off')
const shuffle = ref(false)
// Shuffle ordering as an id list; rebuilt when the playlist loads or shuffle
// is switched on. Empty means "linear order".
let shuffleOrder: string[] = []

let audio: HTMLAudioElement | null = null
let localObjectUrl = ''
// Dedupes concurrent initBgm() calls onto a single load, like useAgentOptions.
let initPromise: Promise<void> | null = null
// Async play() settlements may arrive after a disable, pause, or newer play
// request. Only the latest generation may publish state; the intent tells a
// stale successful request whether it must re-pause the shared element.
let playbackGeneration = 0
let playbackIntent = false

// Synth backend (Web Audio pad). Single shared instance, mirroring the one
// <audio> element below.
const synth = useAmbientSynth()
// Shared visualisation data (spectrum / bands / beat). Zeroes until the
// engine unlocks, so mounting it is always safe.
const viz = useAudioReactivity()

interface PersistedBgm {
  enabled?: boolean
  playing?: boolean
  trackId?: string
  volume?: number
  repeat?: string
  shuffle?: boolean
}

function readPersisted(): PersistedBgm {
  try {
    const raw = localStorage.getItem(STORAGE_KEY)
    return raw ? (JSON.parse(raw) as PersistedBgm) : {}
  } catch {
    return {}
  }
}

// Default-ON feature gate: a fresh profile ships the topbar control visible
// (never auto-sounds — browser autoplay policy demands a gesture anyway). An
// *explicit* persisted `false` is honoured verbatim so an existing user who
// switched it off stays off; older saves that predate the field (or never
// touched the toggle) simply follow the new default.
const enabled = ref(readPersisted().enabled !== false)

function persist() {
  try {
    localStorage.setItem(
      STORAGE_KEY,
      JSON.stringify({
        enabled: enabled.value,
        playing: playing.value,
        trackId: currentTrackId.value,
        volume: volume.value,
        repeat: repeat.value,
        shuffle: shuffle.value,
      }),
    )
  } catch {}
}

function isSafeRelativeMusicSource(file: string): boolean {
  if (!file || ABSOLUTE_URL_SCHEME.test(file) || file.startsWith('//')) return false

  // Check the path independently from its query/fragment. Repeated decoding
  // catches both ordinary and nested percent-encoded traversal; every
  // successful pass shortens the string, so the loop is naturally bounded.
  let decodedPath = file.split(/[?#]/, 1)[0]
  try {
    while (URL_ESCAPE.test(decodedPath)) {
      const next = decodeURIComponent(decodedPath)
      if (next === decodedPath) break
      decodedPath = next
    }
  } catch {
    return false
  }

  const slashPath = decodedPath.replace(/\\/g, '/')
  if (!slashPath || slashPath.startsWith('/')) return false
  return slashPath.split('/').every(segment => segment !== '.' && segment !== '..')
}

/**
 * Resolve a playlist `src` to a fetchable URL. Absolute HTTPS URLs pass
 * through; bundled filenames mirror App.vue's brandMarkUrl: Vite serves
 * `public/` at the root in dev, while the packaged UI serves those assets from
 * `${base}/static/` under the gateway's base path (the gateway maps `/static/`
 * straight to `dist/`, so there is no extra `dist/` segment).
 */
function musicAssetUrl(file: string): string {
  if (HTTPS_STREAM.test(file)) return file
  if (!isSafeRelativeMusicSource(file)) return ''
  if (import.meta.env.DEV) return `/music/${file}`
  const base = document.getElementById('Marguerite-data')?.dataset.basePath || '/control'
  return `${base.replace(/\/$/, '')}/static/music/${file}`
}

function syncMediaState() {
  if (!audio) return
  if (Number.isFinite(audio.duration)) duration.value = audio.duration
  if (Number.isFinite(audio.currentTime)) currentTime.value = audio.currentTime
}

/**
 * Unlock the shared Web Audio graph AND ensure the analyser reactivity loop is
 * running. `startAudioReactivity()` is a no-op while the context is absent
 * (engine unsupported), so calling it once at initBgm is NOT enough: the
 * context only exists after a user-gesture unlock. Every play path goes
 * through this helper so the loop actually starts on the first real gesture.
 * The second call is idempotent (running stays true, no second rAF chain).
 *
 * Implemented as an inline tail (`await unlock(); startAudioReactivity()`) at
 * every call site rather than a wrapper async fn on purpose: a wrapper would
 * add an extra microtask hop between unlock() and el.play(), which the
 * pending-playback tests calibrate against.
 */

function handleEnded() {
  if (!enabled.value) return
  playbackGeneration += 1
  playbackIntent = false
  // Repeat-one: restart the same element instead of advancing.
  if (repeat.value === 'one' && audio) {
    audio.currentTime = 0
    void playAudio(audio, 'repeat-one replay failed')
    return
  }
  const order = shuffle.value ? shuffleOrder : []
  const nextId = rotateTrackId(
    tracks.value.map(t => t.id),
    currentTrackId.value,
    1,
    order,
    repeat.value === 'all',
  )
  if (!nextId) {
    // Repeat-off at the end of the list: stop here, paused on the last track.
    playing.value = false
    persist()
    return
  }
  void playTrack(nextId)
}

function getAudio(): HTMLAudioElement {
  if (!audio) {
    audio = new Audio()
    // Loop is driven by us (repeat modes), never by the element itself.
    audio.loop = false
    audio.preload = 'none'
    audio.volume = volume.value
    // Tap the element for the shared analysis graph (no-op without Web Audio).
    audioEngine.attachMediaElement(audio)
    // Belt-and-braces state sync: toggle()/playTrack() set `playing` directly
    // (test DOMs don't fire media events), while these listeners keep the ref
    // honest when playback changes outside our calls — OS media keys, a
    // mid-stream network failure, or the element pausing itself on error.
    audio.addEventListener('play', () => {
      playbackGeneration += 1
      if (!enabled.value) {
        playbackIntent = false
        audio?.pause()
        playing.value = false
      } else {
        playbackIntent = true
        playing.value = true
      }
      persist()
    })
    audio.addEventListener('pause', () => {
      playbackGeneration += 1
      playbackIntent = false
      playing.value = false
      persist()
    })
    audio.addEventListener('error', () => {
      playbackGeneration += 1
      playbackIntent = false
      playing.value = false
      persist()
    })
    // Transport: position + duration for the progress UI.
    audio.addEventListener('loadedmetadata', syncMediaState)
    audio.addEventListener('durationchange', syncMediaState)
    audio.addEventListener('timeupdate', syncMediaState)
    audio.addEventListener('seeked', syncMediaState)
    audio.addEventListener('ended', handleEnded)
  }
  return audio
}

function trackById(id: string): BgmTrack | undefined {
  return tracks.value.find(t => t.id === id)
}

// Prepend the built-in synth pad to any track list (playlist success, the
// gitignored personal manifest, or the empty/degraded fallbacks) so it is
// always present as the first, factory-shipped track.
function withSynth(list: BgmTrack[]): BgmTrack[] {
  return [{ id: BGM_SYNTH_TRACK_ID, title: '', src: '' }, ...list]
}

// Default selection skips the synth pad: when a playlist track exists the first
// of those is the default, and the pad is only the default when it is the sole
// track (a fresh install with no user music).
function defaultTrackId(): string {
  const firstPlaylist = tracks.value.find(tr => tr.id !== BGM_SYNTH_TRACK_ID)
  return firstPlaylist ? firstPlaylist.id : (tracks.value[0]?.id || '')
}

function rebuildShuffleOrder() {
  shuffleOrder = shuffledIndices(tracks.value.length).map(i => tracks.value[i].id)
}

function sourceForId(id: string): string {
  if (id === BGM_LOCAL_TRACK_ID) return localObjectUrl
  const track = trackById(id)
  return track ? musicAssetUrl(track.src) : ''
}

interface BgmManifest {
  tracks: BgmTrack[]
  /**
   * 是否去探测用户的个人清单 `playlist.local.json`。
   *
   * 为什么要有这个开关（2026-10-21 实测）：旧实现无条件先 fetch
   * `playlist.local.json`，靠 404 的 `.catch()` 判"没有个人清单"。语义上没错
   * （注释里也写着"缺失是正常情形"），但**浏览器会把这次 404 记成一条红色
   * console 错误**——逐路由巡检时 40/40 个页面全中，把真问题淹在噪声里。
   * 改成由清单显式声明后，默认路径**零多余请求**。
   */
  localOverlay: boolean
}

async function fetchManifestMeta(name: string): Promise<BgmManifest | null> {
  const res = await fetch(musicAssetUrl(name), { cache: 'no-cache' })
  if (!res.ok) return null
  const data = (await res.json()) as {
    tracks?: Array<Partial<BgmTrack>>
    localOverlay?: unknown
  }
  if (!Array.isArray(data.tracks)) return null
  const tracks = data.tracks
    .map(t => ({
      id: String(t.id || '').trim(),
      title: String(t.title || '').trim(),
      src: String(t.src || '').trim(),
      artist: t.artist ? String(t.artist) : undefined,
      genre: t.genre ? String(t.genre) : undefined,
      duration: typeof t.duration === 'number' && Number.isFinite(t.duration) ? t.duration : undefined,
    }))
    .filter(t => (
      !!t.id
      && !!t.src
      && (HTTPS_STREAM.test(t.src) || isSafeRelativeMusicSource(t.src))
    ))
    .map(t => ({ ...t, title: t.title || t.src }))
  return { tracks, localOverlay: data.localOverlay === true }
}

async function loadPlaylist(): Promise<void> {
  playlistError.value = false
  try {
    const base = await fetchManifestMeta('playlist.json')
    if (base === null) throw new Error('playlist.json missing or malformed')
    // 个人清单只在官方清单显式声明时探测：默认 false → 不产生任何 404。
    // 语义沿用"local 完全替换 base"（见 README）。
    let chosen = base
    if (base.localOverlay) {
      const local = await fetchManifestMeta('playlist.local.json').catch(() => null)
      if (local) chosen = local
    }
    tracks.value = withSynth(chosen.tracks)
  } catch (err: unknown) {
    // Missing manifests are a supported setup (no bundled music): the control
    // degrades to "Choose local file…" only.
    console.warn('[useBgm] playlist load failed:', err instanceof Error ? err.message : err)
    playlistError.value = true
    tracks.value = withSynth([])
  }
  if (shuffle.value) rebuildShuffleOrder()
}

async function playAudio(el: HTMLAudioElement, failureLabel: string): Promise<void> {
  if (!enabled.value) return
  const generation = ++playbackGeneration
  playbackIntent = true
  try {
    await el.play()
    if (generation !== playbackGeneration || !enabled.value) {
      if (!enabled.value || !playbackIntent) el.pause()
      return
    }
    playing.value = true
  } catch (err: unknown) {
    if (generation !== playbackGeneration) return
    // Autoplay policy or a missing/broken file: reflect reality as paused
    // rather than a stuck "playing" button.
    playbackIntent = false
    console.warn(`[useBgm] ${failureLabel}:`, err instanceof Error ? err.message : err)
    playing.value = false
  }
  persist()
}

async function playTrack(id: string): Promise<void> {
  if (!enabled.value) return
  // Built-in synth pad: route to the Web Audio backend (no <audio> element).
  if (id === BGM_SYNTH_TRACK_ID) {
    await audioEngine.unlock()
    startAudioReactivity()
    if (audio) audio.pause() // stop the other backend if it was the source
    const generation = ++playbackGeneration
    playbackIntent = true
    const ok = await synth.play()
    if (generation !== playbackGeneration || !enabled.value) {
      synth.pause()
      playing.value = false
      persist()
      return
    }
    currentTrackId.value = id
    currentTime.value = 0
    duration.value = 0
    playing.value = ok
    persist()
    return
  }
  synth.pause() // stop the pad before starting a file/stream
  const src = sourceForId(id)
  if (!src) return
  const el = getAudio()
  // Only swap the source on a genuine track change so re-playing the current
  // track resumes from where it paused instead of restarting.
  if (currentTrackId.value !== id || !el.src) {
    el.src = src
    currentTrackId.value = id
    currentTime.value = 0
    duration.value = Number.NaN
  }
  // Unlock the shared graph on this user gesture right before playback starts,
  // and bring the analyser loop up (it was a no-op at initBgm, pre-context).
  await audioEngine.unlock()
  startAudioReactivity()
  await playAudio(el, 'play blocked/failed')
}

/**
 * Background-music state + controls, backed by a single shared `Audio`
 * element. Persisted (localStorage `Marguerite-bgm`): the opt-in feature
 * gate, the selected playlist track, volume, repeat/shuffle mode, and whether
 * playback was left on — `initBgm()` restores them, degrading to paused when
 * the browser rejects the resume without a user gesture.
 */
export function useBgm() {
  function initBgm(): Promise<void> {
    if (initPromise) return initPromise
    initPromise = (async () => {
      const saved = readPersisted()
      const restoreGeneration = playbackGeneration
      if (typeof saved.volume === 'number' && Number.isFinite(saved.volume)) {
        volume.value = Math.min(1, Math.max(0, saved.volume))
      }
      // Mode restoration is fully optional: older saves simply keep defaults.
      if (typeof saved.repeat === 'string' && REPEAT_MODES.includes(saved.repeat as RepeatMode)) {
        repeat.value = saved.repeat as RepeatMode
      }
      if (typeof saved.shuffle === 'boolean') shuffle.value = saved.shuffle
      synth.setVolume(volume.value)
      await loadPlaylist()
      startAudioReactivity()
      // Loading manifests yields to user interaction. Never apply the stale
      // startup snapshot over a local-file selection or an intervening
      // disable/re-enable cycle. Keep a real selection, otherwise choose the
      // new default, and persist the current (necessarily non-stale) state.
      if (restoreGeneration !== playbackGeneration) {
        if (!currentTrackId.value) currentTrackId.value = defaultTrackId()
        persist()
        return
      }
      // Restore the selection: a persisted playlist id wins; the session-only
      // local slot (or an id gone from the manifest) falls back to the default
      // track — the first playlist entry, or the built-in pad on a fresh
      // install — unplayed.
      const savedId = String(saved.trackId || '')
      const restorable = savedId !== BGM_LOCAL_TRACK_ID && !!trackById(savedId)
      currentTrackId.value = restorable ? savedId : defaultTrackId()
      // Object URLs and removed playlist entries cannot survive a reload.
      // Normalize that stale persisted state now so the fallback selection is
      // visibly paused and a later reload cannot reinterpret it as playable.
      if (!restorable && (savedId !== '' || saved.playing === true)) persist()
      // Resume only while the feature gate is on: a disable while a session
      // was left playing must never come back as sound on the next launch. A
      // stale/local selection also falls back paused instead of silently
      // switching to and starting the playlist default.
      if (enabled.value && saved.playing === true && restorable) {
        await playTrack(currentTrackId.value)
      }
    })()
    return initPromise
  }

  function setEnabled(on: boolean) {
    enabled.value = on
    if (!on) {
      // Invalidate even a play() that has not yet set `playing`; its eventual
      // settlement cannot restore sound or persisted playback state.
      playbackGeneration += 1
      playbackIntent = false
      if (audio) audio.pause()
      synth.pause()
      playing.value = false
    }
    persist()
  }

  async function toggle(): Promise<void> {
    if (!enabled.value) return
    if (playing.value) {
      playbackGeneration += 1
      playbackIntent = false
      if (audio) audio.pause()
      synth.pause()
      playing.value = false
      persist()
      return
    }
    // Nothing selected yet (fresh profile): start the default first track.
    const id = currentTrackId.value && sourceForId(currentTrackId.value)
      ? currentTrackId.value
      : defaultTrackId()
    if (!id) return
    await playTrack(id)
  }

  async function selectTrack(id: string): Promise<void> {
    if (!enabled.value) return
    if (id === currentTrackId.value && playing.value) return
    await playTrack(id)
  }

  function setVolume(v: number) {
    const clamped = Math.min(1, Math.max(0, v))
    volume.value = clamped
    if (audio) audio.volume = clamped
    synth.setVolume(clamped)
    persist()
  }

  async function playLocalFile(file: File): Promise<void> {
    if (!enabled.value) return
    synth.pause()
    if (localObjectUrl) URL.revokeObjectURL(localObjectUrl)
    localObjectUrl = URL.createObjectURL(file)
    localTrackTitle.value = file.name
    // Set the source directly: playTrack's same-id guard would skip the swap
    // when a *different* file is picked into the same local slot.
    const el = getAudio()
    el.src = localObjectUrl
    currentTrackId.value = BGM_LOCAL_TRACK_ID
    currentTime.value = 0
    duration.value = Number.NaN
    await audioEngine.unlock()
    startAudioReactivity()
    await playAudio(el, 'local file play failed')
  }

  // --- Transport -----------------------------------------------------------

  /** Seek the current file/stream track (synth pad has no timeline). */
  function seek(seconds: number) {
    if (currentTrackId.value === BGM_SYNTH_TRACK_ID) return
    const el = audio
    if (!el || !Number.isFinite(seconds)) return
    const max = Number.isFinite(duration.value) ? duration.value : seconds
    const target = clamp(seconds, 0, Math.max(0, max))
    el.currentTime = target
    currentTime.value = target
  }

  async function stepTrack(direction: 1 | -1): Promise<void> {
    if (!enabled.value) return
    const order = shuffle.value ? shuffleOrder : []
    const target = rotateTrackId(
      tracks.value.map(t => t.id),
      currentTrackId.value,
      direction,
      order,
      true, // manual prev/next always wrap
    )
    if (!target) return
    await playTrack(target)
  }

  function next(): Promise<void> {
    return stepTrack(1)
  }

  function prev(): Promise<void> {
    return stepTrack(-1)
  }

  function setRepeat(mode: RepeatMode) {
    repeat.value = REPEAT_MODES.includes(mode) ? mode : 'off'
    persist()
  }

  function setShuffle(on: boolean) {
    shuffle.value = !!on
    if (shuffle.value) rebuildShuffleOrder()
    persist()
  }

  const currentTitle = computed(() => {
    if (currentTrackId.value === BGM_LOCAL_TRACK_ID) return localTrackTitle.value
    if (currentTrackId.value === BGM_SYNTH_TRACK_ID) return t(BGM_SYNTH_I18N_KEY)
    return trackById(currentTrackId.value)?.title || ''
  })

  // Live output level for the synth pad (0..1, RMS + smoothing). For file or
  // stream tracks the shared engine analyser covers it via `audio` below;
  // this legacy amplitude stays as-is for ParticleField compatibility.
  const amplitude = computed(() => {
    if (currentTrackId.value === BGM_SYNTH_TRACK_ID && playing.value) return synth.amplitude.value
    return 0
  })

  return {
    enabled,
    tracks,
    playing,
    currentTrackId,
    currentTitle,
    amplitude,
    volume,
    playlistError,
    localTrackTitle,
    // Transport
    currentTime,
    duration,
    repeat,
    shuffle,
    // Shared audio-analysis data (spectrum / bands / beat) for visualisers.
    audio: viz,
    initBgm,
    setEnabled,
    toggle,
    selectTrack,
    setVolume,
    playLocalFile,
    seek,
    next,
    prev,
    setRepeat,
    setShuffle,
  }
}
