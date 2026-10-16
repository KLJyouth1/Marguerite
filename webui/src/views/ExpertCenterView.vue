<template>
  <div class="control-stage expert-stage">
    <header class="control-stage__header">
      <div class="control-stage__title-block">
        <p class="expert-kicker">Agents · Experts · Swarm</p>
        <h1 class="control-stage__title">智能体与专家中心</h1>
        <p class="control-stage__subtitle">预设智能体 / 专家库（蒸馏行业经验）/ 模式蜂群规格 — 子代理扇出、工具白名单与记忆检索同一编排面。</p>
      </div>
      <div class="control-stage__actions"><button class="btn btn--ghost" @click="loadAll"><Icon name="refresh" :size="16" /><span>刷新</span></button></div>
    </header>

    <section class="expert-constellation" aria-hidden="true">
      <span v-for="n in expertOrbit" :key="n" class="expert-constellation__node">{{ n }}</span>
    </section>

    <section v-if="error" class="expert-error">{{ error }}</section>

    <!-- 模式与蜂群 -->
    <section v-if="spec" class="expert-card">
      <div class="expert-card__title">模式与蜂群（mode.spec）</div>
      <div class="expert-pills">
        <span class="expert-pill expert-pill--accent">当前模式：{{ spec.label }}</span>
        <span class="expert-pill">风险等级 {{ spec.risk_level }}</span>
        <span class="expert-pill">工具白名单 {{ spec.allowed_tools }} / 黑名单 {{ spec.denied_tools }}</span>
      </div>
      <div v-if="spec.bloom" class="expert-pills">
        <span class="expert-pill">蜂群分片上限（数量上限）{{ spec.bloom.max_swarm_shards }}</span>
        <span class="expert-pill">子专家上限 {{ spec.bloom.max_sub_experts }}</span>
        <span class="expert-pill">推理深度 {{ spec.bloom.thinking_depth }}</span>
        <span class="expert-pill">验证轮数 {{ spec.bloom.verification_passes }}</span>
        <span class="expert-pill">思考强度 {{ spec.bloom.reasoning_effort }}</span>
        <span class="expert-pill">快照安全网 {{ spec.bloom.snapshot_enabled ? '开' : '关' }}</span>
        <span class="expert-pill">自主闭环预算 {{ spec.bloom.budget_tokens }} token / {{ spec.bloom.max_iterations }} 轮</span>
      </div>
      <p class="expert-note">蜂群数量上限由内置模式规格（bloom.yaml / omni_transcend.yaml）决定；智能体、智能体团队、专家团队在 Bloom / OmniTranscend 模式下均可按此上限启动蜂群并行攻坚。</p>
    </section>

    <!-- 蜂群控制（数量上限设置） -->
    <section class="expert-card">
      <div class="expert-card__title">蜂群控制 · 数量上限设置（mode.swarm.get / set）</div>
      <div class="expert-form">
        <label class="expert-field">
          <span>蜂群分片上限（0 = 内置默认；硬上限 {{ swarm.maxShardsCap }}）</span>
          <input v-model.number="swarmShards" type="number" min="0" :max="swarm.maxShardsCap" class="expert-input" />
        </label>
        <label class="expert-field">
          <span>子专家上限（0 = 内置默认；硬上限 {{ swarm.maxSubExpertsCap }}）</span>
          <input v-model.number="swarmSubExperts" type="number" min="0" :max="swarm.maxSubExpertsCap" class="expert-input" />
        </label>
        <button class="btn btn--primary" :disabled="swarmBusy" @click="saveSwarm">保存上限</button>
        <button class="btn btn--ghost" :disabled="swarmBusy" @click="resetSwarm">恢复内置默认</button>
      </div>
      <div class="expert-pills expert-pills--swarm">
        <span class="expert-pill">生效分片上限 {{ swarm.effectiveShards }}</span>
        <span class="expert-pill">生效子专家上限 {{ swarm.effectiveSubExperts }}</span>
        <span class="expert-pill">已配置 {{ swarm.configuredShards ?? '内置默认' }} / {{ swarm.configuredSubExperts ?? '内置默认' }}</span>
      </div>
      <p class="expert-note">智能体 / 团队 / 专家启动蜂群时按此上限扇出（orchestration run_bloom_swarm 实时读取）；持久化 data_dir/swarm_config.json，重启不丢。</p>
    </section>

    <!-- 团队 / 蜂群一键启动 -->
    <section class="expert-card">
      <div class="expert-card__title">团队 / 蜂群一键启动（team.run · 真实并行 spawn）</div>
      <div class="expert-form">
        <input v-model="teamGoal" class="expert-input expert-input--wide" placeholder="目标描述（如：为登录功能设计接口契约、实现代码、编写单测）" @keyup.enter="launchTeam" />
        <button class="btn btn--primary" :disabled="teamBusy || !teamGoal.trim()" @click="launchTeam">{{ teamBusy ? '团队执行中…' : '启动专家团队' }}</button>
      </div>
      <p class="expert-note">自动角色检测 → 专家成员并行 spawn → lead 综合。未配置 API Key 时成员真实启动但 LLM 调用失败（如实回报）。蜂群（Bloom 双领队）在 Bloom / OmniTranscend 模式任务分析时自动扇出，数量上限见「蜂群控制」卡。</p>
      <div v-if="teamResult" class="expert-team">
        <div class="expert-team__head">策略 {{ teamResult.strategy }} · 成员产出 {{ teamResult.members }} 段</div>
        <ul v-if="teamResult.summaries && teamResult.summaries.length">
          <li v-for="(s, i) in teamResult.summaries" :key="i">{{ s }}</li>
        </ul>
        <pre v-if="teamResult.lead_reply">{{ teamResult.lead_reply }}</pre>
      </div>
      <div v-if="teamError" class="expert-note expert-note--err">{{ teamError }}</div>
    </section>

    <!-- 蜂群后台化 -->
    <section class="expert-card">
      <div class="expert-card__title">蜂群后台化（team.run.background · long_running 台账）</div>
      <div class="expert-form">
        <input v-model="bgGoal" class="expert-input expert-input--wide" placeholder="后台蜂群目标…（立即返回 taskId，执行不阻塞）" @keyup.enter="launchBackgroundTeam" />
        <button class="btn btn--ghost" :disabled="bgBusy || !bgGoal.trim()" @click="launchBackgroundTeam">启动后台蜂群</button>
      </div>
      <p v-if="bgNote" class="expert-note">{{ bgNote }}</p>
      <div v-if="bgTask" class="expert-pills expert-pills--swarm">
        <span class="expert-pill">任务 {{ bgTask.taskId }}</span>
        <span class="expert-pill">状态 {{ bgTask.status }}</span>
        <span class="expert-pill">重试 {{ bgTask.retryCount }}/{{ bgTask.maxRetries }}</span>
        <span class="expert-pill">检查点 {{ bgTask.checkpoints?.length ?? 0 }}</span>
      </div>
      <p class="expert-note">后台蜂群注册进 long_running 台账（心跳 15s / 检查点 / 完成态），引擎健康页「任务执行质量」卡同步可见；lead 综合与成员产出写入检查点。</p>
    </section>

    <!-- 专家团队持久化组合 -->
    <section class="expert-card">
      <div class="expert-card__title">专家团队持久化组合（team.compositions.*）</div>
      <div class="expert-form">
        <input v-model="compName" class="expert-input" placeholder="组合名" />
        <input v-model="compGoal" class="expert-input expert-input--wide" placeholder="目标描述" />
        <textarea v-model="compMembers" class="expert-textarea" placeholder="成员清单（每行：角色：简述）&#10;架构设计：接口契约&#10;编码实现：核心逻辑" />
        <button class="btn btn--primary" :disabled="compBusy || !compName.trim()" @click="saveComp">保存组合</button>
      </div>
      <div v-for="c in comps" :key="c.id" class="expert-comp">
        <b>{{ c.name }}</b>
        <span class="expert-item__desc">{{ c.goal }}</span>
        <span class="expert-item__desc">{{ (c.members ?? []).map(m => m.role).join(' / ') }}</span>
        <span class="expert-comp__actions">
          <button class="btn btn--sm btn--ghost" :disabled="compBusy" @click="loadComp(c)">载入</button>
          <button class="btn btn--sm btn--primary" :disabled="compBusy" @click="rerunComp(c)">后台重跑</button>
          <button class="btn btn--sm btn--ghost" :disabled="compBusy" @click="removeComp(c.name)">删除</button>
        </span>
      </div>
      <p v-if="comps.length === 0" class="expert-note">暂无已保存组合 —— 填写上方表单保存；重跑会把成员清单作为团队构成注入目标（参与角色检测）。</p>
    </section>

    <!-- 用户行为蒸馏 -->
    <section class="expert-card">
      <div class="expert-card__title">用户行为蒸馏 → 优化主智能体</div>
      <div class="expert-pills">
        <span class="expert-pill">行为事件 {{ behavior.events ?? 0 }}</span>
        <span class="expert-pill">对话轮次 {{ behavior.turns ?? 0 }}</span>
        <span class="expert-pill">工具 Top：<span v-for="(t, i) in (behavior.tool_uses ?? [])" :key="i">{{ t.tool }}×{{ t.count }} </span></span>
      </div>
      <div v-if="profile" class="expert-pills expert-pills--swarm">
        <span class="expert-pill expert-pill--accent">主智能体画像：{{ profile.role }} · {{ profile.tone }}</span>
        <span class="expert-pill">专长：{{ profile.expertise }}</span>
        <span class="expert-pill">来源 {{ profile.source_count }} 条上下文</span>
      </div>
      <p class="expert-note">每轮对话经 record_behavior 落账（B3 行为捕获，memory.behavior.summary 聚合）；画像由 agent.profile.auto 从最近上下文确定性抽取（不烧 token），经 persona 注入主智能体 —— 越用越懂你。</p>
    </section>

    <!-- 预设智能体 -->
    <section class="expert-card">
      <div class="expert-card__title">预设智能体（agents.list，内置在前）</div>
      <div v-if="agents.length" class="expert-grid">
        <div v-for="a in agents" :key="a.id" class="expert-item">
          <b>{{ a.id }}</b>
          <span class="expert-item__meta">
            <i v-if="a.is_builtin" class="expert-tag">内置</i>
            <i v-if="a.enabled === false" class="expert-tag expert-tag--dim">停用</i>
          </span>
          <span class="expert-item__desc">{{ a.model || '（默认模型路由）' }}</span>
        </div>
      </div>
      <p v-else class="expert-note">暂无智能体记录 — 可在「Agent」页创建自定义智能体（含自定义规则与逻辑）</p>
    </section>

    <!-- 预设专家库（可在对话框 @ 引用） -->
    <section class="expert-card">
      <div class="expert-card__title">专家团 / 预设专家库（experts.list · 可在对话框 @ 引用）</div>
      <p class="expert-note">
        在对话框输入 <b>@</b> 即可从下方专家与预设智能体中选取引用；此处「在对话框引用」会复制
        <code>@名称</code> 到剪贴板，粘贴进输入框即可（引用为消息文本标注，不改写发送负载）。
      </p>
      <p v-if="citeNote" class="expert-note expert-note--ok">{{ citeNote }}</p>
      <div class="expert-grid">
        <article v-for="e in experts" :key="e.id" class="expert-item">
          <b>{{ e.label }}</b>
          <span class="expert-item__desc">{{ e.lineage }}</span>
          <span class="expert-item__desc">{{ e.mindset }}</span>
          <span class="expert-item__desc">{{ e.skills }}</span>
          <span class="expert-item__playbook">剧本 {{ e.playbook.length }} 步</span>
          <button class="btn btn--sm btn--ghost expert-item__cite" @click="citeExpert(e)">在对话框引用 @</button>
        </article>
      </div>
      <p v-if="experts.length === 0" class="expert-note">暂无预设专家 —— 专家库为空或网关未就绪。</p>
    </section>

    <!-- 团队 / 蜂群 / 第三方说明 -->
    <section class="expert-card">
      <div class="expert-card__title">智能体团队 / 专家团队 / 蜂群 / 第三方生态</div>
      <ul class="expert-list">
        <li>智能体团队：core/agent/team.rs 已提供 Team/Mailbox/依赖拓扑（真实核心能力）；团队编排 UI 逐步接线（诚实状态）。</li>
        <li>蜂群：Bloom / OmniTranscend 模式内建双领队蜂群，数量上限见上「模式与蜂群」卡（max_swarm_shards）。</li>
        <li>三种并行协调（智能体 / 团队 / 专家）：dispatch 扇出 + subagent trellis 事件驱动（core/orchestration 已接线）。</li>
        <li>第三方专家与专家团队：market.import 可导入第三方插件清单；专家档案经 experts.list 展示（蒸馏式预设）。</li>
      </ul>
    </section>
  </div>
