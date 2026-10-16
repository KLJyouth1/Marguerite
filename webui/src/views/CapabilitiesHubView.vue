<template>
  <div class="control-stage hub-stage">
    <header class="control-stage__header">
      <div class="control-stage__title-block">
        <p class="hub-kicker">Capabilities</p>
        <h1 class="control-stage__title">能力中心</h1>
        <p class="control-stage__subtitle">
          全部能力、设置分区与扩展入口的单一导航。健康度实时取自 plans.capabilities —
          真货、部分、叙事一目了然。
        </p>
      </div>
      <div class="control-stage__actions">
        <button class="btn btn--ghost" @click="load">
          <Icon name="refresh" :size="16" /><span>刷新</span>
        </button>
      </div>
    </header>

    <section v-if="stats" class="hub-meter" aria-label="能力健康度">
      <div class="hub-meter__bar">
        <span class="hub-meter__seg hub-meter__seg--real" :style="{ width: pct(stats.real) }" />
        <span class="hub-meter__seg hub-meter__seg--partial" :style="{ width: pct(stats.partial) }" />
        <span class="hub-meter__seg hub-meter__seg--narrative" :style="{ width: pct(stats.narrative) }" />
      </div>
      <div class="hub-pills">
        <span class="hub-pill">已实装 <b>{{ stats.implemented }}</b>/{{ stats.total }}</span>
        <span class="hub-pill hub-pill--accent">真货 {{ stats.real }}</span>
        <span class="hub-pill">部分 {{ stats.partial }}</span>
        <span class="hub-pill hub-pill--dim">纯叙事 {{ stats.narrative }}</span>
      </div>
    </section>

    <div class="hub-grid">
      <a
        v-for="(s, i) in sections"
        :key="s.name"
        class="hub-card"
        :href="s.href || undefined"
        :style="{ '--i': String(i) }"
        @click="s.href ? undefined : open(s)"
      >
        <div class="hub-card__icon"><Icon :name="s.icon" :size="18" /></div>
        <div class="hub-card__body">
          <b>{{ s.name }}</b>
          <span>{{ s.desc }}</span>
          <i>{{ s.status }}</i>
        </div>
        <span class="hub-card__arrow" aria-hidden="true">→</span>
      </a>
    </div>
  </div>
</template>

<script setup lang="ts">
import { ref, onMounted } from 'vue'
import Icon from '@/components/Icon.vue'
import { useRpcStore } from '@/stores/rpc'
import type { IconName } from '@/utils/icons'

const rpc = useRpcStore()
interface CapStats { total: number; real: number; partial: number; implemented: number; narrative: number }
interface Capabilities { stats?: CapStats }
const stats = ref<CapStats | null>(null)

const sections: Array<{ name: string; icon: IconName; desc: string; status: string; href: string }> = [
  { name: '模型服务', icon: 'model', desc: '引擎健康页：Rust 模型档案 + 运行时状态 + 模型路由 + 能力健康度', status: '已建页', href: '/control/engine' },
  { name: '能力', icon: 'capabilities', desc: '能力健康度 real/partial/叙事（plans.capabilities），本页实时取自该接口', status: '实时', href: '/control/engine' },
  { name: '设置', icon: 'settings', desc: '网关连接 / AI 配置 / 偏好 / 行为 / 隐私 / 沙箱 / 记忆 / 外观 / 键盘 / 高级（统一入口）', status: '设置内', href: '/settings' },
  { name: '扩展 / 插件', icon: 'boxes', desc: 'PluginsView / SettingsExtensionsPanel', status: '已建页', href: '/control/plugins' },
  { name: 'Agent 预设', icon: 'agents', desc: 'AgentsView + ExpertCenterView（智能体与专家中心）', status: '已建页', href: '/control/expert-center' },
  { name: '插件市场', icon: 'plugin', desc: 'PluginsView（market）', status: '已建页', href: '/control/plugins' },
  { name: 'Web UI 插件', icon: 'appWindow', desc: '浏览器驾驶舱 / 编辑器 / Games / EngineHealth', status: '已建页', href: '/control/browser-cockpit' },
  { name: '宠物', icon: 'pet', desc: 'PetView：茉莉仙子粒子艺术', status: '已建页', href: '/control/pet' },
  { name: '社区插件', icon: 'boxes', desc: '插件市场清单（manifest/catalog）', status: '已建页', href: '/control/plugins' },
  { name: '关系图谱', icon: 'gitBranch', desc: 'GraphView（KB 2.0 知识图谱）', status: '已建页', href: '/control/graph' },
  { name: '企业协同与 SSH', icon: 'collab', desc: 'SshCollabView（主机台账+探测）', status: '已建页', href: '/control/ssh-collab' },
  { name: '终端沙箱', icon: 'command', desc: 'TerminalView：sandbox.terminal.* 独立页', status: '已建页', href: '/control/terminal' },
  { name: '任务执行与质量', icon: 'gauge', desc: 'EngineHealth 任务质量卡 + 计划/执行坞', status: '已建页', href: '/control/engine' },
]

