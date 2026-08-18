<template>
  <div class="control-stage evolve-stage">
    <header class="control-stage__header">
      <div class="control-stage__title-block">
        <h1 class="control-stage__title">自进化</h1>
        <p class="control-stage__subtitle">EvolutionRunner — prompt 自进化循环：诊断 → 变异 → 门禁 → 采纳/回滚</p>
      </div>
      <div class="control-stage__actions">
        <button class="btn btn--ghost" @click="loadHistory">
          <Icon name="refresh" :size="16" /><span>刷新</span>
        </button>
      </div>
    </header>

    <section v-if="error" class="evolve-error">{{ error }}</section>

    <section class="evolve-stats">
      <div class="evolve-stat">
        <div class="evolve-stat__label">总轮次</div>
        <div class="evolve-stat__value">{{ records.length }}</div>
      </div>
      <div class="evolve-stat">
        <div class="evolve-stat__label">已采纳</div>
        <div class="evolve-stat__value evolve-stat__value--ok">{{ acceptedCount }}</div>
      </div>
      <div class="evolve-stat">
        <div class="evolve-stat__label">最新评分</div>
        <div class="evolve-stat__value">{{ latestScore }}</div>
      </div>
    </section>

    <section class="evolve-timeline">
      <div v-for="r in records" :key="r.round" class="evolve-record">
        <div class="evolve-record__dot" :class="{ 'is-accepted': r.accepted }"></div>
        <div class="evolve-record__body">
          <div class="evolve-record__header">
            <span class="evolve-record__round">第 {{ r.round }} 轮</span>
            <span class="evolve-record__badge" :class="r.accepted ? 'badge--ok' : 'badge--no'">
              {{ r.accepted ? '采纳' : '拒绝' }}
            </span>
            <span class="evolve-record__time">{{ r.timestamp }}</span>
          </div>
          <p class="evolve-record__diagnosis">{{ r.diagnosis }}</p>
          <div class="evolve-record__scores">
            <span>评分: {{ r.score_before.toFixed(3) }} → <b>{{ r.score_after.toFixed(3) }}</b></span>
            <span v-if="r.rejection_reason" class="evolve-record__rejection">拒绝原因: {{ r.rejection_reason }}</span>
          </div>
        </div>
      </div>
      <div v-if="!records.length && !error" class="evolve-empty">暂无进化记录</div>
    </section>
  </div>
</template>

<script setup lang="ts">
import { ref, computed, onMounted } from 'vue'
import Icon from '@/components/Icon.vue'
import { useRpcStore } from '@/stores/rpc'

const rpc = useRpcStore()

interface EvolveRecord {
  round: number; timestamp: string; diagnosis: string;
  score_before: number; score_after: number; accepted: boolean;
  rejection_reason: string | null;
}

const records = ref<EvolveRecord[]>([])
const error = ref('')

const acceptedCount = computed(() => records.value.filter(r => r.accepted).length)
const latestScore = computed(() => records.value.length ? records.value[records.value.length - 1].score_after.toFixed(3) : '—')

async function loadHistory() {
  error.value = ''
  try {
    const res = await rpc.call<{ records: EvolveRecord[]; error?: string }>('evolve.history')
    if (res.error) { error.value = res.error } else { records.value = (res.records || []).reverse() }
  } catch (e) {
    error.value = `加载失败: ${e instanceof Error ? e.message : String(e)}`
  }
}

onMounted(loadHistory)
</script>

<style scoped>
.evolve-stage { padding: 24px 28px; max-width: 900px; margin: 0 auto; }
.evolve-error { margin: 16px 0; padding: 10px 14px; background: color-mix(in srgb, var(--danger) 10%, var(--card)); border: 1px solid color-mix(in srgb, var(--danger) 30%, transparent); border-radius: var(--radius-sm); color: var(--danger); font-size: 12px; }
.evolve-stats { display: grid; grid-template-columns: repeat(3, 1fr); gap: 12px; margin: 20px 0; }
.evolve-stat { background: var(--card); border: 1px solid var(--hairline); border-radius: var(--radius-md); padding: 16px; text-align: center; }
.evolve-stat__label { font-size: 11px; color: var(--text-dim); }
.evolve-stat__value { font-size: 28px; font-weight: 700; margin-top: 4px; }
.evolve-stat__value--ok { color: var(--ok); }
.evolve-timeline { position: relative; padding-left: 24px; }
.evolve-timeline::before { content: ''; position: absolute; left: 7px; top: 0; bottom: 0; width: 2px; background: var(--hairline); }
.evolve-record { position: relative; margin-bottom: 16px; }
.evolve-record__dot { position: absolute; left: -22px; top: 4px; width: 12px; height: 12px; border-radius: 50%; background: var(--hairline); border: 2px solid var(--bg); }
.evolve-record__dot.is-accepted { background: var(--ok); }
.evolve-record__body { background: var(--card); border: 1px solid var(--hairline); border-radius: var(--radius-md); padding: 12px 14px; }
.evolve-record__header { display: flex; align-items: center; gap: 10px; }
.evolve-record__round { font-weight: 600; font-size: 13px; }
.evolve-record__badge { font-size: 10px; font-weight: 700; padding: 2px 8px; border-radius: var(--radius-pill); }
.badge--ok { background: color-mix(in srgb, var(--ok) 15%, transparent); color: var(--ok); }
.badge--no { background: color-mix(in srgb, var(--danger) 15%, transparent); color: var(--danger); }
.evolve-record__time { margin-left: auto; font-size: 10px; color: var(--text-dim); }
.evolve-record__diagnosis { font-size: 12px; color: var(--text-muted); margin: 6px 0; line-height: 1.5; }
.evolve-record__scores { display: flex; gap: 16px; font-size: 11px; color: var(--text-dim); }
.evolve-record__rejection { color: var(--danger); }
.evolve-empty { text-align: center; padding: 40px; color: var(--text-dim); font-size: 13px; }
</style>