</template>

<script setup lang="ts">
import { onMounted, onUnmounted, ref } from 'vue'
import Icon from '@/components/Icon.vue'
import { useRpcStore } from '@/stores/rpc'

const rpc = useRpcStore()
const expertOrbit = ['Lead', 'Architect', 'Builder', 'Reviewer', 'Memory', 'Tools', 'Knowledge']

interface Expert { id: string; label: string; lineage: string; mindset: string; expertise: string; skills: string; playbook: string[] }
interface AgentItem { id: string; is_builtin?: boolean; enabled?: boolean; model?: string }
interface ModeSpec { kind: string; label: string; risk_level: string; allowed_tools: number; denied_tools: number; bloom?: { max_swarm_shards: number; max_sub_experts: number; thinking_depth: number; verification_passes: number; reasoning_effort: string; snapshot_enabled: boolean; budget_tokens: number; max_iterations: number } | null }

const experts = ref<Expert[]>([])
const agents = ref<AgentItem[]>([])
const spec = ref<ModeSpec | null>(null)
const error = ref('')
const citeNote = ref('')

/** 复制 `@名称` 到剪贴板，供用户在对话框输入框中引用该专家。 */
async function citeExpert(e: Expert) {
  const ref_ = `@${e.label}`
  try {
    await navigator.clipboard.writeText(ref_)
    citeNote.value = `已复制「${ref_}」——粘贴进对话框输入框即可引用`
  } catch {
    citeNote.value = `引用写法：「${ref_}」——在对话框输入框粘贴即可`
  }
  window.setTimeout(() => { citeNote.value = '' }, 5000)
}

