<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import Icon from '@/components/Icon.vue'
import { useDocumentEvent } from '@/composables/useDocumentEvent'
import { useDialogLayer } from '@/composables/useDialogA11y'
import { useBgm, BGM_LOCAL_TRACK_ID, BGM_SYNTH_TRACK_ID, type RepeatMode } from '@/composables/useBgm'
import { formatClock, fractionToSeek } from '@/composables/bgmFormat'

// Topbar background-music control: a split button next to the language/theme
// menus. The note button toggles play/pause of the current track (and turns
// into a 3-bar bouncing equaliser while playing); the caret opens a player panel
// (reusing the global .theme-menu* classes so the three topbar popovers can
// never drift in look) with transport (prev/play·pause/next), a seekable
// progress bar, three-state repeat + shuffle, the playlist.json tracks, a
// volume slider, and a session-only "Choose local file…" escape hatch.

const { t } = useI18n()
const {
  tracks,
  playing,
  currentTrackId,
  currentTitle,
  currentTime,
  duration,
  repeat,
  shuffle,
  volume,
  localTrackTitle,
  initBgm,
  toggle,
  selectTrack,
  setVolume,
  playLocalFile,
  seek,
  next,
  prev,
  setRepeat,
  setShuffle,
} = useBgm()

const menuOpen = ref(false)
const toggleRef = ref<HTMLButtonElement | null>(null)
const caretRef = ref<HTMLButtonElement | null>(null)
const fileInputRef = ref<HTMLInputElement | null>(null)
useDialogLayer(computed(() => menuOpen.value))

onMounted(() => { void initBgm() })

function onToggle() {
  void toggle()
}

function onPrev() {
  void prev()
}

function onNext() {
  void next()
}

// Repeat cycles off → all → one → off; the icon + aria-label track the state.
const REPEAT_NEXT: Record<RepeatMode, RepeatMode> = { off: 'all', all: 'one', one: 'off' }
function onRepeat() {
  setRepeat(REPEAT_NEXT[repeat.value])
}
const repeatIcon = computed(() => (repeat.value === 'one' ? 'repeatOne' : 'repeat'))
const repeatLabel = computed(() =>
  repeat.value === 'off'
    ? t('chrome.bgm.repeatOff')
    : repeat.value === 'all'
      ? t('chrome.bgm.repeatAll')
      : t('chrome.bgm.repeatOne'),
)

function onShuffle() {
  setShuffle(!shuffle.value)
}

// Seek bar: the synth pad has no timeline, so the bar stays inert there.
const hasTimeline = computed(
  () => Number.isFinite(duration.value) && duration.value > 0 && currentTrackId.value !== BGM_SYNTH_TRACK_ID,
)
const progressValue = computed(() =>
  hasTimeline.value ? Math.min(1000, Math.max(0, (currentTime.value / duration.value) * 1000)) : 0,
)
function onSeekInput(e: Event) {
  const ratio = Number((e.target as HTMLInputElement).value) / 1000
  const target = fractionToSeek(ratio, duration.value)
  if (target !== null) seek(target)
}

function pickTrack(id: string) {
  void selectTrack(id)
  menuOpen.value = false
  toggleRef.value?.focus()
}

function onVolumeInput(e: Event) {
  const target = e.target as HTMLInputElement
  setVolume(Number(target.value))
}

function chooseLocalFile() {
  fileInputRef.value?.click()
}

function onLocalFilePicked(e: Event) {
  const input = e.target as HTMLInputElement
  const file = input.files?.[0]
  // Clear so re-picking the same file fires change again.
  input.value = ''
  if (!file) return
  void playLocalFile(file)
  menuOpen.value = false
  toggleRef.value?.focus()
}

useDocumentEvent('click', (e) => {
  if (!menuOpen.value) return
  const wrap = caretRef.value?.closest('.bgm-menu-wrap')
  if (wrap && e.target instanceof Node && !wrap.contains(e.target)) {
    menuOpen.value = false
  }
})

useDocumentEvent('keydown', (e) => {
  if (e.key === 'Escape' && menuOpen.value) {
    menuOpen.value = false
    caretRef.value?.focus()
  }
})
</script>

