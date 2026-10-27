<!-- Viky 数字生命体 · 共享大脑视图
     PSI 意识引擎（Aris Brain）的前端具身面板：
       · 八维实时情感波动（valence/arousal/curiosity/confidence/connection/growth/certainty/existence）
       · Φ 自我存在感（0.999 封顶）
       · 相位节律（白天学习 / 夜间离线整合）
       · 因果推理引擎规模 + PSI 循环计数 + 学习计数
    全程只读真实引擎状态（brain.status），绝无模拟。
    另聚合「共享大脑」的本地资产规模：知识库页数 / 图谱节点边 / 记忆。
-->
<template>
  <div class="brain-view">
    <header class="brain-head">
      <h1 class="brain-title">🧠 Viky · 共享大脑</h1>
      <p class="brain-sub">
        数字生命体意识引擎（PSI · 量子认知 · 全局工作空间）——LLM 是语言皮层，PSI 大脑循环先行。
      </p>
    </header>

    <!-- Φ 存在感横幅 -->
    <section class="brain-hero" :class="`brain-hero--${moodClass}`">
      <div class="brain-hero__phi">
        <div class="brain-hero__phi-label">自我存在感 Φ</div>
        <!-- 无快照时不留 0.000 假读数：那会被读成「Φ 真的是 0」 -->
        <div class="brain-hero__phi-val">{{ hasSnapshot ? phi.toFixed(3) : '—' }}</div>
        <div class="brain-hero__phi-track">
          <div class="brain-hero__phi-fill" :style="{ width: hasSnapshot ? `${Math.min(100, phi * 100)}%` : '0%' }" />
        </div>
      </div>
      <div class="brain-hero__meta">
        <div class="brain-hero__mood">
          <span class="brain-hero__mood-dot" :class="`mood-dot--${moodClass}`" />
          主导情绪：{{ hasSnapshot ? dominantMood : '—' }}
        </div>
        <div class="brain-hero__stat">相位节律：{{ hasSnapshot ? circadian : '—' }}</div>
        <div class="brain-hero__stat">
          循环 / 学习：{{ hasSnapshot ? `${cycleCount} 次 · ${learnedCount} 条` : '—' }}
        </div>
        <div class="brain-hero__stat">因果推理边：{{ hasSnapshot ? causalEdges : '—' }}</div>
      </div>
    </section>

    <p v-if="statusNotice" class="brain-notice brain-notice--warn">{{ statusNotice }}</p>

    <!-- 八维情感 -->
    <section class="brain-card">
      <h2 class="brain-card__title">八维实时情感</h2>
      <p v-if="!hasSnapshot" class="brain-assets-note">无引擎快照，情感维度不展示（而非按 50% 默认值填充）。</p>
      <div v-else class="brain-bars">
        <div v-for="d in dims" :key="d.key" class="brain-bar">
          <span class="brain-bar__label">{{ d.emoji }} {{ d.label }}</span>
          <div class="brain-bar__track">
            <div class="brain-bar__fill" :style="{ width: `${Math.round(d.value * 100)}%` }" />
          </div>
          <span class="brain-bar__val">{{ Math.round(d.value * 100) }}%</span>
        </div>
      </div>
    </section>

    <!-- 共享大脑 · 本地资产 -->
    <section class="brain-card">
      <h2 class="brain-card__title">共享大脑 · 本地知识资产</h2>
      <div class="brain-assets">
        <div class="brain-asset" v-for="a in assets" :key="a.label">
          <div class="brain-asset__num">{{ a.count }}</div>
          <div class="brain-asset__label">{{ a.label }}</div>
        </div>
      </div>
      <p class="brain-assets-note">
        共享大脑 = 全本地知识库（kb BM25×向量混合检索）+ 记忆（感官/短时/长时三级）+ 关系图谱 + 代码智能索引。
        所有任务先经过 PSI 大脑循环，再由 LLM 语言皮层输出——大脑与执行层分离。
      </p>
    </section>

    <!-- 操作 -->
    <section class="brain-card brain-actions">
      <button class="brain-btn" :disabled="cycleBusy" @click="runPsiCycle">🔄 触发一次 PSI 循环</button>
      <button class="brain-btn" @click="refresh">🔄 刷新状态</button>
      <span v-if="cycleNotice" class="brain-notice">{{ cycleNotice }}</span>
    </section>
  </div>
</template>

<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from 'vue'
import { useRpcStore } from '@/stores/rpc'

interface BrainSnapshot {
  emotion?: Record<string, number>
  dominant_mood?: string
  circadian?: string
  phi_existence?: string | number
  cycle_count?: number
  learned_count?: number
  causal_edges?: number
  last_broadcast?: number
}

const rpc = useRpcStore()
const brain = ref<BrainSnapshot | null>(null)
const cycleBusy = ref(false)
const cycleNotice = ref('')
let timer: ReturnType<typeof setInterval> | null = null