async function loadAll() {
  error.value = ''
  try {
    // 冷加载竞态守卫：WS 仍处于 connecting 时先等连接就绪，避免把「未就绪」
    // 当成加载失败，把 `not connected (state: connecting)` 裸串渲染给用户。
    if (!rpc.isConnected) await rpc.waitForConnection(10_000)
    const [e, a, s] = await Promise.all([
      rpc.call<{ experts: Expert[] }>('experts.list', {}),
      rpc.call<{ agents: AgentItem[] }>('agents.list', {}),
      rpc.call<ModeSpec>('mode.spec', {}),
    ])
    void loadSwarm()
    void loadBehavior()
    void loadComps()
    experts.value = e.experts ?? []
    agents.value = a.agents ?? []
    spec.value = s
  } catch (err) {
    error.value = `加载失败: ${err instanceof Error ? err.message : String(err)}`
  }
}

interface SwarmCfg { configuredShards?: number | null; configuredSubExperts?: number | null; effectiveShards: number; effectiveSubExperts: number; maxShardsCap: number; maxSubExpertsCap: number }
const swarm = ref<SwarmCfg>({ effectiveShards: 11, effectiveSubExperts: 5, maxShardsCap: 200, maxSubExpertsCap: 50 })
const swarmShards = ref(0)
const swarmSubExperts = ref(0)
const swarmBusy = ref(false)