<template>
  <div class="theme-menu-wrap bgm-menu-wrap">
    <button
      ref="toggleRef"
      type="button"
      class="btn btn--icon btn--ghost bgm-toggle"
      :class="{ 'is-playing': playing }"
      :title="playing ? t('chrome.bgm.pause') : t('chrome.bgm.play')"
      :aria-label="playing ? t('chrome.bgm.pause') : t('chrome.bgm.play')"
      :aria-pressed="playing"
      data-testid="bgm-toggle"
      @click="onToggle"
    >
      <!-- While playing the note swaps for a 3-bar bouncing equaliser; paused
           keeps the music glyph. -->
      <span v-if="playing" class="bgm-eq" aria-hidden="true">
        <span></span><span></span><span></span>
      </span>
      <Icon v-else name="music" :size="16" />
    </button>
    <button
      ref="caretRef"
      type="button"
      class="btn btn--ghost bgm-caret"
      :title="t('chrome.bgm.label')"
      :aria-label="t('chrome.bgm.label')"
      aria-haspopup="menu"
      :aria-expanded="menuOpen"
      data-testid="bgm-menu-trigger"
      @click.stop="menuOpen = !menuOpen"
    >
      <Icon name="chevronDown" :size="12" />
    </button>
    <div v-if="menuOpen" class="theme-menu bgm-menu" role="menu" :aria-label="t('chrome.bgm.label')">
      <!-- Transport row: prev / play·pause / next / repeat / shuffle. -->
      <div class="bgm-menu__transport" role="group" :aria-label="t('chrome.bgm.transport')">
        <button
          type="button"
          class="bgm-tbtn"
          :aria-label="t('chrome.bgm.prev')"
          data-testid="bgm-prev"
          @click.stop="onPrev"
        >
          <Icon name="skipBack" :size="15" />
        </button>
        <button
          type="button"
          class="bgm-tbtn bgm-tbtn--main"
          :aria-label="playing ? t('chrome.bgm.pause') : t('chrome.bgm.play')"
          data-testid="bgm-playpause"
          @click.stop="onToggle"
        >
          <Icon v-if="!playing" name="play" :size="15" />
          <span v-else class="bgm-eq bgm-eq--sm" aria-hidden="true">
            <span></span><span></span><span></span>
          </span>
        </button>
        <button
          type="button"
          class="bgm-tbtn"
          :aria-label="t('chrome.bgm.next')"
          data-testid="bgm-next"
          @click.stop="onNext"
        >
          <Icon name="skipForward" :size="15" />
        </button>
        <button
          type="button"
          class="bgm-tbtn"
          :class="{ 'is-active': repeat !== 'off' }"
          :aria-pressed="repeat !== 'off'"
          :aria-label="repeatLabel"
          data-testid="bgm-repeat"
          @click.stop="onRepeat"
        >
          <Icon :name="repeatIcon" :size="14" />
        </button>
        <button
          type="button"
          class="bgm-tbtn"
          :class="{ 'is-active': shuffle }"
          :aria-pressed="shuffle"
          :aria-label="t('chrome.bgm.shuffle')"
          data-testid="bgm-shuffle"
          @click.stop="onShuffle"
        >
          <Icon name="shuffle" :size="14" />
        </button>
      </div>
      <!-- Seek row: dragging must not dismiss the menu. -->
      <div class="bgm-menu__seek" role="none" @click.stop>
        <span class="bgm-seek__time" data-testid="bgm-time-current">{{ formatClock(currentTime) }}</span>
        <input
          type="range"
          min="0"
          max="1000"
          step="10"
          :value="progressValue"
          :disabled="!hasTimeline"
          :aria-label="t('chrome.bgm.seek')"
          data-testid="bgm-seek"
          @input="onSeekInput"
        />
        <span class="bgm-seek__time" data-testid="bgm-time-total">{{ formatClock(duration) }}</span>
      </div>
      <button
        v-for="track in tracks"
        :key="track.id"
        type="button"
        class="theme-menu__item"
        role="menuitemradio"
        :aria-checked="currentTrackId === track.id"
        :data-testid="`bgm-track-${track.id}`"
        @click="pickTrack(track.id)"
      >
        <Icon name="music" :size="14" />
        <span class="bgm-menu__title">{{ track.title }}</span>
        <Icon v-if="currentTrackId === track.id" class="theme-menu__check" name="check" :size="14" />
      </button>
      <!-- Session-only local pick: shown as a selectable row once a file has
           been chosen, so it can be toggled back to after a playlist track. -->
      <button
        v-if="localTrackTitle"
        type="button"
        class="theme-menu__item"
        role="menuitemradio"
        :aria-checked="currentTrackId === BGM_LOCAL_TRACK_ID"
        data-testid="bgm-track-local"
        @click="pickTrack(BGM_LOCAL_TRACK_ID)"
      >
        <Icon name="fileText" :size="14" />
        <span class="bgm-menu__title">{{ localTrackTitle }}</span>
        <Icon v-if="currentTrackId === BGM_LOCAL_TRACK_ID" class="theme-menu__check" name="check" :size="14" />
      </button>
      <div v-if="!tracks.length && !localTrackTitle" class="bgm-menu__empty">
        {{ t('chrome.bgm.noTracks') }}
      </div>
      <!-- Volume row: adjusting must not dismiss the menu. -->
      <div class="bgm-menu__volume" role="none" @click.stop>
        <Icon name="volume" :size="14" />
        <input
          type="range"
          min="0"
          max="1"
          step="0.05"
          :value="volume"
          :aria-label="t('chrome.bgm.volume')"
          data-testid="bgm-volume"
          @input="onVolumeInput"
        />
      </div>
      <button
        type="button"
        class="theme-menu__item theme-menu__item--more"
        role="menuitem"
        data-testid="bgm-choose-local"
        @click="chooseLocalFile"
      >
        <Icon name="paperclip" :size="14" />
        <span>{{ t('chrome.bgm.chooseLocalFile') }}</span>
      </button>
      <input
        ref="fileInputRef"
        type="file"
        accept="audio/*"
        class="bgm-file-input"
        tabindex="-1"
        aria-hidden="true"
        @change="onLocalFilePicked"
      />
    </div>
    <!-- Screen-reader note of what is currently playing; visual users get the
         accent-lit note button instead. -->
    <span v-if="playing && currentTitle" class="bgm-sr-now-playing">{{ currentTitle }}</span>
  </div>
