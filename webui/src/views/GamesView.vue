<template>
  <div class="control-stage games-stage">
    <header class="control-stage__header">
      <div class="control-stage__title-block">
        <h1 class="control-stage__title">内置小游戏</h1>
        <p class="control-stage__subtitle">Rust 引擎 · 离线可玩 · 动作序列驱动（2048 / 井字棋 / 贪吃蛇）</p>
      </div>
      <div class="control-stage__actions">
        <button class="btn btn--ghost" @click="load">
          <Icon name="refresh" :size="16" /><span>刷新</span>
        </button>
      </div>
    </header>

    <section v-if="error" class="games-error">{{ error }}</section>

    <!-- 能力健康度（plans.capabilities → 透明统计） -->
    <section v-if="stats" class="games-health">
      <div class="games-health__title">Agent 能力健康度</div>
      <div class="games-health__row">
        <span class="games-health__pill">已实装 <b>{{ stats.implemented }}</b>/{{ stats.total }}</span>
        <span class="games-health__pill games-health__pill--real">真货 {{ stats.real }}</span>
        <span class="games-health__pill games-health__pill--partial">部分 {{ stats.partial }}</span>
        <span class="games-health__pill games-health__pill--narrative">纯叙事 {{ stats.narrative }}</span>
      </div>
      <div v-if="narrative.length" class="games-health__narrative">
        规划中（用户安装后可翻转）：
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
          <input v-model="inputs[g.id]" class="games-card__input" :placeholder="g.id === 'tictactoe' ? '如：0 3 1 4 2' : '如：left up left'" />
          <button class="btn btn--primary" :disabled="busy === g.id" @click="play(g.id)">
            {{ busy === g.id ? '运行中…' : '运行' }}
          </button>
        </div>
        <pre v-if="results[g.id]" class="games-card__render">{{ results[g.id] }}</pre>
      </div>
    </section>

    <section v-if="games.length === 0 && !error" class="games-empty">
      <Icon name="info" :size="16" /><span>加载中…（需网关支持 games.list / games.play）</span>
    </section>
  </div>
</template>

<script setup lang="ts">
import { ref, onMounted } from 'vue'
import Icon from '@/components/Icon.vue'
import { useRpcStore } from '@/stores/rpc'

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
const busy = ref('')
const inputs = ref<Record<string, string>>({ tictactoe: '0 3 1 4 2', '2048': 'left up left', snake: 'up left down' })
const results = ref<Record<string, string>>({})

async function load() {
  error.value = ''
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
  }
}

async function play(gameId: string) {
  busy.value = gameId
  error.value = ''
  try {
    const moves = (inputs.value[gameId] ?? '').trim().split(/\s+/).filter(Boolean)
    const r = await rpc.call<GamePlayResult>('games.play', { game: gameId, moves })
    results.value[gameId] = r.ok ? (r.render ?? '') : (r.error ?? '运行失败')
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
.games-error { margin: 16px 0; padding: 10px 14px; background: color-mix(in srgb, var(--danger) 10%, var(--card)); border: 1px solid color-mix(in srgb, var(--danger) 30%, transparent); border-radius: var(--radius-sm); color: var(--danger); font-size: 12px; }
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