async function loadSwarm() {
  try {
    const s = await rpc.call<SwarmCfg>('mode.swarm.get', {})
    swarm.value = s
    swarmShards.value = s.configuredShards ?? 0
    swarmSubExperts.value = s.configuredSubExperts ?? 0
  } catch { /* 静默 */ }
}

async function saveSwarm() {
  swarmBusy.value = true
  try {
    const s = await rpc.call<SwarmCfg>('mode.swarm.set', { maxSwarmShards: swarmShards.value, maxSubExperts: swarmSubExperts.value })
    swarm.value = s
    swarmShards.value = s.configuredShards ?? 0
    swarmSubExperts.value = s.configuredSubExperts ?? 0
  } catch { /* 静默 */ } finally { swarmBusy.value = false }
}

async function resetSwarm() {
  swarmShards.value = 0
  swarmSubExperts.value = 0
  await saveSwarm()
}

interface TeamResult { ok: boolean; strategy?: string; members?: number; summaries?: string[]; lead_reply?: string; error?: string }
interface BehaviorSummary { events?: number; turns?: number; tool_uses?: Array<{ tool: string; count: number }> }
interface AutoProfile { role?: string; tone?: string; expertise?: string; source_count?: number }
const teamGoal = ref('')
const teamBusy = ref(false)
const teamResult = ref<TeamResult | null>(null)
const teamError = ref('')
const behavior = ref<BehaviorSummary>({})
const profile = ref<AutoProfile | null>(null)