</template>

<style scoped>
.bgm-toggle {
  color: var(--text-muted);
}

.bgm-toggle.is-playing {
  color: var(--accent);
}

/* Narrow caret hugging the note button so the pair reads as one control. */
.bgm-caret {
  display: inline-flex;
  align-items: center;
  width: auto;
  padding: 0 2px;
  margin-left: -6px;
  color: var(--text-muted);
}

.bgm-caret:hover {
  color: var(--text);
}

.bgm-menu {
  min-width: 220px;
  max-width: 300px;
}

/* --- Mini equaliser (collapsed toggle + panel main button) ---------------- */
.bgm-eq {
  display: inline-flex;
  align-items: flex-end;
  justify-content: center;
  gap: 2px;
  height: 12px;
}
.bgm-eq--sm {
  height: 10px;
}
.bgm-eq > span {
  width: 2px;
  height: 100%;
  background: currentColor;
  border-radius: var(--radius-xs);
  transform-origin: bottom;
  animation: bgm-eq-bounce var(--dur-pulse) ease-in-out infinite;
}
.bgm-eq > span:nth-child(2) {
  animation-delay: calc(var(--dur-pulse) * 0.18);
}
.bgm-eq > span:nth-child(3) {
  animation-delay: calc(var(--dur-pulse) * 0.36);
}
@keyframes bgm-eq-bounce {
  0%, 100% { transform: scaleY(0.3); }
  50% { transform: scaleY(1); }
}

/* --- Transport row ---------------------------------------------------------- */
.bgm-menu__transport {
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 2px;
  padding: 6px 8px 2px;
}
.bgm-tbtn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 28px;
  height: 28px;
  border: none;
  border-radius: var(--radius-control);
  background: transparent;
  color: var(--text-muted);
  cursor: pointer;
  transition:
    color var(--dur-fast) var(--ease-standard),
    background var(--dur-fast) var(--ease-standard);
}
.bgm-tbtn:hover {
  color: var(--text);
  background: var(--bg-hover);
}
.bgm-tbtn.is-active {
  color: var(--accent);
}
.bgm-tbtn--main {
  width: 32px;
  height: 32px;
  color: var(--accent);
}

/* --- Seek row ---------------------------------------------------------------- */
.bgm-menu__seek {
  display: flex;
  align-items: center;
  gap: var(--sp-2);
  padding: 4px 10px 6px;
  color: var(--text-muted);
}
.bgm-seek__time {
  font-family: var(--font-mono);
  font-size: 10px;
  min-width: 34px;
  text-align: center;
  font-variant-numeric: tabular-nums;
}
.bgm-menu__seek input[type='range'] {
  flex: 1;
  min-width: 0;
  accent-color: var(--accent);
}
.bgm-menu__seek input[type='range']:disabled {
  opacity: var(--state-disabled-opacity);
}

.bgm-menu__title {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.bgm-menu__empty {
  padding: 7px 10px;
  color: var(--text-muted);
  font-size: var(--fs-sm);
}

.bgm-menu__volume {
  display: flex;
  align-items: center;
  gap: var(--sp-2);
  padding: 7px 10px;
  color: var(--text-muted);
}

.bgm-menu__volume input[type='range'] {
  flex: 1;
  min-width: 0;
  accent-color: var(--accent);
}

.bgm-file-input {
  display: none;
}

/* Off-screen but screen-reader-reachable "now playing" note. */
.bgm-sr-now-playing {
  position: absolute;
  width: 1px;
  height: 1px;
  margin: -1px;
  padding: 0;
  overflow: hidden;
  clip: rect(0 0 0 0);
  white-space: nowrap;
  border: 0;
}
</style>