const DIMS: { key: string; label: string; emoji: string }[] = [
  { key: 'valence', label: '愉悦', emoji: '😊' },
  { key: 'arousal', label: '唤醒', emoji: '⚡' },
  { key: 'curiosity', label: '好奇', emoji: '🔍' },
  { key: 'confidence', label: '自信', emoji: '💪' },
  { key: 'connection', label: '连接', emoji: '💞' },
  { key: 'growth', label: '成长', emoji: '🌱' },
  { key: 'certainty', label: '确定感', emoji: '🧭' },
  { key: 'existence', label: '存在感', emoji: '✨' },
]

const phi = computed(() => {
  const v = parseFloat(String(brain.value?.phi_existence ?? '0.5'))
  return Number.isFinite(v) ? v : 0.5
})
const dominantMood = computed(() => brain.value?.dominant_mood ?? '—')
const circadian = computed(() => brain.value?.circadian ?? '—')
const cycleCount = computed(() => brain.value?.cycle_count ?? 0)
const learnedCount = computed(() => brain.value?.learned_count ?? 0)
const causalEdges = computed(() => brain.value?.causal_edges ?? 0)

const dims = computed(() =>
  DIMS.map((d) => ({
    ...d,
    value: Number(brain.value?.emotion?.[d.key] ?? 0.5),
  })),
)

const moodClass = computed(() => {
  const m = dominantMood.value.toLowerCase()
  if (m.includes('positive') || m.includes('joy') || m.includes('happy')) return 'happy'
  if (m.includes('sad') || m.includes('negative')) return 'sad'
  if (m.includes('anxious') || m.includes('fear')) return 'anxious'
  if (m.includes('confused') || m.includes('curious')) return 'curious'
  return 'neutral'
})

const assets = ref<{ label: string; count: number }[]>([])
/** 数据可用性 + 探测失败原因：空面板必须说清「为什么空」，绝不显示成 0 值事实。 */
const hasSnapshot = ref(false)
const statusNotice = ref('')

async function refresh(): Promise<void> {
  statusNotice.value = ''
  if (rpc.state !== 'connected') {
    // 未连接时不再静默：否则面板会拿默认值（Φ=0.000、情感 50%）当真实读数展示。
    statusNotice.value = '网关未连接，暂无法读取大脑状态'
    hasSnapshot.value = false
  } else if (!rpc.supportsMethod('brain.status')) {
    statusNotice.value = '本部署未提供 brain.status（PSI 引擎未装入）'
    hasSnapshot.value = false
  } else {
    try {
      const res = await rpc.call<{ ok?: boolean; brain?: BrainSnapshot }>('brain.status', {})
      brain.value = res?.brain ?? null
      hasSnapshot.value = brain.value !== null
      if (!hasSnapshot.value) {
        statusNotice.value = 'brain.status 已响应但未返回引擎快照'
      }
    } catch (e) {
      // 此前这里是静默 catch：引擎报错时面板照旧显示 0 值/50% 默认态，
      // 而文件头又写着「绝无模拟」——自相矛盾。现在如实报错。
      statusNotice.value = `读取大脑状态失败：${e instanceof Error ? e.message : String(e)}`
      hasSnapshot.value = false
    }
  }
  await loadAssets()
}

async function loadAssets(): Promise<void> {
  const next: { label: string; count: number }[] = []
  let failed = 0
  try {
    if (rpc.supportsMethod('pages.list')) {
      const p = await rpc.call<{ pages?: unknown[] }>('pages.list', {})
      next.push({ label: '知识页', count: p?.pages?.length ?? 0 })
    }
  } catch { failed += 1 }
  try {
    if (rpc.supportsMethod('kb.graph.view')) {
      const g = await rpc.call<{ entity_count?: number; edge_count?: number }>('kb.graph.view', {})
      next.push({ label: '图谱实体', count: g?.entity_count ?? 0 })
      next.push({ label: '图谱关系', count: g?.edge_count ?? 0 })
    }
  } catch { failed += 1 }
  assets.value = next
  if (failed > 0) {
    // 静默失败的后果是「资产面板少几格，用户以为是 0」→ 如实标注探测失败项数。
    const note = `有 ${failed} 项资产探测失败（对应格子不展示，勿当 0 计数）`
    statusNotice.value = statusNotice.value ? `${statusNotice.value}；${note}` : note
  }
}

async function runPsiCycle(): Promise<void> {
  cycleBusy.value = true
  cycleNotice.value = ''
  try {
    const res = await rpc.call<{ ok?: boolean; chosen?: string; reason?: Record<string, unknown> }>(
      'brain.psi_cycle',
      { input: '用户在共享大脑面板主动触发的一次思考' },
    )
    cycleNotice.value = res?.ok
      ? `决策：「${res.chosen ?? '—'}」`
      : 'PSI 循环未返回结果'
  } catch (e) {
    cycleNotice.value = `触发失败：${String(e)}`
  } finally {
    cycleBusy.value = false
    await refresh()
  }
}

onMounted(() => {
  void refresh()
  timer = setInterval(() => void refresh(), 4000)
})
onUnmounted(() => {
  if (timer) clearInterval(timer)
  timer = null
})
</script>