function pct(n: number): string {
  const total = stats.value?.total || 1
  return `${Math.max(0, Math.round((n / total) * 100))}%`
}

async function load() {
  try {
    const r = await rpc.call<Capabilities>('plans.capabilities', {})
    stats.value = r.stats ?? null
  } catch { /* 静默 */ }
}
function open(s: { href: string }) {
  if (s.href) return
  window.open('/settings', '_self')
}
onMounted(load)
</script>

<style scoped>
.hub-stage {
  padding: 24px 28px;
  max-width: 1200px;
  margin: 0 auto;
}

.hub-kicker {
  margin: 0 0 4px;
  font-family: var(--font-mono);
  font-size: 10px;
  letter-spacing: 0.22em;
  text-transform: uppercase;
  color: var(--accent);
}

.hub-meter {
  display: flex;
  flex-direction: column;
  gap: 10px;
}

.hub-meter__bar {
  display: flex;
  height: 8px;
  border-radius: var(--radius-pill);
  overflow: hidden;
  background: color-mix(in srgb, var(--text) 8%, transparent);
}

.hub-meter__seg { display: block; height: 100%; min-width: 0; }
.hub-meter__seg--real { background: var(--ok, var(--accent)); }
.hub-meter__seg--partial { background: var(--warn, var(--accent)); }
.hub-meter__seg--narrative { background: color-mix(in srgb, var(--text) 22%, transparent); }

.hub-pills {
  display: flex;
  flex-wrap: wrap;
  gap: 8px;
}

.hub-pill {
  padding: 4px 10px;
  border-radius: var(--radius-pill);
  background: color-mix(in srgb, var(--text) 8%, var(--card));
  font-size: 12px;
}

.hub-pill--accent { color: var(--accent); }
.hub-pill--dim { color: var(--text-dim); }

.hub-grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(280px, 1fr));
  gap: 12px;
}

.hub-card {
  display: grid;
  grid-template-columns: auto 1fr auto;
  gap: 12px;
  align-items: start;
  padding: 16px;
  background:
    linear-gradient(145deg, color-mix(in srgb, var(--accent) 7%, transparent), transparent 48%),
    var(--card);
  border: 1px solid var(--hairline);
  border-radius: var(--radius-card);
  text-decoration: none;
  color: inherit;
  animation: hub-rise var(--dur-enter) var(--ease-out) both;
  animation-delay: calc(var(--i, 0) * 35ms);
  transition:
    border-color var(--dur-fast) var(--ease-standard),
    transform var(--dur-fast) var(--ease-spring);
}

.hub-card:hover {
  border-color: var(--accent);
  transform: translateY(-2px);
}

.hub-card__icon { color: var(--accent); margin-top: 2px; }

.hub-card__body {
  display: flex;
  flex-direction: column;
  gap: 4px;
  min-width: 0;
}

.hub-card__body b { font-family: var(--font-display); font-size: 14px; }
.hub-card__body span { font-size: 11px; color: var(--text-dim); line-height: 1.45; }
.hub-card__body i { font-style: normal; font-size: 11px; color: var(--accent); }

.hub-card__arrow {
  color: var(--text-dim);
  font-size: 14px;
  transition: transform var(--dur-fast) var(--ease-spring), color var(--dur-fast) var(--ease-standard);
}

.hub-card:hover .hub-card__arrow {
  color: var(--accent);
  transform: translateX(3px);
}

@keyframes hub-rise {
  from { opacity: 0; transform: translateY(8px); }
  to { opacity: 1; transform: translateY(0); }
}

@media (prefers-reduced-motion: reduce) {
  .hub-card { animation: none; }
}
</style>