async function launchTeam() {
  const goal = teamGoal.value.trim()
  if (!goal) return
  teamBusy.value = true
  teamError.value = ''
  teamResult.value = null
  try {
    const r = await rpc.call<TeamResult>('team.run', { goal })
    if (r.ok) teamResult.value = r
    else teamError.value = r.error ?? '团队执行失败'
  } catch (e) { teamError.value = e instanceof Error ? e.message : String(e) } finally { teamBusy.value = false }
}

async function loadBehavior() {
  try { behavior.value = await rpc.call<BehaviorSummary>('memory.behavior.summary', {}) } catch { /* 静默 */ }
  try { profile.value = await rpc.call<AutoProfile>('agent.profile.auto', {}) } catch { /* 静默 */ }
}

interface BgTask { taskId: string; status: string; retryCount: number; maxRetries: number; checkpoints?: unknown[] }
interface Composition { id: string; name: string; goal: string; members: Array<{ role: string; brief: string }> }
const bgGoal = ref('')
const bgBusy = ref(false)
const bgNote = ref('')
const bgTask = ref<BgTask | null>(null)
let bgTimer: ReturnType<typeof setInterval> | null = null
const compName = ref('')
const compGoal = ref('')
const compMembers = ref('')
const comps = ref<Composition[]>([])
const compBusy = ref(false)

async function pollBgTask(taskId: string) {
  try {
    const r = await rpc.call<{ tasks: BgTask[] }>('long_running.list', {})
    const t = (r.tasks ?? []).find(x => x.taskId === taskId)
    if (t) {
      bgTask.value = t
      if (t.status === 'completed' || t.status === 'failed' || t.status === 'interrupted') {
        if (bgTimer) { clearInterval(bgTimer); bgTimer = null }
      }
    }
  } catch { /* 静默 */ }
}

async function launchBackgroundTeam() {
  const goal = bgGoal.value.trim()
  if (!goal) return
  bgBusy.value = true
  bgNote.value = ''
  try {
    const r = await rpc.call<{ ok: boolean; taskId?: string; message?: string; error?: string }>('team.run.background', { goal })
    if (r.ok && r.taskId) {
      bgNote.value = r.message ?? '后台蜂群已启动'
      if (bgTimer) clearInterval(bgTimer)
      bgTimer = setInterval(() => void pollBgTask(r.taskId as string), 5000)
      void pollBgTask(r.taskId)
    } else { bgNote.value = r.error ?? '启动失败' }
  } catch (e) { bgNote.value = e instanceof Error ? e.message : String(e) } finally { bgBusy.value = false }
}

