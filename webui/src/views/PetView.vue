<template>
  <div class="control-stage pet-stage">
    <header class="control-stage__header">
      <div class="control-stage__title-block">
        <h1 class="control-stage__title">鲸鱼娘</h1>
        <p class="control-stage__subtitle">DSH Pet 原生化 · 随 AI 运行状态切换动作，摸头/投喂小鱼干提升好感度（状态持久化）</p>
      </div>
      <div class="control-stage__actions">
        <button class="btn btn--ghost" @click="load">
          <Icon name="refresh" :size="16" /><span>刷新</span>
        </button>
      </div>
    </header>

    <section v-if="error" class="pet-error">{{ error }}</section>

    <section class="pet-card">
      <!-- 内联 SVG 鲸鱼（非 emoji，mood 驱动表情） -->
      <svg viewBox="0 0 200 120" class="pet-whale" role="img" aria-label="鲸鱼娘">
        <g v-if="happy || working" stroke="var(--accent-secondary)" stroke-width="2.5" fill="none" stroke-linecap="round">
          <path d="M100 8 Q104 -2 100 -10" />
          <path d="M106 10 Q112 2 108 -6" />
        </g>
        <path d="M160 55 Q185 38 194 28 Q184 60 194 92 Q185 82 160 65 Z" fill="var(--accent-secondary)" />
        <ellipse cx="105" cy="60" rx="62" ry="36" fill="var(--accent)" />
        <ellipse cx="112" cy="76" rx="44" ry="18" fill="color-mix(in srgb, var(--card) 85%, transparent)" />
        <ellipse cx="120" cy="72" rx="14" ry="8" fill="var(--accent-secondary)" transform="rotate(-18 120 72)" />
        <circle cx="72" cy="50" :r="sleepy ? 2.5 : 5" fill="var(--text)" />
        <circle v-if="working" cx="72" cy="50" r="8.5" fill="none" stroke="var(--text)" stroke-width="1" opacity="0.4" />
        <path v-if="happy || working" d="M56 66 Q66 76 78 66" stroke="var(--text)" stroke-width="2" fill="none" stroke-linecap="round" />
        <line v-else-if="sleepy" x1="60" y1="66" x2="72" y2="66" stroke="var(--text)" stroke-width="2" stroke-linecap="round" />
        <path v-else d="M58 64 Q68 70 76 64" stroke="var(--text)" stroke-width="2" fill="none" stroke-linecap="round" />
        <ellipse v-if="happy" cx="52" cy="62" rx="7" ry="4" fill="var(--accent-secondary)" opacity="0.7" />
        <text v-if="sleepy" x="86" y="30" font-size="13" fill="var(--text)" opacity="0.7">Z z z</text>
        <animateTransform attributeName="transform" type="translate" :values="working ? '0 0; 0 -4; 0 0' : '0 0; 0 -4; 0 0'" :dur="working ? '1.2s' : '3.2s'" repeatCount="indefinite" />
      </svg>

      <div class="pet-mood">{{ moodLabel }}</div>
      <div class="pet-stats">能量 {{ energy }}/100 · 好感度 {{ affection }}</div>
      <div class="pet-bar">
        <div class="pet-bar__fill" :style="{ width: energy + '%', background: energy < 20 ? 'var(--warn)' : 'var(--accent)' }"></div>
      </div>
      <div class="pet-actions">
        <button class="btn btn--primary" :disabled="busy" @click="act('pet')">
          <Icon name="sparkles" :size="16" /><span>摸头</span>
        </button>
        <button class="btn btn--secondary" :disabled="busy" @click="act('feed')">
          <Icon name="plus" :size="16" /><span>投喂小鱼干</span>
        </button>
      </div>
      <div class="pet-hint">能量每 10 秒自然衰减 1；低于 20 进入困倦。互动回能并提升好感度。</div>
    </section>
  </div>
</template>

<script setup lang="ts">
import { ref, computed, onMounted, onUnmounted } from 'vue'
import Icon from '@/components/Icon.vue'
import { useRpcStore } from '@/stores/rpc'

const rpc = useRpcStore()

interface PetView { mood: string; energy: number; affection: number }

const pet = ref<PetView | null>(null)
const busy = ref(false)
const error = ref('')

const mood = computed(() => pet.value?.mood ?? 'idle')
const energy = computed(() => pet.value?.energy ?? 100)
const affection = computed(() => pet.value?.affection ?? 0)
const happy = computed(() => mood.value === 'happy')
const sleepy = computed(() => mood.value === 'sleepy')
const working = computed(() => mood.value === 'working')

const moodLabel = computed(() => {
  switch (mood.value) {
    case 'happy': return '开心'
    case 'sleepy': return '困倦'
    case 'working': return '工作中'
    default: return '空闲'
  }
})

async function load() {
  try {
    pet.value = await rpc.call<PetView>('pet.status')
  } catch (e) {
    error.value = e instanceof Error ? e.message : String(e)
  }
}

async function act(action: 'pet' | 'feed') {
  busy.value = true
  error.value = ''
  try {
    pet.value = await rpc.call<PetView>('pet.action', { action })
  } catch (e) {
    error.value = e instanceof Error ? e.message : String(e)
  } finally {
    busy.value = false
  }
}

let timer: ReturnType<typeof setInterval> | null = null
onMounted(() => {
  void load()
  timer = setInterval(() => void load(), 15000)
})
onUnmounted(() => {
  if (timer) clearInterval(timer)
})
</script>

<style scoped>
.pet-stage { padding: 24px 28px; max-width: 900px; margin: 0 auto; }
.pet-error { margin: 16px 0; padding: 10px 14px; background: color-mix(in srgb, var(--danger) 10%, var(--card)); border: 1px solid color-mix(in srgb, var(--danger) 30%, transparent); border-radius: var(--radius-sm); color: var(--danger); font-size: 12px; }
.pet-card { margin-top: 16px; padding: 28px; background: var(--card); border: 1px solid var(--hairline); border-radius: var(--radius-md); text-align: center; }
.pet-whale { width: 100%; max-width: 340px; display: block; margin: 0 auto; }
.pet-mood { font-size: 15px; font-weight: 700; color: var(--accent); margin-top: 8px; }
.pet-stats { font-size: 12px; color: var(--text-dim); margin-top: 4px; }
.pet-bar { height: 8px; border-radius: var(--radius-pill); background: var(--bg-hover); margin: 14px auto 0; max-width: 300px; overflow: hidden; }
.pet-bar__fill { height: 100%; transition: width var(--dur-base) var(--ease-standard); }
.pet-actions { display: flex; gap: 8px; justify-content: center; margin-top: 16px; }
.pet-hint { font-size: 10px; color: var(--text-dim); margin-top: 12px; }
</style>