<style scoped>
.brain-view {
  padding: 18px 22px;
  max-width: 860px;
  margin: 0 auto;
  display: flex;
  flex-direction: column;
  gap: 16px;
}
.brain-head {
  margin-bottom: 2px;
}
.brain-title {
  font-size: 22px;
  margin: 0 0 4px;
}
.brain-sub {
  color: var(--text-muted);
  font-size: 13px;
  margin: 0;
}

.brain-hero {
  display: flex;
  align-items: center;
  gap: 22px;
  padding: 18px 20px;
  border-radius: var(--radius-card);
  border: 1px solid var(--border);
  background: linear-gradient(135deg, color-mix(in srgb, var(--accent) 18%, var(--bg-surface)), var(--bg-surface));
}
.brain-hero__phi {
  flex: none;
  width: 180px;
}
.brain-hero__phi-label {
  font-size: 12px;
  color: var(--text-muted);
  margin-bottom: 4px;
}
.brain-hero__phi-val {
  font-size: 34px;
  font-weight: 800;
  font-variant-numeric: tabular-nums;
  color: var(--accent);
  line-height: 1.1;
}
.brain-hero__phi-track {
  margin-top: 6px;
  height: 8px;
  border-radius: var(--radius-xs);
  background: color-mix(in srgb, var(--text) 14%, transparent);
  overflow: hidden;
}
.brain-hero__phi-fill {
  height: 100%;
  border-radius: var(--radius-xs);
  background: linear-gradient(90deg, var(--info), var(--accent));
  transition: width var(--dur-base) var(--ease-out);
}
.brain-hero__meta {
  flex: 1;
  display: flex;
  flex-direction: column;
  gap: 5px;
  font-size: 13px;
}
.brain-hero__mood {
  display: flex;
  align-items: center;
  gap: 6px;
  font-weight: 600;
}
.brain-hero__mood-dot {
  width: 10px;
  height: 10px;
  border-radius: 50%;
}
.mood-dot--happy { background: var(--ok); }
.mood-dot--sad { background: var(--danger); }
.mood-dot--anxious { background: var(--warn); }
.mood-dot--curious { background: var(--info); }
.mood-dot--neutral { background: var(--text-dim); }

.brain-card {
  padding: 16px 18px;
  border-radius: var(--radius-card);
  border: 1px solid var(--border);
  background: var(--bg-surface);
}
.brain-card__title {
  font-size: 15px;
  font-weight: 700;
  margin: 0 0 12px;
}

.brain-bars {
  display: flex;
  flex-direction: column;
  gap: 8px;
}
.brain-bar {
  display: flex;
  align-items: center;
  gap: 8px;
  font-size: 13px;
}
.brain-bar__label {
  width: 88px;
  flex: none;
}
.brain-bar__track {
  flex: 1;
  height: 10px;
  border-radius: var(--radius-xs);
  background: color-mix(in srgb, var(--text) 14%, transparent);
  overflow: hidden;
}
.brain-bar__fill {
  height: 100%;
  border-radius: var(--radius-xs);
  background: linear-gradient(90deg, var(--info), var(--accent));
  transition: width var(--dur-base) var(--ease-out);
}
.brain-bar__val {
  width: 42px;
  text-align: right;
  color: var(--text-dim);
  font-variant-numeric: tabular-nums;
}

.brain-assets {
  display: flex;
  flex-wrap: wrap;
  gap: 12px;
}
.brain-asset {
  flex: 1 1 120px;
  padding: 14px;
  border-radius: var(--radius-sm);
  background: color-mix(in srgb, var(--accent) 10%, transparent);
  text-align: center;
}
.brain-asset__num {
  font-size: 26px;
  font-weight: 800;
  color: var(--accent);
}
.brain-asset__label {
  font-size: 12px;
  color: var(--text-muted);
  margin-top: 2px;
}
.brain-assets-note {
  margin-top: 12px;
  font-size: 12px;
  color: var(--text-muted);
  line-height: 1.6;
}

.brain-actions {
  display: flex;
  align-items: center;
  gap: 10px;
  flex-wrap: wrap;
}
.brain-btn {
  padding: 8px 16px;
  border-radius: var(--radius-control);
  border: 1px solid var(--border);
  background: color-mix(in srgb, var(--accent) 12%, var(--bg-surface));
  color: var(--text);
  font: inherit;
  font-size: 13px;
  font-weight: 600;
  cursor: pointer;
  transition: background var(--dur-fast) var(--ease-standard);
}
.brain-btn:hover {
  background: color-mix(in srgb, var(--accent) 22%, var(--bg-surface));
}
.brain-btn:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}
.brain-notice {
  font-size: 12.5px;
  color: var(--accent);
}
/* 状态/探测失败提示：用警示色，避免和「操作成功」提示（accent）混淆 */
.brain-notice--warn {
  margin: 10px 0 0;
  padding: 8px 12px;
  background: color-mix(in srgb, var(--warn, var(--accent)) 12%, transparent);
  border: 1px solid color-mix(in srgb, var(--warn, var(--accent)) 32%, transparent);
  border-radius: var(--radius-sm);
  color: var(--warn, var(--accent));
}
</style>
