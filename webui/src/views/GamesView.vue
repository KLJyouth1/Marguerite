<template>
  <div class="control-stage games-stage">
    <header class="control-stage__header">
      <div class="control-stage__title-block">
        <h1 class="control-stage__title">{{ t('games.title') }}</h1>
        <p class="control-stage__subtitle">{{ t('games.subtitle') }}</p>
      </div>
      <div class="control-stage__actions">
        <button class="btn btn--ghost" @click="load">
          <Icon name="refresh" :size="16" /><span>{{ t('games.refresh') }}</span>
        </button>
      </div>
    </header>

    <!-- 统一错误态组件（含重试），不再自绘一条 error 条 -->
    <ErrorState v-if="error" :message="error" :on-retry="load" />

    <!-- 能力健康度（plans.capabilities → 透明统计） -->
    <section v-if="stats" class="games-health">
      <div class="games-health__title">{{ t('games.health.title') }}</div>
      <div class="games-health__row">
        <i18n-t keypath="games.health.implemented" tag="span" class="games-health__pill">
          <template #done><b>{{ stats.implemented }}</b></template>
          <template #total>{{ stats.total }}</template>
        </i18n-t>
        <span class="games-health__pill games-health__pill--real">{{ t('games.health.real', { n: stats.real }) }}</span>
        <span class="games-health__pill games-health__pill--partial">{{ t('games.health.partial', { n: stats.partial }) }}</span>
        <span class="games-health__pill games-health__pill--narrative">{{ t('games.health.narrative', { n: stats.narrative }) }}</span>
      </div>
      <div v-if="narrative.length" class="games-health__narrative">
        {{ t('games.health.planned') }}
        <span v-for="id in narrative" :key="id" class="games-health__tag">{{ id }}</span>
      </div>
    </section>

    <section class="games-grid">
      <div v-for="g in games" :key="g.id" class="games-card">
        <div class="games-card__head">
          <Icon name="play" :size="16" />
          <strong>{{ g.name }}</strong>
          <span class="games-card__controls">{{ g.controls }}</span>
        </div>
        <div class="games-card__body">
          <input
            v-model="inputs[g.id]"
            class="games-card__input"
            :placeholder="g.id === 'tictactoe' ? t('games.placeholder.board') : t('games.placeholder.moves')"
          />
          <button class="btn btn--primary" :disabled="busy === g.id" @click="play(g.id)">
            {{ busy === g.id ? t('games.running') : t('games.run') }}
          </button>
        </div>
        <pre v-if="results[g.id]" class="games-card__render">{{ results[g.id] }}</pre>
      </div>
    </section>

    <section v-if="!error && games.length === 0 && loading" class="games-empty">
      <Icon name="info" :size="16" /><span>{{ t('games.loading') }}</span>
    </section>
    <section v-else-if="!error && games.length === 0" class="games-empty">
      <Icon name="info" :size="16" /><span>{{ t('games.empty') }}</span>
    </section>
  </div>
</template>

<script setup lang="ts">
import { ref, onMounted } from 'vue'
import { useI18n } from 'vue-i18n'
import ErrorState from '@/components/ErrorState.vue'
import Icon from '@/components/Icon.vue'
import { useRpcStore } from '@/stores/rpc'

const { t } = useI18n()
const rpc = useRpcStore()

interface GameInfo { id: string; name: string; controls: string; status: string }
interface GamesListResult { total: number; games: GameInfo[] }
interface GamePlayResult { ok: boolean; game: string; moves: string[]; render?: string; error?: string }
interface CapabilityStats { total: number; real: number; partial: number; implemented: number; narrative: number }
interface CapabilitiesPayload { stats: CapabilityStats; narrativeIds?: string[] }

const games = ref<GameInfo[]>([])
const stats = ref<CapabilityStats | null>(null)
const narrative = ref<string[]>([])
const error = ref('')
const loading = ref(false)
const busy = ref('')
const inputs = ref<Record<string, string>>({ tictactoe: '0 3 1 4 2', '2048': 'left up left', snake: 'up left down' })
const results = ref<Record<string, string>>({})

async function load() {
  error.value = ''
  // 「加载中」必须是一个**会结束**的状态：此前空清单与「加载中」共用同一段文案，
  // 请求成功但返回 0 条时，界面会永远停在「加载中…」——对用户是假的进度承诺。
  loading.value = true
  try {
    const g = await rpc.call<GamesListResult>('games.list', {})
    games.value = g.games ?? []
    if (rpc.supportsMethod('plans.capabilities')) {
      const c = await rpc.call<CapabilitiesPayload>('plans.capabilities', {})
      stats.value = c.stats ?? null
      narrative.value = c.narrativeIds ?? []
    }
  } catch (e) {
    error.value = e instanceof Error ? e.message : String(e)
  } finally {
    loading.value = false
  }
}

async function play(gameId: string) {
  busy.value = gameId
  error.value = ''
  try {
    const moves = (inputs.value[gameId] ?? '').trim().split(/\s+/).filter(Boolean)
    const r = await rpc.call<GamePlayResult>('games.play', { game: gameId, moves })
    results.value[gameId] = r.ok ? (r.render ?? '') : (r.error ?? t('games.playFailed'))
  } catch (e) {
    results.value[gameId] = e instanceof Error ? e.message : String(e)
  } finally {
    busy.value = ''
  }
}

onMounted(load)
</script>

<style scoped>
.games-stage { padding: 24px 28px; max-width: 1100px; margin: 0 auto; }
.games-health { margin-top: 16px; padding: 12px 14px; background: var(--card); border: 1px solid var(--hairline); border-radius: var(--radius-md); }
.games-health__title { font-size: 13px; font-weight: 600; margin-bottom: 8px; }
.games-health__row { display: flex; flex-wrap: wrap; gap: 8px; }
.games-health__pill { padding: 4px 10px; border-radius: 999px; background: color-mix(in srgb, var(--text) 8%, var(--card)); font-size: 12px; }
.games-health__pill--real { color: var(--accent); }
.games-health__pill--partial { color: var(--accent-secondary); }
.games-health__pill--narrative { color: var(--text-dim); }
.games-health__narrative { margin-top: 8px; font-size: 12px; color: var(--text-dim); }
.games-health__tag { display: inline-block; margin: 0 6px 4px 0; padding: 2px 8px; border: 1px solid var(--hairline); border-radius: 999px; }
.games-grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(280px, 1fr)); gap: 14px; margin-top: 18px; }
.games-card { background: var(--card); border: 1px solid var(--hairline); border-radius: var(--radius-md); padding: 14px; }
.games-card__head { display: flex; align-items: center; gap: 8px; margin-bottom: 10px; }
.games-card__controls { margin-left: auto; font-size: 11px; color: var(--text-dim); }
.games-card__body { display: flex; gap: 8px; }
.games-card__input { flex: 1; min-width: 0; padding: 8px 10px; border: 1px solid var(--hairline); border-radius: var(--radius-sm); background: var(--bg); color: var(--text); font-family: var(--font-mono); font-size: 12px; }
.games-card__render { margin: 10px 0 0; padding: 10px; border-radius: var(--radius-sm); background: color-mix(in srgb, var(--bg) 80%, var(--card)); border: 1px solid var(--hairline); font-family: var(--font-mono); font-size: 12px; white-space: pre-wrap; }
.games-empty { margin-top: 18px; display: flex; gap: 8px; align-items: center; color: var(--text-dim); font-size: 13px; }
</style>