async function loadComps() {
  try {
    const r = await rpc.call<{ ok: boolean; items: Composition[] }>('team.compositions.list', {})
    comps.value = r.items ?? []
  } catch { /* 静默 */ }
}

function parseMembers(text: string): Array<{ role: string; brief: string }> {
  return text.split('\n')
    .map(l => {
      const t = l.trim()
      if (!t) return { role: '', brief: '' }
      const sep = t.indexOf('：') >= 0 ? t.indexOf('：') : t.indexOf(':')
      if (sep < 0) return { role: t, brief: '' }
      return { role: t.slice(0, sep).trim(), brief: t.slice(sep + 1).trim() }
    })
    .filter(m => m.role)
}

async function saveComp() {
  const name = compName.value.trim()
  if (!name) return
  compBusy.value = true
  try {
    const r = await rpc.call<{ ok: boolean; items: Composition[]; error?: string }>('team.compositions.save', { name, goal: compGoal.value.trim(), members: parseMembers(compMembers.value) })
    comps.value = r.items ?? []
    compName.value = ''; compGoal.value = ''; compMembers.value = ''
  } catch { /* 静默 */ } finally { compBusy.value = false }
}

async function removeComp(name: string) {
  compBusy.value = true
  try {
    const r = await rpc.call<{ ok: boolean; items: Composition[] }>('team.compositions.remove', { name })
    comps.value = r.items ?? []
  } catch { /* 静默 */ } finally { compBusy.value = false }
}

function loadComp(c: Composition) {
  compName.value = c.name
  compGoal.value = c.goal
  compMembers.value = (c.members ?? []).map(m => m.brief ? m.role + '：' + m.brief : m.role).join('\n')
}

async function rerunComp(c: Composition) {
  const roster = (c.members ?? []).map(m => m.brief ? m.role + '：' + m.brief : m.role).join('；')
  bgGoal.value = (roster ? '团队构成：' + roster + '。目标：' : '目标：') + c.goal
  await launchBackgroundTeam()
}

onMounted(loadAll)
onUnmounted(() => { if (bgTimer) clearInterval(bgTimer) })
</script>

<style scoped>
.expert-stage { padding: 24px 28px; max-width: 1180px; margin: 0 auto; }
.expert-kicker {
  margin: 0 0 4px;
  font-family: var(--font-mono);
  font-size: 10px;
  letter-spacing: 0.22em;
  text-transform: uppercase;
  color: var(--accent);
}
.expert-constellation {
  display: flex;
  flex-wrap: wrap;
  gap: 8px;
}
.expert-constellation__node {
  padding: 5px 11px;
  border-radius: var(--radius-pill);
  border: 1px solid color-mix(in srgb, var(--accent) 25%, var(--hairline));
  background: color-mix(in srgb, var(--accent) 8%, var(--card));
  font-family: var(--font-mono);
  font-size: 10px;
  letter-spacing: 0.12em;
  text-transform: uppercase;
  color: var(--text-muted);
  animation: expert-node-in var(--dur-enter) var(--ease-out) both;
}
.expert-constellation__node:nth-child(1) { animation-delay: 40ms; }
.expert-constellation__node:nth-child(2) { animation-delay: 80ms; }
.expert-constellation__node:nth-child(3) { animation-delay: 120ms; }
.expert-constellation__node:nth-child(4) { animation-delay: 160ms; }
.expert-constellation__node:nth-child(5) { animation-delay: 200ms; }
.expert-constellation__node:nth-child(6) { animation-delay: 240ms; }
.expert-constellation__node:nth-child(7) { animation-delay: 280ms; }
@keyframes expert-node-in {
  from { opacity: 0; transform: translateY(6px); }
  to { opacity: 1; transform: translateY(0); }
}
.expert-error { margin: 12px 0; padding: 10px 14px; background: color-mix(in srgb, var(--danger) 10%, var(--card)); border: 1px solid color-mix(in srgb, var(--danger) 30%, transparent); border-radius: var(--radius-sm); color: var(--danger); font-size: 12px; }
.expert-card {
  margin-top: 16px;
  padding: 16px;
  background:
    linear-gradient(150deg, color-mix(in srgb, var(--accent) 7%, transparent), transparent 46%),
    var(--card);
  border: 1px solid var(--hairline);
  border-radius: var(--radius-card);
}
.expert-card__title { font-family: var(--font-display); font-size: 14px; font-weight: 700; margin-bottom: 10px; }
.expert-pills { display: flex; flex-wrap: wrap; gap: 8px; }
.expert-pill { padding: 4px 10px; border-radius: 999px; background: color-mix(in srgb, var(--text) 8%, var(--card)); font-size: 12px; }
.expert-pill--accent { color: var(--accent); }
.expert-pills--swarm { margin-top: 10px; }
.expert-field { display: flex; flex-direction: column; gap: 4px; font-size: 11px; color: var(--text-dim); }
.expert-input { width: 120px; padding: 7px 9px; border: 1px solid var(--hairline); border-radius: var(--radius-sm); background: var(--bg); color: var(--text); font-size: 12px; }
.expert-input--wide { flex: 1; min-width: 260px; width: auto; }
.expert-team { margin-top: 12px; border-top: 1px solid var(--hairline); padding-top: 10px; }
.expert-team__head { font-size: 12px; font-weight: 600; margin-bottom: 6px; }
.expert-team ul { margin: 0; padding-left: 18px; font-size: 12px; color: var(--text-dim); }
.expert-team pre { margin: 8px 0 0; padding: 10px; background: var(--bg); border: 1px solid var(--hairline); border-radius: var(--radius-sm); font-family: var(--font-mono, monospace); font-size: 11px; white-space: pre-wrap; word-break: break-all; max-height: 320px; overflow: auto; }
.expert-note--err { color: var(--danger); }
.expert-textarea { flex-basis: 100%; width: 100%; min-height: 64px; padding: 8px 10px; border: 1px solid var(--hairline); border-radius: var(--radius-sm); background: var(--bg); color: var(--text); font-size: 12px; resize: vertical; font-family: var(--font-mono, monospace); }
.expert-comp { display: flex; gap: 10px; align-items: center; padding: 7px 0; border-bottom: 1px solid var(--hairline); font-size: 12px; }
.expert-comp b { min-width: 110px; color: var(--accent); }
.expert-comp__actions { margin-left: auto; display: flex; gap: 6px; }
.expert-note { margin: 10px 0 0; font-size: 12px; color: var(--text-dim); }
.expert-grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(250px, 1fr)); gap: 10px; }
.expert-item { display: flex; flex-direction: column; gap: 5px; padding: 10px; background: var(--bg); border: 1px solid var(--hairline); border-radius: var(--radius-sm); }
.expert-item b { font-size: 13px; color: var(--accent); }
.expert-item__meta { display: flex; gap: 6px; }
.expert-tag { font-style: normal; font-size: 10px; padding: 2px 7px; border-radius: 999px; background: color-mix(in srgb, var(--accent) 15%, var(--card)); color: var(--accent); }
.expert-tag--dim { background: var(--bg-hover); color: var(--text-dim); }
.expert-item__desc { font-size: 11px; color: var(--text-dim); word-break: break-all; }
.expert-item__playbook { font-size: 10px; color: var(--text-dim); }
.expert-item__cite { align-self: flex-start; margin-top: 2px; }
.expert-note--ok { color: var(--ok, var(--accent)); }
.expert-list { margin: 0; padding-left: 18px; font-size: 12px; color: var(--text-dim); line-height: 1.9; }
.expert-form { display: flex; gap: 12px; align-items: flex-end; flex-wrap: wrap; }
</style>